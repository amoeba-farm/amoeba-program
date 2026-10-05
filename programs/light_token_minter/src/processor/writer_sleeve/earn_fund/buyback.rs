//! Buy-back mark: the permissionless per-sleeve crank (`ManageEarnFundV1`
//! selector 9, ops 0-2 and multipart ops 4-8) and slot valuation (op 3).
//!
//! The crank writes its own mark and unpublished round; slot valuation
//! writes the fund and slot. Neither changes a sleeve, book, pool,
//! position or policy. See `crate::buyback_mark_math` for the statistic and the
//! viability rules, and `docs/earn-fund/EARN_FUND_V2.md` for the interface.
use super::*;
use crate::ameba_dlmm_state::AmoebaDlmmPoolStatus;
use crate::buyback_mark_math::{
    ask_bin_at_depth, bid_at_depth, bin_price, depth_atoms, fold_sample, marked_range_value,
    series_head, series_viable, whole_usd_delta, BucketEntry, BuybackParams, SaleCapacity,
    BUCKET_SECS, CONTRACT_ATOMS, MAX_MARK_SERIES, NO_ASK, NO_BID, RING_BUCKETS,
};
use crate::buyback_mark_state::*;
use crate::processor::ameba_dlmm::load_pool;
use crate::processor::writer_sleeve::dlmm::{observe_writer_lane, WriterLaneAccounts};
use crate::state::{
    derive_writer_dlmm_position_pda, OraclePhase, OracleSettlementStatus, WriterDlmmPolicyV1,
    WriterDlmmPositionV1,
};

const SAMPLE_FIXED: usize = 13;
const SERIES_ACCOUNTS: usize = 6;

