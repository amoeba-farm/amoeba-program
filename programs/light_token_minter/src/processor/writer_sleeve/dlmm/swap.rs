use super::*;
use crate::ameba_dlmm_math::AmoebaDlmmSwapDirection;
use crate::ameba_dlmm_state::{derive_ameba_dlmm_authority_pda, AmoebaDlmmPoolV1};
use crate::state::WriterDlmmBinV1;
use crate::writer_dlmm_math::{WriterDlmmBuybackLimits, WriterDlmmCash, WriterDlmmSeriesLimits};
use crate::writer_dlmm_quote::{WriterDlmmRouteQuote, WriterDlmmSwapPolicy};

pub(in crate::processor) struct WriterSwapState {
    pub(in crate::processor) context: WriterPolicyContext,
    pub(in crate::processor) policy: Box<WriterDlmmPolicyV1>,
    pub(in crate::processor) position: Box<WriterDlmmPositionV1>,
    pub(in crate::processor) market: Market,
    pub(in crate::processor) series_index: usize,
    pub(in crate::processor) series: Vec<WriterSeries>,
    pub(in crate::processor) series_limits: Vec<WriterDlmmSeriesLimits>,
    pub(in crate::processor) eligible: bool,
    pub(in crate::processor) before_cash: u64,
    pub(in crate::processor) before_hot_cash: u64,

    pub(in crate::processor) before_mint_supply: u64,
    pub(in crate::processor) before_retirement: u64,
    pub(in crate::processor) round_trip_fee: u64,
    pub(in crate::processor) risk: crate::writer_dlmm_math::WriterDlmmRiskLimits,
}

impl WriterSwapState {
    pub(in crate::processor) fn position(&self) -> &WriterDlmmPositionV1 {
        &self.position
    }
    pub(in crate::processor) fn bins(&self) -> &[WriterDlmmBinV1] {
        &self.position.bins[..usize::from(self.position.bin_count)]
    }
    pub(in crate::processor) fn quote_policy(&self, tick: u64) -> WriterDlmmSwapPolicy<'_> {
        WriterDlmmSwapPolicy {
            eligible: self.eligible,
            participation: self
                .context
                .sleeve
                .has_time_participation()
                .then(|| self.context.sleeve.participation_totals()),
            book: &self.series,
            series_index: self.series_index,
            cash: WriterDlmmCash {
                assets_atoms: self.context.sleeve.accounted_asset_atoms,
                principal_atoms: self.context.sleeve.writer_principal_atoms,
                allocated_lp_quote_atoms: self.policy.total_pool_quote_atoms
                    - self.policy.total_uncommitted_quote_atoms,
                pooled_quote_atoms: self.policy.total_pool_quote_atoms,
            },
            risk: self.risk,
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
        }
    }
}

/// Reads all writer companions from the current 32-account collective prefix,
/// plus the sleeve vault's canonical WriterCash custody when the route carries it.
/// A canonical absent writer lane remains ordinary liquidity, including historical sleeves.
pub(in crate::processor) fn load_swap_state_with_cash<'a, 'info>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'info>],
    pool: &AmoebaDlmmPoolV1,
    book_context: WriterBookContext,
    cash_sidecar_info: Option<&'a AccountInfo<'info>>,
    validated_market: &Market,
) -> Result<Option<WriterSwapState>, ProgramError> {
    if accounts.len() < 31 {
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
    let snapshot = load_writer_policy_snapshot(
        program_id,
        &accounts[25],
        sleeve_info.key,
        accounts[30].key,
        book_context.sleeve.policy_version,
    )?;
    let context = WriterPolicyContext {
        group: book_context.group,
        sleeve: book_context.sleeve,
        book: book_context.book,
        snapshot,
    };
    let policy = load_policy(
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
        &accounts[2],
        index as u8,
    )?;
    if context.sleeve.vault_config != *accounts[1].key
        || context.sleeve.usdc_vault != *accounts[27].key
        || context.sleeve.policy_snapshot != *accounts[25].key
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    let _registry = load_writer_policy_registry(program_id, &accounts[30], accounts[1].key)?;
    let mut market = *validated_market;
    let lane = observe_writer_lane(
        program_id,
        &WriterLaneAccounts {
            sleeve: sleeve_info,
            usdc_vault: &accounts[27],
            usdc_mint: accounts[10].key,
            cash_sidecar: cash_sidecar_info,
            market: &accounts[2],
            mint: &accounts[9],
            staging: &accounts[28],
            retirement: &accounts[29],
            token_program: &accounts[19],
        },
        &context.sleeve,
        &context.book,
        &policy,
        index,
        position.option_inventory_atoms,
        &mut market,
    )?;
    swap_state_from_authenticated(
        context,
        policy,
        position,
        market,
        pool,
        index,
        lane,
        current_unix_timestamp()?,
    )
    .map(Some)
}

/// Build the writer route view from canonical state and authenticated custody.
/// Physical loaders own PDA/owner/policy-chain/oracle checks and the observation
/// of issuer custody. Ordinary and resident routes share every financial check.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(in crate::processor) fn swap_state_from_authenticated(
    context: WriterPolicyContext,
    policy: Box<WriterDlmmPolicyV1>,
    position: Box<WriterDlmmPositionV1>,
    market: Market,
    pool: &AmoebaDlmmPoolV1,
    index: usize,
    lane: WriterLane,
    now: u64,
) -> Result<WriterSwapState, ProgramError> {
    swap_state_from_authenticated_view(
        context,
        policy,
        position,
        market,
        pool,
        index,
        lane,
        now,
        Vec::new(),
        Vec::new(),
        false,
    )
}

