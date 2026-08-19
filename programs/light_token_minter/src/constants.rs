use solana_program::{pubkey, pubkey::Pubkey};

/// Fresh-state namespace for every Amoeba-owned PDA and compressed Light address.
pub const CURRENT_STATE_NAMESPACE_SEED: &[u8] = b"ameba-spread-v2";
/// Shared deterministic address domain for proof-backed representations of current typed PDAs.
pub const COMPRESSED_STATE_LEAF_ADDRESS_SEED: &[u8] = b"compressed-state-v1";
/// Current Devnet Light V2 address space. A logical record has one address independent of the
/// state tree that presently stores its leaf, so every create/read/update/close path pins this
/// same tree instead of deriving from a mutable state-tree context.
pub const LIGHT_DEFAULT_ADDRESS_TREE_V2: Pubkey =
    pubkey!("amt2kaJA14v3urZbZvnc5v2np8jqvc4Z8zDep5wbtzx");
pub const LIGHT_TOKEN_COMPRESSIBLE_CONFIG: Pubkey =
    pubkey!("ACXg8a7VaqecBWrSbdu73W4Pg9gsqXJ3EXAqkHyhvVXg");
pub const LIGHT_TOKEN_RENT_SPONSOR: Pubkey = pubkey!("r18WwUxfG8kQ69bQPAB2jV6zGNKy3GosFGctjQoV4ti");
/// One compressed-state instruction deliberately stays within one combined Light proof batch.
pub const MAX_COMPRESSED_STATE_SESSION_RECORDS: usize = 8;
/// Compact leaf payloads contain only non-derivable state. Canonical identities are reconstructed
/// from the unchanged instruction accounts and verified against the compressed address.
pub const MAX_COMPRESSED_STATE_LEAF_BYTES: usize = 768;
/// Leaves and proof metadata must still fit below the protocol-wide 16,384-byte outer cap.
pub const MAX_COMPRESSED_INNER_INSTRUCTION_BYTES: usize = 12_288;
pub const VAULT_PDA_SEED: &[u8] = b"vault_config";
/// Temporary project bootstrap authority allowed to initialize the singleton vault config PDA.
pub const VAULT_CONFIG_BOOTSTRAP_AUTHORITY: Pubkey =
    pubkey!("99riHvpFvwfz2tbrWbanEMPz5iyhHHThBbM7eY35vMwL");
