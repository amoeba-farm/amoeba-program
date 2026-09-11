//! Authenticated carry-forward behind the ordinary governed dispatcher.
use super::compressed_state::{RequiredAccessKind, RequiredAccessSpec, RequiredAccesses};
use super::*;
use crate::state::CompressedStateDomain;

mod checkpoints;
mod openings;
mod periods;
mod state;
use checkpoints::*;
use openings::*;
use periods::*;
use state::*;

pub(in crate::processor) use checkpoints::{record_fresh_accept, AcceptedEvent};
pub(in crate::processor) use openings::{
    inherited_reward_opening, require_carry_resolved_before_expiry,
};
pub(in crate::processor) use periods::{require_fresh_discovery, require_import_complete};

use crate::instruction::OracleCarryForwardActionV1 as Action;

fn decode(payload: &[u8]) -> Result<Action, ProgramError> {
    if payload.len() > MAX_INSTRUCTION_DATA_BYTES {
        return invalid();
    }
    <Action as borsh::BorshDeserialize>::try_from_slice(payload)
        .map_err(|_| VaultError::InvalidInstructionData.into())
}

/// Payload excludes the outer instruction tag and the governed epoch tail.
/// This exact contract is also encoded by the TypeScript account/proof plan.
pub(in crate::processor) fn required_accesses(
    payload: &[u8],
    account_count: usize,
) -> Result<RequiredAccesses, ProgramError> {
    use CompressedStateDomain::{
        OracleSourceDescriptor as Descriptor, OracleSourceState as Source,
        OracleUsdcSkuPool as Sku, OracleUsdcSourceReward as Reward,
    };
    use RequiredAccessKind::{Initialize, Mutable, ReadOnly as Read};
    let s = RequiredAccessSpec::new;
    let (expected, specs): (usize, Vec<RequiredAccessSpec>) = match decode(payload)? {
        Action::RegisterRoot => (7, vec![]),
        Action::RegisterSuccessor => (10, vec![]),
        Action::CaptureCurrent { kind, .. } => (
            if kind == 2 { 10 } else { 9 },
            vec![s(3, Source, Read), s(3, Descriptor, Read)],
        ),
        Action::Import { .. } => (
            18,
            vec![
                s(4, Sku, Read),
                s(5, Source, Initialize),
                s(5, Descriptor, Initialize),
                s(7, Reward, Initialize),
                s(11, Source, Read),
                s(11, Descriptor, Read),
            ],
        ),
        Action::SkipInactive => (9, vec![s(5, Source, Read), s(5, Descriptor, Read)]),
        Action::BeginSelection => (
            10,
            vec![
                s(2, Source, Read),
                s(2, Descriptor, Read),
                s(6, Source, Read),
                s(6, Descriptor, Read),
            ],
        ),
        Action::ScanCheckpoint => (3, vec![]),
        Action::FreezeOpening => (
            13,
            vec![
                s(3, Source, Mutable),
                s(3, Descriptor, Read),
                s(8, Sku, Read),
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
        Action::RegisterRoot => register_period(program, accounts, true),
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
    }
}
