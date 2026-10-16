//! Canonical Release 1 digest preimages.
//!
//! Large fixed account schemas are serialized into heap-backed vectors here so
//! SBF callers do not need a 1.4 KiB local array in a single stack frame.

use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    release1_state::{
        CheckpointAttestationV1, CouncilRotationProposalV1, EmergencyFreezeObservationV1,
        EmergencyFreezeResolutionV1, ProgramDataFailureObservationV1, StateCheckpointV1,
        UpgradeProposalV2,
    },
    state::OptionalPubkeyV1,
    GovernanceError, GovernanceResult,
};

pub const PROPOSAL_DIGEST_DOMAIN_V2: &[u8] = b"AMOEBA_UPGRADE_PROPOSAL_V2";
pub const PROPOSAL_DIGEST_MATERIAL_LEN_V2: usize = 1_416;
pub const PROPOSAL_DIGEST_PREIMAGE_LEN_V2: usize =
    PROPOSAL_DIGEST_DOMAIN_V2.len() + PROPOSAL_DIGEST_MATERIAL_LEN_V2;

pub const STATE_CHECKPOINT_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_STATE_CHECKPOINT_V1";
pub const STATE_CHECKPOINT_DIGEST_MATERIAL_LEN_V1: usize = 573;
pub const STATE_CHECKPOINT_DIGEST_PREIMAGE_LEN_V1: usize =
    STATE_CHECKPOINT_DIGEST_DOMAIN_V1.len() + STATE_CHECKPOINT_DIGEST_MATERIAL_LEN_V1;
pub const STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1: &[u8] = b"AMOEBA_CHECKPOINT_HARD_ROOT_V1";
pub const STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1: usize = 176;
pub const STATE_CHECKPOINT_HARD_ROOT_PREIMAGE_LEN_V1: usize =
    STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1.len() + STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1;

pub const COUNCIL_ROTATION_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_COUNCIL_ROTATION_V1";
pub const COUNCIL_ROTATION_DIGEST_MATERIAL_LEN_V1: usize = 240;
pub const COUNCIL_ROTATION_DIGEST_PREIMAGE_LEN_V1: usize =
    COUNCIL_ROTATION_DIGEST_DOMAIN_V1.len() + COUNCIL_ROTATION_DIGEST_MATERIAL_LEN_V1;

pub const EMERGENCY_RESOLUTION_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_EMERGENCY_RESOLUTION_V1";
pub const EMERGENCY_RESOLUTION_DIGEST_MATERIAL_LEN_V1: usize = 442;
pub const EMERGENCY_RESOLUTION_DIGEST_PREIMAGE_LEN_V1: usize =
    EMERGENCY_RESOLUTION_DIGEST_DOMAIN_V1.len() + EMERGENCY_RESOLUTION_DIGEST_MATERIAL_LEN_V1;

pub const EMERGENCY_FREEZE_OBSERVATION_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_EMERGENCY_FREEZE_OBSERVATION_V1";
pub const EMERGENCY_FREEZE_OBSERVATION_DIGEST_MATERIAL_LEN_V1: usize = 450;
pub const EMERGENCY_FREEZE_OBSERVATION_DIGEST_PREIMAGE_LEN_V1: usize =
    EMERGENCY_FREEZE_OBSERVATION_DIGEST_DOMAIN_V1.len()
        + EMERGENCY_FREEZE_OBSERVATION_DIGEST_MATERIAL_LEN_V1;

pub const PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_PROGRAMDATA_FAILURE_OBSERVATION_V1";
pub const PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_MATERIAL_LEN_V1: usize = 444;
pub const PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_PREIMAGE_LEN_V1: usize =
    PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_DOMAIN_V1.len()
        + PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_MATERIAL_LEN_V1;

pub const CHECKPOINT_ATTESTATION_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_CHECKPOINT_ATTESTATION_V1";
pub const CHECKPOINT_ATTESTATION_DIGEST_MATERIAL_LEN_V1: usize = 314;
pub const CHECKPOINT_ATTESTATION_DIGEST_PREIMAGE_LEN_V1: usize =
    CHECKPOINT_ATTESTATION_DIGEST_DOMAIN_V1.len() + CHECKPOINT_ATTESTATION_DIGEST_MATERIAL_LEN_V1;

