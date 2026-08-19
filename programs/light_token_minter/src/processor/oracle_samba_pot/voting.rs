use super::*;

#[inline(never)]
pub(in crate::processor) fn process_try_open_oracle_emergency_dispute_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: TryOpenOracleEmergencyDisputeParams,
) -> ProgramResult {
    if accounts.len() < 13 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let dispute_info = &accounts[3];
    let target_info = &accounts[4];
    let pot_info = &accounts[5];
    let pot_token_info = &accounts[6];
    let config_info = &accounts[7];
    let staking_pool_info = &accounts[8];
    let samba_mint_info = &accounts[9];
    let token_program_info = &accounts[10];
    let associated_token_program_info = &accounts[11];
    let system_program_info = &accounts[12];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *associated_token_program_info.key != crate::associated_token::id()
    {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    validate_system_program(system_program_info)?;
    if crate::bytes32_is_zero(&params.target_id) {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }

    let _config = load_current_canonical_vault_config(program_id, config_info)?;
    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    if month.phase == OraclePhase::Closed {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    if params.kind == OracleEmergencyDisputeKind::Update {
        oracle_usdc::ensure_current_cash_update_emergency_window_open(&market)?;
    }
    let packet = validate_cash_emergency_target(
        program_id,
        month_info.key,
        &month,
        params.kind,
        &params.target_id,
        target_info,
        &accounts[13..],
        None,
        None,
    )?;

    let staking_pool = load_canonical_oracle_staking_pool(
        program_id,
        staking_pool_info,
        &derive_oracle_major_token_config_pda(program_id).0,
    )?;
    let samba_mint = validate_oracle_samba_mint(&staking_pool, samba_mint_info, config_info.key)?;
    if staking_pool.governance_lock_count == 0
        || packet.snapshot_total_major_tokens != staking_pool.samba_supply
        || samba_mint.supply > staking_pool.samba_supply
    {
        return Err(VaultError::InvalidOracleStakingPool.into());
    }

    let case_hash = oracle_emergency_case_hash(month_info.key, params.kind, &params.target_id);
    if let Some(expected_case_hash) = params.expected_case_hash {
        if expected_case_hash != case_hash {
            return Err(VaultError::InvalidOracleEmergencyDispute.into());
        }
    }
    let (expected_dispute, dispute_bump) =
        derive_oracle_emergency_dispute_v3_pda(program_id, month_info.key, &case_hash);
    let (expected_pot, pot_bump) =
        derive_oracle_samba_emergency_pot_pda(program_id, dispute_info.key);
    let expected_pot_token = crate::associated_token::get_associated_token_address_with_program_id(
        &expected_pot,
        samba_mint_info.key,
        token_program_info.key,
    );
    if *dispute_info.key != expected_dispute
        || *pot_info.key != expected_pot
        || *pot_token_info.key != expected_pot_token
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    validate_create_only_program_account_target(program_id, dispute_info)?;
    validate_create_only_program_account_target(program_id, pot_info)?;
    create_program_account(
        cranker_info,
        dispute_info,
        system_program_info,
        program_id,
        OracleEmergencyDisputeV3::LEN,
        &[
            ORACLE_EMERGENCY_DISPUTE_V3_PDA_SEED,
            month_info.key.as_ref(),
            &case_hash,
            &[dispute_bump],
        ],
    )?;
    create_program_account(
        cranker_info,
        pot_info,
        system_program_info,
        program_id,
        OracleSambaEmergencyPot::LEN,
        &[
            ORACLE_SAMBA_EMERGENCY_POT_PDA_SEED,
            dispute_info.key.as_ref(),
            &[pot_bump],
        ],
    )?;
    invoke_create_associated_token_account_idempotent(
        cranker_info,
        pot_token_info,
        pot_info,
        samba_mint_info,
        system_program_info,
        token_program_info,
        associated_token_program_info,
    )?;

    let slot = Clock::get()?.slot;
    validate_oracle_economics(&month.economics)?;
    let commit_deadline_slot = slot
        .checked_add(month.economics.emergency_commit_window_slots)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let reveal_deadline_slot = commit_deadline_slot
        .checked_add(month.economics.emergency_reveal_window_slots)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let pot = OracleSambaEmergencyPot {
        is_initialized: true,
        bump: pot_bump,
        account_discriminator: OracleSambaEmergencyPot::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSambaEmergencyPot::ACCOUNT_VERSION,
        dispute: *dispute_info.key,
        month: *month_info.key,
        samba_mint: *samba_mint_info.key,
        token_account: *pot_token_info.key,
        payout_mode: OracleSambaEmergencyPayoutMode::Open,
        last_updated_slot: slot,
        ..OracleSambaEmergencyPot::default()
    };
    validate_pot_token_account(&pot, pot_info, pot_token_info)?;
    let dispute = build_open_dispute_v3(
        month_info.key,
        dispute_bump,
        case_hash,
        params.kind,
        params.target_id,
        target_info.key,
        cranker_info.key,
        &packet,
        &month.economics,
        pot_info.key,
        commit_deadline_slot,
        reveal_deadline_slot,
    )?;
    if params.kind != OracleEmergencyDisputeKind::BucketMedian {
        set_cash_emergency_guard_dispute_binding(
            program_id,
            params.kind,
            month_info.key,
            target_info,
            &accounts[13..],
            &Pubkey::default(),
            dispute_info.key,
            slot,
        )?;
    }
    store_state(pot_info, &pot)?;
    store_state(dispute_info, &dispute)
}

#[inline(never)]
pub(in crate::processor) fn process_commit_oracle_emergency_vote_v3(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CommitOracleEmergencyVoteV2Params,
) -> ProgramResult {
    if accounts.len() != 11 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let voter_info = &accounts[0];
    let config_info = &accounts[1];
    let staking_pool_info = &accounts[2];
    let samba_mint_info = &accounts[3];
    let voter_token_info = &accounts[4];
    let dispute_info = &accounts[5];
    let pot_info = &accounts[6];
    let pot_token_info = &accounts[7];
    let vote_info = &accounts[8];
    let token_program_info = &accounts[9];
    let system_program_info = &accounts[10];
    if !voter_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    validate_system_program(system_program_info)?;
    if params.samba_amount == 0 || crate::bytes32_is_zero(&params.commit_hash) {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }

    load_current_canonical_vault_config(program_id, config_info)?;
    let staking_pool = load_canonical_oracle_staking_pool(
        program_id,
        staking_pool_info,
        &derive_oracle_major_token_config_pda(program_id).0,
    )?;
    let samba_mint = validate_oracle_samba_mint(&staking_pool, samba_mint_info, config_info.key)?;
    if staking_pool.governance_lock_count == 0 {
        return Err(VaultError::SambaSupplyLockedForVoting.into());
    }
    let voter_token =
        validate_owner_samba_token_account(voter_token_info, voter_info.key, samba_mint_info.key)?;
    if voter_token.amount < params.samba_amount {
        return Err(VaultError::InsufficientSambaTokens.into());
    }

    let mut dispute = load_dispute_v3(program_id, dispute_info)?;
    let mut pot = load_pot(program_id, dispute_info, &dispute, pot_info)?;
    let pot_token = validate_pot_token_account(&pot, pot_info, pot_token_info)?;
    let slot = Clock::get()?.slot;
    if dispute.status != OracleChallengeStatus::Open
        || pot.payout_mode != OracleSambaEmergencyPayoutMode::Open
        || pot.samba_mint != *samba_mint_info.key
        || dispute.snapshot_total_major_tokens != staking_pool.samba_supply
        || samba_mint.supply > staking_pool.samba_supply
        || dispute.committed_power != pot.total_committed
        || dispute.committed_vote_count != pot.committed_vote_count
        || slot > dispute.commit_deadline_slot
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let (next_committed, next_vote_count) = next_v3_vote_totals(&dispute, params.samba_amount)?;

    let (expected_vote, vote_bump) =
        derive_oracle_emergency_vote_v3_pda(program_id, dispute_info.key, voter_info.key);
    if *vote_info.key != expected_vote {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    validate_create_only_program_account_target(program_id, vote_info)?;
    create_program_account(
        voter_info,
        vote_info,
        system_program_info,
        program_id,
        OracleEmergencyVoteRecordV3::LEN,
        &[
            ORACLE_EMERGENCY_VOTE_V3_PDA_SEED,
            dispute_info.key.as_ref(),
            voter_info.key.as_ref(),
            &[vote_bump],
        ],
    )?;

    invoke_token_transfer_checked(
        token_program_info,
        voter_token_info,
        samba_mint_info,
        pot_token_info,
        voter_info,
        params.samba_amount,
        samba_mint.decimals,
        &[],
    )?;
    let post_voter =
        validate_owner_samba_token_account(voter_token_info, voter_info.key, samba_mint_info.key)?;
    let post_pot = validate_token_account(pot_token_info)?;
    if post_voter.amount
        != voter_token
            .amount
            .checked_sub(params.samba_amount)
            .ok_or(VaultError::ArithmeticOverflow)?
        || post_pot.amount
            != pot_token
                .amount
                .checked_add(params.samba_amount)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }

    dispute.committed_power = next_committed;
    dispute.committed_vote_count = next_vote_count;
    pot.total_committed = next_committed;
    pot.remaining_liability = pot
        .remaining_liability
        .checked_add(params.samba_amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pot.committed_vote_count = next_vote_count;
    pot.last_updated_slot = slot;
    let vote = OracleEmergencyVoteRecordV3 {
        is_initialized: true,
        bump: vote_bump,
        account_discriminator: OracleEmergencyVoteRecordV3::ACCOUNT_DISCRIMINATOR,
        account_version: OracleEmergencyVoteRecordV3::ACCOUNT_VERSION,
        month: dispute.month,
        dispute: *dispute_info.key,
        pot: *pot_info.key,
        voter: *voter_info.key,
        samba_mint: *samba_mint_info.key,
        commit_hash: params.commit_hash,
        locked_amount: params.samba_amount,
        snapshot_power: voter_token.amount,
        voting_power: params.samba_amount,
        choice: 0,
        status: OracleEmergencyVoteStatus::Committed,
        escrow_disposition: OracleEscrowDisposition::Unsettled,
        committed_slot: slot,
        revealed_slot: 0,
    };
    validate_pot_token_account(&pot, pot_info, pot_token_info)?;
    store_state(vote_info, &vote)?;
    store_state(pot_info, &pot)?;
    store_state(dispute_info, &dispute)
}

#[inline(never)]
pub(in crate::processor) fn process_reveal_oracle_emergency_vote_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RevealOracleEmergencyVoteParams,
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let voter_info = &accounts[0];
    let dispute_info = &accounts[1];
    let pot_info = &accounts[2];
    let vote_info = &accounts[3];
    if !voter_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut dispute = load_dispute_v3(program_id, dispute_info)?;
    let pot = load_pot(program_id, dispute_info, &dispute, pot_info)?;
    let mut vote = load_vote_v3(program_id, dispute_info, &dispute, pot_info, vote_info)?;
    let slot = Clock::get()?.slot;
    if dispute.status != OracleChallengeStatus::Open
        || pot.payout_mode != OracleSambaEmergencyPayoutMode::Open
        || vote.voter != *voter_info.key
        || vote.samba_mint != pot.samba_mint
        || vote.status != OracleEmergencyVoteStatus::Committed
        || vote.escrow_disposition != OracleEscrowDisposition::Unsettled
        || slot <= dispute.commit_deadline_slot
        || slot > dispute.reveal_deadline_slot
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    validate_oracle_emergency_choice(dispute.kind, params.choice)?;
    if params.choice >= dispute.choice_count {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let expected_hash = commitment_hash_v3(
        program_id,
        dispute_info.key,
        &dispute.dispute_id,
        voter_info.key,
        vote.locked_amount,
        &vote.samba_mint,
        params.choice,
        &params.salt,
    );
    if vote.commit_hash != expected_hash {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let index = usize::from(params.choice);
    dispute.choice_power[index] = dispute.choice_power[index]
        .checked_add(vote.voting_power)
        .ok_or(VaultError::ArithmeticOverflow)?;
    dispute.choice_vote_count[index] = dispute.choice_vote_count[index]
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    dispute.revealed_power = dispute
        .revealed_power
        .checked_add(vote.voting_power)
        .ok_or(VaultError::ArithmeticOverflow)?;
    dispute.revealed_vote_count = dispute
        .revealed_vote_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if dispute.revealed_power > dispute.committed_power
        || dispute.revealed_vote_count > dispute.committed_vote_count
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    vote.choice = params.choice;
    vote.status = OracleEmergencyVoteStatus::Revealed;
    vote.revealed_slot = slot;
    store_state(vote_info, &vote)?;
    store_state(dispute_info, &dispute)
}
