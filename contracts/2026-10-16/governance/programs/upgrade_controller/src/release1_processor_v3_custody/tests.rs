use super::*;
use crate::artifact_merkle::{
    artifact_merkle_proof, artifact_merkle_root, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
};
use borsh::BorshDeserialize;

fn exact_failure_binding_fixture() -> (
    ProgramDataObservationV1,
    UpgradeProposalV3,
    ProgramDataCapacityPolicyV1,
) {
    let mut observation =
        ProgramDataObservationV1::try_from_slice(&vec![0; ProgramDataObservationV1::LEN]).unwrap();
    let mut proposal = UpgradeProposalV3::try_from_slice(&vec![0; UpgradeProposalV3::LEN]).unwrap();
    let mut capacity =
        ProgramDataCapacityPolicyV1::try_from_slice(&vec![0; ProgramDataCapacityPolicyV1::LEN])
            .unwrap();

    proposal.upgrade_executed_slot = 41;
    proposal.artifact_length = 23_451;
    proposal.artifact_sha256 = [0x11; 32];
    proposal.artifact_chunk_merkle_root = [0x22; 32];
    proposal.artifact_scheme_id = [0x33; 32];
    proposal.artifact_chunk_size = 16_384;
    proposal.artifact_chunk_count = 2;
    proposal.minimum_required_capacity = 32_768;
    capacity.observation_scheme_id = [0x44; 32];

    observation.purpose = ProgramDataObservationPurposeV1::PostUpgrade;
    observation.deployed_slot = proposal.upgrade_executed_slot;
    observation.start_slot = proposal.upgrade_executed_slot + 1;
    observation.expected_artifact_length = proposal.artifact_length;
    observation.expected_artifact_sha256 = proposal.artifact_sha256;
    observation.expected_artifact_merkle_root = proposal.artifact_chunk_merkle_root;
    observation.expected_artifact_scheme_id = proposal.artifact_scheme_id;
    observation.artifact_chunk_size = proposal.artifact_chunk_size;
    observation.artifact_chunk_count = proposal.artifact_chunk_count;
    observation.minimum_required_capacity = proposal.minimum_required_capacity;
    observation.raw_observation_scheme_id = capacity.observation_scheme_id;

    (observation, proposal, capacity)
}

#[test]
fn compact_failure_proof_authenticates_the_expected_leaf() {
    let chunk_size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
    let artifact = vec![0x5a; chunk_size + 17];
    let root = artifact_merkle_root(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    let proof = artifact_merkle_proof(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, 1).unwrap();
    let leaf = artifact_chunk_leaf_hash(1, &artifact[chunk_size..]).unwrap();
    let mut nodes = [[0; 32]; MAX_ARTIFACT_PROOF_DEPTH_V1];
    nodes[..proof.len()].copy_from_slice(&proof);
    validate_expected_leaf_proof(
        &root,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        1,
        &leaf,
        proof.len() as u8,
        &nodes,
    )
    .unwrap();

    let mut wrong_leaf = leaf;
    wrong_leaf[0] ^= 1;
    assert!(validate_expected_leaf_proof(
        &root,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        1,
        &wrong_leaf,
        proof.len() as u8,
        &nodes,
    )
    .is_err());
}

#[test]
fn zero_tail_hash_is_derived_from_exact_index_length_and_bytes() {
    let zero = vec![0; 1_037];
    let expected = programdata_zero_tail_zero_hash(3, zero.len()).unwrap();
    assert_eq!(
        expected,
        programdata_zero_tail_chunk_hash(3, &zero).unwrap()
    );

    let mut nonzero = zero.clone();
    nonzero[1_036] = 1;
    assert_ne!(
        expected,
        programdata_zero_tail_chunk_hash(3, &nonzero).unwrap()
    );
    assert_ne!(
        expected,
        programdata_zero_tail_zero_hash(4, zero.len()).unwrap()
    );
}

#[test]
fn exact_region_chunk_preserves_the_final_partial_length() {
    let payload: Vec<u8> = (0..31).collect();
    assert_eq!(
        exact_region_chunk(&payload, 3, 20, 8, 0).unwrap(),
        &payload[3..11]
    );
    assert_eq!(
        exact_region_chunk(&payload, 3, 20, 8, 2).unwrap(),
        &payload[19..23]
    );
    assert!(exact_region_chunk(&payload, 3, 20, 8, 3).is_err());
}

#[test]
fn caller_authored_wrong_failure_observations_are_rejected() {
    let (observation, proposal, capacity) = exact_failure_binding_fixture();
    validate_failure_observation_binding(&observation, &proposal, &capacity).unwrap();

    let mut wrong = observation.clone();
    wrong.expected_artifact_length += 1;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());

    let mut wrong = observation.clone();
    wrong.expected_artifact_sha256[0] ^= 1;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());

    let mut wrong = observation.clone();
    wrong.expected_artifact_merkle_root[0] ^= 1;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());

    let mut wrong = observation.clone();
    wrong.expected_artifact_scheme_id[0] ^= 1;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());

    let mut wrong = observation.clone();
    wrong.raw_observation_scheme_id[0] ^= 1;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());

    let mut wrong = observation.clone();
    wrong.deployed_slot -= 1;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());

    let mut wrong = observation.clone();
    wrong.start_slot = proposal.upgrade_executed_slot;
    assert!(validate_failure_observation_binding(&wrong, &proposal, &capacity).is_err());
}

