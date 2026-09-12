use super::*;

pub(super) fn checked_premium(
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

pub(super) fn checked_fee(premium_atoms: u64, fee_bps: u16) -> Result<u64, ProgramError> {
    let numerator = u128::from(premium_atoms)
        .checked_mul(u128::from(fee_bps))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let fee = numerator
        .checked_add(9_999)
        .ok_or(VaultError::ArithmeticOverflow)?
        / 10_000;
    u64::try_from(fee).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(super) fn is_current_active_writer_auction(
    sleeve: &WriterSleeveV1,
    auction_key: &Pubkey,
    auction: &WriterAuctionV1,
) -> bool {
    sleeve.auction_nonce == auction.auction_nonce && sleeve.active_auction == Some(*auction_key)
}

#[inline]
pub(super) fn writer_auction_commit_window_open(now: u64, bid_deadline: u64) -> bool {
    now < bid_deadline
}

#[inline]
pub(super) fn writer_auction_bid_window_open(now: u64, bid_deadline: u64) -> bool {
    now <= bid_deadline
}

#[inline]
pub(super) fn writer_auction_reveal_window_open(
    now: u64,
    bid_deadline: u64,
    reveal_deadline: u64,
) -> bool {
    now > bid_deadline && now < reveal_deadline
}

#[inline]
pub(super) fn writer_auction_planning_window_open(
    now: u64,
    reveal_deadline: u64,
    execute_deadline: u64,
) -> bool {
    now >= reveal_deadline && now <= execute_deadline
}

#[inline]
pub(super) fn writer_auction_execute_deadline_open(now: u64, execute_deadline: u64) -> bool {
    now <= execute_deadline
}

#[inline]
pub(super) fn writer_auction_deadline_sequence_valid(
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
pub(super) fn writer_auction_abortable(
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
pub(super) fn writer_auction_policy_inputs_match(
    auction: &WriterAuctionV1,
    snapshot: &WriterPolicySnapshotV1,
) -> bool {
    auction.policy_version == snapshot.policy_version
        && auction.scenario_set_hash == snapshot.scenario_set_hash
        && auction.risk_limit_hash == snapshot.risk_limit_hash
}