pub(super) fn process(program: &Pubkey, a: &[AccountInfo], op: u8) -> ProgramResult {
    match op {
        2 => close(program, a),
        3 => value_slot(program, a),
        4 | 5 => begin_round(program, a, op == 4),
        6 => sample(program, a, false, true),
        7 => finish_round(program, a),
        8 => close_round(program, a),
        0 | 1 => sample(program, a, op == 0, false),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

fn invalid() -> ProgramError {
    VaultError::EarnFundInvalidAccount.into()
}

fn account_hash(account: &AccountInfo) -> Result<[u8; 32], ProgramError> {
    let data = account.try_borrow_data()?;
    Ok(
        solana_program::hash::hashv(&[account.key.as_ref(), account.owner.as_ref(), &data])
            .to_bytes(),
    )
}

/// Fund activity unrelated to pricing does not interrupt a round. The other
/// dependencies include authenticated book/policy bytes, not caller versions.
fn fixed_commitment(program: &Pubkey, a: &[AccountInfo]) -> Result<[u8; 32], ProgramError> {
    let fund = load_fund(program, &a[1])?;
    let mut hash = solana_program::hash::Hasher::default();
    hash.hash(a[1].key.as_ref());
    hash.hash(&fund.buyback_params);
    for index in [2, 3, 4, 5, 6, 7, 8, 10, 11] {
        hash.hash(&account_hash(&a[index])?);
    }
    Ok(hash.result().to_bytes())
}

fn check_round(program: &Pubkey, a: &[AccountInfo], data: &[u8], count: usize) -> ProgramResult {
    if a.len() < 14
        || !a[0].is_signer
        || !a[8].is_writable
        || !a[13].is_writable
        || a[13].owner != program
        || valid_round(program, a[13].key, a[5].key, data) != Some(count)
        || data[ROUND_PENDING] != 1
        || data[ROUND_COMMITMENT..ROUND_COMMITMENT + 32] != fixed_commitment(program, a)?
    {
        return Err(VaultError::EarnFundMarkUnavailable.into());
    }
    Ok(())
}

/// Ops 4/5: create or restart an unpublished round. No series accounts.
#[inline(never)]
fn begin_round(program: &Pubkey, a: &[AccountInfo], create: bool) -> ProgramResult {
    if a.len() != 14
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[8].is_writable
        || !a[13].is_writable
        || *a[9].key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let fund = load_fund(program, &a[1])?;
    let params = BuybackParams::decode(&fund.buyback_params).unwrap_or(BuybackParams::INVALID);
    load_canonical_vault_config(program, &a[2])?;
    let group = load_writer_settlement_group(program, &a[3])?;
    load_oracle_month_state(&a[4], program)?;
    let sleeve = load_writer_sleeve(program, &a[5], a[3].key)?;
    let book = load_writer_series_book(program, &a[6], a[5].key, a[3].key)?;
    let policy = load_exact_zero_padded_state::<WriterDlmmPolicyV1>(
        &a[7],
        program,
        WriterDlmmPolicyV1::LEN,
        VaultError::InvalidWriterPolicySnapshot,
    )?;
    let count = usize::from(book.series_count);
    let clock = Clock::get()?;
    let now = u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::EarnFundNotReady)?;
    if count == 0
        || count > MAX_MARK_SERIES
        || group.anchor_oracle_month != *a[4].key
        || !policy.is_initialized
        || !policy.has_current_layout()
        || !policy.sealed
        || policy.sleeve != *a[5].key
        || *a[10].key != sleeve.usdc_vault
        || *a[11].key
            != crate::compressed_custody::derive_compressed_custody(
                program,
                crate::compressed_custody::CustodyKind::WriterCash,
                a[10].key,
            )
            .0
        || !matches!(
            sleeve.status,
            WriterSleeveStatus::Funding | WriterSleeveStatus::Active
        )
        || sleeve.last_updated_slot >= clock.slot
        || book.last_updated_slot >= clock.slot
        || now.saturating_add(u64::from(params.expiry_guard_secs)) >= sleeve.expiry_ts
    {
        return Err(VaultError::EarnFundMarkUnavailable.into());
    }
    if create {
        let (key, bump) = derive_buyback_mark(program, a[5].key);
        if *a[8].key != key {
            return Err(invalid());
        }
        validate_create_only_program_account_target(program, &a[8])?;
        create_program_account(
            &a[0],
            &a[8],
            &a[9],
            program,
            mark_len(count),
            &[BUYBACK_MARK_SEED, a[5].key.as_ref(), &[bump]],
        )?;
        let mut data = a[8].try_borrow_mut_data()?;
        data[0] = 1;
        data[1] = bump;
        data[2..5].copy_from_slice(&BUYBACK_MARK_DISCRIMINATOR);
        data[5] = BUYBACK_MARK_VERSION;
        put_key(&mut data, OFF_SLEEVE, a[5].key);
        put_key(&mut data, OFF_BOOK, a[6].key);
        put_key(&mut data, OFF_POLICY, a[7].key);
        put_key(&mut data, OFF_RENT_PAYER, a[0].key);
        data[OFF_SERIES_COUNT] = count as u8;
        put_u64(&mut data, OFF_CREATED_TS, now);
        for (index, record) in book.records[..count].iter().enumerate() {
            let pool =
                crate::ameba_dlmm_state::derive_ameba_dlmm_pool_pda(program, &record.market).0;
            put_key(&mut data, series_offset(index) + S_POOL, &pool);
            put_key(
                &mut data,
                series_offset(index) + S_POSITION,
                &derive_writer_dlmm_position_pda(program, &pool, a[5].key).0,
            );
        }
    }
    if a[8].owner != program {
        return Err(invalid());
    }
    let mark = a[8].try_borrow_data()?;
    if valid_mark(program, a[8].key, a[5].key, &mark) != Some(count)
        || get_key(&mark, OFF_BOOK) != *a[6].key
        || get_key(&mark, OFF_POLICY) != *a[7].key
        || get_u64(&mark, OFF_HEAD_SLOT) >= clock.slot
    {
        return Err(invalid());
    }
    let (round_key, bump) = derive_buyback_round(program, a[5].key);
    if *a[13].key != round_key {
        return Err(invalid());
    }
    if a[13].owner != program {
        validate_create_only_program_account_target(program, &a[13])?;
        create_program_account(
            &a[0],
            &a[13],
            &a[9],
            program,
            round_len(count),
            &[BUYBACK_ROUND_SEED, a[5].key.as_ref(), &[bump]],
        )?;
        let mut data = a[13].try_borrow_mut_data()?;
        data[0] = 1;
        data[1] = bump;
        data[2..5].copy_from_slice(b"BBR");
        data[5] = 1;
        put_key(&mut data, 6, a[5].key);
        put_key(&mut data, 38, a[0].key);
        data[ROUND_COUNT] = count as u8;
    }
    let commitment = fixed_commitment(program, a)?;
    let mut data = a[13].try_borrow_mut_data()?;
    if valid_round(program, a[13].key, a[5].key, &data) != Some(count) {
        return Err(invalid());
    }
    data[ROUND_COMMITMENT..ROUND_COMMITMENT + 32].copy_from_slice(&commitment);
    put_u64(&mut data, ROUND_TS, now);
    put_u64(&mut data, ROUND_SLOT, clock.slot);
    data[ROUND_NEXT] = 0;
    data[ROUND_PENDING] = 1;
    let ring_slot = ((now / BUCKET_SECS) % RING_BUCKETS as u64) as usize;
    data[ROUND_FRESH] =
        u8::from(read_bucket(&mark, count, ring_slot, 0).index != now / BUCKET_SECS);
    data[round_mark_offset(count)..].copy_from_slice(&mark);
    Ok(())
}

/// Op 7: validate every sampled pool and market before publishing one complete
/// mark. Position quote changes also update the authenticated book revision.
#[inline(never)]
fn finish_round(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() < 14 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut round = a[13].try_borrow_mut_data()?;
    let count = valid_round(program, a[13].key, a[5].key, &round).ok_or_else(invalid)?;
    check_round(program, a, &round, count)?;
    if a.len() != 14 + 2 * count || usize::from(round[ROUND_NEXT]) != count {
        return Err(invalid());
    }
    let book = load_writer_series_book(program, &a[6], a[5].key, a[3].key)?;
    let params = BuybackParams::decode(&load_fund(program, &a[1])?.buyback_params)
        .unwrap_or(BuybackParams::INVALID);
    let now =
        u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| VaultError::EarnFundNotReady)?;
    let started = get_u64(&round, ROUND_TS);
    if now / BUCKET_SECS != started / BUCKET_SECS
        || now.saturating_sub(started) > u64::from(params.max_head_age_secs)
    {
        return Err(VaultError::EarnFundMarkUnavailable.into());
    }
    let mark = &round[round_mark_offset(count)..];
    if valid_mark(program, a[8].key, a[5].key, mark) != Some(count) {
        return Err(invalid());
    }
    for (index, record) in book.records[..count].iter().enumerate() {
        let (pool, market) = (&a[14 + 2 * index], &a[15 + 2 * index]);
        let at = ROUND_HEADER_LEN + 64 * index;
        if *pool.key != get_key(mark, series_offset(index) + S_POOL)
            || *market.key != record.market
            || round[at..at + 32] != account_hash(pool)?
            || round[at + 32..at + 64] != account_hash(market)?
        {
            return Err(VaultError::EarnFundMarkUnavailable.into());
        }
    }
    a[8].try_borrow_mut_data()?.copy_from_slice(mark);
    round[ROUND_PENDING] = 0;
    Ok(())
}

