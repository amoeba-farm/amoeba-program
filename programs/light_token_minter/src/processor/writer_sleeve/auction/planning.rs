use super::*;

fn prior_plan_state(
    index: &WriterBidIndexV1,
    end: usize,
) -> Result<([u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY], u64), ProgramError> {
    let mut issued = [0u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY];
    let mut premium = 0u64;
    for record in index.records.iter().take(end) {
        if record.status == WriterBidStatus::Planned && record.accepted_contract_atoms != 0 {
            let slot = usize::from(record.series_index);
            issued[slot] = issued[slot]
                .checked_add(record.accepted_contract_atoms)
                .ok_or(VaultError::ArithmeticOverflow)?;
            premium = premium
                .checked_add(checked_premium(
                    record.accepted_contract_atoms,
                    record.bid_price_per_contract_atoms,
                )?)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
    }
    Ok((issued, premium))
}

#[allow(clippy::too_many_arguments)]
fn maximum_safe_group_atoms(
    sleeve: &WriterSleeveV1,
    book: &WriterSeriesBookV1,
    snapshot: &WriterPolicySnapshotV1,
    security_cap_atoms: u64,
    prior_issue: &[u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
    prior_premium_atoms: u64,
    series_index: usize,
    bid_price_atoms: u64,
    maximum_atoms: u64,
) -> Result<u64, ProgramError> {
    let lot = book.records[series_index].contract_size_atoms;
    if lot != MarketMintAccounting::CANONICAL_ATOMIC_SCALE || !maximum_atoms.is_multiple_of(lot) {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let mut series = [WriterSeries::EMPTY; crate::constants::WRITER_SERIES_STORAGE_CAPACITY];
    for (index, record) in book
        .records
        .iter()
        .take(usize::from(book.series_count))
        .enumerate()
    {
        let prior = record
            .external_open_interest_atoms
            .checked_add(prior_issue[index])
            .ok_or(VaultError::ArithmeticOverflow)?;
        series[index] = WriterSeries {
            kind: record.option_kind,
            strike_price_atomic: record.strike_price_atomic,
            cap_price_atomic: record.cap_or_floor_price_atomic,
            contract_size_atoms: record.contract_size_atoms,
            max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
            external_oi_atoms: prior,
        };
    }
    let accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_add(prior_premium_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let locked_primary_premium_atoms = sleeve
        .locked_primary_premium_atoms
        .checked_add(prior_premium_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let security_mode = match sleeve.security_mode {
        WriterSecurityMode::GrossExternalMaxPayout => {
            WriterMathSecurityMode::GrossExternalMaximumPayout
        }
        WriterSecurityMode::ExactExternalEnvelope => WriterMathSecurityMode::ExactExternalEnvelope,
    };
    maximum_safe_issue_quantity(
        &series[..usize::from(book.series_count)],
        series_index,
        bid_price_atoms,
        maximum_atoms,
        WriterIssueAdmissionLimits {
            security_mode,
            security_cap_atoms,
            accounted_asset_atoms,
            locked_primary_premium_atoms,
            writer_principal_atoms: sleeve.writer_principal_atoms,
            operational_buffer_atoms: snapshot.operational_buffer_atoms,
            worst_drawdown_limit: snapshot.worst_drawdown_limit,
            lower_drawdown_limit: snapshot.lower_drawdown_limit,
            upper_drawdown_limit: snapshot.upper_drawdown_limit,
            lower_tail_max_settlement_atomic: snapshot.lower_tail_max_settlement_atomic,
            upper_tail_min_settlement_atomic: snapshot.upper_tail_min_settlement_atomic,
        },
    )
    .map_err(writer_math_error)
}

fn group_allocation_atoms(
    index: &WriterBidIndexV1,
    group_start: usize,
    group_end: usize,
    target: usize,
    safe_atoms: u64,
    lot: u64,
) -> Result<u64, ProgramError> {
    let safe_lots = safe_atoms / lot;
    let mut total_demand_lots = 0u64;
    for record in &index.records[group_start..group_end] {
        if matches!(
            record.status,
            WriterBidStatus::Funded | WriterBidStatus::Planned
        ) {
            total_demand_lots = total_demand_lots
                .checked_add(record.requested_contract_atoms / lot)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
    }
    if total_demand_lots == 0 {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let mut allocated_lots = 0u64;
    let mut target_base = 0u64;
    let mut target_active_offset = None;
    let mut active_offset = 0u64;
    for (offset, record) in index.records[group_start..group_end].iter().enumerate() {
        if !matches!(
            record.status,
            WriterBidStatus::Funded | WriterBidStatus::Planned
        ) {
            continue;
        }
        let demand_lots = record.requested_contract_atoms / lot;
        let base = u64::try_from(
            u128::from(safe_lots)
                .checked_mul(u128::from(demand_lots))
                .ok_or(VaultError::ArithmeticOverflow)?
                / u128::from(total_demand_lots),
        )
        .map_err(|_| VaultError::ArithmeticOverflow)?;
        allocated_lots = allocated_lots
            .checked_add(base)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if group_start + offset == target {
            target_base = base;
            target_active_offset = Some(active_offset);
        }
        active_offset = active_offset
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    let remainder = safe_lots
        .checked_sub(allocated_lots)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let target_offset = target_active_offset.ok_or(VaultError::InvalidWriterBid)?;
    let target_lots = target_base + u64::from(target_offset < remainder);
    target_lots
        .checked_mul(lot)
        .ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

pub(in crate::processor::writer_sleeve) fn process_plan_writer_auction_chunk(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: PlanWriterAuctionChunkV1Params,
) -> ProgramResult {
    if accounts.len() != PLAN_WRITER_AUCTION_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    if params.max_records == 0 || params.max_records > MAX_PLAN_RECORDS_PER_CALL {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let auction_info = &accounts[6];
    let bid_index_info = &accounts[7];
    let active_manifest_info = &accounts[8];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterPolicyContext {
        group,
        sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    let mut auction = load_writer_auction(
        program_id,
        auction_info,
        sleeve_info.key,
        sleeve.auction_nonce,
    )?;
    let mut index = load_writer_bid_index(program_id, bid_index_info, auction_info.key)?;
    validate_bid_index_series_bindings(&index, &book)?;
    let active = load_valid_oracle_active_weight_manifest(
        program_id,
        &group.anchor_oracle_month,
        active_manifest_info,
    )?;
    let now = current_unix_timestamp()?;
    if config.paused
        || sleeve.active_auction != Some(*auction_info.key)
        || sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || auction.status != WriterAuctionStatus::Planning
        || auction.series_book != *book_info.key
        || auction.policy_snapshot != *snapshot_info.key
        || auction.bid_index != *bid_index_info.key
        || auction.bid_count != index.bid_count
        || auction.planning_cursor != index.planning_cursor
        || !writer_auction_planning_window_open(
            now,
            auction.reveal_deadline_ts,
            auction.execute_deadline_ts,
        )
        || active.rolling_manifest_hash != group.active_weight_manifest_hash
        || active.max_open_interest_payout != group.security_cap_atoms
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let end = usize::from(auction.bid_count).min(
        usize::from(auction.planning_cursor)
            .checked_add(usize::from(params.max_records))
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    let mut cached_group_start = usize::MAX;
    let mut cached_group_end = usize::MAX;
    let mut cached_group_safe_atoms = 0u64;
    while usize::from(auction.planning_cursor) < end {
        let cursor = usize::from(auction.planning_cursor);
        let current = index.records[cursor];
        if current.status == WriterBidStatus::Refunded {
            auction.plan_digest = plan_digest(&auction.plan_digest, &current);
            auction.planned_bid_count = auction
                .planned_bid_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
            index.planned_bid_count = auction.planned_bid_count;
            auction.planning_cursor = auction
                .planning_cursor
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
            index.planning_cursor = auction.planning_cursor;
            continue;
        }
        if current.status != WriterBidStatus::Funded {
            return Err(VaultError::InvalidWriterAuction.into());
        }
        let series_index = usize::from(current.series_index);
        if series_index >= usize::from(book.series_count) {
            return Err(VaultError::InvalidWriterBid.into());
        }
        let mut group_start = cursor;
        while group_start > 0 {
            let previous = index.records[group_start - 1];
            if previous.series_index != current.series_index
                || previous.bid_price_per_contract_atoms != current.bid_price_per_contract_atoms
            {
                break;
            }
            group_start -= 1;
        }
        let mut group_end = cursor + 1;
        while group_end < usize::from(index.bid_count) {
            let next = index.records[group_end];
            if next.series_index != current.series_index
                || next.bid_price_per_contract_atoms != current.bid_price_per_contract_atoms
            {
                break;
            }
            group_end += 1;
        }
        let accepted =
            if current.bid_price_per_contract_atoms < auction.reserve_prices_atoms[series_index] {
                0
            } else {
                let safe = if cached_group_start == group_start && cached_group_end == group_end {
                    cached_group_safe_atoms
                } else {
                    let (prior_issue, prior_premium) = prior_plan_state(&index, group_start)?;
                    let mut demand = 0u64;
                    for record in &index.records[group_start..group_end] {
                        if matches!(
                            record.status,
                            WriterBidStatus::Funded | WriterBidStatus::Planned
                        ) {
                            demand = demand
                                .checked_add(record.requested_contract_atoms)
                                .ok_or(VaultError::ArithmeticOverflow)?;
                        }
                    }
                    let per_series_remaining = auction.issue_caps_atoms[series_index]
                        .checked_sub(prior_issue[series_index])
                        .ok_or(VaultError::InvalidWriterAuction)?;
                    let prior_total = prior_issue.iter().try_fold(0u64, |sum, value| {
                        sum.checked_add(*value)
                            .ok_or(VaultError::ArithmeticOverflow)
                    })?;
                    let auction_remaining = snapshot
                        .max_auction_issue_atoms
                        .checked_sub(prior_total)
                        .ok_or(VaultError::InvalidWriterAuction)?;
                    let maximum = demand.min(per_series_remaining).min(auction_remaining);
                    let safe = maximum_safe_group_atoms(
                        &sleeve,
                        &book,
                        &snapshot,
                        group.security_cap_atoms,
                        &prior_issue,
                        prior_premium,
                        series_index,
                        current.bid_price_per_contract_atoms,
                        maximum,
                    )?;
                    cached_group_start = group_start;
                    cached_group_end = group_end;
                    cached_group_safe_atoms = safe;
                    safe
                };
                group_allocation_atoms(
                    &index,
                    group_start,
                    group_end,
                    cursor,
                    safe,
                    book.records[series_index].contract_size_atoms,
                )?
            };
        let mut planned = current;
        planned.accepted_contract_atoms = accepted;
        planned.status = if accepted == 0 {
            WriterBidStatus::Refundable
        } else {
            WriterBidStatus::Planned
        };
        index.records[cursor] = planned;
        auction.plan_digest = plan_digest(&auction.plan_digest, &planned);
        auction.planned_bid_count = auction
            .planned_bid_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        index.planned_bid_count = auction.planned_bid_count;
        if accepted != 0 {
            auction.planned_issue_atoms[series_index] = auction.planned_issue_atoms[series_index]
                .checked_add(accepted)
                .ok_or(VaultError::ArithmeticOverflow)?;
            auction.accepted_contract_atoms = auction
                .accepted_contract_atoms
                .checked_add(accepted)
                .ok_or(VaultError::ArithmeticOverflow)?;
            let premium = checked_premium(accepted, planned.bid_price_per_contract_atoms)?;
            auction.accepted_premium_atoms = auction
                .accepted_premium_atoms
                .checked_add(premium)
                .ok_or(VaultError::ArithmeticOverflow)?;
            auction.accepted_fee_atoms = auction
                .accepted_fee_atoms
                .checked_add(checked_fee(premium, snapshot.primary_fee_bps)?)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        auction.planning_cursor = auction
            .planning_cursor
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        index.planning_cursor = auction.planning_cursor;
    }
    if auction.planning_cursor == auction.bid_count {
        auction.status = WriterAuctionStatus::Executing;
    }
    let slot = Clock::get()?.slot;
    auction.last_updated_slot = slot;
    index.last_updated_slot = slot;
    store_state(bid_index_info, &index)?;
    store_state(auction_info, &auction)
}