/// Exact immutable V2 proposal commitments. Mutable state, lifecycle slots,
/// approval accumulators, reason codes, stored digest, and account header are
/// deliberately excluded.
pub fn canonical_proposal_digest_material_v2(
    proposal: &UpgradeProposalV2,
) -> GovernanceResult<Vec<u8>> {
    proposal.primary_proposal.validate()?;
    proposal.rollback_proposal.validate()?;
    proposal.rollback_buffer.validate()?;

    let mut out = Vec::with_capacity(PROPOSAL_DIGEST_MATERIAL_LEN_V2);
    put_u8(&mut out, proposal.proposal_class as u8);
    put_u8(&mut out, proposal.creation_gate_status as u8);
    put_bool(&mut out, proposal.zero_tail_required);
    put_u8(&mut out, proposal.proposal_flags);
    put_u64(&mut out, proposal.proposal_id);
    put_u64(&mut out, proposal.target_nonce);
    put_u64(&mut out, proposal.creation_slot);
    put_bytes(&mut out, &proposal.cluster_domain);
    put_pubkey(&mut out, &proposal.controller_program);
    put_pubkey(&mut out, &proposal.controller_config);
    put_pubkey(&mut out, &proposal.protocol_gate);
    put_u64(&mut out, proposal.policy_version);
    put_bytes(&mut out, &proposal.policy_hash);
    put_u64(&mut out, proposal.creation_council_version);
    put_bytes(&mut out, &proposal.creation_council_hash);
    put_u64(&mut out, proposal.creation_gate_epoch);
    // The exact frozen epoch is established only when the target nonce is
    // consumed.  It is mutable lifecycle evidence and is bound by every
    // frozen-or-later approval/instruction, not by the immutable creation
    // digest.
    put_pubkey(&mut out, &proposal.target_program);
    put_pubkey(&mut out, &proposal.target_programdata);
    put_pubkey(&mut out, &proposal.upgradeable_loader);
    put_pubkey(&mut out, &proposal.authority_pda);
    put_pubkey(&mut out, &proposal.canonical_spill_treasury);
    put_pubkey(&mut out, &proposal.buffer_pubkey);
    put_pubkey(&mut out, &proposal.buffer_loader_owner);
    put_pubkey(&mut out, &proposal.buffer_uploader_authority);
    put_pubkey(&mut out, &proposal.buffer_final_authority);
    put_pubkey(&mut out, &proposal.buffer_verification);
    put_pubkey(&mut out, &proposal.programdata_verification);
    put_u64(&mut out, proposal.artifact_length);
    put_bytes(&mut out, &proposal.artifact_sha256);
    put_bytes(&mut out, &proposal.artifact_chunk_merkle_root);
    put_bytes(&mut out, &proposal.chunk_hash_domain);
    put_u32(&mut out, proposal.chunk_size);
    put_u32(&mut out, proposal.chunk_count);
    put_bytes(&mut out, &proposal.source_commit_hash);
    put_bytes(&mut out, &proposal.source_tree_hash);
    put_bytes(&mut out, &proposal.build_input_inventory_hash);
    put_bytes(&mut out, &proposal.reproducible_build_receipt_hash);
    put_bytes(&mut out, &proposal.package_receipt_hash);
    put_bytes(&mut out, &proposal.release_intent_hash);
    put_bytes(&mut out, &proposal.expected_execution_pre_payload_hash);
    put_bytes(&mut out, &proposal.expected_execution_pre_chunk_root);
    put_bytes(&mut out, &proposal.current_raw_programdata_hash);
    put_u64(&mut out, proposal.deployed_slot);
    put_u64(&mut out, proposal.current_capacity);
    put_u64(&mut out, proposal.extension_delta);
    put_u64(&mut out, proposal.expected_post_capacity);
    put_pubkey(&mut out, &proposal.prestate_checkpoint);
    put_pubkey(&mut out, &proposal.required_poststate_checkpoint);
    put_bytes(&mut out, &proposal.checkpoint_schema_id);
    put_bytes(&mut out, &proposal.checkpoint_policy_hash);
    put_optional_pubkey(&mut out, &proposal.primary_proposal);
    put_optional_pubkey(&mut out, &proposal.rollback_proposal);
    put_optional_pubkey(&mut out, &proposal.rollback_buffer);
    put_bytes(&mut out, &proposal.rollback_artifact_sha256);
    put_bytes(&mut out, &proposal.rollback_artifact_chunk_root);
    put_u8(&mut out, proposal.vote_requirement as u8);
    put_pubkey(&mut out, &proposal.vote_program);
    put_pubkey(&mut out, &proposal.vote_result_pda);
    put_u64(&mut out, proposal.review_start_slot);
    put_u64(&mut out, proposal.review_end_slot);
    put_u64(&mut out, proposal.not_before_slot);
    put_u64(&mut out, proposal.expiry_slot);
    finish_material(out, PROPOSAL_DIGEST_MATERIAL_LEN_V2)
}

