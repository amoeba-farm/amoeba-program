use super::*;
use crate::ameba_dlmm_math::AmoebaDlmmSwapDirection;
use crate::ameba_dlmm_state::{derive_ameba_dlmm_authority_pda, AmoebaDlmmPoolV1};
use crate::state::WriterDlmmBinV1;
use crate::writer_dlmm_math::{WriterDlmmBuybackLimits, WriterDlmmCash, WriterDlmmSeriesLimits};
use crate::writer_dlmm_quote::{WriterDlmmRouteQuote, WriterDlmmSwapPolicy};

pub(in crate::processor) struct WriterSwapState {
    context: WriterPolicyContext,
    policy: Box<WriterDlmmPolicyV1>,
    position: Box<WriterDlmmPositionV1>,
    market: Market,
    series_index: usize,
    series: Vec<WriterSeries>,
    series_limits: Vec<WriterDlmmSeriesLimits>,
    eligible: bool,
    before_cash: u64,
    before_fee: u64,
    before_mint_supply: u64,
    before_retirement: u64,
    round_trip_fee: u64,
}

impl WriterSwapState {
    pub(in crate::processor) fn bins(&self) -> &[WriterDlmmBinV1] {
        &self.position.bins[..usize::from(self.position.bin_count)]
    }
    pub(in crate::processor) fn quote_policy(&self, tick: u64) -> WriterDlmmSwapPolicy<'_> {
        WriterDlmmSwapPolicy {
            eligible: self.eligible,
            book: &self.series,
            series_index: self.series_index,
            cash: WriterDlmmCash {
                assets_atoms: self.context.sleeve.accounted_asset_atoms,
                principal_atoms: self.context.sleeve.writer_principal_atoms,
                allocated_lp_quote_atoms: self.policy.total_pool_quote_atoms
                    - self.policy.total_uncommitted_quote_atoms,
            },
            risk: risk_limits(&self.context.snapshot, &self.context.group),
            buyback: WriterDlmmBuybackLimits {
                monthly_buyback_cap_atoms: self.policy.monthly_buyback_cap_atoms,
                transaction_buyback_cap_atoms: self.policy.transaction_buyback_cap_atoms,
                reserve_release_spend_ratio_ppm: self.policy.reserve_release_spend_ratio_ppm,
                tick_size_quote_atoms: tick,
                price_separation_ticks: self.policy.price_separation_ticks,
                round_trip_fee_quote_atoms: self.round_trip_fee,
            },
            series_limits: &self.series_limits,
            month_spent_atoms: self.policy.monthly_spent_atoms,
            series_month_spent_atoms: self.policy.series_monthly_spent_atoms[self.series_index],
            primary_fee_bps: self.context.snapshot.primary_fee_bps,
        }
    }
}

