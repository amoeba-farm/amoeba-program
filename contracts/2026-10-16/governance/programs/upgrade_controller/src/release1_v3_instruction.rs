//! Closed fixed-wire instructions for the capacity-safe Release 1 lifecycle.
//!
//! Tags 39..=52 belong to the scalable observation and authority-ceremony
//! protocols.  This module begins at 53 and exposes only typed payloads and
//! fixed account contracts.  It contains no arbitrary CPI bytes, program IDs,
//! account vectors, or persisted dynamic collections.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    artifact_merkle::{MAX_ARTIFACT_BYTES_V1, MAX_ARTIFACT_PROOF_DEPTH_V1},
    council::VALID_APPROVAL_MASK,
    release1_authority_instruction::CeremonyEnvelopeV1,
    release1_ceremony_state::MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
    release1_state::{
        EmergencyFreezeResolutionKindV1, EmergencyFreezeResolutionStateV1, ProposalStateV2,
        StateCheckpointPhaseV1, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        NO_FAILING_CHUNK_INDEX_V1, RELEASE1_APPROVAL_THRESHOLD,
    },
    release1_v3_state::{ProgramDataMismatchClassV2, ProgramDataVerificationStatusV2},
    state::{
        GateStatusV1, OptionalPubkeyV1, ProposalClassV1, RELEASE1_MIN_COUNCIL_REVIEW_SLOTS,
        RELEASE1_MIN_MAJOR_DELAY_SLOTS, RELEASE1_MIN_PROPOSAL_EXPIRY_SLOTS,
        RELEASE1_MIN_ROLLBACK_DELAY_SLOTS, RELEASE1_MIN_ROUTINE_DELAY_SLOTS,
        RELEASE1_MIN_TERMINAL_DELAY_SLOTS,
    },
};

pub const INITIALIZE_CONTROLLER_V2_TAG: u8 = 53;
pub const CREATE_PROPOSAL_V3_TAG: u8 = 54;
pub const APPROVE_PROPOSAL_V3_TAG: u8 = 55;
pub const FINALIZE_GOVERNANCE_V3_TAG: u8 = 56;
pub const QUEUE_PROPOSAL_V3_TAG: u8 = 57;
pub const FREEZE_PROPOSAL_V3_TAG: u8 = 58;
pub const CANCEL_PROPOSAL_V3_TAG: u8 = 59;
pub const EXPIRE_PROPOSAL_V3_TAG: u8 = 60;
pub const GUARDIAN_FREEZE_V2_TAG: u8 = 61;
pub const CREATE_EMERGENCY_RESOLUTION_V2_TAG: u8 = 62;
pub const APPROVE_EMERGENCY_RESOLUTION_V2_TAG: u8 = 63;
pub const QUEUE_EMERGENCY_RESOLUTION_V2_TAG: u8 = 64;
pub const EXECUTE_EMERGENCY_RESOLUTION_V2_TAG: u8 = 65;
pub const EXPIRE_EMERGENCY_RESOLUTION_V2_TAG: u8 = 66;
pub const CREATE_CHECKPOINT_V2_TAG: u8 = 67;
pub const RECAST_CHECKPOINT_V2_TAG: u8 = 68;
pub const FINALIZE_CHECKPOINT_V2_TAG: u8 = 69;
pub const BIND_PROGRAMDATA_VERIFICATION_V2_TAG: u8 = 70;
pub const FINALIZE_PROGRAMDATA_VERIFICATION_V2_TAG: u8 = 71;
pub const OBSERVE_PROGRAMDATA_FAILURE_V2_TAG: u8 = 72;
pub const APPROVE_UNFREEZE_V2_TAG: u8 = 73;
pub const EXECUTE_UNFREEZE_V2_TAG: u8 = 74;

pub const MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN: usize = 16_384;
pub const INITIALIZE_CONTROLLER_V2_ACCOUNT_COUNT: usize = 22;
pub const BOOTSTRAP_INITIAL_SEAT_TERM_END_V2: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct SeatTermV2 {
    pub term_start_slot: u64,
    pub term_end_slot: u64,
}

impl SeatTermV2 {
    pub const LEN: usize = 16;

    fn validate(&self) -> Result<(), ProgramError> {
        if self.term_end_slot != BOOTSTRAP_INITIAL_SEAT_TERM_END_V2
            || self.term_end_slot <= self.term_start_slot
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

/// Signer-visible inputs needed to derive the full immutable capacity-policy
/// account during initialization. Every identity, fixed geometry value, and
/// creation field omitted here is derived by the controller.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CapacityPolicyInputV1 {
    pub expected_policy_digest: [u8; 32],
}

impl CapacityPolicyInputV1 {
    pub const LEN: usize = 32;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[self.expected_policy_digest])
    }
}

/// Compact immutable release manifest. The processor derives controller,
/// ProgramData, loader, capacity, authority, scheme, finalized, and creation
/// fields, then requires the resulting account digest to match the commitment
/// carried here.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ControllerReleaseInputV1 {
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub source_commitment: [u8; 32],
    pub source_tree_commitment: [u8; 32],
    pub build_inputs_commitment: [u8; 32],
    pub toolchain_commitment: [u8; 32],
    pub package_commitment: [u8; 32],
    pub release_manifest_commitment: [u8; 32],
    pub abi_commitment: [u8; 32],
    pub expected_release_digest: [u8; 32],
}

impl ControllerReleaseInputV1 {
    pub const LEN: usize = 328;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.artifact_sha256,
            self.artifact_merkle_root,
            self.source_commitment,
            self.source_tree_commitment,
            self.build_inputs_commitment,
            self.toolchain_commitment,
            self.package_commitment,
            self.release_manifest_commitment,
            self.abi_commitment,
            self.expected_release_digest,
        ])?;
        if self.artifact_length == 0 || self.artifact_length > MAX_ARTIFACT_BYTES_V1 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

/// Compact proposal creation manifest. Runtime identities, canonical PDAs,
/// current slot/timing, fixed schemes, disabled vote fields, and all lifecycle
/// fields are derived by the controller. The transaction accounts themselves
/// bind the buffer and uploader identities.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProposalManifestV3 {
    pub proposal_class: ProposalClassV1,
    pub expected_proposal_id: u64,
    pub expected_target_nonce: u64,
    pub expected_gate_status: GateStatusV1,
    pub expected_gate_epoch: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub expected_policy_version: u64,
    pub expected_policy_hash: [u8; 32],
    pub expected_council_version: u64,
    pub expected_council_hash: [u8; 32],
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_chunk_merkle_root: [u8; 32],
    pub source_commit_hash: [u8; 32],
    pub source_tree_hash: [u8; 32],
    pub build_input_inventory_hash: [u8; 32],
    pub reproducible_build_receipt_hash: [u8; 32],
    pub package_receipt_hash: [u8; 32],
    pub release_intent_hash: [u8; 32],
    pub minimum_required_capacity: u64,
    pub checkpoint_schema_id: [u8; 32],
    pub checkpoint_policy_hash: [u8; 32],
    pub primary_proposal: OptionalPubkeyV1,
    pub rollback_proposal: OptionalPubkeyV1,
    pub rollback_buffer: OptionalPubkeyV1,
    pub rollback_artifact_length: u64,
    pub rollback_artifact_sha256: [u8; 32],
    pub rollback_artifact_chunk_root: [u8; 32],
    pub plan_valid_until_slot: u64,
}

impl ProposalManifestV3 {
    pub const LEN: usize = 693;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
            self.expected_policy_hash,
            self.expected_council_hash,
            self.artifact_sha256,
            self.artifact_chunk_merkle_root,
            self.source_commit_hash,
            self.source_tree_hash,
            self.build_input_inventory_hash,
            self.reproducible_build_receipt_hash,
            self.package_receipt_hash,
            self.release_intent_hash,
            self.checkpoint_schema_id,
            self.checkpoint_policy_hash,
        ])?;
        self.primary_proposal
            .validate()
            .map_err(ProgramError::from)?;
        self.rollback_proposal
            .validate()
            .map_err(ProgramError::from)?;
        self.rollback_buffer
            .validate()
            .map_err(ProgramError::from)?;
        if self.expected_proposal_id == 0
            || self.expected_proposal_id == u64::MAX
            || self.expected_target_nonce == 0
            || self.expected_target_nonce == u64::MAX
            || !matches!(
                self.expected_gate_status,
                GateStatusV1::Active | GateStatusV1::EmergencyFrozen
            )
            || self.expected_gate_epoch == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_policy_version == 0
            || self.expected_council_version == 0
            || self.artifact_length == 0
            || self.artifact_length > MAX_ARTIFACT_BYTES_V1
            || self.minimum_required_capacity < self.artifact_length
            || self.minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.plan_valid_until_slot == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        let rollback_present = self.rollback_proposal.present;
        let rollback_shape = rollback_present == self.rollback_buffer.present
            && rollback_present == (self.rollback_artifact_length != 0)
            && rollback_present == (self.rollback_artifact_sha256 != [0; 32])
            && rollback_present == (self.rollback_artifact_chunk_root != [0; 32]);
        if !rollback_shape
            || (rollback_present && self.rollback_artifact_length > MAX_ARTIFACT_BYTES_V1)
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        match self.proposal_class {
            ProposalClassV1::EmergencyRollback => {
                if !self.primary_proposal.present || rollback_present {
                    return Err(ProgramError::InvalidInstructionData);
                }
            }
            ProposalClassV1::RoutineUpgrade
            | ProposalClassV1::EconomicChange
            | ProposalClassV1::ConstitutionalChange => {
                if self.primary_proposal.present || !rollback_present {
                    return Err(ProgramError::InvalidInstructionData);
                }
            }
            ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        Ok(())
    }
}