/// Retain the atomic action's authenticated quote view. Its geometry and policy
/// terms were loaded once, and every intervening canonical projection updates
/// the matching book and cached OI together. This is only for that owned view;
/// ordinary account loaders continue to reconstruct it in the strict wrapper.
/// The owning action must advance this policy spending month once at the same
/// captured `now` before constructing any cached chunk; projections preserve it.
#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn swap_state_from_authenticated_cached(
    context: WriterPolicyContext,
    policy: Box<WriterDlmmPolicyV1>,
    position: Box<WriterDlmmPositionV1>,
    market: Market,
    pool: &AmoebaDlmmPoolV1,
    index: usize,
    lane: WriterLane,
    now: u64,
    series: Vec<WriterSeries>,
    series_limits: Vec<WriterDlmmSeriesLimits>,
) -> Result<WriterSwapState, ProgramError> {
    swap_state_from_authenticated_view(
        context,
        policy,
        position,
        market,
        pool,
        index,
        lane,
        now,
        series,
        series_limits,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn swap_state_from_authenticated_view(
    context: WriterPolicyContext,
    mut policy: Box<WriterDlmmPolicyV1>,
    position: Box<WriterDlmmPositionV1>,
    market: Market,
    pool: &AmoebaDlmmPoolV1,
    index: usize,
    lane: WriterLane,
    now: u64,
    mut series: Vec<WriterSeries>,
    mut series_limits: Vec<WriterDlmmSeriesLimits>,
    cached: bool,
) -> Result<WriterSwapState, ProgramError> {
    if index >= usize::from(context.book.series_count)
        || index >= usize::from(policy.series_count)
        || usize::from(position.bin_count) > position.bins.len()
        || usize::from(position.series_index) != index
        || context.book.records[index].market != pool.market
        || position.market != pool.market
        || position.option_inventory_atoms != policy.series_pool_inventory_atoms[index]
        || policy.total_pool_quote_atoms
            < position
                .allocated_quote_atoms
                .checked_add(position.uncommitted_quote_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?
        || position.bins[..usize::from(position.bin_count)]
            .iter()
            .any(|bin| bin.bin_id > pool.maximum_bin_id)
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    // Supply reconciliation and writer cash deficits make only this lane ineligible.
    // Ordinary LP custody is checked independently by the shared pool loader.
    let mut eligible = lane.reconciled
        && context.sleeve.status == WriterSleeveStatus::Active
        && context.group.status == WriterSettlementGroupStatus::Active;
    let round_trip_fee = match crate::writer_dlmm_math::writer_dlmm_price_bounds(
        policy.series[index].seller_floor_quote_atoms,
        pool.tick_size_quote_atomic,
        policy.price_separation_ticks,
    ) {
        Ok((_, _, fee)) => fee,
        Err(_) => {
            eligible = false;
            0
        }
    };
    if !cached {
        advance_spending_month(&mut policy, now)?;
    }
    let count = usize::from(context.book.series_count);
    if count == 0 || count > crate::constants::WRITER_MAX_LIVE_SERIES {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    if cached {
        if series.len() != count
            || series_limits.len() != usize::from(policy.series_count)
            || series[index].external_oi_atoms
                != context.book.records[index].external_open_interest_atoms
        {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
    } else {
        series.clear();
        for record in &context.book.records[..count] {
            if !record.active {
                return Err(VaultError::InvalidWriterSeriesBook.into());
            }
            series.push(WriterSeries {
                kind: record.option_kind,
                strike_price_atomic: record.strike_price_atomic,
                cap_price_atomic: record.cap_or_floor_price_atomic,
                contract_size_atoms: record.contract_size_atoms,
                max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
                external_oi_atoms: record.external_open_interest_atoms,
            });
        }
        series_limits.clear();
        series_limits.extend(
            policy.series[..usize::from(policy.series_count)]
                .iter()
                .map(|terms| WriterDlmmSeriesLimits {
                    conservative_claim_value_atoms: terms.conservative_claim_value_atoms,
                    seller_floor_quote_atoms: terms.seller_floor_quote_atoms,
                    monthly_buyback_cap_atoms: terms.monthly_buyback_cap_atoms,
                    transaction_buyback_cap_atoms: terms.transaction_buyback_cap_atoms,
                }),
        );
    }
    let risk = risk_limits(&context.snapshot, &context.group);
    Ok(WriterSwapState {
        context,
        policy,
        position,
        market,
        series_index: index,
        series,
        series_limits,
        eligible,
        before_cash: lane.before_cash,
        before_hot_cash: lane.before_hot_cash,

        before_mint_supply: lane.mint_supply,
        before_retirement: lane.retired,
        round_trip_fee,
        risk,
    })
}

/// The accounts a writer lane's custody, supply and cash are read from.
pub(in crate::processor) struct WriterLaneAccounts<'a, 'info> {
    pub(in crate::processor) sleeve: &'a AccountInfo<'info>,
    pub(in crate::processor) usdc_vault: &'a AccountInfo<'info>,
    pub(in crate::processor) usdc_mint: &'a Pubkey,
    /// The vault's canonical compressed WriterCash sidecar, when supplied.
    pub(in crate::processor) cash_sidecar: Option<&'a AccountInfo<'info>>,
    pub(in crate::processor) market: &'a AccountInfo<'info>,
    pub(in crate::processor) mint: &'a AccountInfo<'info>,
    pub(in crate::processor) staging: &'a AccountInfo<'info>,
    pub(in crate::processor) retirement: &'a AccountInfo<'info>,
    pub(in crate::processor) token_program: &'a AccountInfo<'info>,
}

pub(in crate::processor) struct WriterLane {
    /// Status-independent: supply, issuer custody, external interest, Market
    /// outstanding and cash backing all reconcile (`writer_lane_reconciled`).
    pub(in crate::processor) reconciled: bool,
    /// Hot vault plus the canonical WriterCash sidecar's quote atoms.
    pub(in crate::processor) before_cash: u64,
    pub(in crate::processor) before_hot_cash: u64,
    pub(in crate::processor) mint_supply: u64,
    pub(in crate::processor) staged: u64,
    pub(in crate::processor) retired: u64,
}

/// The one observation of a writer lane shared by every writer swap and every
/// writer liquidity action (add, remove, sweep, relocate), so the two paths
/// cannot drift. Writer cash is the hot sleeve vault plus the canonical
/// compressed WriterCash sidecar when it is supplied; omitting an existing
/// sidecar only understates cash (fails closed). A swap makes only this lane
/// ineligible when it does not reconcile; a liquidity action is rejected.
/// `market` is the caller's loaded `a.market`; its canonical mint is validated here.
#[inline(never)]
// Keep the existing accounting interface and its explicit inputs.
#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn observe_writer_lane(
    program_id: &Pubkey,
    a: &WriterLaneAccounts,
    sleeve: &WriterSleeveV1,
    book: &WriterSeriesBookV1,
    policy: &WriterDlmmPolicyV1,
    index: usize,
    inventory: u64,
    market: &mut Market,
) -> Result<WriterLane, ProgramError> {
    validate_vault_token_account(a.usdc_vault, a.usdc_mint, a.sleeve.key)?;
    let before_hot_cash = validate_token_account(a.usdc_vault)?.amount;
    let before_cash = before_hot_cash
        .checked_add(crate::compressed_custody::writer_cash(
            program_id,
            a.cash_sidecar,
            a.usdc_vault.key,
            a.usdc_mint,
        )?)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let mint = validate_canonical_market_mint(a.market, market, a.mint, 0)?;
    let staged = custody::observe_market_staging_amount(
        program_id,
        a.market,
        a.staging,
        a.mint,
        a.token_program,
    )?;
    let retired = custody::observe_writer_retirement_custody_amount(
        program_id,
        a.sleeve,
        a.market,
        a.retirement,
        a.mint,
        a.token_program,
    )?;
    Ok(WriterLane {
        reconciled: writer_lane_reconciled(
            book,
            index,
            inventory,
            staged.checked_add(retired),
            mint.supply,
            market_outstanding_contract_amount(market)?,
            sleeve.accounted_asset_atoms,
            policy.total_pool_quote_atoms,
            before_cash,
        ),
        before_cash,
        before_hot_cash,
        mint_supply: mint.supply,
        staged,
        retired,
    })
}

/// The series book's identities for one writer lane:
/// physical supply = issuer custody (pool inventory + staging + retirement
/// custody) + external interest (managed + individual) + compressed-retired +
/// forfeited, Market outstanding = external interest + pool inventory, and the
/// observed writer cash (hot + WriterCash sidecar) backs every accounted asset
/// not lent to the pool.
#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn writer_lane_reconciled(
    book: &WriterSeriesBookV1,
    index: usize,
    inventory: u64,
    staged_and_retired: Option<u64>,
    mint_supply: u64,
    market_outstanding: u64,
    accounted_assets: u64,
    pooled_quote: u64,
    cash: u64,
) -> bool {
    let record = &book.records[index];
    let (Some(issuer), Some(external)) = (
        staged_and_retired.and_then(|value| value.checked_add(inventory)),
        book.external_total(index),
    ) else {
        return false;
    };
    mint_supply == record.total_physical_supply_atoms
        && issuer == record.issuer_controlled_atoms
        && mint_supply
            .checked_sub(issuer)
            .and_then(|v| v.checked_sub(book.individual.compressed_retired_atoms[index]))
            .and_then(|v| v.checked_sub(book.individual.forfeited_atoms[index]))
            == Some(external)
        && accounted_assets
            .checked_sub(pooled_quote)
            .is_some_and(|required| cash >= required)
        && external.checked_add(inventory) == Some(market_outstanding)
}

/// Project the normal writer portion without token CPIs or account writes.
/// The caller quotes against authenticated state, projects Pool/FIFO effects,
/// then applies this same accounting before batching the physical transfers.
/// `slot` is the instruction's runtime slot; compressed retirement preserves
/// physical mint supply while recording the retired contracts in the book.
pub(in crate::processor) fn project_swap_effects(
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &mut AmoebaDlmmPoolV1,
    retirement_is_compressed: bool,
    slot: u64,
) -> ProgramResult {
    if route.writer_fills.is_empty() {
        return Ok(());
    }
    project_swap_effects_deferred(
        state,
        route,
        direction,
        pool,
        retirement_is_compressed,
        slot,
    )?;
    finalize_projected_swap(state, route)
}

/// Apply the exact admitted financial effects while leaving derived reserve metrics and
/// the book digest for the atomic caller's final state. Every progressive quote must still
/// admit its updated book and cash; before storing, the caller must recompute those metrics
/// with the last admitted route and then refresh the book digest. This permits several
/// source chunks in one instruction without repeatedly hashing the entire writer book.
pub(in crate::processor) fn project_swap_effects_deferred(
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &mut AmoebaDlmmPoolV1,
    retirement_is_compressed: bool,
    slot: u64,
) -> ProgramResult {
    project_swap_effects_inner(
        state,
        route,
        direction,
        pool,
        retirement_is_compressed,
        slot,
        true,
    )
}

/// The private atomic path validates the complete policy once after every
/// progressive projection and before its first CPI. Per-fill price, solvency,
/// spending, position layout and physical-supply checks remain unchanged.
pub(in crate::processor) fn project_swap_effects_cached(
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &mut AmoebaDlmmPoolV1,
    retirement_is_compressed: bool,
    slot: u64,
) -> ProgramResult {
    project_swap_effects_inner(
        state,
        route,
        direction,
        pool,
        retirement_is_compressed,
        slot,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn project_swap_effects_inner(
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &mut AmoebaDlmmPoolV1,
    retirement_is_compressed: bool,
    slot: u64,
    validate_policy_layout: bool,
) -> ProgramResult {
    if route.writer_fills.is_empty() {
        return Ok(());
    }
    if !state.eligible {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
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
            if writer_cash != swept {
                return Err(VaultError::WriterSupplyMismatch.into());
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
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            if retirement_is_compressed {
                // The caller moves this exact input into program-bound retirement
                // custody. Physical mint supply stays outstanding until cleanup.
                state.context.book.individual.compressed_retired_atoms[state.series_index] =
                    state.context.book.individual.compressed_retired_atoms[state.series_index]
                        .checked_add(totals.retired_option_atoms)
                        .ok_or(VaultError::ArithmeticOverflow)?;
            }
            consume_market_contracts(
                &mut state.market,
                totals.retired_option_atoms,
                !retirement_is_compressed,
            )?;
            if !retirement_is_compressed {
                record.total_physical_supply_atoms = record
                    .total_physical_supply_atoms
                    .checked_sub(totals.retired_option_atoms)
                    .ok_or(VaultError::ArithmeticOverflow)?;
            }
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
        || (validate_policy_layout && !state.policy.has_current_layout())
        || market_outstanding_contract_amount(&state.market)?
            != state
                .context
                .book
                .external_total(state.series_index)
                .and_then(|value| value.checked_add(state.position.option_inventory_atoms))
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    state.context.sleeve.last_updated_slot = slot;
    state.context.book.last_updated_slot = slot;
    state.position.last_updated_slot = slot;
    // Only this series' OI changed; payoff terms and the other series are immutable
    // throughout this projection. Retain the allocation across progressive chunks.
    if state.series.len() != usize::from(state.context.book.series_count) {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    state.series[state.series_index].external_oi_atoms =
        state.context.book.records[state.series_index].external_open_interest_atoms;
    Ok(())
}

fn finalize_projected_swap(
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
) -> ProgramResult {
    update_cash_metrics_with_admission(
        &mut state.context.sleeve,
        &state.context.group,
        &state.context.book,
        &state.context.snapshot,
        &state.policy,
        true,
        route.admitted_cash(),
    )?;
    state.context.book.book_digest = writer_book_digest(&state.context.book);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn finish_swap_with_cash(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    state: &mut WriterSwapState,
    route: &WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    pool: &mut AmoebaDlmmPoolV1,
    cash_sidecar_info: Option<&AccountInfo>,
    premium_is_compressed: bool,
    retirement_is_compressed: bool,
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

    let (_, bump) = derive_ameba_dlmm_authority_pda(program_id, pool_info.key);
    let bump_bytes = [bump];
    let pool_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::AMOEBA_DLMM_AUTHORITY_PDA_SEED,
        pool_info.key.as_ref(),
        &bump_bytes,
    ];
    let totals = route.writer;
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
            if writer_cash != swept {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            if writer_cash > 0 && !premium_is_compressed {
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

            let hot_after = validate_token_account(cash_info)?.amount;
            let compressed_after = crate::compressed_custody::load(
                program_id,
                cash_sidecar_info,
                crate::compressed_custody::CustodyKind::WriterCash,
                cash_info.key,
                &Pubkey::default(),
                &pool.quote_mint,
            )?
            // Classic delivery does not move the validated compressed balance.
            .map_or(state.before_cash - state.before_hot_cash, |cash| {
                cash.quote_atoms
            });
            if hot_after
                .checked_add(compressed_after)
                .and_then(|total| total.checked_sub(state.before_cash))
                != Some(writer_cash)
                || (premium_is_compressed && hot_after != state.before_hot_cash)
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
        }
        AmoebaDlmmSwapDirection::OptionForQuote => {
            if !retirement_is_compressed {
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
                let sleeve_seeds = writer_sleeve_signer_seeds(
                    &state.context.sleeve.settlement_group,
                    &sleeve_bump,
                );
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
                    custody::close_sleeve_token_custody(
                        &state.context.sleeve,
                        sleeve_info,
                        retirement_info,
                        actor,
                        token_info,
                    )?;
                }
            }
        }
    }
    project_swap_effects(
        state,
        route,
        direction,
        pool,
        retirement_is_compressed,
        crate::compact_error::slot()?,
    )?;
    let record = &state.context.book.records[state.series_index];
    if !state.position.has_current_layout()
        || !state.policy.has_current_layout()
        || validate_mint_account(mint_info, token_info.key)?.supply
            != record.total_physical_supply_atoms
        || state
            .before_mint_supply
            .checked_sub(if retirement_is_compressed {
                0
            } else {
                totals.retired_option_atoms
            })
            != Some(record.total_physical_supply_atoms)
        || market_outstanding_contract_amount(&state.market)?
            != record
                .external_open_interest_atoms
                .checked_add(state.context.book.individual.series[state.series_index].outstanding)
                .and_then(|v| v.checked_add(state.position.option_inventory_atoms))
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    store_state(&accounts[4], state.context.sleeve.as_ref())?;
    store_state(&accounts[6], state.context.book.as_ref())?;
    store_state(&accounts[24], state.policy.as_ref())?;
    // The ordinary router commits the staged writer position together with
    // pool/page/order balances. An imported physical account is only a marker.
    if !resident::position_is_resident(program_id, market_info, state.position.as_ref())? {
        store_state(&accounts[26], state.position.as_ref())?;
    }
    store_state(market_info, &state.market)
}
