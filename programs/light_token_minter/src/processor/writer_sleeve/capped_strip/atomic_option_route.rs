//! One financial executor over the existing normal option-market sources.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::{
    ameba_dlmm_math::AmoebaDlmmSwapDirection,
    ameba_dlmm_state::AmoebaDlmmPoolStatus,
    atomic_option_quote::AtomicOptionChunk,
    atomic_option_route as wire, market_router as router, multi_order as mo,
    state::{WriterDlmmBinV1, WriterDlmmPolicyV1},
    writer_dlmm_math::{WriterDlmmBuybackLimits, WriterDlmmCash, WriterDlmmSeriesLimits},
    writer_dlmm_quote::{PublicOrderRouteLimits, WriterDlmmRouteConfig, WriterDlmmSwapPolicy},
};
mod primary_only;
mod projection;
mod settlement;
#[path = "atomic_option_route_sources.rs"]
mod sources;
pub(in crate::processor) use projection::prepare as prepare_projection;

/// Move authenticated large states through the ordinary projector without
/// cloning them into Solana's non-reclaiming transaction heap for every asset.
/// A missing value exists only during that projector call; any error aborts the
/// instruction before persistence or settlement.
struct Owned<T>(Option<T>);
impl<T> Owned<T> {
    fn new(value: T) -> Self {
        Self(Some(value))
    }
    fn take(&mut self) -> Result<T, ProgramError> {
        self.0.take().ok_or_else(invalid)
    }
    fn restore(&mut self, value: T) {
        self.0 = Some(value);
    }
}
impl<T> core::ops::Deref for Owned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("authenticated state present")
    }
}
impl<T> core::ops::DerefMut for Owned<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("authenticated state present")
    }
}
struct Context {
    state: Owned<dlmm::WriterPolicyContext>,
    policy: Owned<Box<WriterDlmmPolicyV1>>,
    lane: Option<Box<Lane>>,
    cash: cash_custody::CompressedCustodyV1,
    hot_before: u64,
    cold_before: u64,
    quote_delta: i128,
    action_spent: u64,
    series_spent: Vec<u64>,
    series: Vec<crate::writer_sleeve_math::WriterSeries>,
    terms: Vec<WriterDlmmSeriesLimits>,
    grid: crate::writer_sleeve_math::PreparedWriterStrip,
    // Retain allocations across assets. prepare_into authenticates the current
    // complete book and target before reusing the payoff and group buffers.
    prepared_cache: Option<crate::writer_sleeve_math::PreparedWriterReserve>,
    last_admitted: Option<crate::writer_dlmm_quote::AdmittedWriterCash>,
    anchor_month: Option<Box<OracleMonthState>>,
    writer_updated: bool,
    primary_updated: bool,
}
struct Asset {
    resident: router::ResidentRouterState,
    resident_present: bool,
    // End of the strictly validated source envelope, before any account write.
    source_used_len: usize,
    market: Box<Market>,
    series_index: usize,
    mint_before: u64,
    staging_before: u64,
    retired_before: u64,
    fresh: u64,
    retired: u64,
    quote_delta: i128,
    option_before: u64,
    quote_before: u64,
    normal_updated: bool,
    writer_updated: bool,
    touched_pages: Vec<u16>,
}
struct Inputs {
    escrow: Vec<u64>,
    markets: Vec<[u64; 2]>,
    cash: Vec<u64>,
}
struct ProjectedAction {
    p: wire::Fill,
    order: mo::Order,
    net: Vec<mo::Asset>,
    inputs: Inputs,
    contexts: Vec<Context>,
    assets: Vec<Asset>,
    asset_start: usize,
    delegate_start: usize,
    custody_start: usize,
    merkle_start: usize,
    quote_delta: i128,
    fee: u64,
    slot: u64,
}
/// The caller has already loaded and authenticated this exact Market prefix.
/// Retain full resident source identity validation while avoiding a second
/// Market decode and PDA search for every leg.
fn load_resident(
    program: &Pubkey,
    info: &AccountInfo,
    market: &Market,
    policy: Option<&Pubkey>,
    range: core::ops::Range<usize>,
) -> Result<router::ResidentRouterState, ProgramError> {
    // No external source may supply cached identities: only verified imports
    // and validated ordinary commits write this program-owned Market payload.
    // Financial projection changes amounts/links while preserving identities.
    if info.owner != program || info.executable {
        return Err(invalid());
    }
    let data = info.try_data()?;
    // The private Market loader validated this range and every padding byte.
    // There is no CPI or account-data write between that load and this decode.
    let payload = data.get(range).ok_or_else(invalid)?;
    let state = router::ResidentRouterState::try_from_slice(payload).map_err(|_| invalid())?;
    state.validate_financial_route(program, info.key, market, policy)?;
    Ok(state)
}
/// Program-owned Markets acquire their canonical bump at creation. All later
/// prefix writes preserve that immutable identity; the funded Order also binds
/// this exact key. Validate that identity without repeating the bump search.
fn load_market_for_route(
    program: &Pubkey,
    info: &AccountInfo,
) -> Result<(Market, Option<core::ops::Range<usize>>), ProgramError> {
    if info.owner != program || info.executable {
        return Err(invalid());
    }
    let data = info.try_data()?;
    let range = crate::market_router_account::payload_range(&data)
        .map_err(|_| VaultError::InvalidConfigAccount)?;
    let prefix = data
        .get(..Market::LEN)
        .ok_or(VaultError::InvalidConfigAccount)?;
    // SAFETY: the complete envelope and exact canonical Market prefix were
    // checked above, identically to the public load_state::<Market> path.
    let market = unsafe { <Market as crate::fixed_codec::FixedStateDecode>::decode_fixed(prefix) }
        .map_err(|_| VaultError::InvalidConfigAccount)?;
    let bump = [market.bump];
    let expected = Pubkey::create_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            MARKET_PDA_SEED,
            &market.market_id,
            &bump,
        ],
        program,
    )
    .map_err(|_| invalid())?;
    if *info.key != expected
        || !market.is_initialized
        || market_outstanding_contract_amount(&market).is_err()
    {
        return Err(VaultError::InvalidMarketAccount.into());
    }
    Ok((market, range))
}
fn add_signed(value: u64, delta: i128) -> Result<u64, ProgramError> {
    i128::from(value)
        .checked_add(delta)
        .and_then(|v| u64::try_from(v).ok())
        .ok_or_else(invalid)
}
fn context_role(context: usize) -> usize {
    wire::COMMON + context * wire::CONTEXT_ACCOUNTS
}