#[inline(never)]
fn close_round(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 4 || !a[0].is_signer || a[2].owner != program {
        return Err(VaultError::InvalidAccountList.into());
    }
    let data = a[2].try_borrow_data()?;
    if valid_round(program, a[2].key, a[1].key, &data).is_none() || get_key(&data, 38) != *a[3].key
    {
        return Err(invalid());
    }
    if a[1].owner == program && a[1].data_len() != 0 {
        let sleeve = load_writer_sleeve_without_group_meta(program, &a[1])?;
        if !matches!(
            sleeve.status,
            WriterSleeveStatus::SettlementFinalized
                | WriterSleeveStatus::FundingRefunds
                | WriterSleeveStatus::Closed
        ) {
            return Err(VaultError::EarnFundNotReady.into());
        }
    }
    drop(data);
    close_program_account(program, &a[2], &a[3])
}

/// op 0 Create (the mark must not exist; the cranker pays its rent) and
/// op 1 Sample (the mark must exist). Accounts: cranker (s; w when
/// creating), fund, vault config, group, anchor month, sleeve, book, writer
/// DLMM policy, mark (w), system, sleeve USDC vault, WriterCash sidecar
/// (canonical address; w when it exists), SPL Token, then per series in book
/// order: pool, writer position (system-owned while never created), Market,
/// option mint, market staging custody, writer retirement custody.
///
/// Writes only the mark. One sample per slot, refused when the sleeve, book,
/// a pool or a position was written in the current slot. The policy must be
/// sealed; every pool must be the canonical non-order pool of the series'
/// market, bound to the sleeve's anchor month and expiry; positions must be
/// the canonical fund-lane PDAs stored at creation. An ask counts only while
/// the lane is eligible exactly as the swap path decides
/// (`observe_writer_lane`). A bid counts only on the same tradable, eligible
/// lane, when the highest bid is within the buy-back path's per-contract
/// bounds (the conservative claim value and the separated bid below the
/// seller floor) and the policy's remaining buy-back budget (sleeve and
/// series month, series and sleeve transaction caps) covers the depth at
/// that highest bid; otherwise the sample's bid is `NO_BID` (a worthless
/// liability for entrants). The reserve-release ratio and the post-trade
/// solvency the swap also applies are not modelled (as for asks). Creation needs a Funding or
/// Active sleeve with 1–18 series; it records a first sample that only seeds
/// the turnover counters.
#[inline(never)]
fn sample(program: &Pubkey, a: &[AccountInfo], create: bool, chunk: bool) -> ProgramResult {
    if a.len() < SAMPLE_FIXED
        || !a[0].is_signer
        || !a[8].is_writable
        || *a[9].key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Loaded values are reduced to the fields used, so no whole account
    // struct is moved or boxed here.
    let params = BuybackParams::decode(&load_fund(program, &a[1])?.buyback_params)
        .unwrap_or(BuybackParams::INVALID);
    let paused = load_canonical_vault_config(program, &a[2])?.paused;
    let group = load_writer_settlement_group(program, &a[3])?;
    let sleeve = load_writer_sleeve(program, &a[5], a[3].key)?;
    let book = load_writer_series_book(program, &a[6], a[5].key, a[3].key)?;
    // Owned by this program and naming this sleeve: only the canonical PDA.
    let policy = load_exact_zero_padded_state::<WriterDlmmPolicyV1>(
        &a[7],
        program,
        WriterDlmmPolicyV1::LEN,
        VaultError::InvalidWriterPolicySnapshot,
    )?;
    let count = usize::from(book.series_count);
    let fixed = SAMPLE_FIXED + usize::from(chunk);
    let (start, take, round_ts, round_slot, round_fresh) = if chunk {
        let data = a.get(13).ok_or_else(invalid)?.try_borrow_data()?;
        check_round(program, a, &data, count)?;
        let start = usize::from(data[ROUND_NEXT]);
        let take = a.len().saturating_sub(fixed) / SERIES_ACCOUNTS;
        if take == 0 || take > 7 || start + take > count {
            return Err(invalid());
        }
        (
            start,
            take,
            get_u64(&data, ROUND_TS),
            get_u64(&data, ROUND_SLOT),
            data[ROUND_FRESH] == 1,
        )
    } else {
        (0, count, 0, 0, false)
    };
    if group.anchor_oracle_month != *a[4].key
        || !policy.is_initialized
        || !policy.has_current_layout()
        || !policy.sealed
        || policy.sleeve != *a[5].key
        || count == 0
        || count > MAX_MARK_SERIES
        || a.len() != fixed + SERIES_ACCOUNTS * take
        || *a[10].key != sleeve.usdc_vault
        || *a[11].key
            != crate::compressed_custody::derive_compressed_custody(
                program,
                crate::compressed_custody::CustodyKind::WriterCash,
                a[10].key,
            )
            .0
    {
        return Err(invalid());
    }
    // The canonical WriterCash sidecar counts toward cash backing when it exists.
    let cash_sidecar = (a[11].owner == program).then_some(&a[11]);
    let game = {
        let month = load_oracle_month_state(&a[4], program)?;
        month.phase == OraclePhase::Game
            && month.settlement_record.is_none()
            && month.settlement_status != OracleSettlementStatus::Final
    };
    let clock = Clock::get()?;
    let current_now =
        u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::EarnFundNotReady)?;
    let now = if chunk { round_ts } else { current_now };
    let sample_slot = if chunk { round_slot } else { clock.slot };
    if chunk
        && (current_now / BUCKET_SECS != now / BUCKET_SECS
            || current_now.saturating_sub(now) > u64::from(params.max_head_age_secs))
    {
        return Err(VaultError::EarnFundMarkUnavailable.into());
    }
    if create {
        let (key, bump) = derive_buyback_mark(program, a[5].key);
        if *a[8].key != key
            || !a[0].is_writable
            || !matches!(
                sleeve.status,
                WriterSleeveStatus::Funding | WriterSleeveStatus::Active
            )
        {
            return Err(invalid());
        }
        validate_create_only_program_account_target(program, &a[8])?;
        create_program_account(
            &a[0],
            &a[8],
            &a[9],
            program,
            mark_len(count),
            &[BUYBACK_MARK_SEED, a[5].key.as_ref(), &[bump]],
        )?;
        let mut data = a[8].try_borrow_mut_data()?;
        data[0] = 1;
        data[1] = bump;
        data[2..5].copy_from_slice(&BUYBACK_MARK_DISCRIMINATOR);
        data[5] = BUYBACK_MARK_VERSION;
        put_key(&mut data, OFF_SLEEVE, a[5].key);
        put_key(&mut data, OFF_BOOK, a[6].key);
        put_key(&mut data, OFF_POLICY, a[7].key);
        put_key(&mut data, OFF_RENT_PAYER, a[0].key);
        data[OFF_SERIES_COUNT] = count as u8;
        put_u64(&mut data, OFF_CREATED_TS, now);
        for index in 0..count {
            let pool = a[SAMPLE_FIXED + SERIES_ACCOUNTS * index].key;
            let at = series_offset(index);
            put_key(&mut data, at + S_POOL, pool);
            put_key(
                &mut data,
                at + S_POSITION,
                &derive_writer_dlmm_position_pda(program, pool, a[5].key).0,
            );
        }
    } else if a[8].owner != program {
        return Err(invalid());
    }
    let mut storage = a[if chunk { 13 } else { 8 }].try_borrow_mut_data()?;
    let mut data = &mut storage[if chunk { round_mark_offset(count) } else { 0 }..];
    if valid_mark(program, a[8].key, a[5].key, &data) != Some(count)
        || get_key(&data, OFF_BOOK) != *a[6].key
        || get_key(&data, OFF_POLICY) != *a[7].key
    {
        return Err(invalid());
    }
    if sleeve.last_updated_slot >= sample_slot
        || book.last_updated_slot >= sample_slot
        || sample_slot <= get_u64(&data, OFF_HEAD_SLOT)
    {
        return Err(VaultError::EarnFundMarkUnavailable.into());
    }
    // V1 at sleeve level: the same predicate the collective swap enforces.
    let open = !paused
        && game
        && sleeve.status == WriterSleeveStatus::Active
        && group.status == WriterSettlementGroupStatus::Active
        && now.saturating_add(u64::from(params.expiry_guard_secs)) < sleeve.expiry_ts;
    let capacity = SaleCapacity {
        accounted_assets: sleeve.accounted_asset_atoms,
        exact_reserve: sleeve.exact_reserve_atoms,
        shared_reserve: group.shared_reserve,
        pooled_quote: policy.total_pool_quote_atoms,
        allocated_lp_quote: policy
            .total_pool_quote_atoms
            .saturating_sub(policy.total_uncommitted_quote_atoms),
        operational_buffer: sleeve.operational_buffer_atoms,
        totals: sleeve.participation_totals(),
    };
    let current = now / BUCKET_SECS;
    let ring_slot = (current % RING_BUCKETS as u64) as usize;
    let fresh = if chunk {
        round_fresh
    } else {
        read_bucket(&data, count, ring_slot, 0).index != current
    };
    // The first sample, and the first after a gap of more than one bucket,
    // only re-seed the turnover counters: turnover from the gap is never
    // booked as recent flow.
    let seed = get_u64(&data, OFF_SAMPLE_COUNT) == 0
        || current > get_u64(&data, OFF_LAST_BUCKET).saturating_add(1);
    let month_rolled = policy.spending_month_start_ts != get_u64(&data, OFF_SPEND_MONTH);
    let mut numerator = 0u128;
    let mut bid_numerator = 0u128;
    let mut all_viable = true;
    for (index, record) in book.records[..count]
        .iter()
        .enumerate()
        .skip(start)
        .take(take)
    {
        let at = series_offset(index);
        // pool, writer position, Market, option mint, market staging
        // custody, writer retirement custody.
        let s = &a[fixed + SERIES_ACCOUNTS * (index - start)..][..SERIES_ACCOUNTS];
        let (pool_info, position_info) = (&s[0], &s[1]);
        if *pool_info.key != get_key(&data, at + S_POOL)
            || *position_info.key != get_key(&data, at + S_POSITION)
            || *s[2].key != record.market
        {
            return Err(invalid());
        }
        // `load_pool` binds the pool to the canonical PDA of `pool.market`.
        let pool = load_pool(program, pool_info)?;
        if pool.account_version == crate::dlmm_order_state::ORDER_POOL_VERSION
            || pool.market != record.market
            || pool.oracle_month != group.anchor_oracle_month
            || pool.expiry_ts != sleeve.expiry_ts
        {
            return Err(VaultError::InvalidAmoebaDlmmPool.into());
        }
        if pool.last_updated_slot >= sample_slot {
            return Err(VaultError::EarnFundMarkUnavailable.into());
        }
        let tick = pool.tick_size_quote_atomic;
        let open_interest = record.external_open_interest_atoms;
        let max_payout = record.max_payout_per_contract_atoms;
        let depth = depth_atoms(&params, open_interest);
        let terms = &policy.series[index];
        let spend = policy.series_monthly_spent_atoms[index];
        // What the buy-back path could pay now (`admit_writer_dlmm_retirement`):
        // the remaining month budgets and the transaction caps, per contract
        // at most the conservative claim value and the separated bid.
        let budget = policy
            .monthly_buyback_cap_atoms
            .saturating_sub(policy.monthly_spent_atoms)
            .min(terms.monthly_buyback_cap_atoms.saturating_sub(spend))
            .min(terms.transaction_buyback_cap_atoms)
            .min(policy.transaction_buyback_cap_atoms);
        let max_bid = terms.conservative_claim_value_atoms.min(
            terms
                .seller_floor_quote_atoms
                .saturating_sub(tick.saturating_mul(u64::from(policy.price_separation_ticks))),
        );
        let (ask, bid) = if position_info.owner == program {
            // The key is the canonical PDA stored at creation. Decoded on the
            // stack: one boxed position per series would exhaust the 32 KiB
            // heap at 16 series.
            let position = load_exact_zero_padded_state::<WriterDlmmPositionV1>(
                position_info,
                program,
                WriterDlmmPositionV1::LEN,
                VaultError::InvalidWriterSleeve,
            )?;
            if !position.is_initialized
                || !position.has_current_layout()
                || position.pool != *pool_info.key
                || position.sleeve != *a[5].key
                || position.policy != *a[7].key
                || position.market != record.market
                || usize::from(position.series_index) != index
            {
                return Err(VaultError::InvalidWriterSleeve.into());
            }
            if position.last_updated_slot >= sample_slot {
                return Err(VaultError::EarnFundMarkUnavailable.into());
            }
            // An ask counts only if the lane can execute it: the swap path's
            // own eligibility (`load_swap_state_with_cash`; a frozen lane's
            // asks are unbuyable). Sleeve and group status are in `open`.
            let mut market = load_valid_market(program, &s[2])?;
            let lane = observe_writer_lane(
                program,
                &WriterLaneAccounts {
                    sleeve: &a[5],
                    usdc_vault: &a[10],
                    usdc_mint: &pool.quote_mint,
                    cash_sidecar,
                    market: &s[2],
                    mint: &s[3],
                    staging: &s[4],
                    retirement: &s[5],
                    token_program: &a[12],
                },
                &sleeve,
                &book,
                &policy,
                index,
                position.option_inventory_atoms,
                &mut market,
            )?;
            let bins = &position.bins[..usize::from(position.bin_count)];
            let live = lane.reconciled
                && crate::writer_dlmm_math::writer_dlmm_price_bounds(
                    terms.seller_floor_quote_atoms,
                    tick,
                    policy.price_separation_ticks,
                )
                .is_ok()
                && !market.paused;
            (
                // Asks below the seller floor (it can be raised by an
                // amendment) never fill, so they hold no executable depth.
                ask_bin_at_depth(
                    bins.iter().map(|bin| {
                        let fills = tick.saturating_mul(u64::from(bin.bin_id))
                            >= terms.seller_floor_quote_atoms;
                        (bin.bin_id, bin.option_atoms * u64::from(fills))
                    }),
                    depth,
                )
                .filter(|_| live),
                bid_at_depth(
                    bins.iter().rev().map(|bin| (bin.bin_id, bin.quote_atoms)),
                    depth,
                    tick,
                )
                .filter(|(top, _)| {
                    let price = tick.saturating_mul(u64::from(*top));
                    live && price <= max_bid
                        && depth.saturating_mul(price) / CONTRACT_ATOMS <= budget
                }),
            )
        } else {
            (None, None)
        };
        let tradable = open && pool.status == AmoebaDlmmPoolStatus::Active;
        let bin = match ask {
            Some(bin)
                if tradable
                    && capacity.admits(depth, bin_price(bin, tick, max_payout), max_payout) =>
            {
                bin
            }
            _ => NO_ASK,
        };
        // The statistic is the bin where the bids reach the depth; the
        // bounds and budget above are tested at the highest bid.
        let bid_bin = match bid {
            Some((_, bin)) if tradable => bin,
            _ => NO_BID,
        };
        let valid = bin != NO_ASK;
        let premium = record.primary_premium_collected_atoms;
        let premium_from = if seed {
            premium
        } else {
            get_u64(&data, at + S_CUM_PREMIUM)
        };
        let spend_from = if seed {
            spend
        } else if month_rolled {
            0
        } else {
            get_u64(&data, at + S_CUM_SPEND)
        };
        let (sale_usd, premium_counted) = whole_usd_delta(premium, premium_from);
        let (buy_usd, spend_counted) = whole_usd_delta(spend, spend_from);
        put_u64(&mut data, at + S_CUM_PREMIUM, premium_counted);
        put_u64(&mut data, at + S_CUM_SPEND, spend_counted);
        let mut entry = if fresh {
            BucketEntry::default()
        } else {
            read_bucket(&data, count, ring_slot, index)
        };
        fold_sample(&mut entry, current, bin, bid_bin, valid, sale_usd, buy_usd);
        write_bucket(&mut data, count, ring_slot, index, &entry);
        let head = series_head(
            (0..RING_BUCKETS).map(|slot| read_bucket(&data, count, slot, index)),
            current,
            tick,
            max_payout,
        );
        let viable = series_viable(&params, &head, sleeve.writer_principal_atoms, valid);
        put_u64(&mut data, at + S_HEAD_PRICE, head.price);
        put_u64(&mut data, at + S_HEAD_BID_PRICE, head.bid_price);
        data[at + S_VIABLE] = u8::from(viable);
        data[at + S_VALID_BUCKETS] = head.valid_buckets;
    }
    if chunk && start + take < count {
        let _ = data;
        for offset in 0..take {
            let s = &a[fixed + SERIES_ACCOUNTS * offset..];
            let at = ROUND_HEADER_LEN + 64 * (start + offset);
            storage[at..at + 32].copy_from_slice(&account_hash(&s[0])?);
            storage[at + 32..at + 64].copy_from_slice(&account_hash(&s[2])?);
        }
        storage[ROUND_NEXT] = (start + take) as u8;
        return Ok(());
    }
    for (index, record) in book.records[..count].iter().enumerate() {
        let at = series_offset(index);
        numerator += u128::from(record.external_open_interest_atoms)
            * u128::from(get_u64(&data, at + S_HEAD_PRICE));
        bid_numerator += u128::from(record.external_open_interest_atoms)
            * u128::from(get_u64(&data, at + S_HEAD_BID_PRICE));
        all_viable &= record.external_open_interest_atoms == 0 || data[at + S_VIABLE] == 1;
    }
    let liability = slot_liability_atoms(numerator)?;
    // `L_hi = min(L, ceil(Σ OI_i × b_i / 10^6))`: the entry-side liability
    // (the same rounding as `L`; at most one atom in the entrant's favour).
    let upper_liability = slot_liability_atoms(bid_numerator)?.min(liability);
    data[OFF_HEAD_ALL_VIABLE] = u8::from(all_viable);
    put_u64(&mut data, OFF_HEAD_UPPER_LIABILITY, upper_liability);
    put_u64(&mut data, OFF_HEAD_TS, now);
    put_u64(&mut data, OFF_HEAD_SLOT, sample_slot);
    put_u64(&mut data, OFF_HEAD_BOOK_SLOT, book.last_updated_slot);
    put_u64(&mut data, OFF_HEAD_LIABILITY, liability);
    put_u64(&mut data, OFF_SPEND_MONTH, policy.spending_month_start_ts);
    put_u64(&mut data, OFF_LAST_BUCKET, current);
    let samples = get_u64(&data, OFF_SAMPLE_COUNT).saturating_add(1);
    put_u64(&mut data, OFF_SAMPLE_COUNT, samples);
    if chunk {
        let _ = data;
        for offset in 0..take {
            let s = &a[fixed + SERIES_ACCOUNTS * offset..];
            let at = ROUND_HEADER_LEN + 64 * (start + offset);
            storage[at..at + 32].copy_from_slice(&account_hash(&s[0])?);
            storage[at + 32..at + 64].copy_from_slice(&account_hash(&s[2])?);
        }
        storage[ROUND_NEXT] = count as u8;
    }
    Ok(())
}

