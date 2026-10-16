use borsh::{BorshDeserialize, BorshSerialize};

use crate::fixed_codec::{FixedStateEncode, ReferenceBorsh};

use super::{
    OracleActiveWeightManifest, OracleBucketMedianState, OracleChallengeStatus, OracleConfig,
    OracleEconomicParams, OracleEconomicsConfig, OracleEscrowDisposition,
    OracleMaturityLadderRegistry, OracleMonthState, OracleOpeningClaim,
    OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OracleProductSkuDraft,
    OracleProductSkuManifest, OracleRecipeWeightManifest, OracleSettlementSourceManifest,
    OracleSkuCoverageManifest, OracleSkuCoverageRecord, OracleSourceChallenge,
    OracleSourceChallengeGuard, OracleSourceObservations, OracleSourceState, OracleSupportPosition,
    OracleUpdateChallenge, OracleUpdateChallengeGuard, OracleUpdateClaimData, OracleUpdateClaimV2,
    OracleUsdcRewardReceipt, OracleUsdcRewardRegistration, OracleUsdcRewardSchedule,
    OracleUsdcRewardVault, OracleUsdcSkuPool, OracleUsdcSourceReward, SettlementRecordV2,
    SettlementSignerRegistry, SettlementSignerSet, UserCollateral,
};
use solana_program::pubkey::Pubkey;

#[test]
fn current_fixed_state_allocations_match_their_maximum_borsh_payloads() {
    let config = OracleConfig {
        is_initialized: true,
        bump: 1,
        admin: Pubkey::new_unique(),
        oracle_authority: Pubkey::new_unique(),
        usdc_mint: Pubkey::new_unique(),
        bond_vault: Pubkey::new_unique(),
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    };
    let month = OracleMonthState {
        settlement_record: Some(Pubkey::new_unique()),
        ..OracleMonthState::default()
    };

    assert_eq!(borsh::to_vec(&config).unwrap().len(), OracleConfig::LEN);
    assert_eq!(
        borsh::to_vec(&OracleEconomicsConfig::default())
            .unwrap()
            .len(),
        OracleEconomicsConfig::LEN
    );
    assert_eq!(borsh::to_vec(&month).unwrap().len(), OracleMonthState::LEN);
    assert_eq!(
        borsh::to_vec(&UserCollateral::default()).unwrap().len(),
        UserCollateral::LEN
    );
    assert!(
        borsh::to_vec(&OracleRecipeWeightManifest::default())
            .unwrap()
            .len()
            <= OracleRecipeWeightManifest::LEN
    );
    assert!(
        borsh::to_vec(&OracleActiveWeightManifest::default())
            .unwrap()
            .len()
            <= OracleActiveWeightManifest::LEN
    );
    assert!(
        borsh::to_vec(&OracleUpdateClaimV2::default())
            .unwrap()
            .len()
            <= OracleUpdateClaimV2::LEN
    );
}

fn assert_fixed_codec_matches_borsh<T>(value: T)
where
    T: BorshSerialize
        + BorshDeserialize
        + FixedStateEncode
        + ReferenceBorsh
        + std::fmt::Debug
        + PartialEq,
{
    let encoded = borsh::to_vec(&value).expect("serialize fixed account");
    assert_eq!(encoded, value.reference_borsh_bytes());
    let mut direct = vec![0; value.maximum_encoded_len()];
    value.encode_fixed(&mut direct);
    assert_eq!(&direct[..encoded.len()], encoded);
    assert!(direct[encoded.len()..].iter().all(|byte| *byte == 0));
    assert_eq!(
        T::try_from_slice(&encoded).unwrap_or_else(|error| {
            panic!(
                "decode fixed account {} from slice: {error}",
                std::any::type_name::<T>()
            )
        }),
        value
    );
    let mut reader = std::io::Cursor::new(encoded);
    assert_eq!(
        T::deserialize_reader(&mut reader).expect("decode fixed account from reader"),
        value
    );
}