/// Bounded guardian freeze guard. The controller derives the full emergency
/// observation from the live Loader graph in the same transaction as freezing.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GuardianFreezeManifestV2 {
    pub expected_gate_epoch: u64,
    pub expected_target_nonce: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub freeze_reason_code: u16,
    pub plan_valid_until_slot: u64,
}

impl GuardianFreezeManifestV2 {
    pub const LEN: usize = 98;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
        ])?;
        if self.expected_gate_epoch == 0
            || self.expected_target_nonce == 0
            || self.expected_current_deployment_generation == 0
            || self.freeze_reason_code == 0
            || self.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.plan_valid_until_slot == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

/// Compact emergency-resume creation manifest. The full resolution, immutable
/// delay window, trusted deployment identity, and observation fields are
/// reconstructed from current canonical accounts.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct EmergencyResolutionManifestV2 {
    pub resolution_kind: EmergencyFreezeResolutionKindV1,
    pub expected_gate_epoch: u64,
    pub expected_target_nonce: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub expected_freeze_observation_digest: [u8; 32],
    pub expected_programdata_observation_digest: [u8; 32],
    pub expected_programdata_observation_generation: u64,
    pub expected_policy_version: u64,
    pub expected_policy_hash: [u8; 32],
    pub expected_council_version: u64,
    pub expected_council_hash: [u8; 32],
    pub plan_valid_until_slot: u64,
}

impl EmergencyResolutionManifestV2 {
    pub const LEN: usize = 249;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
            self.expected_freeze_observation_digest,
            self.expected_programdata_observation_digest,
            self.expected_policy_hash,
            self.expected_council_hash,
        ])?;
        if self.resolution_kind != EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade
            || self.expected_gate_epoch == 0
            || self.expected_target_nonce == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_programdata_observation_generation == 0
            || self.expected_policy_version == 0
            || self.expected_council_version == 0
            || self.plan_valid_until_slot == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

/// Candidate fields that cannot be derived from the protected on-chain state.
/// Three independent CheckpointAttestationV1 accounts bind the resulting exact
/// checkpoint digest before the canonical StateCheckpointV2 is created.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CheckpointManifestV2 {
    pub phase: StateCheckpointPhaseV1,
    pub checkpoint_generation: u64,
    pub previous_checkpoint_digest: [u8; 32],
    pub expected_subject_digest: [u8; 32],
    pub expected_gate_epoch: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub expected_observation_digest: [u8; 32],
    pub expected_observation_generation: u64,
    pub program_owned_state_root: [u8; 32],
    pub program_owned_state_count: u64,
    pub logical_compressed_state_root: [u8; 32],
    pub logical_compressed_state_count: u64,
    pub semantic_custody_accounting_root: [u8; 32],
    pub hard_combined_root: [u8; 32],
    pub external_metadata_observation_root: [u8; 32],
    pub external_raw_balance_observation_root: [u8; 32],
    pub schema_identifier: [u8; 32],
    pub admitted_positive_donation_root: [u8; 32],
    pub admitted_positive_donation_count: u64,
    pub forbidden_drift_count: u32,
    pub expected_council_version: u64,
    pub expected_council_hash: [u8; 32],
    pub expected_checkpoint_digest: [u8; 32],
    pub plan_valid_until_slot: u64,
}

impl CheckpointManifestV2 {
    pub const LEN: usize = 557;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_subject_digest,
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
            self.expected_observation_digest,
            self.program_owned_state_root,
            self.logical_compressed_state_root,
            self.semantic_custody_accounting_root,
            self.hard_combined_root,
            self.external_metadata_observation_root,
            self.external_raw_balance_observation_root,
            self.schema_identifier,
            self.expected_council_hash,
            self.expected_checkpoint_digest,
        ])?;
        if self.checkpoint_generation == 0
            || (self.checkpoint_generation == 1 && self.previous_checkpoint_digest != [0; 32])
            || (self.checkpoint_generation > 1 && self.previous_checkpoint_digest == [0; 32])
            || self.expected_gate_epoch == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_observation_generation == 0
            || self.expected_council_version == 0
            || self.forbidden_drift_count != 0
            || self.plan_valid_until_slot == 0
            || (self.admitted_positive_donation_count == 0
                && self.admitted_positive_donation_root != [0; 32])
            || (self.admitted_positive_donation_count != 0
                && self.admitted_positive_donation_root == [0; 32])
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CheckpointAttestationGuardV2 {
    pub manifest: CheckpointManifestV2,
    pub seat_index: u8,
    pub expected_previous_attestation_digest: [u8; 32],
}

impl CheckpointAttestationGuardV2 {
    pub const LEN: usize = CheckpointManifestV2::LEN + 1 + 32;

