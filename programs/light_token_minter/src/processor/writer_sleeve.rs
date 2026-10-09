use super::*;
use crate::{
    constants::{MAX_AMOEBA_DLMM_BINS_PER_SWAP, MAX_AMOEBA_DLMM_BIN_COUNT},
    instruction::{
        CleanupWriterCustodyV1Params, InitializeWriterPolicyRegistryV1Params,
        ManageWriterPolicyAuthorityActionV1, ManageWriterPolicyAuthorityV1Params,
        ReconcileWriterSupplyV1Params, SealWriterPolicyV1Params, SetCollectiveMarketPausedV1Params,
    },
    state::{
        WriterPolicyRegistryV1, WriterPolicySnapshotV1, WriterReserveRoundingMode,
        WriterSecurityMode, WriterSeriesBookV1, WriterSeriesCustodyStatus, WriterSeriesRecordV1,
        WriterSeriesSettlementStatus, WriterSettlementGroupStatus, WriterSettlementGroupV1,
        WriterSleeveStatus, WriterSleeveV1,
    },
    writer_sleeve_math::{
        drawdown_checks, exact_reserve, security_exposure as calculate_security_exposure,
        WriterMathError, WriterSecurityMode as WriterMathSecurityMode, WriterSeries,
    },
};

mod accounts;
mod capped_strip;
pub(super) use capped_strip::close_option_routes as process_atomic_position_close;
pub(super) use capped_strip::direct_wallet_trade as process_direct_wallet_strip_trade;
pub(super) use capped_strip::prepare_projection as prepare_atomic_projection;
pub(super) use capped_strip::trade as process_shared_strip_trade;
mod buyback_issuance;
mod classic_order_settlement;
mod compressed_settlement;
pub(in crate::processor) use classic_order_settlement::settle as settle_classic_order;
pub(super) use compressed_settlement::process_trading_session_close;
pub(super) mod dlmm;
mod individual;
mod individual_buyback;
mod participation;
mod portfolio_hedge;
mod reconcile;
mod settlement;
mod terminal_cleanup;
pub(super) use terminal_cleanup::require_paid_terminal_market;

use accounts::*;

const WRITER_SERIES_FAMILY_HASH_DOMAIN: &[u8] = b"ameba-writer-series-family-g3";
const WRITER_BOOK_HASH_DOMAIN: &[u8] = b"ameba-writer-book-g3";
const WRITER_PAYOFF_HASH_DOMAIN: &[u8] = b"ameba-writer-payoff-g3";
const WRITER_RISK_HASH_DOMAIN: &[u8] = b"ameba-writer-risk-g3";
const WRITER_POLICY_HASH_DOMAIN: &[u8] = b"ameba-writer-policy-g3";
const WRITER_COVERAGE_HASH_DOMAIN: &[u8] = b"ameba-writer-coverage-g3";
const WRITER_SERIES_FAMILY_HASH_MAX_BYTES: usize =
    WRITER_SERIES_FAMILY_HASH_DOMAIN.len() + 4 + crate::constants::WRITER_MAX_LIVE_SERIES * 64;
const WRITER_BOOK_HASH_MAX_BYTES: usize = WRITER_BOOK_HASH_DOMAIN.len()
    + 32
    + 32
    + 4
    + crate::constants::WRITER_MAX_LIVE_SERIES * (32 + 32 + 7 * 8 + 20)
    + 8;

pub(super) struct CollectiveDlmmContext {
    /// Canonically decoded once; callers may reuse these during this instruction.
    pub market: Box<Market>,
    pub anchor_month: Box<OracleMonthState>,
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
    market: Box<Market>,
    option_mint: Pubkey,
    quote_mint: Pubkey,
    expiry_ts: u64,
    tick_size_quote_atomic: u64,
    maximum_price_quote_atomic: u64,
    maximum_bin_id: u16,
    maximum_bins_per_swap: u8,
}

mod collective_binding;
mod commitments;
mod custody;
// Keep the existing accounting interface and its explicit inputs.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_market_staging<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    market: &Market,
    staging: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    token: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
) -> ProgramResult {
    custody::load_or_create_market_staging(
        program,
        payer,
        market_info,
        market,
        staging,
        mint,
        token,
        system,
    )?;
    Ok(())
}
mod dispatch;
pub(super) mod earn_fund;
mod initialization;
mod lifecycle;
mod metrics;
mod policy;
mod privileges;

pub(super) use collective_binding::{
    load_collective_dlmm_context, load_collective_dlmm_context_with_book,
    load_collective_settlement_group_for_dlmm,
};
pub(super) use commitments::{
    writer_book_digest, writer_group_commitment, writer_series_family_hash,
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
#[cfg(not(feature = "mainnet-v3"))]
pub(super) use policy::process_initialize_policy_registry;
pub(super) use policy::{process_manage_policy_authority, process_seal_policy};
use privileges::validate_pack_writer_account_privileges;

pub(in crate::processor) fn fill_multi_order_from_option_routes(
    program: &Pubkey,
    a: &[AccountInfo],
    wire: crate::atomic_option_route::Fill,
) -> ProgramResult {
    capped_strip::fill_option_routes(program, a, wire)
}
