//! Authenticated carry-forward behind the ordinary governed dispatcher.
use super::compressed_state::{RequiredAccessKind, RequiredAccessSpec, RequiredAccesses};
use super::*;
use crate::state::CompressedStateDomain;

mod bucket_rank;
mod checkpoints;
pub(in crate::processor) mod compressed;
mod history;
mod openings;
mod periods;
mod scan_transition;
mod state;
pub(in crate::processor) use bucket_rank::verified_bucket_rank;
use bucket_rank::*;
use checkpoints::*;
pub(in crate::processor) use history::verified_history_median;
use history::*;
use openings::*;
use periods::*;
use scan_transition::*;
use state::*;

pub(in crate::processor) use checkpoints::{record_fresh_accept, AcceptedEvent};
pub(in crate::processor) use openings::{
    inherited_reward_opening, require_carry_resolved_before_expiry,
};
pub(in crate::processor) use periods::{require_fresh_discovery, require_import_complete};

pub(in crate::processor) fn journal_address_for_transport(
    program: &Pubkey,
    source: &Pubkey,
) -> (Pubkey, u8) {
    address(program, JOURNAL_SEED, source.as_ref())
}

use crate::instruction::OracleCarryForwardActionV1 as Action;

fn decode(payload: &[u8]) -> Result<Action, ProgramError> {
    if payload.len() > MAX_INSTRUCTION_DATA_BYTES {
        return invalid();
    }
    <Action as borsh::BorshDeserialize>::try_from_slice(payload)
        .map_err(|_| VaultError::InvalidInstructionData.into())
}

/// Preserve the classic ScanCheckpoint ordering before any compressed leaf is
/// decoded or authenticated. The source identity is only a key; the mutable
/// carry account remains the sole typed account loaded here.
pub(in crate::processor) fn validate_scan_checkpoint_cursor_for_transport(
    program: &Pubkey,
    source_key: &Pubkey,
    carry_info: &AccountInfo,
    checkpoint_key: &Pubkey,
) -> ProgramResult {
    let carry = load_carry(program, source_key, carry_info)?;
    validate_scan_cursor(&carry, checkpoint_key)
}

/// Authenticate the full domain-13 leaf and its typed checkpoint context
/// without creating a temporary classic account. This runs before the Light
/// proof CPI so no state transition is attempted for an unauthenticated read.
pub(in crate::processor) fn validate_scan_checkpoint_leaf_for_transport(
    program: &Pubkey,
    source_key: &Pubkey,
    carry_info: &AccountInfo,
    checkpoint_key: &Pubkey,
    data: &[u8],
) -> ProgramResult {
    let carry = load_carry(program, source_key, carry_info)?;
    validate_scan_cursor(&carry, checkpoint_key)?;
    if data.len() != Checkpoint::LEN {
        return invalid();
    }
    let checkpoint =
        Checkpoint::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
    compressed::validate_carry_payload(
        program,
        checkpoint_key,
        CompressedStateDomain::OracleCarryCheckpoint,
        data,
    )?;
    let read = AuthenticatedCheckpointRead {
        canonical_key: *checkpoint_key,
        record: checkpoint,
    };
    validate_authenticated_checkpoint(program, &carry, &read)?;
    if read.record.month != carry.parent_month || read.record.sequence != carry.remaining {
        return invalid();
    }
    Ok(())
}

/// Apply the already proof-authenticated checkpoint view through the shared
/// typed transition and persist only the existing mutable CarrySource account.
pub(in crate::processor) fn apply_scan_checkpoint_leaf_for_transport(
    program: &Pubkey,
    source_key: &Pubkey,
    carry_info: &AccountInfo,
    checkpoint_key: &Pubkey,
    data: &[u8],
) -> ProgramResult {
    let mut carry = load_carry(program, source_key, carry_info)?;
    validate_scan_cursor(&carry, checkpoint_key)?;
    if data.len() != Checkpoint::LEN {
        return invalid();
    }
    let checkpoint =
        Checkpoint::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
    compressed::validate_carry_payload(
        program,
        checkpoint_key,
        CompressedStateDomain::OracleCarryCheckpoint,
        data,
    )?;
    apply_authenticated_checkpoint_read(
        program,
        &mut carry,
        AuthenticatedCheckpointRead {
            canonical_key: *checkpoint_key,
            record: checkpoint,
        },
    )?;
    save(program, carry_info, &carry)
}

