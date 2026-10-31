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
#[cfg(feature = "v1-formation")]
use bucket_rank::*;
use checkpoints::*;
pub(in crate::processor) use checkpoints::{record_fresh_accept, AcceptedEvent};
pub(in crate::processor) use history::verified_history_median;
#[cfg(feature = "v1-formation")]
use history::*;
pub use history::{history_median_address, history_median_cursor};
#[cfg(feature = "v1-formation")]
use openings::*;
pub(in crate::processor) use openings::{
    inherited_reward_opening, require_carry_resolved_before_expiry,
};
use periods::*;
pub(in crate::processor) use periods::{
    is_latest_carry_month, require_cleanup_dependencies, require_fresh_discovery,
    require_import_complete,
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
    // Without the v1 formation its selectors reject like an unassigned selector, undecoded.
    #[cfg(not(feature = "v1-formation"))]
    if payload.first().is_some_and(|selector| {
        crate::governance_manifest::is_v1_formation_carry_selector(*selector)
    }) {
        return Err(OracleError::InvalidInstructionData.into());
    }
    match decode(payload)? {
        Action::Council(action) => super::council::process(program, accounts, action),
        Action::SponsoredSource(action) => super::sponsorship::process(program, accounts, action),
        Action::TerminalCleanup(params) => {
            super::terminal_cleanup::process(program, accounts, params)
        }
        Action::FailedMonth { kind } => super::failed_month::process(program, accounts, kind),
        #[cfg(feature = "v1-formation")]
        Action::RegisterRoot => register_period(program, accounts, true),
        #[cfg(feature = "v1-formation")]
        Action::RegisterSuccessor => register_period(program, accounts, false),
        #[cfg(feature = "v1-formation")]
        Action::CaptureCurrent {
            kind,
            previous_hash,
        } => capture_current(program, accounts, kind, previous_hash),
        #[cfg(feature = "v1-formation")]
        Action::Import { sku_index, proof } => import_source(program, accounts, sku_index, &proof),
        #[cfg(feature = "v1-formation")]
        Action::SkipInactive => skip_inactive_source(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::BeginSelection => begin_selection(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::ScanCheckpoint => scan_checkpoint(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::FreezeOpening => freeze_opening(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::BeginHistoryMedian { mode, lower, upper } => {
            begin_history_median(program, accounts, mode, lower, upper)
        }
        #[cfg(feature = "v1-formation")]
        Action::ScanHistoryMedian => scan_history_median(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::CloseHistoryMedian => close_history_median(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::BeginBucketRank {
            mode,
            lower,
            upper,
            nonce,
        } => begin_bucket_rank(program, accounts, mode, lower, upper, nonce),
        #[cfg(feature = "v1-formation")]
        Action::ScanBucketRank => scan_bucket_rank(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::CloseBucketRank => close_bucket_rank(program, accounts),
        #[cfg(feature = "v1-formation")]
        Action::BackfillSourceEvidence => {
            super::oracle_evidence::backfill_source(program, accounts)
        }
        #[cfg(feature = "v1-formation")]
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
        // Checked above, before decoding.
        #[cfg(not(feature = "v1-formation"))]
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[cfg(test)]
mod v1_formation_selectors {
    use super::*;
    use crate::governance_manifest::{
        is_v1_formation_carry_selector, V1_FORMATION_CARRY_SELECTORS,
    };
    use borsh::BorshSerialize;

    /// A build without the v1 formation refuses its selectors (complete payloads included) like
    /// an unassigned one, before any account; the selectors a Live month uses still reach their
    /// handlers, which refuse an empty account list by their own checks.
    #[test]
    fn formation_selectors_route_only_with_v1_formation() {
        let program = Pubkey::new_unique();
        let invalid_data = Err(OracleError::InvalidInstructionData.into());
        let formation = [
            Action::RegisterRoot,
            Action::RegisterSuccessor,
            Action::CaptureCurrent {
                kind: 1,
                previous_hash: [2; 32],
            },
            Action::Import {
                sku_index: 3,
                proof: vec![[4; 32]],
            },
            Action::SkipInactive,
            Action::BeginSelection,
            Action::ScanCheckpoint,
            Action::FreezeOpening,
            Action::BeginHistoryMedian {
                mode: 0,
                lower: 1,
                upper: 1,
            },
            Action::ScanHistoryMedian,
            Action::CloseHistoryMedian,
            Action::BeginBucketRank {
                mode: 0,
                lower: 1,
                upper: 1,
                nonce: [5; 32],
            },
            Action::ScanBucketRank,
            Action::CloseBucketRank,
            Action::BackfillSourceEvidence,
            Action::BackfillClaimEvidence { role: 2 },
        ];
        let mut selectors: Vec<u8> = Vec::new();
        for action in formation {
            let payload = action.try_to_vec().unwrap();
            assert!(is_v1_formation_carry_selector(payload[0]));
            selectors.push(payload[0]);
            let result = process(&program, &[], &payload);
            if cfg!(feature = "v1-formation") {
                assert_ne!(result, invalid_data, "{action:?}");
            } else {
                assert_eq!(result, invalid_data, "{action:?}");
            }
        }
        assert_eq!(selectors, V1_FORMATION_CARRY_SELECTORS);
        // The evidence writer (14, 15) stays in every build.
        for action in [
            Action::WriteEvidence {
                kind: 1,
                hash: [1; 32],
                total: 1,
                offset: 0,
                bytes: vec![0],
            },
            Action::CloseEvidenceDraft,
        ] {
            let payload = action.try_to_vec().unwrap();
            assert!(!is_v1_formation_carry_selector(payload[0]));
            assert_ne!(process(&program, &[], &payload), invalid_data, "{action:?}");
        }
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