    fn validate_create(&self) -> Result<(), ProgramError> {
        self.manifest.validate()?;
        if self.seat_index >= 5 || self.expected_previous_attestation_digest != [0; 32] {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    fn validate_recast(&self) -> Result<(), ProgramError> {
        self.manifest.validate()?;
        if self.seat_index >= 5 || self.expected_previous_attestation_digest == [0; 32] {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataVerificationManifestV2 {
    pub expected: ProposalGuardV3,
    pub expected_observation_digest: [u8; 32],
    pub expected_observation_generation: u64,
    pub expected_observation_root: [u8; 32],
    pub expected_observation_finalized_slot: u64,
    pub verification_generation: u64,
    pub previous_verification_digest: [u8; 32],
    pub plan_valid_until_slot: u64,
}

impl ProgramDataVerificationManifestV2 {
    pub const LEN: usize = ProposalGuardV3::LEN + 128;

    fn validate(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        require_hashes(&[
            self.expected_observation_digest,
            self.expected_observation_root,
        ])?;
        if self.expected.expected_state != ProposalStateV2::UpgradeExecuted
            || self.expected_observation_generation == 0
            || self.expected_observation_finalized_slot == 0
            || self.verification_generation == 0
            || (self.verification_generation == 1 && self.previous_verification_digest != [0; 32])
            || (self.verification_generation > 1 && self.previous_verification_digest == [0; 32])
            || self.plan_valid_until_slot == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

/// Compact mechanical witness for the first post-upgrade verification
/// failure. The processor derives the full failure-observation account and its
/// Clock-bound digest. Operator-supplied bytes or leaf hashes are never trusted
/// without a canonical proof or an on-chain derivation.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataFailureProofV2 {
    pub proof_len: u8,
    pub nodes: [[u8; 32]; MAX_ARTIFACT_PROOF_DEPTH_V1],
}

impl ProgramDataFailureProofV2 {
    pub const LEN: usize = 1 + 32 * MAX_ARTIFACT_PROOF_DEPTH_V1;

    fn validate(&self) -> Result<(), ProgramError> {
        let used = usize::from(self.proof_len);
        if used > MAX_ARTIFACT_PROOF_DEPTH_V1
            || self.nodes[used..].iter().any(|node| *node != [0; 32])
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataFailureWitnessV2 {
    pub expected_proposal: ProposalGuardV3,
    pub expected_verification_digest: [u8; 32],
    pub expected_verification_generation: u64,
    pub expected_observation_generation: u64,
    pub expected_observation_state_hash: [u8; 32],
    pub mismatch_class: ProgramDataMismatchClassV2,
    pub failing_chunk_index: u32,
    pub expected_leaf_hash: [u8; 32],
    pub proof: ProgramDataFailureProofV2,
    pub plan_valid_until_slot: u64,
}

impl ProgramDataFailureWitnessV2 {
    pub const LEN: usize = ProposalGuardV3::LEN + 350;

    fn validate(&self) -> Result<(), ProgramError> {
        self.expected_proposal.validate()?;
        self.proof.validate()?;
        let verification_present = self.expected_verification_digest != [0; 32];
        if self.expected_proposal.expected_state != ProposalStateV2::UpgradeExecuted
            || verification_present != (self.expected_verification_generation != 0)
            || self.expected_observation_generation == 0
            || self.plan_valid_until_slot == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        let proof_present =
            self.proof.proof_len != 0 || self.proof.nodes.iter().any(|node| *node != [0; 32]);
        match self.mismatch_class {
            ProgramDataMismatchClassV2::ArtifactPayload => {
                if self.failing_chunk_index == NO_FAILING_CHUNK_INDEX_V1
                    || self.expected_leaf_hash == [0; 32]
                    || self.expected_observation_state_hash == [0; 32]
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
            }
            ProgramDataMismatchClassV2::ZeroTail => {
                if self.failing_chunk_index == NO_FAILING_CHUNK_INDEX_V1
                    || self.expected_leaf_hash != [0; 32]
                    || proof_present
                    || self.expected_observation_state_hash == [0; 32]
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
            }
            ProgramDataMismatchClassV2::ObservationStale
            | ProgramDataMismatchClassV2::ObservationScheme => {
                if self.failing_chunk_index != NO_FAILING_CHUNK_INDEX_V1
                    || self.expected_leaf_hash != [0; 32]
                    || proof_present
                    || self.expected_observation_state_hash == [0; 32]
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
            }
            _ => {
                if self.failing_chunk_index != NO_FAILING_CHUNK_INDEX_V1
                    || self.expected_leaf_hash != [0; 32]
                    || proof_present
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProposalGuardV3 {
    pub expected_proposal_digest: [u8; 32],
    pub expected_state: ProposalStateV2,
    pub expected_gate_status: GateStatusV1,
    pub expected_gate_epoch: u64,
    pub expected_target_nonce: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
}

impl ProposalGuardV3 {
    pub const LEN: usize = 122;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_proposal_digest,
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
        ])?;
        if self.expected_gate_epoch == 0
            || self.expected_target_nonce == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_state == ProposalStateV2::TokenReviewOpen
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct EmergencyResolutionGuardV2 {
    pub expected_resolution_digest: [u8; 32],
    pub expected_state: EmergencyFreezeResolutionStateV1,
    pub expected_gate_status: GateStatusV1,
    pub expected_gate_epoch: u64,
    pub expected_target_nonce: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub expected_freeze_observation_digest: [u8; 32],
    pub expected_programdata_observation_digest: [u8; 32],
    pub expected_observation_generation: u64,
    pub expected_checkpoint_digest: [u8; 32],
}

impl EmergencyResolutionGuardV2 {
    pub const LEN: usize = 226;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_resolution_digest,
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
            self.expected_freeze_observation_digest,
            self.expected_programdata_observation_digest,
        ])?;
        if self.expected_gate_status != GateStatusV1::EmergencyFrozen
            || self.expected_gate_epoch == 0
            || self.expected_target_nonce == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_observation_generation == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataVerificationGuardV2 {
    pub expected_proposal_digest: [u8; 32],
    pub expected_verification_digest: [u8; 32],
    pub expected_verification_generation: u64,
    pub expected_status: ProgramDataVerificationStatusV2,
    pub expected_gate_epoch: u64,
    pub expected_target_nonce: u64,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub expected_observation_digest: [u8; 32],
    pub expected_observation_generation: u64,
    pub expected_actual_capacity: u64,
    pub expected_authority: Pubkey,
}

impl ProgramDataVerificationGuardV2 {
    pub const LEN: usize = 241;

    pub fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_proposal_digest,
            self.expected_verification_digest,
            self.expected_capacity_policy_digest,
            self.expected_current_deployment_digest,
            self.expected_observation_digest,
        ])?;
        if self.expected_verification_generation == 0
            || self.expected_gate_epoch == 0
            || self.expected_target_nonce == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_observation_generation == 0
            || self.expected_actual_capacity == 0
            || self.expected_authority == Pubkey::default()
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct UnfreezeGuardV2 {
    pub expected_proposal_digest: [u8; 32],
    pub expected_checkpoint_digest: [u8; 32],
    pub expected_checkpoint_generation: u64,
    pub expected_verification_digest: [u8; 32],
    pub expected_verification_generation: u64,
    pub expected_original_council_version: u64,
    pub expected_original_council_hash: [u8; 32],
    pub expected_current_council_version: u64,
    pub expected_current_council_hash: [u8; 32],
    pub expected_gate_epoch: u64,
    pub expected_target_nonce: u64,
    pub expected_current_deployment_digest: [u8; 32],
    pub expected_current_deployment_generation: u64,
    pub expected_artifact_sha256: [u8; 32],
    pub expected_artifact_merkle_root: [u8; 32],
    pub expected_actual_capacity: u64,
    pub expected_approval_bitset: u8,
    pub expected_approval_count: u8,
}

impl UnfreezeGuardV2 {
    pub const LEN: usize = 322;

    fn validate(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.expected_proposal_digest,
            self.expected_checkpoint_digest,
            self.expected_verification_digest,
            self.expected_original_council_hash,
            self.expected_current_council_hash,
            self.expected_current_deployment_digest,
            self.expected_artifact_sha256,
            self.expected_artifact_merkle_root,
        ])?;
        validate_approval_pair(self.expected_approval_bitset, self.expected_approval_count)?;
        if self.expected_checkpoint_generation == 0
            || self.expected_verification_generation == 0
            || self.expected_original_council_version == 0
            || self.expected_current_council_version == 0
            || self.expected_gate_epoch == 0
            || self.expected_target_nonce == 0
            || self.expected_current_deployment_generation == 0
            || self.expected_actual_capacity == 0
            || self.expected_approval_count > RELEASE1_APPROVAL_THRESHOLD
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

trait WireValidate {
    fn validate_wire(&self) -> Result<(), ProgramError>;
}

macro_rules! fixed_instruction {
    ($name:ident, $tag:ident, $tag_value:expr, $payload_len:expr, { $($field:ident: $ty:ty),* $(,)? }) => {
        pub const $tag: u8 = $tag_value;

        #[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
        pub struct $name {
            $(pub $field: $ty,)*
        }

        impl $name {
            pub const TAG: u8 = $tag;
            pub const PAYLOAD_LEN: usize = $payload_len;
            pub const LEN: usize = 1 + Self::PAYLOAD_LEN;

            pub fn pack(&self) -> Result<Vec<u8>, ProgramError> {
                self.validate_wire()?;
                let payload = self
                    .try_to_vec()
                    .map_err(|_| ProgramError::InvalidInstructionData)?;
                if payload.len() != Self::PAYLOAD_LEN || Self::LEN > MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let mut data = Vec::with_capacity(Self::LEN);
                data.push(Self::TAG);
                data.extend_from_slice(&payload);
                Ok(data)
            }

            pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
                if data.len() != Self::LEN
                    || data.len() > MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN
                    || data.first().copied() != Some(Self::TAG)
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let value = Self::try_from_slice(&data[1..])
                    .map_err(|_| ProgramError::InvalidInstructionData)?;
                value.validate_wire()?;
                Ok(value)
            }
        }
    };
}

fixed_instruction!(
    InitializeControllerV2,
    INITIALIZE_CONTROLLER_V2_TAG_ALIAS,
    INITIALIZE_CONTROLLER_V2_TAG,
    632,
    {
        cluster_domain: [u8; 32],
        initial_policy_version: u64,
        initial_council_version: u64,
        next_proposal_id: u64,
        target_nonce: u64,
        initial_gate_epoch: u64,
        policy_activation_slot: u64,
        routine_delay_slots: u64,
        major_delay_slots: u64,
        rollback_delay_slots: u64,
        terminal_delay_slots: u64,
        vote_review_slots: u64,
        proposal_expiry_slots: u64,
        expected_policy_hash: [u8; 32],
        expected_council_hash: [u8; 32],
        seat_terms: [SeatTermV2; 5],
        capacity_policy: CapacityPolicyInputV1,
        controller_release: ControllerReleaseInputV1
    }
);

fixed_instruction!(
    CreateProposalV3,
    CREATE_PROPOSAL_V3_TAG_ALIAS,
    CREATE_PROPOSAL_V3_TAG,
    ProposalManifestV3::LEN,
    { manifest: ProposalManifestV3 }
);

fixed_instruction!(
    ApproveProposalV3,
    APPROVE_PROPOSAL_V3_TAG_ALIAS,
    APPROVE_PROPOSAL_V3_TAG,
    ProposalGuardV3::LEN + 8 + 32 + 1 + 1,
    {
        expected: ProposalGuardV3,
        expected_creation_council_version: u64,
        expected_creation_council_hash: [u8; 32],
        expected_approval_bitset: u8,
        expected_approval_count: u8
    }
);

fixed_instruction!(
    FinalizeGovernanceV3,
    FINALIZE_GOVERNANCE_V3_TAG_ALIAS,
    FINALIZE_GOVERNANCE_V3_TAG,
    ProposalGuardV3::LEN + 1 + 1,
    {
        expected: ProposalGuardV3,
        expected_approval_bitset: u8,
        expected_approval_count: u8
    }
);

fixed_instruction!(QueueProposalV3, QUEUE_PROPOSAL_V3_TAG_ALIAS, QUEUE_PROPOSAL_V3_TAG, ProposalGuardV3::LEN, {
    expected: ProposalGuardV3
});

fixed_instruction!(FreezeProposalV3, FREEZE_PROPOSAL_V3_TAG_ALIAS, FREEZE_PROPOSAL_V3_TAG, ProposalGuardV3::LEN + 8, {
    expected: ProposalGuardV3,
    expected_next_gate_epoch: u64
});

fixed_instruction!(CancelProposalV3, CANCEL_PROPOSAL_V3_TAG_ALIAS, CANCEL_PROPOSAL_V3_TAG, ProposalGuardV3::LEN + 8 + 32 + 1 + 1 + 2, {
    expected: ProposalGuardV3,
    expected_cancellation_council_version: u64,
    expected_cancellation_council_hash: [u8; 32],
    expected_cancellation_approval_bitset: u8,
    expected_cancellation_approval_count: u8,
    cancellation_reason_code: u16
});

fixed_instruction!(ExpireProposalV3, EXPIRE_PROPOSAL_V3_TAG_ALIAS, EXPIRE_PROPOSAL_V3_TAG, ProposalGuardV3::LEN, {
    expected: ProposalGuardV3
});

fixed_instruction!(GuardianFreezeV2, GUARDIAN_FREEZE_V2_TAG_ALIAS, GUARDIAN_FREEZE_V2_TAG, GuardianFreezeManifestV2::LEN, {
    manifest: GuardianFreezeManifestV2
});

fixed_instruction!(CreateEmergencyResolutionV2, CREATE_EMERGENCY_RESOLUTION_V2_TAG_ALIAS, CREATE_EMERGENCY_RESOLUTION_V2_TAG, EmergencyResolutionManifestV2::LEN, {
    manifest: EmergencyResolutionManifestV2
});

fixed_instruction!(ApproveEmergencyResolutionV2, APPROVE_EMERGENCY_RESOLUTION_V2_TAG_ALIAS, APPROVE_EMERGENCY_RESOLUTION_V2_TAG, EmergencyResolutionGuardV2::LEN + 1 + 1, {
    expected: EmergencyResolutionGuardV2,
    expected_approval_bitset: u8,
    expected_approval_count: u8
});

fixed_instruction!(QueueEmergencyResolutionV2, QUEUE_EMERGENCY_RESOLUTION_V2_TAG_ALIAS, QUEUE_EMERGENCY_RESOLUTION_V2_TAG, EmergencyResolutionGuardV2::LEN, {
    expected: EmergencyResolutionGuardV2
});

fixed_instruction!(ExecuteEmergencyResolutionV2, EXECUTE_EMERGENCY_RESOLUTION_V2_TAG_ALIAS, EXECUTE_EMERGENCY_RESOLUTION_V2_TAG, EmergencyResolutionGuardV2::LEN + CeremonyEnvelopeV1::LEN, {
    expected: EmergencyResolutionGuardV2,
    envelope: CeremonyEnvelopeV1
});

fixed_instruction!(ExpireEmergencyResolutionV2, EXPIRE_EMERGENCY_RESOLUTION_V2_TAG_ALIAS, EXPIRE_EMERGENCY_RESOLUTION_V2_TAG, EmergencyResolutionGuardV2::LEN, {
    expected: EmergencyResolutionGuardV2
});

fixed_instruction!(CreateCheckpointV2, CREATE_CHECKPOINT_V2_TAG_ALIAS, CREATE_CHECKPOINT_V2_TAG, CheckpointAttestationGuardV2::LEN, {
    attestation: CheckpointAttestationGuardV2
});

fixed_instruction!(RecastCheckpointV2, RECAST_CHECKPOINT_V2_TAG_ALIAS, RECAST_CHECKPOINT_V2_TAG, CheckpointAttestationGuardV2::LEN, {
    attestation: CheckpointAttestationGuardV2
});

fixed_instruction!(FinalizeCheckpointV2, FINALIZE_CHECKPOINT_V2_TAG_ALIAS, FINALIZE_CHECKPOINT_V2_TAG, CheckpointManifestV2::LEN, {
    manifest: CheckpointManifestV2
});

fixed_instruction!(BindProgramDataVerificationV2, BIND_PROGRAMDATA_VERIFICATION_V2_TAG_ALIAS, BIND_PROGRAMDATA_VERIFICATION_V2_TAG, ProgramDataVerificationManifestV2::LEN, {
    manifest: ProgramDataVerificationManifestV2
});

fixed_instruction!(FinalizeProgramDataVerificationV2, FINALIZE_PROGRAMDATA_VERIFICATION_V2_TAG_ALIAS, FINALIZE_PROGRAMDATA_VERIFICATION_V2_TAG, ProgramDataVerificationGuardV2::LEN, {
    expected: ProgramDataVerificationGuardV2
});

fixed_instruction!(ObserveProgramDataFailureV2, OBSERVE_PROGRAMDATA_FAILURE_V2_TAG_ALIAS, OBSERVE_PROGRAMDATA_FAILURE_V2_TAG, ProgramDataFailureWitnessV2::LEN, {
    witness: ProgramDataFailureWitnessV2
});

fixed_instruction!(ApproveUnfreezeV2, APPROVE_UNFREEZE_V2_TAG_ALIAS, APPROVE_UNFREEZE_V2_TAG, UnfreezeGuardV2::LEN, {
    expected: UnfreezeGuardV2
});

fixed_instruction!(ExecuteUnfreezeV2, EXECUTE_UNFREEZE_V2_TAG_ALIAS, EXECUTE_UNFREEZE_V2_TAG, UnfreezeGuardV2::LEN + 32 + CeremonyEnvelopeV1::LEN, {
    expected: UnfreezeGuardV2,
    linked_proposal: Pubkey,
    envelope: CeremonyEnvelopeV1
});

impl WireValidate for InitializeControllerV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        require_hashes(&[
            self.cluster_domain,
            self.expected_policy_hash,
            self.expected_council_hash,
        ])?;
        let minimum_expiry_slots = 1u64
            .checked_add(self.vote_review_slots)
            .and_then(|slots| slots.checked_add(self.major_delay_slots))
            .and_then(|slots| slots.checked_add(self.vote_review_slots))
            .and_then(|slots| slots.checked_add(2))
            .ok_or(ProgramError::InvalidInstructionData)?;
        if self.initial_policy_version == 0
            || self.initial_council_version == 0
            || self.next_proposal_id == 0
            || self.next_proposal_id == u64::MAX
            || self.target_nonce == 0
            || self.target_nonce == u64::MAX
            || self.initial_gate_epoch != 1
            || self.policy_activation_slot == 0
            || self.routine_delay_slots < RELEASE1_MIN_ROUTINE_DELAY_SLOTS
            || self.major_delay_slots < RELEASE1_MIN_MAJOR_DELAY_SLOTS
            || self.rollback_delay_slots < RELEASE1_MIN_ROLLBACK_DELAY_SLOTS
            || self.terminal_delay_slots < RELEASE1_MIN_TERMINAL_DELAY_SLOTS
            || self.vote_review_slots < RELEASE1_MIN_COUNCIL_REVIEW_SLOTS
            || self.proposal_expiry_slots < RELEASE1_MIN_PROPOSAL_EXPIRY_SLOTS
            || self.major_delay_slots < self.routine_delay_slots
            || self.rollback_delay_slots > self.routine_delay_slots
            || self.terminal_delay_slots < self.major_delay_slots
            || minimum_expiry_slots >= self.proposal_expiry_slots
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        for term in &self.seat_terms {
            term.validate()?;
        }
        self.capacity_policy.validate()?;
        self.controller_release.validate()
    }
}

impl WireValidate for CreateProposalV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.manifest.validate()
    }
}

impl WireValidate for ApproveProposalV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        validate_approval_pair(self.expected_approval_bitset, self.expected_approval_count)?;
        if self.expected.expected_state != ProposalStateV2::BufferVerified
            || self.expected_creation_council_version == 0
            || self.expected_creation_council_hash == [0; 32]
            || self.expected_approval_count >= RELEASE1_APPROVAL_THRESHOLD
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for FinalizeGovernanceV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        validate_approval_pair(self.expected_approval_bitset, self.expected_approval_count)?;
        if self.expected.expected_state != ProposalStateV2::CouncilApproved
            || self.expected_approval_count != RELEASE1_APPROVAL_THRESHOLD
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for QueueProposalV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        require_proposal_state(&self.expected, ProposalStateV2::GovernanceSatisfied)
    }
}

impl WireValidate for FreezeProposalV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        require_proposal_state(&self.expected, ProposalStateV2::Timelocked)?;
        let expected_next = self
            .expected
            .expected_gate_epoch
            .checked_add(1)
            .ok_or(ProgramError::InvalidInstructionData)?;
        if self.expected_next_gate_epoch != expected_next
            || !matches!(
                self.expected.expected_gate_status,
                GateStatusV1::Active | GateStatusV1::EmergencyFrozen
            )
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for CancelProposalV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        validate_approval_pair(
            self.expected_cancellation_approval_bitset,
            self.expected_cancellation_approval_count,
        )?;
        if !is_prefreeze_proposal_state(self.expected.expected_state)
            || self.expected_cancellation_council_version == 0
            || self.expected_cancellation_council_hash == [0; 32]
            || self.expected_cancellation_approval_count >= RELEASE1_APPROVAL_THRESHOLD
            || self.cancellation_reason_code == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for ExpireProposalV3 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        if !is_prefreeze_proposal_state(self.expected.expected_state) {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for GuardianFreezeV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.manifest.validate()
    }
}

impl WireValidate for CreateEmergencyResolutionV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.manifest.validate()
    }
}

