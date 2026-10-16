//! The typed ScanCheckpoint transition: one backwards step over an authenticated checkpoint.
use super::*;

/// The pre-load portion of ScanCheckpoint. It stays before checkpoint loading so a bad carry
/// cursor fails before any checkpoint bytes are decoded.
pub(super) fn validate_scan_cursor(carry: &CarrySource, checkpoint_key: &Pubkey) -> ProgramResult {
    if carry.status != SELECTING || carry.remaining == 0 || carry.cursor != *checkpoint_key {
        return invalid();
    }
    Ok(())
}

/// Apply the state transition after the checkpoint has been authenticated: month/sequence checks
/// precede eligibility, the cursor is always consumed, and zero remaining is the only completion
/// condition.
pub(super) fn apply_scan_transition(
    carry: &mut CarrySource,
    checkpoint_key: &Pubkey,
    checkpoint: &Checkpoint,
) -> ProgramResult {
    if checkpoint.month != carry.parent_month || checkpoint.sequence != carry.remaining {
        return invalid();
    }
    // Validate the terminal predecessor shape before touching the carry value, so a failure
    // leaves no partial in-memory transition.
    if (carry.remaining == 1) != (crate::pubkey_is_default(&checkpoint.previous)) {
        return invalid();
    }
    if checkpoint.observed_at <= carry.cutoff
        && checkpoint.accepted_at < carry.deadline
        && (crate::pubkey_is_default(&carry.selected_checkpoint)
            || (checkpoint.observed_at, checkpoint.sequence)
                > (carry.observed_at, carry.selected_sequence))
    {
        carry.selected_checkpoint = *checkpoint_key;
        carry.selected_sequence = checkpoint.sequence;
        carry.value = checkpoint.value;
        carry.observed_at = checkpoint.observed_at;
        carry.accepted_at = checkpoint.accepted_at;
        carry.evidence_hash = checkpoint.evidence_hash;
        carry.archive_hash = checkpoint.archive_hash;
        carry.contributor = checkpoint.contributor;
        carry.origin_source = checkpoint.origin_source;
        carry.origin_checkpoint = checkpoint.origin_checkpoint;
    }
    carry.cursor = checkpoint.previous;
    carry.remaining = carry
        .remaining
        .checked_sub(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    if (carry.remaining == 0) != (crate::pubkey_is_default(&carry.cursor)) {
        return invalid();
    }
    if carry.remaining == 0 && crate::pubkey_is_default(&carry.selected_checkpoint) {
        carry.status = NO_ELIGIBLE_CHECKPOINT;
    }
    Ok(())
}
