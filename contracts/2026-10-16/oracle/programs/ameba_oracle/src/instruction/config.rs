use super::*;

/// Tag 1 payload: one Forming period for one product and fixed expiry.
#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct CreatePeriodParams {
    pub product_id: [u8; 32],
    pub planned_listing_ts: u64,
    pub expiry_ts: u64,
    pub value_decimals: u8,
}

crate::fixed_codec::fixed_instruction_deserialize!(CreatePeriodParams, 49, {
    product_id: [u8; 32],
    planned_listing_ts: u64,
    expiry_ts: u64,
    value_decimals: u8,
});

#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct RotateVaultAuthoritiesV2Params {
    pub new_admin: Pubkey,
    pub new_oracle_authority: Pubkey,
}

crate::fixed_codec::fixed_instruction_deserialize!(RotateVaultAuthoritiesV2Params, 64, {
    new_admin: Pubkey,
    new_oracle_authority: Pubkey,
});

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct BootstrapVaultGovernanceV2Params {
    pub new_oracle_authority: Pubkey,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct ActivateVaultV2Params {
    pub expected_collateral_freeze_authority: Option<Pubkey>,
}
