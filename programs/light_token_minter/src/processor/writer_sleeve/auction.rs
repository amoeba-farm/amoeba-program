use super::*;
use crate::instruction::{
    FinalizeOrAbortWriterAuctionV1Params, PlaceWriterBidV1Params, PlanWriterAuctionChunkV1Params,
    RevealWriterAuctionV1Params,
};

const COMMIT_WRITER_AUCTION_ACCOUNT_COUNT: usize = 14;
const PLACE_WRITER_BID_ACCOUNT_COUNT: usize = 19;
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

mod bidding;
mod commitments;
mod custody;
mod execution;
mod finalization;
mod planning;
mod reveal;
mod rules;

pub(super) use bidding::process_place_writer_bid;
use bidding::validate_bid_index_series_bindings;
use commitments::{
    bid_index_digest, bid_precedes, plan_digest, writer_auction_reveal_binding_matches,
};
pub(super) use custody::{
    load_or_create_market_staging, market_signer_seeds, observe_market_staging_amount,
    observe_writer_retirement_custody_amount,
};
pub(super) use execution::process_execute_writer_auction_fill;
pub(super) use finalization::{
    process_cancel_or_refund_writer_bid, process_finalize_or_abort_writer_auction,
};
pub(super) use planning::process_plan_writer_auction_chunk;
pub(super) use reveal::process_reveal_writer_auction;
use rules::{
    checked_fee, checked_premium, is_current_active_writer_auction, writer_auction_abortable,
    writer_auction_bid_window_open, writer_auction_execute_deadline_open,
    writer_auction_planning_window_open, writer_auction_policy_inputs_match,
    writer_auction_reveal_window_open,
};
