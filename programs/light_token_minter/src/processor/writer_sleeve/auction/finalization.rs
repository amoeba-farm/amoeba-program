use super::*;

pub(in crate::processor::writer_sleeve) fn process_finalize_or_abort_writer_auction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: FinalizeOrAbortWriterAuctionV1Params,
) -> ProgramResult {
    if accounts.len() != FINALIZE_WRITER_AUCTION_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let auction_info = &accounts[2];
    let bid_index_info = &accounts[3];
    let escrow_info = &accounts[4];
    if !actor_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let mut auction = load_writer_auction(
        program_id,
        auction_info,
        sleeve_info.key,
        sleeve.auction_nonce,
    )?;
    let mut index = load_writer_bid_index(program_id, bid_index_info, auction_info.key)?;
    if sleeve.active_auction != Some(*auction_info.key)
        || auction.bid_index != *bid_index_info.key
        || auction.escrow != *escrow_info.key
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let now = current_unix_timestamp()?;
    if params.abort {
        if !writer_auction_abortable(
            auction.status,
            now,
            auction.reveal_deadline_ts,
            auction.execute_deadline_ts,
        ) {
            return Err(VaultError::InvalidWriterDeadline.into());
        }
        for record in index.records.iter_mut().take(usize::from(index.bid_count)) {
            if matches!(
                record.status,
                WriterBidStatus::Funded | WriterBidStatus::Planned
            ) {
                record.status = WriterBidStatus::Refundable;
            }
        }
        auction.refundable_atoms = auction.total_escrow_atoms;
        auction.status = WriterAuctionStatus::Refundable;
    } else {
        if auction.status != WriterAuctionStatus::Executing
            || index
                .records
                .iter()
                .take(usize::from(index.bid_count))
                .any(|record| {
                    matches!(
                        record.status,
                        WriterBidStatus::Funded | WriterBidStatus::Planned
                    )
                })
        {
            return Err(VaultError::InvalidWriterLifecycle.into());
        }
        auction.status = WriterAuctionStatus::Finalized;
    }
    let physical = validate_token_account(escrow_info)?.amount;
    if physical < auction.total_escrow_atoms {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let slot = Clock::get()?.slot;
    sleeve.active_auction = None;
    sleeve.last_updated_slot = slot;
    auction.last_updated_slot = slot;
    index.last_updated_slot = slot;
    store_state(sleeve_info, &sleeve)?;
    store_state(bid_index_info, &index)?;
    store_state(auction_info, &auction)
}

pub(in crate::processor::writer_sleeve) fn process_cancel_or_refund_writer_bid(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != CANCEL_OR_REFUND_WRITER_BID_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let auction_info = &accounts[2];
    let bid_index_info = &accounts[3];
    let bid_info = &accounts[4];
    let escrow_info = &accounts[5];
    let refund_info = &accounts[6];
    let settlement_mint_info = &accounts[7];
    let token_program_info = &accounts[8];
    if !actor_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    let sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let mut auction = load_writer_auction_for_refund(
        program_id,
        auction_info,
        sleeve_info.key,
        sleeve.auction_nonce,
    )?;
    let mut index = load_writer_bid_index(program_id, bid_index_info, auction_info.key)?;
    let mut bid = load_writer_bid(program_id, bid_info, auction_info.key)?;
    if auction.bid_index != *bid_index_info.key
        || auction.escrow != *escrow_info.key
        || bid.refund_token_account != *refund_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
    {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let position = index
        .records
        .iter()
        .take(usize::from(index.bid_count))
        .position(|record| record.bid == *bid_info.key)
        .ok_or(VaultError::InvalidWriterBid)?;
    let mut summary = index.records[position];
    if summary.bidder != bid.bidder
        || summary.order_id != bid.order_id
        || summary.series_index != bid.series_index
        || summary.escrowed_atoms != bid.escrowed_atoms
    {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let now = current_unix_timestamp()?;
    let early_cancel = summary.status == WriterBidStatus::Funded
        && auction.status == WriterAuctionStatus::Bidding
        && writer_auction_bid_window_open(now, auction.bid_deadline_ts)
        && is_current_active_writer_auction(&sleeve, auction_info.key, &auction);
    if early_cancel && *actor_info.key != bid.bidder {
        return Err(VaultError::Unauthorized.into());
    }
    let refundable_status = matches!(
        summary.status,
        WriterBidStatus::Refundable | WriterBidStatus::Cancelled
    ) || matches!(
        auction.status,
        WriterAuctionStatus::Refundable | WriterAuctionStatus::Finalized
    );
    if !early_cancel && !refundable_status {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let charged = bid
        .premium_charged_atoms
        .checked_add(bid.fee_charged_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let refundable = bid
        .escrowed_atoms
        .checked_sub(charged)
        .and_then(|amount| amount.checked_sub(bid.refunded_atoms))
        .ok_or(VaultError::InvalidWriterBid)?;
    if refundable == 0 {
        return Err(VaultError::InvalidWriterBid.into());
    }
    validate_vault_token_account(escrow_info, settlement_mint_info.key, auction_info.key)?;
    let refund = validate_token_account(refund_info)?;
    if refund.owner != bid.bidder
        || refund.mint != *settlement_mint_info.key
        || refund.state != AccountState::Initialized
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let escrow_before = validate_token_account(escrow_info)?.amount;
    let refund_before = refund.amount;
    if escrow_before < auction.total_escrow_atoms || refundable > auction.total_escrow_atoms {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let auction_bump = [auction.bump];
    let auction_signer_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_AUCTION_PDA_SEED,
        sleeve_info.key.as_ref(),
        &auction.auction_nonce.to_le_bytes(),
        &auction_bump,
    ];
    invoke_token_transfer_checked(
        token_program_info,
        escrow_info,
        settlement_mint_info,
        refund_info,
        auction_info,
        refundable,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[auction_signer_seeds],
    )?;
    if escrow_before.checked_sub(validate_token_account(escrow_info)?.amount) != Some(refundable)
        || validate_token_account(refund_info)?
            .amount
            .checked_sub(refund_before)
            != Some(refundable)
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let slot = Clock::get()?.slot;
    bid.refunded_atoms = bid
        .refunded_atoms
        .checked_add(refundable)
        .ok_or(VaultError::ArithmeticOverflow)?;
    bid.status = WriterBidStatus::Refunded;
    bid.last_updated_slot = slot;
    summary.status = WriterBidStatus::Refunded;
    index.records[position] = summary;
    index.refunded_bid_count = index
        .refunded_bid_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    index.last_updated_slot = slot;
    auction.refunded_bid_count = index.refunded_bid_count;
    auction.total_escrow_atoms = auction
        .total_escrow_atoms
        .checked_sub(refundable)
        .ok_or(VaultError::ArithmeticOverflow)?;
    auction.refundable_atoms = auction.refundable_atoms.saturating_sub(refundable);
    auction.last_updated_slot = slot;
    store_state(bid_info, &bid)?;
    store_state(bid_index_info, &index)?;
    store_state(auction_info, &auction)
}
