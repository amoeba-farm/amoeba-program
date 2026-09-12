use super::*;

#[inline(never)]
pub(super) fn validate_bid_index_series_bindings(
    index: &WriterBidIndexV1,
    book: &WriterSeriesBookV1,
) -> ProgramResult {
    if index.records[..usize::from(index.bid_count)]
        .iter()
        .any(|record| usize::from(record.series_index) >= usize::from(book.series_count))
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    Ok(())
}

// Historical fixture construction only; tag 233 cannot invoke this in the program.

pub(in crate::processor::writer_sleeve) fn process_place_writer_bid(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: PlaceWriterBidV1Params,
) -> ProgramResult {
    if accounts.len() != PLACE_WRITER_BID_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let bidder_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let auction_info = &accounts[2];
    let bid_index_info = &accounts[3];
    let bid_info = &accounts[4];
    let book_info = &accounts[5];
    let snapshot_info = &accounts[6];
    let source_info = &accounts[7];
    let escrow_info = &accounts[8];
    let settlement_mint_info = &accounts[9];
    let claim_destination_info = &accounts[10];
    let token_program_info = &accounts[11];
    let system_program_info = &accounts[12];
    if !bidder_info.is_signer || !bidder_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *system_program_info.key != system_program::id()
        || params.bid_price_per_contract_atoms == 0
        || params.requested_contract_atoms == 0
    {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let mut auction = load_writer_auction(
        program_id,
        auction_info,
        sleeve_info.key,
        sleeve.auction_nonce,
    )?;
    let mut index = load_writer_bid_index(program_id, bid_index_info, auction_info.key)?;
    let book = load_writer_series_book(
        program_id,
        book_info,
        sleeve_info.key,
        &sleeve.settlement_group,
    )?;
    validate_bid_index_series_bindings(&index, &book)?;
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        &sleeve.policy_registry,
        sleeve.policy_version,
    )?;
    let series_index = usize::from(params.series_index);
    if sleeve.active_auction != Some(*auction_info.key)
        || sleeve.status != WriterSleeveStatus::Active
        || auction.status != WriterAuctionStatus::Bidding
        || auction.series_book != *book_info.key
        || auction.policy_snapshot != *snapshot_info.key
        || !writer_auction_policy_inputs_match(&auction, &snapshot)
        || auction.bid_index != *bid_index_info.key
        || auction.escrow != *escrow_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
        || series_index >= usize::from(book.series_count)
        || usize::from(index.bid_count) >= crate::constants::WRITER_MAX_FUNDED_BIDS
        || !writer_auction_bid_window_open(current_unix_timestamp()?, auction.bid_deadline_ts)
        || !params
            .requested_contract_atoms
            .is_multiple_of(book.records[series_index].contract_size_atoms)
    {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let record = &book.records[series_index];
    if record.market != *accounts[13].key || record.contract_mint != *accounts[14].key {
        return Err(VaultError::InvalidAccountList.into());
    }
    match params.delivery_mode {
        crate::state::WriterBidDeliveryMode::LightToken => {
            if claim_destination_info.owner == &light_token_program_id() {
                let _ = crate::processor::scoped_settlement::load_scoped_holder_token_account(
                    program_id,
                    claim_destination_info,
                    bidder_info.key,
                    &record.contract_mint,
                )?;
            } else {
                validate_light_associated_token_destination(
                    bidder_info.key,
                    &record.contract_mint,
                    claim_destination_info,
                )?;
            }
        }
        crate::state::WriterBidDeliveryMode::ClassicSpl => {
            let destination = validate_token_account(claim_destination_info)?;
            if destination.owner != *bidder_info.key
                || destination.mint != record.contract_mint
                || destination.state != AccountState::Initialized
            {
                return Err(VaultError::InvalidTokenAccount.into());
            }
        }
    }
    validate_vault_token_account(escrow_info, settlement_mint_info.key, auction_info.key)?;
    let source = validate_token_account(source_info)?;
    if source.owner != *bidder_info.key
        || source.mint != *settlement_mint_info.key
        || source.state != AccountState::Initialized
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let premium = checked_premium(
        params.requested_contract_atoms,
        params.bid_price_per_contract_atoms,
    )?;
    let maximum_fee = checked_fee(premium, snapshot.primary_fee_bps)?;
    let escrowed = premium
        .checked_add(maximum_fee)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if source.amount < escrowed {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let (expected_bid, bid_bump) = derive_writer_bid_pda(
        program_id,
        auction_info.key,
        bidder_info.key,
        params.order_id,
    );
    if *bid_info.key != expected_bid {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, bid_info)?;
    let new_record = WriterBidIndexRecordV1 {
        occupied: true,
        status: WriterBidStatus::Funded,
        series_index: params.series_index,
        reserved: 0,
        bid_price_per_contract_atoms: params.bid_price_per_contract_atoms,
        requested_contract_atoms: params.requested_contract_atoms,
        accepted_contract_atoms: 0,
        executed_contract_atoms: 0,
        escrowed_atoms: escrowed,
        bid: *bid_info.key,
        bidder: *bidder_info.key,
        order_id: params.order_id,
    };
    let count = usize::from(index.bid_count);
    let insertion = (0..count)
        .find(|position| bid_precedes(&new_record, &index.records[*position], &book))
        .unwrap_or(count);
    for destination in (insertion + 1..=count).rev() {
        index.records[destination] = index.records[destination - 1];
    }
    index.records[insertion] = new_record;
    index.bid_count = index
        .bid_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    index.rolling_digest = bid_index_digest(&index.rolling_digest, &new_record);
    let escrow_before = validate_token_account(escrow_info)?.amount;
    if escrow_before < auction.total_escrow_atoms {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    invoke_token_transfer_checked(
        token_program_info,
        source_info,
        settlement_mint_info,
        escrow_info,
        bidder_info,
        escrowed,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[],
    )?;
    if validate_token_account(escrow_info)?
        .amount
        .checked_sub(escrow_before)
        != Some(escrowed)
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    create_program_account(
        bidder_info,
        bid_info,
        system_program_info,
        program_id,
        WriterBidV1::LEN,
        &[
            crate::constants::WRITER_BID_PDA_SEED,
            auction_info.key.as_ref(),
            bidder_info.key.as_ref(),
            &params.order_id.to_le_bytes(),
            &[bid_bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let bid = WriterBidV1 {
        is_initialized: true,
        bump: bid_bump,
        account_discriminator: WriterBidV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterBidV1::ACCOUNT_VERSION,
        auction: *auction_info.key,
        bidder: *bidder_info.key,
        refund_token_account: *source_info.key,
        claim_destination: *claim_destination_info.key,
        order_id: params.order_id,
        series_index: params.series_index,
        status: WriterBidStatus::Funded,
        delivery_mode: params.delivery_mode,
        reserved: [0; 5],
        bid_price_per_contract_atoms: params.bid_price_per_contract_atoms,
        requested_contract_atoms: params.requested_contract_atoms,
        accepted_contract_atoms: 0,
        executed_contract_atoms: 0,
        escrowed_atoms: escrowed,
        premium_charged_atoms: 0,
        fee_charged_atoms: 0,
        refunded_atoms: 0,
        placed_slot: slot,
        last_updated_slot: slot,
    };
    auction.bid_count = index.bid_count;
    auction.total_escrow_atoms = auction
        .total_escrow_atoms
        .checked_add(escrowed)
        .ok_or(VaultError::ArithmeticOverflow)?;
    auction.last_updated_slot = slot;
    index.last_updated_slot = slot;
    store_state(bid_info, &bid)?;
    store_state(bid_index_info, &index)?;
    store_state(auction_info, &auction)?;
    if params.delivery_mode == crate::state::WriterBidDeliveryMode::LightToken {
        crate::processor::scoped_settlement::authorize_collective_settlement(
            program_id,
            &[
                accounts[0].clone(),
                accounts[13].clone(),
                accounts[14].clone(),
                accounts[10].clone(),
                accounts[15].clone(),
                accounts[16].clone(),
                accounts[17].clone(),
                accounts[18].clone(),
                accounts[12].clone(),
            ],
            false,
        )?;
    }
    Ok(())
}
