//! The production (mainnet) artifact, built without `v1-formation`
//! (`target/deploy/ameba_oracle.so`; `scripts/check.sh oracle` builds it before these tests, and
//! `AMEBA_ORACLE_PRODUCTION_SO` names another), loaded the way mainnet will take it: as an upgrade
//! of an oracle running the formation build (`SBF_OUT_DIR`, the build every other test runs).
//!
//! - Its first-byte table: exactly the instructions a Live v1 month needs until it pays out (and
//!   the admin, collateral and cleanup paths) reach the governance envelope (`KEPT`, a list kept
//!   here on purpose, apart from the program's own), plus 210 when the build routes heads; every
//!   v1 formation byte is refused as an unassigned byte is, before any account is read.
//! - Every kept instruction and tag-30 selector still decodes and reaches its own checks, with
//!   the formation build's result; the formation selectors (and sponsorship's Activate) are
//!   refused.
//! - A month formed, listed and updated by the formation build updates, settles and pays out on
//!   the production artifact after the upgrade: 199, 167, 201, 169, 197, 75, 113, 116, 170, 171,
//!   173, 174 and 30/24.
#![cfg(all(
    feature = "test-sbf",
    not(feature = "localnet-dev-authority"),
    not(feature = "localnet-governance")
))]

mod common;

use ameba_oracle::{
    constants::{ORACLE_SETTLEMENT_GRACE_SECONDS, ORACLE_UPDATE_REVEAL_WINDOW_SLOTS},
    error::OracleError,
    governance_gate::{
        derive_protocol_gate_pda, GovernanceInstructionTailV1, PINNED_CONTROLLER_PROGRAM_ID,
    },
    governance_manifest::{
        classify_instruction_tag_for_build, FeatureGates, InstructionGovernanceClass,
        HEADS_INSTRUCTION_TAG, V1_FORMATION_CARRY_SELECTORS, V1_FORMATION_INSTRUCTION_TAGS,
    },
    instruction::{
        self, AccumulateOracleSettlementSourceBucketParams, ChallengeOracleUpdateClaimParams,
        CommitOracleUpdateClaimV2Params, OracleCarryForwardActionV1 as Action,
        OracleCouncilActionV1, OracleInstructionTag, OracleSponsoredActionV1,
        ProposeOracleSourceV3Params, ProposeSettlementSignerRotationParams,
        RecomputeOracleBucketMedianV1Params, RevealOracleUpdateClaimV3Params,
        RotateVaultAuthoritiesV2Params, SettlementLeaf, TerminalCleanupParams, UpdateFinalization,
        UpsertSettlementParams,
    },
    sponsorship::{SponsorTerms, DEFAULT_RESEARCHER_BPS, DEFAULT_RETENTION_BPS},
    state::{
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_sku_pool_pda,
        derive_oracle_usdc_source_reward_pda, derive_user_collateral_pda, OracleClaimStatus,
        OracleEmergencyDisputeKind, OracleEscrowKind, OracleMonthState, OraclePhase,
        OracleUpdateClaimOutcome, OracleUpdateClaimV2, OracleUsdcRewardKind,
        OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    },
};
use ameba_oracle_interface::PeriodStatus;
use borsh::BorshSerialize;
use common::{cohort::*, production::*, settle::*, *};
use solana_program_test::BanksClientError;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};

const DAY: u64 = 86_400;

/// Every instruction byte the production artifact must route, independent of the program's own
/// manifest (`governance_manifest::V1_FORMATION_INSTRUCTION_TAGS`), so that dropping one is a
/// test failure rather than a quiet change. Collateral: 9, 10, 11, 140. Admin: 2, 109, 129 (the
/// only unpause). Updates during Game: 166, 167, 168, 198, 199, 201. Settlement: 75, 79, 113, 116,
/// 197. Signer rotation: 91-94. Rewards: 170, 171, 173, 174. Escrows and failure refunds: 169,
/// 191, 194, 195. Tag 30: council, terminal cleanup, evidence and sponsorship settlement (the
/// failed-month cleanup, 30/25, lives there too).
const KEPT: [u8; 31] = [
    2, 9, 10, 11, 30, 75, 79, 91, 92, 93, 94, 109, 113, 116, 129, 140, 166, 167, 168, 169, 170,
    171, 173, 174, 191, 194, 195, 197, 198, 199, 201,
];