#[test]
fn fixed_account_codecs_are_byte_identical_to_borsh() {
    assert_fixed_codec_matches_borsh(OracleConfig {
        is_initialized: true,
        bump: 7,
        admin: Pubkey::new_unique(),
        oracle_authority: Pubkey::new_unique(),
        usdc_mint: Pubkey::new_unique(),
        bond_vault: Pubkey::new_unique(),
        paused: true,
        account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
        account_version: OracleConfig::ACCOUNT_VERSION,
    });
    assert_fixed_codec_matches_borsh(OracleSourceState::default());
    assert_fixed_codec_matches_borsh(OracleSourceObservations::default());
    assert_fixed_codec_matches_borsh(OracleSourceChallenge::default());
    assert_fixed_codec_matches_borsh(OracleRecipeWeightManifest::default());
    assert_fixed_codec_matches_borsh(OracleSettlementSourceManifest::default());
    assert_fixed_codec_matches_borsh(OracleActiveWeightManifest::default());
    assert_fixed_codec_matches_borsh(OracleBucketMedianState::default());
    assert_fixed_codec_matches_borsh(OracleUsdcSkuPool::default());
    assert_fixed_codec_matches_borsh(OracleUsdcSourceReward::default());
    assert_fixed_codec_matches_borsh(OracleUsdcRewardSchedule::default());
    assert_fixed_codec_matches_borsh(OracleEconomicParams::default());
    assert_fixed_codec_matches_borsh(OracleSkuCoverageManifest::default());
    assert_fixed_codec_matches_borsh(OracleUpdateChallenge::default());
    assert_fixed_codec_matches_borsh(OracleUpdateClaimData::default());
    assert_fixed_codec_matches_borsh(OracleUpdateChallengeGuard::default());
    assert_fixed_codec_matches_borsh(OracleSourceChallengeGuard::default());
    assert_fixed_codec_matches_borsh(OracleUsdcRewardRegistration::default());
    assert_fixed_codec_matches_borsh(OracleUsdcRewardReceipt::default());
    assert_fixed_codec_matches_borsh(SettlementRecordV2::default());
    assert_fixed_codec_matches_borsh(OracleSupportPosition::default());
    assert_fixed_codec_matches_borsh(OracleUsdcRewardVault::default());
    assert_fixed_codec_matches_borsh(OracleUpdateClaimV2::default());
    assert_fixed_codec_matches_borsh(SettlementSignerRegistry::default());
    assert_fixed_codec_matches_borsh(OracleProductSkuManifest::default());
    assert_fixed_codec_matches_borsh(OracleSkuCoverageRecord::default());
    assert_fixed_codec_matches_borsh(OracleMaturityLadderRegistry::default());
    assert_fixed_codec_matches_borsh(UserCollateral::default());
    assert_fixed_codec_matches_borsh(OracleEconomicsConfig::default());
    assert_fixed_codec_matches_borsh(SettlementSignerSet::default());
    assert_fixed_codec_matches_borsh(OracleProductSkuDraft::default());
    for settlement_record in [None, Some(Pubkey::new_unique())] {
        assert_fixed_codec_matches_borsh(OracleMonthState {
            settlement_record,
            ..OracleMonthState::default()
        });
    }
    assert_fixed_codec_matches_borsh(OracleOpeningClaim {
        is_initialized: true,
        bump: 19,
        // These identities are rehydrated from canonical source context, not stored in the claim.
        month: Pubkey::default(),
        source: Pubkey::default(),
        source_id: [0; 32],
        attempt: 23,
        claimant: Pubkey::new_unique(),
        opening_state: 29,
        source_time: 31,
        stake: 37,
        canonical_locator_hash: [0; 32],
        source_definition_hash: [0; 32],
        evidence_hash: [4; 32],
        archive_url_hash: [5; 32],
        submitted_slot: 41,
        challenge_deadline_slot: 43,
        status: OracleOpeningClaimStatus::Accepted,
        escrow_disposition: OracleEscrowDisposition::Transferred,
    });
    assert_fixed_codec_matches_borsh(OracleOpeningClaimChallenge {
        is_initialized: true,
        bump: 47,
        month: Pubkey::new_unique(),
        challenge_id: [6; 32],
        claim: Pubkey::new_unique(),
        claim_attempt: 53,
        source: Pubkey::new_unique(),
        source_id: [7; 32],
        challenger: Pubkey::new_unique(),
        alternative_opening_state: 59,
        alternative_source_time: 61,
        bond: 67,
        required_bond: 71,
        status: OracleChallengeStatus::Accepted,
        canonical_locator_hash: [8; 32],
        source_definition_hash: [9; 32],
        evidence_hash: [10; 32],
        archive_url_hash: [11; 32],
        rule_review_slot: 73,
        escrow_disposition: OracleEscrowDisposition::Slashed,
        account_discriminator: OracleOpeningClaimChallenge::ACCOUNT_DISCRIMINATOR,
        account_version: OracleOpeningClaimChallenge::ACCOUNT_VERSION,
        council_authority_version: 79,
    });
}