impl WireValidate for ApproveEmergencyResolutionV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        validate_approval_pair(self.expected_approval_bitset, self.expected_approval_count)?;
        if self.expected.expected_state != EmergencyFreezeResolutionStateV1::Draft
            || self.expected.expected_checkpoint_digest == [0; 32]
            || self.expected_approval_count >= RELEASE1_APPROVAL_THRESHOLD
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for QueueEmergencyResolutionV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        require_resolution_state(
            &self.expected,
            EmergencyFreezeResolutionStateV1::CouncilApproved,
        )?;
        if self.expected.expected_checkpoint_digest == [0; 32] {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for ExecuteEmergencyResolutionV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        require_resolution_state(&self.expected, EmergencyFreezeResolutionStateV1::Timelocked)?;
        if self.expected.expected_checkpoint_digest == [0; 32] {
            return Err(ProgramError::InvalidInstructionData);
        }
        self.envelope.validate()
    }
}

impl WireValidate for ExpireEmergencyResolutionV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        if !matches!(
            self.expected.expected_state,
            EmergencyFreezeResolutionStateV1::Draft
                | EmergencyFreezeResolutionStateV1::CouncilApproved
                | EmergencyFreezeResolutionStateV1::Timelocked
        ) {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for CreateCheckpointV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.attestation.validate_create()
    }
}

impl WireValidate for RecastCheckpointV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.attestation.validate_recast()
    }
}

impl WireValidate for FinalizeCheckpointV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.manifest.validate()
    }
}

impl WireValidate for BindProgramDataVerificationV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.manifest.validate()
    }
}

impl WireValidate for FinalizeProgramDataVerificationV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        if self.expected.expected_status != ProgramDataVerificationStatusV2::ObservationBound {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for ObserveProgramDataFailureV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.witness.validate()
    }
}