/// The first-byte classes each build routes: the production artifact without the v1 formation
/// (heads as this test crate's default features say, which `--features test-sbf` leaves as the
/// production build has them), the formation build with it.
fn gates(formation: bool) -> FeatureGates {
    FeatureGates {
        dev: false,
        heads: cfg!(feature = "heads"),
        v1_formation: formation,
    }
}

/// Sends each instruction alone in its own transaction and returns each outcome; with
/// `envelope`, each carries the governance tail and gate as senders add them. A fee payer of its
/// own keeps every call's transactions distinct from another call's (the probes repeat across
/// builds), and each transaction takes the bank's latest blockhash, so a slow run never outlives
/// one.
async fn outcomes(
    h: &mut Harness,
    instructions: &[Instruction],
    envelope: bool,
) -> Vec<Result<(), BanksClientError>> {
    let payer = Keypair::new();
    h.fund(&payer.pubkey(), 1_000_000_000).await;
    let mut results = Vec::new();
    for ix in instructions {
        let mut ix = ix.clone();
        if envelope {
            ix.data
                .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(1).encode());
            ix.accounts.push(AccountMeta::new_readonly(
                derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &program()).0,
                false,
            ));
        }
        let blockhash: Hash = h.ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx =
            Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
        results.push(
            process_now(&h.ctx.banks_client, tx)
                .await
                .0
                .map_err(BanksClientError::TransactionError),
        );
    }
    results
}

/// A transaction's outcome as text: `Ok`, or its instruction error (`Custom(n)` for the
/// program's own codes).
fn code(result: &Result<(), BanksClientError>) -> String {
    match result {
        Ok(()) => "Ok".into(),
        Err(BanksClientError::TransactionError(TransactionError::InstructionError(_, error))) => {
            format!("{error:?}")
        }
        Err(other) => format!("{other:?}"),
    }
}

fn program_error(error: OracleError) -> String {
    format!("{:?}", InstructionError::Custom(error as u32))
}

fn bare(byte: u8) -> Instruction {
    Instruction {
        program_id: program(),
        accounts: vec![],
        data: vec![byte],
    }
}

/// One instruction to `program()` with `tag` and `payload` and no accounts of its own: a
/// decodable payload that reaches the handler, which then refuses the empty account list.
fn probe(tag: u8, payload: Vec<u8>) -> Instruction {
    Instruction {
        program_id: program(),
        accounts: vec![],
        data: [vec![tag], payload].concat(),
    }
}

fn carry(action: Action) -> Instruction {
    probe(30, action.try_to_vec().unwrap())
}

fn bytes<T: BorshSerialize>(value: &T) -> Vec<u8> {
    value.try_to_vec().unwrap()
}