/// Reads all writer companions from the current 32-account collective prefix.
/// A canonical absent writer lane remains ordinary liquidity, including historical sleeves.
pub(in crate::processor) fn load_swap_state(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    pool: &AmoebaDlmmPoolV1,
) -> Result<Option<WriterSwapState>, ProgramError> {
    if accounts.len() < 32 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let sleeve_info = &accounts[4];
    let policy_info = &accounts[24];
    let position_info = &accounts[26];
    let expected_policy = derive_writer_dlmm_policy_pda(program_id, sleeve_info.key).0;
    let expected_position =
        derive_writer_dlmm_position_pda(program_id, accounts[7].key, sleeve_info.key).0;
    if *policy_info.key != expected_policy || *position_info.key != expected_position {
        return Err(VaultError::InvalidPda.into());
    }
    if policy_info.owner != program_id {
        validate_canonical_system_zero_pda_proof(&expected_policy, policy_info)?;
        validate_canonical_system_zero_pda_proof(&expected_position, position_info)?;
        return Ok(None);
    }
    let context = load_writer_policy_context(
        program_id,
        sleeve_info,
        &accounts[5],
        &accounts[6],
        &accounts[25],
        Some(accounts[30].key),
    )?;
    let mut policy = load_policy(
        program_id,
        policy_info,
        sleeve_info,
        &context.snapshot,
        &context.book,
        true,
    )?;
    let index = context.book.records[..usize::from(context.book.series_count)]
        .iter()
        .position(|record| record.market == *accounts[2].key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    if position_info.owner != program_id {
        validate_canonical_system_zero_pda_proof(&expected_position, position_info)?;
        if policy.series_pool_inventory_atoms[index] != 0 {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        return Ok(None);
    }
    let position = load_position(
        program_id,
        position_info,
        accounts[7].key,
        sleeve_info.key,
        policy_info.key,
        accounts[2].key,
        index as u8,
    )?;
    if position.option_inventory_atoms != policy.series_pool_inventory_atoms[index]
        || policy.total_pool_quote_atoms
            < position
                .allocated_quote_atoms
                .checked_add(position.uncommitted_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?
        || position.bins[..usize::from(position.bin_count)]
            .iter()
            .any(|bin| bin.bin_id > pool.maximum_bin_id)
        || context.sleeve.vault_config != *accounts[1].key
        || context.sleeve.usdc_vault != *accounts[27].key
        || context.sleeve.policy_snapshot != *accounts[25].key
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    let registry = load_writer_policy_registry(program_id, &accounts[30], accounts[1].key)?;
    if registry.protocol_fee_vault != *accounts[31].key
        || derive_writer_protocol_fee_vault_pda(program_id, accounts[30].key).0 != *accounts[31].key
    {
        return Err(VaultError::InvalidWriterPolicyRegistry.into());
    }
    validate_vault_token_account(&accounts[31], accounts[10].key, accounts[30].key)?;
    validate_vault_token_account(&accounts[27], accounts[10].key, sleeve_info.key)?;
    let before_fee = validate_token_account(&accounts[31])?.amount;
    let before_cash = validate_token_account(&accounts[27])?.amount;
    let mut market = load_valid_market(program_id, &accounts[2])?;
    let mint = validate_canonical_market_mint(&accounts[2], &mut market, &accounts[9], 0)?;
    let staged = auction::observe_market_staging_amount(
        program_id,
        &accounts[2],
        &accounts[28],
        &accounts[9],
        &accounts[19],
    )?;
    let retired = auction::observe_writer_retirement_custody_amount(
        program_id,
        sleeve_info,
        &accounts[2],
        &accounts[29],
        &accounts[9],
        &accounts[19],
    )?;
    let record = &context.book.records[index];
    let issuer = position
        .option_inventory_atoms
        .checked_add(staged)
        .and_then(|value| value.checked_add(retired))
        .ok_or(VaultError::ArithmeticOverflow)?;
    // Supply reconciliation and writer cash deficits make only this lane ineligible.
    // Ordinary LP custody is checked independently by the shared pool loader.
    let mut eligible = context.sleeve.status == WriterSleeveStatus::Active
        && context.sleeve.active_close_request.is_none()
        && context.sleeve.active_auction.is_none()
        && context.group.status == WriterSettlementGroupStatus::Active
        && mint.supply == record.total_physical_supply_atoms
        && issuer == record.issuer_controlled_atoms
        && mint.supply.checked_sub(issuer) == Some(record.external_open_interest_atoms)
        && context
            .sleeve
            .accounted_asset_atoms
            .checked_sub(policy.total_pool_quote_atoms)
            .is_some_and(|cash| before_cash >= cash)
        && market_outstanding_contract_amount(&market)?
            == record
                .external_open_interest_atoms
                .checked_add(position.option_inventory_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
    let round_trip_fee = match crate::writer_dlmm_math::writer_dlmm_price_bounds(
        policy.series[index].seller_floor_quote_atoms,
        pool.tick_size_quote_atomic,
        policy.price_separation_ticks,
        pool.swap_fee_bps,
        context.snapshot.primary_fee_bps,
    ) {
        Ok((_, _, fee)) => fee,
        Err(_) => {
            eligible = false;
            0
        }
    };
    advance_spending_month(&mut policy, current_unix_timestamp()?)?;
    let series = writer_book_math_series(&context.book)?;
    let series_limits = policy.series[..usize::from(policy.series_count)]
        .iter()
        .map(|terms| WriterDlmmSeriesLimits {
            conservative_claim_value_atoms: terms.conservative_claim_value_atoms,
            seller_floor_quote_atoms: terms.seller_floor_quote_atoms,
            monthly_buyback_cap_atoms: terms.monthly_buyback_cap_atoms,
            transaction_buyback_cap_atoms: terms.transaction_buyback_cap_atoms,
        })
        .collect();
    Ok(Some(WriterSwapState {
        context,
        policy,
        position,
        market,
        series_index: index,
        series,
        series_limits,
        eligible,
        before_cash,
        before_fee,
        before_mint_supply: mint.supply,
        before_retirement: retired,
        round_trip_fee,
    }))
}

/// Apply only the already-admitted writer portion after the ordinary trader transfers.
/// Pool totals are adjusted for canonical sweeps/burns before final shared custody validation.
pub(in crate::processor) fn finish_swap(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &mut AmoebaDlmmPoolV1,
) -> ProgramResult {
    if route.writer_fills.is_empty() {
        return Ok(());
    }
    if !state.eligible {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let actor = &accounts[0];
    let market_info = &accounts[2];
    let sleeve_info = &accounts[4];
    let pool_info = &accounts[7];
    let authority_info = &accounts[8];
    let mint_info = &accounts[9];
    let option_vault = &accounts[11];
    let quote_vault = &accounts[12];
    let light_info = &accounts[15];
    let cpi_info = &accounts[16];
    let option_interface = &accounts[17];
    let quote_interface = &accounts[18];
    let token_info = &accounts[19];
    let system_info = &accounts[20];
    let cash_info = &accounts[27];
    let retirement_info = &accounts[29];
    let fee_info = &accounts[31];
    let (_, bump) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let bump_bytes = [bump];
    let pool_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        pool_info.key.as_ref(),
        &bump_bytes,
    ];
    let totals = route.writer;
    for fill in &route.writer_fills {
        let bin = state.position.bins[..usize::from(state.position.bin_count)]
            .iter_mut()
            .find(|bin| bin.bin_id == fill.bin_id)
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
        bin.option_atoms = fill.option_reserve_after;
        bin.quote_atoms = fill.quote_reserve_after;
    }
    let record = &mut state.context.book.records[state.series_index];
    match direction {
        AmoebaDlmmSwapDirection::QuoteForOption => {
            let writer_cash = totals
                .net_premium_atoms
                .checked_add(totals.lp_fee_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            let swept = totals
                .gross_premium_atoms
                .checked_add(totals.lp_fee_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if writer_cash.checked_add(totals.primary_fee_atoms) != Some(swept) {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            if writer_cash > 0 {
                invoke_light_token_account_transfer_with_signer_seeds(
                    writer_cash,
                    MarketMintAccounting::CANONICAL_DECIMALS,
                    light_info,
                    cpi_info,
                    actor,
                    quote_vault,
                    cash_info,
                    authority_info,
                    &accounts[10],
                    quote_interface,
                    token_info,
                    system_info,
                    &[pool_seeds],
                )?;
            }
            if totals.primary_fee_atoms > 0 {
                invoke_light_token_account_transfer_with_signer_seeds(
                    totals.primary_fee_atoms,
                    MarketMintAccounting::CANONICAL_DECIMALS,
                    light_info,
                    cpi_info,
                    actor,
                    quote_vault,
                    fee_info,
                    authority_info,
                    &accounts[10],
                    quote_interface,
                    token_info,
                    system_info,
                    &[pool_seeds],
                )?;
            }
            state.context.sleeve.accounted_asset_atoms = state
                .context
                .sleeve
                .accounted_asset_atoms
                .checked_add(writer_cash)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.context.sleeve.locked_primary_premium_atoms = state
                .context
                .sleeve
                .locked_primary_premium_atoms
                .checked_add(totals.net_premium_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            record.primary_premium_collected_atoms = record
                .primary_premium_collected_atoms
                .checked_add(totals.net_premium_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            record.external_open_interest_atoms = record
                .external_open_interest_atoms
                .checked_add(totals.sold_option_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            record.issuer_controlled_atoms = record
                .issuer_controlled_atoms
                .checked_sub(totals.sold_option_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.position.option_inventory_atoms = state
                .position
                .option_inventory_atoms
                .checked_sub(totals.sold_option_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            pool.accounted_quote_reserve = pool
                .accounted_quote_reserve
                .checked_sub(swept)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if validate_token_account(cash_info)?
                .amount
                .checked_sub(state.before_cash)
                != Some(writer_cash)
                || validate_token_account(fee_info)?
                    .amount
                    .checked_sub(state.before_fee)
                    != Some(totals.primary_fee_atoms)
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            let retirement = load_or_create_writer_retirement_custody(
                program_id,
                actor,
                sleeve_info,
                market_info,
                retirement_info,
                mint_info,
                token_info,
                system_info,
            )?;
            if retirement.amount != state.before_retirement {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            invoke_light_token_account_transfer_with_signer_seeds(
                totals.retired_option_atoms,
                MarketMintAccounting::CANONICAL_DECIMALS,
                light_info,
                cpi_info,
                actor,
                option_vault,
                retirement_info,
                authority_info,
                mint_info,
                option_interface,
                token_info,
                system_info,
                &[pool_seeds],
            )?;
            let sleeve_bump = [state.context.sleeve.bump];
            let sleeve_seeds =
                writer_sleeve_signer_seeds(&state.context.sleeve.settlement_group, &sleeve_bump);
            invoke_token_burn_checked(
                token_info,
                retirement_info,
                mint_info,
                sleeve_info,
                totals.retired_option_atoms,
                MarketMintAccounting::CANONICAL_DECIMALS,
                &[&sleeve_seeds],
            )?;
            if validate_token_account(retirement_info)?.amount != state.before_retirement {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            if state.before_retirement == 0 {
                funding::close_sleeve_token_custody(
                    &state.context.sleeve,
                    sleeve_info,
                    retirement_info,
                    actor,
                    token_info,
                )?;
            }
            consume_market_contracts(&mut state.market, totals.retired_option_atoms, true)?;
            record.total_physical_supply_atoms = record
                .total_physical_supply_atoms
                .checked_sub(totals.retired_option_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            record.external_open_interest_atoms = record
                .external_open_interest_atoms
                .checked_sub(totals.retired_option_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.context.sleeve.accounted_asset_atoms = state
                .context
                .sleeve
                .accounted_asset_atoms
                .checked_sub(totals.spent_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.position.allocated_quote_atoms = state
                .position
                .allocated_quote_atoms
                .checked_sub(totals.spent_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.policy.total_pool_quote_atoms = state
                .policy
                .total_pool_quote_atoms
                .checked_sub(totals.spent_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.policy.monthly_spent_atoms = state
                .policy
                .monthly_spent_atoms
                .checked_add(totals.spent_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            state.policy.series_monthly_spent_atoms[state.series_index] =
                state.policy.series_monthly_spent_atoms[state.series_index]
                    .checked_add(totals.spent_quote_atoms)
                    .ok_or(VaultError::ArithmeticOverflow)?;
            pool.accounted_option_reserve = pool
                .accounted_option_reserve
                .checked_sub(totals.retired_option_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
    }
    state.policy.series_pool_inventory_atoms[state.series_index] =
        state.position.option_inventory_atoms;
    record.custody_status = if record.issuer_controlled_atoms == 0 {
        WriterSeriesCustodyStatus::Closed
    } else {
        WriterSeriesCustodyStatus::Open
    };
    let mut count = 0;
    for index in 0..usize::from(state.position.bin_count) {
        let bin = state.position.bins[index];
        if bin.option_atoms != 0 || bin.quote_atoms != 0 {
            state.position.bins[count] = bin;
            count += 1;
        }
    }
    state.position.bins[count..].fill(WriterDlmmBinV1::default());
    state.position.bin_count = count as u8;
    if !state.position.has_current_layout()
        || !state.policy.has_current_layout()
        || validate_mint_account(mint_info, token_info.key)?.supply
            != record.total_physical_supply_atoms
        || state
            .before_mint_supply
            .checked_sub(totals.retired_option_atoms)
            != Some(record.total_physical_supply_atoms)
        || market_outstanding_contract_amount(&state.market)?
            != record
                .external_open_interest_atoms
                .checked_add(state.position.option_inventory_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    update_cash_metrics(
        &mut state.context.sleeve,
        &state.context.group,
        &state.context.book,
        &state.context.snapshot,
        &state.policy,
        true,
    )?;
    let slot = Clock::get()?.slot;
    state.context.sleeve.last_updated_slot = slot;
    state.context.book.last_updated_slot = slot;
    state.context.book.book_digest = writer_book_digest(&state.context.book);
    state.position.last_updated_slot = slot;
    store_state(&accounts[4], state.context.sleeve.as_ref())?;
    store_state(&accounts[6], state.context.book.as_ref())?;
    store_state(&accounts[24], state.policy.as_ref())?;
    store_state(&accounts[26], state.position.as_ref())?;
    store_state(market_info, &state.market)
}
