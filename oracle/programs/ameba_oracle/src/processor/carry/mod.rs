//! Authenticated carry-forward behind the ordinary dispatcher. Selectors 0-7 register carry
//! periods, import inherited sources and select their openings from immutable checkpoints; the
//! history-median (8-10) and bucket-rank (11-13) selectors build bounded median certificates; the
//! evidence selectors (14-17) publish and recover immutable public preimages and change no
//! economic state.
use super::*;
use borsh::BorshDeserialize;

mod bucket_rank;
mod checkpoints;
mod history;
mod openings;
mod periods;
mod reads;
mod scan_transition;
mod state;
pub(in crate::processor) use bucket_rank::verified_bucket_rank;
use bucket_rank::*;
use checkpoints::*;
pub(in crate::processor) use checkpoints::{record_fresh_accept, AcceptedEvent};
pub(in crate::processor) use history::verified_history_median;
use history::*;
pub use history::{history_median_address, history_median_cursor};
use openings::*;
pub(in crate::processor) use openings::{
    inherited_reward_opening, require_carry_resolved_before_expiry,
};
use periods::*;
pub(in crate::processor) use periods::{
    require_cleanup_dependencies, require_fresh_discovery, require_import_complete,
};
pub use reads::{read_source_checkpoint, read_source_journal, CheckpointView, JournalView};
use scan_transition::*;
use state::*;

use crate::instruction::OracleCarryForwardActionV1 as Action;

fn decode(payload: &[u8]) -> Result<Action, ProgramError> {
    if payload.len() > MAX_INSTRUCTION_DATA_BYTES {
        return invalid();
    }
    <Action as borsh::BorshDeserialize>::try_from_slice(payload)
        .map_err(|_| OracleError::InvalidInstructionData.into())
}

/// Integration entrypoint: invoke only from the ordinary dispatcher.
#[inline(never)]
pub(in crate::processor) fn process(
    program: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    match decode(payload)? {
        Action::Council(action) => super::council::process(program, accounts, action),
        Action::SponsoredSource(action) => super::sponsorship::process(program, accounts, action),
        Action::TerminalCleanup(params) => {
            super::terminal_cleanup::process(program, accounts, params)
        }
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
    }
}

#[cfg(test)]
mod unassigned_selectors {
    use super::*;
    use borsh::BorshDeserialize;

    #[test]
    fn setup_selectors_outside_the_surface_reject_before_accounts() {
        for (selector, magic, payload_len) in [
            (19, *b"CFM1", 1),
            (20, *b"CFM1", 9),
            (25, *b"OCL1", 309),
            (26, *b"OCE1", 277),
        ] {
            let mut complete = vec![selector];
            complete.extend_from_slice(&magic);
            complete.resize(5 + payload_len, 1);
            for payload in [vec![selector], complete.clone(), {
                let mut trailing = complete;
                trailing.push(0);
                trailing
            }] {
                assert!(Action::try_from_slice(&payload).is_err());
                assert!(Action::deserialize_reader(&mut payload.as_slice()).is_err());
                assert_eq!(
                    process(&Pubkey::new_unique(), &[], &payload),
                    Err(OracleError::InvalidInstructionData.into())
                );
            }
        }
        assert_eq!(Action::try_from_slice(&[0]).unwrap(), Action::RegisterRoot);
        assert_eq!(
            Action::try_from_slice(&[1]).unwrap(),
            Action::RegisterSuccessor
        );
        assert_eq!(
            Action::try_from_slice(&[15]).unwrap(),
            Action::CloseEvidenceDraft
        );
        assert_eq!(
            Action::try_from_slice(&[16]).unwrap(),
            Action::BackfillSourceEvidence
        );
    }
}
