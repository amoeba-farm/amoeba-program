use super::*;
use crate::state::{WriterSettlementGroupStatus, WriterSleeveStatus};

const INITIALIZE_COLLECTIVE_PREFIX_ACCOUNTS: usize = 20;
const SET_COLLECTIVE_STATUS_ACCOUNTS: usize = 16;
pub(super) const COLLECTIVE_SWAP_FIXED_ACCOUNTS: usize = 31;
const SETTLE_COLLECTIVE_POOL_ACCOUNTS: usize = 6;

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

pub(super) fn validate_collective_pool_binding(
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
    let writer_sides =
        super::super::writer_sleeve::dlmm::activation_sides(program_id, accounts, &pool)?;
    let normalized = normalized_accounts(&accounts[..13], 6, 9);
    process_set_collective_pool_status_core(program_id, &normalized, params, writer_sides)
}

#[inline(never)]
pub(super) fn process_collective_swap_exact_in(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SwapAmoebaDlmmExactInV1Params,
) -> ProgramResult {
    if accounts.len() < COLLECTIVE_SWAP_FIXED_ACCOUNTS + compressed_delivery::COMPRESSION_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    let (accounts, compression) =
        accounts.split_at(accounts.len() - compressed_delivery::COMPRESSION_ACCOUNTS);
    // Optional canonical WriterCash observation follows pages, before delivery.
    // This supports the shared sleeve cash without moving or recounting it.
    let expected_cash = crate::compressed_custody::derive_compressed_custody(
        program_id,
        crate::compressed_custody::CustodyKind::WriterCash,
        accounts[27].key,
    )
    .0;
    let (accounts, cash_sidecar) = match accounts.split_last() {
        Some((last, prefix))
            if prefix.len() >= COLLECTIVE_SWAP_FIXED_ACCOUNTS && *last.key == expected_cash =>
        {
            (prefix, Some(last))
        }
        _ => (accounts, None),
    };
    // Existing trader/config/Market/month prefix, then sleeve/group/book before pool.
    let (context, book_context) =
        super::super::writer_sleeve::load_collective_dlmm_context_with_book(
            program_id,
            &accounts[4],
            &accounts[5],
            &accounts[6],
            &accounts[2],
            &accounts[3],
        )?;
    let config = super::swap::load_swap_config(program_id, &accounts[1])?;
    let pool = super::swap::load_swap_pool(program_id, &accounts[7])?;
    if pool.account_version == crate::dlmm_order_state::ORDER_POOL_VERSION {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_collective_pool_binding(&config, &accounts[2], &accounts[3], &context, &pool)?;
    if context.group_status != WriterSettlementGroupStatus::Active
        || !matches!(context.sleeve_status, WriterSleeveStatus::Active)
        || context.anchor_month_settled
    {
        return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
    }
    let ordinary_only = [2, 4, 6, 24, 26, 27, 28, 29]
        .iter()
        .all(|index| !accounts[*index].is_writable);
    let mut writer = if ordinary_only {
        None
    } else {
        super::super::writer_sleeve::dlmm::load_swap_state_with_cash(
            program_id,
            accounts,
            &pool,
            book_context,
            cash_sidecar,
            &context.market,
        )?
    };
    let normalized: Vec<_> = accounts[..4]
        .iter()
        .chain(accounts[7..23].iter())
        .chain(accounts[31..].iter())
        .cloned()
        .collect();
    process_collective_swap_exact_in_core_with_writer(
        program_id,
        &normalized,
        params,
        &mut writer,
        accounts,
        compression,
        super::swap::LoadedSwapState {
            config,
            pool,
            market: context.market,
            month: context.anchor_month,
        },
    )?;
    Ok(())
}

#[inline(never)]
pub(super) fn process_collective_compressed_swap_exact_in<'a>(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'a>],
    params: crate::ameba_dlmm_instruction::SwapCollectiveCompressedExactInV1Params,
) -> ProgramResult {
    process_collective_compressed_swap_with_authority(program_id, accounts, params, None)
}

pub(in crate::processor) fn process_trading_session_compressed_swap<'a>(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'a>],
    params: crate::ameba_dlmm_instruction::SwapCollectiveCompressedExactInV1Params,
    owner: Pubkey,
) -> ProgramResult {
    if accounts.first().map(|i| *i.key)
        != Some(crate::trading_session::derive(program_id, &owner).0)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    process_collective_compressed_swap_with_authority(program_id, accounts, params, Some(owner))
}

fn process_collective_compressed_swap_with_authority<'a>(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'a>],
    params: crate::ameba_dlmm_instruction::SwapCollectiveCompressedExactInV1Params,
    trading_owner: Option<Pubkey>,
) -> ProgramResult {
    let mut compressed = super::compressed_swap::parse(program_id, accounts, &params)?;
    compressed.trading_owner = trading_owner;
    let base = compressed.base;
    let (context, book_context) =
        super::super::writer_sleeve::load_collective_dlmm_context_with_book(
            program_id, &base[4], &base[5], &base[6], &base[2], &base[3],
        )?;
    let config = super::swap::load_swap_config(program_id, &base[1])?;
    let pool = super::swap::load_swap_pool(program_id, &base[7])?;
    if pool.account_version == crate::dlmm_order_state::ORDER_POOL_VERSION {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_collective_pool_binding(&config, &base[2], &base[3], &context, &pool)?;
    if context.group_status != WriterSettlementGroupStatus::Active
        || !matches!(context.sleeve_status, WriterSleeveStatus::Active)
        || context.anchor_month_settled
    {
        return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
    }
    let mut writer = super::super::writer_sleeve::dlmm::load_swap_state_with_cash(
        program_id,
        base,
        &pool,
        book_context,
        super::compressed_swap::existing_sidecar(program_id, compressed.writer_cash_custody)?,
        &context.market,
    )?;
    let normalized: Vec<_> = base[..4]
        .iter()
        .chain(base[7..23].iter())
        .chain(base[31..].iter())
        .cloned()
        .collect();
    super::swap::process_collective_swap_mode(
        program_id,
        &normalized,
        params.swap,
        &mut writer,
        base,
        &[],
        Some((&compressed, &params)),
        super::swap::LoadedSwapState {
            config,
            pool,
            market: context.market,
            month: context.anchor_month,
        },
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
