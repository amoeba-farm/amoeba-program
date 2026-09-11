use super::*;

fn require_no_fresh_claim(
    program: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    claim_info: &AccountInfo,
) -> ProgramResult {
    let expected = derive_oracle_opening_claim_pda(program, &source.month, source_key).0;
    if claim_info.owner == &system_program::id() {
        return validate_canonical_system_zero_pda_proof(&expected, claim_info);
    }
    let claim =
        load_valid_oracle_opening_claim(program, &source.month, source_key, claim_info, source)?;
    if !matches!(
        claim.status,
        OracleOpeningClaimStatus::Rejected | OracleOpeningClaimStatus::TimedOut
    ) || claim.escrow_disposition == OracleEscrowDisposition::Unsettled
    {
        return invalid();
    }
    Ok(())
}

fn target_context(
    program: &Pubkey,
    market_info: &AccountInfo,
    month_info: &AccountInfo,
    source_info: &AccountInfo,
    carry_info: &AccountInfo,
    period_info: &AccountInfo,
) -> Result<(OracleMonthState, OracleSourceState, CarrySource), ProgramError> {
    let (market, month) = load_valid_market_and_oracle_month(program, market_info, month_info)?;
    let source = load_valid_oracle_source(program, month_info.key, source_info)?;
    let carry = load_carry(program, source_info.key, carry_info)?;
    let period = load_period(program, month_info.key, period_info)?;
    if period.next_import != period.expected_imports
        || period.predecessor != carry.parent_month
        || period.underlying != market.instrument.underlying_id
        || period.expiry != market.instrument.expiry_ts
        || carry.month != *month_info.key
        || carry.parent_source == *source_info.key
        || carry.definition_hash != definition_hash(&period.underlying, &source)
        || month.weight_scheme_version != 1
        || month.phase != OraclePhase::Opening
        || now()? < month.listing_ts
        || now()? >= market.instrument.expiry_ts
    {
        return invalid();
    }
    Ok((month, source, carry))
}

/// Accounts: target market/month/source/carry/period, canonical fresh claim,
/// parent source, its journal, parent month and rent payer. Source pairs are logical reads.
/// The latest journal head is captured AFTER the fresh opening deadline; new parent
/// checkpoints cannot then acquire an eligible earlier acceptance timestamp.
pub(super) fn begin_selection(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 10 {
        return invalid();
    }
    let (month, source, mut carry) = target_context(program, &a[0], &a[1], &a[2], &a[3], &a[4])?;
    if carry.status != IMPORTED {
        return invalid();
    }
    if source.status == OracleSourceStatus::Active {
        let fresh = load_valid_oracle_opening_claim(program, a[1].key, a[2].key, &a[5], &source)?;
        if fresh.status != OracleOpeningClaimStatus::Accepted
            || !source.opening_submitted
            || source.baseline_state != fresh.opening_state
            || source.opening_evidence_hash != fresh.evidence_hash
        {
            return invalid();
        }
        carry.status = FRESH_OPENING;
        return save(program, &a[3], &carry);
    }
    if source.status != OracleSourceStatus::Frozen
        || source.opening_submitted
        || source.observation_count != 0
        || source.support_stake_total == 0
        || source.listing_bond_locked == 0
    {
        return invalid();
    }
    require_no_fresh_claim(program, a[2].key, &source, &a[5])?;
    if a[6].key != &carry.parent_source || a[8].key != &carry.parent_month {
        return invalid();
    }
    let parent = load_valid_oracle_source(program, &carry.parent_month, &a[6])?;
    let period = load_period(program, a[1].key, &a[4])?;
    if definition_hash(&period.underlying, &parent) != carry.definition_hash {
        return invalid();
    }
    carry.cutoff = oracle_opening_start_ts(&month)?;
    carry.deadline = month.listing_ts;
    if carry.cutoff >= carry.deadline {
        return invalid();
    }
    let journal_pda = address(program, JOURNAL_SEED, a[6].key.as_ref());
    if a[7].owner == &system_program::id() {
        validate_canonical_system_zero_pda_proof(&journal_pda.0, &a[7])?;
        carry.status = NO_ELIGIBLE_CHECKPOINT;
    } else {
        let journal = load_journal(program, a[6].key, &a[7])?;
        // A missed acceptance is a hard error, not permission to select a stale head.
        if journal.observation_count != parent.observation_count
            || journal.rolling_observation_hash != parent.rolling_observation_hash
        {
            return invalid();
        }
        carry.cursor = journal.head;
        carry.remaining = journal.count;
        carry.status = SELECTING;
    }
    save(program, &a[3], &carry)
}

