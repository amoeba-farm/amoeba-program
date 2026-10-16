//! Focused native transaction-runtime test of the council's resolution-and-checkpoint helper:
//! a wrong checkpoint account rolls back every resolution write; the exact one commits them.
//! Quorum and the voting dispatcher are covered by the council tests.
use super::*;
use crate::compact_error::CompactAccountInfo;
use borsh::BorshSerialize;
use solana_program_test::{processor, ProgramTest};
use solana_sdk::{
    account::Account,
    clock::Clock,
    instruction::{AccountMeta, Instruction},
    signature::Signer,
    transaction::Transaction,
};

fn adapter_entry(
    program: &Pubkey,
    a: &[AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    adapter(program, a, data).map_err(Into::into)
}

fn adapter(program: &Pubkey, a: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if a.len() != 13 || data != [0] {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut month: OracleMonthState = OracleMonthState::deserialize(&mut &a[1].try_data()?[..])?;
    let dispute = CouncilResolutionContext {
        month: *a[1].key,
        kind: OracleEmergencyDisputeKind::Update,
        target_id: [34; 32],
        case_key: *a[2].key,
    };
    // Quorum is a prerequisite of this focused helper test.
    let result = apply_emergency_resolution_and_checkpoint(
        program,
        &a[4..10],
        a[1].key,
        &mut month,
        a[2].key,
        &dispute,
        &a[3],
        &a[0],
        0,
        None,
        None,
        &a[10..13],
    );
    if let Err(error) = result {
        // Observe the helper's writes before returning the error to the runtime.
        // This distinguishes a checkpoint failure from an earlier admission failure.
        let source = load_valid_oracle_source(program, a[1].key, &a[5])?;
        let claim =
            load_valid_oracle_update_claim_v2_from_account(program, a[1].key, a[5].key, &a[4])?;
        if source.current_state != 110
            || source.observation_count != 2
            || claim.claim.status != OracleClaimStatus::Finalized
            || month.accepted_cash_update_count != 1
        {
            return Err(ProgramError::Custom(999_001));
        }
        return Err(error);
    }
    store_state(&a[1], &month)
}
fn fixture<T: BorshSerialize>(value: &T, len: usize) -> Account {
    let mut data = value.try_to_vec().unwrap();
    data.resize(len, 0);
    Account {
        lamports: 100_000_000,
        data,
        owner: crate::id(),
        executable: false,
        rent_epoch: 0,
    }
}
async fn bytes(
    ctx: &mut solana_program_test::ProgramTestContext,
    keys: &[Pubkey],
) -> Vec<Option<Account>> {
    let mut result = vec![];
    for key in keys {
        result.push(ctx.banks_client.get_account(*key).await.unwrap());
    }
    result
}

#[tokio::test]
async fn council_checkpoint_success_and_transaction_rollback() {
    for fail_checkpoint in [true, false] {
        let program = crate::id();
        let month_key = Pubkey::new_unique();
        let source_id = [31; 32];
        let claim_id = [32; 32];
        let challenge_id = [33; 32];
        let bucket_id = [35; 32];
        let contributor = Pubkey::new_unique();
        let (source_key, sb) = derive_oracle_source_pda(&program, &month_key, &source_id);
        let (obs_key, ob) = derive_oracle_source_observations_pda(&program, &source_key);
        let (claim_key, cb) = derive_oracle_update_claim_v2_pda(
            &program,
            &month_key,
            &source_key,
            &contributor,
            &claim_id,
        );
        let (challenge_key, hb) =
            derive_oracle_update_challenge_pda(&program, &month_key, &claim_key, &challenge_id);
        let (guard_key, gb) =
            derive_oracle_update_challenge_guard_pda(&program, &month_key, &claim_key);
        let dispute_key = case_address(&program, &month_key, &challenge_key).0;
        let (manifest_key, mb) = derive_oracle_active_weight_manifest_pda(&program, &month_key);
        let (bucket_key, bb) = derive_oracle_bucket_median_pda(&program, &month_key, &bucket_id);
        let (journal_key, journal_bump) = Pubkey::find_program_address(
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                b"g3-oracle-knowledge",
                source_key.as_ref(),
            ],
            &program,
        );
        let prior_checkpoint = Pubkey::new_unique();
        let checkpoint_key = Pubkey::find_program_address(
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                b"g3-oracle-checkpoint",
                source_key.as_ref(),
                dispute_key.as_ref(),
            ],
            &program,
        )
        .0;
        let supplied_checkpoint = if fail_checkpoint {
            Pubkey::new_unique()
        } else {
            checkpoint_key
        };
        let month = OracleMonthState {
            is_initialized: true,
            phase: OraclePhase::Game,
            pending_resolution_count: 1,
            recipe_hash: [36; 32],
            frozen_source_count: 1,
            opening_resolved_source_count: 1,
            opened_source_count: 1,
            weight_scheme_version: 1,
            effective_weight_total_bps: 10_000,
            weight_manifest_hash: [37; 32],
            active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
            active_weight_group_count: 1,
            active_weight_manifest_hash: [38; 32],
            ..Default::default()
        };
        let source = OracleSourceState {
            is_initialized: true,
            bump: sb,
            month: month_key,
            source_id,
            bucket_id,
            status: OracleSourceStatus::Active,
            opening_submitted: true,
            baseline_state: 100,
            current_state: 100,
            observation_count: 1,
            latest_source_time: 100,
            rolling_observation_hash: [39; 32],
            last_finalized_step: 1,
            ..Default::default()
        };
        let mut observations = OracleSourceObservations {
            is_initialized: true,
            bump: ob,
            account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
            account_version: OracleSourceObservations::ACCOUNT_VERSION,
            month: month_key,
            source: source_key,
            ..Default::default()
        };
        observations.states[0] = 100;
        observations.source_times[0] = 100;
        let earliest = 1 + ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS;
        let claim = OracleUpdateClaimV2 {
            claim: OracleUpdateClaimData {
                is_initialized: true,
                bump: cb,
                month: month_key,
                claim_id,
                source: source_key,
                source_id,
                claimant: contributor,
                prior_state: 100,
                new_state: 110,
                source_time: 200,
                stake: 1,
                status: OracleClaimStatus::Revealed,
                evidence_hash: [40; 32],
                archive_url_hash: [41; 32],
                account_discriminator: OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR,
                account_version: OracleUpdateClaimV2::ACCOUNT_VERSION,
                ..Default::default()
            },
            commit_hash: [42; 32],
            commit_slot: 1,
            earliest_reveal_slot: earliest,
            reveal_deadline_slot: earliest + ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
            revealed_slot: earliest,
            council_review_pending: true,
            freshness_reward_multiplier: 1,
            revealed_at_ts: 200,
            prior_finalized_step: source.last_finalized_step,
        };
        let challenge = OracleUpdateChallenge {
            is_initialized: true,
            bump: hb,
            month: month_key,
            challenge_id,
            claim: claim_key,
            claim_id,
            challenger: Pubkey::new_unique(),
            alternative_state: 120,
            alternative_source_time: 200,
            bond: 1,
            required_bond: 1,
            evidence_hash: [43; 32],
            archive_url_hash: [44; 32],
            account_discriminator: OracleUpdateChallenge::ACCOUNT_DISCRIMINATOR,
            account_version: OracleUpdateChallenge::ACCOUNT_VERSION,
            ..Default::default()
        };
        let guard = OracleUpdateChallengeGuard {
            is_initialized: true,
            bump: gb,
            account_discriminator: OracleUpdateChallengeGuard::ACCOUNT_DISCRIMINATOR,
            account_version: OracleUpdateChallengeGuard::ACCOUNT_VERSION,
            month: month_key,
            claim: claim_key,
            claim_id,
            challenge: challenge_key,
            challenge_id,
            active_dispute: dispute_key,
            resolution_step: 2,
            created_slot: 1,
            ..Default::default()
        };
        let manifest = OracleActiveWeightManifest {
            is_initialized: true,
            bump: mb,
            account_discriminator: OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleActiveWeightManifest::ACCOUNT_VERSION,
            month: month_key,
            phase: OracleRecipeWeightPhase::Finalized,
            expected_source_count: 1,
            processed_source_count: 1,
            expected_group_count: 1,
            processed_group_count: 1,
            processed_bucket_weight_bps: 10_000,
            rolling_manifest_hash: month.active_weight_manifest_hash,
            ..Default::default()
        };
        let bucket = OracleBucketMedianState {
            is_initialized: true,
            bump: bb,
            account_discriminator: OracleBucketMedianState::ACCOUNT_DISCRIMINATOR,
            account_version: OracleBucketMedianState::ACCOUNT_VERSION,
            month: month_key,
            bucket_id,
            bucket_weight_bps: 10_000,
            frozen_source_count: 1,
            active_source_count: 1,
            status: OracleBucketMedianStatus::Live,
            ..Default::default()
        };
        let mut test = ProgramTest::default();
        test.prefer_bpf(false);
        test.add_program(
            "focused_emergency_helper",
            program,
            processor!(adapter_entry),
        );
        for (key, a) in [
            (month_key, fixture(&month, OracleMonthState::LEN)),
            (source_key, fixture(&source, OracleSourceState::LEN)),
            (
                obs_key,
                fixture(&observations, OracleSourceObservations::LEN),
            ),
            (claim_key, fixture(&claim, OracleUpdateClaimV2::LEN)),
            (
                challenge_key,
                fixture(&challenge, OracleUpdateChallenge::LEN),
            ),
            (guard_key, fixture(&guard, OracleUpdateChallengeGuard::LEN)),
            (
                manifest_key,
                fixture(&manifest, OracleActiveWeightManifest::LEN),
            ),
            (bucket_key, fixture(&bucket, OracleBucketMedianState::LEN)),
            (dispute_key, fixture(&[0u8; 1], 1)),
        ] {
            test.add_account(key, a);
        }
        // Accepted opening history already exists before this update.
        let mut journal_data = b"OKJ".to_vec();
        journal_data.extend_from_slice(&[2, 1, journal_bump]);
        journal_data.extend_from_slice(source_key.as_ref());
        journal_data.extend_from_slice(prior_checkpoint.as_ref());
        journal_data.extend_from_slice(&source.rolling_observation_hash);
        journal_data.extend_from_slice(&1u32.to_le_bytes());
        journal_data.extend_from_slice(&1u32.to_le_bytes());
        test.add_account(
            journal_key,
            Account {
                lamports: 100_000_000,
                data: journal_data,
                owner: program,
                executable: false,
                rent_epoch: 0,
            },
        );
        let mut ctx = test.start_with_context().await;
        let mut clock: Clock = ctx.banks_client.get_sysvar().await.unwrap();
        clock.unix_timestamp = 200 + crate::constants::ORACLE_UPDATE_CHALLENGE_SECONDS as i64;
        ctx.set_sysvar(&clock);
        let keys = [
            month_key,
            dispute_key,
            challenge_key,
            claim_key,
            source_key,
            obs_key,
            manifest_key,
            guard_key,
            bucket_key,
            journal_key,
            supplied_checkpoint,
            checkpoint_key,
        ];
        let before = bytes(&mut ctx, &keys).await;
        let mut metas = vec![AccountMeta::new(ctx.payer.pubkey(), true)];
        metas.extend(keys[..11].iter().map(|k| AccountMeta::new(*k, false)));
        metas.push(AccountMeta::new_readonly(system_program::id(), false));
        let ix = Instruction {
            program_id: program,
            accounts: metas,
            data: vec![0],
        };
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&ctx.payer.pubkey()),
            &[&ctx.payer],
            ctx.last_blockhash,
        );
        let result = ctx.banks_client.process_transaction(tx).await;
        if fail_checkpoint {
            assert!(result.is_err(), "wrong checkpoint accepted");
            assert_eq!(
                bytes(&mut ctx, &keys).await,
                before,
                "resolution writes escaped failed transaction"
            );
            assert!(
                format!("{result:?}").contains(&format!(
                    "Custom({})",
                    OracleError::InvalidOracleState as u32
                )),
                "{result:?}"
            );
        } else {
            result.unwrap();
            let after = bytes(&mut ctx, &keys).await;
            let s = OracleSourceState::deserialize(&mut after[4].as_ref().unwrap().data.as_slice())
                .unwrap();
            assert_eq!(s.current_state, 110);
            assert_eq!(s.observation_count, 2);
            assert_eq!(s.last_finalized_step, 2);
            let c =
                OracleUpdateClaimV2::deserialize(&mut after[3].as_ref().unwrap().data.as_slice())
                    .unwrap();
            assert_eq!(c.claim.status, OracleClaimStatus::Finalized);
            assert!(!c.council_review_pending);
            let h =
                OracleUpdateChallenge::deserialize(&mut after[2].as_ref().unwrap().data.as_slice())
                    .unwrap();
            assert_eq!(h.status, OracleChallengeStatus::Rejected);
            let m = OracleMonthState::deserialize(&mut after[0].as_ref().unwrap().data.as_slice())
                .unwrap();
            assert_eq!(m.pending_resolution_count, 0);
            assert_eq!(m.accepted_cash_update_count, 1);
            let checkpoint = &after[10].as_ref().unwrap().data;
            assert_eq!(&checkpoint[..3], b"OKC");
            assert_eq!(&checkpoint[6..38], source_key.as_ref());
            assert_eq!(&checkpoint[70..102], dispute_key.as_ref());
            assert_eq!(
                u64::from_le_bytes(checkpoint[146..154].try_into().unwrap()),
                110
            );
            let journal = &after[9].as_ref().unwrap().data;
            assert_eq!(&journal[..3], b"OKJ");
            assert_eq!(&journal[38..70], checkpoint_key.as_ref());
            assert_eq!(&journal[70..102], &s.rolling_observation_hash);
            assert_eq!(journal[106], 2);
        }
    }
}
