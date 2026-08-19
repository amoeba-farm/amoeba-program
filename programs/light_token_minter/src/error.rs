use solana_program::program_error::ProgramError;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VaultError {
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
    #[error("vault config already initialized")]
    AlreadyInitialized = 6005,
    #[error("vault config is not initialized")]
    NotInitialized = 6006,
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
    #[error("invalid light token program")]
    InvalidLightTokenProgram = 6014,
    #[error("invalid market account")]
    InvalidMarketAccount = 6015,
    #[error("invalid user collateral account")]
    InvalidUserCollateralAccount = 6017,
    #[error("invalid position account")]
    InvalidPositionAccount = 6018,
    #[error("invalid market configuration")]
    InvalidMarketConfig = 6019,
    #[error("insufficient available collateral")]
    InsufficientAvailableCollateral = 6022,
    #[error("arithmetic overflow")]
    ArithmeticOverflow = 6039,
    #[error("invalid remaining accounts")]
    InvalidRemainingAccounts = 6040,
    #[error("invalid contract mint")]
    InvalidContractMint = 6041,
    #[error("mint allowance exceeded")]
    MintAllowanceExceeded = 6042,
    #[error("invalid compression witness")]
    InvalidCompressionWitness = 6043,
    #[error("market is paused")]
    MarketPaused = 6048,
    #[error("invalid market authority")]
    InvalidMarketAuthority = 6049,
    #[error("invalid market page")]
    InvalidMarketPage = 6050,
    #[error("market page hash mismatch")]
    MarketPageHashMismatch = 6051,
    #[error("market page id mismatch")]
    MarketPageIdMismatch = 6052,
    #[error("invalid settlement record")]
    InvalidSettlementRecord = 6053,
    #[error("settlement record hash mismatch")]
    SettlementRecordHashMismatch = 6054,
    #[error("invalid settlement account")]
    InvalidSettlementAccount = 6056,
    #[error("market has expired")]
    MarketExpired = 6057,
    #[error("market has not reached settlement time")]
    MarketNotExpired = 6058,
    #[error("position settlement already claimed")]
    SettlementAlreadyClaimed = 6059,
    #[error("settlement liquidity mismatch")]
    SettlementLiquidityMismatch = 6060,
    #[error("invalid SPL interface account")]
    InvalidSplInterfaceAccount = 6066,
    #[error("invalid compressed token authority")]
    InvalidCompressedTokenAuthority = 6067,
    #[error("invalid light token account")]
    InvalidLightTokenAccount = 6068,
    #[error("missing instructions sysvar account")]
    MissingInstructionsSysvar = 6082,
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
    #[error("insufficient short position")]
    InsufficientShortPosition = 6091,
    #[error("invalid oracle month account")]
    InvalidOracleMonthAccount = 6099,
    #[error("invalid oracle source account")]
    InvalidOracleSourceAccount = 6100,
    #[error("invalid oracle player ledger")]
    InvalidOraclePlayerLedger = 6101,
    #[error("invalid oracle treasury account")]
    InvalidOracleTreasury = 6102,
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
    #[error("invalid oracle major token config")]
    InvalidOracleMajorTokenConfig = 6110,
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
    #[error("invalid market mint accounting")]
    InvalidMarketMintAccounting = 6132,
    #[error("market mint supply does not reconcile to issued supply")]
    MarketMintSupplyMismatch = 6133,
    #[error("requested contract amount exceeds backed outstanding supply")]
    InsufficientBackedContractSupply = 6134,
    #[error("market contract mint violates canonical authority or mint policy")]
    InvalidMarketMintPolicy = 6135,
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
    #[error("invalid oracle staking pool")]
    InvalidOracleStakingPool = 6162,
    #[error("invalid sAMBA unstake request")]
    InvalidOracleUnstakeRequest = 6163,
    #[error("insufficient sAMBA")]
    InsufficientSambaTokens = 6164,
    #[error("invalid AMBA to sAMBA exchange rate")]
    InvalidSambaExchangeRate = 6165,
    #[error("the seven-day sAMBA unbonding period is still active")]
    UnstakeCooldownActive = 6166,
    #[error("invalid canonical sAMBA mint or vote vault")]
    InvalidSambaMint = 6168,
    #[error("sAMBA supply changes are locked while an emergency checkpoint is unresolved")]
    SambaSupplyLockedForVoting = 6169,
    #[error("invalid canonical oracle reward funnel or AMBA intake account")]
    InvalidOracleRewardFunnel = 6170,
    #[error("oracle reward funnel has no AMBA to sweep")]
    OracleRewardFunnelEmpty = 6171,
    #[error("invalid canonical queued sAMBA activation")]
    InvalidOracleStakeActivation = 6172,
    #[error("the queued sAMBA activation delay is still active")]
    StakeActivationCooldownActive = 6173,
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
    #[error("oracle work instruction uses the wrong reward currency generation")]
    InvalidOracleWorkRewardCurrency = 6187,
    #[error("oracle USDC reward schedule is not fully funded")]
    OracleUsdcRewardScheduleUnderfunded = 6188,
    #[error("oracle USDC reward schedule is already frozen")]
    OracleUsdcRewardScheduleFrozen = 6189,
    #[error("oracle USDC reward entitlements are not finalized")]
    OracleUsdcRewardEntitlementsNotFinalized = 6190,
    #[error("oracle USDC reward entitlements have already finalized")]
    OracleUsdcRewardEntitlementsAlreadyFinalized = 6191,
    #[error("oracle USDC reward has already been claimed")]
    OracleUsdcRewardAlreadyClaimed = 6192,
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
    #[error("invalid canonical Amoeba DLMM pool")]
    InvalidAmoebaDlmmPool = 6206,
    #[error("invalid canonical Amoeba DLMM reserve page")]
    InvalidAmoebaDlmmBinPage = 6207,
    #[error("invalid canonical Amoeba DLMM share page")]
    InvalidAmoebaDlmmSharePage = 6208,
    #[error("invalid canonical Amoeba DLMM position")]
    InvalidAmoebaDlmmPosition = 6209,
    #[error("invalid Amoeba DLMM status transition")]
    InvalidAmoebaDlmmStatusTransition = 6210,
    #[error("invalid Amoeba DLMM price grid")]
    InvalidAmoebaDlmmGrid = 6211,
    #[error("Amoeba DLMM route crosses too many bins or pages")]
    AmoebaDlmmRouteTooLarge = 6212,
    #[error("invalid Amoeba DLMM liquidity state or share operation")]
    InvalidAmoebaDlmmLiquidity = 6213,
    #[error("insufficient Amoeba DLMM liquidity")]
    InsufficientAmoebaDlmmLiquidity = 6214,
    #[error("Amoeba DLMM swap minimum output was not met")]
    AmoebaDlmmSlippageExceeded = 6215,
    #[error("Amoeba DLMM price limit was exceeded")]
    AmoebaDlmmPriceLimitExceeded = 6216,
    #[error("Amoeba DLMM swap deadline has elapsed")]
    AmoebaDlmmDeadlineElapsed = 6217,
    #[error("Amoeba DLMM market has expired or settled")]
    AmoebaDlmmMarketNotTradable = 6218,
    #[error("Amoeba DLMM reserve or custody invariant failed")]
    AmoebaDlmmInvariantViolation = 6219,
    #[error("invalid canonical Amoeba DLMM token vault")]
    InvalidAmoebaDlmmVault = 6220,
    #[error("invalid or incomplete Amoeba DLMM page route")]
    InvalidAmoebaDlmmRoute = 6221,
    #[error("invalid Amoeba DLMM fee configuration")]
    InvalidAmoebaDlmmFees = 6222,
    #[error("invalid canonical Amoeba DLMM settlement")]
    InvalidAmoebaDlmmSettlement = 6223,
    #[error("Amoeba DLMM pool, page, position, or vault is not empty")]
    AmoebaDlmmNotEmpty = 6224,
    #[error("invalid Amoeba DLMM Light account lifecycle request")]
    InvalidAmoebaDlmmLightLifecycle = 6225,
    #[error("duplicate Amoeba DLMM page account")]
    DuplicateAmoebaDlmmPage = 6226,
    #[error("Amoeba DLMM pages are not in canonical traversal order")]
    UnorderedAmoebaDlmmPage = 6227,
    #[error("unexpected writable account in an Amoeba DLMM route")]
    UnexpectedAmoebaDlmmWritableAccount = 6228,
    #[error("only the configured Amoeba DLMM liquidity manager may perform this action")]
    UnauthorizedAmoebaDlmmManager = 6229,
    #[error("Amoeba DLMM account is cold and must be loaded before use")]
    AmoebaDlmmAccountNotHot = 6230,
    #[error("invalid capital-independent oracle median state or recomputation")]
    InvalidOracleMedian = 6231,
    #[error("invalid or exhausted archive-backed oracle observation history")]
    InvalidOracleObservation = 6232,
    #[error("oracle bucket security budget is invalid")]
    InvalidOracleSecurityBudget = 6233,
    #[error("writer issuance would exceed the oracle security-backed open-interest cap")]
    OracleOpenInterestCapExceeded = 6234,
    #[error("invalid or already finalized trading-fee oracle bounty sweep")]
    InvalidOracleBountySweep = 6235,
    #[error("invalid canonical writer policy registry")]
    InvalidWriterPolicyRegistry = 6237,
    #[error("invalid immutable writer policy snapshot")]
    InvalidWriterPolicySnapshot = 6238,
    #[error("invalid canonical writer settlement group")]
    InvalidWriterSettlementGroup = 6239,
    #[error("invalid canonical writer sleeve")]
    InvalidWriterSleeve = 6240,
    #[error("invalid canonical writer series book or record")]
    InvalidWriterSeriesBook = 6241,
    #[error("writer lifecycle transition is not permitted")]
    InvalidWriterLifecycle = 6242,
    #[error("writer series capacity has been exceeded")]
    WriterSeriesCapacityExceeded = 6243,
    #[error("writer series are not in canonical order")]
    InvalidWriterSeriesOrder = 6244,
    #[error("writer reserve or custody solvency invariant failed")]
    WriterSolvencyViolation = 6245,
    #[error("writer aggregate oracle-security exposure exceeds its group cap")]
    WriterSecurityCapExceeded = 6246,
    #[error("invalid canonical writer auction")]
    InvalidWriterAuction = 6247,
    #[error("invalid funded writer bid")]
    InvalidWriterBid = 6248,
    #[error("invalid staged writer close request")]
    InvalidWriterCloseRequest = 6249,
    #[error("writer auction or close deadline is invalid")]
    InvalidWriterDeadline = 6250,
    #[error("writer supply does not reconcile to canonical custody")]
    WriterSupplyMismatch = 6251,
    #[error("writer minimum proceeds or payout was not met")]
    WriterSlippageExceeded = 6252,
    #[error("a full pre-settlement Flat exit is not permitted")]
    WriterFullExitNotAllowed = 6253,
    #[error("writer arithmetic input is outside the admitted checked-u128 domain")]
    WriterArithmeticAdmissionFailed = 6254,
}

impl From<VaultError> for ProgramError {
    fn from(value: VaultError) -> Self {
        ProgramError::Custom(value as u32)
    }
}