impl WireValidate for ApproveUnfreezeV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        if self.expected.expected_approval_count >= RELEASE1_APPROVAL_THRESHOLD {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for ExecuteUnfreezeV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        self.expected.validate()?;
        if self.expected.expected_approval_count != RELEASE1_APPROVAL_THRESHOLD
            || self.linked_proposal == Pubkey::default()
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        self.envelope.validate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Release1V3Instruction {
    InitializeController(Box<InitializeControllerV2>),
    CreateProposal(Box<CreateProposalV3>),
    ApproveProposal(ApproveProposalV3),
    FinalizeGovernance(FinalizeGovernanceV3),
    QueueProposal(QueueProposalV3),
    FreezeProposal(FreezeProposalV3),
    CancelProposal(CancelProposalV3),
    ExpireProposal(ExpireProposalV3),
    GuardianFreeze(Box<GuardianFreezeV2>),
    CreateEmergencyResolution(Box<CreateEmergencyResolutionV2>),
    ApproveEmergencyResolution(ApproveEmergencyResolutionV2),
    QueueEmergencyResolution(QueueEmergencyResolutionV2),
    ExecuteEmergencyResolution(ExecuteEmergencyResolutionV2),
    ExpireEmergencyResolution(ExpireEmergencyResolutionV2),
    CreateCheckpoint(Box<CreateCheckpointV2>),
    RecastCheckpoint(Box<RecastCheckpointV2>),
    FinalizeCheckpoint(FinalizeCheckpointV2),
    BindProgramDataVerification(Box<BindProgramDataVerificationV2>),
    FinalizeProgramDataVerification(Box<FinalizeProgramDataVerificationV2>),
    ObserveProgramDataFailure(Box<ObserveProgramDataFailureV2>),
    ApproveUnfreeze(ApproveUnfreezeV2),
    ExecuteUnfreeze(ExecuteUnfreezeV2),
}

impl Release1V3Instruction {
    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.is_empty() || data.len() > MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN {
            return Err(ProgramError::InvalidInstructionData);
        }
        match data[0] {
            INITIALIZE_CONTROLLER_V2_TAG => InitializeControllerV2::unpack(data)
                .map(Box::new)
                .map(Self::InitializeController),
            CREATE_PROPOSAL_V3_TAG => CreateProposalV3::unpack(data)
                .map(Box::new)
                .map(Self::CreateProposal),
            APPROVE_PROPOSAL_V3_TAG => ApproveProposalV3::unpack(data).map(Self::ApproveProposal),
            FINALIZE_GOVERNANCE_V3_TAG => {
                FinalizeGovernanceV3::unpack(data).map(Self::FinalizeGovernance)
            }
            QUEUE_PROPOSAL_V3_TAG => QueueProposalV3::unpack(data).map(Self::QueueProposal),
            FREEZE_PROPOSAL_V3_TAG => FreezeProposalV3::unpack(data).map(Self::FreezeProposal),
            CANCEL_PROPOSAL_V3_TAG => CancelProposalV3::unpack(data).map(Self::CancelProposal),
            EXPIRE_PROPOSAL_V3_TAG => ExpireProposalV3::unpack(data).map(Self::ExpireProposal),
            GUARDIAN_FREEZE_V2_TAG => GuardianFreezeV2::unpack(data)
                .map(Box::new)
                .map(Self::GuardianFreeze),
            CREATE_EMERGENCY_RESOLUTION_V2_TAG => CreateEmergencyResolutionV2::unpack(data)
                .map(Box::new)
                .map(Self::CreateEmergencyResolution),
            APPROVE_EMERGENCY_RESOLUTION_V2_TAG => {
                ApproveEmergencyResolutionV2::unpack(data).map(Self::ApproveEmergencyResolution)
            }
            QUEUE_EMERGENCY_RESOLUTION_V2_TAG => {
                QueueEmergencyResolutionV2::unpack(data).map(Self::QueueEmergencyResolution)
            }
            EXECUTE_EMERGENCY_RESOLUTION_V2_TAG => {
                ExecuteEmergencyResolutionV2::unpack(data).map(Self::ExecuteEmergencyResolution)
            }
            EXPIRE_EMERGENCY_RESOLUTION_V2_TAG => {
                ExpireEmergencyResolutionV2::unpack(data).map(Self::ExpireEmergencyResolution)
            }
            CREATE_CHECKPOINT_V2_TAG => CreateCheckpointV2::unpack(data)
                .map(Box::new)
                .map(Self::CreateCheckpoint),
            RECAST_CHECKPOINT_V2_TAG => RecastCheckpointV2::unpack(data)
                .map(Box::new)
                .map(Self::RecastCheckpoint),
            FINALIZE_CHECKPOINT_V2_TAG => {
                FinalizeCheckpointV2::unpack(data).map(Self::FinalizeCheckpoint)
            }
            BIND_PROGRAMDATA_VERIFICATION_V2_TAG => BindProgramDataVerificationV2::unpack(data)
                .map(Box::new)
                .map(Self::BindProgramDataVerification),
            FINALIZE_PROGRAMDATA_VERIFICATION_V2_TAG => {
                FinalizeProgramDataVerificationV2::unpack(data)
                    .map(Box::new)
                    .map(Self::FinalizeProgramDataVerification)
            }
            OBSERVE_PROGRAMDATA_FAILURE_V2_TAG => ObserveProgramDataFailureV2::unpack(data)
                .map(Box::new)
                .map(Self::ObserveProgramDataFailure),
            APPROVE_UNFREEZE_V2_TAG => ApproveUnfreezeV2::unpack(data).map(Self::ApproveUnfreeze),
            EXECUTE_UNFREEZE_V2_TAG => ExecuteUnfreezeV2::unpack(data).map(Self::ExecuteUnfreeze),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

macro_rules! account_meta {
    ($key:expr, readonly) => {
        AccountMeta::new_readonly($key, false)
    };
    ($key:expr, writable) => {
        AccountMeta::new($key, false)
    };
    ($key:expr, signer_readonly) => {
        AccountMeta::new_readonly($key, true)
    };
    ($key:expr, signer_writable) => {
        AccountMeta::new($key, true)
    };
}

macro_rules! closed_builder {
    (
        $accounts:ident { $($field:ident: $mode:ident),* $(,)? },
        $function:ident,
        $instruction:ty
    ) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $accounts {
            $(pub $field: Pubkey,)*
        }

        pub fn $function(
            controller_program: Pubkey,
            accounts: $accounts,
            instruction: $instruction,
        ) -> Result<Instruction, ProgramError> {
            Ok(Instruction {
                program_id: controller_program,
                accounts: vec![
                    $(account_meta!(accounts.$field, $mode),)*
                ],
                data: instruction.pack()?,
            })
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitializeControllerV2Accounts {
    pub payer: Pubkey,
    pub initializer: Pubkey,
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_config: Pubkey,
    pub authority_pda: Pubkey,
    pub protocol_gate: Pubkey,
    pub policy: Pubkey,
    pub council: Pubkey,
    pub capacity_policy: Pubkey,
    pub controller_release: Pubkey,
    pub canonical_spill_treasury: Pubkey,
    pub guardian: Pubkey,
    pub seat_authorities: [Pubkey; 5],
    pub system_program: Pubkey,
}

pub fn initialize_controller_v2_instruction(
    controller_program_id: Pubkey,
    accounts: InitializeControllerV2Accounts,
    instruction: InitializeControllerV2,
) -> Result<Instruction, ProgramError> {
    let mut metas = vec![
        account_meta!(accounts.payer, signer_writable),
        account_meta!(accounts.initializer, signer_readonly),
        account_meta!(accounts.controller_program, readonly),
        account_meta!(accounts.controller_programdata, readonly),
        account_meta!(accounts.target_program, readonly),
        account_meta!(accounts.target_programdata, readonly),
        account_meta!(accounts.upgradeable_loader, readonly),
        account_meta!(accounts.controller_config, writable),
        account_meta!(accounts.authority_pda, readonly),
        account_meta!(accounts.protocol_gate, writable),
        account_meta!(accounts.policy, writable),
        account_meta!(accounts.council, writable),
        account_meta!(accounts.capacity_policy, writable),
        account_meta!(accounts.controller_release, writable),
        account_meta!(accounts.canonical_spill_treasury, readonly),
        account_meta!(accounts.guardian, readonly),
    ];
    metas.extend(
        accounts
            .seat_authorities
            .into_iter()
            .map(|authority| account_meta!(authority, readonly)),
    );
    metas.push(account_meta!(accounts.system_program, readonly));
    Ok(Instruction {
        program_id: controller_program_id,
        accounts: metas,
        data: instruction.pack()?,
    })
}

closed_builder!(
    CreateProposalV3Accounts {
        payer: signer_writable,
        creator_seat_authority: signer_readonly,
        controller_config: writable,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        canonical_spill_treasury: readonly,
        buffer: readonly,
        buffer_uploader_authority: readonly,
        proposal: writable,
        system_program: readonly,
    },
    create_proposal_v3_instruction,
    CreateProposalV3
);

closed_builder!(
    ApproveProposalV3Accounts {
        controller_config: readonly,
        policy: readonly,
        creation_council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        proposal: writable,
        buffer_verification: readonly,
        seat_authority: signer_readonly,
    },
    approve_proposal_v3_instruction,
    ApproveProposalV3
);

closed_builder!(
    FinalizeGovernanceV3Accounts {
        controller_config: readonly,
        policy: readonly,
        creation_council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        proposal: writable,
    },
    finalize_governance_v3_instruction,
    FinalizeGovernanceV3
);

closed_builder!(
    QueueProposalV3Accounts {
        controller_config: readonly,
        policy: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        proposal: writable,
    },
    queue_proposal_v3_instruction,
    QueueProposalV3
);

closed_builder!(
    FreezeProposalV3Accounts {
        controller_config: writable,
        policy: readonly,
        creation_council: readonly,
        protocol_gate: writable,
        capacity_policy: readonly,
        current_deployment: readonly,
        proposal: writable,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        rollback_proposal: readonly,
        rollback_buffer_verification: readonly,
        rollback_buffer: readonly,
    },
    freeze_proposal_v3_instruction,
    FreezeProposalV3
);

closed_builder!(
    CancelProposalV3Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        proposal: writable,
        seat_authority: signer_readonly,
    },
    cancel_proposal_v3_instruction,
    CancelProposalV3
);

closed_builder!(
    ExpireProposalV3Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        proposal: writable,
    },
    expire_proposal_v3_instruction,
    ExpireProposalV3
);

closed_builder!(
    GuardianFreezeV2Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: writable,
        capacity_policy: readonly,
        current_deployment: readonly,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        guardian: signer_readonly,
        emergency_freeze_observation: writable,
        system_program: readonly,
    },
    guardian_freeze_v2_instruction,
    GuardianFreezeV2
);

closed_builder!(
    CreateEmergencyResolutionV2Accounts {
        payer: signer_writable,
        creator_seat_authority: signer_readonly,
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        emergency_freeze_observation: readonly,
        programdata_observation: readonly,
        emergency_resolution: writable,
        system_program: readonly,
    },
    create_emergency_resolution_v2_instruction,
    CreateEmergencyResolutionV2
);

closed_builder!(
    ApproveEmergencyResolutionV2Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        emergency_freeze_observation: readonly,
        programdata_observation: readonly,
        emergency_checkpoint: readonly,
        emergency_resolution: writable,
        seat_authority: signer_readonly,
    },
    approve_emergency_resolution_v2_instruction,
    ApproveEmergencyResolutionV2
);

closed_builder!(
    QueueEmergencyResolutionV2Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        emergency_freeze_observation: readonly,
        programdata_observation: readonly,
        emergency_checkpoint: readonly,
        emergency_resolution: writable,
    },
    queue_emergency_resolution_v2_instruction,
    QueueEmergencyResolutionV2
);

