use super::*;

pub(in crate::processor) fn floor_winning_entitlement(
    total_committed: u64,
    vote_power: u64,
    winning_power: u64,
) -> Result<u64, ProgramError> {
    if total_committed == 0
        || vote_power == 0
        || winning_power == 0
        || vote_power > winning_power
        || winning_power > total_committed
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    u64::try_from(
        u128::from(total_committed)
            .checked_mul(u128::from(vote_power))
            .ok_or(VaultError::ArithmeticOverflow)?
            / u128::from(winning_power),
    )
    .map_err(|_| VaultError::ArithmeticOverflow.into())
}

#[inline(never)]
pub(in crate::processor) fn process_register_oracle_samba_winning_vote(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 6 || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let dispute_info = &accounts[1];
    let pot_info = &accounts[2];
    let vote_info = &accounts[3];
    let registration_info = &accounts[4];
    let system_program_info = &accounts[5];
    validate_system_program(system_program_info)?;
    let dispute = load_dispute_v3(program_id, dispute_info)?;
    let mut pot = load_pot(program_id, dispute_info, &dispute, pot_info)?;
    let vote = load_vote_v3(program_id, dispute_info, &dispute, pot_info, vote_info)?;
    if dispute.status != OracleChallengeStatus::Accepted
        || pot.payout_mode != OracleSambaEmergencyPayoutMode::Redistribute
        || pot.registration_finalized
        || vote.status != OracleEmergencyVoteStatus::Revealed
        || vote.choice != pot.winning_choice
        || vote.escrow_disposition != OracleEscrowDisposition::Unsettled
        || vote.samba_mint != pot.samba_mint
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let (expected, bump) =
        derive_oracle_samba_winning_vote_pda(program_id, pot_info.key, vote_info.key);
    if *registration_info.key != expected {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    validate_create_only_program_account_target(program_id, registration_info)?;
    let base =
        floor_winning_entitlement(pot.total_committed, vote.voting_power, pot.winning_power)?;
    if base < vote.locked_amount {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let next_power = pot
        .registered_power
        .checked_add(vote.voting_power)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let next_count = pot
        .registered_vote_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let next_base = pot
        .registered_base_total
        .checked_add(base)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if next_power > pot.winning_power
        || next_count > pot.winning_vote_count
        || next_base > pot.total_committed
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    create_program_account(
        payer_info,
        registration_info,
        system_program_info,
        program_id,
        OracleSambaWinningVote::LEN,
        &[
            ORACLE_SAMBA_WINNING_VOTE_PDA_SEED,
            pot_info.key.as_ref(),
            vote_info.key.as_ref(),
            &[bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let registration = OracleSambaWinningVote {
        is_initialized: true,
        bump,
        account_discriminator: OracleSambaWinningVote::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSambaWinningVote::ACCOUNT_VERSION,
        pot: *pot_info.key,
        dispute: *dispute_info.key,
        vote: *vote_info.key,
        voter: vote.voter,
        voting_power: vote.voting_power,
        base_entitlement: base,
        registered_slot: slot,
    };
    pot.registered_power = next_power;
    pot.registered_vote_count = next_count;
    pot.registered_base_total = next_base;
    if crate::pubkey_is_default(&pot.dust_recipient_vote)
        || vote_info.key.to_bytes() < pot.dust_recipient_vote.to_bytes()
    {
        pot.dust_recipient_vote = *vote_info.key;
    }
    let power_complete = next_power == pot.winning_power;
    let count_complete = next_count == pot.winning_vote_count;
    if power_complete != count_complete {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    if power_complete {
        pot.dust_amount = pot
            .total_committed
            .checked_sub(next_base)
            .ok_or(VaultError::ArithmeticOverflow)?;
        pot.registration_finalized = true;
    }
    pot.last_updated_slot = slot;
    store_state(registration_info, &registration)?;
    store_state(pot_info, &pot)
}

pub(in crate::processor) fn load_winning_registration(
    program_id: &Pubkey,
    dispute_info: &AccountInfo,
    pot_info: &AccountInfo,
    vote_info: &AccountInfo,
    vote: &OracleEmergencyVoteRecordV3,
    registration_info: &AccountInfo,
) -> Result<OracleSambaWinningVote, ProgramError> {
    let registration: OracleSambaWinningVote = load_exact_zero_padded_state(
        registration_info,
        program_id,
        OracleSambaWinningVote::LEN,
        VaultError::InvalidOracleEmergencyDispute,
    )?;
    let (expected, bump) =
        derive_oracle_samba_winning_vote_pda(program_id, pot_info.key, vote_info.key);
    if !registration.is_initialized
        || registration.account_discriminator != OracleSambaWinningVote::ACCOUNT_DISCRIMINATOR
        || registration.account_version != OracleSambaWinningVote::ACCOUNT_VERSION
        || registration.bump != bump
        || *registration_info.key != expected
        || registration.pot != *pot_info.key
        || registration.dispute != *dispute_info.key
        || registration.vote != *vote_info.key
        || registration.voter != vote.voter
        || registration.voting_power != vote.voting_power
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(registration)
}

#[inline(never)]
pub(in crate::processor) fn process_settle_oracle_samba_emergency_vote_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let payer_info = &accounts[0];
    let dispute_info = &accounts[1];
    let pot_info = &accounts[2];
    let pot_token_info = &accounts[3];
    let samba_mint_info = &accounts[4];
    let vote_info = &accounts[5];
    let destination_info = &accounts[6];
    let receipt_info = &accounts[7];
    let token_program_info = &accounts[8];
    let system_program_info = &accounts[9];
    if !payer_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    validate_system_program(system_program_info)?;
    let trailing = &accounts[10..];
    let dispute = load_dispute_v3(program_id, dispute_info)?;
    let mut pot = load_pot(program_id, dispute_info, &dispute, pot_info)?;
    let pre_pot_token = validate_pot_token_account(&pot, pot_info, pot_token_info)?;
    let mint = validate_mint_account(samba_mint_info, token_program_info.key)?;
    let mut vote = load_vote_v3(program_id, dispute_info, &dispute, pot_info, vote_info)?;
    let expected_destination =
        crate::associated_token::get_associated_token_address_with_program_id(
            &vote.voter,
            samba_mint_info.key,
            token_program_info.key,
        );
    if dispute.status != OracleChallengeStatus::Accepted
        || pot.payout_mode == OracleSambaEmergencyPayoutMode::Open
        || pot.samba_mint != *samba_mint_info.key
        || vote.samba_mint != *samba_mint_info.key
        || vote.escrow_disposition != OracleEscrowDisposition::Unsettled
        || !matches!(
            vote.status,
            OracleEmergencyVoteStatus::Committed | OracleEmergencyVoteStatus::Revealed
        )
        || *destination_info.key != expected_destination
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    validate_vault_token_account(destination_info, samba_mint_info.key, &vote.voter)?;
    let pre_destination = validate_token_account(destination_info)?;

    let (amount, disposition) = match pot.payout_mode {
        OracleSambaEmergencyPayoutMode::Open => {
            return Err(VaultError::InvalidOracleEmergencyDispute.into())
        }
        OracleSambaEmergencyPayoutMode::RefundAll => {
            if !trailing.is_empty() {
                return Err(VaultError::InvalidAccountList.into());
            }
            (vote.locked_amount, OracleEscrowDisposition::Refunded)
        }
        OracleSambaEmergencyPayoutMode::Redistribute => {
            let is_winner = vote.status == OracleEmergencyVoteStatus::Revealed
                && vote.choice == pot.winning_choice;
            if !is_winner {
                if !trailing.is_empty() {
                    return Err(VaultError::InvalidAccountList.into());
                }
                (0, OracleEscrowDisposition::Slashed)
            } else {
                if trailing.len() != 1 || !pot.registration_finalized {
                    return Err(VaultError::InvalidAccountList.into());
                }
                let registration = load_winning_registration(
                    program_id,
                    dispute_info,
                    pot_info,
                    vote_info,
                    &vote,
                    &trailing[0],
                )?;
                let dust = if pot.dust_recipient_vote == *vote_info.key {
                    pot.dust_amount
                } else {
                    0
                };
                (
                    registration
                        .base_entitlement
                        .checked_add(dust)
                        .ok_or(VaultError::ArithmeticOverflow)?,
                    OracleEscrowDisposition::Transferred,
                )
            }
        }
    };
    if amount > pot.remaining_liability {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let (expected_receipt, receipt_bump) =
        derive_oracle_samba_vote_settlement_pda(program_id, pot_info.key, vote_info.key);
    if *receipt_info.key != expected_receipt {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    validate_create_only_program_account_target(program_id, receipt_info)?;
    create_program_account(
        payer_info,
        receipt_info,
        system_program_info,
        program_id,
        OracleSambaVoteSettlementReceipt::LEN,
        &[
            ORACLE_SAMBA_VOTE_SETTLEMENT_PDA_SEED,
            pot_info.key.as_ref(),
            vote_info.key.as_ref(),
            &[receipt_bump],
        ],
    )?;

    if amount > 0 {
        invoke_token_transfer_checked(
            token_program_info,
            pot_token_info,
            samba_mint_info,
            destination_info,
            pot_info,
            amount,
            mint.decimals,
            &[&[
                CURRENT_STATE_NAMESPACE_SEED,
                ORACLE_SAMBA_EMERGENCY_POT_PDA_SEED,
                dispute_info.key.as_ref(),
                &[pot.bump],
            ]],
        )?;
    }
    let post_pot_token = validate_token_account(pot_token_info)?;
    let post_destination = validate_token_account(destination_info)?;
    if post_pot_token.amount
        != pre_pot_token
            .amount
            .checked_sub(amount)
            .ok_or(VaultError::ArithmeticOverflow)?
        || post_destination.amount
            != pre_destination
                .amount
                .checked_add(amount)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    pot.remaining_liability = pot
        .remaining_liability
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pot.total_paid = pot
        .total_paid
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    pot.settled_vote_count = pot
        .settled_vote_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if pot.total_paid.checked_add(pot.remaining_liability) != Some(pot.total_committed)
        || post_pot_token.amount < pot.remaining_liability
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let slot = Clock::get()?.slot;
    pot.last_updated_slot = slot;
    if vote.status == OracleEmergencyVoteStatus::Committed {
        vote.status = OracleEmergencyVoteStatus::Expired;
    }
    vote.escrow_disposition = disposition;
    let receipt = OracleSambaVoteSettlementReceipt {
        is_initialized: true,
        bump: receipt_bump,
        account_discriminator: OracleSambaVoteSettlementReceipt::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSambaVoteSettlementReceipt::ACCOUNT_VERSION,
        pot: *pot_info.key,
        dispute: *dispute_info.key,
        vote: *vote_info.key,
        voter: vote.voter,
        destination: *destination_info.key,
        amount,
        disposition,
        settled_slot: slot,
    };
    store_state(receipt_info, &receipt)?;
    store_state(vote_info, &vote)?;
    store_state(pot_info, &pot)
}
