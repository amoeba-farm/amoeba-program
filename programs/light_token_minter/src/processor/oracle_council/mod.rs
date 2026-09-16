//! Council-only oracle adjudication. No token electorate, deposits, or voter payouts.
use super::*;
use borsh::{BorshDeserialize, BorshSerialize};
mod authority;
mod effects;
mod helpers;
mod state;
mod targets;
use crate::instruction::OracleCouncilActionV1;
pub use authority::decode_council;
use authority::*;
use effects::*;
pub(super) use helpers::*;
use state::*;
pub use state::{council_decision, CouncilCase, CouncilRound};
use targets::*;

fn evidence(target: &AccountInfo) -> Result<[u8; 32], ProgramError> {
    Ok(hashv(&[
        b"amoeba-council-evidence-v1",
        target.key.as_ref(),
        &target.try_borrow_data()?,
    ])
    .to_bytes())
}
fn scope() -> &'static [u8] {
    #[cfg(feature = "mainnet-v3")]
    {
        b"solana-mainnet-beta:council-oracle-v1"
    }
    #[cfg(not(feature = "mainnet-v3"))]
    {
        b"solana-devnet:council-oracle-v1"
    }
}
fn case_hash(program: &Pubkey, controller: &Pubkey, case: &CouncilCase) -> [u8; 32] {
    hashv(&[
        b"amoeba-council-case-v1",
        scope(),
        program.as_ref(),
        controller.as_ref(),
        case.month.as_ref(),
        case.target.as_ref(),
        &case.target_id,
        &[case.kind as u8, case.choices, case.fallback],
        &case.evidence_hash,
        &case.snapshot_slot.to_le_bytes(),
        &case.opened_slot.to_le_bytes(),
        &case.deadline.to_le_bytes(),
    ])
    .to_bytes()
}