/// `ceil(numerator / 10^6)` quote atoms (`Σ OI_i × price_i` over the series).
#[inline(never)]
fn slot_liability_atoms(numerator: u128) -> Result<u64, ProgramError> {
    u64::try_from(numerator.div_ceil(u128::from(CONTRACT_ATOMS)))
        .map_err(|_| VaultError::ArithmeticOverflow.into())
}

/// op 2. caller (s), sleeve (or its closed address), mark (w), rent payer
/// (w). Once the sleeve is settled, refunded or closed.
#[inline(never)]
fn close(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 4 || !a[0].is_signer || a[2].owner != program {
        return Err(VaultError::InvalidAccountList.into());
    }
    {
        let data = a[2].try_borrow_data()?;
        if valid_mark(program, a[2].key, a[1].key, &data).is_none()
            || get_key(&data, OFF_RENT_PAYER) != *a[3].key
        {
            return Err(invalid());
        }
    }
    if a[1].owner == program && a[1].data_len() != 0 {
        let sleeve = load_writer_sleeve_without_group_meta(program, &a[1])?;
        if !matches!(
            sleeve.status,
            WriterSleeveStatus::SettlementFinalized
                | WriterSleeveStatus::FundingRefunds
                | WriterSleeveStatus::Closed
        ) {
            return Err(VaultError::EarnFundNotReady.into());
        }
    }
    close_program_account(program, &a[2], &a[3])
}

