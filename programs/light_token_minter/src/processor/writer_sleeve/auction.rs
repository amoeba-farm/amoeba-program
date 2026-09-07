use super::*;
use crate::instruction::{
    CommitWriterAuctionV1Params, FinalizeOrAbortWriterAuctionV1Params, PlaceWriterBidV1Params,
    PlanWriterAuctionChunkV1Params, RevealWriterAuctionV1Params,
};

const COMMIT_WRITER_AUCTION_ACCOUNT_COUNT: usize = 14;
const PLACE_WRITER_BID_ACCOUNT_COUNT: usize = 13;
const CANCEL_OR_REFUND_WRITER_BID_ACCOUNT_COUNT: usize = 9;
const REVEAL_WRITER_AUCTION_ACCOUNT_COUNT: usize = 6;
const PLAN_WRITER_AUCTION_ACCOUNT_COUNT: usize = 9;
const EXECUTE_WRITER_AUCTION_FILL_ACCOUNT_COUNT: usize = 26;
const FINALIZE_WRITER_AUCTION_ACCOUNT_COUNT: usize = 5;
const MAX_PLAN_RECORDS_PER_CALL: u16 = 8;
const WRITER_AUCTION_RESERVE_DOMAIN: &[u8] = b"ameba-writer-auction-reserve-v1";
const WRITER_AUCTION_RESERVE_SLOT_DOMAIN: &[u8] = b"ameba-writer-auction-reserve-slot-v1";
const WRITER_AUCTION_RESERVE_PREIMAGE_LEN: usize =
    WRITER_AUCTION_RESERVE_DOMAIN.len() + 8 * 32 + 5 * 8 + 64 * 8;
const WRITER_BID_INDEX_DIGEST_DOMAIN: &[u8] = b"ameba-writer-bid-index-v1";
const WRITER_PLAN_DIGEST_DOMAIN: &[u8] = b"ameba-writer-auction-plan-v1";

