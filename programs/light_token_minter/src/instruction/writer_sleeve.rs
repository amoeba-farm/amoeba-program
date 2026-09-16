use super::*;
use crate::state::{WriterReserveRoundingMode, WriterSecurityMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
#[repr(u8)]
pub enum ScopedSettlementActionV1 {
    Authorize = 0,
    Execute = 1,
    Revoke = 2,
}

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

    pub v2_feature_flags: u8,

    pub drawdown_scale: u64,
    pub worst_drawdown_limit: u64,
    pub upper_drawdown_limit: u64,
    pub lower_drawdown_limit: u64,
    pub lower_tail_max_settlement_atomic: u64,
    pub upper_tail_min_settlement_atomic: u64,
    pub operational_buffer_atoms: u64,
    pub max_issue_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(SealWriterPolicyV1Params, 255, {
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

    v2_feature_flags: u8,

    drawdown_scale: u64,
    worst_drawdown_limit: u64,
    upper_drawdown_limit: u64,
    lower_drawdown_limit: u64,
    lower_tail_max_settlement_atomic: u64,
    upper_tail_min_settlement_atomic: u64,
    operational_buffer_atoms: u64,
    max_issue_atoms: u64,

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
pub struct ClaimCollectiveLongV1Params {
    pub claim_atoms: u64,
}

crate::fixed_codec::fixed_instruction_deserialize!(ClaimCollectiveLongV1Params, 8, {
    claim_atoms: u64,
});