/// op 3 (permissionless): value the fund's slot of one sleeve into the
/// fund's aggregate. Accounts: cranker (s), fund (w), slot (w), sleeve,
/// series book, buy-back mark (`PDA(sleeve)`, may not exist; for an Active
/// sleeve any other account is refused).
///
/// This is also the re-validation of a recorded value: whenever the slot
/// cannot be priced now — no fresh, all-viable head that covers the book's
/// current state (a trade, settlement step or any other book write since the
/// head), the sleeve or book written in this slot, within the expiry guard,
/// expired awaiting settlement, a sleeve that no longer loads — the slot is
/// recorded as unpriced, which stops instant exits of invested money until
/// it is valued again from a later sample. Settled and refunded sleeves are
/// exact and final.
#[inline(never)]
fn value_slot(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 6 || !a[0].is_signer || !a[1].is_writable || !a[2].is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut fund = load_fund(program, &a[1])?;
    let mut slot = load_slot(program, &a[2], a[3].key)?;
    let params = BuybackParams::decode(&fund.buyback_params).unwrap_or(BuybackParams::INVALID);
    let clock = Clock::get()?;
    let now = u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::EarnFundNotReady)?;
    match slot_value(program, &slot, &params, &a[3..], now, clock.slot)? {
        Some(value) => {
            if fund
                .book
                .record(
                    &mut slot.mark,
                    value.lower,
                    value.upper,
                    value.valued_ts,
                    value.exact,
                    now,
                )
                .map_err(fund_error)?
            {
                slot.sample_slot = value.sample_slot;
                slot.book_slot = value.book_slot;
            }
        }
        None => fund.book.invalidate(&mut slot.mark),
    }
    slot.write(&mut a[2].try_borrow_mut_data()?);
    store_state(&a[1], fund.as_ref())
}

