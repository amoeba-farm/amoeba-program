//! Terminal ordinary-USDC escrow settlement and finite cash reward registration/claims.
//!
//! Current cash rewards and participant bonds use canonical collateral accounts only.
//! Emergency voting power remains sAMBA; only an eligible winning vote's reward is paid
//! from the separately deposited classic-SPL collateral-mint custody.

use super::oracle_usdc::{
    calculate_oracle_usdc_equal_share, calculate_oracle_usdc_source_reward_allocation,
    calculate_oracle_usdc_supporter_reward, load_oracle_usdc_reward_schedule,
    load_oracle_usdc_reward_vault, load_oracle_usdc_sku_pool, load_oracle_usdc_source_reward,
};
use super::*;
use crate::{
    constants::{
        MAX_ORACLE_USDC_MERGE_DEPTH, ORACLE_USDC_REWARD_RECEIPT_PDA_SEED,
        ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED, ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED,
        ORACLE_USDC_REWARD_VAULT_PDA_SEED,
    },
    instruction::{ClaimOracleUsdcRewardParams, SettleOracleEscrowParams},
    state::{
        derive_oracle_usdc_reward_receipt_pda, derive_oracle_usdc_reward_registration_pda,
        OracleSourceState, OracleUsdcRewardKind, OracleUsdcRewardReceipt,
        OracleUsdcRewardRegistration, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
        OracleUsdcRewardVault, OracleUsdcSkuPool, OracleUsdcSourceReward,
    },
};

mod claims;
mod escrow;
mod registration;

pub(super) use claims::*;
pub(super) use escrow::*;
pub(super) use registration::*;
