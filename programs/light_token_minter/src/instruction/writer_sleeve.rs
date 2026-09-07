use super::*;
use crate::state::{
    WriterAuctionPriorityRule, WriterBidDeliveryMode, WriterReserveRoundingMode, WriterSecurityMode,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct InitializeWriterPolicyRegistryV1Params {
    pub policy_authority: Pubkey,
    pub rotation_delay_slots: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(InitializeWriterPolicyRegistryV1Params, 40, {
    policy_authority: Pubkey,
    rotation_delay_slots: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
#[repr(u8)]
pub enum ManageWriterPolicyAuthorityActionV1 {
    Propose = 0,
    Activate = 1,
    Cancel = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ManageWriterPolicyAuthorityV1Params {
    pub action: ManageWriterPolicyAuthorityActionV1,
    pub new_authority: Pubkey,
}

crate::fixed_codec::fixed_instruction_deserialize!(ManageWriterPolicyAuthorityV1Params, 33, {
    action: ManageWriterPolicyAuthorityActionV1,
    new_authority: Pubkey,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct SealWriterPolicyV1Params {
    pub policy_version: u64,
    pub regime_input_version: u64,
    pub scenario_set_hash: [u8; 32],
    pub risk_limit_hash: [u8; 32],
    pub policy_hash: [u8; 32],
    pub model_margin_vector_hash: [u8; 32],
    pub execution_cost_vector_hash: [u8; 32],
    pub beta_ppm: u32,
    pub lambda_ppm: u64,
    pub security_mode: WriterSecurityMode,
    pub reserve_rounding_mode: WriterReserveRoundingMode,
    pub auction_priority_rule: WriterAuctionPriorityRule,
    pub v2_feature_flags: u8,
    pub primary_fee_bps: u16,
    pub drawdown_scale: u64,
    pub worst_drawdown_limit: u64,
    pub upper_drawdown_limit: u64,
    pub lower_drawdown_limit: u64,
    pub lower_tail_max_settlement_atomic: u64,
    pub upper_tail_min_settlement_atomic: u64,
    pub operational_buffer_atoms: u64,
    pub max_auction_issue_atoms: u64,
    pub max_close_flat_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(SealWriterPolicyV1Params, 266, {
    policy_version: u64,
    regime_input_version: u64,
    scenario_set_hash: [u8; 32],
    risk_limit_hash: [u8; 32],
    policy_hash: [u8; 32],
    model_margin_vector_hash: [u8; 32],
    execution_cost_vector_hash: [u8; 32],
    beta_ppm: u32,
    lambda_ppm: u64,
    security_mode: WriterSecurityMode,
    reserve_rounding_mode: WriterReserveRoundingMode,
    auction_priority_rule: WriterAuctionPriorityRule,
    v2_feature_flags: u8,
    primary_fee_bps: u16,
    drawdown_scale: u64,
    worst_drawdown_limit: u64,
    upper_drawdown_limit: u64,
    lower_drawdown_limit: u64,
    lower_tail_max_settlement_atomic: u64,
    upper_tail_min_settlement_atomic: u64,
    operational_buffer_atoms: u64,
    max_auction_issue_atoms: u64,
    max_close_flat_atoms: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct WriterAmountV1Params {
    pub amount_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(WriterAmountV1Params, 8, {
    amount_atoms: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct SetCollectiveMarketPausedV1Params {
    pub paused: bool,
}

crate::fixed_codec::fixed_instruction_deserialize!(SetCollectiveMarketPausedV1Params, 1, {
    paused: bool,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ReconcileWriterSupplyV1Params {
    pub series_index: u8,
    pub target_kind: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(ReconcileWriterSupplyV1Params, 2, {
    series_index: u8,
    target_kind: u8,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct CleanupWriterCustodyV1Params {
    pub series_index: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(CleanupWriterCustodyV1Params, 1, {
    series_index: u8,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct CommitWriterAuctionV1Params {
    pub auction_nonce: u64,
    /// Reveal-input precommitment; the program stores its actual-slot-bound digest.
    pub reserve_vector_commitment: [u8; 32],
    pub bid_deadline_ts: u64,
    pub reveal_deadline_ts: u64,
    pub execute_deadline_ts: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(CommitWriterAuctionV1Params, 64, {
    auction_nonce: u64,
    reserve_vector_commitment: [u8; 32],
    bid_deadline_ts: u64,
    reveal_deadline_ts: u64,
    execute_deadline_ts: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct PlaceWriterBidV1Params {
    pub order_id: u64,
    pub series_index: u8,
    pub delivery_mode: WriterBidDeliveryMode,
    pub bid_price_per_contract_atoms: u64,
    pub requested_contract_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(PlaceWriterBidV1Params, 26, {
    order_id: u64,
    series_index: u8,
    delivery_mode: WriterBidDeliveryMode,
    bid_price_per_contract_atoms: u64,
    requested_contract_atoms: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct RevealWriterAuctionV1Params {
    pub reserve_prices_atoms: [u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
    pub issue_caps_atoms: [u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
    pub nonce: [u8; 32],
}

crate::fixed_codec::fixed_instruction_deserialize!(RevealWriterAuctionV1Params, 544, {
    reserve_prices_atoms: [u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
    issue_caps_atoms: [u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
    nonce: [u8; 32],
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct PlanWriterAuctionChunkV1Params {
    pub max_records: u16,
}

crate::fixed_codec::fixed_instruction_deserialize!(PlanWriterAuctionChunkV1Params, 2, {
    max_records: u16,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct FinalizeOrAbortWriterAuctionV1Params {
    pub abort: bool,
}

crate::fixed_codec::fixed_instruction_deserialize!(FinalizeOrAbortWriterAuctionV1Params, 1, {
    abort: bool,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct BeginWriterCloseV1Params {
    pub flat_amount_atoms: u64,
    pub minimum_withdrawal_atoms: u64,
    pub deadline_ts: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(BeginWriterCloseV1Params, 24, {
    flat_amount_atoms: u64,
    minimum_withdrawal_atoms: u64,
    deadline_ts: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct WriterSeriesIndexV1Params {
    pub series_index: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(WriterSeriesIndexV1Params, 1, {
    series_index: u8,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ProcessWriterCloseCancellationV1Params {
    /// 0..series_count-1 selects one claim refund; 255 selects the final Flat refund.
    pub selector: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(ProcessWriterCloseCancellationV1Params, 1, {
    selector: u8,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ClaimCollectiveLongV1Params {
    pub claim_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(ClaimCollectiveLongV1Params, 8, {
    claim_atoms: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct ClaimWriterFlatResidualV1Params {
    pub flat_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(ClaimWriterFlatResidualV1Params, 8, {
    flat_atoms: u64,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub struct PrepareWriterBidIndexV1Params {
    pub auction_nonce: u64,
    pub phase: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(PrepareWriterBidIndexV1Params, 9, {
    auction_nonce: u64,
    phase: u8,
});