/// A decodable payload for every kept byte but 30.
fn kept_probes() -> Vec<(u8, Instruction)> {
    let key = Pubkey::new_unique;
    let payloads: Vec<(u8, Vec<u8>)> = vec![
        // UpdateConfig: five absent options.
        (2, vec![0; 5]),
        (9, vec![]),
        (10, 1u64.to_le_bytes().to_vec()),
        (11, 1u64.to_le_bytes().to_vec()),
        (75, vec![]),
        (79, vec![]),
        (
            91,
            bytes(&ProposeSettlementSignerRotationParams {
                target_version: 2,
                threshold: 1,
                signer_count: 1,
                signers: [key(); 8],
                rotation_delay_slots: 0,
                activate_after_slot: 0,
            }),
        ),
        (92, vec![]),
        (93, vec![]),
        (
            94,
            bytes(&ProposeSettlementSignerRotationParams {
                target_version: 2,
                threshold: 1,
                signer_count: 1,
                signers: [key(); 8],
                rotation_delay_slots: 0,
                activate_after_slot: 0,
            }),
        ),
        (
            109,
            bytes(&RotateVaultAuthoritiesV2Params {
                new_admin: key(),
                new_oracle_authority: key(),
            }),
        ),
        (
            113,
            bytes(&AccumulateOracleSettlementSourceBucketParams {
                bucket_id: [1; 32],
                finalize_collection: false,
            }),
        ),
        (
            116,
            bytes(&UpsertSettlementParams {
                settlement: SettlementLeaf {
                    schema_version: SettlementLeaf::CURRENT_SCHEMA_VERSION,
                    ..SettlementLeaf::default()
                },
            }),
        ),
        // ActivateVaultV2: no freeze authority.
        (129, vec![0]),
        (140, 1u64.to_le_bytes().to_vec()),
        (
            166,
            bytes(&CommitOracleUpdateClaimV2Params {
                claim_id: [1; 32],
                commit_hash: [2; 32],
                stake: 1,
            }),
        ),
        (167, vec![]),
        (
            168,
            bytes(&ChallengeOracleUpdateClaimParams {
                challenge_id: [1; 32],
                alternative_state: 1,
                alternative_source_time: 1,
                bond: 1,
                evidence_hash: [2; 32],
                archive_url: String::new(),
            }),
        ),
        // SettleOracleUsdcEscrow: kind 5, an update claim.
        (169, vec![5]),
        (170, vec![]),
        (171, vec![]),
        (173, vec![]),
        // ClaimOracleUsdcReward: kind 3, an update.
        (174, vec![3]),
        (191, vec![]),
        (194, vec![0]),
        (195, vec![]),
        (
            197,
            bytes(&RecomputeOracleBucketMedianV1Params {
                bucket_id: [1; 32],
                mode: 0,
            }),
        ),
        (
            198,
            bytes(&RevealOracleUpdateClaimV3Params {
                claim_id: [1; 32],
                prior_state: 1,
                new_state: 2,
                source_time: 1,
                evidence_hash: [2; 32],
                archive_url: String::new(),
                secret_salt: [3; 32],
            }),
        ),
        // FinalizeOracleUpdateClaimV2: accept, step 1.
        (199, [vec![0], 1u64.to_le_bytes().to_vec()].concat()),
        (201, vec![]),
    ];
    payloads
        .into_iter()
        .map(|(tag, payload)| (tag, probe(tag, payload)))
        .collect()
}

/// Tag 30 actions the production artifact keeps: the evidence writer (14, 15), the council (18),
/// sponsorship settlement (22: reconcile, bounty, reserve) and the terminal cleanup (24).
fn kept_carry_probes() -> Vec<(&'static str, Instruction)> {
    vec![
        (
            "30/14",
            carry(Action::WriteEvidence {
                kind: 1,
                hash: [1; 32],
                total: 1,
                offset: 0,
                bytes: vec![0],
            }),
        ),
        ("30/15", carry(Action::CloseEvidenceDraft)),
        (
            "30/18",
            carry(Action::Council(OracleCouncilActionV1 {
                operation: 0,
                kind: OracleEmergencyDisputeKind::BucketMedian,
                target_id: [1; 32],
                expected_case_hash: [2; 32],
                expected_epoch: 1,
                expected_seats_hash: [3; 32],
                choice: 0,
            })),
        ),
        (
            "30/22 reconcile",
            carry(Action::SponsoredSource(OracleSponsoredActionV1::Reconcile)),
        ),
        (
            "30/22 bounty",
            carry(Action::SponsoredSource(
                OracleSponsoredActionV1::ClaimBounty,
            )),
        ),
        (
            "30/22 reserve",
            carry(Action::SponsoredSource(
                OracleSponsoredActionV1::WithdrawReserve { amount: 1 },
            )),
        ),
        (
            "30/24",
            carry(Action::TerminalCleanup(TerminalCleanupParams {
                kind: 0,
                bucket_id: [0; 32],
                expected_data_hash: [0; 32],
                expected_lamports: 0,
            })),
        ),
    ]
}