fn parse_inputs(p: &wire::Fill, count: usize) -> Result<Inputs, ProgramError> {
    let mut result = Inputs {
        escrow: vec![0; count + 1],
        markets: vec![[0; 2]; count],
        cash: vec![0; usize::from(p.contexts)],
    };
    let mut seen = Vec::new();
    for batch in &p.batches {
        if batch.output_tree == batch.output_queue
            || batch.output_tree >= p.merkle_accounts
            || batch.output_queue >= p.merkle_accounts
        {
            return Err(invalid());
        }
        for input in &batch.inputs {
            let asset = usize::from(input.asset);
            let w = &input.witness;
            if input.amount == 0
                || asset > count
                || w.tree_index >= p.merkle_accounts
                || w.queue_index >= p.merkle_accounts
                || w.tree_index == w.queue_index
                || batch.proof.is_none() && !w.prove_by_index
                || seen.contains(&(w.tree_index, w.queue_index, w.leaf_index))
            {
                return Err(invalid());
            }
            seen.push((w.tree_index, w.queue_index, w.leaf_index));
            let total = match input.source {
                wire::Source::Escrow => &mut result.escrow[asset],
                wire::Source::Market(i) => {
                    let i = usize::from(i);
                    if i >= count || asset != 0 && asset != i + 1 {
                        return Err(invalid());
                    }
                    &mut result.markets[i][usize::from(asset != 0)]
                }
                wire::Source::WriterCash(i) => {
                    if asset != 0 || i >= p.contexts {
                        return Err(invalid());
                    }
                    &mut result.cash[usize::from(i)]
                }
            };
            *total = checked(total.checked_add(input.amount))?;
        }
    }
    Ok(result)
}