#[test]
fn unprovable_or_restart_only_failure_classes_cannot_be_recorded() {
    for mismatch_class in [
        ProgramDataMismatchClassV2::ArtifactLength,
        ProgramDataMismatchClassV2::ObservationStale,
        ProgramDataMismatchClassV2::ObservationScheme,
    ] {
        assert!(validate_recordable_programdata_mismatch_class(mismatch_class).is_err());
    }
    for mismatch_class in [
        ProgramDataMismatchClassV2::ProgramLinkage,
        ProgramDataMismatchClassV2::Header,
        ProgramDataMismatchClassV2::Authority,
        ProgramDataMismatchClassV2::CapacityDecrease,
        ProgramDataMismatchClassV2::ArtifactPayload,
        ProgramDataMismatchClassV2::ZeroTail,
    ] {
        validate_recordable_programdata_mismatch_class(mismatch_class).unwrap();
    }
}

#[test]
fn rollback_activation_and_execution_accept_only_live_byte_mismatch_classes() {
    for mismatch_class in [
        ProgramDataMismatchClassV2::ProgramLinkage,
        ProgramDataMismatchClassV2::ProgramOwner,
        ProgramDataMismatchClassV2::ProgramExecutable,
        ProgramDataMismatchClassV2::ProgramDataOwner,
        ProgramDataMismatchClassV2::ProgramDataExecutable,
        ProgramDataMismatchClassV2::Header,
        ProgramDataMismatchClassV2::Authority,
        ProgramDataMismatchClassV2::CapacityDecrease,
        ProgramDataMismatchClassV2::CapacityAboveRuntimeMaximum,
        ProgramDataMismatchClassV2::ArtifactLength,
        ProgramDataMismatchClassV2::ObservationStale,
        ProgramDataMismatchClassV2::ObservationScheme,
    ] {
        assert!(validate_rollback_activation_mismatch_class(mismatch_class).is_err());
        assert!(!is_loader_executable_rollback_failure(mismatch_class));
    }
    for mismatch_class in [
        ProgramDataMismatchClassV2::ArtifactPayload,
        ProgramDataMismatchClassV2::ZeroTail,
    ] {
        validate_rollback_activation_mismatch_class(mismatch_class).unwrap();
        assert!(is_loader_executable_rollback_failure(mismatch_class));
    }
}