#[test]
fn fixed_account_codecs_reject_invalid_booleans_and_truncation() {
    let mut invalid_bool = borsh::to_vec(&OracleUsdcSkuPool::default()).unwrap();
    invalid_bool[0] = 2;
    assert!(OracleUsdcSkuPool::try_from_slice(&invalid_bool).is_err());
    let mut invalid_bool = borsh::to_vec(&UserCollateral::default()).unwrap();
    invalid_bool[0] = 2;
    assert!(UserCollateral::try_from_slice(&invalid_bool).is_err());
    let valid = borsh::to_vec(&OracleConfig::default()).unwrap();
    assert!(OracleConfig::try_from_slice(&valid[..valid.len() - 1]).is_err());
}

#[test]
fn current_economic_defaults_contain_only_emergency_governance_windows() {
    assert_eq!(
        OracleEconomicParams::default(),
        OracleEconomicParams {
            emergency_supermajority_bps: 6_000,
            emergency_commit_window_slots: if cfg!(feature = "localnet-governance") {
                750
            } else {
                216_000
            },
            emergency_reveal_window_slots: if cfg!(feature = "localnet-governance") {
                750
            } else {
                216_000
            },
        }
    );
    assert_eq!(
        borsh::to_vec(&OracleEconomicParams::default())
            .unwrap()
            .len(),
        OracleEconomicParams::LEN
    );
}

