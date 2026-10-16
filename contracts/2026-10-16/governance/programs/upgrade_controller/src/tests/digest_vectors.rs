use crate::{
    digest::{canonical_proposal_digest_material, compute_proposal_digest},
    proposal::validate_proposal_static,
    state::{OptionalPubkeyV1, ProposalClassV1, ProposalStateV1},
};

use super::support::{key, proposal};

fn assert_digest_changes(mutator: impl FnOnce(&mut crate::state::UpgradeProposalV1)) {
    let baseline = proposal();
    let baseline_digest = compute_proposal_digest(&baseline).unwrap();
    let mut changed = baseline;
    mutator(&mut changed);
    assert_ne!(compute_proposal_digest(&changed).unwrap(), baseline_digest);
}

#[test]
fn canonical_proposal_is_internally_valid() {
    assert_eq!(validate_proposal_static(&proposal()), Ok(()));
    assert_eq!(
        canonical_proposal_digest_material(&proposal())
            .unwrap()
            .len(),
        crate::digest::PROPOSAL_DIGEST_MATERIAL_LEN
    );
}

#[test]
fn digest_covers_every_security_commitment_category() {
    assert_digest_changes(|p| p.cluster_domain[0] ^= 1);
    assert_digest_changes(|p| p.controller_program = key(60));
    assert_digest_changes(|p| p.controller_config = key(61));
    assert_digest_changes(|p| p.protocol_gate = key(62));
    assert_digest_changes(|p| p.policy_version += 1);
    assert_digest_changes(|p| p.policy_hash[0] ^= 1);
    assert_digest_changes(|p| p.target_program = key(63));
    assert_digest_changes(|p| p.target_programdata = key(64));
    assert_digest_changes(|p| p.upgradeable_loader = key(65));
    assert_digest_changes(|p| p.authority_pda = key(66));
    assert_digest_changes(|p| p.canonical_spill_treasury = key(67));
    assert_digest_changes(|p| p.proposal_id += 1);
    assert_digest_changes(|p| p.target_nonce += 1);
    assert_digest_changes(|p| p.proposal_class = ProposalClassV1::EmergencyRollback);
    assert_digest_changes(|p| p.council_version += 1);
    assert_digest_changes(|p| p.council_hash[0] ^= 1);
    assert_digest_changes(|p| p.creation_gate_epoch += 1);
    assert_digest_changes(|p| p.freeze_gate_epoch += 1);
    assert_digest_changes(|p| p.buffer_pubkey = key(68));
    assert_digest_changes(|p| p.buffer_loader_owner = key(69));
    assert_digest_changes(|p| p.buffer_authority = key(70));
    assert_digest_changes(|p| p.artifact_length += 1);
    assert_digest_changes(|p| p.artifact_sha256[0] ^= 1);
    assert_digest_changes(|p| p.source_commit_hash[0] ^= 1);
    assert_digest_changes(|p| p.source_tree_hash[0] ^= 1);
    assert_digest_changes(|p| p.build_input_inventory_hash[0] ^= 1);
    assert_digest_changes(|p| p.reproducible_build_receipt_hash[0] ^= 1);
    assert_digest_changes(|p| p.package_receipt_hash[0] ^= 1);
    assert_digest_changes(|p| p.release_intent_hash[0] ^= 1);
    assert_digest_changes(|p| p.current_deployed_payload_hash[0] ^= 1);
    assert_digest_changes(|p| p.current_raw_programdata_hash[0] ^= 1);
    assert_digest_changes(|p| p.deployed_slot += 1);
    assert_digest_changes(|p| p.current_capacity += 1);
    assert_digest_changes(|p| p.extension_delta += 1);
    assert_digest_changes(|p| p.expected_post_capacity += 1);
    assert_digest_changes(|p| p.prestate_checkpoint = key(71));
    assert_digest_changes(|p| p.required_poststate_checkpoint = key(72));
    assert_digest_changes(|p| {
        p.rollback_proposal = OptionalPubkeyV1::some(key(73)).unwrap();
    });
    assert_digest_changes(|p| {
        p.rollback_buffer = OptionalPubkeyV1::some(key(74)).unwrap();
    });
    assert_digest_changes(|p| {
        p.rollback_artifact_hash = [75; 32];
    });
    assert_digest_changes(|p| p.vote_program = key(76));
    assert_digest_changes(|p| p.vote_result_pda = key(77));
    assert_digest_changes(|p| p.vote_requirement = crate::state::VoteRequirementV1::Affirmative);
    assert_digest_changes(|p| p.review_start_slot += 1);
    assert_digest_changes(|p| p.review_end_slot += 1);
    assert_digest_changes(|p| p.not_before_slot += 1);
    assert_digest_changes(|p| p.expiry_slot += 1);
}

#[test]
fn lifecycle_and_approval_accumulators_are_not_part_of_static_digest() {
    let baseline = proposal();
    let digest = compute_proposal_digest(&baseline).unwrap();
    let mut changed = baseline;
    changed.state = ProposalStateV1::Frozen;
    changed.council_approval_bitset = 0b11100;
    changed.council_approval_count = 3;
    changed.poststate_approval_bitset = 0b11100;
    changed.poststate_approval_count = 3;
    changed.unfreeze_approval_bitset = 0b11101;
    changed.unfreeze_approval_count = 4;
    changed.cancellation_reason_code = 123;
    changed.terminal_reason_code = 456;
    changed.proposal_digest = [99; 32];
    assert_eq!(compute_proposal_digest(&changed).unwrap(), digest);
}