closed_builder!(
    ExecuteEmergencyResolutionV2Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: writable,
        capacity_policy: readonly,
        current_deployment: writable,
        emergency_resolution: writable,
        emergency_freeze_observation: readonly,
        programdata_observation: readonly,
        emergency_checkpoint: readonly,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        instructions_sysvar: readonly,
    },
    execute_emergency_resolution_v2_instruction,
    ExecuteEmergencyResolutionV2
);

closed_builder!(
    ExpireEmergencyResolutionV2Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        emergency_resolution: writable,
    },
    expire_emergency_resolution_v2_instruction,
    ExpireEmergencyResolutionV2
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateCheckpointV2Accounts {
    pub payer: Pubkey,
    pub controller_config: Pubkey,
    pub policy: Pubkey,
    pub current_council: Pubkey,
    pub protocol_gate: Pubkey,
    pub subject: Pubkey,
    pub linked_primary_or_authority: Pubkey,
    pub capacity_policy: Pubkey,
    pub current_deployment: Pubkey,
    pub programdata_observation: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub checkpoint: Pubkey,
    pub checkpoint_attestation: Pubkey,
    pub seat_authority: Pubkey,
    pub system_program: Pubkey,
}

pub fn create_checkpoint_v2_instruction(
    controller_program: Pubkey,
    accounts: CreateCheckpointV2Accounts,
    instruction: CreateCheckpointV2,
) -> Result<Instruction, ProgramError> {
    Ok(Instruction {
        program_id: controller_program,
        accounts: vec![
            account_meta!(accounts.payer, signer_writable),
            account_meta!(accounts.controller_config, readonly),
            account_meta!(accounts.policy, readonly),
            account_meta!(accounts.current_council, readonly),
            account_meta!(accounts.protocol_gate, readonly),
            account_meta!(accounts.subject, readonly),
            account_meta!(accounts.linked_primary_or_authority, readonly),
            account_meta!(accounts.capacity_policy, readonly),
            account_meta!(accounts.current_deployment, readonly),
            account_meta!(accounts.programdata_observation, readonly),
            account_meta!(accounts.target_program, readonly),
            account_meta!(accounts.target_programdata, readonly),
            account_meta!(accounts.checkpoint, readonly),
            account_meta!(accounts.checkpoint_attestation, writable),
            account_meta!(accounts.seat_authority, signer_readonly),
            account_meta!(accounts.system_program, readonly),
        ],
        data: instruction.pack()?,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecastCheckpointV2Accounts {
    pub controller_config: Pubkey,
    pub policy: Pubkey,
    pub current_council: Pubkey,
    pub protocol_gate: Pubkey,
    pub subject: Pubkey,
    pub linked_primary_or_authority: Pubkey,
    pub capacity_policy: Pubkey,
    pub current_deployment: Pubkey,
    pub programdata_observation: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub checkpoint: Pubkey,
    pub checkpoint_attestation: Pubkey,
    pub seat_authority: Pubkey,
}

pub fn recast_checkpoint_v2_instruction(
    controller_program: Pubkey,
    accounts: RecastCheckpointV2Accounts,
    instruction: RecastCheckpointV2,
) -> Result<Instruction, ProgramError> {
    Ok(Instruction {
        program_id: controller_program,
        accounts: vec![
            account_meta!(accounts.controller_config, readonly),
            account_meta!(accounts.policy, readonly),
            account_meta!(accounts.current_council, readonly),
            account_meta!(accounts.protocol_gate, readonly),
            account_meta!(accounts.subject, readonly),
            account_meta!(accounts.linked_primary_or_authority, readonly),
            account_meta!(accounts.capacity_policy, readonly),
            account_meta!(accounts.current_deployment, readonly),
            account_meta!(accounts.programdata_observation, readonly),
            account_meta!(accounts.target_program, readonly),
            account_meta!(accounts.target_programdata, readonly),
            account_meta!(accounts.checkpoint, readonly),
            account_meta!(accounts.checkpoint_attestation, writable),
            account_meta!(accounts.seat_authority, signer_readonly),
        ],
        data: instruction.pack()?,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalizeCheckpointV2Accounts {
    pub payer: Pubkey,
    pub controller_config: Pubkey,
    pub policy: Pubkey,
    pub current_council: Pubkey,
    pub protocol_gate: Pubkey,
    pub subject: Pubkey,
    pub linked_primary_or_authority: Pubkey,
    pub capacity_policy: Pubkey,
    pub current_deployment: Pubkey,
    pub programdata_observation: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub checkpoint: Pubkey,
    pub checkpoint_attestations: [Pubkey; 3],
    pub system_program: Pubkey,
}

pub fn finalize_checkpoint_v2_instruction(
    controller_program: Pubkey,
    accounts: FinalizeCheckpointV2Accounts,
    instruction: FinalizeCheckpointV2,
) -> Result<Instruction, ProgramError> {
    let subject = if matches!(
        instruction.manifest.phase,
        StateCheckpointPhaseV1::Poststate | StateCheckpointPhaseV1::Emergency
    ) {
        account_meta!(accounts.subject, writable)
    } else {
        account_meta!(accounts.subject, readonly)
    };
    let mut metas = vec![
        account_meta!(accounts.payer, signer_writable),
        account_meta!(accounts.controller_config, readonly),
        account_meta!(accounts.policy, readonly),
        account_meta!(accounts.current_council, readonly),
        account_meta!(accounts.protocol_gate, readonly),
        subject,
        account_meta!(accounts.linked_primary_or_authority, readonly),
        account_meta!(accounts.capacity_policy, readonly),
        account_meta!(accounts.current_deployment, readonly),
        account_meta!(accounts.programdata_observation, readonly),
        account_meta!(accounts.target_program, readonly),
        account_meta!(accounts.target_programdata, readonly),
        account_meta!(accounts.checkpoint, writable),
    ];
    metas.extend(
        accounts
            .checkpoint_attestations
            .into_iter()
            .map(|attestation| account_meta!(attestation, readonly)),
    );
    metas.push(account_meta!(accounts.system_program, readonly));
    Ok(Instruction {
        program_id: controller_program,
        accounts: metas,
        data: instruction.pack()?,
    })
}

closed_builder!(
    BindProgramDataVerificationV2Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        programdata_observation: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        programdata_verification: writable,
        system_program: readonly,
    },
    bind_programdata_verification_v2_instruction,
    BindProgramDataVerificationV2
);

closed_builder!(
    FinalizeProgramDataVerificationV2Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        capacity_policy: readonly,
        current_deployment: readonly,
        programdata_observation: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        programdata_verification: writable,
    },
    finalize_programdata_verification_v2_instruction,
    FinalizeProgramDataVerificationV2
);

closed_builder!(
    ObserveProgramDataFailureV2Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        primary_proposal: readonly,
        programdata_verification: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        programdata_observation: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        failure_observation: writable,
        system_program: readonly,
    },
    observe_programdata_failure_v2_instruction,
    ObserveProgramDataFailureV2
);

closed_builder!(
    ApproveUnfreezeV2Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: readonly,
        proposal: writable,
        poststate_checkpoint: readonly,
        programdata_verification: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        seat_authority: signer_readonly,
    },
    approve_unfreeze_v2_instruction,
    ApproveUnfreezeV2
);

closed_builder!(
    ExecuteUnfreezeV2Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: writable,
        proposal: writable,
        linked_proposal: writable,
        poststate_checkpoint: readonly,
        programdata_verification: readonly,
        capacity_policy: readonly,
        current_deployment: writable,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        instructions_sysvar: readonly,
    },
    execute_unfreeze_v2_instruction,
    ExecuteUnfreezeV2
);

fn require_hashes(values: &[[u8; 32]]) -> Result<(), ProgramError> {
    if values.contains(&[0; 32]) {
        Err(ProgramError::InvalidInstructionData)
    } else {
        Ok(())
    }
}

fn validate_approval_pair(bitset: u8, count: u8) -> Result<(), ProgramError> {
    if bitset & !VALID_APPROVAL_MASK != 0 || bitset.count_ones() as u8 != count {
        Err(ProgramError::InvalidInstructionData)
    } else {
        Ok(())
    }
}

fn require_proposal_state(
    guard: &ProposalGuardV3,
    expected: ProposalStateV2,
) -> Result<(), ProgramError> {
    if guard.expected_state == expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidInstructionData)
    }
}