#[test]
fn oracle_state_allocations_cover_their_borsh_payloads() {
    let cases = [
        (
            "reward vault",
            borsh::to_vec(&OracleUsdcRewardVault::default()).expect("serialize reward vault"),
            OracleUsdcRewardVault::LEN,
        ),
        (
            "reward schedule",
            borsh::to_vec(&OracleUsdcRewardSchedule::default()).expect("serialize reward schedule"),
            OracleUsdcRewardSchedule::LEN,
        ),
        (
            "SKU pool",
            borsh::to_vec(&OracleUsdcSkuPool::default()).expect("serialize SKU pool"),
            OracleUsdcSkuPool::LEN,
        ),
        (
            "source reward",
            borsh::to_vec(&OracleUsdcSourceReward::default()).expect("serialize source reward"),
            OracleUsdcSourceReward::LEN,
        ),
        (
            "canonical source",
            borsh::to_vec(&OracleSourceState::default()).expect("serialize canonical source"),
            OracleSourceState::LEN,
        ),
        (
            "support position",
            borsh::to_vec(&OracleSupportPosition::default()).expect("serialize support position"),
            OracleSupportPosition::LEN,
        ),
        (
            "source challenge",
            borsh::to_vec(&OracleSourceChallenge::default()).expect("serialize source challenge"),
            OracleSourceChallenge::LEN,
        ),
        (
            "source challenge guard",
            borsh::to_vec(&OracleSourceChallengeGuard::default())
                .expect("serialize source challenge guard"),
            OracleSourceChallengeGuard::LEN,
        ),
        (
            "opening claim",
            borsh::to_vec(&OracleOpeningClaim::default()).expect("serialize opening claim"),
            OracleOpeningClaim::LEN,
        ),
        (
            "opening challenge",
            borsh::to_vec(&OracleOpeningClaimChallenge::default())
                .expect("serialize opening challenge"),
            OracleOpeningClaimChallenge::LEN,
        ),
        (
            "current update claim",
            borsh::to_vec(&OracleUpdateClaimV2::default()).expect("serialize current update claim"),
            OracleUpdateClaimV2::LEN,
        ),
        (
            "update challenge",
            borsh::to_vec(&OracleUpdateChallenge::default()).expect("serialize update challenge"),
            OracleUpdateChallenge::LEN,
        ),
        (
            "update challenge guard",
            borsh::to_vec(&OracleUpdateChallengeGuard::default())
                .expect("serialize update challenge guard"),
            OracleUpdateChallengeGuard::LEN,
        ),
        (
            "reward registration",
            borsh::to_vec(&OracleUsdcRewardRegistration::default())
                .expect("serialize reward registration"),
            OracleUsdcRewardRegistration::LEN,
        ),
        (
            "reward receipt",
            borsh::to_vec(&OracleUsdcRewardReceipt::default()).expect("serialize reward receipt"),
            OracleUsdcRewardReceipt::LEN,
        ),
        (
            "oracle month",
            borsh::to_vec(&OracleMonthState::default()).expect("serialize oracle month"),
            OracleMonthState::LEN,
        ),
        (
            "product SKU draft",
            borsh::to_vec(&OracleProductSkuDraft::default()).expect("serialize product SKU draft"),
            OracleProductSkuDraft::LEN,
        ),
        (
            "product SKU manifest",
            borsh::to_vec(&OracleProductSkuManifest::default())
                .expect("serialize product SKU manifest"),
            OracleProductSkuManifest::LEN,
        ),
        (
            "maturity ladder registry",
            borsh::to_vec(&OracleMaturityLadderRegistry::default())
                .expect("serialize maturity ladder registry"),
            OracleMaturityLadderRegistry::LEN,
        ),
        (
            "SKU coverage manifest",
            borsh::to_vec(&OracleSkuCoverageManifest::default())
                .expect("serialize SKU coverage manifest"),
            OracleSkuCoverageManifest::LEN,
        ),
        (
            "SKU coverage record",
            borsh::to_vec(&OracleSkuCoverageRecord::default())
                .expect("serialize SKU coverage record"),
            OracleSkuCoverageRecord::LEN,
        ),
        (
            "settlement signer registry",
            borsh::to_vec(&SettlementSignerRegistry::default()).expect("serialize registry"),
            SettlementSignerRegistry::LEN,
        ),
        (
            "settlement signer set",
            borsh::to_vec(&SettlementSignerSet::default()).expect("serialize signer set"),
            SettlementSignerSet::LEN,
        ),
        (
            "settlement record",
            borsh::to_vec(&SettlementRecordV2::default()).expect("serialize settlement record"),
            SettlementRecordV2::LEN,
        ),
    ];

    for (label, payload, allocation) in cases {
        assert!(
            payload.len() <= allocation,
            "{label} serializes to {} bytes but allocates only {allocation}",
            payload.len()
        );
    }
}

#[test]
fn product_sku_draft_has_one_exact_fixed_frontier_allocation() {
    for expected_sku_count in [1u16, 48, 52, crate::constants::MAX_ORACLE_REQUIRED_SKUS] {
        let mut draft = OracleProductSkuDraft {
            is_initialized: true,
            account_discriminator: OracleProductSkuDraft::ACCOUNT_DISCRIMINATOR,
            account_version: OracleProductSkuDraft::ACCOUNT_VERSION,
            expected_sku_count,
            expected_sku_root: [3; 32],
            appended_sku_count: expected_sku_count,
            frontier_mask: expected_sku_count,
            last_sku_id: [7; 32],
            ..OracleProductSkuDraft::default()
        };
        for (level, node) in draft.merkle_frontier.iter_mut().enumerate() {
            if draft.frontier_mask & (1u16 << level) != 0 {
                *node = [7; 32];
            }
        }
        assert!(draft.has_canonical_layout());
        assert_eq!(
            borsh::to_vec(&draft)
                .expect("serialize fixed-frontier product SKU draft")
                .len(),
            OracleProductSkuDraft::LEN
        );
    }
}