/// Accounts: target source identity (no source contents read), carry, next immutable checkpoint.
/// Exactly one backwards step. Zero remaining is the ONLY completion condition.
pub(super) fn scan_checkpoint(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 3 {
        return invalid();
    }
    let mut carry = load_carry(program, a[0].key, &a[1])?;
    if carry.status != SELECTING || carry.remaining == 0 || carry.cursor != *a[2].key {
        return invalid();
    }
    let checkpoint = load_checkpoint(program, &a[2], &carry.parent_source)?;
    if checkpoint.month != carry.parent_month || checkpoint.sequence != carry.remaining {
        return invalid();
    }
    if checkpoint.observed_at <= carry.cutoff
        && checkpoint.accepted_at < carry.deadline
        && (carry.selected_checkpoint == Pubkey::default()
            || (checkpoint.observed_at, checkpoint.sequence)
                > (carry.observed_at, carry.selected_sequence))
    {
        carry.selected_checkpoint = *a[2].key;
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
    save(program, &a[1], &carry)
}

/// Accounts: payer, market/month/source/carry/period, canonical fresh claim,
/// native observations, target SKU, target reward schedule, journal/checkpoint/System.
/// Checkpoint event identity is the create-once carry companion. No reward claim is created.
pub(super) fn freeze_opening(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 13 {
        return invalid();
    }
    let (mut month, mut source, mut carry) =
        target_context(program, &a[1], &a[2], &a[3], &a[4], &a[5])?;
    if carry.status != SELECTING
        || carry.remaining != 0
        || carry.cursor != Pubkey::default()
        || carry.selected_checkpoint == Pubkey::default()
        || carry.value == 0
        || carry.cutoff != oracle_opening_start_ts(&month)?
        || carry.deadline != month.listing_ts
        || carry.observed_at > carry.cutoff
        || carry.accepted_at >= carry.deadline
        || source.status != OracleSourceStatus::Frozen
        || source.opening_submitted
        || source.observation_count != 0
        || source.support_stake_total == 0
        || source.listing_bond_locked == 0
    {
        return invalid();
    }
    require_no_fresh_claim(program, a[3].key, &source, &a[6])?;
    let schedule = oracle_usdc::load_oracle_usdc_reward_schedule(program, a[2].key, &a[9])?;
    let sku = oracle_usdc::load_oracle_usdc_sku_pool(program, a[9].key, a[2].key, &a[8])?;
    if schedule.phase != crate::state::OracleUsdcRewardSchedulePhase::Funded
        || sku.bucket_id != source.bucket_id
        || sku.listing_bond != source.listing_bond_locked
    {
        return invalid();
    }
    let mut observations =
        load_valid_oracle_source_observations(program, a[2].key, a[3].key, &a[7])?;
    if observations.states.iter().any(|value| *value != 0)
        || observations.source_times.iter().any(|value| *value != 0)
    {
        return invalid();
    }
    // Version 2's first slot is a standing-state anchor, NEVER a median sample.
    // Only one anchor is cached locally; immutable checkpoint references retain history.
    observations.account_version = OracleSourceObservations::INHERITED_ANCHOR_VERSION;
    observations.states[0] = carry.value;
    observations.source_times[0] = carry.observed_at;
    source.baseline_state = carry.value;
    source.current_state = carry.value;
    source.observation_count = 1;
    source.opening_submitted = true;
    source.opening_evidence_hash = hashv(&[
        b"oracle-carried-opening-v1",
        a[2].key.as_ref(),
        a[3].key.as_ref(),
        carry.parent_source.as_ref(),
        carry.selected_checkpoint.as_ref(),
        &carry.definition_hash,
        &carry.cutoff.to_le_bytes(),
        &carry.deadline.to_le_bytes(),
        &carry.value.to_le_bytes(),
        &carry.observed_at.to_le_bytes(),
        &carry.evidence_hash,
        &carry.archive_hash,
    ])
    .to_bytes();
    source.rolling_observation_hash = hashv(&[
        b"oracle-carried-anchor-v1",
        &source.opening_evidence_hash,
        carry.origin_checkpoint.as_ref(),
        carry.contributor.as_ref(),
    ])
    .to_bytes();
    source.status = OracleSourceStatus::Active;
    month.opened_source_count = month
        .opened_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.opening_resolved_source_count = month
        .opening_resolved_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if month.opened_source_count > month.frozen_source_count
        || month.opening_resolved_source_count > month.frozen_source_count
    {
        return invalid();
    }
    month.last_updated_slot = Clock::get()?.slot;
    append_checkpoint(
        program,
        &a[0],
        a[3].key,
        &source,
        &observations,
        AcceptedEvent {
            event: *a[4].key,
            value: carry.value,
            observed_at: carry.observed_at,
            evidence_hash: carry.evidence_hash,
            archive_hash: carry.archive_hash,
            contributor: carry.contributor,
        },
        &a[10..],
        None,
        Some((carry.origin_source, carry.origin_checkpoint)),
    )?;
    carry.status = FROZEN_CARRY;
    save(program, &a[4], &carry)?;
    store_state(&a[3], &source)?;
    store_state(&a[7], &*observations)?;
    store_oracle_month_state(&a[2], &month)
}

/// Mandatory expiry hook. A permissionless complete scan can prove no eligible
/// checkpoint; a caller cannot preempt that work by expiring an imported source.
/// At immutable contract expiry, ordinary failed-period cleanup must still be possible.
pub(in crate::processor) fn require_carry_resolved_before_expiry(
    program: &Pubkey,
    source: &Pubkey,
    companion: &AccountInfo,
    contract_expiry: u64,
) -> ProgramResult {
    if companion.owner == &system_program::id() {
        return validate_canonical_system_zero_pda_proof(
            &address(program, SOURCE_SEED, source.as_ref()).0,
            companion,
        );
    }
    let carry = load_carry(program, source, companion)?;
    if carry.status != NO_ELIGIBLE_CHECKPOINT && now()? < contract_expiry {
        return invalid();
    }
    Ok(())
}

/// Reward registration consumes this proof instead of inventing an accepted opening
/// claim. Count the source as resolved/active, but issue no opening/discovery payout right.
pub(in crate::processor) fn validate_inherited_reward_registration(
    program: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    info: &AccountInfo,
) -> ProgramResult {
    let carry = load_carry(program, source_key, info)?;
    if carry.status != FROZEN_CARRY
        || carry.month != source.month
        || source.status != OracleSourceStatus::Active
        || source.baseline_state != carry.value
        || !source.opening_submitted
        || carry.selected_checkpoint == Pubkey::default()
    {
        return invalid();
    }
    Ok(())
}

/// Every reward-source registration proves either canonical absence or the exact
/// companion. Only a frozen inherited opening can replace a fresh accepted claim.
pub(in crate::processor) fn inherited_reward_opening(
    program: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    info: &AccountInfo,
) -> Result<bool, ProgramError> {
    if info.owner == &system_program::id() {
        validate_canonical_system_zero_pda_proof(
            &address(program, SOURCE_SEED, source_key.as_ref()).0,
            info,
        )?;
        return Ok(false);
    }
    let carry = load_carry(program, source_key, info)?;
    if carry.month != source.month {
        return invalid();
    }
    if carry.status != FROZEN_CARRY {
        return Ok(false);
    }
    validate_inherited_reward_registration(program, source_key, source, info)?;
    Ok(true)
}
