use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum VaultInstruction {
    /// Initialize the single vault PDA config.
    ///
    /// Accounts:
    /// 0. [signer, writable] Admin/payer
    /// 1. [writable] Vault config PDA (also token authority PDA)
    /// 2. [] USDC mint account
    /// 3. [] Vault token account (owner must be PDA, mint must be USDC)
    /// 4. [] SPL token program
    /// 5. [] System program
    Initialize,

    /// Update admin, mint, vault token account, and pause status.
    ///
    /// Accounts:
    /// 0. [signer] Current admin
    /// 1. [writable] Vault config PDA
    /// 2. [] Mint account (current or new)
    /// 3. [] Vault token account (current or new)
    /// 4. [] SPL token program
    UpdateConfig {
        new_admin: Option<Pubkey>,
        new_oracle_authority: Option<Pubkey>,
        new_usdc_mint: Option<Pubkey>,
        new_vault_token_account: Option<Pubkey>,
        paused: Option<bool>,
    },

    /// Rotate the two vault governance authorities without letting either current authority act
    /// alone. The vault must be paused and both current and both incoming authorities sign.
    /// Accounts: 0 current admin; 1 current oracle; 2 incoming admin; 3 incoming oracle;
    /// 4 writable typed config; 5 signer registry; 6 active signer set.
    RotateVaultAuthoritiesV2 {
        params: RotateVaultAuthoritiesV2Params,
    },

    /// Split the fresh bootstrap config's duplicated authority before the governed signer registry
    /// exists. This is a one-time, paused-only transition; later changes use governed rotation.
    /// Accounts: 0 bootstrap admin signer; 1 incoming oracle signer; 2 writable typed config;
    /// 3 canonical, not-yet-created settlement signer registry PDA.
    BootstrapVaultGovernanceV2 {
        params: BootstrapVaultGovernanceV2Params,
    },

    /// Unpause only after typed, distinct governance and the canonical current settlement signer
    /// set are initialized and mutually separated, and after revalidating immutable collateral
    /// mint/vault custody. The payload binds the deployment's exact expected collateral-mint
    /// freeze authority (`None` is explicit, not a wildcard).
    /// Accounts: 0 current admin signer; 1 writable typed config; 2 signer registry;
    /// 3 registry's current signer set; 4 collateral mint; 5 vault token account;
    /// 6 SPL Token program.
    ActivateVaultV2 {
        params: ActivateVaultV2Params,
    },

    /// Deposit operator-owned USDC as unreceipted protocol funding.
    ///
    /// Accounts:
    /// 0. [signer] Current canonical vault-config admin
    /// 1. [writable] Admin-owned USDC token account
    /// 2. [writable] Vault USDC token account
    /// 3. [] Vault config PDA
    /// 4. [] USDC mint account
    /// 5. [] SPL token program
    DepositUsdc {
        amount: u64,
    },

    /// Initialize a market header without accepting caller-selected contract mints.
    ///
    /// Accounts:
    /// 0. [signer, writable] Current config admin / payer
    /// 1. [writable] New canonical market PDA
    /// 2. [] Canonical vault config PDA
    /// 3. [] System program
    InitMarketV2 {
        params: InitMarketV2Params,
    },

    /// Create and bind the market's canonical zero-supply classic-SPL contract mint, then create
    /// its Light SPL-interface account. Light and classic-SPL accounts share this mint identity.
    ///
    /// Accounts:
    /// 0. [signer, writable] Current config admin / payer
    /// 1. [writable] Canonical market PDA
    /// 2. [] Canonical vault config PDA
    /// 3. [writable] Canonical contract mint PDA
    /// 4. [writable] Canonical Light SPL-interface PDA
    /// 5. [] Light token program
    /// 6. [] Light CPI authority PDA
    /// 7. [] Classic SPL token program
    /// 8. [] System program
    CreateMarketContractMintV3,

    /// Set the market circuit breaker. This remains callable while the global pause is active.
    ///
    /// Accounts:
    /// 0. [signer] Current config admin
    /// 1. [] Canonical vault config PDA
    /// 2. [writable] Canonical market PDA
    SetMarketPaused {
        params: SetMarketPausedParams,
    },

    /// Bootstrap the typed signer registry and immutable version-1 signer set. The vault must be
    /// globally paused and the current admin, distinct oracle authority, and recovery authority all
    /// sign. The governed fields normally initialize to threshold 3 and signer_count 5.
    InitializeSettlementSignerRegistry {
        params: InitializeSettlementSignerRegistryParams,
    },

    /// Schedule an ordinary signer-set rotation. Both distinct governance authorities sign and the
    /// current signer threshold attests the exact pending-set digest in preceding Ed25519 checks.
    ProposeSettlementSignerRotation {
        params: ProposeSettlementSignerRotationParams,
    },

    /// Permissionlessly activate the exact pending signer set after its timelock.
    ActivateSettlementSignerRotation,

    /// Cancel a pending rotation before activation with both governance authorities.
    CancelSettlementSignerRotation,

    /// Schedule signer recovery without current-set attestations. This requires a global pause,
    /// both governance authorities, the configured recovery authority, and a doubled timelock.
    ProposeEmergencySettlementSignerRecovery {
        params: ProposeSettlementSignerRotationParams,
    },

    /// Publish schema-v4 settlement state only after a finalized canonical terminal-source
    /// manifest binds the recipe and source digests.
    UpsertSettlementV3 {
        params: UpsertSettlementParams,
        proof: Box<ValidityProof>,
        new_settlement_output: Option<Box<CompressionOutput>>,
        existing_settlement: Option<Box<CompressedSettlementWitness>>,
    },

    /// Create or update the compressed market-page registry leaf for a trade item route.
    UpsertMarketPageV2 {
        params: UpsertMarketPageParams,
        proof: Box<ValidityProof>,
        new_page_output: Option<Box<CompressionOutput>>,
        existing_page: Option<Box<CompressedMarketPageWitness>>,
    },

    /// Create the current user collateral PDA used by writer collateral.
    InitUserCollateral,

    /// Deposit collateral into the exchange vault and credit the user's free balance.
    DepositCollateral {
        amount: u64,
    },

    /// Withdraw free collateral from the exchange vault.
    WithdrawCollateral {
        amount: u64,
    },

    /// Withdraw only one canonical owner's available collateral with independent approval from
    /// the current config admin. Locked collateral is never debited.
    ///
    /// Accounts:
    /// 0. [signer] Current canonical vault-config admin
    /// 1. [signer] Canonical collateral owner
    /// 2. [writable] Canonical vault USDC token account
    /// 3. [writable] Owner's canonical classic-SPL USDC associated token account
    /// 4. [] Canonical vault config PDA (PDA transfer authority)
    /// 5. [writable] Canonical `UserCollateral` PDA for account 1
    /// 6. [] Canonical USDC mint
    /// 7. [] Classic SPL token program
    AdminAssistedWithdrawCollateral {
        amount: u64,
    },

    /// Update the governed economics template used only by subsequently created months.
    ConfigureOracleEconomicsTemplateV2 {
        params: ConfigureOracleEconomicsTemplateV2Params,
    },
    /// Create a rolling far-month series in fail-closed source submission, snapshotting the exact
    /// required terminal-SKU Merkle root and count into a new coverage manifest.
    InitializeOracleMonthV5 {
        params: InitializeOracleMonthV5Params,
    },
    /// Append one packet-safe chunk to a nonce-scoped product-SKU draft. The exact final append
    /// validates the complete ordered identifier set, derives its root, and creates the immutable
    /// product-level commitment consumed by every V5 month.
    ///
    /// Accounts: 0 writable config-admin signer/payer; 1 oracle-authority signer; 2 vault config;
    /// 3 writable product-SKU draft PDA; 4 writable product-SKU manifest PDA; 5 system program.
    ConfigureOracleProductSkuManifest {
        params: ConfigureOracleProductSkuManifestParams,
    },
    /// Terminalize one still-Candidate V5 source after the immutable expiry can no longer preserve
    /// the full post-submission review windows. The month remains fail-closed in SourceSubmission.
    ///
    /// Accounts: 0 cranker signer; 1 market; 2 writable month; 3 writable OSC; 4 writable source;
    /// 5 writable OSK only when the source has nonzero support.
    ExpireUnlistableOracleSourceV2,
    /// Fail-closed cleanup for a V5 cash source challenge after the immutable expiry can no longer
    /// preserve all review and Opening windows. This only cancels the stale challenge, releases its
    /// canonical guards, and drains the pending-resolution count; it cannot activate any source.
    ///
    /// Accounts: 0 cranker signer; 1 market; 2 writable month; 3 writable OSC;
    /// 4 writable challenge; 5 writable primary-source guard; 6 writable comparison-source guard
    /// only for a differentiation challenge; final writable staking pool only when the stale
    /// challenge is `RuleReviewUnresolved`.
    CancelStaleOracleSourceChallengeV2,
    /// Resolve a V5 source OED/v3 while atomically maintaining exact OSC/OSK coverage.
    ///
    /// Accounts: 0 cranker signer; 1 market; 2 writable month; 3 writable OED/v3 dispute;
    /// 4 writable source challenge; 5 writable OEP/v1 pot; 6 writable pot sAMBA ATA;
    /// 7 writable staking pool; then source, primary guard, optional comparison source and guard.
    /// When a comparison exists, append its exact SKU pool, challenged-source reward, and
    /// comparison-source reward before the writable OSC and exact writable OSK.
    ResolveOracleEmergencyDisputeV4 {
        params: ResolveOracleEmergencyDisputeParams,
    },
    /// Reconcile one V5 pre-listing escrow after the month has crossed its immutable latest-safe
    /// coverage boundary. The schedule counter and the escrow's own counted latch change
    /// atomically, so an already-accounted principal cannot be counted twice.
    ///
    /// Accounts begin with cranker, market, month, and writable reward schedule. Listing then uses
    /// writable source, source reward, and proposer collateral; support uses source, writable
    /// support position, and supporter collateral; source challenge uses writable source, source
    /// reward, challenge, both collateral ledgers, primary guard, and optional comparison guard.
    SettleFailedOracleMonthEscrowV2 {
        params: SettleOracleEscrowParams,
    },
    /// Release one failed V5 month's complete cash-reward reservation after its coverage remains
    /// incomplete and every tracked pre-listing escrow has been reconciled.
    /// Accounts: cranker signer, market, month, OSC, writable reward vault, writable schedule.
    AbortOracleUsdcRewardScheduleV2,
    /// Terminalize a zero-support V5 Candidate after Placement without changing otherwise-complete
    /// SKU coverage. Its listing principal remains refundable through the counted escrow lane.
    /// Accounts: cranker signer, market, writable month, OSC, writable source.
    TimeoutUnsupportedOracleSourceV2,
    InitializeOracleUsdcRewardVault,
    DepositOracleUsdcRewards {
        params: DepositOracleUsdcRewardsParams,
    },
    BeginOracleUsdcRewardSchedule {
        params: BeginOracleUsdcRewardScheduleParams,
    },
    AddOracleUsdcSkuBudget {
        params: AddOracleUsdcSkuBudgetParams,
    },
    FinalizeOracleUsdcRewardSchedule,
    /// Challenge one current cash source and atomically reserve every affected source guard.
    ///
    /// Accounts: 0 challenger signer/payer; 1 market; 2 writable month; 3 SKU; 4 source;
    /// 5 writable challenger collateral; 6 writable challenge; 7 system program;
    /// 8 writable primary-source guard. Non-independent-source challenges additionally use
    /// 9 comparison source and 10 writable comparison-source guard. A V5 month appends its exact
    /// writable reward schedule after those fixed/conditional challenge accounts.
    ChallengeOracleSourceV2 {
        params: ChallengeOracleSourceParams,
    },
    SubmitOracleOpeningClaimV2 {
        params: SubmitOracleOpeningClaimParams,
    },
    ChallengeOracleOpeningClaimV2 {
        params: ChallengeOracleOpeningClaimParams,
    },
    ResolveOracleOpeningClaimChallengeV2 {
        params: ResolveOracleOpeningClaimChallengeParams,
    },
    FinalizeOracleOpeningClaimV2,
    /// Commit a current cash update claim.
    ///
    /// Accounts: 0 claimant signer/payer; 1 market; 2 writable oracle month;
    /// 3 oracle source; 4 finalized active-weight manifest; 5 SKU;
    /// 6 writable update claim; 7 writable claimant collateral; 8 system program.
    CommitOracleUpdateClaimV3 {
        params: CommitOracleUpdateClaimV2Params,
    },
    /// Reveal a current cash update commitment.
    ///
    /// Accounts: 0 claimant signer; 1 market; 2 oracle month; 3 oracle source;
    /// 4 finalized active-weight manifest; 5 active-source weight; 6 writable update claim.
    RevealOracleUpdateClaimV3 {
        params: RevealOracleUpdateClaimV3Params,
    },
    SettleExpiredOracleUpdateCommitmentV3,
    /// Challenge one current update claim and create its immutable one-challenge guard atomically.
    ///
    /// Accounts: 0 challenger signer/payer; 1 market; 2 month; 3 SKU; 4 claim; 5 source;
    /// 6 writable challenger collateral; 7 writable challenge; 8 system program;
    /// 9 writable update-challenge guard.
    ChallengeOracleUpdateClaimV2 {
        params: ChallengeOracleUpdateClaimParams,
    },
    /// Finalize a current cash update claim.
    ///
    /// Fixed accounts: 0 oracle-authority signer; 1 market; 2 config; 3 writable month;
    /// 4 writable source; 5 active-weight manifest; 6 active-source weight; 7 writable claim.
    /// Unchallenged acceptance appends the exact absent update-guard PDA proof. Challenged
    /// acceptance/rejection appends the writable challenge and initialized guard. An unresolved
    /// challenge appends the writable challenge, writable staking pool, sAMBA mint, and writable
    /// guard.
    FinalizeOracleUpdateClaimV2 {
        params: FinalizeOracleUpdateClaimV2Params,
    },
    /// Settle one ordinary-work USDC escrow. The three pre-listing kinds additionally carry the
    /// writable counted schedule and per-object latch.
    ///
    /// Guarded account tails are exact for these kinds:
    /// - SourceChallenge: 0 cranker signer; 1 market; 2 writable month; 3 writable source;
    ///   4 writable challenge; 5 writable proposer collateral; 6 writable challenger collateral;
    ///   7 writable primary guard; and 8 writable comparison guard only for differentiation.
    /// - UpdateClaim: 0 cranker signer; 1 market; 2 month; 3 source; 4 writable claim;
    ///   5 writable claimant collateral; 6 read-only exact absent update-guard PDA proof.
    /// - UpdateChallenge: 0 cranker signer; 1 market; 2 month; 3 writable claim;
    ///   4 writable challenge; 5 writable claimant collateral; 6 writable challenger collateral;
    ///   7 read-only initialized update guard.
    SettleOracleUsdcEscrow {
        params: SettleOracleEscrowParams,
    },
    /// Register one terminal final-root source for deterministic cash rewards.
    /// Accounts: payer, month, writable schedule, writable SKU pool, OAW manifest, final source,
    /// final active-source weight, writable final source reward, and an accepted opening claim
    /// exactly when the final source is Active.
    RegisterOracleUsdcRewardSource,
    RegisterOracleUsdcRewardUpdate,
    FinalizeOracleUsdcRewardEntitlements,
    ClaimOracleUsdcReward {
        params: ClaimOracleUsdcRewardParams,
    },

    /// Configure the real AMBA SPL token mint and program-owned custody account.
    ConfigureOracleMajorToken {
        params: ConfigureOracleMajorTokenParams,
    },

    /// Deposit real AMBA SPL tokens into oracle custody and credit the player's ledger.
    DepositOracleMajorTokens {
        params: DepositOracleMajorTokensParams,
    },

    /// Withdraw available AMBA SPL tokens from oracle custody.
    WithdrawOracleMajorTokens {
        params: WithdrawOracleMajorTokensParams,
    },

    /// Create the canonical sAMBA SPL mint, vote vault, and liquid-staking pool.
    /// Accounts: oracle authority signer/payer, vault config, Major config, AMBA mint,
    /// staking pool, sAMBA mint, sAMBA vote vault, token program, system program.
    InitializeOracleSambaPool {
        params: InitializeOracleSambaPoolParams,
    },

    /// Create the canonical reward-funnel PDA and its classic SPL AMBA associated token account.
    /// This setup lane is permissionless and remains available while the protocol is paused.
    /// Accounts: payer, vault config, Major config, AMBA mint, funnel PDA, funnel ATA,
    /// token program, associated-token program, system program.
    InitializeOracleRewardFunnel {
        params: InitializeOracleRewardFunnelParams,
    },

    /// Atomically sweep the funnel's entire AMBA balance into canonical custody and allocate it
    /// across Game, Scramble, Challenge, sAMBA backing, and Reserve. This inbound-only lane remains
    /// available while paused. Accounts: cranker, vault config, Major config, funnel PDA, funnel
    /// ATA, AMBA vault, AMBA mint, treasury, staking pool, sAMBA mint, token program.
    SweepOracleRewardFunnel {
        params: SweepOracleRewardFunnelParams,
    },

    /// Lock available AMBA in an owner-scoped activation request. Queued principal remains outside
    /// active staking backing and earns no rewards until delayed activation.
    /// Accounts: owner signer/payer, vault config, Major config, player ledger, staking pool,
    /// stake-activation PDA, system program.
    QueueStakeAmbaForSamba {
        params: QueueStakeAmbaForSambaParams,
    },

    /// After the fixed activation delay, mint transferable sAMBA at the then-current exchange rate.
    /// The canonical reward funnel must be empty so unswept rewards are priced before minting.
    /// Accounts: owner signer, vault config, Major config, staking pool, stake-activation PDA,
    /// reward-funnel PDA, reward-funnel ATA, sAMBA mint, owner sAMBA token account, token program.
    ActivateQueuedStakeAmbaForSamba {
        params: ActivateQueuedStakeAmbaForSambaParams,
    },

    /// Return queued AMBA to the owner's available ledger. This owner safety lane remains usable
    /// while paused and while governance freezes sAMBA supply.
    /// Accounts: owner signer, vault config, player ledger, stake-activation PDA.
    CancelQueuedStakeAmba {
        params: CancelQueuedStakeAmbaParams,
    },

    /// Burn sAMBA and reserve its current AMBA value behind a seven-day unbonding request.
    /// Accounts: owner signer/payer, vault config, Major config, staking pool, unstake request,
    /// sAMBA mint, owner sAMBA token account, token program, system program.
    RequestUnstakeSamba {
        params: RequestUnstakeSambaParams,
    },

    /// Complete a mature unbonding request and credit its reserved AMBA to available balance.
    /// Accounts: owner signer/payer, vault config, player ledger, staking pool,
    /// unstake request, system program.
    CompleteUnstakeSamba {
        params: CompleteUnstakeSambaParams,
    },

    /// Coverage-aware source proposal. The bucket/SKU must prove membership in the immutable
    /// month manifest. It remains available while an incomplete month is extending submission.
    ProposeOracleSourceV3 {
        params: ProposeOracleSourceV3Params,
    },

    /// Coverage-aware source support. The first supported source for a SKU creates its canonical
    /// coverage record and advances the exact month coverage count.
    SupportOracleSourceV3 {
        params: SupportOracleSourceV3Params,
    },

    /// Close source submission only at exact terminal-SKU coverage. On-time completion preserves
    /// the original calendar; late completion shifts listing to retain every review window.
    FinalizeOracleSkuCoverage,

    /// Coverage-aware source-challenge resolution. Removing the final supported source for a SKU
    /// invalidates the coverage latch so submission must reopen before recipe freeze. MergeSource
    /// appends the exact SKU pool plus challenged/comparison source-reward records immediately
    /// after the comparison source and before both guards, then appends OSC/OSK last. The fixed
    /// prefix is authority, config, month, source, and challenge.
    ResolveOracleSourceChallengeV2 {
        params: ResolveOracleSourceChallengeParams,
    },

    /// Terminalize a stale current-cash update claim without moving either USDC or sAMBA.
    ///
    /// Accounts:
    /// 0. [signer] Permissionless cranker.
    /// 1. [] Market.
    /// 2. [writable] Current-cash oracle month.
    /// 3. [] Canonical source.
    /// 4. [writable] Canonical UC2/v1 update claim.
    /// 5. [] Canonical absent-guard proof, or [writable] exact OUG/v1 guard.
    /// 6. [writable, optional] Exact UCH/v3 challenge when the guard exists.
    /// 7. [writable, optional] Canonical staking pool for an unresolved checkpoint.
    CancelStaleOracleUpdateClaimV2,

    /// Neutralize a stale current-cash update emergency dispute while leaving all principal in
    /// its existing USDC and sAMBA escrow lanes for tags 169 and 180.
    ///
    /// Accounts:
    /// 0. [signer] Permissionless cranker.
    /// 1. [] Market.
    /// 2. [writable] Current-cash oracle month.
    /// 3. [] Canonical source.
    /// 4. [writable] Canonical UC2/v1 update claim.
    /// 5. [writable] Exact UCH/v3 challenge.
    /// 6. [writable] Exact OUG/v1 guard.
    /// 7. [writable] Exact OED/v3 update dispute.
    /// 8. [writable] Exact OEP/v1 sAMBA pot.
    /// 9. [] Canonical OEP sAMBA ATA.
    /// 10. [writable] Canonical staking pool.
    AbortStaleOracleUpdateEmergencyDisputeV2,

    /// Re-enter fail-closed source submission after a resolution made coverage incomplete.
    ReopenOracleSkuCoverage,

    /// Begin fresh recipe weights only when the immutable SKU coverage manifest remains complete.
    BeginOracleRecipeWeightsV3 {
        params: BeginOracleRecipeWeightsV2Params,
    },

    /// Freeze one bucket's canonical source membership in ordered chunks.
    AccumulateOracleRecipeBucketV2 {
        params: AccumulateOracleRecipeBucketV2Params,
    },

    /// Finalize a complete 10_000-bps canonical recipe-weight manifest.
    FinalizeOracleRecipeWeightsV2,

    /// Create or extend the immutable terminal-source commitment; the final chunk freezes it.
    AccumulateOracleSettlementSourceBucket {
        params: AccumulateOracleSettlementSourceBucketParams,
    },

    BeginOracleActiveWeights {
        params: BeginOracleActiveWeightsParams,
    },
    AccumulateOracleActiveWeightGroup {
        params: AccumulateOracleActiveWeightGroupParams,
    },
    FinalizeOracleActiveWeights,
    RecomputeOracleBucketMedianV1 {
        params: RecomputeOracleBucketMedianV1Params,
    },
    /// Advance from opening-print mode to game mode after every frozen source is terminal.
    FinalizeOracleOpeningPhase,

    /// Permissionlessly mark an unclaimed frozen source inactive after the opening window closes.
    ///
    /// Accounts:
    /// 0. [signer] Cranker
    /// 1. [] Market PDA
    /// 2. [writable] Oracle month PDA
    /// 3. [writable] Oracle source PDA
    ExpireOracleOpeningSource,

    /// Open the cash-month OED/v3 emergency lane and its isolated per-dispute sAMBA pot ATA.
    ///
    /// Accounts: 0 cranker signer/payer; 1 market; 2 month; 3 writable OED/v3;
    /// 4 target; 5 writable pot PDA; 6 writable pot sAMBA ATA; 7 vault config;
    /// 8 staking pool; 9 sAMBA mint; 10 classic SPL token program;
    /// 11 associated-token program; 12 system program; 13+ exact kind-specific accounts:
    /// - Source: source, writable primary guard, then comparison source and writable comparison
    ///   guard only for differentiation.
    /// - Update: claim, source, active-weight manifest, active-source weight, writable update guard.
    /// - Opening: claim, source.
    TryOpenOracleEmergencyDisputeV2 {
        params: TryOpenOracleEmergencyDisputeParams,
    },

    /// Commit a V3 vote and transfer its exact sAMBA principal into the isolated pot ATA.
    ///
    /// Accounts: 0 voter signer/payer; 1 config; 2 staking pool; 3 sAMBA mint;
    /// 4 writable voter sAMBA account; 5 writable OED/v3; 6 writable pot; 7 writable pot ATA;
    /// 8 writable OEV/v3; 9 classic SPL token program; 10 system program.
    CommitOracleEmergencyVoteV3 {
        params: CommitOracleEmergencyVoteV2Params,
    },

    /// Reveal one V3 vote under the amount-, mint-, program-, and dispute-bound commitment.
    ///
    /// Accounts: 0 voter signer; 1 writable OED/v3; 2 pot; 3 writable OEV/v3.
    RevealOracleEmergencyVoteV2 {
        params: RevealOracleEmergencyVoteParams,
    },

    /// Resolve the target, freeze the pot as Redistribute or RefundAll, and release one sAMBA
    /// governance lock. This instruction never moves AMBA, USDC, or sAMBA principal.
    ///
    /// Accounts: 0 cranker signer; 1 market; 2 writable month; 3 writable OED/v3;
    /// 4 writable target;
    /// 5 writable pot; 6 pot ATA; 7 writable staking pool; 8+ exact kind-specific cash-resolution
    /// accounts:
    /// - Source: writable source, writable primary guard, then writable comparison source and
    ///   writable comparison guard only for differentiation (regardless of the resolved choice).
    /// - Update: writable claim, writable source, read-only OAW, read-only OAS, writable update
    ///   guard.
    /// - Opening: writable claim, writable source.
    ResolveOracleEmergencyDisputeV2 {
        params: ResolveOracleEmergencyDisputeParams,
    },

    /// Permissionlessly register one revealed winning vote. The last canonical registration
    /// finalizes exact dust for the lexicographically smallest winning vote PDA.
    ///
    /// Accounts: 0 payer signer; 1 OED/v3; 2 writable pot; 3 OEV/v3;
    /// 4 writable winning-vote registration; 5 system program.
    RegisterOracleSambaWinningVote,

    /// Permissionlessly settle one V3 vote to its voter's canonical classic-SPL sAMBA ATA.
    ///
    /// Accounts: 0 payer signer; 1 OED/v3; 2 writable pot; 3 writable pot ATA; 4 sAMBA mint;
    /// 5 writable OEV/v3; 6 writable canonical voter ATA; 7 writable settlement receipt;
    /// 8 classic SPL token program; 9 system program; 10 winning registration only when the
    /// resolved pot is Redistribute and the vote revealed the winning choice.
    SettleOracleSambaEmergencyVoteV2,

    /// Close the oracle month after the option settlement has been recorded.
    CloseOracleMonth,

    /// Permissionlessly finalize a quiet oracle month after the post-expiry evidence window.
    FinalizeOracleMonth,

    /// Create the single governed writer-policy registry and its fee vault.
    InitializeWriterPolicyRegistryV1 {
        params: InitializeWriterPolicyRegistryV1Params,
    },
    /// Propose, activate, or cancel the delayed writer-policy authority rotation.
    ManageWriterPolicyAuthorityV1 {
        params: ManageWriterPolicyAuthorityV1Params,
    },
    /// Bind one underlying-expiry sleeve to a canonical shared settlement world.
    InitializeWriterSettlementGroupV1,
    /// Create sleeve accounting, its fixed-capacity book, USDC vault, and Flat token surfaces.
    InitializeWriterSleeveV1,
    /// Register one existing canonical Market in the sleeve's fixed-capacity series book.
    RegisterWriterSeriesV1,
    /// Seal one immutable, versioned economic-policy snapshot for the sleeve.
    SealWriterPolicyV1 {
        params: SealWriterPolicyV1Params,
    },
    /// Freeze series registration and admit writer-principal funding.
    OpenWriterFundingV1,
    /// Deposit writer principal and mint equal-par Flat ownership.
    DepositWriterPrincipalV1 {
        params: WriterAmountV1Params,
    },
    /// Reverse still-uncommitted funding before sleeve activation.
    WithdrawWriterPrincipalV1 {
        params: WriterAmountV1Params,
    },
    /// Activate a funded sleeve after recomputing every oracle and solvency commitment.
    ActivateWriterSleeveV1,
    /// Operate the collective Market circuit breaker under sleeve-level admission rules.
    SetCollectiveMarketPausedV1 {
        params: SetCollectiveMarketPausedV1Params,
    },
    /// Reconcile one series' physical supply/custody state into aggregate external OI.
    ReconcileWriterSupplyV1 {
        params: ReconcileWriterSupplyV1Params,
    },
    /// Burn issuer-controlled claim atoms from retirement custody in bounded work.
    CleanupWriterCustodyV1 {
        params: CleanupWriterCustodyV1Params,
    },
    /// Commit the next funded primary auction and its sealed reserve-price vector.
    CommitWriterAuctionV1 {
        params: CommitWriterAuctionV1Params,
    },
    /// Fund one deterministic primary-auction bid.
    PlaceWriterBidV1 {
        params: PlaceWriterBidV1Params,
    },
    /// Cancel an eligible bid or refund its unaccepted/unexecuted escrow.
    CancelOrRefundWriterBidV1,
    /// Reveal the reserve-price and issue-cap vectors committed by the sealed auction.
    RevealWriterAuctionV1 {
        params: Box<RevealWriterAuctionV1Params>,
    },
    /// Advance deterministic clearing over a bounded number of funded-bid records.
    PlanWriterAuctionChunkV1 {
        params: PlanWriterAuctionChunkV1Params,
    },
    /// Execute one already-planned accepted fill atomically into sleeve custody and buyer claims.
    ExecuteWriterAuctionFillV1,
    /// Finalize a fully executed auction or abort it without creating liabilities.
    FinalizeOrAbortWriterAuctionV1 {
        params: FinalizeOrAbortWriterAuctionV1Params,
    },
    /// Lock Flat and snapshot the whole-book proportional close basket.
    BeginWriterCloseV1 {
        params: BeginWriterCloseV1Params,
    },
    /// Deposit the next required series basket component into retirement custody.
    DepositWriterCloseBasketV1 {
        params: WriterSeriesIndexV1Params,
    },
    /// Atomically finalize the whole-book close and release its statewise-safe USDC amount.
    FinalizeWriterCloseV1,
    /// Refund one staged close component, or the final Flat escrow after cancellation.
    ProcessWriterCloseCancellationV1 {
        params: ProcessWriterCloseCancellationV1Params,
    },
    /// Copy the exact canonical anchor settlement into the shared sleeve settlement group.
    PublishWriterGroupSettlementV1,
    /// Freeze the long-reserve and Flat-residual ledgers at the shared settlement value.
    FinalizeWriterSleeveSettlementV1,
    /// Burn/deliver collective long claims and pay from the frozen long-reserve ledger.
    ClaimCollectiveLongV1 {
        params: ClaimCollectiveLongV1Params,
    },
    /// Burn Flat and pay pro rata from the independently frozen residual ledger.
    ClaimWriterFlatResidualV1 {
        params: ClaimWriterFlatResidualV1Params,
    },
    /// Close a fully exhausted collective sleeve and its terminal companion state.
    CloseWriterSleeveV1,

    /// Run one current instruction against transient native views of bounded Light-compressed
    /// typed state. The exact inner accounts come first, followed by one system-program account
    /// and the packed Light CPI accounts. Every transient view is closed before success returns.
    ExecuteCompressedStateV1 {
        params: ExecuteCompressedStateParams,
    },
}