pub const MARKET_PDA_SEED: &[u8] = b"market";
pub const USER_COLLATERAL_PDA_SEED: &[u8] = b"user_collateral";
pub const POSITION_PDA_SEED: &[u8] = b"position";
/// Market/month-scoped immutable settlement record.
pub const SETTLEMENT_V2_PDA_SEED: &[u8] = b"settlement_v2";
pub const ORACLE_MONTH_PDA_SEED: &[u8] = b"oracle_month";
pub const ORACLE_SOURCE_PDA_SEED: &[u8] = b"oracle_source";
pub const ORACLE_SOURCE_OBSERVATIONS_PDA_SEED: &[u8] = b"oracle_source_observations";
/// Month-scoped immutable commitment to the complete required terminal-SKU set.
pub const ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED: &[u8] = b"oracle-sku-coverage-v1";
/// Product-scoped, create-once commitment to the complete canonical terminal-SKU set.
pub const ORACLE_PRODUCT_SKU_MANIFEST_PDA_SEED: &[u8] = b"oracle-product-skus-v1";
/// Nonce-scoped governed draft used to assemble one packet-safe product SKU manifest.
pub const ORACLE_PRODUCT_SKU_DRAFT_PDA_SEED: &[u8] = b"oracle-product-sku-draft-v1";
/// Product-scoped rolling maturity cursor shared by every strike/market in one planned rung.
pub const ORACLE_MATURITY_LADDER_PDA_SEED: &[u8] = b"oracle-maturity-ladder-v1";
/// One month/SKU coverage counter. The SKU id is committed by the manifest Merkle root.
pub const ORACLE_SKU_COVERAGE_RECORD_PDA_SEED: &[u8] = b"oracle-sku-record-v1";
pub const ORACLE_PLAYER_LEDGER_PDA_SEED: &[u8] = b"oracle_player";
pub const ORACLE_TREASURY_PDA_SEED: &[u8] = b"oracle_treasury";
pub const ORACLE_MAJOR_TOKEN_CONFIG_PDA_SEED: &[u8] = b"oracle_major_token_config";
/// Canonical source-neutral AMBA intake account for creator-fee reward contributions.
pub const ORACLE_REWARD_FUNNEL_PDA_SEED: &[u8] = b"oracle_reward_funnel";
pub const ORACLE_STAKING_POOL_PDA_SEED: &[u8] = b"oracle_staking_pool";
pub const ORACLE_SAMBA_MINT_PDA_SEED: &[u8] = b"oracle_samba_mint";
pub const ORACLE_SAMBA_VOTE_VAULT_PDA_SEED: &[u8] = b"oracle_samba_vote_vault";
pub const ORACLE_UNSTAKE_REQUEST_PDA_SEED: &[u8] = b"oracle_unstake_request";
/// Owner-scoped AMBA principal waiting to become reward-eligible sAMBA backing.
pub const ORACLE_STAKE_ACTIVATION_PDA_SEED: &[u8] = b"oracle_stake_activation";
pub const ORACLE_ECONOMICS_CONFIG_PDA_SEED: &[u8] = b"oracle_economics_config";
pub const ORACLE_SUPPORT_POSITION_PDA_SEED: &[u8] = b"oracle_support";
pub const ORACLE_SOURCE_CHALLENGE_PDA_SEED: &[u8] = b"oracle_source_challenge";
/// Reusable active-challenge reservation for one current source.
///
/// The typed guard account persists, while terminal resolution clears its active challenge and
/// dispute bindings so a later non-overlapping challenge can reuse the same canonical PDA.
///
/// This seed is exactly 32 bytes, Solana's per-seed maximum.
pub const ORACLE_SOURCE_CHALLENGE_GUARD_PDA_SEED: &[u8] = b"oracle-source-challenge-guard-v1";
pub const ORACLE_OPENING_CLAIM_PDA_SEED: &[u8] = b"oracle_opening_claim";
pub const ORACLE_OPENING_CLAIM_CHALLENGE_PDA_SEED: &[u8] = b"oracle_opening_claim_challenge";
pub const ORACLE_UPDATE_CHALLENGE_PDA_SEED: &[u8] = b"oracle_update_challenge";
/// Permanent one-challenge-per-update-claim guard.
pub const ORACLE_UPDATE_CHALLENGE_GUARD_PDA_SEED: &[u8] = b"oracle-update-guard-v1";
/// Current sAMBA emergency-dispute address domain.
pub const ORACLE_EMERGENCY_DISPUTE_V3_PDA_SEED: &[u8] = b"oracle-emergency-dispute-v3";
/// Current sAMBA emergency-vote address domain.
pub const ORACLE_EMERGENCY_VOTE_V3_PDA_SEED: &[u8] = b"oracle-emergency-vote-v3";
/// Dispute-scoped authority whose canonical classic-SPL ATA isolates all sAMBA vote principal.
pub const ORACLE_SAMBA_EMERGENCY_POT_PDA_SEED: &[u8] = b"oracle-samba-pot-v1";
/// One permissionless, create-only winning-vote registration used to finalize pro-rata dust.
pub const ORACLE_SAMBA_WINNING_VOTE_PDA_SEED: &[u8] = b"oracle-samba-winner-v1";
/// One immutable settlement receipt for a current sAMBA emergency vote.
pub const ORACLE_SAMBA_VOTE_SETTLEMENT_PDA_SEED: &[u8] = b"oracle-samba-settle-v1";
/// Domain separator for current emergency vote commitments.
pub const ORACLE_SAMBA_VOTE_COMMITMENT_V3_DOMAIN: &[u8] = b"amoeba-oracle-samba-vote-v3";
/// A current dispute admits at most this many distinct vote records.
///
/// Each frozen minimum vote is `ceil(snapshot_sAMBA_supply / 256)`, which makes the principal
/// conservation bound itself cap successful commitments at 256 without an unbounded registry.
pub const ORACLE_MAX_V3_VOTERS: u32 = 256;
pub const ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED: &[u8] = b"oracle-recipe-weights-v2";
pub const ORACLE_SETTLEMENT_SOURCE_MANIFEST_PDA_SEED: &[u8] = b"oracle-settlement-sources-v1";
pub const ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED: &[u8] = b"oracle-active-weights-v1";
pub const ORACLE_BUCKET_MEDIAN_PDA_SEED: &[u8] = b"oracle-bucket-median-v1";
/// Canonical program-owned authority whose classic-SPL ATA holds finite USDC oracle rewards.
pub const ORACLE_USDC_REWARD_VAULT_PDA_SEED: &[u8] = b"oracle-usdc-reward-vault-v1";
/// One frozen reward schedule for a cash-economics oracle month.
pub const ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED: &[u8] = b"oracle-usdc-reward-schedule-v1";
/// One finite, fully reserved reward and bond configuration for a SKU/bucket.
pub const ORACLE_USDC_SKU_POOL_PDA_SEED: &[u8] = b"oracle-usdc-sku-pool-v1";
/// Per-source contributor counts and cash-reward registration state.
pub const ORACLE_USDC_SOURCE_REWARD_PDA_SEED: &[u8] = b"oracle-usdc-source-reward-v1";
/// Unique post-finality registration for an update or emergency dispute.
///
/// Solana limits every individual PDA seed to 32 bytes, so the human-readable
/// `oracle-usdc-reward-registration-v1` domain is deliberately split in two.
pub const ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED: &[u8] = b"oracle-usdc-reward";
pub const ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED: &[u8] = b"registration-v1";
/// Unique cash payout receipt.
pub const ORACLE_USDC_REWARD_RECEIPT_PDA_SEED: &[u8] = b"oracle-usdc-reward-receipt-v1";
pub const ORACLE_UPDATE_CLAIM_V2_PDA_SEED: &[u8] = b"oracle-update-claim-v2";
pub const ORACLE_RECIPE_WEIGHT_MANIFEST_HASH_DOMAIN: &[u8] = b"amoeba-oracle-weight-manifest-v1";
pub const ORACLE_CANONICAL_RECIPE_HASH_DOMAIN: &[u8] = b"amoeba-oracle-canonical-recipe-v1";
pub const ORACLE_SETTLEMENT_SOURCE_DIGEST_DOMAIN: &[u8] = b"amoeba-oracle-settlement-sources-v2";
pub const ORACLE_ACTIVE_WEIGHT_MANIFEST_HASH_DOMAIN: &[u8] =
    b"amoeba-oracle-active-weight-manifest-v1";
