//! Closed Release 1 checkpoint, council-rotation, and expiry processors.
//!
//! Every public entry point validates the complete account contract and builds
//! the final account bytes before the first write.  Canonical checkpoints are
//! never stored in a draft state: seats attest through independent PDAs and a
//! permissionless finalizer creates the checkpoint only after observing three
//! byte-identical, current-council attestations.

use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
};
use solana_sdk_ids::system_program;

use crate::{
    council::{
        compute_council_set_hash, record_seat_approval, validate_council_guardian_separation,
        validate_council_set, VALID_APPROVAL_MASK,
    },
    instruction::{
        ActivateCouncilRotationV1, ApproveCouncilRotationV1, CancelCouncilRotationV1,
        CheckpointCandidateV1, CheckpointSubjectStateV1, CouncilRotationExpectationV1,
        CreateCandidateCouncilSetV1, CreateCheckpointAttestationV1, CreateCouncilRotationV1,
        ExpireCouncilRotationV1, ExpireEmergencyResolutionV1, FinalizeCheckpointV1,
        QueueCouncilRotationV1, RecastCheckpointAttestationV1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_attestation_pda,
        derive_checkpoint_pda, derive_controller_config_pda, derive_council_pda,
        derive_emergency_checkpoint_pda, derive_emergency_freeze_observation_pda,
        derive_emergency_resolution_pda, derive_gate_pda, derive_policy_pda,
        derive_programdata_check_pda, derive_programdata_failure_observation_pda,
        derive_proposal_pda, derive_upgradeable_programdata_address, CHECKPOINT_ATTESTATION_SEED,
        CHECKPOINT_SEED, COUNCIL_ROTATION_SEED, COUNCIL_SEED, EMERGENCY_CHECKPOINT_SEED,
        UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, store_fixed_controller_account, validate_exact_privileges,
    },
    release1_digest::{
        compute_checkpoint_attestation_digest_v1, compute_council_rotation_digest_v1,
        compute_state_checkpoint_digest_v1, compute_state_checkpoint_hard_combined_root_v1,
        validate_checkpoint_attestation_digest_v1, validate_council_rotation_digest_v1,
        validate_emergency_freeze_observation_digest_v1, validate_emergency_resolution_digest_v1,
        validate_programdata_failure_observation_digest_v1, validate_proposal_digest_v2,
        validate_state_checkpoint_digest_v1,
    },
    release1_loader_accounts::{loader_account_data_hash, validate_program_programdata_linkage},
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, CheckpointAttestationV1,
        CouncilRotationProposalV1, CouncilRotationStateV1, EmergencyFreezeObservationV1,
        EmergencyFreezeResolutionStateV1, EmergencyFreezeResolutionV1,
        ProgramDataFailureObservationV1, ProgramDataMismatchClassV1,
        ProgramDataVerificationStatusV1, ProgramDataVerificationV1, ProposalStateV2,
        StateCheckpointPhaseV1, StateCheckpointV1, UpgradeProposalV2,
        CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR, CHECKPOINT_ATTESTATION_V1_RESERVED_LEN,
        COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1, COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1,
        COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR, COUNCIL_ROTATION_PROPOSAL_V1_RESERVED_LEN,
        EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1, LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
        LOADER_V3_PROGRAM_ACCOUNT_LEN_V1, RELEASE1_ACCOUNT_VERSION_V1, RELEASE1_APPROVAL_THRESHOLD,
        STATE_CHECKPOINT_V1_DISCRIMINATOR, STATE_CHECKPOINT_V1_RESERVED_LEN,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernancePolicyV1, ProposalClassV1, ProtocolGateV1, ACCOUNT_VERSION_V1,
        COUNCIL_SEAT_RESERVED_LEN, GOVERNANCE_COUNCIL_DISCRIMINATOR,
        GOVERNANCE_COUNCIL_RESERVED_LEN,
    },
    GovernanceError, GovernanceResult,
};

const EXACT_ROUTINE_APPROVAL_MASK_COUNT: u8 = RELEASE1_APPROVAL_THRESHOLD;

#[derive(Debug)]
enum CheckpointSubject {
    Proposal(Box<UpgradeProposalV2>),
    Emergency(Box<EmergencyFreezeResolutionV1>),
}

#[derive(Debug)]
struct CheckpointBinding {
    subject: CheckpointSubject,
    subject_key: Pubkey,
    subject_digest: [u8; 32],
    checkpoint: Pubkey,
    checkpoint_bump: u8,
}

