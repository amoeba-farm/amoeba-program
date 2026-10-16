//! Capacity-safe Release 1 proposal, guardian, emergency-resolution, and
//! checkpoint processors.
//!
//! This module deliberately contains no Loader mutation. Every entry point
//! validates its complete closed account contract, derives every consensus
//! identity from controller-owned state, prepares exact fixed-width bytes, and
//! performs the first account write only after all fallible checks complete.

use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    hash::hashv,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::{instructions, Sysvar},
};
use solana_sdk_ids::{compute_budget, system_program, sysvar as sysvar_ids};
use solana_system_interface::instruction as system_instruction;

use crate::{
    artifact_merkle::artifact_chunk_count,
    authorization::validate_seat_authority,
    council::{
        record_seat_approval, validate_council_guardian_separation, validate_council_set,
        VALID_APPROVAL_MASK,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_capacity_policy_pda,
        derive_checkpoint_attestation_pda, derive_checkpoint_pda, derive_controller_config_pda,
        derive_council_pda, derive_current_deployment_state_pda,
        derive_emergency_checkpoint_v2_pda, derive_emergency_freeze_observation_pda,
        derive_emergency_resolution_v2_pda, derive_gate_pda, derive_policy_pda,
        derive_programdata_check_pda, derive_programdata_observation_pda, derive_proposal_pda,
        derive_upgradeable_programdata_address, CHECKPOINT_ATTESTATION_SEED, CHECKPOINT_SEED,
        EMERGENCY_CHECKPOINT_V2_SEED, EMERGENCY_FREEZE_OBSERVATION_SEED,
        EMERGENCY_RESOLUTION_V2_SEED, PROPOSAL_SEED, UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        load_upgrade_proposal_v3, require_distinct_accounts, store_fixed_controller_account,
        validate_exact_privileges,
    },
    release1_authority_instruction::{
        CeremonyEnvelopeV1, MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1,
        MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    release1_ceremony_digest::{
        compute_current_deployment_digest_v1, compute_programdata_observation_subject_digest_v1,
        validate_capacity_policy_digest_v1, validate_current_deployment_digest_v1,
        validate_programdata_observation_digest_v1,
    },
    release1_ceremony_state::{
        CurrentDeploymentStateV1, ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1,
        ProgramDataObservationStatusV1, ProgramDataObservationV1,
    },
    release1_digest::{
        compute_checkpoint_attestation_digest_v1, validate_checkpoint_attestation_digest_v1,
        STATE_CHECKPOINT_HARD_ROOT_DOMAIN_V1,
    },
    release1_loader_accounts::{
        validate_buffer_account, validate_program_programdata_linkage, LOADER_BUFFER_METADATA_LEN,
        LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, CheckpointAttestationV1,
        EmergencyFreezeResolutionKindV1, EmergencyFreezeResolutionStateV1, ProposalStateV2,
        StateCheckpointPhaseV1, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR, CHECKPOINT_ATTESTATION_V1_RESERVED_LEN,
        PROPOSAL_COMPLETED_TERMINAL_REASON_V1, PROPOSAL_EXPIRED_TERMINAL_REASON_V1,
        PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1,
        PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1, RELEASE1_ACCOUNT_VERSION_V1,
        RELEASE1_APPROVAL_THRESHOLD,
    },
    release1_v3_digest::{
        compute_emergency_freeze_observation_digest_v2,
        compute_emergency_freeze_resolution_digest_v2, compute_state_checkpoint_digest_v2,
        compute_upgrade_proposal_digest_v3, validate_emergency_freeze_observation_digest_v2,
        validate_emergency_freeze_resolution_digest_v2,
        validate_programdata_verification_digest_v2, validate_state_checkpoint_digest_v2,
        validate_upgrade_proposal_digest_v3,
    },
    release1_v3_instruction::{
        ApproveEmergencyResolutionV2, ApproveProposalV3, ApproveUnfreezeV2, CancelProposalV3,
        CheckpointManifestV2, CreateCheckpointV2, CreateEmergencyResolutionV2, CreateProposalV3,
        EmergencyResolutionGuardV2, ExecuteEmergencyResolutionV2, ExecuteUnfreezeV2,
        ExpireEmergencyResolutionV2, ExpireProposalV3, FinalizeCheckpointV2, FinalizeGovernanceV3,
        FreezeProposalV3, GuardianFreezeV2, ProposalGuardV3, QueueEmergencyResolutionV2,
        QueueProposalV3, RecastCheckpointV2, UnfreezeGuardV2,
    },
    release1_v3_state::{
        EmergencyFreezeObservationV2, EmergencyFreezeResolutionV2, ProgramDataVerificationStatusV2,
        ProgramDataVerificationV2, StateCheckpointV2, UpgradeProposalV3,
        CAPACITY_SAFE_ACCOUNT_VERSION_V2, CAPACITY_SAFE_ACCOUNT_VERSION_V3,
        EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN_ID,
        EMERGENCY_FREEZE_OBSERVATION_V2_DISCRIMINATOR,
        EMERGENCY_FREEZE_OBSERVATION_V2_RESERVED_LEN,
        EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID,
        EMERGENCY_FREEZE_RESOLUTION_V2_DISCRIMINATOR, EMERGENCY_FREEZE_RESOLUTION_V2_RESERVED_LEN,
        EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V2,
        EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V2, STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID,
        STATE_CHECKPOINT_V2_DISCRIMINATOR, STATE_CHECKPOINT_V2_RESERVED_LEN,
        UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID, UPGRADE_PROPOSAL_V3_DISCRIMINATOR,
        UPGRADE_PROPOSAL_V3_RESERVED_LEN,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProtocolGateV1, VoteRequirementV1,
    },
    GovernanceError, GovernanceResult,
};

/// Canonical reason used only when an approved V3 proposal consumes the target
/// nonce and crosses the freeze boundary.
pub const GOVERNED_UPGRADE_FREEZE_REASON_V3: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeProgramDataHeaderV2 {
    deployed_slot: u64,
    raw_data_length: u64,
    capacity: u64,
    authority: Option<Pubkey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProgramDataObservationExpectationV2 {
    artifact_length: u64,
    artifact_sha256: [u8; 32],
    artifact_merkle_root: [u8; 32],
    artifact_scheme_id: [u8; 32],
    deployed_slot: u64,
    exact_capacity: Option<u64>,
}

struct LifecycleContext {
    config: Box<ControllerConfigV1>,
    gate: Box<ProtocolGateV1>,
    capacity: Box<ProgramDataCapacityPolicyV1>,
    deployment: Box<CurrentDeploymentStateV1>,
}

type EmergencyEvidenceV2 = (
    Box<EmergencyFreezeObservationV2>,
    Box<ProgramDataObservationV1>,
    Box<StateCheckpointV2>,
);

enum CheckpointSubjectRecord {
    Proposal(Box<UpgradeProposalV3>),
    Emergency(Box<EmergencyFreezeResolutionV2>),
}

struct CheckpointBindingV2 {
    checkpoint: Pubkey,
    bump: u8,
    checkpoint_subject: Pubkey,
    subject_digest: [u8; 32],
    observation_subject: Pubkey,
    observation_subject_digest: [u8; 32],
    expected_observation: Option<Pubkey>,
    purpose: ProgramDataObservationPurposeV1,
    minimum_required_capacity: u64,
    observation_expectation: ProgramDataObservationExpectationV2,
    record: CheckpointSubjectRecord,
}

type PreparedCheckpointAttestation = (
    LifecycleContext,
    Box<GovernanceCouncilSetV1>,
    CheckpointBindingV2,
    Box<StateCheckpointV2>,
    u64,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnfreezeAccumulatorActionV2 {
    Continue,
    ResetForCurrentCouncil,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActivatedDeploymentEvidenceV2 {
    artifact_length: u64,
    artifact_sha256: [u8; 32],
    artifact_merkle_root: [u8; 32],
    artifact_scheme_id: [u8; 32],
    actual_capacity: u64,
    programdata_observation: Pubkey,
    observation_generation: u64,
    observation_root: [u8; 32],
    observation_digest: [u8; 32],
    deployed_slot: u64,
    installed_authority: Pubkey,
    source_commitment: [u8; 32],
    build_inputs_commitment: [u8; 32],
    package_commitment: [u8; 32],
    release_manifest_commitment: [u8; 32],
    release_commitment: Pubkey,
    release_commitment_digest: [u8; 32],
}

#[cfg(test)]
mod tests;

mod account_context;
mod activation;
mod checkpoint_evidence;
mod checkpoints;
mod emergency;
mod emergency_evidence;
mod envelope;
mod guardian;
mod lifecycle;
mod observations;
mod proposal_guards;
mod proposals;
mod unfreeze;
mod unfreeze_evidence;

use account_context::{
    all_distinct, checked_nonterminal_increment, current_slot, exact_account_count,
    executable_readonly, load_capacity_policy, load_config, load_council, load_current_council,
    load_current_deployment, load_gate, load_lifecycle_context, load_policy, load_proposal,
    read_runtime_programdata_header, readonly, require_guard_deployment,
    require_proposal_deployment_current, require_runtime_matches_trusted_deployment,
    signer_readonly, signer_writable, system_program_account, writable,
};
#[cfg(test)]
use activation::apply_activated_deployment_v2;
use activation::{
    build_activated_deployment_v2, commit_four_fixed_accounts, terminalize_unfreeze_pair_v3,
};
#[cfg(test)]
use checkpoint_evidence::emergency_checkpoint_subjects;
use checkpoint_evidence::{
    build_checkpoint_attestation, derive_checkpoint_candidate, load_checkpoint_subject_binding,
    prepare_checkpoint_attestation,
};
pub use checkpoints::{
    process_create_checkpoint_v2, process_finalize_checkpoint_v2, process_recast_checkpoint_v2,
};
pub use emergency::{
    process_approve_emergency_resolution_v2, process_create_emergency_resolution_v2,
    process_execute_emergency_resolution_v2, process_expire_emergency_resolution_v2,
    process_queue_emergency_resolution_v2,
};
use emergency_evidence::{
    check_emergency_resolution_guard, load_emergency_resolution, validate_emergency_evidence,
};
use envelope::validate_canonical_envelope;
pub use guardian::process_guardian_freeze_v2;
#[cfg(test)]
use lifecycle::require_freeze_execution_runway;
pub use lifecycle::{
    process_cancel_proposal_v3, process_expire_proposal_v3, process_freeze_proposal_v3,
};
use observations::{
    candidate_observation_expectation, expected_programdata_observation_subject_digest,
    failed_primary_observation_expectation, load_emergency_freeze_observation,
    load_fresh_programdata_observation, require_observation_matches_runtime,
    trusted_observation_expectation,
};
use proposal_guards::{
    check_proposal_guard, derive_proposal_timing, is_prefreeze_state, require_approval_window,
    require_creation_gate, require_exact_recorded_quorum, verify_exact_proposal_timing,
};
use proposals::load_verified_buffer_for_proposal;
pub use proposals::{
    process_approve_proposal_v3, process_create_proposal_v3, process_finalize_governance_v3,
    process_queue_proposal_v3,
};
pub use unfreeze::{process_approve_unfreeze_v2, process_execute_unfreeze_v2};
#[cfg(test)]
use unfreeze_evidence::classify_unfreeze_accumulator_v2;
use unfreeze_evidence::{
    check_unfreeze_guard_v2, load_accepted_poststate_for_unfreeze_v2,
    load_verified_programdata_for_unfreeze_v2, prepare_unfreeze_accumulator_v2,
    require_current_unfreeze_quorum_v2, require_unfreeze_gate_binding,
    validate_live_verified_programdata_v2,
};
