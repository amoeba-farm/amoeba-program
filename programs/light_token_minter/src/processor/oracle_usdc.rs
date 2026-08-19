//! Append-only USDC custody, bond, and finite-reward processors for oracle months.
//!
//! Ordinary oracle participation uses the canonical collateral mint and
//! `UserCollateral`; it never mutates the AMBA player ledger or AMBA treasury.
//! Emergency voting remains in the separately verified sAMBA lane.

use super::*;
use crate::{
    constants::{
        MAX_ORACLE_USDC_MERGE_DEPTH, ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
        ORACLE_USDC_REWARD_VAULT_PDA_SEED, ORACLE_USDC_SKU_POOL_PDA_SEED,
        ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
    },
    instruction::{
        AddOracleUsdcSkuBudgetParams, ChallengeOracleOpeningClaimParams,
        ChallengeOracleSourceParams, ChallengeOracleUpdateClaimParams,
        CommitOracleUpdateClaimV2Params, DepositOracleUsdcRewardsParams,
        FinalizeOracleUpdateClaimV2Params, ProposeOracleSourceParams,
        ResolveOracleOpeningClaimChallengeParams, RevealOracleUpdateClaimV3Params,
        SubmitOracleOpeningClaimParams, SupportOracleSourceParams,
    },
    state::{
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_reward_vault_pda,
        derive_oracle_usdc_sku_pool_pda, derive_oracle_usdc_source_reward_pda,
        OracleUpdateClaimOutcome, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
        OracleUsdcRewardVault, OracleUsdcSkuPool, OracleUsdcSourceReward,
    },
};

mod accounting;
mod opening;
mod schedule;
mod sources;
mod update_claims;
mod update_settlement;

pub(super) use accounting::*;
pub(super) use opening::*;
pub(super) use schedule::*;
pub(super) use sources::*;
pub(super) use update_claims::*;
pub(super) use update_settlement::*;
