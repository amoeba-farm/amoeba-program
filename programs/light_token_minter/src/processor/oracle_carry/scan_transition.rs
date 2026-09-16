//! Typed ScanCheckpoint transition shared by the classic loader and a future
//! authenticated read-only compressed adapter.
//!
//! This module deliberately stops at the typed transition boundary.  The
//! compressed-state wrapper still owns proof, domain, revision, context, and
//! leaf authentication.  No AccountInfo is fabricated here and no caller
//! supplied value is trusted without the wrapper's authentication contract.
use super::*;

/// The pre-load portion of ScanCheckpoint.  It must remain before checkpoint
/// loading so a bad carry cursor has the same error precedence as the classic
/// AccountInfo path.
pub(super) fn validate_scan_cursor(carry: &CarrySource, checkpoint_key: &Pubkey) -> ProgramResult {
    if carry.status != SELECTING || carry.remaining == 0 || carry.cursor != *checkpoint_key {
        return invalid();
    }
    Ok(())
}

/// Apply the state transition after the checkpoint has been authenticated.
/// This contains the old typed business kernel byte-for-byte in behavior:
/// month/sequence checks precede eligibility, the cursor is always consumed,
/// and zero remaining is the only completion condition.
pub(super) fn apply_scan_transition(
    carry: &mut CarrySource,
    checkpoint_key: &Pubkey,
    checkpoint: &Checkpoint,
) -> ProgramResult {
    if checkpoint.month != carry.parent_month || checkpoint.sequence != carry.remaining {
        return invalid();
    }
    // Validate the terminal predecessor shape before touching the typed carry
    // value.  The classic path used to discover this after local mutation and
    // before save; prechecking preserves the same error while making the
    // storage-neutral adapter fail without an in-memory partial transition.
    if (carry.remaining == 1) != (checkpoint.previous == Pubkey::default()) {
        return invalid();
    }
    if checkpoint.observed_at <= carry.cutoff
        && checkpoint.accepted_at < carry.deadline
        && (carry.selected_checkpoint == Pubkey::default()
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
        .ok_or(VaultError::ArithmeticOverflow)?;
    if (carry.remaining == 0) != (carry.cursor == Pubkey::default()) {
        return invalid();
    }
    if carry.remaining == 0 && carry.selected_checkpoint == Pubkey::default() {
        carry.status = NO_ELIGIBLE_CHECKPOINT;
    }
    Ok(())
}

/// A typed checkpoint read after the compressed wrapper has authenticated its
/// domain-13 leaf, context, revision, canonical address, and exact record
/// encoding.  The adapter never constructs an AccountInfo or writes the leaf.
pub(super) struct AuthenticatedCheckpointRead {
    pub(super) canonical_key: Pubkey,
    pub(super) record: Checkpoint,
}

/// Re-check the immutable checkpoint identity at the typed boundary.  These
/// checks mirror `load_checkpoint` after its physical AccountInfo checks; the
/// wrapper must perform the physical compact-leaf checks before constructing
/// this value.
pub(super) fn validate_authenticated_checkpoint(
    program: &Pubkey,
    carry: &CarrySource,
    read: &AuthenticatedCheckpointRead,
) -> ProgramResult {
    let expected = checkpoint_address(program, &carry.parent_source, &read.record.event);
    if read.canonical_key != expected.0
        || !header_matches(&read.record, expected.1)
        || read.record.source != carry.parent_source
        || read.record.sequence == 0
        || read.record.value == 0
        || read.record.observed_at == 0
        || read.record.accepted_at < read.record.observed_at
        || read.record.evidence_hash == [0; 32]
        || read.record.archive_hash == [0; 32]
        || read.record.contributor == Pubkey::default()
        || read.record.origin_source == Pubkey::default()
        || read.record.origin_checkpoint == Pubkey::default()
        || (read.record.sequence == 1) != (read.record.previous == Pubkey::default())
    {
        return invalid();
    }
    Ok(())
}

/// Authenticated read-only adapter skeleton.  A future domain-13 route should
/// call this only after the generic wrapper has verified proof, session shape,
/// context binding, leaf revision/length, and the canonical checkpoint PDA.
/// It returns before persistence so the caller can save the existing mutable
/// CarrySource through the normal typed path and retain transaction atomicity.
pub(super) fn apply_authenticated_checkpoint_read(
    program: &Pubkey,
    carry: &mut CarrySource,
    read: AuthenticatedCheckpointRead,
) -> ProgramResult {
    // Preserve ScanCheckpoint's old precedence: carry state/cursor first,
    // authenticated checkpoint contents second, business relation third.
    validate_scan_cursor(carry, &read.canonical_key)?;
    validate_authenticated_checkpoint(program, carry, &read)?;
    apply_scan_transition(carry, &read.canonical_key, &read.record)
}