fn checked_premium(
    quantity_atoms: u64,
    price_per_contract_atoms: u64,
) -> Result<u64, ProgramError> {
    if !quantity_atoms.is_multiple_of(MarketMintAccounting::CANONICAL_ATOMIC_SCALE) {
        return Err(VaultError::InvalidWriterBid.into());
    }
    let contracts = quantity_atoms / MarketMintAccounting::CANONICAL_ATOMIC_SCALE;
    contracts
        .checked_mul(price_per_contract_atoms)
        .ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

fn checked_fee(premium_atoms: u64, fee_bps: u16) -> Result<u64, ProgramError> {
    let numerator = u128::from(premium_atoms)
        .checked_mul(u128::from(fee_bps))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let fee = numerator
        .checked_add(9_999)
        .ok_or(VaultError::ArithmeticOverflow)?
        / 10_000;
    u64::try_from(fee).map_err(|_| VaultError::ArithmeticOverflow.into())
}

fn is_current_active_writer_auction(
    sleeve: &WriterSleeveV1,
    auction_key: &Pubkey,
    auction: &WriterAuctionV1,
) -> bool {
    sleeve.auction_nonce == auction.auction_nonce && sleeve.active_auction == Some(*auction_key)
}

#[inline]
fn writer_auction_commit_window_open(now: u64, bid_deadline: u64) -> bool {
    now < bid_deadline
}

#[inline]
fn writer_auction_bid_window_open(now: u64, bid_deadline: u64) -> bool {
    now <= bid_deadline
}

#[inline]
fn writer_auction_reveal_window_open(now: u64, bid_deadline: u64, reveal_deadline: u64) -> bool {
    now > bid_deadline && now < reveal_deadline
}

#[inline]
fn writer_auction_planning_window_open(
    now: u64,
    reveal_deadline: u64,
    execute_deadline: u64,
) -> bool {
    now >= reveal_deadline && now <= execute_deadline
}

#[inline]
fn writer_auction_execute_deadline_open(now: u64, execute_deadline: u64) -> bool {
    now <= execute_deadline
}

#[inline]
fn writer_auction_deadline_sequence_valid(
    bid_deadline: u64,
    reveal_deadline: u64,
    execute_deadline: u64,
    expiry: u64,
) -> bool {
    match bid_deadline.checked_add(1) {
        Some(first_reveal_timestamp) => {
            first_reveal_timestamp < reveal_deadline
                && reveal_deadline < execute_deadline
                && execute_deadline < expiry
        }
        None => false,
    }
}

#[inline]
fn writer_auction_abortable(
    status: WriterAuctionStatus,
    now: u64,
    reveal_deadline: u64,
    execute_deadline: u64,
) -> bool {
    (status == WriterAuctionStatus::Bidding && now >= reveal_deadline)
        || (matches!(
            status,
            WriterAuctionStatus::Planning | WriterAuctionStatus::Executing
        ) && now > execute_deadline)
}

#[inline]
fn writer_auction_policy_inputs_match(
    auction: &WriterAuctionV1,
    snapshot: &WriterPolicySnapshotV1,
) -> bool {
    auction.policy_version == snapshot.policy_version
        && auction.scenario_set_hash == snapshot.scenario_set_hash
        && auction.risk_limit_hash == snapshot.risk_limit_hash
}

fn reserve_reveal_precommitment_hash(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    auction: &Pubkey,
    auction_state: &WriterAuctionV1,
    snapshot: &WriterPolicySnapshotV1,
    params: &RevealWriterAuctionV1Params,
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(WRITER_AUCTION_RESERVE_PREIMAGE_LEN);
    bytes.extend_from_slice(WRITER_AUCTION_RESERVE_DOMAIN);
    bytes.extend_from_slice(program_id.as_ref());
    bytes.extend_from_slice(sleeve.as_ref());
    bytes.extend_from_slice(auction.as_ref());
    bytes.extend_from_slice(&auction_state.auction_nonce.to_le_bytes());
    bytes.extend_from_slice(&snapshot.policy_version.to_le_bytes());
    bytes.extend_from_slice(&snapshot.policy_hash);
    bytes.extend_from_slice(&auction_state.scenario_set_hash);
    bytes.extend_from_slice(&auction_state.risk_limit_hash);
    bytes.extend_from_slice(&snapshot.series_family_hash);
    for value in params.reserve_prices_atoms {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in params.issue_caps_atoms {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&params.nonce);
    bytes.extend_from_slice(&auction_state.bid_deadline_ts.to_le_bytes());
    bytes.extend_from_slice(&auction_state.reveal_deadline_ts.to_le_bytes());
    bytes.extend_from_slice(&auction_state.execute_deadline_ts.to_le_bytes());
    debug_assert_eq!(bytes.len(), WRITER_AUCTION_RESERVE_PREIMAGE_LEN);
    hashv(&[bytes.as_slice()]).to_bytes()
}

#[inline]
fn slot_bound_reserve_commitment(precommitment: &[u8; 32], commit_slot: u64) -> [u8; 32] {
    hashv(&[
        WRITER_AUCTION_RESERVE_SLOT_DOMAIN,
        precommitment,
        &commit_slot.to_le_bytes(),
    ])
    .to_bytes()
}

#[inline]
fn writer_auction_reveal_binding_matches(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    auction: &Pubkey,
    auction_state: &WriterAuctionV1,
    snapshot: &WriterPolicySnapshotV1,
    params: &RevealWriterAuctionV1Params,
) -> bool {
    writer_auction_policy_inputs_match(auction_state, snapshot)
        && slot_bound_reserve_commitment(
            &reserve_reveal_precommitment_hash(
                program_id,
                sleeve,
                auction,
                auction_state,
                snapshot,
                params,
            ),
            auction_state.commit_slot,
        ) == auction_state.reserve_vector_commitment
}

fn bid_index_digest(previous: &[u8; 32], record: &WriterBidIndexRecordV1) -> [u8; 32] {
    hashv(&[
        WRITER_BID_INDEX_DIGEST_DOMAIN,
        previous,
        record.bid.as_ref(),
        record.bidder.as_ref(),
        &record.order_id.to_le_bytes(),
        &[record.series_index],
        &record.bid_price_per_contract_atoms.to_le_bytes(),
        &record.requested_contract_atoms.to_le_bytes(),
        &record.escrowed_atoms.to_le_bytes(),
    ])
    .to_bytes()
}

fn plan_digest(previous: &[u8; 32], record: &WriterBidIndexRecordV1) -> [u8; 32] {
    let status = match record.status {
        WriterBidStatus::Planned => 1,
        WriterBidStatus::Refundable => 2,
        _ => 0,
    };
    hashv(&[
        WRITER_PLAN_DIGEST_DOMAIN,
        previous,
        record.bid.as_ref(),
        &record.accepted_contract_atoms.to_le_bytes(),
        &[status],
    ])
    .to_bytes()
}

fn bid_precedes(
    candidate: &WriterBidIndexRecordV1,
    existing: &WriterBidIndexRecordV1,
    book: &WriterSeriesBookV1,
) -> bool {
    if candidate.bid_price_per_contract_atoms != existing.bid_price_per_contract_atoms {
        return candidate.bid_price_per_contract_atoms > existing.bid_price_per_contract_atoms;
    }
    let candidate_id = book.records[usize::from(candidate.series_index)].series_id;
    let existing_id = book.records[usize::from(existing.series_index)].series_id;
    if candidate_id != existing_id {
        return candidate_id < existing_id;
    }
    if candidate.order_id != existing.order_id {
        return candidate.order_id < existing.order_id;
    }
    candidate.bid.to_bytes() < existing.bid.to_bytes()
}

#[inline(never)]
fn validate_bid_index_series_bindings(
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

pub(super) fn process_commit_writer_auction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CommitWriterAuctionV1Params,
) -> ProgramResult {
    if accounts.len() != COMMIT_WRITER_AUCTION_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let sleeve_info = &accounts[3];
    let group_info = &accounts[4];
    let book_info = &accounts[5];
    let snapshot_info = &accounts[6];
    let auction_info = &accounts[7];
    let bid_index_info = &accounts[8];
    let escrow_info = &accounts[9];
    let fee_vault_info = &accounts[10];
    let settlement_mint_info = &accounts[11];
    let token_program_info = &accounts[12];
    let system_program_info = &accounts[13];
    if !authority_info.is_signer || !authority_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id()
        || *system_program_info.key != system_program::id()
        || crate::bytes32_is_zero(&params.reserve_vector_commitment)
    {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let WriterPolicyContext {
        group,
        mut sleeve,
        book: _,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        Some(registry_info.key),
    )?;
    let now = current_unix_timestamp()?;
    let expected_nonce = sleeve
        .auction_nonce
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if config.paused
        || registry.policy_authority != *authority_info.key
        || registry.protocol_fee_vault != *fee_vault_info.key
        || sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || params.auction_nonce != expected_nonce
        || now >= sleeve.expiry_ts
        || !writer_auction_commit_window_open(now, params.bid_deadline_ts)
        || !writer_auction_deadline_sequence_valid(
            params.bid_deadline_ts,
            params.reveal_deadline_ts,
            params.execute_deadline_ts,
            sleeve.expiry_ts,
        )
    {
        return Err(VaultError::InvalidWriterDeadline.into());
    }
    validate_vault_token_account(fee_vault_info, settlement_mint_info.key, registry_info.key)?;
    if config.usdc_mint != *settlement_mint_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
    {
        return Err(VaultError::InvalidMint.into());
    }
    let (expected_auction, auction_bump) =
        derive_writer_auction_pda(program_id, sleeve_info.key, params.auction_nonce);
    let (expected_index, index_bump) = derive_writer_bid_index_pda(program_id, auction_info.key);
    let (expected_escrow, escrow_bump) =
        derive_writer_auction_escrow_pda(program_id, auction_info.key);
    if *auction_info.key != expected_auction
        || *bid_index_info.key != expected_index
        || *escrow_info.key != expected_escrow
    {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, auction_info)?;
    super::bid_index_preparation::validate_ready_bid_index(program_id, bid_index_info)?;
    create_program_account(
        authority_info,
        auction_info,
        system_program_info,
        program_id,
        WriterAuctionV1::LEN,
        &[
            crate::constants::WRITER_AUCTION_PDA_SEED,
            sleeve_info.key.as_ref(),
            &params.auction_nonce.to_le_bytes(),
            &[auction_bump],
        ],
    )?;
    create_classic_token_pda(
        program_id,
        authority_info,
        escrow_info,
        settlement_mint_info,
        auction_info.key,
        token_program_info,
        system_program_info,
        &[
            crate::constants::WRITER_AUCTION_ESCROW_PDA_SEED,
            auction_info.key.as_ref(),
            &[escrow_bump],
        ],
    )?;
    if validate_token_account(escrow_info)?.amount != 0 {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    let slot = Clock::get()?.slot;
    let auction = WriterAuctionV1 {
        is_initialized: true,
        bump: auction_bump,
        account_discriminator: WriterAuctionV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterAuctionV1::ACCOUNT_VERSION,
        sleeve: *sleeve_info.key,
        series_book: *book_info.key,
        policy_snapshot: *snapshot_info.key,
        auction_nonce: params.auction_nonce,
        escrow: *escrow_info.key,
        bid_index: *bid_index_info.key,
        fee_vault: *fee_vault_info.key,
        policy_version: snapshot.policy_version,
        scenario_set_hash: snapshot.scenario_set_hash,
        risk_limit_hash: snapshot.risk_limit_hash,
        // The caller supplies the reveal-input precommitment. Store only the commitment bound to
        // the actual execution slot so a client never has to predict that slot and any later slot
        // mutation invalidates reveal. This preserves the V1 instruction and account layouts.
        reserve_vector_commitment: slot_bound_reserve_commitment(
            &params.reserve_vector_commitment,
            slot,
        ),
        reveal_hash: [0; 32],
        revealed_nonce: [0; 32],
        commit_slot: slot,
        bid_deadline_ts: params.bid_deadline_ts,
        reveal_deadline_ts: params.reveal_deadline_ts,
        execute_deadline_ts: params.execute_deadline_ts,
        bid_count: 0,
        planned_bid_count: 0,
        executed_bid_count: 0,
        refunded_bid_count: 0,
        planning_cursor: 0,
        reserved_0: [0; 6],
        total_escrow_atoms: 0,
        accepted_premium_atoms: 0,
        accepted_fee_atoms: 0,
        accepted_contract_atoms: 0,
        refundable_atoms: 0,
        plan_digest: [0; 32],
        status: WriterAuctionStatus::Bidding,
        reserved_1: [0; 7],
        last_updated_slot: slot,
        reserve_prices_atoms: [0; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
        issue_caps_atoms: [0; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
        planned_issue_atoms: [0; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
        executed_issue_atoms: [0; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
    };
    let index = WriterBidIndexV1 {
        is_initialized: true,
        bump: index_bump,
        account_discriminator: WriterBidIndexV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterBidIndexV1::ACCOUNT_VERSION,
        auction: *auction_info.key,
        bid_count: 0,
        planned_bid_count: 0,
        executed_bid_count: 0,
        refunded_bid_count: 0,
        planning_cursor: 0,
        rolling_digest: [0; 32],
        last_updated_slot: slot,
        records: vec![WriterBidIndexRecordV1::EMPTY; crate::constants::WRITER_MAX_FUNDED_BIDS]
            .into_boxed_slice()
            .try_into()
            .map_err(|_| VaultError::ArithmeticOverflow)?,
    };
    sleeve.auction_nonce = params.auction_nonce;
    sleeve.active_auction = Some(*auction_info.key);
    sleeve.last_updated_slot = slot;
    store_state(auction_info, &auction)?;
    store_state(bid_index_info, &index)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_place_writer_bid(
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
    match params.delivery_mode {
        crate::state::WriterBidDeliveryMode::LightToken => {
            validate_light_associated_token_destination(
                bidder_info.key,
                &record.contract_mint,
                claim_destination_info,
            )?;
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
    store_state(auction_info, &auction)
}

pub(super) fn process_reveal_writer_auction(
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

pub(super) fn process_plan_writer_auction_chunk(
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

#[allow(clippy::too_many_arguments)]
fn load_or_create_market_staging<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    market: &Market,
    staging_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<TokenAccount, ProgramError> {
    let (expected, bump) = derive_contract_mint_staging_pda(program_id, market_info.key);
    if *staging_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if staging_info.owner == token_program_info.key {
        validate_vault_token_account(staging_info, mint_info.key, market_info.key)?;
        return validate_token_account(staging_info);
    }
    validate_create_only_program_account_target(program_id, staging_info)?;
    create_program_account(
        payer_info,
        staging_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        &[
            CONTRACT_MINT_STAGING_PDA_SEED,
            market_info.key.as_ref(),
            &[bump],
        ],
    )?;
    invoke_token_initialize_account3(token_program_info, staging_info, mint_info, market_info.key)?;
    validate_vault_token_account(staging_info, mint_info.key, market_info.key)?;
    let _ = market;
    validate_token_account(staging_info)
}

fn observe_market_staging_amount(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    staging_info: &AccountInfo,
    mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
) -> Result<u64, ProgramError> {
    let expected = derive_contract_mint_staging_pda(program_id, market_info.key).0;
    if *staging_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if staging_info.owner == token_program_info.key {
        validate_vault_token_account(staging_info, mint_info.key, market_info.key)?;
        return Ok(validate_token_account(staging_info)?.amount);
    }
    validate_create_only_program_account_target(program_id, staging_info)?;
    Ok(0)
}

fn observe_writer_retirement_custody_amount(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    market_info: &AccountInfo,
    custody_info: &AccountInfo,
    mint_info: &AccountInfo,
    token_program_info: &AccountInfo,
) -> Result<u64, ProgramError> {
    let expected =
        derive_writer_retirement_custody_pda(program_id, sleeve_info.key, market_info.key).0;
    if *custody_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if custody_info.owner == token_program_info.key {
        validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
        return Ok(validate_token_account(custody_info)?.amount);
    }
    validate_create_only_program_account_target(program_id, custody_info)?;
    Ok(0)
}

fn market_signer_seeds<'a>(market: &'a Market, bump: &'a [u8; 1]) -> [&'a [u8]; 4] {
    [
        CURRENT_STATE_NAMESPACE_SEED,
        MARKET_PDA_SEED,
        &market.market_id,
        bump,
    ]
}

pub(super) fn process_execute_writer_auction_fill(
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
        crate::state::WriterBidDeliveryMode::LightToken => {
            Some(load_canonical_light_token_account(
                destination_info,
                &bid.bidder,
                contract_mint_info.key,
            )?)
        }
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
        crate::state::WriterBidDeliveryMode::LightToken => load_canonical_light_token_account(
            destination_info,
            &bid.bidder,
            contract_mint_info.key,
        )?,
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

pub(super) fn process_finalize_or_abort_writer_auction(
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

pub(super) fn process_cancel_or_refund_writer_bid(
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