pub const ORACLE_SKU_LEAF_HASH_DOMAIN: &[u8] = b"amoeba-oracle-sku-leaf-v1";
pub const ORACLE_SKU_NODE_HASH_DOMAIN: &[u8] = b"amoeba-oracle-sku-node-v1";
/// Deterministic client-side pad leaf for non-power-of-two terminal-SKU trees.
pub const ORACLE_SKU_EMPTY_HASH_DOMAIN: &[u8] = b"amoeba-oracle-sku-empty-v1";
pub const ORACLE_UPDATE_COMMITMENT_V2_DOMAIN: &[u8] = b"ameba-oracle-update-commit-v2";
pub const ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN: &[u8] = b"ameba-oracle-update-evidence-v1";
pub const ORACLE_BUCKET_MEDIAN_HASH_DOMAIN: &[u8] = b"ameba-oracle-bucket-median-v1";
pub const ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS: u64 = 1;
pub const ORACLE_UPDATE_REVEAL_WINDOW_SLOTS: u64 = 150;
/// sAMBA burned for unstaking remains in its owner-scoped request for exactly seven days.
pub const ORACLE_SAMBA_UNBONDING_SECONDS: u64 = 7 * 24 * 60 * 60;
/// Newly queued AMBA must remain outside active backing for a full seven days. Activation mints
/// sAMBA at the then-current exchange rate, so rewards funded during this interval belong only to
/// shares that were already active.
pub const ORACLE_SAMBA_STAKE_ACTIVATION_SECONDS: u64 = 7 * 24 * 60 * 60;
/// Fixed allocation of each swept creator-fee contribution. Integer dust goes to Reserve.
pub const ORACLE_REWARD_GAME_BPS: u16 = 4_500;
pub const ORACLE_REWARD_SCRAMBLE_BPS: u16 = 2_000;
pub const ORACLE_REWARD_CHALLENGE_BPS: u16 = 1_500;
pub const ORACLE_REWARD_STAKING_BPS: u16 = 1_500;
/// Canonical classic-SPL contract mint. Light and classic-SPL accounts use this same mint
/// identity; no protocol-specific mirror mint exists.
pub const CONTRACT_MINT_PDA_SEED: &[u8] = b"contract_mint_v1";
/// Deterministic zero-balance classic-SPL bridge used only while issuing one market's contracts
/// into Light token accounts. This is a token account, not another mint.
pub const CONTRACT_MINT_STAGING_PDA_SEED: &[u8] = b"contract_mint_staging_v1";
pub const WRITER_POLICY_REGISTRY_PDA_SEED: &[u8] = b"writer_policy_registry_v1";
pub const WRITER_PROTOCOL_FEE_VAULT_PDA_SEED: &[u8] = b"writer_fee_vault_v1";
pub const WRITER_SETTLEMENT_GROUP_PDA_SEED: &[u8] = b"writer_settlement_group_v1";
pub const WRITER_SLEEVE_PDA_SEED: &[u8] = b"writer_sleeve_v1";
pub const WRITER_SERIES_BOOK_PDA_SEED: &[u8] = b"writer_series_book_v1";
pub const WRITER_POLICY_SNAPSHOT_PDA_SEED: &[u8] = b"writer_policy_snapshot_v1";
pub const WRITER_SLEEVE_USDC_VAULT_PDA_SEED: &[u8] = b"writer_sleeve_usdc_v1";
pub const WRITER_FLAT_MINT_PDA_SEED: &[u8] = b"writer_flat_mint_v1";
pub const WRITER_FLAT_STAGING_PDA_SEED: &[u8] = b"writer_flat_staging_v1";
pub const WRITER_FLAT_BURN_CUSTODY_PDA_SEED: &[u8] = b"writer_flat_burn_v1";
pub const WRITER_RETIREMENT_CUSTODY_PDA_SEED: &[u8] = b"writer_retirement_v1";
pub const WRITER_AUCTION_PDA_SEED: &[u8] = b"writer_auction_v1";
pub const WRITER_BID_INDEX_PDA_SEED: &[u8] = b"writer_bid_index_v1";
pub const WRITER_AUCTION_ESCROW_PDA_SEED: &[u8] = b"writer_auction_escrow_v1";
pub const WRITER_BID_PDA_SEED: &[u8] = b"writer_bid_v1";
pub const WRITER_CLOSE_REQUEST_PDA_SEED: &[u8] = b"writer_close_v1";
pub const WRITER_CLOSE_FLAT_ESCROW_PDA_SEED: &[u8] = b"writer_close_flat_escrow_v1";
pub const WRITER_MAX_LIVE_SERIES: usize = 20;
pub const WRITER_SERIES_STORAGE_CAPACITY: usize = 32;
pub const WRITER_MAX_CANDIDATE_POINTS: usize = 128;
pub const WRITER_MAX_FUNDED_BIDS: usize = 128;
pub const WRITER_RATIO_SCALE_PPM: u64 = 1_000_000;
pub const WRITER_ORACLE_METHODOLOGY_VERSION: u64 = 1;
pub const PRODUCTION_MIN_WRITER_POLICY_ROTATION_DELAY_SLOTS: u64 = 216_000;
pub const MIN_WRITER_POLICY_ROTATION_DELAY_SLOTS: u64 =
    PRODUCTION_MIN_WRITER_POLICY_ROTATION_DELAY_SLOTS;
