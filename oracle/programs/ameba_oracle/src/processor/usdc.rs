//! Append-only USDC custody, bond, and finite-reward processors for oracle months.
//!
//! Ordinary oracle participation uses the canonical collateral mint and `UserCollateral` in the
//! bond vault. The reward vault is a separate program-owned USDC account.

use super::*;
// The formation modules read these too (`v1-formation`).
#[cfg_attr(not(feature = "v1-formation"), allow(unused_imports))]
use crate::{
    constants::{
        MAX_ORACLE_USDC_MERGE_DEPTH, ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
        ORACLE_USDC_REWARD_VAULT_PDA_SEED, ORACLE_USDC_SKU_POOL_PDA_SEED,
        ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
    },
    instruction::{
        AddOracleUsdcSkuBudgetParams, ChallengeOracleSourceParams,
        ChallengeOracleUpdateClaimParams, CommitOracleUpdateClaimV2Params,
        DepositOracleUsdcRewardsParams, FinalizeOracleUpdateClaimV2Params,
        ProposeOracleSourceParams, ProposeOracleSourceV3Params, RevealOracleUpdateClaimV3Params,
        SupportOracleSourceParams, SupportOracleSourceV3Params,
    },
    state::{
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_reward_vault_pda,
        derive_oracle_usdc_sku_pool_pda, derive_oracle_usdc_source_reward_pda,
        OracleUpdateClaimOutcome, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
        OracleUsdcRewardVault, OracleUsdcSkuPool, OracleUsdcSourceReward,
    },
};

mod accounting;
#[cfg(feature = "v1-formation")]
mod opening;
#[cfg(feature = "v1-formation")]
mod schedule;
mod sources;
mod update_claims;
mod update_settlement;

pub(super) use accounting::*;
#[cfg(feature = "v1-formation")]
pub(super) use opening::*;
#[cfg(feature = "v1-formation")]
pub(super) use schedule::*;
pub(super) use sources::*;
pub(super) use update_claims::*;
pub(super) use update_settlement::*;

#[cfg(test)]
mod tests;