/// Payload excludes the outer instruction tag and the governed epoch tail.
/// This exact contract is also encoded by the TypeScript account/proof plan.
pub(in crate::processor) fn required_accesses(
    payload: &[u8],
    account_count: usize,
) -> Result<RequiredAccesses, ProgramError> {
    use CompressedStateDomain::{
        OracleCarryCheckpoint as Checkpoint, OracleCarryJournal as Journal,
        OracleSourceDescriptor as Descriptor, OracleSourceObservations as Observations,
        OracleSourceState as Source, OracleUsdcSkuPool as Sku, OracleUsdcSourceReward as Reward,
    };
    use RequiredAccessKind::{Initialize, Mutable, MutableOrInitialize, ReadOnly as Read};
    let s = RequiredAccessSpec::new;
    let (expected, specs): (usize, Vec<RequiredAccessSpec>) = match decode(payload)? {
        Action::Council(action) => {
            return super::oracle_council::required_accesses(&action, account_count)
        }
        Action::RegisterRoot => (7, vec![]),
        Action::BackfillSourceEvidence | Action::BackfillClaimEvidence { .. } => {
            (8, vec![s(2, Source, Read), s(2, Descriptor, Read)])
        }
        Action::WriteEvidence { .. } => (3, vec![]),
        Action::CloseEvidenceDraft => (2, vec![]),
        Action::BeginHistoryMedian { .. } => (
            7,
            vec![
                s(3, Source, Read),
                s(3, Descriptor, Read),
                s(4, Journal, Read),
            ],
        ),
        Action::ScanHistoryMedian => (4, vec![s(3, Checkpoint, Read)]),
        Action::CloseHistoryMedian => (2, vec![]),
        Action::BeginBucketRank { .. } => (7, vec![]),
        Action::ScanBucketRank => (9, vec![s(5, Source, Read), s(5, Descriptor, Read)]),
        Action::CloseBucketRank => (2, vec![]),
        Action::RegisterSuccessor => (10, vec![]),
        Action::CaptureCurrent { kind, .. } => (
            if kind == 2 { 10 } else { 9 },
            vec![
                s(3, Source, Read),
                s(3, Descriptor, Read),
                s(4, Observations, Read),
                s(6, Journal, MutableOrInitialize),
                s(7, Checkpoint, Initialize),
            ],
        ),
        Action::Import { .. } => (
            23,
            vec![
                s(4, Sku, Read),
                s(5, Source, Initialize),
                s(5, Descriptor, Initialize),
                s(6, Observations, Initialize),
                s(7, Reward, Initialize),
                s(11, Source, Read),
                s(11, Descriptor, Read),
            ],
        ),
        Action::SkipInactive => (10, vec![s(5, Source, Read), s(5, Descriptor, Read)]),
        Action::BeginSelection => (
            10,
            vec![
                s(2, Source, Read),
                s(2, Descriptor, Read),
                s(6, Source, Read),
                s(6, Descriptor, Read),
                s(7, Journal, Read),
            ],
        ),
        Action::ScanCheckpoint => (
            3,
            vec![s(2, CompressedStateDomain::OracleCarryCheckpoint, Read)],
        ),
        Action::FreezeOpening => (
            13,
            vec![
                s(3, Source, Mutable),
                s(3, Descriptor, Read),
                s(7, Observations, Mutable),
                s(8, Sku, Read),
                s(10, Journal, MutableOrInitialize),
                s(11, Checkpoint, Initialize),
            ],
        ),
    };
    if account_count != expected
        || specs.len() > crate::constants::MAX_COMPRESSED_STATE_SESSION_RECORDS
    {
        return invalid();
    }
    Ok(RequiredAccesses::from_slice(&specs))
}

/// Integration entrypoint: invoke ONLY from the existing governed dispatcher,
/// passing its authenticated compressed-inner transport decision. No new bypass.
#[inline(never)]
pub(in crate::processor) fn process(
    program: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
    compressed_inner: bool,
) -> ProgramResult {
    let required = required_accesses(payload, accounts.len())?;
    if required.len() != 0 {
        require_compressed_state_transport(compressed_inner)?;
    }
    // The wrapper already verifies every logical read remains unchanged. All source
    // views are physically writable during materialization, including logical reads.
    match decode(payload)? {
        Action::Council(action) => super::oracle_council::process(program, accounts, action),
        Action::RegisterRoot => register_period(program, accounts, true),
        Action::BackfillSourceEvidence => {
            super::oracle_evidence::backfill_source(program, accounts)
        }
        Action::BackfillClaimEvidence { role } => {
            super::oracle_evidence::backfill_claim(program, accounts, role)
        }
        Action::WriteEvidence {
            kind,
            hash,
            total,
            offset,
            bytes,
        } => super::oracle_evidence::write(program, accounts, kind, hash, total, offset, &bytes),
        Action::CloseEvidenceDraft => super::oracle_evidence::close_draft(program, accounts),
        Action::RegisterSuccessor => register_period(program, accounts, false),
        Action::CaptureCurrent {
            kind,
            previous_hash,
        } => capture_current(program, accounts, kind, previous_hash),
        Action::Import { sku_index, proof } => import_source(program, accounts, sku_index, &proof),
        Action::SkipInactive => skip_inactive_source(program, accounts),
        Action::BeginSelection => begin_selection(program, accounts),
        Action::ScanCheckpoint => scan_checkpoint(program, accounts),
        Action::FreezeOpening => freeze_opening(program, accounts),
        Action::BeginHistoryMedian { mode, lower, upper } => {
            begin_history_median(program, accounts, mode, lower, upper)
        }
        Action::ScanHistoryMedian => scan_history_median(program, accounts),
        Action::CloseHistoryMedian => close_history_median(program, accounts),
        Action::BeginBucketRank {
            mode,
            lower,
            upper,
            nonce,
        } => begin_bucket_rank(program, accounts, mode, lower, upper, nonce),
        Action::ScanBucketRank => scan_bucket_rank(program, accounts),
        Action::CloseBucketRank => close_bucket_rank(program, accounts),
    }
}