pub const AMOEBA_DLMM_POOL_PDA_SEED: &[u8] = b"ameba-dlmm-pool-v1";
pub const AMOEBA_DLMM_AUTHORITY_PDA_SEED: &[u8] = b"ameba-dlmm-authority-v1";
pub const AMOEBA_DLMM_BIN_PAGE_PDA_SEED: &[u8] = b"ameba-dlmm-page-v1";
pub const AMOEBA_DLMM_SHARE_PAGE_PDA_SEED: &[u8] = b"ameba-dlmm-shares-v1";
pub const AMOEBA_DLMM_POSITION_PDA_SEED: &[u8] = b"ameba-dlmm-position-v1";
pub const AMOEBA_DLMM_VAULT_PDA_SEED: &[u8] = b"ameba-dlmm-vault-v1";
pub const AMOEBA_DLMM_BINS_PER_PAGE: usize = 32;
pub const CANONICAL_CONTRACT_SIZE_ATOMIC: u64 = 1_000_000;
pub const AMOEBA_DLMM_TICK_SIZE_QUOTE_ATOMIC: u64 = 50_000;
/// One scalar bitmap covers 64 lazily-created pages. Current markets use only
/// pages 0 through 7 (240 priced bins), so this remains generous headroom
/// without renting 48 mostly-zero bitmap words in every hot pool account.
pub const MAX_AMOEBA_DLMM_BIN_COUNT: u16 = 2_048;
pub const MAX_AMOEBA_DLMM_PAGE_COUNT: u16 = 64;
pub const AMOEBA_DLMM_PAGE_BITMAP_WORDS: usize = 16;
pub const MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES: usize = 32;
pub const MAX_AMOEBA_DLMM_BINS_PER_SWAP: u8 = 8;
pub const MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP: u8 = 8;
pub const MAX_AMOEBA_DLMM_SWAP_FEE_BPS: u16 = 1_000;
pub const MARKET_PAGE_LEAF_ADDRESS_SEED: &[u8] = b"market_page";
pub const SETTLEMENT_LEAF_ADDRESS_SEED: &[u8] = b"settlement_leaf";
/// One rulebook calendar day. SBF integration tests use a shortened clock while
/// preserving the exact 4/2/1/1 lifecycle proportions.
pub const ORACLE_CALENDAR_DAY_SECONDS: u64 = 24 * 60 * 60;
/// Scramble days 1-4: source placement and support.
pub const ORACLE_PLACEMENT_WINDOW_SECONDS: u64 = 4 * ORACLE_CALENDAR_DAY_SECONDS;
/// Scramble days 5-6: source kill/challenge.
pub const ORACLE_KILL_WINDOW_SECONDS: u64 = 2 * ORACLE_CALENDAR_DAY_SECONDS;
/// Scramble day 7: challenge resolution and immutable recipe freeze.
pub const ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS: u64 = ORACLE_CALENDAR_DAY_SECONDS;
/// Opening is outside Scramble and occupies the final day before listing.
pub const ORACLE_OPENING_WINDOW_SECONDS: u64 = ORACLE_CALENDAR_DAY_SECONDS;
pub const ORACLE_SCRAMBLE_WINDOW_SECONDS: u64 = ORACLE_PLACEMENT_WINDOW_SECONDS
    + ORACLE_KILL_WINDOW_SECONDS
    + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS;
