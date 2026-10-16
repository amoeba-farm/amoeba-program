use crate::ProgramError;
use solana_program::program_error::ProgramError as SolanaProgramError;

/// Custom program errors. Codes 6000 through 6299 keep the numbering already decoded by
/// existing clients; retired numbers are never reused. Codes from 6300 belong to errors that
/// only this program defines.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum OracleError {
    #[error("invalid instruction data")]
    InvalidInstructionData = 6000,
    #[error("invalid account list")]
    InvalidAccountList = 6001,
    #[error("invalid token program")]
    InvalidTokenProgram = 6002,
    #[error("invalid system program")]
    InvalidSystemProgram = 6003,
    #[error("invalid PDA")]
    InvalidPda = 6004,
    #[error("account already initialized")]
    AlreadyInitialized = 6005,
    #[error("unauthorized")]
    Unauthorized = 6007,
    #[error("amount must be greater than zero")]
    AmountMustBePositive = 6008,
    #[error("contract is paused")]
    ContractPaused = 6009,
    #[error("invalid mint account")]
    InvalidMint = 6010,
    #[error("invalid token account")]
    InvalidTokenAccount = 6011,
    #[error("invalid config account")]
    InvalidConfigAccount = 6012,
    #[error("account key mismatch")]
    AccountMismatch = 6013,
    #[error("invalid oracle period account")]
    InvalidPeriodAccount = 6015,
    #[error("invalid user collateral account")]
    InvalidUserCollateralAccount = 6017,
    #[error("insufficient available collateral")]
    InsufficientAvailableCollateral = 6022,
    #[error("arithmetic overflow")]
    ArithmeticOverflow = 6039,
    #[error("invalid compression witness")]
    InvalidCompressionWitness = 6043,
    #[error("invalid settlement record")]
    InvalidSettlementRecord = 6053,
    #[error("settlement record hash mismatch")]
    SettlementRecordHashMismatch = 6054,
    #[error("invalid settlement account")]
    InvalidSettlementAccount = 6056,
    #[error("period has not reached settlement time")]
    PeriodNotExpired = 6058,
    #[error("invalid instructions sysvar account")]
    InvalidInstructionsSysvar = 6083,
    #[error("missing required oracle attestations")]
    MissingSettlementOracleSignatures = 6084,
    #[error("invalid oracle attestation instruction")]
    InvalidSettlementOracleSignature = 6085,
    #[error("oracle signer is not on the approved allowlist")]
    InvalidSettlementOracleSigner = 6086,
    #[error("duplicate oracle signer attestation")]
    DuplicateSettlementOracleSigner = 6087,
    #[error("oracle attestation payload does not match settlement data")]
    SettlementOraclePayloadMismatch = 6088,
    #[error("invalid oracle month account")]
    InvalidOracleMonthAccount = 6099,
    #[error("invalid oracle source account")]
    InvalidOracleSourceAccount = 6100,
    #[error("oracle instruction is invalid for the current phase")]
    InvalidOraclePhase = 6103,
    #[error("oracle timing window is not open")]
    OracleTimingWindowClosed = 6104,
    #[error("invalid oracle state")]
    InvalidOracleState = 6105,
    #[error("oracle opening already submitted")]
    OracleOpeningAlreadySubmitted = 6106,
    #[error("invalid oracle challenge account")]
    InvalidOracleChallengeAccount = 6107,
    #[error("invalid oracle update account")]
    InvalidOracleUpdateAccount = 6108,
    #[error("invalid oracle emergency dispute account")]
    InvalidOracleEmergencyDispute = 6109,
    #[error("an oracle opening claim is already pending for this source")]
    OracleOpeningClaimPending = 6115,
    #[error("oracle opening claim is not ready for finalization")]
    OracleOpeningClaimNotFinalizable = 6116,
    #[error("oracle opening claim evidence is invalid")]
    InvalidOracleOpeningEvidence = 6117,
    #[error("invalid oracle opening claim account")]
    InvalidOracleOpeningClaim = 6118,
    #[error("oracle escrow has already reached a terminal disposition")]
    OracleEscrowAlreadySettled = 6119,
    #[error("oracle escrow is not eligible for settlement")]
    OracleEscrowNotSettleable = 6120,
    #[error("invalid oracle recipe weight manifest")]
    InvalidOracleWeightManifest = 6122,
    #[error("oracle recipe weight manifest has already finalized")]
    OracleWeightManifestFinalized = 6123,
    #[error("oracle recipe weight input is not in canonical order")]
    InvalidOracleWeightOrder = 6124,
    #[error("oracle recipe weight scheme is not canonically verified")]
    OracleWeightSchemeUnverified = 6125,
    #[error("oracle recipe weight manifest hash does not match the approved hash")]
    OracleWeightManifestHashMismatch = 6126,
    #[error("oracle recipe weight manifest is incomplete")]
    OracleWeightManifestIncomplete = 6127,
    #[error("invalid source supplied to the oracle recipe weight manifest")]
    InvalidOracleWeightSource = 6128,
    #[error("settlement record is immutable and conflicts with the submitted settlement")]
    SettlementRecordImmutable = 6129,
    #[error("collateral mint and vault identity are immutable after initialization")]
    CollateralIdentityImmutable = 6140,
    #[error("invalid settlement signer registry")]
    InvalidSettlementSignerRegistry = 6141,
    #[error("invalid settlement signer set")]
    InvalidSettlementSignerSet = 6142,
    #[error("invalid governed settlement signer configuration")]
    InvalidSettlementSignerConfiguration = 6143,
    #[error("a settlement signer rotation is already pending")]
    SettlementSignerRotationPending = 6144,
    #[error("settlement signer rotation timelock has not elapsed")]
    SettlementSignerRotationNotReady = 6145,
    #[error("settlement signer rotation lacks required governance authorization")]
    SettlementSignerGovernanceRequired = 6146,
    #[error("emergency settlement signer recovery requires a global pause")]
    SettlementSignerRecoveryRequiresPause = 6147,
    #[error("settlement signer-set version does not match the active registry")]
    SettlementSignerSetVersionMismatch = 6149,
    #[error("settlement signer rotation delay is below the protocol safety floor")]
    SettlementSignerTimelockTooShort = 6150,
    #[error("invalid oracle active-weight manifest")]
    InvalidOracleActiveWeightManifest = 6155,
    #[error("oracle active weights are not finalized")]
    OracleActiveWeightSchemeUnverified = 6157,
    #[error("oracle update commitment reveal timing is invalid")]
    InvalidOracleUpdateRevealTiming = 6159,
    #[error("oracle update commitment does not match the reveal")]
    OracleUpdateCommitmentMismatch = 6160,
    #[error("settlement grace period has not elapsed")]
    SettlementGracePeriodActive = 6161,
    #[error("invalid oracle USDC reward vault")]
    InvalidOracleUsdcRewardVault = 6181,
    #[error("invalid oracle USDC reward schedule")]
    InvalidOracleUsdcRewardSchedule = 6182,
    #[error("invalid oracle USDC SKU pool")]
    InvalidOracleUsdcSkuPool = 6183,
    #[error("invalid oracle USDC source reward account")]
    InvalidOracleUsdcSourceReward = 6184,
    #[error("invalid oracle USDC reward registration")]
    InvalidOracleUsdcRewardRegistration = 6185,
    #[error("invalid oracle USDC reward receipt")]
    InvalidOracleUsdcRewardReceipt = 6186,
    #[error("oracle USDC reward schedule is not fully funded")]
    OracleUsdcRewardScheduleUnderfunded = 6188,
    #[error("oracle USDC reward schedule is already frozen")]
    OracleUsdcRewardScheduleFrozen = 6189,
    #[error("oracle USDC reward entitlements are not finalized")]
    OracleUsdcRewardEntitlementsNotFinalized = 6190,
    #[error("oracle USDC bond does not equal the frozen SKU requirement")]
    InvalidOracleUsdcBond = 6193,
    #[error("oracle USDC reward registration is incomplete")]
    OracleUsdcRewardRegistrationIncomplete = 6194,
    #[error("oracle USDC reward vault has insufficient unreserved custody")]
    InsufficientOracleUsdcRewardCustody = 6195,
    #[error("oracle month does not satisfy the rolling three-month maturity ladder")]
    InvalidOracleMaturityLadder = 6196,
    #[error("invalid canonical oracle terminal-SKU coverage manifest")]
    InvalidOracleSkuCoverageManifest = 6197,
    #[error("invalid canonical oracle terminal-SKU coverage record")]
    InvalidOracleSkuCoverageRecord = 6198,
    #[error("terminal-SKU membership proof does not match the frozen month manifest")]
    InvalidOracleSkuMembershipProof = 6199,
    #[error("terminal-SKU source coverage is incomplete")]
    OracleSkuCoverageIncomplete = 6200,
    #[error("terminal-SKU source coverage has already finalized")]
    OracleSkuCoverageAlreadyFinalized = 6201,
    #[error("there is not enough time before expiry to preserve the full review calendar")]
    OracleSkuCoverageExtensionTooLate = 6202,
    #[error("this oracle lifecycle version requires its current coverage-aware instruction")]
    OracleSkuCoverageInstructionRequired = 6203,
    #[error("invalid canonical oracle product-SKU draft")]
    InvalidOracleProductSkuDraft = 6204,
    #[error("this instruction must use the current compressed-state transport")]
    CompressedStateTransportRequired = 6205,
    #[error("invalid capital-independent oracle median state or recomputation")]
    InvalidOracleMedian = 6231,
    #[error("invalid or exhausted archive-backed oracle observation history")]
    InvalidOracleObservation = 6232,
    #[error("oracle bucket security budget is invalid")]
    InvalidOracleSecurityBudget = 6233,
    #[error("invalid or already finalized trading-fee oracle bounty sweep")]
    InvalidOracleBountySweep = 6235,
    #[error("missing signed governance instruction tail")]
    MissingGovernanceTail = 6255,
    #[error("invalid signed governance instruction tail")]
    InvalidGovernanceTail = 6256,
    #[error("missing canonical governance gate account")]
    MissingGovernanceGate = 6257,
    #[error("duplicate canonical governance gate account")]
    DuplicateGovernanceGate = 6258,
    #[error("invalid governance gate account privileges")]
    InvalidGovernanceGatePrivileges = 6259,
    #[error("invalid governance gate account owner")]
    InvalidGovernanceGateOwner = 6260,
    #[error("invalid canonical governance gate PDA")]
    InvalidGovernanceGatePda = 6261,
    #[error("invalid governance gate account data")]
    InvalidGovernanceGateData = 6262,
    #[error("governance gate is frozen")]
    GovernanceGateFrozen = 6263,
    #[error("signed governance epoch does not match the gate")]
    GovernanceGateEpochMismatch = 6264,
    #[error("reviewed governance bridge controller identity is unavailable")]
    GovernanceBridgeIdentityUnavailable = 6265,
    #[error("no bounty is claimable for this authenticated entitlement")]
    NoOracleBountyClaimable = 6266,
    #[error("instruction data exceeds the 16,384-byte cap")]
    InstructionTooLarge = 6300,
    #[error("the target address is foreign-owned or carries data")]
    AddressOccupied = 6301,
    #[error("the signer is not the recorded program upgrade authority")]
    InvalidUpgradeAuthority = 6302,
    #[error("invalid period identity, schedule or value decimals")]
    InvalidPeriodSchedule = 6303,
    #[error("the period is not in the status this transition requires")]
    InvalidPeriodTransition = 6304,
    #[error("the period can still list; no failure condition holds yet")]
    PeriodNotFailable = 6305,
    #[error("the CFM launch plan or registry does not match its approved commitment")]
    InvalidCfmLaunchPlan = 6306,
    #[error("invalid CFM launch receipt, or the launch is not at this step")]
    InvalidCfmLaunchReceipt = 6307,
    #[error("three distinct current council seats must sign the CFM launch")]
    CfmLaunchCouncilRequired = 6308,
}

impl From<OracleError> for ProgramError {
    #[inline(always)]
    fn from(value: OracleError) -> Self {
        ProgramError::Custom(value as u32)
    }
}

impl From<OracleError> for SolanaProgramError {
    fn from(value: OracleError) -> Self {
        SolanaProgramError::Custom(value as u32)
    }
}