/// The v1 formation's tag-30 actions: carry formation (0-7), the median certificates (8-13), the
/// evidence backfills (16, 17) and sponsorship's Activate (an underwritten source's proposal).
fn formation_carry_probes() -> Vec<(&'static str, Instruction)> {
    let activate = OracleSponsoredActionV1::Activate {
        terms: SponsorTerms {
            agent_id: [1; 32],
            researcher: Pubkey::new_unique(),
            candidate_hash: [2; 32],
            funding: 0,
            researcher_bps: DEFAULT_RESEARCHER_BPS,
            retention_bps: DEFAULT_RETENTION_BPS,
            maximum_bond: 1,
            maximum_outstanding: 1,
        },
        source: ProposeOracleSourceV3Params {
            source_id: [2; 32],
            bucket_id: [3; 32],
            source_type_hash: [4; 32],
            canonical_locator_hash: [5; 32],
            source_definition_hash: [6; 32],
            listing_bond: 1,
            sku_index: 0,
            sku_proof: vec![],
        },
    };
    vec![
        ("30/0", carry(Action::RegisterRoot)),
        ("30/1", carry(Action::RegisterSuccessor)),
        (
            "30/2",
            carry(Action::CaptureCurrent {
                kind: 1,
                previous_hash: [1; 32],
            }),
        ),
        (
            "30/3",
            carry(Action::Import {
                sku_index: 0,
                proof: vec![[1; 32]],
            }),
        ),
        ("30/4", carry(Action::SkipInactive)),
        ("30/5", carry(Action::BeginSelection)),
        ("30/6", carry(Action::ScanCheckpoint)),
        ("30/7", carry(Action::FreezeOpening)),
        (
            "30/8",
            carry(Action::BeginHistoryMedian {
                mode: 0,
                lower: 1,
                upper: 1,
            }),
        ),
        ("30/9", carry(Action::ScanHistoryMedian)),
        ("30/10", carry(Action::CloseHistoryMedian)),
        (
            "30/11",
            carry(Action::BeginBucketRank {
                mode: 0,
                lower: 1,
                upper: 1,
                nonce: [1; 32],
            }),
        ),
        ("30/12", carry(Action::ScanBucketRank)),
        ("30/13", carry(Action::CloseBucketRank)),
        ("30/16", carry(Action::BackfillSourceEvidence)),
        ("30/17", carry(Action::BackfillClaimEvidence { role: 2 })),
        ("30/22 activate", carry(Action::SponsoredSource(activate))),
    ]
}

#[tokio::test]
async fn the_production_artifact_routes_exactly_what_a_live_month_needs() {
    let Some((mut h, production)) = start_formation().await else {
        return;
    };
    // The manifest's formation set is exactly the assigned bytes outside `KEPT`.
    let assigned: Vec<u8> = OracleInstructionTag::ALL.iter().map(|t| *t as u8).collect();
    for tag in KEPT {
        assert!(assigned.contains(&tag), "{tag} is not an assigned byte");
    }
    let formation_tags: Vec<u8> = assigned
        .iter()
        .copied()
        .filter(|tag| !KEPT.contains(tag))
        .collect();
    assert_eq!(formation_tags, V1_FORMATION_INSTRUCTION_TAGS);
    let carry_selectors: Vec<u8> = formation_carry_probes()
        .iter()
        .filter(|(label, _)| !label.ends_with("activate"))
        .map(|(_, ix)| ix.data[1])
        .collect();
    assert_eq!(carry_selectors, V1_FORMATION_CARRY_SELECTORS);

    let bytes: Vec<Instruction> = (0u8..=u8::MAX).map(bare).collect();
    let kept = kept_probes();
    let kept_ix: Vec<Instruction> = kept.iter().map(|(_, ix)| ix.clone()).collect();
    let carry_kept = kept_carry_probes();
    let carry_kept_ix: Vec<Instruction> = carry_kept.iter().map(|(_, ix)| ix.clone()).collect();
    let carry_formation = formation_carry_probes();
    let carry_formation_ix: Vec<Instruction> =
        carry_formation.iter().map(|(_, ix)| ix.clone()).collect();

    let mut runs = Vec::new();
    for formation in [true, false] {
        if !formation {
            h.upgrade_oracle(&production).await;
        }
        runs.push((
            outcomes(&mut h, &bytes, false).await,
            outcomes(&mut h, &kept_ix, true).await,
            outcomes(&mut h, &carry_kept_ix, true).await,
            outcomes(&mut h, &carry_formation_ix, true).await,
        ));
    }
    let routed = program_error(OracleError::MissingGovernanceTail);
    let refused = program_error(OracleError::InvalidInstructionData);
    for ((bare, kept_out, carry_kept_out, carry_formation_out), formation) in
        runs.iter().zip([true, false])
    {
        // A bare byte: an assigned one the build routes reaches the envelope (and finds no
        // tail); any other is refused before it.
        for (byte, result) in bare.iter().enumerate() {
            let byte = byte as u8;
            let class = classify_instruction_tag_for_build(byte, gates(formation));
            let expected = match class {
                InstructionGovernanceClass::RecognizedMutating
                | InstructionGovernanceClass::FeatureGatedMutating => &routed,
                _ => &refused,
            };
            assert_eq!(
                code(result),
                *expected,
                "byte {byte}, formation {formation}"
            );
            if !formation {
                let production_routes = KEPT.contains(&byte)
                    || (byte == HEADS_INSTRUCTION_TAG && cfg!(feature = "heads"));
                assert_eq!(code(result) == routed, production_routes, "byte {byte}");
            }
        }
        // Kept instructions decode and reach their handlers, which refuse the empty account list.
        for ((tag, _), result) in kept.iter().zip(kept_out) {
            assert_ne!(code(result), refused, "tag {tag}, formation {formation}");
        }
        for ((label, _), result) in carry_kept.iter().zip(carry_kept_out) {
            assert_ne!(code(result), refused, "{label}, formation {formation}");
        }
        // Formation selectors reach their handlers only in the formation build.
        for ((label, _), result) in carry_formation.iter().zip(carry_formation_out) {
            if formation {
                assert_ne!(code(result), refused, "{label}");
            } else {
                assert_eq!(code(result), refused, "{label}");
            }
        }
    }
    // The kept paths answer exactly as the formation build's.
    let (formation_run, production_run) = (&runs[0], &runs[1]);
    for (index, (tag, _)) in kept.iter().enumerate() {
        assert_eq!(
            code(&formation_run.1[index]),
            code(&production_run.1[index]),
            "tag {tag}"
        );
    }
    for (index, (label, _)) in carry_kept.iter().enumerate() {
        assert_eq!(
            code(&formation_run.2[index]),
            code(&production_run.2[index]),
            "{label}"
        );
    }
}

