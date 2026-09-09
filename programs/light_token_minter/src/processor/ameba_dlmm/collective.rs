use super::*;
use crate::state::{WriterSettlementGroupStatus, WriterSleeveStatus};

const INITIALIZE_COLLECTIVE_PREFIX_ACCOUNTS: usize = 20;
const SET_COLLECTIVE_STATUS_ACCOUNTS: usize = 13;
const COLLECTIVE_SWAP_FIXED_ACCOUNTS: usize = 24;
const SETTLE_COLLECTIVE_POOL_ACCOUNTS: usize = 17;

fn normalized_accounts<'a>(
    accounts: &[AccountInfo<'a>],
    prefix_end: usize,
    suffix_start: usize,
) -> Vec<AccountInfo<'a>> {
    accounts[..prefix_end]
        .iter()
        .chain(accounts[suffix_start..].iter())
        .cloned()
        .collect()
}

fn validate_collective_pool_binding(
    config: &VaultConfig,
    market_info: &AccountInfo,
    anchor_month_info: &AccountInfo,
    context: &super::super::writer_sleeve::CollectiveDlmmContext,
    pool: &AmoebaDlmmPoolV1,
) -> ProgramResult {
    if pool.market != *market_info.key
        || pool.oracle_month != *anchor_month_info.key
        || pool.option_mint != context.option_mint
        || pool.quote_mint != context.quote_mint
        || pool.quote_mint != config.usdc_mint
        || pool.expiry_ts != context.expiry_ts
        || pool.tick_size_quote_atomic != context.tick_size_quote_atomic
        || pool.maximum_price_quote_atomic != context.maximum_price_quote_atomic
        || pool.maximum_bin_id != context.maximum_bin_id
        || pool.swap_fee_bps != context.swap_fee_bps
        || pool.maximum_bins_per_swap != context.maximum_bins_per_swap
    {
        return Err(VaultError::InvalidAmoebaDlmmPool.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn process_initialize_collective_pool(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeAmoebaDlmmPoolV1Params,
) -> ProgramResult {
    if accounts.len() < INITIALIZE_COLLECTIVE_PREFIX_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Existing init prefix: admin, config, Market, month. Collective companions are inserted
    // immediately before the pool, and the Light proof tail remains byte-for-byte unchanged.
    let context = super::super::writer_sleeve::load_collective_dlmm_context(
        program_id,
        &accounts[4],
        &accounts[5],
        &accounts[6],
        &accounts[2],
        &accounts[3],
    )?;
    if !matches!(
        context.group_status,
        WriterSettlementGroupStatus::Anchored | WriterSettlementGroupStatus::Active
    ) || matches!(
        context.sleeve_status,
        WriterSleeveStatus::Expired
            | WriterSleeveStatus::SettlementFinalized
            | WriterSleeveStatus::Closed
    ) {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let normalized = normalized_accounts(accounts, 4, 7);
    process_initialize_collective_pool_core(program_id, &normalized, params)
}

#[inline(never)]
pub(super) fn process_set_collective_pool_status(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetAmoebaDlmmPoolStatusV1Params,
) -> ProgramResult {
    if accounts.len() != SET_COLLECTIVE_STATUS_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Existing prefix through active weights, then sleeve/group/book before pool.
    let context = super::super::writer_sleeve::load_collective_dlmm_context(
        program_id,
        &accounts[6],
        &accounts[7],
        &accounts[8],
        &accounts[2],
        &accounts[3],
    )?;
    let config = load_canonical_vault_config(program_id, &accounts[1])?;
    let pool = load_pool(program_id, &accounts[9])?;
    validate_collective_pool_binding(&config, &accounts[2], &accounts[3], &context, &pool)?;
    if context.group_status != WriterSettlementGroupStatus::Active
        || context.sleeve_status != WriterSleeveStatus::Active
        || context.active_weight_manifest_hash
            != load_valid_oracle_active_weight_manifest(program_id, accounts[3].key, &accounts[5])?
                .rolling_manifest_hash
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let normalized = normalized_accounts(accounts, 6, 9);
    process_set_collective_pool_status_core(program_id, &normalized, params)
}

#[inline(never)]
pub(super) fn process_collective_swap_exact_in(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SwapAmoebaDlmmExactInV1Params,
) -> ProgramResult {
    if accounts.len() <= COLLECTIVE_SWAP_FIXED_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Existing trader/config/Market/month prefix, then sleeve/group/book before pool.
    let context = super::super::writer_sleeve::load_collective_dlmm_context(
        program_id,
        &accounts[4],
        &accounts[5],
        &accounts[6],
        &accounts[2],
        &accounts[3],
    )?;
    let config = load_canonical_vault_config(program_id, &accounts[1])?;
    let pool = load_pool(program_id, &accounts[7])?;
    validate_collective_pool_binding(&config, &accounts[2], &accounts[3], &context, &pool)?;
    if context.group_status != WriterSettlementGroupStatus::Active
        || context.sleeve_status != WriterSleeveStatus::Active
        || context.anchor_month_settled
    {
        return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
    }
    let normalized: Vec<_> = accounts[..4]
        .iter()
        .chain(accounts[7..23].iter())
        .chain(accounts[24..].iter())
        .cloned()
        .collect();
    process_collective_swap_exact_in_core(program_id, &normalized, params)?;
    // The original trade signature also grants the exact owner/mint settlement capability.
    super::super::scoped_settlement::authorize_collective_settlement(
        program_id,
        &[
            accounts[0].clone(),
            accounts[2].clone(),
            accounts[9].clone(),
            accounts[13].clone(),
            accounts[23].clone(),
            accounts[15].clone(),
            accounts[21].clone(),
            accounts[22].clone(),
            accounts[20].clone(),
        ],
        false,
    )
}

#[inline(never)]
pub(super) fn process_settle_collective_pool(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != SETTLE_COLLECTIVE_POOL_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let group_info = &accounts[4];
    let pool_info = &accounts[5];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    let market = load_valid_market(program_id, market_info)?;
    let group = super::super::writer_sleeve::load_collective_settlement_group_for_dlmm(
        program_id, group_info,
    )?;
    let month = load_oracle_month_state(month_info, program_id)?;
    let (expected_month, expected_month_bump) =
        derive_oracle_month_pda(program_id, &group.anchor_market, group.expiry_ts);
    let mut pool = load_pool(program_id, pool_info)?;
    if group.status != WriterSettlementGroupStatus::Settled
        || crate::bytes32_is_zero(&group.final_settlement_commitment)
        || group.anchor_oracle_month != *month_info.key
        || *month_info.key != expected_month
        || !month.is_initialized
        || month.bump != expected_month_bump
        || month.market != group.anchor_market
        || month.phase != OraclePhase::Settled
        || month.settlement_status != OracleSettlementStatus::Final
        || month.finalized_at_ts == 0
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != group.expiry_ts
        || market.collateral_mint != group.settlement_mint
        || pool.market != *market_info.key
        || pool.oracle_month != *month_info.key
        || pool.option_mint
            != market
                .long_contract_mint
                .ok_or(VaultError::InvalidContractMint)?
        || pool.quote_mint != config.usdc_mint
        || pool.expiry_ts != group.expiry_ts
        || matches!(
            pool.status,
            AmoebaDlmmPoolStatus::Settled | AmoebaDlmmPoolStatus::Closed
        )
    {
        return Err(VaultError::InvalidAmoebaDlmmSettlement.into());
    }
    let slot = Clock::get()?.slot;
    sweep_pool_oracle_bounty(
        program_id,
        accounts,
        month_info.key,
        pool_info,
        &mut pool,
        slot,
    )?;
    pool.status = AmoebaDlmmPoolStatus::Settled;
    pool.settlement_price_atomic = group.settlement_price_atomic;
    pool.settled_slot = slot;
    pool.last_updated_slot = slot;
    store_light_state(pool_info, &pool)?;
    emit_event(
        &EVENT_POOL_SETTLED,
        AmoebaDlmmEvent::Settled(SettledEvent {
            pool: *pool_info.key,
            settlement: *group_info.key,
            settlement_price_atomic: group.settlement_price_atomic,
            slot,
        }),
    )
}
