use super::*;

pub(super) fn load_agent(
    program: &Pubkey,
    info: &AccountInfo,
) -> Result<AgentReserve, ProgramError> {
    let agent: AgentReserve = load_exact_zero_padded_state(
        info,
        program,
        AgentReserve::LEN,
        VaultError::InvalidOracleState,
    )?;
    let (key, bump) = reserve_address(program, &agent.agent_id, &agent.authority);
    if info.key != &key
        || agent.bump != bump
        || agent.discriminator != AgentReserve::MAGIC
        || agent.agent_id == [0; 32]
        || agent.authority == Pubkey::default()
    {
        return invalid();
    }
    Ok(agent)
}
pub(super) fn load_agreement(
    program: &Pubkey,
    info: &AccountInfo,
) -> Result<SourceSponsorship, ProgramError> {
    let value: SourceSponsorship = load_exact_zero_padded_state(
        info,
        program,
        SourceSponsorship::LEN,
        VaultError::InvalidOracleState,
    )?;
    let (key, bump) = agreement_address(program, &value.source);
    if info.key != &key
        || value.bump != bump
        || value.discriminator != SourceSponsorship::MAGIC
        || value.bond == 0
        || value.candidate_hash == [0; 32]
        || value.funding > 1
        || value.outcome > 3
        || value.researcher_bps
            != if value.funding == 1 {
                10_000
            } else {
                DEFAULT_RESEARCHER_BPS
            }
        || value.retention_bps != DEFAULT_RETENTION_BPS
        || (value.funding == 1 && (value.funder != value.reserve || value.researcher_bps != 10_000))
    {
        return invalid();
    }
    Ok(value)
}
pub(super) fn load_exposure(
    program: &Pubkey,
    info: &AccountInfo,
    reserve: &Pubkey,
    funder: &Pubkey,
) -> Result<SponsorExposure, ProgramError> {
    let value: SponsorExposure = load_exact_zero_padded_state(
        info,
        program,
        SponsorExposure::LEN,
        VaultError::InvalidOracleState,
    )?;
    let (key, bump) = exposure_address(program, reserve, funder);
    if info.key != &key
        || value.bump != bump
        || value.discriminator != SponsorExposure::MAGIC
        || &value.reserve != reserve
        || &value.funder != funder
    {
        return invalid();
    }
    Ok(value)
}

pub(super) fn ensure_collateral<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    owner: &Pubkey,
) -> ProgramResult {
    let (key, bump) = derive_user_collateral_pda(program, owner);
    if info.key != &key {
        return invalid();
    }
    if info.owner == program {
        load_canonical_user_collateral(program, info, owner)?;
        return Ok(());
    }
    create_program_account(
        payer,
        info,
        system,
        program,
        UserCollateral::LEN,
        &[
            crate::constants::USER_COLLATERAL_PDA_SEED,
            owner.as_ref(),
            &[bump],
        ],
    )?;
    store_state(
        info,
        &UserCollateral {
            is_initialized: true,
            bump,
            owner: *owner,
            available_balance: 0,
            position_locked_balance: 0,
            last_action_slot: Clock::get()?.slot,
        },
    )
}

/// Reloads on each leg so even researcher == sponsor or reserve == funder is
/// safe. No cached recipient snapshots can overwrite an earlier credit.
pub(super) fn move_cash(
    program: &Pubkey,
    from: &AccountInfo,
    from_owner: &Pubkey,
    to: &AccountInfo,
    to_owner: &Pubkey,
    amount: u64,
) -> ProgramResult {
    let mut debit = load_canonical_user_collateral(program, from, from_owner)?;
    let mut credit = load_canonical_user_collateral(program, to, to_owner)?;
    if from.key == to.key {
        return invalid();
    }
    debit.available_balance = debit
        .available_balance
        .checked_sub(amount)
        .ok_or(VaultError::InvalidOracleUsdcBond)?;
    credit.available_balance = add(credit.available_balance, amount)?;
    let slot = Clock::get()?.slot;
    debit.last_action_slot = slot;
    credit.last_action_slot = slot;
    store_state(from, &debit)?;
    store_state(to, &credit)
}
