use super::*;

pub(in crate::processor) fn settle_cash_listing_bond_loaded(
    program_id: &Pubkey,
    source_info: &AccountInfo,
    proposer_collateral_info: &AccountInfo,
    mut source: OracleSourceState,
) -> ProgramResult {
    if source.listing_bond_locked == 0 {
        return Err(VaultError::OracleEscrowAlreadySettled.into());
    }
    if !matches!(
        source.status,
        OracleSourceStatus::Active | OracleSourceStatus::Inactive | OracleSourceStatus::TimedOut
    ) {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let mut proposer_collateral =
        load_canonical_user_collateral(program_id, proposer_collateral_info, &source.proposer)?;
    let slot = Clock::get()?.slot;
    credit_cash_collateral_at_slot(&mut proposer_collateral, source.listing_bond_locked, slot)?;
    source.listing_bond_locked = 0;
    store_state(source_info, &source)?;
    store_state(proposer_collateral_info, &proposer_collateral)
}

pub(in crate::processor) fn settle_counted_v5_source_challenge(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    require_failed: bool,
) -> ProgramResult {
    // cranker, market, month, schedule, source, source reward, challenge, proposer collateral,
    // challenger collateral, primary guard, and optional comparison guard.
    if !(10..=11).contains(&accounts.len()) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let month_info = &accounts[2];
    let schedule_info = &accounts[3];
    let source_info = &accounts[4];
    let source_reward_info = &accounts[5];
    let challenge_info = &accounts[6];
    let proposer_collateral_info = &accounts[7];
    let challenger_collateral_info = &accounts[8];
    let guard_accounts = &accounts[9..];

    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut challenge =
        load_valid_oracle_source_challenge(program_id, month_info.key, challenge_info)?;
    if challenge.failed_schedule_escrow_counted {
        return Err(VaultError::OracleEscrowAlreadySettled.into());
    }
    let listing_settles =
        challenge.status == OracleChallengeStatus::Accepted && source.listing_bond_locked > 0;
    let required_count = if listing_settles { 2 } else { 1 };
    let (mut schedule, mut month) =
        load_counted_v5_schedule(program_id, accounts, required_count, require_failed)?;
    let mut source_reward = super::oracle_usdc::load_oracle_usdc_source_reward(
        program_id,
        schedule_info.key,
        month_info.key,
        source_info.key,
        source_reward_info,
    )?;
    if listing_settles && source_reward.listing_escrow_counted {
        return Err(VaultError::OracleEscrowAlreadySettled.into());
    }

    require_signed_cranker(&accounts[0])?;
    if challenge.source != *source_info.key
        || challenge.source_id != source.source_id
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
        && !crate::bytes32_is_zero(&challenge.comparison_source_id);
    if guard_accounts.len() != if has_comparison { 2 } else { 1 } {
        return Err(VaultError::InvalidAccountList.into());
    }
    let source_guard_info = &guard_accounts[0];
    if !source_guard_info.is_writable
        || source_guard_info.key == source_info.key
        || source_guard_info.key == challenge_info.key
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let mut source_guard = load_canonical_source_challenge_guard(
        program_id,
        month_info.key,
        source_info.key,
        &source.source_id,
        source_guard_info,
    )?;
    let mut comparison_guard = if has_comparison {
        let comparison_guard_info = &guard_accounts[1];
        if !comparison_guard_info.is_writable
            || comparison_guard_info.key == source_guard_info.key
            || comparison_guard_info.key == challenge_info.key
            || comparison_guard_info.key == source_info.key
        {
            return Err(VaultError::InvalidOracleChallengeAccount.into());
        }
        Some((
            load_canonical_source_challenge_guard(
                program_id,
                month_info.key,
                &challenge.comparison_source,
                &challenge.comparison_source_id,
                comparison_guard_info,
            )?,
            comparison_guard_info,
        ))
    } else {
        None
    };
    let mut proposer_collateral =
        load_canonical_user_collateral(program_id, proposer_collateral_info, &source.proposer)?;
    let mut challenger_collateral = load_canonical_user_collateral(
        program_id,
        challenger_collateral_info,
        &challenge.challenger,
    )?;
    let clock = Clock::get()?;
    let slot = clock.slot;
    let now = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(VaultError::InvalidSettlementRecord))?;
    let mut timed_out = false;
    match challenge.status {
        OracleChallengeStatus::Accepted => {
            if !matches!(
                source.status,
                OracleSourceStatus::Rejected | OracleSourceStatus::Merged
            ) || source.listing_bond_locked == 0
            {
                return Err(VaultError::OracleEscrowNotSettleable.into());
            }
            let pot = source
                .listing_bond_locked
                .checked_add(challenge.bond)
                .ok_or(VaultError::ArithmeticOverflow)?;
            credit_cash_collateral_at_slot(&mut challenger_collateral, pot, slot)?;
            source.listing_bond_locked = 0;
            challenge.escrow_disposition = OracleEscrowDisposition::Refunded;
        }
        OracleChallengeStatus::Rejected => {
            credit_cash_collateral_at_slot(&mut proposer_collateral, challenge.bond, slot)?;
            challenge.escrow_disposition = OracleEscrowDisposition::Slashed;
        }
        OracleChallengeStatus::Cancelled => {
            credit_cash_collateral_at_slot(&mut challenger_collateral, challenge.bond, slot)?;
            challenge.escrow_disposition = OracleEscrowDisposition::Refunded;
        }
        OracleChallengeStatus::Open | OracleChallengeStatus::RuleReview => {
            if now < oracle_opening_start_ts(&month)? {
                return Err(VaultError::OracleEscrowNotSettleable.into());
            }
            credit_cash_collateral_at_slot(&mut proposer_collateral, challenge.bond, slot)?;
            challenge.status = OracleChallengeStatus::Rejected;
            challenge.escrow_disposition = OracleEscrowDisposition::Slashed;
            decrement_pending_oracle_resolution(&mut month)?;
            month.last_updated_slot = slot;
            timed_out = true;
        }
        OracleChallengeStatus::RuleReviewUnresolved => {
            return Err(VaultError::OracleEscrowNotSettleable.into())
        }
    }
    challenge.failed_schedule_escrow_counted = true;
    if timed_out {
        release_source_challenge_guard(
            &mut source_guard,
            challenge_info.key,
            &challenge.challenge_id,
            &Pubkey::default(),
            slot,
        )?;
        store_state(source_guard_info, &source_guard)?;
        if let Some((mut guard, guard_info)) = comparison_guard.take() {
            release_source_challenge_guard(
                &mut guard,
                challenge_info.key,
                &challenge.challenge_id,
                &Pubkey::default(),
                slot,
            )?;
            store_state(guard_info, &guard)?;
        }
    }
    if listing_settles {
        source_reward.listing_escrow_counted = true;
        source_reward.last_updated_slot = slot;
    }
    decrement_counted_v5_schedule_at(&mut schedule, required_count, slot)?;
    store_state(source_info, &source)?;
    store_state(challenge_info, &challenge)?;
    store_state(proposer_collateral_info, &proposer_collateral)?;
    store_state(challenger_collateral_info, &challenger_collateral)?;
    store_oracle_month_state(month_info, &month)?;
    if listing_settles {
        store_state(source_reward_info, &source_reward)?;
    }
    store_state(schedule_info, &schedule)
}

