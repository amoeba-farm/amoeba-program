use borsh::{BorshDeserialize, BorshSerialize};
use light_compressible::rent::RentConfig;
use light_sdk_types::interface::account::compression_info::{CompressionInfo, CompressionState};
use solana_program::pubkey::Pubkey;

#[cfg(not(target_endian = "little"))]
compile_error!("fixed state codecs require the little-endian Solana account ABI");

use crate::{
    ameba_dlmm_state::AmoebaDlmmPoolStatus,
    constants::MAX_SETTLEMENT_SIGNER_COUNT,
    error::VaultError,
    instruction::ManageWriterPolicyAuthorityActionV1,
    state::{
        InstrumentDefinition, Market, MarketMintAccounting, MarketParameters, OptionKind,
        OracleActiveWeightManifest, OracleBucketMedianState, OracleBucketMedianStatus,
        OracleBucketSourceIndex, OracleChallengeStatus, OracleClaimStatus, OracleEconomicParams,
        OracleEconomicsConfig, OracleEmergencyDisputeKind, OracleEmergencyDisputeV3,
        OracleEmergencyVoteRecordV3, OracleEmergencyVoteStatus, OracleEscrowDisposition,
        OracleMajorTokenConfig, OracleMaturityLadderRegistry, OracleMonthState, OracleOpeningClaim,
        OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OraclePhase, OraclePlayerLedger,
        OracleProductSkuDraft, OracleProductSkuManifest, OracleRecipeSourceIndex,
        OracleRecipeWeightManifest, OracleRecipeWeightPhase, OracleRewardFunnel,
        OracleSambaEmergencyPayoutMode, OracleSambaEmergencyPot, OracleSambaVoteSettlementReceipt,
        OracleSambaWinningVote, OracleSettlementSourceManifest, OracleSettlementStatus,
        OracleSkuCoverageManifest, OracleSkuCoverageRecord, OracleSourceChallenge,
        OracleSourceChallengeGuard, OracleSourceObservations, OracleSourceState,
        OracleSourceStatus, OracleStakeActivation, OracleStakingPool, OracleSupportPosition,
        OracleTreasuryState, OracleUnstakeRequest, OracleUpdateChallenge,
        OracleUpdateChallengeGuard, OracleUpdateClaimData, OracleUpdateClaimV2,
        OracleUsdcRewardKind, OracleUsdcRewardReceipt, OracleUsdcRewardRegistration,
        OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase, OracleUsdcRewardVault,
        OracleUsdcSkuPool, OracleUsdcSourceReward, PositionRecord, SettlementObservation,
        SettlementRecordV2, SettlementSignerRegistry, SettlementSignerSet, SettlementStyle,
        UserCollateral, VaultConfig, WriterAuctionPriorityRule, WriterAuctionStatus,
        WriterAuctionV1, WriterBidDeliveryMode, WriterBidIndexRecordV1, WriterBidIndexV1,
        WriterBidStatus, WriterBidV1, WriterCloseRequestStatus, WriterCloseRequestV1,
        WriterPolicyRegistryV1, WriterPolicySnapshotV1, WriterReserveRoundingMode,
        WriterSecurityMode, WriterSeriesBookV1, WriterSeriesCustodyStatus, WriterSeriesRecordV1,
        WriterSeriesSettlementStatus, WriterSettlementGroupStatus, WriterSettlementGroupV1,
        WriterSleeveStatus, WriterSleeveV1, ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT,
        WRITER_BID_STORAGE_CAPACITY, WRITER_SERIES_STORAGE_CAPACITY,
    },
};

mod accounts;
mod cursor;
mod fields;
mod oracle;
mod state_codecs;
mod writer_dlmm;
mod state_macros;
mod variable;

pub(crate) use cursor::*;
pub(crate) use fields::*;
pub(crate) use state_macros::*;
pub(crate) use variable::*;
