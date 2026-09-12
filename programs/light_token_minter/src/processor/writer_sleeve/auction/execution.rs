use super::*;

pub(in crate::processor::writer_sleeve) fn process_execute_writer_auction_fill(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != EXECUTE_WRITER_AUCTION_FILL_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let auction_info = &accounts[6];
    let bid_index_info = &accounts[7];
    let bid_info = &accounts[8];
    let escrow_info = &accounts[9];
    let sleeve_vault_info = &accounts[10];
    let fee_vault_info = &accounts[11];
    let settlement_mint_info = &accounts[12];
    let market_info = &accounts[13];
    let contract_mint_info = &accounts[14];
    let staging_info = &accounts[15];
    let retirement_info = &accounts[16];
    let destination_info = &accounts[17];
    let light_program_info = &accounts[18];
    let cpi_authority_info = &accounts[19];
    let interface_info = &accounts[20];
    let token_program_info = &accounts[21];
    let system_program_info = &accounts[22];
    let compressible_config_info = &accounts[23];
    let rent_sponsor_info = &accounts[24];
    let active_manifest_info = &accounts[25];
    if !cranker_info.is_signer || !cranker_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_compression_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
        compressible_config_info,
        rent_sponsor_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterPolicyContext {
        group,
        mut sleeve,
        mut book,
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
    let mut bid = load_writer_bid(program_id, bid_info, auction_info.key)?;
    let active = load_valid_oracle_active_weight_manifest(
        program_id,
        &group.anchor_oracle_month,
        active_manifest_info,
    )?;
    if config.paused
        || config.usdc_mint != *settlement_mint_info.key
        || sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || sleeve.active_auction != Some(*auction_info.key)
        || sleeve.active_close_request.is_some()
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
        || auction.status != WriterAuctionStatus::Executing
        || auction.series_book != *book_info.key
        || auction.policy_snapshot != *snapshot_info.key
        || auction.bid_index != *bid_index_info.key
        || auction.escrow != *escrow_info.key
        || auction.fee_vault != *fee_vault_info.key
        || !writer_auction_execute_deadline_open(
            current_unix_timestamp()?,
            auction.execute_deadline_ts,
        )
        || active.rolling_manifest_hash != group.active_weight_manifest_hash
        || active.max_open_interest_payout != group.security_cap_atoms
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let position = index
        .records
        .iter()
        .take(usize::from(index.bid_count))
        .position(|record| record.bid == *bid_info.key)
        .ok_or(VaultError::InvalidWriterBid)?;
    if index.records[..position]
        .iter()
        .any(|record| record.status == WriterBidStatus::Planned)
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let summary = index.records[position];
    if summary.status != WriterBidStatus::Planned
        || summary.accepted_contract_atoms == 0
        || bid.status != WriterBidStatus::Funded
        || bid.bidder != summary.bidder
        || bid.order_id != summary.order_id
        || bid.series_index != summary.series_index
        || bid.bid_price_per_contract_atoms != summary.bid_price_per_contract_atoms
        || bid.requested_contract_atoms != summary.requested_contract_atoms
        || bid.escrowed_atoms != summary.escrowed_atoms
        || bid.claim_destination != *destination_info.key
    {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let series_index = usize::from(summary.series_index);
    if series_index >= usize::from(book.series_count) {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let stored = book.records[series_index];
    if stored.market != *market_info.key
        || stored.contract_mint != *contract_mint_info.key
        || stored.retirement_custody != *retirement_info.key
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    ensure_market_not_expired(&market)?;
    if market.paused
        || market.market_id != stored.series_id
        || market.long_contract_mint != Some(*contract_mint_info.key)
        || market_outstanding_contract_amount(&market)? != stored.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mint_before =
        validate_canonical_market_mint(market_info, &mut market, contract_mint_info, 0)?;
    if market
        .mint_accounting
        .total_issued
        .checked_sub(market.mint_accounting.total_burned)
        != Some(stored.total_physical_supply_atoms)
        || mint_before.supply != stored.total_physical_supply_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    // Observe every custody input without creating an account or invoking a token program. The
    // complete resulting book is admitted below before the first external mutation.
    let staging_before = observe_market_staging_amount(
        program_id,
        market_info,
        staging_info,
        contract_mint_info,
        token_program_info,
    )?;
    let retirement_before = observe_writer_retirement_custody_amount(
        program_id,
        sleeve_info,
        market_info,
        retirement_info,
        contract_mint_info,
        token_program_info,
    )?;
    let observed_issuer = staging_before
        .checked_add(retirement_before)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if observed_issuer < stored.issuer_controlled_atoms || observed_issuer > mint_before.supply {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let custody_increase = observed_issuer
        .checked_sub(stored.issuer_controlled_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if custody_increase > stored.external_open_interest_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    book.records[series_index].external_open_interest_atoms = stored
        .external_open_interest_atoms
        .checked_sub(custody_increase)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.records[series_index].issuer_controlled_atoms = observed_issuer;
    if observed_issuer != 0 {
        book.records[series_index].custody_status = WriterSeriesCustodyStatus::Open;
    }
    market.mint_accounting.total_consumed = market
        .mint_accounting
        .total_consumed
        .checked_add(custody_increase)
        .ok_or(VaultError::ArithmeticOverflow)?;

    let accepted = summary.accepted_contract_atoms;
    let premium = checked_premium(accepted, summary.bid_price_per_contract_atoms)?;
    let fee = checked_fee(premium, snapshot.primary_fee_bps)?;
    let charged = premium
        .checked_add(fee)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if charged > bid.escrowed_atoms {
        return Err(VaultError::InvalidWriterBid.into());
    }
    book.records[series_index].external_open_interest_atoms = book.records[series_index]
        .external_open_interest_atoms
        .checked_add(accepted)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.records[series_index].total_physical_supply_atoms = book.records[series_index]
        .total_physical_supply_atoms
        .checked_add(accepted)
        .ok_or(VaultError::ArithmeticOverflow)?;
    book.records[series_index].primary_premium_collected_atoms = book.records[series_index]
        .primary_premium_collected_atoms
        .checked_add(premium)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_add(premium)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.locked_primary_premium_atoms = sleeve
        .locked_primary_premium_atoms
        .checked_add(premium)
        .ok_or(VaultError::ArithmeticOverflow)?;
    // Custody reconciliation and newly accepted external OI are one atomic economic transition.
    // Validate reserve, drawdown, security, and exact supply deltas from canonical prestate before
    // creating an account, invoking a token program, or storing any state.
    recompute_writer_metrics(
        &mut sleeve,
        &book,
        &snapshot,
        Some(group.security_cap_atoms),
        true,
    )?;
    validate_vault_token_account(escrow_info, settlement_mint_info.key, auction_info.key)?;
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    validate_vault_token_account(fee_vault_info, settlement_mint_info.key, &snapshot.registry)?;
    let escrow_before = validate_token_account(escrow_info)?.amount;
    let sleeve_vault_before = validate_token_account(sleeve_vault_info)?.amount;
    let fee_vault_before = validate_token_account(fee_vault_info)?.amount;
    if escrow_before < auction.total_escrow_atoms
        || sleeve_vault_before
            < sleeve
                .accounted_asset_atoms
                .checked_sub(premium)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let destination_before = match bid.delivery_mode {
        crate::state::WriterBidDeliveryMode::LightToken => Some(
            crate::processor::scoped_settlement::load_scoped_holder_token_account(
                program_id,
                destination_info,
                &bid.bidder,
                contract_mint_info.key,
            )?,
        ),
        crate::state::WriterBidDeliveryMode::ClassicSpl => {
            let destination = validate_token_account(destination_info)?;
            if destination.owner != bid.bidder
                || destination.mint != *contract_mint_info.key
                || destination.state != AccountState::Initialized
            {
                return Err(VaultError::InvalidTokenAccount.into());
            }
            Some(destination)
        }
    };
    let interface_before = validate_token_account(interface_info)?;
    if interface_before.mint != *contract_mint_info.key || interface_before.owner != cpi_authority()
    {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }
    let realized_staging = load_or_create_market_staging(
        program_id,
        cranker_info,
        market_info,
        &market,
        staging_info,
        contract_mint_info,
        token_program_info,
        system_program_info,
    )?;
    if realized_staging.amount != staging_before {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let market_bump = [market.bump];
    let market_signer = market_signer_seeds(&market, &market_bump);
    if staging_before != 0 {
        let realized_retirement = load_or_create_writer_retirement_custody(
            program_id,
            cranker_info,
            sleeve_info,
            market_info,
            retirement_info,
            contract_mint_info,
            token_program_info,
            system_program_info,
        )?;
        if realized_retirement.amount != retirement_before {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        invoke_token_transfer_checked(
            token_program_info,
            staging_info,
            contract_mint_info,
            retirement_info,
            market_info,
            staging_before,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &[&market_signer],
        )?;
        if validate_token_account(retirement_info)?
            .amount
            .checked_sub(retirement_before)
            != Some(staging_before)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
    }
    if validate_token_account(staging_info)?.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    invoke_token_mint_to_checked(
        token_program_info,
        contract_mint_info,
        staging_info,
        market_info,
        accepted,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&market_signer],
    )?;
    invoke_light_token_account_transfer_with_signer_seeds(
        accepted,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        cranker_info,
        staging_info,
        destination_info,
        market_info,
        contract_mint_info,
        interface_info,
        token_program_info,
        system_program_info,
        &[&market_signer],
    )?;
    let auction_bump = [auction.bump];
    let auction_signer: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_AUCTION_PDA_SEED,
        sleeve_info.key.as_ref(),
        &auction.auction_nonce.to_le_bytes(),
        &auction_bump,
    ];
    if premium != 0 {
        invoke_token_transfer_checked(
            token_program_info,
            escrow_info,
            settlement_mint_info,
            sleeve_vault_info,
            auction_info,
            premium,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &[auction_signer],
        )?;
    }
    if fee != 0 {
        invoke_token_transfer_checked(
            token_program_info,
            escrow_info,
            settlement_mint_info,
            fee_vault_info,
            auction_info,
            fee,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &[auction_signer],
        )?;
    }
    let mint_after = validate_mint_account(contract_mint_info, token_program_info.key)?;
    let destination_after = match bid.delivery_mode {
        crate::state::WriterBidDeliveryMode::LightToken => {
            crate::processor::scoped_settlement::load_scoped_holder_token_account(
                program_id,
                destination_info,
                &bid.bidder,
                contract_mint_info.key,
            )?
        }
        crate::state::WriterBidDeliveryMode::ClassicSpl => {
            validate_token_account(destination_info)?
        }
    };
    if mint_after.supply.checked_sub(mint_before.supply) != Some(accepted)
        || validate_token_account(staging_info)?.amount != 0
        || destination_after
            .amount
            .checked_sub(destination_before.unwrap().amount)
            != Some(accepted)
        || validate_token_account(escrow_info)?
            .amount
            .checked_add(charged)
            != Some(escrow_before)
        || validate_token_account(sleeve_vault_info)?
            .amount
            .checked_sub(sleeve_vault_before)
            != Some(premium)
        || validate_token_account(fee_vault_info)?
            .amount
            .checked_sub(fee_vault_before)
            != Some(fee)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    invoke_token_close_account(
        token_program_info,
        staging_info,
        cranker_info,
        market_info,
        &[&market_signer],
    )?;
    market.mint_accounting.total_issued = market
        .mint_accounting
        .total_issued
        .checked_add(accepted)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if market_outstanding_contract_amount(&market)?
        != book.records[series_index].external_open_interest_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let refund = bid
        .escrowed_atoms
        .checked_sub(charged)
        .ok_or(VaultError::ArithmeticOverflow)?;
    bid.accepted_contract_atoms = accepted;
    bid.executed_contract_atoms = accepted;
    bid.premium_charged_atoms = premium;
    bid.fee_charged_atoms = fee;
    bid.status = if refund == 0 {
        WriterBidStatus::Executed
    } else {
        WriterBidStatus::Refundable
    };
    let mut updated_summary = summary;
    updated_summary.executed_contract_atoms = accepted;
    updated_summary.status = bid.status;
    index.records[position] = updated_summary;
    index.executed_bid_count = index
        .executed_bid_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    auction.executed_bid_count = index.executed_bid_count;
    auction.executed_issue_atoms[series_index] = auction.executed_issue_atoms[series_index]
        .checked_add(accepted)
        .ok_or(VaultError::ArithmeticOverflow)?;
    auction.total_escrow_atoms = auction
        .total_escrow_atoms
        .checked_sub(charged)
        .ok_or(VaultError::ArithmeticOverflow)?;
    auction.refundable_atoms = auction
        .refundable_atoms
        .checked_add(refund)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let slot = Clock::get()?.slot;
    bid.last_updated_slot = slot;
    index.last_updated_slot = slot;
    auction.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(market_info, &market)?;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(bid_info, &bid)?;
    store_state(bid_index_info, &index)?;
    store_state(auction_info, &auction)
}