pub(in crate::processor) fn settle_cash_opening_claim(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 6 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let claim_info = &accounts[4];
    let claimant_collateral_info = &accounts[5];
    require_signed_cranker(cranker_info)?;
    let (_market, _month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut claim = load_valid_oracle_opening_claim(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
        &source,
    )?;
    if claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.stake == 0
        || !matches!(
            claim.status,
            OracleOpeningClaimStatus::Accepted | OracleOpeningClaimStatus::TimedOut
        )
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let mut collateral =
        load_canonical_user_collateral(program_id, claimant_collateral_info, &claim.claimant)?;
    let slot = Clock::get()?.slot;
    credit_cash_collateral_at_slot(&mut collateral, claim.stake, slot)?;
    claim.escrow_disposition = OracleEscrowDisposition::Refunded;
    store_state(claim_info, &claim)?;
    store_state(claimant_collateral_info, &collateral)
}

pub(in crate::processor) fn settle_cash_opening_challenge(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let claim_info = &accounts[4];
    let challenge_info = &accounts[5];
    let claimant_collateral_info = &accounts[6];
    let challenger_collateral_info = &accounts[7];
    require_signed_cranker(cranker_info)?;
    let (_market, _month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut claim = load_valid_oracle_opening_claim(
        program_id,
        month_info.key,
        source_info.key,
        claim_info,
        &source,
    )?;
    let mut challenge =
        load_valid_oracle_opening_challenge(program_id, month_info.key, challenge_info)?;
    if challenge.claim != *claim_info.key
        || challenge.claim_attempt != claim.attempt
        || challenge.source != *source_info.key
        || challenge.source_id != source.source_id
        || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.stake == 0
        || challenge.bond == 0
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let original_claimant = claim.claimant;
    let mut claimant_collateral =
        load_canonical_user_collateral(program_id, claimant_collateral_info, &original_claimant)?;
    let mut challenger_collateral = load_canonical_user_collateral(
        program_id,
        challenger_collateral_info,
        &challenge.challenger,
    )?;
    let slot = Clock::get()?.slot;
    match (challenge.status, claim.status) {
        (OracleChallengeStatus::Rejected, OracleOpeningClaimStatus::Challenged) => {
            credit_cash_collateral_at_slot(&mut claimant_collateral, challenge.bond, slot)?;
            challenge.escrow_disposition = OracleEscrowDisposition::Slashed;
            claim.status = OracleOpeningClaimStatus::Pending;
            claim.challenge_deadline_slot = slot
                .checked_add(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        (OracleChallengeStatus::Accepted, OracleOpeningClaimStatus::Challenged) => {
            if source.status != OracleSourceStatus::OpeningPending {
                return Err(VaultError::OracleEscrowNotSettleable.into());
            }
            credit_cash_collateral_at_slot(&mut challenger_collateral, claim.stake, slot)?;
            claim.claimant = challenge.challenger;
            claim.opening_state = challenge.alternative_opening_state;
            claim.source_time = challenge.alternative_source_time;
            claim.stake = challenge.bond;
            claim.canonical_locator_hash = challenge.canonical_locator_hash;
            claim.source_definition_hash = challenge.source_definition_hash;
            claim.evidence_hash = challenge.evidence_hash;
            claim.archive_url_hash = challenge.archive_url_hash;
            claim.submitted_slot = slot;
            claim.challenge_deadline_slot = slot
                .checked_add(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS)
                .ok_or(VaultError::ArithmeticOverflow)?;
            claim.status = OracleOpeningClaimStatus::Pending;
            challenge.escrow_disposition = OracleEscrowDisposition::Transferred;
        }
        (OracleChallengeStatus::Accepted, OracleOpeningClaimStatus::Rejected) => {
            let pot = claim
                .stake
                .checked_add(challenge.bond)
                .ok_or(VaultError::ArithmeticOverflow)?;
            credit_cash_collateral_at_slot(&mut challenger_collateral, pot, slot)?;
            claim.escrow_disposition = OracleEscrowDisposition::Slashed;
            challenge.escrow_disposition = OracleEscrowDisposition::Refunded;
        }
        (OracleChallengeStatus::Cancelled, OracleOpeningClaimStatus::TimedOut) => {
            credit_cash_collateral_at_slot(&mut claimant_collateral, claim.stake, slot)?;
            credit_cash_collateral_at_slot(&mut challenger_collateral, challenge.bond, slot)?;
            claim.escrow_disposition = OracleEscrowDisposition::Refunded;
            challenge.escrow_disposition = OracleEscrowDisposition::Refunded;
        }
        _ => return Err(VaultError::OracleEscrowNotSettleable.into()),
    }
    store_state(source_info, &source)?;
    store_state(claim_info, &claim)?;
    store_state(challenge_info, &challenge)?;
    store_state(claimant_collateral_info, &claimant_collateral)?;
    store_state(challenger_collateral_info, &challenger_collateral)
}

pub(in crate::processor) fn settle_cash_update_claim(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let claim_info = &accounts[4];
    let claimant_collateral_info = &accounts[5];
    let challenge_guard_proof_info = &accounts[6];
    require_signed_cranker(cranker_info)?;
    let (_market, _month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut claim = load_valid_oracle_update_claim(program_id, month_info.key, claim_info)?;
    let (expected_guard, _) =
        derive_oracle_update_challenge_guard_pda(program_id, month_info.key, claim_info.key);
    validate_canonical_system_zero_pda_proof(&expected_guard, challenge_guard_proof_info)
        .map_err(|_| ProgramError::from(VaultError::InvalidOracleUpdateAccount))?;
    if claim.source != *source_info.key
        || claim.source_id != source.source_id
        || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.stake == 0
        || oracle_update_claim_samba_checkpoint_active(program_id, claim_info)?
        || !matches!(
            claim.status,
            OracleClaimStatus::Finalized | OracleClaimStatus::TimedOut
        )
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let mut collateral =
        load_canonical_user_collateral(program_id, claimant_collateral_info, &claim.claimant)?;
    let slot = Clock::get()?.slot;
    credit_cash_collateral_at_slot(&mut collateral, claim.stake, slot)?;
    claim.escrow_disposition = OracleEscrowDisposition::Refunded;
    store_oracle_update_claim(claim_info, &claim)?;
    store_state(claimant_collateral_info, &collateral)
}

pub(in crate::processor) fn settle_cash_update_challenge(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let claim_info = &accounts[3];
    let challenge_info = &accounts[4];
    let claimant_collateral_info = &accounts[5];
    let challenger_collateral_info = &accounts[6];
    let challenge_guard_info = &accounts[7];
    require_signed_cranker(cranker_info)?;
    let (_market, _month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let mut claim = load_valid_oracle_update_claim(program_id, month_info.key, claim_info)?;
    let mut challenge =
        load_valid_oracle_update_challenge(program_id, month_info.key, challenge_info)?;
    let challenge_guard = load_canonical_update_challenge_guard(
        program_id,
        month_info.key,
        claim_info.key,
        &claim.claim_id,
        challenge_guard_info,
    )?;
    if challenge.claim != *claim_info.key
        || challenge.claim_id != claim.claim_id
        || challenge_guard.challenge != *challenge_info.key
        || challenge_guard.challenge_id != challenge.challenge_id
        || claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || challenge.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.stake == 0
        || challenge.bond == 0
        || oracle_update_claim_samba_checkpoint_active(program_id, claim_info)?
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let mut claimant_collateral =
        load_canonical_user_collateral(program_id, claimant_collateral_info, &claim.claimant)?;
    let mut challenger_collateral = load_canonical_user_collateral(
        program_id,
        challenger_collateral_info,
        &challenge.challenger,
    )?;
    let slot = Clock::get()?.slot;
    let pot = claim
        .stake
        .checked_add(challenge.bond)
        .ok_or(VaultError::ArithmeticOverflow)?;
    match (challenge.status, claim.status) {
        (OracleChallengeStatus::Rejected, OracleClaimStatus::Finalized) => {
            credit_cash_collateral_at_slot(&mut claimant_collateral, pot, slot)?;
            claim.escrow_disposition = OracleEscrowDisposition::Refunded;
            challenge.escrow_disposition = OracleEscrowDisposition::Slashed;
        }
        (OracleChallengeStatus::Accepted, OracleClaimStatus::Rejected) => {
            credit_cash_collateral_at_slot(&mut challenger_collateral, pot, slot)?;
            claim.escrow_disposition = OracleEscrowDisposition::Slashed;
            challenge.escrow_disposition = OracleEscrowDisposition::Refunded;
        }
        (OracleChallengeStatus::Rejected, OracleClaimStatus::Rejected)
        | (OracleChallengeStatus::Cancelled, OracleClaimStatus::TimedOut) => {
            credit_cash_collateral_at_slot(&mut claimant_collateral, claim.stake, slot)?;
            credit_cash_collateral_at_slot(&mut challenger_collateral, challenge.bond, slot)?;
            claim.escrow_disposition = OracleEscrowDisposition::Refunded;
            challenge.escrow_disposition = OracleEscrowDisposition::Refunded;
        }
        _ => return Err(VaultError::OracleEscrowNotSettleable.into()),
    }
    store_oracle_update_claim(claim_info, &claim)?;
    store_state(challenge_info, &challenge)?;
    store_state(claimant_collateral_info, &claimant_collateral)?;
    store_state(challenger_collateral_info, &challenger_collateral)
}

pub(in crate::processor) fn load_counted_v5_schedule(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    required_count: u32,
    require_failed: bool,
) -> Result<(OracleUsdcRewardSchedule, OracleMonthState), ProgramError> {
    if accounts.len() < 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let (market, month) =
        load_valid_market_and_oracle_month(program_id, &accounts[1], &accounts[2])?;
    let schedule_info = &accounts[3];
    let schedule = load_oracle_usdc_reward_schedule(program_id, accounts[2].key, schedule_info)?;
    if schedule.phase == OracleUsdcRewardSchedulePhase::Aborted
        || schedule.outstanding_prelisting_escrow_count < required_count
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    if require_failed {
        if schedule.phase != OracleUsdcRewardSchedulePhase::Funded {
            return Err(VaultError::OracleUsdcRewardScheduleFrozen.into());
        }
        ensure_failed_v5_reward_schedule_window(&market, &month)?;
    }
    Ok((schedule, month))
}

pub(in crate::processor) fn decrement_counted_v5_schedule_at(
    schedule: &mut OracleUsdcRewardSchedule,
    count: u32,
    slot: u64,
) -> ProgramResult {
    schedule.outstanding_prelisting_escrow_count = schedule
        .outstanding_prelisting_escrow_count
        .checked_sub(count)
        .ok_or(VaultError::InvalidOracleUsdcRewardSchedule)?;
    schedule.last_updated_slot = slot;
    Ok(())
}

pub(in crate::processor) fn settle_counted_v5_listing_bond(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    require_failed: bool,
) -> ProgramResult {
    // cranker, market, month, schedule, source, source reward, proposer collateral
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let schedule_info = &accounts[3];
    let (mut schedule, _) = load_counted_v5_schedule(program_id, accounts, 1, require_failed)?;
    let source = load_valid_oracle_source(program_id, accounts[2].key, &accounts[4])?;
    let mut reward = super::oracle_usdc::load_oracle_usdc_source_reward(
        program_id,
        schedule_info.key,
        accounts[2].key,
        accounts[4].key,
        &accounts[5],
    )?;
    if reward.listing_escrow_counted
        || reward.source_id != source.source_id
        || reward.proposer != source.proposer
    {
        return Err(VaultError::InvalidOracleUsdcSourceReward.into());
    }
    require_signed_cranker(&accounts[0])?;
    settle_cash_listing_bond_loaded(program_id, &accounts[4], &accounts[6], source)?;
    let slot = Clock::get()?.slot;
    reward.listing_escrow_counted = true;
    reward.last_updated_slot = slot;
    decrement_counted_v5_schedule_at(&mut schedule, 1, slot)?;
    store_state(&accounts[5], &reward)?;
    store_state(schedule_info, &schedule)
}

pub(in crate::processor) fn settle_counted_v5_support_stake(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    require_failed: bool,
) -> ProgramResult {
    // cranker, market, month, schedule, source, support position, supporter collateral
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let schedule_info = &accounts[3];
    let (mut schedule, _) = load_counted_v5_schedule(program_id, accounts, 1, require_failed)?;
    let source_info = &accounts[4];
    let support_info = &accounts[5];
    let collateral_info = &accounts[6];
    let source = load_valid_oracle_source(program_id, accounts[2].key, source_info)?;
    let mut support = load_valid_oracle_support_position(
        program_id,
        accounts[2].key,
        source_info.key,
        &source.source_id,
        support_info,
        VaultError::OracleEscrowNotSettleable,
    )?;
    if support.failed_schedule_escrow_counted {
        return Err(VaultError::OracleEscrowAlreadySettled.into());
    }
    require_signed_cranker(&accounts[0])?;
    if support.escrow_disposition != OracleEscrowDisposition::Unsettled
        || support.released
        || !matches!(
            source.status,
            OracleSourceStatus::Active
                | OracleSourceStatus::Inactive
                | OracleSourceStatus::Merged
                | OracleSourceStatus::Rejected
                | OracleSourceStatus::TimedOut
        )
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    let mut collateral =
        load_canonical_user_collateral(program_id, collateral_info, &support.supporter)?;
    let slot = Clock::get()?.slot;
    credit_cash_collateral_at_slot(&mut collateral, support.support_stake, slot)?;
    support.released = true;
    support.escrow_disposition = OracleEscrowDisposition::Refunded;
    support.failed_schedule_escrow_counted = true;
    decrement_counted_v5_schedule_at(&mut schedule, 1, slot)?;
    store_state(support_info, &support)?;
    store_state(collateral_info, &collateral)?;
    store_state(schedule_info, &schedule)
}

pub(in crate::processor) fn settle_counted_v5_prelisting_escrow(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    kind: OracleEscrowKind,
    require_failed: bool,
) -> ProgramResult {
    match kind {
        OracleEscrowKind::ListingBond => {
            settle_counted_v5_listing_bond(program_id, accounts, require_failed)
        }
        OracleEscrowKind::SupportStake => {
            settle_counted_v5_support_stake(program_id, accounts, require_failed)
        }
        OracleEscrowKind::SourceChallenge => {
            settle_counted_v5_source_challenge(program_id, accounts, require_failed)
        }
        _ => Err(VaultError::InvalidOracleState.into()),
    }
}

#[inline(never)]
pub(in crate::processor) fn process_settle_oracle_usdc_escrow(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SettleOracleEscrowParams,
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    match params.kind {
        OracleEscrowKind::ListingBond
        | OracleEscrowKind::SupportStake
        | OracleEscrowKind::SourceChallenge => {
            settle_counted_v5_prelisting_escrow(program_id, accounts, params.kind, false)
        }
        OracleEscrowKind::OpeningClaim => settle_cash_opening_claim(program_id, accounts),
        OracleEscrowKind::OpeningChallenge => settle_cash_opening_challenge(program_id, accounts),
        OracleEscrowKind::UpdateClaim => settle_cash_update_claim(program_id, accounts),
        OracleEscrowKind::UpdateChallenge => settle_cash_update_challenge(program_id, accounts),
    }
}

#[inline(never)]
pub(in crate::processor) fn process_settle_failed_oracle_month_escrow_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SettleOracleEscrowParams,
) -> ProgramResult {
    settle_counted_v5_prelisting_escrow(program_id, accounts, params.kind, true)
}

#[inline(never)]
pub(in crate::processor) fn process_abort_oracle_usdc_reward_schedule_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    // cranker, market, month, coverage, reward vault, schedule
    if accounts.len() != 6 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let (market, month) =
        load_valid_market_and_oracle_month(program_id, &accounts[1], &accounts[2])?;
    ensure_failed_v5_reward_schedule_window(&market, &month)?;
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, accounts[2].key, &accounts[3])?;
    let mut reward_vault = load_oracle_usdc_reward_vault(program_id, &accounts[4])?;
    let mut schedule = load_oracle_usdc_reward_schedule(program_id, accounts[2].key, &accounts[5])?;
    if coverage.coverage_finalized
        || schedule.phase != OracleUsdcRewardSchedulePhase::Funded
        || schedule.reward_vault != *accounts[4].key
        || schedule.outstanding_prelisting_escrow_count != 0
        || schedule.registered_source_count != 0
        || schedule.registered_opening_count != 0
        || schedule.registered_update_count != 0
        || schedule.remaining_reward_budget != schedule.total_reward_budget
        || reward_vault.total_reserved < schedule.remaining_reward_budget
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    release_aborted_reward_reservation(&mut reward_vault, &mut schedule)?;
    let slot = Clock::get()?.slot;
    schedule.last_updated_slot = slot;
    reward_vault.last_updated_slot = slot;
    store_state(&accounts[4], &reward_vault)?;
    store_state(&accounts[5], &schedule)
}
