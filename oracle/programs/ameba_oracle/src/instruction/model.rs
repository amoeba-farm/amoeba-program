use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum OracleInstruction {
    /// Create the singleton, paused config and its bond vault. Only the program's recorded
    /// upgrade authority may initialize it. `admin` becomes both the config admin and the oracle
    /// authority; tag 128 later splits the oracle authority off.
    ///
    /// Accounts:
    /// 0. [signer, writable] Payer
    /// 1. [writable] Config PDA (also the bond vault's token authority)
    /// 2. [] Program data account of this program
    /// 3. [signer] Recorded upgrade authority
    /// 4. [] System program
    /// 5. [] USDC mint (six decimals)
    /// 6. [writable] Bond vault: the config PDA's associated token account for the mint
    /// 7. [] SPL token program
    /// 8. [] Associated token account program
    InitializeConfig { admin: Pubkey },

    /// Create-once Forming period for one product and fixed expiry. The listing and expiry must
    /// sit on the rolling three-month ladder and the period publishes six-decimal values.
    ///
    /// Accounts:
    /// 0. [signer, writable] Payer
    /// 1. [] Config PDA
    /// 2. [signer] Current oracle authority
    /// 3. [writable] Period PDA
    /// 4. [] System program
    CreatePeriod { params: CreatePeriodParams },

    /// Update admin, oracle authority, mint, bond vault and pause status. Authority changes and
    /// unpausing require their governed instructions; this one can only pause or confirm.
    ///
    /// Accounts:
    /// 0. [signer] Current admin
    /// 1. [writable] Config PDA
    /// 2. [] USDC mint
    /// 3. [] Bond vault
    /// 4. [] SPL token program
    UpdateConfig {
        new_admin: Option<Pubkey>,
        new_oracle_authority: Option<Pubkey>,
        new_usdc_mint: Option<Pubkey>,
        new_bond_vault: Option<Pubkey>,
        paused: Option<bool>,
    },

    /// Rotate the two config authorities without letting either current authority act alone.
    /// The config must be paused and both current and both incoming authorities sign.
    /// Accounts: 0 current admin; 1 current oracle; 2 incoming admin; 3 incoming oracle;
    /// 4 writable config; 5 signer registry; 6 active signer set.
    RotateVaultAuthoritiesV2 {
        params: RotateVaultAuthoritiesV2Params,
    },

    /// Split the fresh config's duplicated authority before the governed signer registry exists.
    /// This is a one-time, paused-only transition; later changes use governed rotation.
    /// Accounts: 0 bootstrap admin signer; 1 incoming oracle signer; 2 writable config;
    /// 3 canonical, not-yet-created settlement signer registry PDA.
    BootstrapVaultGovernanceV2 {
        params: BootstrapVaultGovernanceV2Params,
    },

    /// Unpause only after distinct governance and the canonical current settlement signer set are
    /// initialized and mutually separated, and after revalidating the immutable mint and bond
    /// vault custody. The payload binds the deployment's exact expected mint freeze authority
    /// (`None` is explicit, not a wildcard).
    /// Accounts: 0 current admin signer; 1 writable config; 2 signer registry;
    /// 3 registry's current signer set; 4 USDC mint; 5 bond vault; 6 SPL token program.
    ActivateVaultV2 { params: ActivateVaultV2Params },

    /// The council-approved CFM launch of a product's first month (`crate::cfm`): begin with
    /// three current council seats, append each registry row, finish once the recipe is indexed.
    /// Accounts: see the `cfm_launch_*` builders.
    CfmLaunchV1 { params: CfmLaunchV1Params },

    /// Create the caller's `UserCollateral` ledger.
    ///
    /// Accounts: 0 [signer, writable] owner and payer; 1 [writable] ledger PDA; 2 system program.
    InitUserCollateral,

    /// Deposit USDC into the bond vault and credit the owner's available balance.
    ///
    /// Accounts: 0 [signer] owner; 1 [writable] owner token account; 2 [writable] bond vault;
    /// 3 config; 4 [writable] ledger; 5 USDC mint; 6 SPL token program.
    DepositCollateral { amount: u64 },

    /// Withdraw available USDC from the bond vault.
    ///
    /// Accounts: 0 [signer] owner; 1 [writable] bond vault; 2 [writable] owner token account;
    /// 3 config; 4 [writable] ledger; 5 USDC mint; 6 SPL token program.
    WithdrawCollateral { amount: u64 },

    /// Governed, bounded authenticated information carry between prospective periods. The
    /// evidence selectors write, close and backfill immutable public preimages and their
    /// publication links.
    OracleCarryForwardV1 { action: OracleCarryForwardActionV1 },

    /// Withdraw only one canonical owner's available collateral with independent approval from
    /// the current config admin. Escrowed collateral is never debited.
    ///
    /// Accounts:
    /// 0. [signer] Current config admin
    /// 1. [signer] Canonical ledger owner
    /// 2. [writable] Bond vault
    /// 3. [writable] Owner's canonical classic-SPL USDC associated token account
    /// 4. [] Config PDA (token transfer authority)
    /// 5. [writable] Canonical `UserCollateral` PDA for account 1
    /// 6. [] USDC mint
    /// 7. [] SPL token program
    AdminAssistedWithdrawCollateral { amount: u64 },

    /// Update the governed economics template used only by subsequently created months.
    /// Accounts: 0 [signer, writable] admin and payer; 1 config; 2 [writable] economics PDA;
    /// 3 system program.
    ConfigureOracleEconomicsTemplateV2 {
        params: ConfigureOracleEconomicsTemplateV2Params,
    },

    /// Create the singleton reward vault PDA and its USDC associated token account. The reward
    /// vault is separate from the bond vault; only frozen schedules reserve against it.
    ///
    /// Accounts: 0 [signer, writable] payer; 1 config; 2 USDC mint; 3 [writable] reward vault
    /// PDA; 4 [writable] reward vault's associated token account; 5 SPL token program;
    /// 6 associated token account program; 7 system program.
    InitializeOracleUsdcRewardVault,

    /// Transfer USDC from any funder into the reward vault's token account.
    ///
    /// Accounts: 0 [signer] funder; 1 config; 2 USDC mint; 3 [writable] reward vault;
    /// 4 [writable] funder token account; 5 [writable] reward vault token account;
    /// 6 SPL token program.
    DepositOracleUsdcRewards {
        params: DepositOracleUsdcRewardsParams,
    },

    /// Create the month's reward schedule while the build window is open.
    ///
    /// Accounts: 0 [signer, writable] month authority and payer; 1 config; 2 period;
    /// 3 [writable] month; 4 reward vault; 5 [writable] schedule PDA; 6 system program.
    BeginOracleUsdcRewardSchedule {
        params: BeginOracleUsdcRewardScheduleParams,
    },

    /// Add one SKU pool with its finite budgets and exact bond policy to a building schedule.
    ///
    /// Accounts: 0 [signer, writable] month authority and payer; 1 config; 2 period; 3 month;
    /// 4 [writable] schedule; 5 [writable] SKU pool PDA; 6 system program.
    AddOracleUsdcSkuBudget {
        params: AddOracleUsdcSkuBudgetParams,
    },

    /// Freeze a building schedule once the reward vault holds every existing reservation plus
    /// this schedule's total budget.
    ///
    /// Accounts: 0 [signer] month authority; 1 config; 2 period; 3 [writable] month; 4 USDC mint;
    /// 5 [writable] reward vault; 6 reward vault token account; 7 [writable] schedule;
    /// 8 SPL token program.
    FinalizeOracleUsdcRewardSchedule,

    /// Challenge one current cash source and atomically reserve every affected source guard.
    ///
    /// Accounts: 0 challenger signer/payer; 1 period; 2 writable month; 3 SKU pool; 4 source;
    /// 5 writable challenger collateral; 6 writable challenge; 7 system program;
    /// 8 writable primary-source guard. Non-independent-source challenges additionally use
    /// 9 comparison source and 10 writable comparison-source guard. The month's exact writable
    /// reward schedule follows those fixed and conditional challenge accounts.
    ChallengeOracleSourceV2 { params: ChallengeOracleSourceParams },

    /// Settle one ordinary-work USDC escrow. The three pre-listing kinds additionally carry the
    /// writable counted schedule and per-object latch.
    ///
    /// Guarded account tails are exact for these kinds:
    /// - ListingBond: 0 cranker signer; 1 period; 2 month; 3 writable schedule; 4 writable source;
    ///   5 writable source reward; 6 writable proposer collateral.
    /// - SupportStake: 0 cranker signer; 1 period; 2 month; 3 writable schedule; 4 source;
    ///   5 writable support position; 6 writable supporter collateral.
    /// - SourceChallenge: 0 cranker signer; 1 period; 2 writable month; 3 writable schedule;
    ///   4 writable source; 5 writable source reward; 6 writable challenge; 7 writable proposer
    ///   collateral; 8 writable challenger collateral; 9 writable primary guard; and 10 writable
    ///   comparison guard only for differentiation.
    /// - OpeningClaim: 0 cranker signer; 1 period; 2 month; 3 source; 4 writable claim;
    ///   5 writable claimant collateral.
    /// - OpeningChallenge: 0 cranker signer; 1 period; 2 month; 3 writable source;
    ///   4 writable claim; 5 writable challenge; 6 writable claimant collateral; 7 writable
    ///   challenger collateral.
    /// - UpdateClaim: 0 cranker signer; 1 period; 2 month; 3 source; 4 writable claim;
    ///   5 writable claimant collateral; 6 read-only exact absent update-guard PDA proof.
    /// - UpdateChallenge: 0 cranker signer; 1 period; 2 month; 3 writable claim;
    ///   4 writable challenge; 5 writable claimant collateral; 6 writable challenger collateral;
    ///   7 read-only initialized update guard.
    SettleOracleUsdcEscrow { params: SettleOracleEscrowParams },

    /// Create the oracle month for one Forming period in fail-closed source submission,
    /// snapshotting the exact required terminal-SKU Merkle root and count into a new coverage
    /// manifest, and record or advance the product's maturity ladder rung.
    ///
    /// Accounts: 0 oracle-authority signer; 1 [signer, writable] payer; 2 period;
    /// 3 [writable] month PDA; 4 [writable] coverage manifest PDA; 5 config; 6 economics;
    /// 7 product SKU manifest; 8 [writable] maturity ladder PDA; 9 system program.
    InitializeOracleMonthV5 {
        params: InitializeOracleMonthV5Params,
    },

    /// Coverage-aware source proposal. The bucket/SKU must prove membership in the immutable
    /// month manifest. It remains available while an incomplete month is extending submission.
    /// The listing bond is debited from the proposer's `UserCollateral`.
    ///
    /// Accounts: 0 [signer, writable] proposer and payer; 1 period; 2 [writable] month;
    /// 3 [writable] schedule; 4 SKU pool; 5 [writable] source PDA; 6 [writable] observations PDA;
    /// 7 [writable] source reward PDA; 8 [writable] proposer collateral; 9 system program;
    /// 10 coverage manifest; 11 canonical absent carry-period PDA proof; 12 locator object;
    /// 13 [writable] locator link; 14 definition object; 15 [writable] definition link.
    ProposeOracleSourceV3 { params: ProposeOracleSourceV3Params },

    /// Coverage-aware source support. The first supported source for a SKU creates its canonical
    /// coverage record and advances the exact month coverage count.
    ///
    /// Accounts: 0 [signer, writable] supporter and payer; 1 period; 2 [writable] month;
    /// 3 [writable] schedule; 4 SKU pool; 5 [writable] source; 6 [writable] source reward;
    /// 7 [writable] supporter collateral; 8 [writable] support position PDA; 9 system program;
    /// 10 [writable] coverage manifest; 11 [writable] coverage record PDA.
    SupportOracleSourceV3 { params: SupportOracleSourceV3Params },

    /// Close source submission only at exact terminal-SKU coverage. On-time completion preserves
    /// the original calendar; late completion shifts listing to retain every review window.
    ///
    /// Accounts: 0 cranker signer; 1 period; 2 [writable] month; 3 [writable] coverage manifest.
    FinalizeOracleSkuCoverage,

    /// Coverage-aware source-challenge resolution. Removing the final supported source for a SKU
    /// invalidates the coverage latch so submission must reopen before recipe freeze. MergeSource
    /// appends the exact SKU pool plus challenged/comparison source-reward records immediately
    /// after the comparison source and before both guards, then appends the coverage manifest and
    /// record last. The fixed prefix is authority, config, month, source, and challenge.
    ResolveOracleSourceChallengeV2 {
        params: ResolveOracleSourceChallengeParams,
    },

    /// Re-enter fail-closed source submission after a resolution made coverage incomplete.
    ///
    /// Accounts: 0 cranker signer; 1 period; 2 [writable] month; 3 [writable] coverage manifest.
    ReopenOracleSkuCoverage,

    /// Append one packet-safe chunk to a nonce-scoped product-SKU draft. The exact final append
    /// validates the complete ordered identifier set, derives its root, and creates the immutable
    /// product-level commitment consumed by every month of that product.
    ///
    /// Accounts: 0 writable config-admin signer/payer; 1 oracle-authority signer; 2 config;
    /// 3 writable product-SKU draft PDA; 4 writable product-SKU manifest PDA; 5 system program.
    ConfigureOracleProductSkuManifest {
        params: ConfigureOracleProductSkuManifestParams,
    },

    /// Terminalize one still-Candidate source after the immutable expiry can no longer preserve
    /// the full post-submission review windows. The month remains fail-closed in SourceSubmission.
    ///
    /// Accounts: 0 cranker signer; 1 period; 2 writable month; 3 writable coverage manifest;
    /// 4 writable source; 5 writable coverage record only when the source has nonzero support.
    ExpireUnlistableOracleSourceV2,

    /// Fail-closed cleanup for a cash source challenge after the immutable expiry can no longer
    /// preserve all review and Opening windows. This only cancels the stale challenge, releases
    /// its canonical guards, and drains the pending-resolution count; it cannot activate any
    /// source.
    ///
    /// Accounts: 0 cranker signer; 1 period; 2 writable month; 3 writable coverage manifest;
    /// 4 writable challenge; 5 writable primary-source guard; 6 writable comparison-source guard
    /// only for a differentiation challenge.
    CancelStaleOracleSourceChallengeV2,

    /// Reconcile one pre-listing escrow after the month has crossed its immutable latest-safe
    /// coverage boundary. The schedule counter and the escrow's own counted latch change
    /// atomically, so an already-accounted principal cannot be counted twice.
    ///
    /// Accounts begin with cranker, period, month, and writable reward schedule. Listing then uses
    /// writable source, source reward, and proposer collateral; support uses source, writable
    /// support position, and supporter collateral; source challenge uses writable source, source
    /// reward, challenge, both collateral ledgers, primary guard, and optional comparison guard.
    SettleFailedOracleMonthEscrowV2 { params: SettleOracleEscrowParams },

    /// Release one failed month's complete cash-reward reservation after its coverage remains
    /// incomplete and every tracked pre-listing escrow has been reconciled.
    ///
    /// Accounts: cranker signer, period, month, coverage manifest, writable reward vault,
    /// writable schedule.
    AbortOracleUsdcRewardScheduleV2,

    /// Terminalize a zero-support Candidate after Placement without changing otherwise-complete
    /// SKU coverage. Its listing principal remains refundable through the counted escrow lane.
    ///
    /// Accounts: cranker signer, period, writable month, coverage manifest, writable source.
    TimeoutUnsupportedOracleSourceV2,

    /// Begin fresh recipe weights only when the immutable SKU coverage manifest remains complete.
    ///
    /// Accounts: 0 [signer, writable] admin or oracle authority and payer; 1 period; 2 config;
    /// 3 [writable] month; 4 [writable] recipe-weight manifest PDA; 5 system program;
    /// 6 coverage manifest; 7 canonical absent carry-period proof or the carry period.
    BeginOracleRecipeWeightsV3 {
        params: BeginOracleRecipeWeightsV2Params,
    },

    /// Freeze one bucket's canonical source membership in ordered chunks.
    ///
    /// Accounts: 0 [signer] admin or oracle authority; 1 period; 2 config; 3 month;
    /// 4 [writable] recipe-weight manifest; then up to four [writable] Candidate sources of the
    /// bucket in ascending source order.
    AccumulateOracleRecipeBucketV2 {
        params: AccumulateOracleRecipeBucketV2Params,
    },

    /// Finalize a complete 10_000-bps canonical recipe-weight manifest and move the month to
    /// Opening.
    ///
    /// Accounts: 0 [signer] admin or oracle authority; 1 period; 2 config; 3 [writable] month;
    /// 4 [writable] recipe-weight manifest.
    FinalizeOracleRecipeWeightsV2,

    /// Permissionlessly authenticate one frozen source in reverse recipe order.
    ///
    /// Accounts: 0 [signer, writable] payer; 1 period; 2 month; 3 finalized recipe manifest;
    /// 4 [writable] recipe source index; 5 [writable] bucket source index; 6 system program;
    /// 7 [writable] the bucket's member page for this position.
    IndexOracleRecipeSourceV1 {
        params: IndexOracleRecipeSourceV1Params,
    },

    /// Create or extend the immutable terminal-source commitment; the final chunk freezes it.
    ///
    /// Accounts: 0 [signer, writable] cranker and payer; 1 period; 2 month; 3 [writable]
    /// settlement-source manifest PDA; 4 system program; then up to eight bucket medians in
    /// ascending group order.
    AccumulateOracleSettlementSourceBucket {
        params: AccumulateOracleSettlementSourceBucketParams,
    },

    /// Submit the bonded opening print of one Frozen source with its sealed archive evidence (a
    /// Wayback archive URL, kind 3, or an authenticated evidence record, kind 4).
    ///
    /// Accounts: 0 [signer, writable] claimant and payer; 1 period; 2 [writable] month;
    /// 3 SKU pool; 4 [writable] source; 5 [writable] claimant collateral; 6 [writable] opening
    /// claim PDA; 7 system program; 8 sealed archive-URL object; 9 [writable] publication link.
    SubmitOracleOpeningClaimV2 {
        params: SubmitOracleOpeningClaimParams,
    },

    /// Challenge a pending opening claim with a bonded alternative print.
    ///
    /// Accounts: 0 [signer, writable] challenger and payer; 1 period; 2 [writable] month;
    /// 3 SKU pool; 4 source; 5 [writable] claim; 6 [writable] challenger collateral;
    /// 7 [writable] challenge PDA; 8 system program; 9 sealed archive-URL object;
    /// 10 [writable] publication link.
    ChallengeOracleOpeningClaimV2 {
        params: ChallengeOracleOpeningClaimParams,
    },

    /// Resolve an opening challenge in rule review.
    ///
    /// Accounts: 0 [signer] admin or oracle authority; 1 config; 2 [writable] month; 3 SKU pool;
    /// 4 [writable] source; 5 [writable] claim; 6 claimant collateral; 7 challenger collateral;
    /// 8 [writable] challenge.
    ResolveOracleOpeningClaimChallengeV2 {
        params: ResolveOracleOpeningClaimChallengeParams,
    },

    /// Accept an unchallenged opening claim after its challenge window: the source becomes
    /// Active with its first observation and its first checkpoint.
    ///
    /// Accounts: 0 [signer, writable] cranker and payer; 1 period; 2 [writable] month;
    /// 3 [writable] source; 4 [writable] observations; 5 [writable] claim; 6 claimant
    /// collateral; 7 [writable] source journal; 8 [writable] checkpoint PDA; 9 system program.
    FinalizeOracleOpeningClaimV2,

    /// Permissionlessly mark an unclaimed frozen source inactive after the opening window closes.
    ///
    /// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 [writable] source;
    /// 4 canonical absent carry-source proof or the carry source.
    ExpireOracleOpeningSource,

    /// Begin the active-weight manifest once every frozen source is terminal.
    ///
    /// Accounts: 0 [signer, writable] admin or oracle authority and payer; 1 period; 2 config;
    /// 3 [writable] month; 4 recipe-weight manifest; 5 [writable] active-weight manifest PDA;
    /// 6 system program.
    BeginOracleActiveWeights {
        params: BeginOracleActiveWeightsParams,
    },

    /// Collect one source of a bucket into the active-weight manifest and its median.
    ///
    /// Accounts: 0 [signer, writable] cranker and payer; 1 period; 2 [writable] month;
    /// 3 [writable] active-weight manifest; 4 system program; 5 [writable] SKU pool;
    /// 6 [writable] bucket median PDA; 7 source; 8 complete recipe source index; 9 bucket
    /// source index; 10 member page; 11 completed bucket-rank certificate (read only when the
    /// bucket has more active sources than the inline median holds).
    AccumulateOracleActiveWeightGroup {
        params: AccumulateOracleActiveWeightGroupParams,
    },

    /// Freeze the active-weight manifest into the month.
    ///
    /// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 recipe-weight manifest;
    /// 4 [writable] active-weight manifest.
    FinalizeOracleActiveWeights,

    /// Advance from opening-print mode to game mode after every frozen source is terminal, at or
    /// after the month's listing and before expiry. The period becomes Live at the month's
    /// listing.
    ///
    /// Accounts: 0 [signer] cranker; 1 [writable] period; 2 [writable] month; 3 finalized
    /// active-weight manifest.
    FinalizeOracleOpeningPhase,

    /// Recompute one bucket median as an ordered one-source-per-call walk after expiry and grace.
    ///
    /// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 [writable] bucket median;
    /// 4 [writable] source; 5 observations, or the absent observations PDA of an inactive
    /// source, or a completed history-median certificate; 6 complete recipe source index;
    /// 7 bucket source index; 8 member page; 9 completed bucket-rank certificate.
    RecomputeOracleBucketMedianV1 {
        params: RecomputeOracleBucketMedianV1Params,
    },

    /// Commit a sealed cash update for one Active source during Game.
    ///
    /// Accounts: 0 [signer, writable] claimant and payer; 1 period; 2 [writable] month; 3 source;
    /// 4 finalized active-weight manifest; 5 SKU pool; 6 [writable] update claim PDA;
    /// 7 [writable] claimant collateral; 8 system program.
    CommitOracleUpdateClaimV3 {
        params: CommitOracleUpdateClaimV2Params,
    },

    /// Refund a commitment nobody revealed by its deadline.
    ///
    /// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 source; 4 [writable] claim;
    /// 5 [writable] claimant collateral.
    SettleExpiredOracleUpdateCommitmentV3,

    /// Challenge one revealed update claim with a bonded alternative, creating its one-challenge
    /// guard atomically.
    ///
    /// Accounts: 0 [signer, writable] challenger and payer; 1 period; 2 month; 3 SKU pool;
    /// 4 claim; 5 source; 6 [writable] challenger collateral; 7 [writable] challenge PDA;
    /// 8 system program; 9 [writable] update-challenge guard PDA; 10 sealed archive-URL object;
    /// 11 [writable] publication link.
    ChallengeOracleUpdateClaimV2 {
        params: ChallengeOracleUpdateClaimParams,
    },

    /// Reveal a committed update with its sealed archive evidence (a Wayback archive URL, kind 3,
    /// or an authenticated evidence record, kind 4).
    ///
    /// Accounts: 0 [signer, writable] claimant and payer; 1 period; 2 month; 3 source;
    /// 4 finalized active-weight manifest; 5 [writable] claim; 6 system program; 7 sealed
    /// archive-URL object; 8 [writable] publication link.
    RevealOracleUpdateClaimV3 {
        params: RevealOracleUpdateClaimV3Params,
    },

    /// Finalize a revealed update claim after its challenge window.
    ///
    /// Fixed accounts: 0 [signer, writable] admin or oracle authority; 1 period; 2 config;
    /// 3 [writable] month; 4 [writable] source; 5 [writable] observations; 6 finalized
    /// active-weight manifest; 7 [writable] claim; 8 [writable] bucket median. Unchallenged
    /// acceptance appends the absent update-guard PDA. Challenged acceptance or rejection appends
    /// the writable challenge and the guard. An unresolved challenge appends the writable
    /// challenge and writable guard. Every outcome that changes the source state appends its
    /// checkpoint tail: source journal, checkpoint PDA keyed by the claim, system program.
    FinalizeOracleUpdateClaimV2 {
        params: FinalizeOracleUpdateClaimV2Params,
    },

    /// Terminalize a stale update claim without moving USDC.
    ///
    /// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; 3 source; 4 [writable] claim;
    /// 5 the absent update-guard PDA, or the [writable] guard; 6 [writable] the guarded challenge
    /// when the guard exists.
    CancelStaleOracleUpdateClaimV2,

    /// Close a Settled month.
    ///
    /// Accounts: 0 [signer] admin or oracle authority; 1 config; 2 period; 3 [writable] month;
    /// 4 the month's settlement record.
    CloseOracleMonth,

    /// Finalize a quiet month from its bucket medians after expiry and grace.
    ///
    /// Accounts: 0 [signer] cranker; 1 period; 2 [writable] month; then every bucket median of
    /// the month in group order.
    FinalizeOracleMonth,

    /// Move a Forming period that can no longer list to Failed.
    ///
    /// Accounts: 0 [signer] cranker; 1 [writable] period; 2 the period's month PDA (the month or
    /// the canonical absent account).
    FailPeriod,

    /// Create the signer registry and its immutable version-1 signer set. The config is paused
    /// and the admin, a distinct oracle authority and the recovery authority all sign.
    ///
    /// Accounts: 0 [signer, writable] admin and payer; 1 [signer] oracle authority; 2 [signer]
    /// recovery authority; 3 config; 4 [writable] registry PDA; 5 [writable] signer set PDA;
    /// 6 system program.
    InitializeSettlementSignerRegistry {
        params: InitializeSettlementSignerRegistryParams,
    },

    /// Schedule an ordinary signer-set rotation. Both governance authorities sign and the current
    /// signer threshold attests the exact pending-set digest in preceding Ed25519 checks.
    ///
    /// Accounts: 0 [signer, writable] admin and payer; 1 [signer] oracle authority; 2 config;
    /// 3 [writable] registry; 4 current signer set; 5 [writable] pending signer set PDA;
    /// 6 instructions sysvar; 7 system program.
    ProposeSettlementSignerRotation {
        params: ProposeSettlementSignerRotationParams,
    },

    /// Activate the exact pending signer set after its timelock. Permissionless.
    ///
    /// Accounts: 0 [writable] registry; 1 pending signer set; 2 config.
    ActivateSettlementSignerRotation,

    /// Cancel a pending rotation before activation.
    ///
    /// Accounts: 0 [signer] admin; 1 [signer] oracle authority; 2 config; 3 [writable] registry.
    CancelSettlementSignerRotation,

    /// Schedule signer recovery without current-set attestations: config paused, both
    /// governance authorities and the recovery authority sign, and the timelock doubles.
    ///
    /// Accounts: 0 [signer, writable] admin and payer; 1 [signer] oracle authority; 2 [signer]
    /// recovery authority; 3 config; 4 [writable] registry; 5 current signer set; 6 [writable]
    /// pending signer set PDA; 7 system program.
    ProposeEmergencySettlementSignerRecovery {
        params: ProposeSettlementSignerRotationParams,
    },

    /// Publish a month's settlement leaf once the signer threshold attests it, after expiry and
    /// grace and the month's finalization: the month becomes Settled and the period Settled.
    ///
    /// Accounts: 0 [signer, writable] publisher and payer; 1 config;
    /// 2 [writable] period; 3 [writable] month; 4 [writable] settlement record PDA; 5 recipe-weight
    /// manifest; 6 active-weight manifest; 7 settlement-source manifest; 8 signer registry;
    /// 9 current signer set; 10 instructions sysvar; 11 system program.
    UpsertSettlementV3 { params: UpsertSettlementParams },

    /// Register one terminal (Active or Inactive) source for the month's cash rewards.
    ///
    /// Accounts: 0 [signer] payer; 1 month; 2 [writable] reward schedule; 3 [writable] SKU pool;
    /// 4 finalized active-weight manifest; 5 source; 6 [writable] source reward; 7 the accepted
    /// opening claim (Active sources with a fresh opening only); last the source's carry-source
    /// account or its canonical absent PDA.
    RegisterOracleUsdcRewardSource,

    /// Register one accepted update for the month's cash rewards.
    ///
    /// Accounts: 0 [signer, writable] payer; 1 month; 2 [writable] reward schedule; 3 [writable]
    /// SKU pool; 4 source; 5 finalized update claim; 6 [writable] registration PDA; 7 bucket
    /// median; 8 system program.
    RegisterOracleUsdcRewardUpdate,

    /// Freeze the month's cash entitlements once the month is settled and every source, opening
    /// and update is registered.
    ///
    /// Accounts: 0 [signer] month authority; 1 period; 2 month; 3 reward vault; 4 [writable]
    /// reward schedule.
    FinalizeOracleUsdcRewardEntitlements,

    /// Claim one finalized cash reward into the recipient's collateral ledger.
    ///
    /// Accounts: 0 [signer, writable] recipient and payer; 1 config; 2 period; 3 month;
    /// 4 [writable] reward schedule; 5 [writable] reward vault; 6 [writable] reward vault token
    /// account; 7 [writable] bond vault; 8 USDC mint; 9 [writable] recipient collateral;
    /// 10 [writable] receipt PDA; 11 token program; 12 system program; then the kind's accounts:
    /// proposer: SKU pool, source, source reward, carry-source proof; supporter: SKU pool,
    /// source, source reward, merge lineage pairs, support position; opening: SKU pool, source,
    /// source reward, accepted opening claim; update: SKU pool, registration, update claim.
    ClaimOracleUsdcReward { params: ClaimOracleUsdcRewardParams },
}