fn update_escrow(
    cohort: &Cohort,
    cranker: &Pubkey,
    source: &Pubkey,
    claim: &Pubkey,
    owner: &Pubkey,
) -> Instruction {
    instruction::settle_oracle_usdc_escrow(
        &program(),
        OracleEscrowKind::UpdateClaim,
        vec![
            AccountMeta::new_readonly(*cranker, true),
            AccountMeta::new_readonly(cohort.period, false),
            AccountMeta::new_readonly(cohort.month, false),
            AccountMeta::new_readonly(*source, false),
            AccountMeta::new(*claim, false),
            AccountMeta::new(derive_user_collateral_pda(&program(), owner).0, false),
            AccountMeta::new_readonly(
                instruction::find_oracle_update_guard_address(&program(), &cohort.month, claim),
                false,
            ),
        ],
    )
}

/// The mainnet sequence: months formed and listed by today's program, then the upgrade, then
/// everything up to the payout on the production artifact.
#[tokio::test]
async fn a_month_listed_before_the_upgrade_settles_and_pays_out_on_the_production_artifact() {
    let Some((mut h, production)) = start_formation().await else {
        return;
    };
    let wide = wide_live(&mut h).await;
    let cohort = &wide.cohort;
    let bucket = cohort.sku_ids[0];
    let (source0, source1, source2) = (wide.sources[0], wide.sources[1], wide.sources[2]);

    // Before the upgrade, in Game: update A committed and revealed (finalized after it), B only
    // committed (its stake refunds after the reveal window), C revealed and never finalized (it
    // goes stale at expiry and grace).
    let updater = h.participant(PARTICIPANT_USDC).await;
    let funded = h.collateral(&updater.pubkey()).await.available_balance;
    let now = cohort.listing + 3_600;
    h.at(now).await;
    let mut claims = Vec::new();
    for (index, (source, id)) in [(source0, 0usize), (source1, 1), (source2, 2)]
        .into_iter()
        .enumerate()
    {
        let print = h
            .print(
                cohort,
                &updater,
                &source,
                &WIDE_SOURCES[id],
                101_000_000 + index as u64,
                now - 60,
            )
            .await;
        claims.push(
            h.commit(
                cohort,
                &updater,
                &source,
                &WIDE_SOURCES[id],
                0,
                [31 + index as u8; 32],
                print,
            )
            .await
            .unwrap(),
        );
    }
    h.advance_slot().await;
    h.reveal(cohort, &updater, &source0, &claims[0])
        .await
        .unwrap();
    h.reveal(cohort, &updater, &source2, &claims[2])
        .await
        .unwrap();

    // The upgrade: formation bytes are refused as unassigned from the next slot on.
    h.upgrade_oracle(&production).await;
    // The formation build formed the month on 15-second test days; mainnet's months were formed
    // on real ones, which the production artifact checks them against. Give the month the
    // calendar mainnet's have: its Scramble starts eight real days before its listing
    // (`scramble_start_ts`, byte 70).
    let mut account = h.account(cohort.month).await.unwrap();
    let start = cohort.listing
        - ameba_oracle::constants::ORACLE_PRE_LISTING_WINDOW_SECONDS
            / ameba_oracle::constants::ORACLE_CALENDAR_DAY_SECONDS
            * DAY;
    account.data[70..78].copy_from_slice(&start.to_le_bytes());
    h.ctx.set_account(
        &cohort.month,
        &solana_sdk::account::AccountSharedData::from(account),
    );
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(
        (month.scramble_start_ts, month.listing_ts),
        (start, cohort.listing)
    );
    // Likewise the settlement signer set: mainnet's has the production rotation delay (216,000
    // slots, byte 336; outside the set hash), the formation build's the test one.
    let set_key = ameba_oracle::state::derive_settlement_signer_set_pda(&program(), 1).0;
    let mut account = h.account(set_key).await.unwrap();
    let delay = ameba_oracle::constants::PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS;
    account.data[336..344].copy_from_slice(&delay.to_le_bytes());
    h.ctx.set_account(
        &set_key,
        &solana_sdk::account::AccountSharedData::from(account),
    );
    let set: ameba_oracle::state::SettlementSignerSet = h.read_state(set_key).await;
    assert_eq!(
        (set.rotation_delay_slots, set.set_hash),
        (delay, set.compute_set_hash())
    );
    let payer = h.payer();
    let period = instruction::create_period(
        &program(),
        &payer.pubkey(),
        &payer.pubkey(),
        product("RAMX"),
        cohort.expiry,
        cohort.expiry + 90 * DAY,
        6,
    );
    assert_oracle_error(
        h.send(period, &[]).await,
        OracleError::InvalidInstructionData,
    );

    // 199: A is accepted a day after its reveal; 169 returns its stake.
    h.at(now + DAY).await;
    h.finalize(
        cohort,
        &source0,
        0,
        &claims[0].key,
        OracleUpdateClaimOutcome::AcceptClaim,
        1,
        UpdateFinalization::Unchallenged,
    )
    .await
    .unwrap();
    let accepted: OracleUpdateClaimV2 = h.read_state(claims[0].key).await;
    assert_eq!(accepted.claim.status, OracleClaimStatus::Finalized);
    h.send(
        update_escrow(
            cohort,
            &payer.pubkey(),
            &source0,
            &claims[0].key,
            &updater.pubkey(),
        ),
        &[],
    )
    .await
    .unwrap();
    // 167: B's commitment refunds once its reveal window has passed.
    h.advance_slots(ORACLE_UPDATE_REVEAL_WINDOW_SLOTS + 2).await;
    h.send(
        instruction::settle_expired_oracle_update_commitment_v3(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &source1,
            &claims[1].key,
            &updater.pubkey(),
        ),
        &[],
    )
    .await
    .unwrap();
    let expired: OracleUpdateClaimV2 = h.read_state(claims[1].key).await;
    assert_eq!(expired.claim.status, OracleClaimStatus::TimedOut);

    // At expiry and grace: 201 times C out and 169 refunds it; then 197, 75 and 113.
    h.at(cohort.expiry + ORACLE_SETTLEMENT_GRACE_SECONDS).await;
    h.send(
        instruction::cancel_stale_oracle_update_claim_v2(
            &program(),
            &payer.pubkey(),
            &cohort.period,
            &cohort.month,
            &source2,
            &claims[2].key,
            None,
        ),
        &[],
    )
    .await
    .unwrap();
    h.send(
        update_escrow(
            cohort,
            &payer.pubkey(),
            &source2,
            &claims[2].key,
            &updater.pubkey(),
        ),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        h.collateral(&updater.pubkey()).await.available_balance,
        funded,
        "every update stake is back"
    );
    let month: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(month.pending_resolution_count, 0);
    h.finalize_wide(&wide).await;

    // 116: the attested publication settles the period.
    let leaf = h.leaf_for(cohort).await;
    let signers: Vec<Keypair> = h
        .settlement_signers
        .iter()
        .map(|key| key.insecure_clone())
        .collect();
    h.publish(
        cohort,
        &[&signers[0], &signers[1], &signers[2]],
        &leaf,
        &leaf,
    )
    .await
    .unwrap();
    assert_eq!(
        h.period(cohort.product, cohort.expiry).await.status,
        PeriodStatus::Settled
    );

    // 169 (listing bonds, support stakes), 170, 171, 173, then 174 pays the proposer and A's
    // updater.
    h.settle_wide_escrows(&wide).await;
    for source in &wide.sources {
        let opening =
            instruction::find_oracle_opening_claim_address(&program(), &cohort.month, source);
        h.send(
            instruction::register_oracle_usdc_reward_source(
                &program(),
                &payer.pubkey(),
                &cohort.month,
                &bucket,
                source,
                Some(&opening),
            ),
            &[],
        )
        .await
        .unwrap();
    }
    h.send(
        instruction::register_oracle_usdc_reward_update(
            &program(),
            &payer.pubkey(),
            &cohort.month,
            &bucket,
            &source0,
            &claims[0].key,
        ),
        &[],
    )
    .await
    .unwrap();
    let oracle = h.oracle_authority.insecure_clone();
    h.advance_slot().await;
    h.send(
        instruction::finalize_oracle_usdc_reward_entitlements(
            &program(),
            &oracle.pubkey(),
            &cohort.period,
            &cohort.month,
        ),
        &[&oracle],
    )
    .await
    .unwrap();
    let schedule = derive_oracle_usdc_reward_schedule_pda(&program(), &cohort.month).0;
    let frozen: OracleUsdcRewardSchedule = h.read_state(schedule).await;
    assert_eq!(
        frozen.phase,
        OracleUsdcRewardSchedulePhase::EntitlementsFinalized
    );
    let sku = derive_oracle_usdc_sku_pool_pda(&program(), &schedule, &bucket).0;
    let reward0 = derive_oracle_usdc_source_reward_pda(&program(), &schedule, &source0).0;
    let proposer = wide.proposer.insecure_clone();
    let proposer_paid = h
        .claim_reward(
            cohort,
            &proposer,
            OracleUsdcRewardKind::SourceProposer,
            &source0,
            vec![
                AccountMeta::new(sku, false),
                AccountMeta::new_readonly(source0, false),
                AccountMeta::new_readonly(reward0, false),
                AccountMeta::new_readonly(
                    instruction::find_carry_source_address(&program(), &source0),
                    false,
                ),
            ],
        )
        .await
        .unwrap();
    assert!(proposer_paid > 0);
    let registration = ameba_oracle::state::derive_oracle_usdc_reward_registration_pda(
        &program(),
        &schedule,
        OracleUsdcRewardKind::Update,
        &claims[0].key,
    )
    .0;
    let update_paid = h
        .claim_reward(
            cohort,
            &updater,
            OracleUsdcRewardKind::Update,
            &claims[0].key,
            vec![
                AccountMeta::new(sku, false),
                AccountMeta::new_readonly(registration, false),
                AccountMeta::new_readonly(claims[0].key, false),
            ],
        )
        .await
        .unwrap();
    assert_eq!(update_paid, UPDATE_REWARD);

    // 30/24: the terminal cleanup closes the month.
    h.cleanup(cohort, 0, [0; 32], None, None).await.unwrap();
    let closed: OracleMonthState = h.read_state(cohort.month).await;
    assert_eq!(closed.phase, OraclePhase::Closed);
    // All of it ran on the production artifact (the harness reinstalls it after each warp): a
    // formation byte is still refused as unassigned.
    let period = instruction::create_period(
        &program(),
        &payer.pubkey(),
        &payer.pubkey(),
        product("NANDX"),
        cohort.expiry,
        cohort.expiry + 90 * DAY,
        6,
    );
    assert_oracle_error(
        h.send(period, &[]).await,
        OracleError::InvalidInstructionData,
    );
}
