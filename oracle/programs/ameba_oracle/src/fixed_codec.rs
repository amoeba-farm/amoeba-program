use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

#[cfg(not(target_endian = "little"))]
compile_error!("fixed state codecs require the little-endian Solana account ABI");

use crate::{
    constants::MAX_SETTLEMENT_SIGNER_COUNT,
    error::OracleError,
    state::{
        OracleActiveWeightManifest, OracleBucketMedianState, OracleBucketMedianStatus,
        OracleBucketSourceIndex, OracleChallengeStatus, OracleClaimStatus, OracleConfig,
        OracleEconomicParams, OracleEconomicsConfig, OracleEmergencyDisputeKind,
        OracleEscrowDisposition, OracleMaturityLadderRegistry, OracleMonthState,
        OracleOpeningClaim, OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OraclePhase,
        OracleProductSkuDraft, OracleProductSkuManifest, OracleRecipeSourceIndex,
        OracleRecipeWeightManifest, OracleRecipeWeightPhase, OracleSettlementSourceManifest,
        OracleSettlementStatus, OracleSkuCoverageManifest, OracleSkuCoverageRecord,
        OracleSourceChallenge, OracleSourceChallengeGuard, OracleSourceObservations,
        OracleSourceState, OracleSourceStatus, OracleSupportPosition, OracleUpdateChallenge,
        OracleUpdateChallengeGuard, OracleUpdateClaimData, OracleUpdateClaimV2,
        OracleUsdcRewardKind, OracleUsdcRewardReceipt, OracleUsdcRewardRegistration,
        OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase, OracleUsdcRewardVault,
        OracleUsdcSkuPool, OracleUsdcSourceReward, SettlementRecordV2, SettlementSignerRegistry,
        SettlementSignerSet, UserCollateral, ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT,
    },
};

mod accounts;
mod compact_borsh;
mod cursor;
mod fields;
mod flat;
mod oracle;
mod state_codecs;
mod state_macros;
mod variable;

#[allow(unused_imports)]
pub(crate) use compact_borsh::*;
pub(crate) use cursor::*;
pub(crate) use fields::*;
pub(crate) use flat::*;
#[allow(unused_imports)]
pub(crate) use state_macros::*;
pub(crate) use variable::*;

#[cfg(test)]
mod payload_equivalence_tests;
#[cfg(test)]
mod tests;