/// 0=open, 1=public ballot (and lazy current round), 2=apply, 3=stale update abort.
/// Common accounts: payer, market, month, case, target, canonical controller Config,
/// current-epoch round, system. Ballot adds readonly seat signer; then target-specific accounts.
#[inline(never)]
pub(super) fn process(
    program: &Pubkey,
    accounts: &[AccountInfo],
    action: OracleCouncilActionV1,
) -> ProgramResult {
    if accounts.len() < 8
        || action.operation > 3
        || !accounts[0].is_signer
        || !accounts[0].is_writable
        || accounts[1].is_writable
        || accounts[1].is_signer
        || !accounts[2].is_writable
        || accounts[2].is_signer
        || !accounts[3].is_writable
        || accounts[3].is_signer
        || !accounts[4].is_writable
        || accounts[4].is_signer
        || accounts[6].is_signer
        || !accounts[6].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (i, a) in accounts.iter().enumerate() {
        if accounts[..i].iter().any(|b| a.key == b.key)
            && !(action.operation == 1 && i == 8 && a.key == accounts[0].key && a.is_signer)
            && !(action.operation == 2
                && action.kind == OracleEmergencyDisputeKind::Update
                && i == 16
                && accounts.len() == 17
                && a.key == &system_program::id()
                && !a.is_writable
                && !a.is_signer)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    validate_system_program(&accounts[7])?;
    let view = council(&accounts[5])?;
    if action.expected_epoch != view.epoch || action.expected_seats_hash != view.digest {
        return Err(VaultError::GovernanceGateEpochMismatch.into());
    }
    let (market, mut month) =
        load_valid_market_and_oracle_month(program, &accounts[1], &accounts[2])?;
    if month.phase == OraclePhase::Closed {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    let start = if action.operation == 1 { 9 } else { 8 };
    if accounts.len() < start {
        return Err(VaultError::InvalidAccountList.into());
    }
    let remaining = &accounts[start..];
    let slot = Clock::get()?.slot;
    let (expected, bump) = case_address(program, accounts[2].key, accounts[4].key);
    if expected != *accounts[3].key {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let (round_key, round_bump) = round_address(program, accounts[3].key, view.epoch);
    if round_key != *accounts[6].key {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let kind = action.kind;
    if kind == OracleEmergencyDisputeKind::Update {
        oracle_usdc::ensure_current_cash_update_continuation_window(&market, &month)?;
    }
    let mut case = if action.operation == 0 {
        if action.expected_case_hash != [0; 32] || crate::bytes32_is_zero(&action.target_id) {
            return Err(VaultError::InvalidOracleEmergencyDispute.into());
        }
        let packet = validate_cash_emergency_target(
            program,
            accounts[2].key,
            &month,
            kind,
            &action.target_id,
            &accounts[4],
            remaining,
            None,
            None,
        )?;
        if packet.authority_version != 1 || packet.snapshot_slot > slot {
            return Err(VaultError::InvalidOracleEmergencyDispute.into());
        }
        validate_oracle_economics(&month.economics)?;
        let deadline = slot
            .checked_add(month.economics.emergency_commit_window_slots)
            .and_then(|n| n.checked_add(month.economics.emergency_reveal_window_slots))
            .ok_or(VaultError::ArithmeticOverflow)?;
        let mut c = CouncilCase {
            discriminator: *b"OCC",
            version: 1,
            initialized: true,
            bump,
            month: *accounts[2].key,
            target: *accounts[4].key,
            target_id: action.target_id,
            case_hash: [0; 32],
            evidence_hash: evidence(&accounts[4])?,
            kind,
            choices: packet.choice_count,
            fallback: packet.fallback_choice,
            snapshot_slot: packet.snapshot_slot,
            opened_slot: slot,
            deadline,
            finalized: false,
            outcome: packet.fallback_choice,
            reason: 0,
            decision_epoch: 0,
            seat_mask: 0,
            resolved_slot: 0,
        };
        c.case_hash = case_hash(program, accounts[5].key, &c);
        validate_create_only_program_account_target(program, &accounts[3])?;
        create_program_account(
            &accounts[0],
            &accounts[3],
            &accounts[7],
            program,
            CouncilCase::LEN,
            &[
                CASE_SEED,
                accounts[2].key.as_ref(),
                accounts[4].key.as_ref(),
                &[bump],
            ],
        )?;
        if kind != OracleEmergencyDisputeKind::BucketMedian {
            set_cash_emergency_guard_dispute_binding(
                program,
                kind,
                accounts[2].key,
                &accounts[4],
                remaining,
                &Pubkey::default(),
                accounts[3].key,
                slot,
            )?;
        }
        c
    } else {
        let c = load_case(program, &accounts[3])?;
        if c.finalized
            || c.month != *accounts[2].key
            || c.target != *accounts[4].key
            || c.kind != kind
            || c.target_id != action.target_id
            || c.case_hash != action.expected_case_hash
            || c.case_hash != case_hash(program, accounts[5].key, &c)
            || c.evidence_hash != evidence(&accounts[4])?
        {
            return Err(VaultError::InvalidOracleEmergencyDispute.into());
        }
        c
    };
    let mut round = if accounts[6].owner == program {
        load_round(program, &accounts[6], accounts[3].key, &view)?
    } else {
        // A missing current round after rotation contributes no votes. It never extends time.
        validate_create_only_program_account_target(program, &accounts[6])?;
        create_program_account(
            &accounts[0],
            &accounts[6],
            &accounts[7],
            program,
            CouncilRound::LEN,
            &[
                ROUND_SEED,
                accounts[3].key.as_ref(),
                &view.epoch.to_le_bytes(),
                &[round_bump],
            ],
        )?;
        CouncilRound {
            discriminator: *b"OCR",
            version: 1,
            initialized: true,
            bump: round_bump,
            case: *accounts[3].key,
            epoch: view.epoch,
            seats_hash: view.digest,
            ballots: [255; 5],
        }
    };
    match action.operation {
        0 => {}
        1 => {
            if slot < case.opened_slot || slot > case.deadline || action.choice >= case.choices {
                return Err(VaultError::OracleTimingWindowClosed.into());
            }
            let voter = &accounts[8];
            if !voter.is_signer || (voter.is_writable && voter.key != accounts[0].key) {
                return Err(VaultError::InvalidAccountList.into());
            }
            let seat = view
                .seats
                .iter()
                .position(|p| p == voter.key)
                .ok_or(VaultError::Unauthorized)?;
            validate_cash_emergency_target(
                program,
                accounts[2].key,
                &month,
                kind,
                &case.target_id,
                &accounts[4],
                remaining,
                None,
                Some(accounts[3].key),
            )?;
            if round.ballots[seat] != 255 {
                return Err(VaultError::InvalidOracleEmergencyDispute.into());
            }
            round.ballots[seat] = action.choice;
        }
        2 => {
            if slot <= case.deadline {
                return Err(VaultError::OracleTimingWindowClosed.into());
            }
            let (choice, mask, majority) =
                council_decision(&round.ballots, case.choices, case.fallback)?;
            let (core, coverage, merge, checkpoints) = resolution_accounts(kind, remaining)?;
            ensure_emergency_resolver_coverage_lane(&month, kind, coverage.is_some())?;
            let packet = validate_cash_emergency_target(
                program,
                accounts[2].key,
                &month,
                kind,
                &case.target_id,
                &accounts[4],
                core,
                Some(choice),
                Some(accounts[3].key),
            )?;
            if packet.authority_version != 1 || packet.snapshot_slot != case.snapshot_slot {
                return Err(VaultError::InvalidOracleEmergencyDispute.into());
            }
            if kind == OracleEmergencyDisputeKind::Update && choice == 2 && !checkpoints.is_empty()
            {
                return Err(VaultError::InvalidAccountList.into());
            }
            let context = CouncilResolutionContext {
                month: case.month,
                kind,
                target_id: case.target_id,
                case_key: *accounts[3].key,
            };
            apply_emergency_resolution_and_checkpoint(
                program,
                core,
                accounts[2].key,
                &mut month,
                accounts[3].key,
                &context,
                &accounts[4],
                &accounts[0],
                choice,
                coverage,
                merge,
                checkpoints,
            )?;
            if kind == OracleEmergencyDisputeKind::Source {
                set_cash_emergency_guard_dispute_binding(
                    program,
                    kind,
                    accounts[2].key,
                    &accounts[4],
                    core,
                    accounts[3].key,
                    &Pubkey::default(),
                    slot,
                )?;
            }
            case.finalized = true;
            case.outcome = choice;
            case.reason = if majority { 0 } else { 1 };
            case.decision_epoch = view.epoch;
            case.seat_mask = mask;
            case.resolved_slot = slot;
            month.last_updated_slot = slot;
            store_oracle_month_state(&accounts[2], &month)?;
        }
        3 => {
            abort_stale_update(
                program, accounts, &mut month, &mut case, remaining, slot, view.epoch,
            )?;
        }
        _ => return Err(VaultError::InvalidInstructionData.into()),
    }
    save_council_state(program, &accounts[6], &round)?;
    save_council_state(program, &accounts[3], &case)
}

type ResolutionAccounts<'a, 'i> = (
    &'a [AccountInfo<'i>],
    Option<(&'a AccountInfo<'i>, &'a AccountInfo<'i>)>,
    Option<(
        &'a AccountInfo<'i>,
        &'a AccountInfo<'i>,
        &'a AccountInfo<'i>,
    )>,
    &'a [AccountInfo<'i>],
);
fn resolution_accounts<'a, 'i>(
    kind: OracleEmergencyDisputeKind,
    a: &'a [AccountInfo<'i>],
) -> Result<ResolutionAccounts<'a, 'i>, ProgramError> {
    let empty = &a[a.len()..];
    match kind {
        OracleEmergencyDisputeKind::Source if a.len() == 4 => {
            Ok((&a[..2], Some((&a[2], &a[3])), None, empty))
        }
        OracleEmergencyDisputeKind::Source if a.len() == 9 => Ok((
            &a[..4],
            Some((&a[7], &a[8])),
            Some((&a[4], &a[5], &a[6])),
            empty,
        )),
        OracleEmergencyDisputeKind::Update if a.len() == 6 || a.len() == 9 => {
            Ok((&a[..6], None, None, &a[6..]))
        }
        OracleEmergencyDisputeKind::Opening if a.len() == 2 => Ok((a, None, None, empty)),
        OracleEmergencyDisputeKind::BucketMedian if a.is_empty() => Ok((a, None, None, empty)),
        _ => Err(VaultError::InvalidAccountList.into()),
    }
}

fn abort_stale_update(
    program: &Pubkey,
    accounts: &[AccountInfo],
    month: &mut OracleMonthState,
    case: &mut CouncilCase,
    a: &[AccountInfo],
    slot: u64,
    epoch: u64,
) -> ProgramResult {
    if case.kind != OracleEmergencyDisputeKind::Update || a.len() != 3 || slot <= case.deadline {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let source = load_valid_oracle_source(program, &case.month, &a[0])?;
    let mut claim =
        load_valid_oracle_update_claim_v2_from_account(program, &case.month, a[0].key, &a[1])?;
    let mut challenge = oracle_usdc::load_exact_current_oracle_update_challenge(
        program,
        &case.month,
        &accounts[4],
    )?;
    let mut guard = load_canonical_update_challenge_guard(
        program,
        &case.month,
        a[1].key,
        &claim.claim.claim_id,
        &a[2],
    )?;
    if !claim.council_review_pending
        || challenge.council_authority_version != 1
        || challenge.status != OracleChallengeStatus::RuleReviewUnresolved
        || challenge.claim != *a[1].key
        || challenge.claim_id != claim.claim.claim_id
        || challenge.challenge_id != case.target_id
        || guard.active_dispute != *accounts[3].key
        || guard.challenge != *accounts[4].key
        || guard.resolution_step == 0
        || claim.claim.status != OracleClaimStatus::Revealed
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.bond == 0
        || challenge.bond != challenge.required_bond
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    let market = load_valid_market_and_oracle_month(program, &accounts[1], &accounts[2])?.0;
    oracle_usdc::ensure_oracle_update_cleanup_ready_at(
        &market,
        &source,
        &claim,
        Some(&guard),
        current_unix_timestamp()?,
    )?;
    claim.claim.status = OracleClaimStatus::TimedOut;
    claim.council_review_pending = false;
    challenge.status = OracleChallengeStatus::Cancelled;
    guard.last_updated_slot = slot;
    decrement_pending_oracle_resolution(month)?;
    month.last_updated_slot = slot;
    case.finalized = true;
    case.outcome = case.fallback;
    case.reason = 2;
    case.decision_epoch = epoch;
    case.seat_mask = 0;
    case.resolved_slot = slot;
    store_state(&a[1], &claim)?;
    store_state(&a[2], &guard)?;
    store_state(&accounts[4], &challenge)?;
    store_oracle_month_state(&accounts[2], month)
}

/// Exact proof/materialization inventory for the new council selector.
pub(super) fn required_accesses(
    a: &OracleCouncilActionV1,
    count: usize,
) -> Result<super::compressed_state::RequiredAccesses, ProgramError> {
    use super::compressed_state::{
        RequiredAccessKind as K, RequiredAccessSpec as S, RequiredAccesses,
    };
    use crate::state::CompressedStateDomain as D;
    let base = if a.operation == 1 { 9usize } else { 8 };
    let n = count
        .checked_sub(base)
        .ok_or(VaultError::InvalidAccountList)?;
    let mut v = Vec::new();
    let rw = if a.operation == 2 {
        K::Mutable
    } else {
        K::ReadOnly
    };
    let mut source = |i: usize, k| {
        v.push(S::new(i as u8, D::OracleSourceState, k));
    };
    match a.kind {
        OracleEmergencyDisputeKind::Source => {
            let comparison = match (a.operation, n) {
                (0 | 1, 2) | (2, 4) => false,
                (0 | 1, 4) | (2, 9) => true,
                _ => return Err(VaultError::InvalidAccountList.into()),
            };
            source(base, rw);
            if comparison {
                source(base + 2, rw);
            }
            if a.operation == 2 {
                if comparison {
                    v.push(S::new((base + 4) as u8, D::OracleUsdcSkuPool, K::ReadOnly));
                    v.push(S::new(
                        (base + 5) as u8,
                        D::OracleUsdcSourceReward,
                        K::Mutable,
                    ));
                    v.push(S::new(
                        (base + 6) as u8,
                        D::OracleUsdcSourceReward,
                        K::Mutable,
                    ));
                }
                v.push(S::new(
                    (count - 1) as u8,
                    D::OracleSkuCoverageRecord,
                    K::Mutable,
                ));
            }
        }
        OracleEmergencyDisputeKind::Opening if a.operation <= 2 && n == 2 => {
            source(base + 1, rw);
        }
        OracleEmergencyDisputeKind::Update if a.operation == 3 && n == 3 => {
            source(base, K::ReadOnly);
        }
        OracleEmergencyDisputeKind::Update if n == 6 || (a.operation == 2 && n == 9) => {
            source(base + 1, rw);
            v.push(S::new((base + 2) as u8, D::OracleSourceObservations, rw));
            if n == 9 {
                v.push(S::new(
                    (base + 6) as u8,
                    D::OracleCarryJournal,
                    K::MutableOrInitialize,
                ));
                v.push(S::new(
                    (base + 7) as u8,
                    D::OracleCarryCheckpoint,
                    K::Initialize,
                ));
            }
        }
        OracleEmergencyDisputeKind::BucketMedian if a.operation <= 2 && n == 0 => {}
        _ => return Err(VaultError::InvalidAccountList.into()),
    }
    Ok(RequiredAccesses::from_slice(&v))
}

fn load_council_state<T: BorshDeserialize>(
    info: &AccountInfo,
    program: &Pubkey,
    len: usize,
    _error: VaultError,
) -> Result<T, ProgramError> {
    if info.owner != program || info.data_len() != len || info.executable {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    T::try_from_slice(&info.try_borrow_data()?)
        .map_err(|_| VaultError::InvalidOracleEmergencyDispute.into())
}
fn save_council_state<T: BorshSerialize>(
    program: &Pubkey,
    info: &AccountInfo,
    value: &T,
) -> ProgramResult {
    let bytes = borsh::to_vec(value).map_err(|_| VaultError::InvalidOracleEmergencyDispute)?;
    if info.owner != program || !info.is_writable || bytes.len() != info.data_len() {
        return Err(VaultError::InvalidAccountList.into());
    }
    info.try_borrow_mut_data()?.copy_from_slice(&bytes);
    Ok(())
}
