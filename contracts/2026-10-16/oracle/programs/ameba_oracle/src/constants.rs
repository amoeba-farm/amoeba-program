/// Every program-owned PDA begins with the oracle namespace seed.
pub const CURRENT_STATE_NAMESPACE_SEED: &[u8] = ameba_oracle_interface::NAMESPACE_SEED;
/// One current-period SKU leaf and a state/descriptor pair per source.
pub const MAX_ORACLE_ACTIVE_WEIGHT_SOURCES_PER_STEP: usize = 1;
/// State records one formation step may touch: eight bucket medians per settlement-source step,
/// half as many sources per recipe-bucket step.
pub const MAX_ORACLE_STATE_RECORDS_PER_STEP: usize = 8;
/// Singleton config PDA. It is also the token authority of the bond vault.
pub const VAULT_PDA_SEED: &[u8] = b"vault_config";
pub const USER_COLLATERAL_PDA_SEED: &[u8] = b"user_collateral";
/// Period/month-scoped immutable settlement record.
pub const SETTLEMENT_V2_PDA_SEED: &[u8] = b"settlement_v2";
pub const ORACLE_MONTH_PDA_SEED: &[u8] = b"g3-oracle_month";
pub const ORACLE_SOURCE_PDA_SEED: &[u8] = b"oracle_source";
pub const ORACLE_SOURCE_OBSERVATIONS_PDA_SEED: &[u8] = b"oracle_source_observations";
/// Month-scoped immutable commitment to the complete required terminal-SKU set.
pub const ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED: &[u8] = b"oracle-sku-coverage-v1";
/// Product-scoped, create-once commitment to the complete canonical terminal-SKU set.
pub const ORACLE_PRODUCT_SKU_MANIFEST_PDA_SEED: &[u8] = b"oracle-product-skus-v1";
/// Nonce-scoped governed draft used to assemble one packet-safe product SKU manifest.
pub const ORACLE_PRODUCT_SKU_DRAFT_PDA_SEED: &[u8] = b"oracle-product-sku-draft-v1";
/// Product-scoped rolling maturity cursor shared by every period in one planned rung.
pub const ORACLE_MATURITY_LADDER_PDA_SEED: &[u8] = b"g3-oracle-maturity-ladder-v1";
/// One month/SKU coverage counter. The SKU id is committed by the manifest Merkle root.
pub const ORACLE_SKU_COVERAGE_RECORD_PDA_SEED: &[u8] = b"oracle-sku-record-v1";
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
/// Bond, reward and collateral custody uses one classic-SPL USDC mint with six decimals.
pub const COLLATERAL_MINT_DECIMALS: u8 = 6;
/// Every period this program creates publishes its settlement value with six decimals, the same
/// atomic scale as the oracle month's settlement base and the collateral mint.
pub const PERIOD_VALUE_DECIMALS: u8 = 6;
/// One rulebook calendar day. Host-side unit tests built with `test-sbf` use a shortened clock
/// while preserving the exact 4/2/1/1 lifecycle proportions.
#[cfg(not(feature = "test-sbf"))]
pub const ORACLE_CALENDAR_DAY_SECONDS: u64 = 24 * 60 * 60;
#[cfg(feature = "test-sbf")]
pub const ORACLE_CALENDAR_DAY_SECONDS: u64 = 15;
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
/// A newly listed period remains live across the listing month and the next two maturity months,
/// expiring at the matching boundary three calendar months after listing.
pub const ORACLE_ROLLING_MATURITY_MONTHS: i64 = 3;
/// Every rolling monthly period lists and expires at the protocol-wide UTC roll boundary.
/// V1 uses exactly midnight UTC; individual periods cannot choose a private time of day.
pub const ORACLE_MONTH_ROLL_SECOND_UTC: u64 = 0;
/// Terminal-SKU manifests are deliberately bounded so membership proofs and account work remain
/// deterministic. Each product registers its own count (1..=256) with its manifest.
pub const MAX_ORACLE_REQUIRED_SKUS: u16 = 256;
pub const MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH: usize = 8;
/// A tag-190 chunk with two governance signatures and all six required accounts remains below
/// Solana's `PACKET_DATA_SIZE`. Larger product manifests are assembled in an OPD draft.
pub const MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS: usize = 16;
/// Bounds transitive source-merge proofs so a supporter claim remains transaction-sized.
pub const MAX_ORACLE_USDC_MERGE_DEPTH: u8 = 16;
/// Representational bound shared with the frozen recipe's checked u16 population.
/// Membership pages and rank certificates decouple this from transaction proof limits.
pub const MAX_ORACLE_BUCKET_SOURCES: usize = u16::MAX as usize;
/// Small-bucket fast path only; larger populations use authenticated rank certificates.
pub const INLINE_ORACLE_BUCKET_MEDIAN_CAPACITY: usize = 8;
pub const ORACLE_BUCKET_MEMBERS_PER_PAGE: usize = 6;
/// Immutable first-page cache. Later observations remain in the mandatory checkpoint
/// chain and are verified through a complete, bounded streaming median walk.
pub const MAX_ORACLE_SOURCE_OBSERVATIONS: usize = 32;
/// Primary and one-time extended settlement lookbacks count UTC weekdays (Monday-Friday).
pub const ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS: u8 = 5;
pub const ORACLE_SETTLEMENT_FRESHNESS_GRACE_BUSINESS_DAYS: u8 = 5;
/// Accepted prints inside the primary settlement window receive three reward units.
pub const ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER: u8 = 3;
/// Minimum liveness period during which a bonded opening claim can be challenged.
#[cfg(not(feature = "test-sbf"))]
pub const ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS: u64 = 300;
#[cfg(feature = "test-sbf")]
pub const ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS: u64 = 3;
pub const ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES: usize = 384;
pub const ORACLE_OPENING_ARCHIVE_URL_PREFIX: &str = "https://web.archive.org/web/";
pub const ORACLE_OPENING_EVIDENCE_HASH_DOMAIN: &[u8] = b"amoeba-oracle-opening-evidence-v1";
pub const ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN: &[u8] = b"amoeba-oracle-opening-archive-url-v1";
/// Evidence and dispute-resolution window after expiry before the oracle month can finalize.
/// Tests exercise this exact production settlement timing contract instead of a shortened
/// test-only protocol.
pub const ORACLE_SETTLEMENT_GRACE_SECONDS: u64 = 86_400;
/// Production retains the full challenge day. The authorized localnet Game
/// simulation uses a 30-second challenge window; settlement grace is unchanged.
#[cfg(not(any(feature = "localnet-dev-authority", feature = "localnet-governance")))]
pub const ORACLE_UPDATE_CHALLENGE_SECONDS: u64 = 86_400;
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
pub const ORACLE_UPDATE_CHALLENGE_SECONDS: u64 = 30;
/// Only the dev profile accelerates council cases opened by the simulation.
#[cfg(feature = "localnet-dev-authority")]
pub const DEV_COUNCIL_WINDOW_SLOTS: u64 = 150;
/// Hard ceiling checked before any instruction payload is decoded.
pub const MAX_INSTRUCTION_DATA_BYTES: usize = 16_384;
pub const SETTLEMENT_ITEM_ID_MAX_BYTES: usize = 32;
pub const SETTLEMENT_EXPIRY_ID_MAX_BYTES: usize = 24;
pub const SETTLEMENT_SOURCE_URI_MAX_BYTES: usize = 256;
pub const SETTLEMENT_MAX_OBSERVATIONS: usize = 5;
pub const SETTLEMENT_SIGNING_DOMAIN_MAX_BYTES: usize = 64;
pub const SETTLEMENT_SIGNER_REGISTRY_PDA_SEED: &[u8] = b"settlement_signer_registry";
pub const SETTLEMENT_SIGNER_SET_PDA_SEED: &[u8] = b"settlement_signer_set";
/// Bounded storage capacity only. The active signer count and threshold are governed state.
pub const MAX_SETTLEMENT_SIGNER_COUNT: usize = 8;
/// Production governance delay. External SBF tests must serialize this value even when their
/// host-side helper crate uses shortened test-only protocol constants.
pub const PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS: u64 = 216_000;
#[cfg(not(feature = "test-sbf"))]
pub const MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS: u64 =
    PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS;
#[cfg(feature = "test-sbf")]
pub const MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS: u64 = 2;
pub const EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER: u64 = 2;

#[cfg(test)]
mod timing_profile_tests {
    use super::*;

    #[test]
    fn update_challenge_uses_only_explicit_localnet_profiles() {
        let expected = if cfg!(any(
            feature = "localnet-dev-authority",
            feature = "localnet-governance"
        )) {
            30
        } else {
            86_400
        };
        assert_eq!(ORACLE_UPDATE_CHALLENGE_SECONDS, expected);
        assert_eq!(ORACLE_SETTLEMENT_GRACE_SECONDS, 86_400);
    }
}