#[test]
fn product_sku_draft_rejects_noncanonical_frontier_occupancy() {
    let mut draft = OracleProductSkuDraft {
        is_initialized: true,
        account_discriminator: OracleProductSkuDraft::ACCOUNT_DISCRIMINATOR,
        account_version: OracleProductSkuDraft::ACCOUNT_VERSION,
        expected_sku_count: 52,
        expected_sku_root: [4; 32],
        appended_sku_count: 2,
        frontier_mask: 2,
        last_sku_id: [2; 32],
        ..OracleProductSkuDraft::default()
    };
    draft.merkle_frontier[1] = [9; 32];
    assert!(draft.has_canonical_layout());

    let mut wrong_mask = draft.clone();
    wrong_mask.frontier_mask = 1;
    assert!(!wrong_mask.has_canonical_layout());
    let mut stray_node = draft.clone();
    stray_node.merkle_frontier[0] = [9; 32];
    assert!(!stray_node.has_canonical_layout());
    let mut missing_last = draft.clone();
    missing_last.last_sku_id = [0; 32];
    assert!(!missing_last.has_canonical_layout());
    let mut missing_root = draft.clone();
    missing_root.expected_sku_root = [0; 32];
    assert!(!missing_root.has_canonical_layout());
    for wrong_version in [0, 1, 2, 4] {
        let mut stale = draft.clone();
        stale.account_version = wrong_version;
        assert!(!stale.has_canonical_layout());
    }

    let encoded = borsh::to_vec(&draft).expect("serialize fixed-frontier product SKU draft");
    assert_eq!(encoded.len(), OracleProductSkuDraft::LEN);
    assert!(OracleProductSkuDraft::try_from_slice(&encoded[..encoded.len() - 1]).is_err());
}

#[test]
fn month_layout_accepts_only_the_current_schedule_generation() {
    let current = OracleMonthState {
        is_initialized: true,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        ..OracleMonthState::default()
    };
    assert!(current.has_current_layout());
    for schedule_version in [0u8, 1, 3, 4, 5, u8::MAX] {
        let other = OracleMonthState {
            schedule_version,
            ..current.clone()
        };
        assert!(!other.has_current_layout(), "schedule {schedule_version}");
    }
    for discriminator in [*b"OMT", *b"OMX", [0; 3]] {
        let other = OracleMonthState {
            account_discriminator: discriminator,
            ..current.clone()
        };
        assert!(!other.has_current_layout());
    }
    let other = OracleMonthState {
        work_reward_currency_version: 0,
        ..current.clone()
    };
    assert!(!other.has_current_layout());
}

#[test]
fn sku_coverage_manifest_accepts_only_its_own_discriminator() {
    let manifest = OracleSkuCoverageManifest {
        account_discriminator: OracleSkuCoverageManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSkuCoverageManifest::ACCOUNT_VERSION,
        ..OracleSkuCoverageManifest::default()
    };
    assert!(manifest.has_canonical_layout());
    for discriminator in [*b"OSX", *b"OSK", [0; 3]] {
        let other = OracleSkuCoverageManifest {
            account_discriminator: discriminator,
            ..manifest.clone()
        };
        assert!(!other.has_canonical_layout());
    }
}

#[test]
fn governed_localnet_defaults_do_not_rewrite_stored_month_terms() {
    let production = OracleEconomicParams::production();
    assert_eq!(production.emergency_commit_window_slots, 216_000);
    assert_eq!(production.emergency_reveal_window_slots, 216_000);
    let development = OracleEconomicParams::governed_localnet();
    assert_eq!(development.emergency_commit_window_slots, 750);
    assert_eq!(development.emergency_reveal_window_slots, 750);
    assert_eq!(
        OracleEconomicParams::default(),
        if cfg!(feature = "localnet-governance") {
            development.clone()
        } else {
            production.clone()
        }
    );
    let existing = OracleMonthState {
        economics: production,
        ..OracleMonthState::default()
    };
    let bytes = existing.try_to_vec().unwrap();
    assert_eq!(
        OracleMonthState::try_from_slice(&bytes).unwrap().economics,
        existing.economics
    );
    assert_ne!(existing.economics, development);
}