/// Base14: custody owner, funded Order, SOL payer, config, quote mint,
/// Light Token/CPI/SPL/System, Light System/registered/compression authority/
/// compression program, quote interface. Context10: sleeve/group/series book/
/// snapshot/policy/registry/optional primary lane/anchor month/hot cash/cash
/// custody. Four per net asset: live resident Market/mint/staging/interface.
/// Owner delegates, optional classic retirement observations, Merkle tail.
pub(in crate::processor) fn fill(
    program: &Pubkey,
    a: &[AccountInfo],
    p: wire::Fill,
) -> ProgramResult {
    if projection::is_cache(a, &p)? {
        return projection::execute(
            program,
            a,
            crate::atomic_projection::ProjectionAction::Fill(p),
        );
    }
    execute(program, a, p, None)
}
pub(in crate::processor) fn close(
    program: &Pubkey,
    a: &[AccountInfo],
    p: wire::Close,
    owner: Pubkey,
) -> ProgramResult {
    if a.len() < wire::COMMON || current_unix_timestamp()? > p.deadline_ts {
        return Err(invalid());
    }
    let order = crate::processor::multi_order::load(program, &a[1], a[0].key, &p.route.nonce)?;
    // This immediate close consumes the complete held position. Generic funded
    // mixed buy/sell orders retain their unrestricted Fill/Cancel lifecycle.
    if order.owner != owner
        || mo::assets(&order.legs)
            .ok_or_else(invalid)?
            .iter()
            .any(|x| x.delta <= 0)
    {
        return Err(invalid());
    }
    if projection::is_cache(a, &p.route)? {
        return projection::execute(
            program,
            a,
            crate::atomic_projection::ProjectionAction::Close(p),
        );
    }
    execute(
        program,
        a,
        p.route,
        Some((p.minimum_net_quote, &p.delegated_inputs, p.pay_fee_in_usdc)),
    )
}
fn project(
    program: &Pubkey,
    a: &[AccountInfo],
    mut p: wire::Fill,
    close: Option<(u64, &[u8], bool)>,
    readonly: bool,
    prepared_proofs: Option<&[[u8; 128]]>,
) -> Result<ProjectedAction, ProgramError> {
    if a.len() < wire::COMMON
        || !a[2].is_signer
        || !a[2].is_writable
        || !readonly && !a[1].is_writable
        || a[1].is_signer
        || !crate::light_token_instruction::is_program(a[5].key)
        || !crate::light_token_instruction::is_cpi_authority(a[6].key)
        || !crate::token_instruction::check_id(a[7].key)
        || !crate::is_system_program(a[8].key)
        || !crate::light_token_instruction::is_light_system_program(a[9].key)
        || !crate::light_token_instruction::is_registered_program(a[10].key)
        || !crate::light_token_instruction::is_compression_authority(a[11].key)
        || !crate::light_token_instruction::is_compression_program(a[12].key)
    {
        return Err(invalid());
    }
    let order = if readonly {
        crate::processor::multi_order::load_readonly(program, &a[1], a[0].key, &p.nonce)?
    } else {
        crate::processor::multi_order::load(program, &a[1], a[0].key, &p.nonce)?
    };
    let net = mo::assets(&order.legs).ok_or_else(invalid)?;
    let count = net.len();
    let asset_start = wire::COMMON + usize::from(p.contexts) * wire::CONTEXT_ACCOUNTS;
    let delegate_start = asset_start + 4 * count;
    let custody_start = delegate_start + usize::from(p.delegate_accounts);
    let proof_start = custody_start + usize::from(p.custody_accounts);
    let merkle_start = proof_start + usize::from(p.proof_accounts);
    let now = current_unix_timestamp()?;
    if p.legs.len() != count
        || p.batches.is_empty()
        || a.len() != merkle_start + usize::from(p.merkle_accounts)
        || a.len() > 255
        || p.merkle_accounts < 2
        || *a[4].key != order.quote_mint
        || order.status
            != if close.is_some() {
                mo::FILLED
            } else {
                mo::OPEN
            }
        || close.is_none() && now >= order.expiry_ts
        || a[merkle_start..].iter().enumerate().any(|(i, v)| {
            !readonly && !v.is_writable
                || v.is_signer
                || v.executable
                || a[merkle_start..merkle_start + i]
                    .iter()
                    .any(|q| q.key == v.key)
        })
    {
        return Err(invalid());
    }
    let mut used_proofs = vec![false; usize::from(p.proof_accounts)];
    let batch_count = p.batches.len();
    for (ordinal, batch) in p.batches.iter_mut().enumerate() {
        if batch.proof_account == crate::atomic_proof::INLINE {
            continue;
        }
        let index = usize::from(batch.proof_account);
        if batch.proof.is_some() || index >= used_proofs.len() {
            return Err(invalid());
        }
        batch.proof = Some(if let Some(proofs) = prepared_proofs {
            if index != 0 || proofs.len() != batch_count {
                return Err(invalid());
            }
            *proofs.get(ordinal).ok_or_else(invalid)?
        } else {
            crate::atomic_proof::load_for_batch(
                program,
                &a[proof_start + index],
                ordinal,
                batch_count,
            )?
        });
        batch.proof_account = crate::atomic_proof::INLINE;
        used_proofs[index] = true;
    }
    if prepared_proofs.is_some() && p.proof_accounts == 1 {
        used_proofs[0] = true;
    }
    if used_proofs.iter().any(|used| !*used) {
        return Err(invalid());
    }
    let inputs = parse_inputs(&p, count)?;
    if let Some((_, delegation, _)) = close {
        if delegation.len() != p.batches.iter().map(|b| b.inputs.len()).sum::<usize>()
            || inputs.escrow[0] != 0
        {
            return Err(invalid());
        }
        for (input, kind) in p.batches.iter().flat_map(|b| &b.inputs).zip(delegation) {
            if *kind > 2 || *kind != 0 && (input.source != wire::Source::Escrow || input.asset == 0)
            {
                return Err(invalid());
            }
        }
    } else if p.delegate_accounts != 0 || p.legs.iter().any(|l| l.delegate_index != 255) {
        return Err(invalid());
    }
    if close.is_none() && inputs.escrow[0] < order.quote_bound.escrow() {
        return Err(invalid());
    }
    let config = load_canonical_vault_config(program, &a[3])?;
    validate_collateral_mint_account(&a[4], a[7].key)?;
    let mut context_used = vec![false; usize::from(p.contexts)];
    for leg in &p.legs {
        if leg.context_index == wire::NO_CONTEXT {
            continue;
        }
        let i = usize::from(leg.context_index);
        if i >= context_used.len() {
            return Err(invalid());
        }
        context_used[i] = true;
    }
    if context_used.iter().any(|v| !*v) {
        return Err(invalid());
    }
    let mut contexts = Vec::with_capacity(usize::from(p.contexts));
    for c in 0..usize::from(p.contexts) {
        let n = context_role(c);
        if !readonly && [0, 2, 4, 8, 9].iter().any(|i| !a[n + i].is_writable)
            || (0..c).any(|j| a[context_role(j)].key == a[n].key)
        {
            return Err(invalid());
        }
        let state = load_writer_policy_context(
            program,
            &a[n],
            &a[n + 1],
            &a[n + 2],
            &a[n + 3],
            Some(a[n + 5].key),
        )?;
        let mut policy = dlmm::load_policy(
            program,
            &a[n + 4],
            &a[n],
            &state.snapshot,
            &state.book,
            true,
        )?;
        load_writer_policy_registry(program, &a[n + 5], a[3].key)?;
        if state.sleeve.vault_config != *a[3].key || state.sleeve.usdc_vault != *a[n + 8].key {
            return Err(invalid());
        }
        validate_vault_token_account(&a[n + 8], a[4].key, a[n].key)?;
        let hot_before = validate_token_account(&a[n + 8])?.amount;
        let cash = cash_state_checked(program, &a[n + 9], a[n + 8].key, a[4].key, !readonly)?;
        if cash.option_atoms != 0 {
            return Err(invalid());
        }
        dlmm::advance_spending_month(&mut policy, now)?;
        let expected_lane = strip::derive(program, a[n].key).0;
        if *a[n + 6].key != expected_lane {
            return Err(invalid());
        }
        let lane = if crate::is_system_program(a[n + 6].owner) && a[n + 6].data_is_empty() {
            None
        } else {
            Some(load_checked(
                program,
                &a[n + 6],
                a[n].key,
                a[n + 1].key,
                a[n + 2].key,
                Some(&policy.rolling_policy_hash),
                state.book.series_count,
                !readonly,
            )?)
        };
        let series = writer_book_math_series(&state.book)?;
        let targets = p
            .legs
            .iter()
            .enumerate()
            .filter(|(_, l)| usize::from(l.context_index) == c)
            .map(|(i, _)| {
                state.book.records[..usize::from(state.book.series_count)]
                    .iter()
                    .position(|r| r.market == net[i].market)
                    .ok_or_else(invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let risk = dlmm::risk_limits(&state.snapshot, &state.group);
        let grid = crate::writer_sleeve_math::PreparedWriterStrip::for_targets(
            &series,
            risk.lower_tail_max_settlement_atomic,
            risk.upper_tail_min_settlement_atomic,
            &targets,
        )
        .map_err(writer_math_error)?;
        let series_spent = vec![0; series.len()];
        let terms = policy.series[..series.len()]
            .iter()
            .map(|t| WriterDlmmSeriesLimits {
                conservative_claim_value_atoms: t.conservative_claim_value_atoms,
                seller_floor_quote_atoms: t.seller_floor_quote_atoms,
                monthly_buyback_cap_atoms: t.monthly_buyback_cap_atoms,
                transaction_buyback_cap_atoms: t.transaction_buyback_cap_atoms,
            })
            .collect();
        contexts.push(Context {
            cold_before: cash.quote_atoms,
            state: Owned::new(state),
            policy: Owned::new(policy),
            lane,
            cash,
            hot_before,
            quote_delta: 0,
            action_spent: 0,
            series_spent,
            series,
            terms,
            grid,
            prepared_cache: None,
            last_admitted: None,
            anchor_month: None,
            writer_updated: false,
            primary_updated: false,
        });
    }
    let slot = crate::compact_error::slot()?;
    let mut assets = Vec::with_capacity(count);
    for (i, net) in net.iter().enumerate() {
        let n = asset_start + 4 * i;
        let leg = &p.legs[i];
        let c = usize::from(leg.context_index);
        if !readonly && (!a[n].is_writable || !a[n + 1].is_writable)
            || *a[n].key != net.market
            || *a[n + 1].key != net.mint
        {
            return Err(invalid());
        }
        let mut standalone_month = None;
        let (mut market, series_index, resident_range) = if leg.context_index == wire::NO_CONTEXT {
            if leg.retirement_index != 255 || !crate::is_system_program(a[n + 3].key) {
                return Err(invalid());
            }
            let (market, range) = load_market_for_route(program, &a[n])?;
            let month = load_valid_oracle_month(program, &a[n], &a[n + 2], &market)?;
            standalone_month = Some(Box::new(month));
            (Box::new(market), usize::MAX, range)
        } else {
            let context = &mut contexts[c];
            let cn = context_role(c);
            let binding = load_collective_group_binding(
                &a[cn],
                &a[cn + 2],
                a[n].key,
                &context.state.sleeve,
                &context.state.group,
                &context.state.book,
            )?;
            let (market, range) = load_market_for_route(program, &a[n])?;
            let market = super::super::collective_binding::collective_market_binding_from_loaded(
                market, &binding,
            )?
            .market;
            if context.anchor_month.is_none() {
                context.anchor_month = Some(load_collective_anchor_month_status(
                    program,
                    &a[cn + 7],
                    &binding,
                )?);
            }
            let index = context.state.book.records[..usize::from(context.state.book.series_count)]
                .iter()
                .position(|r| r.market == net.market)
                .ok_or_else(invalid)?;
            (market, index, range)
        };
        let month = standalone_month
            .as_deref()
            .or_else(|| {
                contexts
                    .get(c)
                    .and_then(|context| context.anchor_month.as_deref())
            })
            .ok_or_else(invalid)?;
        ensure_market_value_flow_unpaused(&config, &market)?;
        ensure_oracle_game_window(&market, month)?;
        if month.settlement_record.is_some()
            || month.settlement_status == OracleSettlementStatus::Final
            || now >= market.instrument.expiry_ts
        {
            return Err(VaultError::AmoebaDlmmMarketNotTradable.into());
        }
        let mint = validate_canonical_market_mint(&a[n], &mut market, &a[n + 1], 0)?;
        let resident_present = resident_range.is_some();
        let source_used_len = resident_range
            .as_ref()
            .map_or(Market::LEN, |range| range.end);
        let resident = if let Some(range) = resident_range {
            load_resident(
                program,
                &a[n],
                &market,
                (leg.context_index != wire::NO_CONTEXT).then(|| a[context_role(c) + 4].key),
                range,
            )?
        } else {
            if leg.context_index == wire::NO_CONTEXT {
                return Err(invalid());
            }
            primary_only::primary_only_resident(
                program,
                a[n].key,
                &market,
                a[4].key,
                a[context_role(c) + 7].key,
            )?
        };
        if resident.pool.status != AmoebaDlmmPoolStatus::Active
            || resident.has_hot_sources()
            || resident.pool.quote_mint != *a[4].key
            || order
                .legs
                .iter()
                .filter(|l| l.market == net.market)
                .any(|l| l.expiry_ts != market.instrument.expiry_ts)
        {
            return Err(invalid());
        }
        if leg.context_index == wire::NO_CONTEXT && resident.pool.oracle_month != *a[n + 2].key {
            return Err(invalid());
        }
        let staging_before =
            if leg.context_index == wire::NO_CONTEXT || crate::is_system_program(a[n + 2].key) {
                0
            } else if readonly {
                custody::observe_market_staging_amount_readonly(
                    program,
                    &a[n],
                    &a[n + 2],
                    &a[n + 1],
                    &a[7],
                )?
            } else {
                custody::observe_market_staging_amount(program, &a[n], &a[n + 2], &a[n + 1], &a[7])?
            };
        let retired_before = if leg.retirement_index == 255 {
            0
        } else {
            let j = usize::from(leg.retirement_index);
            if j >= usize::from(p.custody_accounts) {
                return Err(invalid());
            }
            if readonly {
                custody::observe_writer_retirement_custody_amount_readonly(
                    program,
                    &a[context_role(c)],
                    &a[n],
                    &a[custody_start + j],
                    &a[n + 1],
                    &a[7],
                )?
            } else {
                custody::observe_writer_retirement_custody_amount(
                    program,
                    &a[context_role(c)],
                    &a[n],
                    &a[custody_start + j],
                    &a[n + 1],
                    &a[7],
                )?
            }
        };
        let quantity = u64::try_from(net.delta.unsigned_abs()).map_err(|_| invalid())?;
        if inputs.escrow[i + 1]
            < if close.is_some() || net.delta < 0 {
                quantity
            } else {
                0
            }
        {
            return Err(invalid());
        }
        assets.push(Asset {
            option_before: resident.total_option()?,
            quote_before: resident.total_quote()?,
            resident,
            resident_present,
            source_used_len,
            market,
            series_index,
            mint_before: mint.supply,
            staging_before,
            retired_before,
            fresh: 0,
            retired: 0,
            quote_delta: 0,
            normal_updated: false,
            writer_updated: false,
            touched_pages: vec![],
        });
    }
    let mut quote_delta = 0i128;
    for i in 0..count {
        let buy = close.is_none() && net[i].delta > 0;
        let direction = if buy {
            AmoebaDlmmSwapDirection::QuoteForOption
        } else {
            AmoebaDlmmSwapDirection::OptionForQuote
        };
        let quantity = u64::try_from(net[i].delta.unsigned_abs()).map_err(|_| invalid())?;
        let mut remaining = quantity;
        let mut writer_changed = false;
        while remaining != 0 {
            if p.legs[i].context_index == wire::NO_CONTEXT {
                let asset = &mut assets[i];
                let (bins, unloaded) = sources::lp_sources(&asset.resident, buy)?;
                let (orders, makers, fifo_boundary) = sources::order_sources(&asset.resident, buy)?;
                let chunk = crate::atomic_option_quote::quote_next_atomic_option_chunk_prepared(
                    WriterDlmmRouteConfig {
                        direction,
                        amount_in: if buy { u64::MAX } else { remaining },
                        minimum_amount_out: 0,
                        limit_bin_id: p.legs[i].limit_bin_id,
                        tick_size_quote_atomic: asset.resident.pool.tick_size_quote_atomic,
                        maximum_bin_id: asset.resident.pool.maximum_bin_id,
                        maximum_bins: asset.resident.pool.maximum_bins_per_swap,
                        unloaded_ordinary_boundary: unloaded,
                    },
                    &bins,
                    &[],
                    None,
                    &makers,
                    &[],
                    None,
                    PublicOrderRouteLimits {
                        allow_partial: true,
                        maximum_option_output: if buy { remaining } else { u64::MAX },
                        maximum_order_fills: crate::dlmm_order_math::MAX_ORDER_FILLS,
                    },
                    None,
                )
                .map_err(|_| VaultError::InvalidAmoebaDlmmRoute)?
                .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
                let route = chunk.route();
                if fifo_boundary.is_some_and(|bound| {
                    if buy {
                        route.quote.last_bin_id > bound
                    } else {
                        route.quote.last_bin_id < bound
                    }
                }) {
                    return Err(VaultError::InvalidAmoebaDlmmRoute.into());
                }
                let filled = if buy {
                    route.quote.amount_out
                } else {
                    route.quote.amount_in
                };
                if filled == 0 || filled > remaining || route.quote.protocol_fee != 0 {
                    return Err(invalid());
                }
                let premium = if buy {
                    route.quote.amount_in
                } else {
                    route.quote.amount_out
                };
                let change = if buy {
                    -i128::from(premium)
                } else {
                    i128::from(premium)
                };
                quote_delta = quote_delta.checked_add(change).ok_or_else(invalid)?;
                asset.quote_delta = asset.quote_delta.checked_add(change).ok_or_else(invalid)?;
                let before_option = asset.resident.pool.accounted_option_reserve;
                let before_quote = asset.resident.pool.accounted_quote_reserve;
                mark_normal(asset, route)?;
                let effects = sources::project_normal(
                    &mut asset.resident,
                    orders.as_ref(),
                    route,
                    direction,
                    slot,
                )?;
                sources::commit_normal(
                    &mut asset.resident,
                    before_option,
                    before_quote,
                    effects,
                    buy,
                )?;
                remaining -= filled;
                continue;
            }
            let asset = &mut assets[i];
            let context = &mut contexts[usize::from(p.legs[i].context_index)];
            let (bins, unloaded) = sources::lp_sources(&asset.resident, buy)?;
            let (orders, makers, fifo_boundary) = sources::order_sources(&asset.resident, buy)?;
            for (index, t) in context.terms.iter_mut().enumerate() {
                t.transaction_buyback_cap_atoms = context.policy.series[index]
                    .transaction_buyback_cap_atoms
                    .saturating_sub(context.series_spent[index]);
            }
            let series = &context.series;
            let terms = &context.terms;
            let previous_oi = series[asset.series_index].external_oi_atoms;
            let price_bounds = crate::writer_dlmm_math::writer_dlmm_price_bounds(
                terms[asset.series_index].seller_floor_quote_atoms,
                asset.resident.pool.tick_size_quote_atomic,
                context.policy.price_separation_ticks,
            );
            let round_trip_fee = price_bounds.as_ref().map_or(0, |(_, _, fee)| *fee);
            let observed_cash = add_signed(
                checked(context.hot_before.checked_add(context.cold_before))?,
                context.quote_delta,
            )?;
            let assigned = asset
                .resident
                .writer_position
                .as_ref()
                .map_or(0, |p| p.option_inventory_atoms);
            let mint_supply = checked(asset.mint_before.checked_add(asset.fresh))?;
            let reconciled = (compact_supply_reconciled(
                &context.state.book,
                asset.series_index,
                assigned,
                mint_supply,
                market_outstanding_contract_amount(&asset.market)?,
            ) || dlmm::writer_lane_reconciled(
                &context.state.book,
                asset.series_index,
                assigned,
                asset.staging_before.checked_add(asset.retired_before),
                mint_supply,
                market_outstanding_contract_amount(&asset.market)?,
                context.state.sleeve.accounted_asset_atoms,
                context.policy.total_pool_quote_atoms,
                observed_cash,
            )) && context
                .state
                .sleeve
                .accounted_asset_atoms
                .checked_sub(context.policy.total_pool_quote_atoms)
                .is_some_and(|required| observed_cash >= required);
            let eligible = reconciled
                && price_bounds.is_ok()
                && context.state.sleeve.status == WriterSleeveStatus::Active
                && context.state.group.status == WriterSettlementGroupStatus::Active;
            let policy = WriterDlmmSwapPolicy {
                eligible,
                participation: context
                    .state
                    .sleeve
                    .has_time_participation()
                    .then(|| context.state.sleeve.participation_totals()),
                book: series,
                series_index: asset.series_index,
                cash: WriterDlmmCash {
                    assets_atoms: context.state.sleeve.accounted_asset_atoms,
                    principal_atoms: context.state.sleeve.writer_principal_atoms,
                    allocated_lp_quote_atoms: checked(
                        context
                            .policy
                            .total_pool_quote_atoms
                            .checked_sub(context.policy.total_uncommitted_quote_atoms),
                    )?,
                    pooled_quote_atoms: context.policy.total_pool_quote_atoms,
                },
                risk: dlmm::risk_limits(&context.state.snapshot, &context.state.group),
                buyback: WriterDlmmBuybackLimits {
                    monthly_buyback_cap_atoms: context.policy.monthly_buyback_cap_atoms,
                    transaction_buyback_cap_atoms: context
                        .policy
                        .transaction_buyback_cap_atoms
                        .saturating_sub(context.action_spent),
                    reserve_release_spend_ratio_ppm: context.policy.reserve_release_spend_ratio_ppm,
                    tick_size_quote_atoms: asset.resident.pool.tick_size_quote_atomic,
                    price_separation_ticks: context.policy.price_separation_ticks,
                    round_trip_fee_quote_atoms: round_trip_fee,
                },
                series_limits: terms,
                month_spent_atoms: context.policy.monthly_spent_atoms,
                series_month_spent_atoms: context.policy.series_monthly_spent_atoms
                    [asset.series_index],
            };
            let position = asset.resident.writer_position.as_ref();
            if position.is_none()
                && context.policy.series_pool_inventory_atoms[asset.series_index] != 0
            {
                return Err(invalid());
            }
            let known_before_fifo = |bin: u16| {
                fifo_boundary
                    .is_none_or(|boundary| if buy { bin < boundary } else { bin > boundary })
            };
            let writer = position
                .map_or(&[][..], |p| &p.bins[..usize::from(p.bin_count)])
                .iter()
                .filter(|row| known_before_fifo(row.bin_id))
                .copied()
                .collect::<Vec<_>>();
            let primary = context
                .lane
                .as_ref()
                .map_or(&[][..], |l| {
                    let row = &l.rows[asset.series_index];
                    &row.bins[..usize::from(row.bin_count)]
                })
                .iter()
                .filter(|row| known_before_fifo(row.bin_id))
                .copied()
                .collect::<Vec<_>>();
            let limit_bin_id = fifo_boundary.map_or(p.legs[i].limit_bin_id, |boundary| {
                if buy {
                    boundary.min(p.legs[i].limit_bin_id)
                } else {
                    boundary.max(p.legs[i].limit_bin_id)
                }
            });
            if context.prepared_cache.as_ref().is_none_or(
                |p: &crate::writer_sleeve_math::PreparedWriterReserve| {
                    p.target != asset.series_index
                        || p.initial_oi != series[asset.series_index].external_oi_atoms
                },
            ) {
                context
                    .grid
                    .prepare_into(series, asset.series_index, &mut context.prepared_cache)
                    .map_err(writer_math_error)?;
            }
            let chunk = crate::atomic_option_quote::quote_next_atomic_option_chunk_prepared(
                WriterDlmmRouteConfig {
                    direction,
                    amount_in: if buy { u64::MAX } else { remaining },
                    minimum_amount_out: 0,
                    limit_bin_id,
                    tick_size_quote_atomic: asset.resident.pool.tick_size_quote_atomic,
                    maximum_bin_id: asset.resident.pool.maximum_bin_id,
                    maximum_bins: asset.resident.pool.maximum_bins_per_swap,
                    unloaded_ordinary_boundary: unloaded,
                },
                &bins,
                &writer,
                position.map(|_| &policy),
                &makers,
                &primary,
                context.lane.as_ref().map(|_| &policy),
                PublicOrderRouteLimits {
                    allow_partial: true,
                    maximum_option_output: if buy { remaining } else { u64::MAX },
                    maximum_order_fills: crate::dlmm_order_math::MAX_ORDER_FILLS,
                },
                context.prepared_cache.as_ref(),
            )
            .map_err(|_| VaultError::InvalidAmoebaDlmmRoute)?
            .ok_or(VaultError::InvalidAmoebaDlmmRoute)?;
            let route = chunk.route();
            if let Some(boundary) = fifo_boundary {
                let skipped = if buy {
                    route.quote.last_bin_id > boundary
                } else {
                    route.quote.last_bin_id < boundary
                };
                let primary_tie = matches!(chunk, AtomicOptionChunk::SharedPrimary(_))
                    && route.quote.last_bin_id == boundary;
                let writer_tie =
                    !route.writer_fills.is_empty() && route.quote.last_bin_id == boundary;
                if skipped || primary_tie || writer_tie {
                    return Err(VaultError::InvalidAmoebaDlmmRoute.into());
                }
            }
            let filled = if buy {
                route.quote.amount_out
            } else {
                route.quote.amount_in
            };
            if filled == 0 || filled > remaining || route.quote.protocol_fee != 0 {
                return Err(invalid());
            }
            let premium = if buy {
                route.quote.amount_in
            } else {
                route.quote.amount_out
            };
            let change = if buy {
                -i128::from(premium)
            } else {
                i128::from(premium)
            };
            asset.quote_delta = asset.quote_delta.checked_add(change).ok_or_else(invalid)?;
            quote_delta = quote_delta.checked_add(change).ok_or_else(invalid)?;
            match &chunk {
                AtomicOptionChunk::SharedPrimary(route) => {
                    project_primary(context, asset, route, buy, filled, premium, slot)?
                }
                AtomicOptionChunk::OrdinaryAndPreissued(route) => {
                    let before_option = asset.resident.pool.accounted_option_reserve;
                    let before_quote = asset.resident.pool.accounted_quote_reserve;
                    let mut writer = if route.writer_fills.is_empty() {
                        None
                    } else {
                        Some(dlmm::swap_state_from_authenticated_cached(
                            context.state.take()?,
                            context.policy.take()?,
                            Box::new(asset.resident.writer_position.ok_or_else(invalid)?),
                            *asset.market,
                            &asset.resident.pool,
                            asset.series_index,
                            dlmm::WriterLane {
                                reconciled,
                                before_cash: observed_cash,
                                before_hot_cash: context.hot_before,
                                mint_supply,
                                staged: asset.staging_before,
                                retired: asset.retired_before,
                            },
                            now,
                            core::mem::take(&mut context.series),
                            core::mem::take(&mut context.terms),
                        )?)
                    };
                    mark_normal(asset, route)?;
                    let effects = sources::project_normal(
                        &mut asset.resident,
                        orders.as_ref(),
                        route,
                        direction,
                        slot,
                    )?;
                    if let Some(mut writer) = writer.take() {
                        context.writer_updated = true;
                        asset.writer_updated = true;
                        dlmm::project_swap_effects_cached(
                            &mut writer,
                            route,
                            direction,
                            &mut asset.resident.pool,
                            true,
                            slot,
                        )?;
                        asset.market = Box::new(writer.market);
                        asset.resident.writer_position = Some(*writer.position);
                        context.state.restore(writer.context);
                        context.policy.restore(writer.policy);
                        context.series = writer.series;
                        context.terms = writer.series_limits;
                        context.last_admitted = route.admitted_cash().cloned();
                        let cash = if buy {
                            checked(
                                route
                                    .writer
                                    .net_premium_atoms
                                    .checked_add(route.writer.lp_fee_atoms),
                            )?
                        } else {
                            route.writer.spent_quote_atoms
                        };
                        // Preissued retirement draws pooled quote already held
                        // by this Market; only primary retirement draws the
                        // unpooled WriterCash owner. Sales sweep premium to cash.
                        if buy {
                            context.quote_delta = context
                                .quote_delta
                                .checked_add(i128::from(cash))
                                .ok_or_else(invalid)?;
                        }
                        if !buy {
                            asset.retired = checked(
                                asset.retired.checked_add(route.writer.retired_option_atoms),
                            )?;
                            spend(context, asset.series_index, cash)?;
                        }
                    }
                    sources::commit_normal(
                        &mut asset.resident,
                        before_option,
                        before_quote,
                        effects,
                        buy,
                    )?;
                }
            }
            if !route.writer_fills.is_empty() {
                let next_oi =
                    context.state.book.records[asset.series_index].external_open_interest_atoms;
                if let Some(prepared) = context.prepared_cache.as_mut() {
                    prepared
                        .rebase_same_target(asset.series_index, previous_oi, next_oi)
                        .map_err(writer_math_error)?;
                }
                context.series[asset.series_index].external_oi_atoms = next_oi;
                writer_changed = true;
            }
            remaining -= filled;
        }
        if writer_changed {
            let context = &mut contexts[usize::from(p.legs[i].context_index)];
            let index = assets[i].series_index;
            context
                .grid
                .update(index, context.series[index].external_oi_atoms)
                .map_err(writer_math_error)?;
        }
    }
    if close.is_none() && !order.quote_bound.admits(quote_delta) {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    for context in &mut contexts {
        // The private projections preserve the authenticated policy geometry
        // and check every financial mutation. Validate its complete layout once
        // after all chunks, before any token or Light CPI can run.
        if !context.policy.has_current_layout() {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        let state = &mut *context.state;
        dlmm::update_cash_metrics_with_admission(
            &mut state.sleeve,
            &state.group,
            &state.book,
            &state.snapshot,
            &context.policy,
            true,
            context.last_admitted.as_ref(),
        )?;
        state.book.book_digest = writer_book_digest(&state.book);
    }
    let fee = if close.is_some_and(|(_, _, pay_fee_in_usdc)| pay_fee_in_usdc) {
        crate::trading_session::SPONSOR_FEE_ATOMS
    } else {
        0
    };
    if close.is_some_and(|(floor, _, _)| quote_delta < i128::from(floor) + i128::from(fee)) {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    Ok(ProjectedAction {
        p,
        order,
        net,
        inputs,
        contexts,
        assets,
        asset_start,
        delegate_start,
        custody_start,
        merkle_start,
        quote_delta,
        fee,
        slot,
    })
}
fn execute(
    program: &Pubkey,
    a: &[AccountInfo],
    p: wire::Fill,
    close: Option<(u64, &[u8], bool)>,
) -> ProgramResult {
    let ProjectedAction {
        p,
        mut order,
        net,
        inputs,
        mut contexts,
        assets,
        asset_start,
        delegate_start,
        custody_start,
        merkle_start,
        quote_delta,
        fee,
        slot,
    } = project(program, a, p, close, false, None)?;
    let mut financial_contexts = contexts
        .iter()
        .map(settlement::SettlementContext::from_projected)
        .collect::<Result<Vec<_>, _>>()?;
    let financial_assets = assets
        .iter()
        .map(settlement::SettlementAsset::from_projected)
        .collect::<Result<Vec<_>, _>>()?;
    settlement::settle(
        program,
        a,
        &p,
        &order,
        &net,
        &inputs,
        &mut financial_contexts,
        &financial_assets,
        asset_start,
        delegate_start,
        custody_start,
        merkle_start,
        quote_delta,
        fee,
        close.map(|(_, flags, _)| flags),
    )?;
    for (context, financial) in contexts.iter_mut().zip(financial_contexts) {
        context.cash = financial.cash;
    }
    for (i, asset) in assets.iter().enumerate() {
        // Source identities were authenticated at load, and only the canonical
        // checked LP/FIFO/writer projections above mutate this owned state.
        // Writing it once matches the ordinary route's final atomic commit.
        let mut data = a[asset_start + 4 * i].try_data_mut()?;
        crate::fixed_codec::FixedStateEncode::encode_fixed(
            asset.market.as_ref(),
            &mut data[..Market::LEN],
        );
        if asset.resident_present {
            let payload = borsh::to_vec(&asset.resident).map_err(|_| invalid())?;
            crate::market_router_account::write_payload(&mut data, &payload)?;
        }
    }
    for (i, c) in contexts.iter().enumerate() {
        let n = context_role(i);
        store_state(&a[n], c.state.sleeve.as_ref())?;
        store_state(&a[n + 2], c.state.book.as_ref())?;
        store_state(&a[n + 4], c.policy.as_ref())?;
        if let Some(lane) = &c.lane {
            store(&a[n + 6], lane)?;
        }
        if a[n + 9].owner == program {
            cash_custody::store(&a[n + 9], &c.cash)?;
        } else if c.cash.quote_atoms != 0 {
            return Err(invalid());
        }
    }
    if close.is_none() {
        order.status = mo::FILLED;
        order.filled_quote_delta = quote_delta;
        order.filled_slot = slot;
        crate::processor::multi_order::store(&a[1], &order)?;
    }
    let event = wire::Receipt {
        owner: order.owner,
        custody_owner: order.custody_owner,
        order: *a[1].key,
        close: close.is_some(),
        slot,
        quote_delta,
        sponsor_fee: fee,
        assets: net
            .iter()
            .enumerate()
            .map(|(i, n)| wire::AssetReceipt {
                market: n.market,
                mint: n.mint,
                option_delta: if close.is_some() { -n.delta } else { n.delta },
                quote_delta: assets[i].quote_delta,
            })
            .collect(),
    };
    solana_program::log::sol_log_data(&[
        b"ATOPTFIL",
        &borsh::to_vec(&event).map_err(|_| invalid())?,
    ]);
    Ok(())
}
fn spend(c: &mut Context, index: usize, premium: u64) -> ProgramResult {
    c.action_spent = checked(c.action_spent.checked_add(premium))?;
    c.series_spent[index] = checked(c.series_spent[index].checked_add(premium))?;
    Ok(())
}
fn mark_normal(
    asset: &mut Asset,
    route: &crate::writer_dlmm_quote::WriterDlmmRouteQuote,
) -> ProgramResult {
    asset.normal_updated = true;
    for fill in &route.ordinary_fills {
        let (page, _) = crate::ameba_dlmm_math::bin_to_page(fill.bin_id).map_err(|_| invalid())?;
        if !asset.touched_pages.contains(&page) {
            asset.touched_pages.push(page);
        }
    }
    Ok(())
}
fn project_primary(
    c: &mut Context,
    a: &mut Asset,
    route: &crate::writer_dlmm_quote::WriterDlmmRouteQuote,
    buy: bool,
    quantity: u64,
    premium: u64,
    slot: u64,
) -> ProgramResult {
    c.writer_updated = true;
    c.primary_updated = true;
    let row = &mut c.lane.as_mut().ok_or_else(invalid)?.rows[a.series_index];
    for fill in &route.writer_fills {
        let bin = row.bins[..usize::from(row.bin_count)]
            .iter_mut()
            .find(|b| b.bin_id == fill.bin_id)
            .ok_or_else(invalid)?;
        bin.option_atoms = fill.option_reserve_after;
        bin.quote_atoms = fill.quote_reserve_after;
    }
    let mut count = 0;
    for i in 0..usize::from(row.bin_count) {
        if row.bins[i].option_atoms != 0 || row.bins[i].quote_atoms != 0 {
            row.bins[count] = row.bins[i];
            count += 1;
        }
    }
    row.bins[count..].fill(WriterDlmmBinV1::default());
    row.bin_count = count as u8;
    let state = &mut *c.state;
    let book = &mut *state.book;
    let r = &mut book.records[a.series_index];
    if buy {
        state.sleeve.accounted_asset_atoms =
            checked(state.sleeve.accounted_asset_atoms.checked_add(premium))?;
        state.sleeve.locked_primary_premium_atoms = checked(
            state
                .sleeve
                .locked_primary_premium_atoms
                .checked_add(premium),
        )?;
        r.primary_premium_collected_atoms =
            checked(r.primary_premium_collected_atoms.checked_add(premium))?;
        r.external_open_interest_atoms =
            checked(r.external_open_interest_atoms.checked_add(quantity))?;
        r.total_physical_supply_atoms =
            checked(r.total_physical_supply_atoms.checked_add(quantity))?;
        a.market.mint_accounting.total_issued =
            checked(a.market.mint_accounting.total_issued.checked_add(quantity))?;
        a.fresh = checked(a.fresh.checked_add(quantity))?;
    } else {
        state.sleeve.accounted_asset_atoms =
            checked(state.sleeve.accounted_asset_atoms.checked_sub(premium))?;
        r.external_open_interest_atoms =
            checked(r.external_open_interest_atoms.checked_sub(quantity))?;
        book.individual.compressed_retired_atoms[a.series_index] = checked(
            book.individual.compressed_retired_atoms[a.series_index].checked_add(quantity),
        )?;
        consume_market_contracts(&mut a.market, quantity, false)?;
        c.policy.monthly_spent_atoms = checked(c.policy.monthly_spent_atoms.checked_add(premium))?;
        c.policy.series_monthly_spent_atoms[a.series_index] =
            checked(c.policy.series_monthly_spent_atoms[a.series_index].checked_add(premium))?;
        a.retired = checked(a.retired.checked_add(quantity))?;
    }
    r.custody_status = if r.issuer_controlled_atoms == 0 {
        WriterSeriesCustodyStatus::Closed
    } else {
        WriterSeriesCustodyStatus::Open
    };
    c.quote_delta = c
        .quote_delta
        .checked_add(if buy {
            i128::from(premium)
        } else {
            -i128::from(premium)
        })
        .ok_or_else(invalid)?;
    if !buy {
        spend(c, a.series_index, premium)?;
    }
    c.last_admitted = route.admitted_cash().cloned();
    c.state.sleeve.last_updated_slot = slot;
    c.state.book.last_updated_slot = slot;
    c.lane.as_mut().ok_or_else(invalid)?.last_updated_slot = slot;
    Ok(())
}