fn require_resolution_state(
    guard: &EmergencyResolutionGuardV2,
    expected: EmergencyFreezeResolutionStateV1,
) -> Result<(), ProgramError> {
    if guard.expected_state == expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidInstructionData)
    }
}

fn is_prefreeze_proposal_state(state: ProposalStateV2) -> bool {
    matches!(
        state,
        ProposalStateV2::Draft
            | ProposalStateV2::BufferAdopted
            | ProposalStateV2::BufferVerified
            | ProposalStateV2::CouncilApproved
            | ProposalStateV2::GovernanceSatisfied
            | ProposalStateV2::Timelocked
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        release1_ceremony_state::ProgramDataObservationPurposeV1, state::OptionalPubkeyV1,
    };

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn proposal_guard(state: ProposalStateV2) -> ProposalGuardV3 {
        ProposalGuardV3 {
            expected_proposal_digest: [1; 32],
            expected_state: state,
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: 2,
            expected_target_nonce: 3,
            expected_capacity_policy_digest: [4; 32],
            expected_current_deployment_digest: [5; 32],
            expected_current_deployment_generation: 1,
        }
    }

    fn resolution_guard(state: EmergencyFreezeResolutionStateV1) -> EmergencyResolutionGuardV2 {
        EmergencyResolutionGuardV2 {
            expected_resolution_digest: [1; 32],
            expected_state: state,
            expected_gate_status: GateStatusV1::EmergencyFrozen,
            expected_gate_epoch: 2,
            expected_target_nonce: 3,
            expected_capacity_policy_digest: [4; 32],
            expected_current_deployment_digest: [5; 32],
            expected_current_deployment_generation: 1,
            expected_freeze_observation_digest: [6; 32],
            expected_programdata_observation_digest: [7; 32],
            expected_observation_generation: 1,
            expected_checkpoint_digest: [8; 32],
        }
    }

    fn proposal_manifest() -> ProposalManifestV3 {
        ProposalManifestV3 {
            proposal_class: ProposalClassV1::RoutineUpgrade,
            expected_proposal_id: 1,
            expected_target_nonce: 1,
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: 1,
            expected_capacity_policy_digest: [1; 32],
            expected_current_deployment_digest: [2; 32],
            expected_current_deployment_generation: 1,
            expected_policy_version: 1,
            expected_policy_hash: [3; 32],
            expected_council_version: 1,
            expected_council_hash: [4; 32],
            artifact_length: 16_384,
            artifact_sha256: [5; 32],
            artifact_chunk_merkle_root: [6; 32],
            source_commit_hash: [7; 32],
            source_tree_hash: [8; 32],
            build_input_inventory_hash: [9; 32],
            reproducible_build_receipt_hash: [10; 32],
            package_receipt_hash: [11; 32],
            release_intent_hash: [12; 32],
            minimum_required_capacity: 16_384,
            checkpoint_schema_id: [13; 32],
            checkpoint_policy_hash: [14; 32],
            primary_proposal: OptionalPubkeyV1::none(),
            rollback_proposal: OptionalPubkeyV1::some(key(15)).unwrap(),
            rollback_buffer: OptionalPubkeyV1::some(key(16)).unwrap(),
            rollback_artifact_length: 16_384,
            rollback_artifact_sha256: [17; 32],
            rollback_artifact_chunk_root: [18; 32],
            plan_valid_until_slot: 1_000,
        }
    }

    fn checkpoint_manifest(phase: StateCheckpointPhaseV1) -> CheckpointManifestV2 {
        CheckpointManifestV2 {
            phase,
            checkpoint_generation: 1,
            previous_checkpoint_digest: [0; 32],
            expected_subject_digest: [1; 32],
            expected_gate_epoch: 3,
            expected_capacity_policy_digest: [2; 32],
            expected_current_deployment_digest: [3; 32],
            expected_current_deployment_generation: 1,
            expected_observation_digest: [4; 32],
            expected_observation_generation: 1,
            program_owned_state_root: [5; 32],
            program_owned_state_count: 1,
            logical_compressed_state_root: [6; 32],
            logical_compressed_state_count: 1,
            semantic_custody_accounting_root: [7; 32],
            hard_combined_root: [8; 32],
            external_metadata_observation_root: [9; 32],
            external_raw_balance_observation_root: [10; 32],
            schema_identifier: [11; 32],
            admitted_positive_donation_root: [0; 32],
            admitted_positive_donation_count: 0,
            forbidden_drift_count: 0,
            expected_council_version: 1,
            expected_council_hash: [12; 32],
            expected_checkpoint_digest: [13; 32],
            plan_valid_until_slot: 1_000,
        }
    }

    fn checkpoint_attestation(
        phase: StateCheckpointPhaseV1,
        previous_attestation_digest: [u8; 32],
    ) -> CheckpointAttestationGuardV2 {
        CheckpointAttestationGuardV2 {
            manifest: checkpoint_manifest(phase),
            seat_index: 1,
            expected_previous_attestation_digest: previous_attestation_digest,
        }
    }

    fn unfreeze_guard(count: u8) -> UnfreezeGuardV2 {
        UnfreezeGuardV2 {
            expected_proposal_digest: [1; 32],
            expected_checkpoint_digest: [2; 32],
            expected_checkpoint_generation: 1,
            expected_verification_digest: [3; 32],
            expected_verification_generation: 1,
            expected_original_council_version: 1,
            expected_original_council_hash: [4; 32],
            expected_current_council_version: 2,
            expected_current_council_hash: [5; 32],
            expected_gate_epoch: 3,
            expected_target_nonce: 4,
            expected_current_deployment_digest: [6; 32],
            expected_current_deployment_generation: 1,
            expected_artifact_sha256: [7; 32],
            expected_artifact_merkle_root: [8; 32],
            expected_actual_capacity: 1_048_576,
            expected_approval_bitset: if count == 3 { 0b00111 } else { 0b00011 },
            expected_approval_count: count,
        }
    }

    fn envelope() -> CeremonyEnvelopeV1 {
        CeremonyEnvelopeV1 {
            compute_unit_limit: 1_000_000,
            compute_unit_price_micro_lamports: 1,
            durable_nonce_account: OptionalPubkeyV1::none(),
            durable_nonce_authority: OptionalPubkeyV1::none(),
        }
    }

    macro_rules! assert_codec {
        ($value:expr, $type:ty) => {{
            let value = $value;
            let encoded = value.pack().unwrap();
            assert_eq!(encoded.len(), <$type>::LEN);
            assert_eq!(<$type>::unpack(&encoded).unwrap(), value);
            for length in 0..encoded.len() {
                assert!(<$type>::unpack(&encoded[..length]).is_err());
            }
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(<$type>::unpack(&trailing).is_err());
            let mut wrong_tag = encoded;
            wrong_tag[0] = 0;
            assert!(<$type>::unpack(&wrong_tag).is_err());
        }};
    }

    #[test]
    fn tags_are_contiguous_after_ceremony_and_never_overlap_39_through_52() {
        let tags = [
            INITIALIZE_CONTROLLER_V2_TAG,
            CREATE_PROPOSAL_V3_TAG,
            APPROVE_PROPOSAL_V3_TAG,
            FINALIZE_GOVERNANCE_V3_TAG,
            QUEUE_PROPOSAL_V3_TAG,
            FREEZE_PROPOSAL_V3_TAG,
            CANCEL_PROPOSAL_V3_TAG,
            EXPIRE_PROPOSAL_V3_TAG,
            GUARDIAN_FREEZE_V2_TAG,
            CREATE_EMERGENCY_RESOLUTION_V2_TAG,
            APPROVE_EMERGENCY_RESOLUTION_V2_TAG,
            QUEUE_EMERGENCY_RESOLUTION_V2_TAG,
            EXECUTE_EMERGENCY_RESOLUTION_V2_TAG,
            EXPIRE_EMERGENCY_RESOLUTION_V2_TAG,
            CREATE_CHECKPOINT_V2_TAG,
            RECAST_CHECKPOINT_V2_TAG,
            FINALIZE_CHECKPOINT_V2_TAG,
            BIND_PROGRAMDATA_VERIFICATION_V2_TAG,
            FINALIZE_PROGRAMDATA_VERIFICATION_V2_TAG,
            OBSERVE_PROGRAMDATA_FAILURE_V2_TAG,
            APPROVE_UNFREEZE_V2_TAG,
            EXECUTE_UNFREEZE_V2_TAG,
        ];
        assert_eq!(
            tags,
            core::array::from_fn::<_, 22, _>(|index| 53 + index as u8)
        );
        assert!(tags.iter().all(|tag| !(39..=52).contains(tag)));
    }

    #[test]
    fn representative_codecs_are_exact_and_semantically_strict() {
        assert_codec!(
            CreateProposalV3 {
                manifest: proposal_manifest()
            },
            CreateProposalV3
        );
        assert_codec!(
            QueueProposalV3 {
                expected: proposal_guard(ProposalStateV2::GovernanceSatisfied)
            },
            QueueProposalV3
        );
        assert_codec!(
            FreezeProposalV3 {
                expected: proposal_guard(ProposalStateV2::Timelocked),
                expected_next_gate_epoch: 3,
            },
            FreezeProposalV3
        );
        assert_codec!(
            QueueEmergencyResolutionV2 {
                expected: resolution_guard(EmergencyFreezeResolutionStateV1::CouncilApproved)
            },
            QueueEmergencyResolutionV2
        );
        assert_codec!(
            FinalizeCheckpointV2 {
                manifest: checkpoint_manifest(StateCheckpointPhaseV1::Poststate)
            },
            FinalizeCheckpointV2
        );
        assert_codec!(
            ApproveUnfreezeV2 {
                expected: unfreeze_guard(2)
            },
            ApproveUnfreezeV2
        );
        assert_codec!(
            ExecuteUnfreezeV2 {
                expected: unfreeze_guard(3),
                linked_proposal: key(9),
                envelope: envelope(),
            },
            ExecuteUnfreezeV2
        );

        let mut bad = QueueProposalV3 {
            expected: proposal_guard(ProposalStateV2::GovernanceSatisfied),
        }
        .pack()
        .unwrap();
        bad[1 + 32] = 0xff;
        assert_eq!(
            QueueProposalV3::unpack(&bad),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn every_instruction_length_is_fixed_and_under_the_global_cap() {
        let lengths = [
            InitializeControllerV2::LEN,
            CreateProposalV3::LEN,
            ApproveProposalV3::LEN,
            FinalizeGovernanceV3::LEN,
            QueueProposalV3::LEN,
            FreezeProposalV3::LEN,
            CancelProposalV3::LEN,
            ExpireProposalV3::LEN,
            GuardianFreezeV2::LEN,
            CreateEmergencyResolutionV2::LEN,
            ApproveEmergencyResolutionV2::LEN,
            QueueEmergencyResolutionV2::LEN,
            ExecuteEmergencyResolutionV2::LEN,
            ExpireEmergencyResolutionV2::LEN,
            CreateCheckpointV2::LEN,
            RecastCheckpointV2::LEN,
            FinalizeCheckpointV2::LEN,
            BindProgramDataVerificationV2::LEN,
            FinalizeProgramDataVerificationV2::LEN,
            ObserveProgramDataFailureV2::LEN,
            ApproveUnfreezeV2::LEN,
            ExecuteUnfreezeV2::LEN,
        ];
        assert!(lengths.iter().all(|length| *length > 1 && *length < 1_232));
        assert_eq!(CreateProposalV3::PAYLOAD_LEN, ProposalManifestV3::LEN);
        assert_eq!(InitializeControllerV2::PAYLOAD_LEN, 632);
        assert!(std::hint::black_box(InitializeControllerV2::LEN) < 1_232);
        assert_eq!(INITIALIZE_CONTROLLER_V2_ACCOUNT_COUNT, 22);
        assert_eq!(
            CreateCheckpointV2::PAYLOAD_LEN,
            CheckpointAttestationGuardV2::LEN
        );
        assert_eq!(
            BindProgramDataVerificationV2::PAYLOAD_LEN,
            ProgramDataVerificationManifestV2::LEN
        );
        assert_eq!(UnfreezeGuardV2::LEN, 322);
    }

    #[test]
    fn closed_builders_freeze_order_and_privileges() {
        let queue = queue_proposal_v3_instruction(
            key(1),
            QueueProposalV3Accounts {
                controller_config: key(2),
                policy: key(3),
                protocol_gate: key(4),
                capacity_policy: key(5),
                current_deployment: key(6),
                proposal: key(7),
            },
            QueueProposalV3 {
                expected: proposal_guard(ProposalStateV2::GovernanceSatisfied),
            },
        )
        .unwrap();
        assert_eq!(queue.accounts.len(), 6);
        assert!(queue.accounts[..5]
            .iter()
            .all(|meta| !meta.is_signer && !meta.is_writable));
        assert!(queue.accounts[5].is_writable && !queue.accounts[5].is_signer);

        let create = create_checkpoint_v2_instruction(
            key(1),
            CreateCheckpointV2Accounts {
                payer: key(2),
                controller_config: key(3),
                policy: key(4),
                current_council: key(5),
                protocol_gate: key(6),
                subject: key(7),
                linked_primary_or_authority: key(8),
                capacity_policy: key(9),
                current_deployment: key(10),
                programdata_observation: key(11),
                target_program: key(12),
                target_programdata: key(13),
                checkpoint: key(14),
                checkpoint_attestation: key(15),
                seat_authority: key(16),
                system_program: key(17),
            },
            CreateCheckpointV2 {
                attestation: checkpoint_attestation(StateCheckpointPhaseV1::Prestate, [0; 32]),
            },
        )
        .unwrap();
        assert_eq!(create.accounts.len(), 16);
        assert!(create.accounts[0].is_signer && create.accounts[0].is_writable);
        assert!(!create.accounts[6].is_signer && !create.accounts[6].is_writable);
        assert!(create.accounts[13].is_writable && !create.accounts[13].is_signer);
        assert!(create.accounts[14].is_signer && !create.accounts[14].is_writable);

        let recast = recast_checkpoint_v2_instruction(
            key(1),
            RecastCheckpointV2Accounts {
                controller_config: key(2),
                policy: key(3),
                current_council: key(4),
                protocol_gate: key(5),
                subject: key(6),
                linked_primary_or_authority: key(7),
                capacity_policy: key(8),
                current_deployment: key(9),
                programdata_observation: key(10),
                target_program: key(11),
                target_programdata: key(12),
                checkpoint: key(13),
                checkpoint_attestation: key(14),
                seat_authority: key(15),
            },
            RecastCheckpointV2 {
                attestation: checkpoint_attestation(StateCheckpointPhaseV1::Prestate, [16; 32]),
            },
        )
        .unwrap();
        assert_eq!(recast.accounts.len(), 14);
        assert!(!recast.accounts[5].is_signer && !recast.accounts[5].is_writable);
        assert!(recast.accounts[12].is_writable && !recast.accounts[12].is_signer);
        assert!(recast.accounts[13].is_signer && !recast.accounts[13].is_writable);

        let finalize = finalize_checkpoint_v2_instruction(
            key(1),
            FinalizeCheckpointV2Accounts {
                payer: key(2),
                controller_config: key(3),
                policy: key(4),
                current_council: key(5),
                protocol_gate: key(6),
                subject: key(7),
                linked_primary_or_authority: key(8),
                capacity_policy: key(9),
                current_deployment: key(10),
                programdata_observation: key(11),
                target_program: key(12),
                target_programdata: key(13),
                checkpoint: key(14),
                checkpoint_attestations: [key(15), key(16), key(17)],
                system_program: key(18),
            },
            FinalizeCheckpointV2 {
                manifest: checkpoint_manifest(StateCheckpointPhaseV1::Poststate),
            },
        )
        .unwrap();
        assert_eq!(finalize.accounts.len(), 17);
        assert!(finalize.accounts[0].is_signer && finalize.accounts[0].is_writable);
        assert!(finalize.accounts[5].is_writable);
        assert!(!finalize.accounts[6].is_signer && !finalize.accounts[6].is_writable);
        assert!(finalize.accounts[12].is_writable);

        let finalize_programdata = finalize_programdata_verification_v2_instruction(
            key(1),
            FinalizeProgramDataVerificationV2Accounts {
                controller_config: key(2),
                protocol_gate: key(3),
                proposal: key(4),
                capacity_policy: key(5),
                current_deployment: key(6),
                programdata_observation: key(7),
                target_program: key(8),
                target_programdata: key(9),
                authority_pda: key(10),
                upgradeable_loader: key(11),
                programdata_verification: key(12),
            },
            FinalizeProgramDataVerificationV2 {
                expected: ProgramDataVerificationGuardV2 {
                    expected_proposal_digest: [1; 32],
                    expected_verification_digest: [2; 32],
                    expected_verification_generation: 1,
                    expected_status: ProgramDataVerificationStatusV2::ObservationBound,
                    expected_gate_epoch: 3,
                    expected_target_nonce: 4,
                    expected_capacity_policy_digest: [5; 32],
                    expected_current_deployment_digest: [6; 32],
                    expected_current_deployment_generation: 1,
                    expected_observation_digest: [7; 32],
                    expected_observation_generation: 1,
                    expected_actual_capacity: 1_048_576,
                    expected_authority: key(13),
                },
            },
        )
        .unwrap();
        assert_eq!(finalize_programdata.accounts.len(), 11);
        assert!(finalize_programdata.accounts[2].is_writable);
        assert!(!finalize_programdata.accounts[4].is_writable);
        assert!(finalize_programdata.accounts[10].is_writable);

        let unfreeze = execute_unfreeze_v2_instruction(
            key(1),
            ExecuteUnfreezeV2Accounts {
                controller_config: key(2),
                policy: key(3),
                current_council: key(4),
                protocol_gate: key(5),
                proposal: key(6),
                linked_proposal: key(7),
                poststate_checkpoint: key(8),
                programdata_verification: key(9),
                capacity_policy: key(10),
                current_deployment: key(11),
                target_program: key(12),
                target_programdata: key(13),
                authority_pda: key(14),
                upgradeable_loader: key(15),
                instructions_sysvar: key(16),
            },
            ExecuteUnfreezeV2 {
                expected: unfreeze_guard(3),
                linked_proposal: key(7),
                envelope: envelope(),
            },
        )
        .unwrap();
        assert_eq!(unfreeze.accounts.len(), 15);
        assert!(unfreeze.accounts[3].is_writable);
        assert!(unfreeze.accounts[4].is_writable);
        assert!(unfreeze.accounts[5].is_writable);
        assert!(unfreeze.accounts[9].is_writable);
        assert!(unfreeze.accounts.iter().all(|meta| !meta.is_signer));
    }

    #[test]
    fn purpose_enum_remains_strict_in_the_underlying_observation_contract() {
        assert_eq!(ProgramDataObservationPurposeV1::ProposalPrestate as u8, 2);
    }
}
