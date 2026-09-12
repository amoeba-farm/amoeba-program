use super::*;
use crate::{
    constants::{
        MAX_AMOEBA_DLMM_BINS_PER_SWAP, MAX_AMOEBA_DLMM_BIN_COUNT, MAX_AMOEBA_DLMM_SWAP_FEE_BPS,
    },
    instruction::{
        BeginWriterCloseV1Params, ClaimCollectiveLongV1Params, ClaimWriterFlatResidualV1Params,
        CleanupWriterCustodyV1Params, FinalizeOrAbortWriterAuctionV1Params,
        InitializeWriterPolicyRegistryV1Params, ManageWriterPolicyAuthorityActionV1,
        ManageWriterPolicyAuthorityV1Params, PlaceWriterBidV1Params,
        PlanWriterAuctionChunkV1Params, PrepareWriterBidIndexV1Params,
        ProcessWriterCloseCancellationV1Params, ReconcileWriterSupplyV1Params,
        RevealWriterAuctionV1Params, SealWriterPolicyV1Params, SetCollectiveMarketPausedV1Params,
        WriterAmountV1Params, WriterSeriesIndexV1Params,
    },
    state::{
        WriterAuctionPriorityRule, WriterAuctionStatus, WriterAuctionV1, WriterBidIndexRecordV1,
        WriterBidIndexV1, WriterBidStatus, WriterBidV1, WriterCloseRequestStatus,
        WriterCloseRequestV1, WriterPolicyRegistryV1, WriterPolicySnapshotV1,
        WriterReserveRoundingMode, WriterSecurityMode, WriterSeriesBookV1,
        WriterSeriesCustodyStatus, WriterSeriesRecordV1, WriterSeriesSettlementStatus,
        WriterSettlementGroupStatus, WriterSettlementGroupV1, WriterSleeveStatus, WriterSleeveV1,
    },
    writer_sleeve_math::{
        drawdown_checks, exact_reserve, maximum_safe_issue_quantity, proportional_close_preview,
        security_exposure as calculate_security_exposure, WriterIssueAdmissionLimits,
        WriterMathError, WriterSecurityMode as WriterMathSecurityMode, WriterSeries,
    },
};

mod accounts;
mod auction;
mod bid_index_preparation;
mod close;
pub(super) mod dlmm;
mod funding;
mod reconcile;
mod settlement;

use accounts::*;

const WRITER_SERIES_FAMILY_HASH_DOMAIN: &[u8] = b"ameba-writer-series-family-v1";
const WRITER_BOOK_HASH_DOMAIN: &[u8] = b"ameba-writer-book-v1";
const WRITER_PAYOFF_HASH_DOMAIN: &[u8] = b"ameba-writer-payoff-v1";
const WRITER_RISK_HASH_DOMAIN: &[u8] = b"ameba-writer-risk-v1";
const WRITER_POLICY_HASH_DOMAIN: &[u8] = b"ameba-writer-policy-v1";
const WRITER_COVERAGE_HASH_DOMAIN: &[u8] = b"ameba-writer-coverage-v1";
const WRITER_SERIES_FAMILY_HASH_MAX_BYTES: usize =
    WRITER_SERIES_FAMILY_HASH_DOMAIN.len() + 4 + crate::constants::WRITER_MAX_LIVE_SERIES * 64;
const WRITER_BOOK_HASH_MAX_BYTES: usize = WRITER_BOOK_HASH_DOMAIN.len()
    + 32
    + 32
    + 4
    + crate::constants::WRITER_MAX_LIVE_SERIES * (32 + 32 + 7 * 8);
const WRITER_MAX_PACK_ACCOUNTS: usize = 14 + 3 * crate::constants::WRITER_MAX_LIVE_SERIES;

pub(super) struct CollectiveDlmmContext {
    pub sleeve_status: WriterSleeveStatus,
    pub group_status: WriterSettlementGroupStatus,
    pub active_weight_manifest_hash: [u8; 32],
    pub anchor_month_settled: bool,
    pub option_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub expiry_ts: u64,
    pub tick_size_quote_atomic: u64,
    pub maximum_price_quote_atomic: u64,
    pub maximum_bin_id: u16,
    pub swap_fee_bps: u16,
    pub maximum_bins_per_swap: u8,
}

struct CollectiveGroupBinding {
    sleeve_status: WriterSleeveStatus,
    group_status: WriterSettlementGroupStatus,
    active_weight_manifest_hash: [u8; 32],
    underlying_id: [u8; 32],
    expiry_ts: u64,
    settlement_mint: Pubkey,
    anchor_market: Pubkey,
    anchor_oracle_month: Pubkey,
    series_id: [u8; 32],
    contract_mint: Pubkey,
}

struct CollectiveMarketBinding {
    option_mint: Pubkey,
    quote_mint: Pubkey,
    expiry_ts: u64,
    tick_size_quote_atomic: u64,
    maximum_price_quote_atomic: u64,
    maximum_bin_id: u16,
    swap_fee_bps: u16,
    maximum_bins_per_swap: u8,
}

mod collective_binding;
mod commitments;
mod custody;
mod dispatch;
mod initialization;
mod lifecycle;
mod metrics;
mod policy;
mod privileges;

pub(super) use collective_binding::{
    load_collective_dlmm_context, load_collective_settlement_group_for_dlmm,
};
pub(super) use commitments::{
    writer_book_digest, writer_close_book_digest, writer_group_commitment,
    writer_series_family_hash,
};
use commitments::{
    writer_coverage_manifest_hash, writer_payoff_digest, writer_policy_hash, writer_risk_limit_hash,
};
use custody::create_classic_token_pda;
pub(super) use custody::load_or_create_writer_retirement_custody;
pub(super) use dispatch::process_instruction;
pub(super) use initialization::{
    process_initialize_settlement_group, process_initialize_sleeve, process_register_series,
};
pub(super) use lifecycle::{
    process_activate_sleeve, process_open_funding, process_set_collective_market_paused,
};
pub(super) use metrics::{recompute_writer_metrics, writer_book_math_series};
use metrics::{writer_activation_assets_are_sufficient, writer_math_error};
pub(super) use policy::{
    process_initialize_policy_registry, process_manage_policy_authority, process_seal_policy,
};
use privileges::validate_pack_writer_account_privileges;
