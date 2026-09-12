use super::*;

pub(in crate::processor::writer_sleeve) fn process_reveal_writer_auction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RevealWriterAuctionV1Params,
) -> ProgramResult {
    if accounts.len() != REVEAL_WRITER_AUCTION_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let registry_info = &accounts[1];
    let snapshot_info = &accounts[2];
    let sleeve_info = &accounts[3];
    let auction_info = &accounts[4];
    let bid_index_info = &accounts[5];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let registry = load_writer_policy_registry(program_id, registry_info, &sleeve.vault_config)?;
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        registry_info.key,
        sleeve.policy_version,
    )?;
    let mut auction = load_writer_auction(
        program_id,
        auction_info,
        sleeve_info.key,
        sleeve.auction_nonce,
    )?;
    let index = load_writer_bid_index(program_id, bid_index_info, auction_info.key)?;
    let now = current_unix_timestamp()?;
    if registry.policy_authority != *authority_info.key
        || sleeve.active_auction != Some(*auction_info.key)
        || auction.policy_snapshot != *snapshot_info.key
        || auction.bid_index != *bid_index_info.key
        || auction.status != WriterAuctionStatus::Bidding
        || auction.bid_count != index.bid_count
        || !writer_auction_reveal_window_open(
            now,
            auction.bid_deadline_ts,
            auction.reveal_deadline_ts,
        )
        || !writer_auction_reveal_binding_matches(
            program_id,
            sleeve_info.key,
            auction_info.key,
            &auction,
            &snapshot,
            &params,
        )
        || crate::bytes32_is_zero(&params.nonce)
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let series_count = usize::from(sleeve.series_count);
    let mut total_cap = 0u64;
    for index in 0..crate::constants::WRITER_SERIES_STORAGE_CAPACITY {
        let reserve = params.reserve_prices_atoms[index];
        let cap = params.issue_caps_atoms[index];
        if index < series_count {
            if reserve == 0 || !cap.is_multiple_of(MarketMintAccounting::CANONICAL_ATOMIC_SCALE) {
                return Err(VaultError::InvalidWriterAuction.into());
            }
            total_cap = total_cap
                .checked_add(cap)
                .ok_or(VaultError::ArithmeticOverflow)?;
        } else if reserve != 0 || cap != 0 {
            return Err(VaultError::InvalidWriterAuction.into());
        }
    }
    if total_cap > snapshot.max_auction_issue_atoms {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let slot = Clock::get()?.slot;
    auction.reveal_hash = auction.reserve_vector_commitment;
    auction.revealed_nonce = params.nonce;
    auction.reserve_prices_atoms = params.reserve_prices_atoms;
    auction.issue_caps_atoms = params.issue_caps_atoms;
    auction.status = WriterAuctionStatus::Planning;
    auction.last_updated_slot = slot;
    sleeve.last_updated_slot = slot;
    store_state(auction_info, &auction)?;
    store_state(sleeve_info, &sleeve)
}