struct CheckpointSubjectContext<'a, 'info> {
    program_id: &'a Pubkey,
    subject_info: &'a AccountInfo<'info>,
    checkpoint_info: &'a AccountInfo<'info>,
    config_key: &'a Pubkey,
    config: &'a ControllerConfigV1,
    gate_key: &'a Pubkey,
    gate: &'a ProtocolGateV1,
    candidate: &'a CheckpointCandidateV1,
}

#[derive(Clone, Copy)]
struct CheckpointFinalizationFields {
    council_version: u64,
    council_hash: [u8; 32],
    approval_bitset: u8,
    finalized_slot: u64,
}

#[derive(Clone, Copy)]
struct RollbackPrestateCommitment {
    primary_proposal: Pubkey,
    expected_candidate_full_payload_sha256: [u8; 32],
    artifact_chunk_merkle_root: [u8; 32],
    chunk_hash_domain: [u8; 32],
    capacity: u64,
}

struct AttestationBuildContext<'a> {
    program_id: &'a Pubkey,
    attestation_bump: u8,
    checkpoint: Pubkey,
    subject: Pubkey,
    candidate: &'a CheckpointCandidateV1,
    council_key: Pubkey,
    council: &'a GovernanceCouncilSetV1,
    seat_index: u8,
    seat_authority: Pubkey,
    slot: u64,
    config_key: Pubkey,
}

struct AttestationFinalizationContext<'a> {
    program_id: &'a Pubkey,
    checkpoint: &'a StateCheckpointV1,
    checkpoint_key: &'a Pubkey,
    subject_key: &'a Pubkey,
    council_key: &'a Pubkey,
    council: &'a GovernanceCouncilSetV1,
    minimum_slot: u64,
    finalization_slot: u64,
}

struct RotationAccountContext<'a, 'info> {
    program_id: &'a Pubkey,
    config_info: &'a AccountInfo<'info>,
    policy_info: &'a AccountInfo<'info>,
    current_council_info: &'a AccountInfo<'info>,
    candidate_info: &'a AccountInfo<'info>,
    gate_info: &'a AccountInfo<'info>,
    rotation_info: &'a AccountInfo<'info>,
    expected: &'a CouncilRotationExpectationV1,
    slot: u64,
}

struct LoadedRotationContext {
    config: Box<ControllerConfigV1>,
    current_council: Box<GovernanceCouncilSetV1>,
    candidate: Box<GovernanceCouncilSetV1>,
    rotation: Box<CouncilRotationProposalV1>,
}

#[cfg(test)]
mod tests;

mod account_context;
mod approval;
mod attestations;
mod council;
mod emergency;
mod evidence;
mod finalization;
mod rotation;
mod rotation_context;
mod subject;

use account_context::{
    all_distinct, current_slot, exact_account_count, load_config, load_current_council, load_gate,
    load_policy, require_absent_system_account, validate_candidate_creation_authority_contract,
    validate_readonly, validate_signer_readonly, validate_signer_writable, validate_system_program,
    validate_writable,
};
use approval::{
    validate_approval_mask_at, validate_checkpoint_council_guard, validate_current_seat_at_index,
    validate_gate_guard,
};
#[cfg(test)]
use attestations::validate_checkpoint_acceptance_shape;
use attestations::validate_checkpoint_candidate;
pub use attestations::{
    process_create_checkpoint_attestation_v1, process_recast_checkpoint_attestation_v1,
};
pub use council::process_create_candidate_council_set_v1;
use council::{
    require_active_seat_authority, validate_candidate_seats_at_slot, validate_mask_members_active,
};
pub use emergency::process_expire_emergency_resolution_v1;
#[cfg(test)]
use emergency::validate_emergency_expiry_expectation;
use evidence::{
    validate_phase_evidence, validate_poststate_baseline, validate_rollback_prestate_baseline,
    validate_target_snapshot,
};
#[cfg(test)]
use evidence::{
    validate_poststate_hard_invariants, validate_poststate_outcome,
    validate_rollback_prestate_evidence_commitment,
};
pub use finalization::process_finalize_checkpoint_v1;
pub use rotation::{
    process_activate_council_rotation_v1, process_approve_council_rotation_v1,
    process_cancel_council_rotation_v1, process_expire_council_rotation_v1,
    process_queue_council_rotation_v1,
};
pub use rotation_context::process_create_council_rotation_v1;
use rotation_context::{load_rotation, load_rotation_context, validate_rotation_expectation};
use subject::validate_checkpoint_subject;