pub fn compute_proposal_digest_v2(proposal: &UpgradeProposalV2) -> GovernanceResult<[u8; 32]> {
    let material = canonical_proposal_digest_material_v2(proposal)?;
    Ok(hashv(&[PROPOSAL_DIGEST_DOMAIN_V2, &material]).to_bytes())
}

pub fn validate_proposal_digest_v2(proposal: &UpgradeProposalV2) -> GovernanceResult<()> {
    proposal.validate_schema()?;
    if compute_proposal_digest_v2(proposal)? != proposal.proposal_digest {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn canonical_state_checkpoint_digest_material_v1(
    checkpoint: &StateCheckpointV1,
) -> GovernanceResult<Vec<u8>> {
    let mut out = Vec::with_capacity(STATE_CHECKPOINT_DIGEST_MATERIAL_LEN_V1);
    put_u8(&mut out, checkpoint.phase as u8);
    put_pubkey(&mut out, &checkpoint.controller_config);
    put_pubkey(&mut out, &checkpoint.proposal);
    put_pubkey(&mut out, &checkpoint.emergency_resolution);
    put_bytes(&mut out, &checkpoint.subject_digest);
    put_pubkey(&mut out, &checkpoint.target_program);
    put_pubkey(&mut out, &checkpoint.target_programdata);
    put_u64(&mut out, checkpoint.finalized_observation_slot);
    put_u64(&mut out, checkpoint.gate_epoch);
    put_u64(&mut out, checkpoint.target_programdata_slot);
    put_bytes(&mut out, &checkpoint.target_payload_commitment);
    put_bytes(&mut out, &checkpoint.target_raw_programdata_commitment);
    put_u64(&mut out, checkpoint.target_capacity);
    put_bytes(&mut out, &checkpoint.program_owned_state_root);
    put_u64(&mut out, checkpoint.program_owned_state_count);
    put_bytes(&mut out, &checkpoint.logical_compressed_state_root);
    put_u64(&mut out, checkpoint.logical_compressed_state_count);
    put_bytes(&mut out, &checkpoint.semantic_custody_accounting_root);
    put_bytes(&mut out, &checkpoint.hard_combined_root);
    put_bytes(&mut out, &checkpoint.external_metadata_observation_root);
    put_bytes(&mut out, &checkpoint.external_raw_balance_observation_root);
    put_bytes(&mut out, &checkpoint.schema_identifier);
    put_bytes(&mut out, &checkpoint.admitted_positive_donation_root);
    put_u64(&mut out, checkpoint.admitted_positive_donation_count);
    put_u32(&mut out, checkpoint.forbidden_drift_count);
    finish_material(out, STATE_CHECKPOINT_DIGEST_MATERIAL_LEN_V1)
}

pub fn canonical_state_checkpoint_hard_root_material_v1(
    checkpoint: &StateCheckpointV1,
) -> GovernanceResult<Vec<u8>> {
    let mut out = Vec::with_capacity(STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1);
    put_bytes(&mut out, &checkpoint.schema_identifier);
    put_bytes(&mut out, &checkpoint.program_owned_state_root);
    put_u64(&mut out, checkpoint.program_owned_state_count);
    put_bytes(&mut out, &checkpoint.logical_compressed_state_root);
    put_u64(&mut out, checkpoint.logical_compressed_state_count);
    put_bytes(&mut out, &checkpoint.semantic_custody_accounting_root);
    put_bytes(&mut out, &checkpoint.external_metadata_observation_root);
    finish_material(out, STATE_CHECKPOINT_HARD_ROOT_MATERIAL_LEN_V1)
}

pub fn compute_state_checkpoint_hard_combined_root_v1(
    checkpoint: &StateCheckpointV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_state_checkpoint_hard_root_material_v1(checkpoint)?;
    Ok(hashv(&[STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_state_checkpoint_hard_combined_root_v1(
    checkpoint: &StateCheckpointV1,
) -> GovernanceResult<()> {
    if compute_state_checkpoint_hard_combined_root_v1(checkpoint)? != checkpoint.hard_combined_root
    {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn compute_state_checkpoint_digest_v1(
    checkpoint: &StateCheckpointV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_state_checkpoint_digest_material_v1(checkpoint)?;
    Ok(hashv(&[STATE_CHECKPOINT_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_state_checkpoint_digest_v1(checkpoint: &StateCheckpointV1) -> GovernanceResult<()> {
    checkpoint.validate_schema()?;
    validate_state_checkpoint_hard_combined_root_v1(checkpoint)?;
    if compute_state_checkpoint_digest_v1(checkpoint)? != checkpoint.checkpoint_digest {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn canonical_council_rotation_digest_material_v1(
    rotation: &CouncilRotationProposalV1,
) -> GovernanceResult<Vec<u8>> {
    let mut out = Vec::with_capacity(COUNCIL_ROTATION_DIGEST_MATERIAL_LEN_V1);
    put_pubkey(&mut out, &rotation.controller_config);
    put_pubkey(&mut out, &rotation.target_program);
    put_pubkey(&mut out, &rotation.current_council);
    put_u64(&mut out, rotation.current_council_version);
    put_bytes(&mut out, &rotation.current_council_hash);
    put_pubkey(&mut out, &rotation.candidate_council);
    put_u64(&mut out, rotation.candidate_council_version);
    put_bytes(&mut out, &rotation.candidate_council_hash);
    put_u64(&mut out, rotation.creation_slot);
    put_u64(&mut out, rotation.not_before_slot);
    put_u64(&mut out, rotation.expiry_slot);
    put_u64(&mut out, rotation.target_nonce);
    finish_material(out, COUNCIL_ROTATION_DIGEST_MATERIAL_LEN_V1)
}

pub fn compute_council_rotation_digest_v1(
    rotation: &CouncilRotationProposalV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_council_rotation_digest_material_v1(rotation)?;
    Ok(hashv(&[COUNCIL_ROTATION_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_council_rotation_digest_v1(
    rotation: &CouncilRotationProposalV1,
) -> GovernanceResult<()> {
    rotation.validate_schema()?;
    if compute_council_rotation_digest_v1(rotation)? != rotation.rotation_digest {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn canonical_emergency_resolution_digest_material_v1(
    resolution: &EmergencyFreezeResolutionV1,
) -> GovernanceResult<Vec<u8>> {
    let mut out = Vec::with_capacity(EMERGENCY_RESOLUTION_DIGEST_MATERIAL_LEN_V1);
    put_u8(&mut out, resolution.resolution_kind as u8);
    put_pubkey(&mut out, &resolution.controller_config);
    put_pubkey(&mut out, &resolution.protocol_gate);
    put_pubkey(&mut out, &resolution.target_program);
    put_pubkey(&mut out, &resolution.target_programdata);
    put_pubkey(&mut out, &resolution.emergency_freeze_observation);
    put_u64(&mut out, resolution.frozen_epoch);
    put_u64(&mut out, resolution.freeze_slot);
    put_u16(&mut out, resolution.freeze_reason_code);
    put_u64(&mut out, resolution.creation_slot);
    put_u64(&mut out, resolution.not_before_slot);
    put_u64(&mut out, resolution.expiry_slot);
    put_u64(&mut out, resolution.target_nonce);
    put_pubkey(&mut out, &resolution.observed_program_owner);
    put_bool(&mut out, resolution.observed_program_executable);
    put_u64(&mut out, resolution.observed_program_data_length);
    put_bool(&mut out, resolution.observed_program_header_present);
    put_optional_pubkey(&mut out, &resolution.observed_linked_programdata);
    put_pubkey(&mut out, &resolution.observed_programdata_owner);
    put_bool(&mut out, resolution.observed_programdata_executable);
    put_u64(&mut out, resolution.observed_programdata_data_length);
    put_bool(&mut out, resolution.observed_programdata_header_present);
    put_u64(&mut out, resolution.observed_programdata_slot);
    put_bool(&mut out, resolution.observed_raw_hash_complete);
    put_bytes(&mut out, &resolution.observed_raw_programdata_hash);
    put_u64(&mut out, resolution.observed_capacity);
    put_optional_pubkey(&mut out, &resolution.observed_authority);
    put_pubkey(&mut out, &resolution.emergency_checkpoint);
    finish_material(out, EMERGENCY_RESOLUTION_DIGEST_MATERIAL_LEN_V1)
}

pub fn compute_emergency_resolution_digest_v1(
    resolution: &EmergencyFreezeResolutionV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_emergency_resolution_digest_material_v1(resolution)?;
    Ok(hashv(&[EMERGENCY_RESOLUTION_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_emergency_resolution_digest_v1(
    resolution: &EmergencyFreezeResolutionV1,
) -> GovernanceResult<()> {
    resolution.validate_schema()?;
    if compute_emergency_resolution_digest_v1(resolution)? != resolution.resolution_digest {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn canonical_emergency_freeze_observation_digest_material_v1(
    observation: &EmergencyFreezeObservationV1,
) -> GovernanceResult<Vec<u8>> {
    let mut out = Vec::with_capacity(EMERGENCY_FREEZE_OBSERVATION_DIGEST_MATERIAL_LEN_V1);
    put_pubkey(&mut out, &observation.controller_program);
    put_pubkey(&mut out, &observation.controller_config);
    put_pubkey(&mut out, &observation.protocol_gate);
    put_pubkey(&mut out, &observation.target_program);
    put_pubkey(&mut out, &observation.target_programdata);
    put_pubkey(&mut out, &observation.upgradeable_loader);
    put_pubkey(&mut out, &observation.controller_authority);
    put_u64(&mut out, observation.frozen_epoch);
    put_u64(&mut out, observation.freeze_slot);
    put_u16(&mut out, observation.freeze_reason_code);
    put_pubkey(&mut out, &observation.actual_program_owner);
    put_bool(&mut out, observation.actual_program_executable);
    put_u64(&mut out, observation.actual_program_data_length);
    put_bool(&mut out, observation.program_header_present);
    put_optional_pubkey(&mut out, &observation.actual_linked_programdata);
    put_pubkey(&mut out, &observation.actual_programdata_owner);
    put_bool(&mut out, observation.actual_programdata_executable);
    put_u64(&mut out, observation.actual_programdata_data_length);
    put_bool(&mut out, observation.programdata_header_present);
    put_u64(&mut out, observation.deployed_programdata_slot);
    put_bool(&mut out, observation.raw_hash_complete);
    put_bytes(&mut out, &observation.raw_programdata_sha256);
    put_u64(&mut out, observation.capacity);
    put_optional_pubkey(&mut out, &observation.observed_authority);
    put_bool(&mut out, observation.finalized);
    put_u64(&mut out, observation.finalized_slot);
    finish_material(out, EMERGENCY_FREEZE_OBSERVATION_DIGEST_MATERIAL_LEN_V1)
}

pub fn compute_emergency_freeze_observation_digest_v1(
    observation: &EmergencyFreezeObservationV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_emergency_freeze_observation_digest_material_v1(observation)?;
    Ok(hashv(&[EMERGENCY_FREEZE_OBSERVATION_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_emergency_freeze_observation_digest_v1(
    observation: &EmergencyFreezeObservationV1,
) -> GovernanceResult<()> {
    observation.validate_schema()?;
    if compute_emergency_freeze_observation_digest_v1(observation)?
        != observation.observation_digest
    {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn canonical_programdata_failure_observation_digest_material_v1(
    observation: &ProgramDataFailureObservationV1,
) -> GovernanceResult<Vec<u8>> {
    observation.actual_authority.validate()?;
    let mut out = Vec::with_capacity(PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_MATERIAL_LEN_V1);
    put_pubkey(&mut out, &observation.controller_config);
    put_pubkey(&mut out, &observation.protocol_gate);
    put_pubkey(&mut out, &observation.primary_proposal);
    put_pubkey(&mut out, &observation.target_program);
    put_pubkey(&mut out, &observation.target_programdata);
    put_u64(&mut out, observation.frozen_epoch);
    put_pubkey(&mut out, &observation.actual_program_owner);
    put_bool(&mut out, observation.actual_program_executable);
    put_u64(&mut out, observation.actual_program_data_length);
    put_bool(&mut out, observation.program_header_present);
    put_optional_pubkey(&mut out, &observation.actual_linked_programdata);
    put_bool(&mut out, observation.raw_hash_complete);
    put_bytes(&mut out, &observation.actual_raw_programdata_sha256);
    put_pubkey(&mut out, &observation.actual_owner);
    put_bool(&mut out, observation.actual_executable);
    put_u64(&mut out, observation.actual_data_length);
    put_bool(&mut out, observation.programdata_header_present);
    put_u64(&mut out, observation.actual_programdata_slot);
    put_u64(&mut out, observation.actual_capacity);
    put_optional_pubkey(&mut out, &observation.actual_authority);
    put_u8(&mut out, observation.mismatch_class as u8);
    put_u32(&mut out, observation.failing_chunk_index);
    put_bytes(&mut out, &observation.expected_leaf_hash);
    put_bytes(&mut out, &observation.actual_leaf_hash);
    put_u64(&mut out, observation.finalized_slot);
    finish_material(out, PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_MATERIAL_LEN_V1)
}

pub fn compute_programdata_failure_observation_digest_v1(
    observation: &ProgramDataFailureObservationV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_programdata_failure_observation_digest_material_v1(observation)?;
    Ok(hashv(&[PROGRAMDATA_FAILURE_OBSERVATION_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_programdata_failure_observation_digest_v1(
    observation: &ProgramDataFailureObservationV1,
) -> GovernanceResult<()> {
    observation.validate_schema()?;
    if compute_programdata_failure_observation_digest_v1(observation)?
        != observation.observation_digest
    {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

pub fn canonical_checkpoint_attestation_digest_material_v1(
    attestation: &CheckpointAttestationV1,
) -> GovernanceResult<Vec<u8>> {
    let mut out = Vec::with_capacity(CHECKPOINT_ATTESTATION_DIGEST_MATERIAL_LEN_V1);
    put_pubkey(&mut out, &attestation.controller_program);
    put_pubkey(&mut out, &attestation.controller_config);
    put_pubkey(&mut out, &attestation.checkpoint);
    put_pubkey(&mut out, &attestation.subject);
    put_bytes(&mut out, &attestation.subject_digest);
    put_u8(&mut out, attestation.phase as u8);
    put_bytes(&mut out, &attestation.checkpoint_digest);
    put_pubkey(&mut out, &attestation.council);
    put_u64(&mut out, attestation.council_version);
    put_bytes(&mut out, &attestation.council_hash);
    put_u64(&mut out, attestation.gate_epoch);
    put_u8(&mut out, attestation.seat_index);
    put_pubkey(&mut out, &attestation.seat_authority);
    put_u64(&mut out, attestation.attested_slot);
    finish_material(out, CHECKPOINT_ATTESTATION_DIGEST_MATERIAL_LEN_V1)
}

pub fn compute_checkpoint_attestation_digest_v1(
    attestation: &CheckpointAttestationV1,
) -> GovernanceResult<[u8; 32]> {
    let material = canonical_checkpoint_attestation_digest_material_v1(attestation)?;
    Ok(hashv(&[CHECKPOINT_ATTESTATION_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

pub fn validate_checkpoint_attestation_digest_v1(
    attestation: &CheckpointAttestationV1,
) -> GovernanceResult<()> {
    attestation.validate_schema()?;
    if compute_checkpoint_attestation_digest_v1(attestation)? != attestation.attestation_digest {
        return Err(GovernanceError::Release1DigestMismatch);
    }
    Ok(())
}

fn finish_material(out: Vec<u8>, expected_len: usize) -> GovernanceResult<Vec<u8>> {
    if out.len() != expected_len {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(out)
}

fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put_bool(out: &mut Vec<u8>, value: bool) {
    put_u8(out, u8::from(value));
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_pubkey(out: &mut Vec<u8>, value: &Pubkey) {
    out.extend_from_slice(value.as_ref());
}

fn put_bytes(out: &mut Vec<u8>, value: &[u8; 32]) {
    out.extend_from_slice(value);
}

fn put_optional_pubkey(out: &mut Vec<u8>, value: &OptionalPubkeyV1) {
    put_bool(out, value.present);
    put_pubkey(out, &value.value);
}