pub const ORACLE_PRE_LISTING_WINDOW_SECONDS: u64 =
    ORACLE_SCRAMBLE_WINDOW_SECONDS + ORACLE_OPENING_WINDOW_SECONDS;
/// A newly listed far-month series remains live across the listing month and
/// the next two maturity months, expiring at the matching boundary three
/// calendar months after listing.
pub const ORACLE_ROLLING_MATURITY_MONTHS: i64 = 3;
/// Every rolling monthly series lists and expires at the protocol-wide UTC roll boundary.
/// V1 uses exactly midnight UTC; individual markets cannot choose a private time of day.
pub const ORACLE_MONTH_ROLL_SECOND_UTC: u64 = 0;
/// Canonical governed terminal-SKU count for the RAMX product manifest.
pub const RAMX_ORACLE_PRODUCT_SKU_COUNT: u16 = 52;
/// Canonical governed terminal-MPN count for the NANDX v1 product manifest.
pub const NANDX_ORACLE_PRODUCT_SKU_COUNT: u16 = 48;
/// Terminal-SKU manifests are deliberately bounded so membership proofs and account work remain
/// deterministic. Product-specific counts remain explicit and release-attested above.
pub const MAX_ORACLE_REQUIRED_SKUS: u16 = 256;
pub const MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH: usize = 8;
/// A tag-190 chunk with two governance signatures and all six required accounts remains below
/// Solana's `PACKET_DATA_SIZE`. Larger product manifests are assembled in an OPD draft.
pub const MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS: usize = 16;
/// Bounds transitive V5 source-merge proofs so a supporter claim remains transaction-sized.
pub const MAX_ORACLE_USDC_MERGE_DEPTH: u8 = 16;
/// Consensus execution bound for one median bucket. This is a transaction/computation ceiling,
/// not an administrator-selected source-count target.
/// One recompute step consumes one source-state leaf and its ordinary observation history. The
/// fixed bucket record retains at most one delta per admitted source. This is a protocol transport
/// bound, not a cash- or admin-selected composition cap.
pub const MAX_ORACLE_BUCKET_SOURCES: usize = MAX_COMPRESSED_STATE_SESSION_RECORDS;
/// Accepted opening/update prints retained for one source and one month. Hitting this bound is a
/// fail-closed error; observations are never silently evicted from the temporal median.
pub const MAX_ORACLE_SOURCE_OBSERVATIONS: usize = 32;
/// Primary and one-time extended settlement lookbacks count UTC weekdays (Monday-Friday).
pub const ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS: u8 = 5;
pub const ORACLE_SETTLEMENT_FRESHNESS_GRACE_BUSINESS_DAYS: u8 = 5;
/// Accepted prints inside the primary settlement window receive three reward units.
pub const ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER: u8 = 3;
/// Per-bucket open-interest safety factor from ORACLE V1.1. Exactly 50% is the allowed ceiling.
pub const ORACLE_OI_CAP_KAPPA_BPS: u16 = 5_000;
/// Half of the DLMM's USDC protocol-fee sleeve is reserved for month/SKU update bounties.
pub const ORACLE_BOUNTY_PROTOCOL_FEE_SHARE_BPS: u16 = 5_000;
/// Minimum liveness period during which a bonded opening claim can be challenged.
pub const ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS: u64 = 300;
pub const ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES: usize = 384;
pub const ORACLE_OPENING_ARCHIVE_URL_PREFIX: &str = "https://web.archive.org/web/";
pub const ORACLE_OPENING_EVIDENCE_HASH_DOMAIN: &[u8] = b"amoeba-oracle-opening-evidence-v1";
pub const ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN: &[u8] = b"amoeba-oracle-opening-archive-url-v1";
/// Evidence and dispute-resolution window after option expiry before the oracle month can finalize.
/// This production value is intentionally identical under `test-sbf`: tests must exercise the
/// exact signed settlement timing contract instead of a shortened test-only protocol.
pub const ORACLE_SETTLEMENT_GRACE_SECONDS: u64 = 7_200;
/// Hard ceiling checked before any instruction payload is Borsh-decoded.
pub const MAX_INSTRUCTION_DATA_BYTES: usize = 16_384;
pub const MARKET_PAGE_ITEM_ID_MAX_BYTES: usize = 32;
pub const MARKET_PAGE_NAME_MAX_BYTES: usize = 64;
pub const MARKET_PAGE_SYMBOL_MAX_BYTES: usize = 32;
pub const MARKET_PAGE_TITLE_MAX_BYTES: usize = 128;
pub const MARKET_PAGE_SUBTITLE_MAX_BYTES: usize = 160;
pub const MARKET_PAGE_INFO_HREF_MAX_BYTES: usize = 160;
pub const MARKET_PAGE_PAGE_TITLE_MAX_BYTES: usize = 160;
pub const MARKET_PAGE_META_DESCRIPTION_MAX_BYTES: usize = 280;
pub const MARKET_PAGE_EXPIRY_ID_MAX_BYTES: usize = 24;
pub const MARKET_PAGE_EXPIRY_LABEL_MAX_BYTES: usize = 64;
pub const MARKET_PAGE_MAX_EXPIRIES: usize = 24;
pub const SETTLEMENT_ITEM_ID_MAX_BYTES: usize = 32;
pub const SETTLEMENT_EXPIRY_ID_MAX_BYTES: usize = 24;
pub const SETTLEMENT_SOURCE_URI_MAX_BYTES: usize = 256;
pub const SETTLEMENT_MAX_OBSERVATIONS: usize = 5;
pub const SETTLEMENT_SIGNING_DOMAIN_MAX_BYTES: usize = 64;
pub const SETTLEMENT_SIGNER_REGISTRY_PDA_SEED: &[u8] = b"settlement_signer_registry";
pub const SETTLEMENT_SIGNER_SET_PDA_SEED: &[u8] = b"settlement_signer_set";
/// Bounded storage capacity only. The active signer count and threshold are governed state.
pub const MAX_SETTLEMENT_SIGNER_COUNT: usize = 8;
/// Production governance delay. External SBF tests must serialize this value even though their
/// host-side helper crate is compiled with `test-sbf`.
pub const PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS: u64 = 216_000;
pub const MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS: u64 =
    PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS;
pub const EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER: u64 = 2;