/// One slot's valuation as the crank records it.
pub(super) struct SlotValue {
    pub lower: u64,
    pub upper: u64,
    pub valued_ts: u64,
    pub exact: bool,
    pub sample_slot: u64,
    pub book_slot: u64,
}

/// The value of the fund's lots in one sleeve (`accounts`: sleeve, series
/// book, buy-back mark), or `None` when it cannot be priced now. Settled and
/// refunded sleeves are exact (the claim inputs; the mark is not read and may
/// be closed); Funding sleeves have sold nothing; Active sleeves need a
/// fresh, all-viable head whose book is unchanged since. The lower (exit)
/// value is the claim at the ask-side liability `L` with unrealized gains
/// credited at `g`; the upper (entry) value the claim at the bid-side
/// liability `L_hi <= L` with gains fully credited (equal for exact slots).
#[inline(always)]
pub(super) fn slot_value(
    program: &Pubkey,
    slot: &EarnFundSlotV1,
    params: &BuybackParams,
    accounts: &[AccountInfo],
    now: u64,
    current_slot: u64,
) -> Result<Option<SlotValue>, ProgramError> {
    let (sleeve_info, book_info, mark_info) = (&accounts[0], &accounts[1], &accounts[2]);
    // The slot's sleeve was authenticated by the full loader at allocation;
    // one that no longer loads (e.g. closed) cannot be priced.
    let Ok(sleeve) = load_exact_zero_padded_state::<WriterSleeveV1>(
        sleeve_info,
        program,
        WriterSleeveV1::LEN,
        VaultError::InvalidWriterSleeve,
    ) else {
        return Ok(None);
    };
    let book_slot = {
        let data = book_info.try_borrow_data()?;
        if *book_info.key != sleeve.series_book
            || book_info.owner != program
            || data.len() != WriterSeriesBookV1::LEN
            || data[2..5] != WriterSeriesBookV1::ACCOUNT_DISCRIMINATOR
            || get_key(&data, 6) != *sleeve_info.key
        {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
        get_u64(&data, 112)
    };
    let unwritten = book_slot < current_slot && sleeve.last_updated_slot < current_slot;
    // (principal, assets, liability, upper liability, gain credit, valued
    // at, sample slot).
    let (principal, assets, liability, upper_liability, credit, valued_ts, sample_slot) =
        match sleeve.status {
            WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::FundingRefunds => (
                sleeve.settlement_principal_atoms,
                sleeve.writer_residual_initial_atoms,
                0,
                0,
                10_000,
                now,
                current_slot,
            ),
            WriterSleeveStatus::Funding if unwritten => (
                sleeve.writer_principal_atoms,
                sleeve.accounted_asset_atoms,
                0,
                0,
                params.gain_credit_bps,
                now,
                current_slot,
            ),
            WriterSleeveStatus::Active if unwritten => {
                if mark_info.owner != program {
                    // No mark yet: unpriced. Only the canonical (absent) mark
                    // address says so; any other account is refused, or anyone
                    // could unprice a fresh slot by naming a foreign "mark".
                    if *mark_info.key != derive_buyback_mark(program, sleeve_info.key).0 {
                        return Err(invalid());
                    }
                    return Ok(None);
                }
                let data = mark_info.try_borrow_data()?;
                if valid_mark(program, mark_info.key, sleeve_info.key, &data).is_none() {
                    return Err(invalid());
                }
                let head_ts = get_u64(&data, OFF_HEAD_TS);
                if data[OFF_HEAD_ALL_VIABLE] != 1
                    || now.saturating_sub(head_ts) > u64::from(params.max_head_age_secs)
                    || get_u64(&data, OFF_HEAD_BOOK_SLOT) != book_slot
                    || now.saturating_add(u64::from(params.expiry_guard_secs)) >= sleeve.expiry_ts
                {
                    return Ok(None);
                }
                (
                    sleeve.writer_principal_atoms,
                    sleeve.accounted_asset_atoms,
                    get_u64(&data, OFF_HEAD_LIABILITY),
                    get_u64(&data, OFF_HEAD_UPPER_LIABILITY),
                    params.gain_credit_bps,
                    head_ts,
                    get_u64(&data, OFF_HEAD_SLOT),
                )
            }
            _ => return Ok(None),
        };
    let exact = matches!(
        sleeve.status,
        WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::FundingRefunds
    );
    let value = |liability, credit| {
        marked_range_value(
            slot.range(),
            principal,
            sleeve.capital_seconds,
            assets,
            liability,
            credit,
        )
    };
    let (Some(lower), Some(upper)) = (value(liability, credit), value(upper_liability, 10_000))
    else {
        return Ok(None);
    };
    Ok(Some(SlotValue {
        lower,
        upper,
        valued_ts,
        exact,
        sample_slot,
        book_slot,
    }))
}
