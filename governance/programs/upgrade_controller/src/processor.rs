//! Release 1 instruction dispatcher.
//!
//! The historical tag-0 approval codec remains decodable for regression
//! vectors, but it is deliberately non-executable. The capacity-fragile
//! historical lifecycle and custody tags 1-17, 19-38, and 44-52 are likewise
//! decode-only scaffolds. Tag 18 remains executable because V2 rotation reuses
//! its immutable candidate-council account; tags 39-43 remain the observation
//! and controller-immutability surface.

use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    instruction::{self, MAX_CONTROLLER_INSTRUCTION_DATA_LEN},
    release1_authority_instruction, release1_ceremony_instruction, release1_governance_v2,
    release1_governance_v2_authority_processor::{
        process_approve_bootstrap_activation_proposal_v2,
        process_approve_target_authority_handoff_proposal_v2,
        process_cancel_bootstrap_activation_proposal_v2,
        process_cancel_target_authority_handoff_proposal_v2,
        process_create_bootstrap_activation_proposal_v2,
        process_create_target_authority_handoff_proposal_v2,
        process_execute_bootstrap_activation_proposal_v2,
        process_execute_target_authority_handoff_proposal_v2,
        process_expire_bootstrap_activation_proposal_v2,
        process_expire_target_authority_handoff_proposal_v2,
        process_queue_bootstrap_activation_proposal_v2,
        process_queue_target_authority_handoff_proposal_v2,
    },
    release1_governance_v2_processor::{
        process_approve_council_rotation_proposal_v2,
        process_approve_timing_policy_change_proposal_v1,
        process_cancel_council_rotation_proposal_v2,
        process_cancel_timing_policy_change_proposal_v1,
        process_create_council_rotation_proposal_v2, process_create_governance_timing_profile_v1,
        process_create_timing_policy_change_proposal_v1,
        process_execute_council_rotation_proposal_v2,
        process_execute_timing_policy_change_proposal_v1,
        process_expire_council_rotation_proposal_v2,
        process_expire_timing_policy_change_proposal_v1,
        process_initialize_governance_lifecycle_registry_v2,
        process_queue_council_rotation_proposal_v2, process_queue_timing_policy_change_proposal_v1,
    },
    release1_processor_authority::process_record_controller_immutability_v1,
    release1_processor_checkpoint::process_create_candidate_council_set_v1,
    release1_processor_initialize_v2::process_initialize_controller_v2,
    release1_processor_observation::{
        process_append_programdata_observation_chunk_v1, process_begin_programdata_observation_v1,
        process_finalize_programdata_observation_v1, process_verify_observed_artifact_chunk_v1,
    },
    release1_processor_v3_custody::{
        process_activate_rollback_v2, process_adopt_buffer_v2,
        process_bind_programdata_verification_v2, process_close_abandoned_buffer_v2,
        process_execute_upgrade_v2, process_extend_target_v2,
        process_finalize_buffer_verification_v2, process_finalize_programdata_verification_v2,
        process_observe_programdata_failure_witness_v2, process_verify_buffer_chunk_v2,
    },
    release1_processor_v3_lifecycle::{
        process_approve_emergency_resolution_v2, process_approve_proposal_v3,
        process_approve_unfreeze_v2, process_cancel_proposal_v3, process_create_checkpoint_v2,
        process_create_emergency_resolution_v2, process_create_proposal_v3,
        process_execute_emergency_resolution_v2, process_execute_unfreeze_v2,
        process_expire_emergency_resolution_v2, process_expire_proposal_v3,
        process_finalize_checkpoint_v2, process_finalize_governance_v3, process_freeze_proposal_v3,
        process_guardian_freeze_v2, process_queue_emergency_resolution_v2,
        process_queue_proposal_v3, process_recast_checkpoint_v2,
    },
    release1_v3_custody_instruction, release1_v3_instruction,
};

pub use crate::pda::UPGRADEABLE_LOADER_ID;

// Keep the root dispatcher as a tag router only. Decoding the complete
// `UpgradeControllerInstruction` enum here makes the largest fixed-wire variant
// live across every match arm under SBPF-v0, inflating the entry frame even for
// small lifecycle instructions. Each non-inlined thunk owns exactly one codec
// and one processor call, so unrelated instruction values never share a frame.
macro_rules! typed_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            let instruction = instruction::$instruction::unpack(instruction_data)?;
            $processor(program_id, accounts, instruction)
        }
    };
}

macro_rules! ceremony_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            let instruction =
                release1_ceremony_instruction::$instruction::unpack(instruction_data)?;
            $processor(program_id, accounts, instruction)
        }
    };
}

macro_rules! authority_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            let instruction =
                release1_authority_instruction::$instruction::unpack(instruction_data)?;
            $processor(program_id, accounts, instruction)
        }
    };
}

macro_rules! v3_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            let instruction = release1_v3_instruction::$instruction::unpack(instruction_data)?;
            $processor(program_id, accounts, instruction)
        }
    };
}

macro_rules! v3_boxed_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            // These instructions carry large, fixed manifests. Hand ownership
            // to the processor so the decoded value does not remain live in
            // both dispatcher and processor stack frames.
            let instruction = Box::new(release1_v3_instruction::$instruction::unpack(
                instruction_data,
            )?);
            $processor(program_id, accounts, instruction)
        }
    };
}

macro_rules! custody_v2_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            let instruction =
                release1_v3_custody_instruction::$instruction::unpack(instruction_data)?;
            $processor(program_id, accounts, instruction)
        }
    };
}

macro_rules! governance_v2_dispatch {
    ($name:ident, $instruction:ident, $processor:ident) => {
        #[inline(never)]
        fn $name(
            program_id: &Pubkey,
            accounts: &[AccountInfo<'_>],
            instruction_data: &[u8],
        ) -> ProgramResult {
            let instruction = release1_governance_v2::$instruction::unpack(instruction_data)?;
            $processor(program_id, accounts, instruction)
        }
    };
}

typed_dispatch!(
    dispatch_create_candidate_council_set_v1,
    CreateCandidateCouncilSetV1,
    process_create_candidate_council_set_v1
);
ceremony_dispatch!(
    dispatch_begin_programdata_observation_v1,
    BeginProgramDataObservationV1,
    process_begin_programdata_observation_v1
);
ceremony_dispatch!(
    dispatch_append_programdata_observation_chunk_v1,
    AppendProgramDataObservationChunkV1,
    process_append_programdata_observation_chunk_v1
);
ceremony_dispatch!(
    dispatch_verify_observed_artifact_chunk_v1,
    VerifyObservedArtifactChunkV1,
    process_verify_observed_artifact_chunk_v1
);
ceremony_dispatch!(
    dispatch_finalize_programdata_observation_v1,
    FinalizeProgramDataObservationV1,
    process_finalize_programdata_observation_v1
);
authority_dispatch!(
    dispatch_record_controller_immutability_v1,
    RecordControllerImmutabilityV1,
    process_record_controller_immutability_v1
);
v3_boxed_dispatch!(
    dispatch_initialize_controller_v2,
    InitializeControllerV2,
    process_initialize_controller_v2
);
v3_dispatch!(
    dispatch_create_proposal_v3,
    CreateProposalV3,
    process_create_proposal_v3
);
v3_dispatch!(
    dispatch_approve_proposal_v3,
    ApproveProposalV3,
    process_approve_proposal_v3
);
v3_dispatch!(
    dispatch_finalize_governance_v3,
    FinalizeGovernanceV3,
    process_finalize_governance_v3
);
v3_dispatch!(
    dispatch_queue_proposal_v3,
    QueueProposalV3,
    process_queue_proposal_v3
);
v3_dispatch!(
    dispatch_freeze_proposal_v3,
    FreezeProposalV3,
    process_freeze_proposal_v3
);
v3_dispatch!(
    dispatch_cancel_proposal_v3,
    CancelProposalV3,
    process_cancel_proposal_v3
);
v3_dispatch!(
    dispatch_expire_proposal_v3,
    ExpireProposalV3,
    process_expire_proposal_v3
);
v3_dispatch!(
    dispatch_guardian_freeze_v2,
    GuardianFreezeV2,
    process_guardian_freeze_v2
);
v3_dispatch!(
    dispatch_create_emergency_resolution_v2,
    CreateEmergencyResolutionV2,
    process_create_emergency_resolution_v2
);
v3_dispatch!(
    dispatch_approve_emergency_resolution_v2,
    ApproveEmergencyResolutionV2,
    process_approve_emergency_resolution_v2
);
v3_dispatch!(
    dispatch_queue_emergency_resolution_v2,
    QueueEmergencyResolutionV2,
    process_queue_emergency_resolution_v2
);
v3_dispatch!(
    dispatch_execute_emergency_resolution_v2,
    ExecuteEmergencyResolutionV2,
    process_execute_emergency_resolution_v2
);
v3_dispatch!(
    dispatch_expire_emergency_resolution_v2,
    ExpireEmergencyResolutionV2,
    process_expire_emergency_resolution_v2
);
v3_boxed_dispatch!(
    dispatch_create_checkpoint_v2,
    CreateCheckpointV2,
    process_create_checkpoint_v2
);
v3_boxed_dispatch!(
    dispatch_recast_checkpoint_v2,
    RecastCheckpointV2,
    process_recast_checkpoint_v2
);
v3_boxed_dispatch!(
    dispatch_finalize_checkpoint_v2,
    FinalizeCheckpointV2,
    process_finalize_checkpoint_v2
);
v3_dispatch!(
    dispatch_bind_programdata_verification_v2,
    BindProgramDataVerificationV2,
    process_bind_programdata_verification_v2
);
v3_dispatch!(
    dispatch_finalize_programdata_verification_v2,
    FinalizeProgramDataVerificationV2,
    process_finalize_programdata_verification_v2
);
v3_dispatch!(
    dispatch_observe_programdata_failure_v2,
    ObserveProgramDataFailureV2,
    process_observe_programdata_failure_witness_v2
);
v3_dispatch!(
    dispatch_approve_unfreeze_v2,
    ApproveUnfreezeV2,
    process_approve_unfreeze_v2
);
v3_dispatch!(
    dispatch_execute_unfreeze_v2,
    ExecuteUnfreezeV2,
    process_execute_unfreeze_v2
);
custody_v2_dispatch!(
    dispatch_adopt_buffer_v2,
    AdoptBufferV2,
    process_adopt_buffer_v2
);
custody_v2_dispatch!(
    dispatch_verify_buffer_chunk_v2,
    VerifyBufferChunkV2,
    process_verify_buffer_chunk_v2
);
custody_v2_dispatch!(
    dispatch_finalize_buffer_verification_v2,
    FinalizeBufferVerificationV2,
    process_finalize_buffer_verification_v2
);
custody_v2_dispatch!(
    dispatch_extend_target_v2,
    ExtendTargetV2,
    process_extend_target_v2
);
custody_v2_dispatch!(
    dispatch_execute_upgrade_v2,
    ExecuteUpgradeV2,
    process_execute_upgrade_v2
);
custody_v2_dispatch!(
    dispatch_close_abandoned_buffer_v2,
    CloseAbandonedBufferV2,
    process_close_abandoned_buffer_v2
);
custody_v2_dispatch!(
    dispatch_activate_rollback_v2,
    ActivateRollbackV2,
    process_activate_rollback_v2
);
governance_v2_dispatch!(
    dispatch_initialize_governance_lifecycle_registry_v2,
    InitializeGovernanceLifecycleRegistryV2,
    process_initialize_governance_lifecycle_registry_v2
);
governance_v2_dispatch!(
    dispatch_create_governance_timing_profile_v1,
    CreateGovernanceTimingProfileV1,
    process_create_governance_timing_profile_v1
);
governance_v2_dispatch!(
    dispatch_create_timing_policy_change_proposal_v1,
    CreateTimingPolicyChangeProposalV1,
    process_create_timing_policy_change_proposal_v1
);
governance_v2_dispatch!(
    dispatch_approve_timing_policy_change_proposal_v1,
    ApproveTimingPolicyChangeProposalV1,
    process_approve_timing_policy_change_proposal_v1
);
governance_v2_dispatch!(
    dispatch_cancel_timing_policy_change_proposal_v1,
    CancelTimingPolicyChangeProposalV1,
    process_cancel_timing_policy_change_proposal_v1
);
governance_v2_dispatch!(
    dispatch_expire_timing_policy_change_proposal_v1,
    ExpireTimingPolicyChangeProposalV1,
    process_expire_timing_policy_change_proposal_v1
);
governance_v2_dispatch!(
    dispatch_queue_timing_policy_change_proposal_v1,
    QueueTimingPolicyChangeProposalV1,
    process_queue_timing_policy_change_proposal_v1
);
governance_v2_dispatch!(
    dispatch_execute_timing_policy_change_proposal_v1,
    ExecuteTimingPolicyChangeProposalV1,
    process_execute_timing_policy_change_proposal_v1
);
governance_v2_dispatch!(
    dispatch_create_council_rotation_proposal_v2,
    CreateCouncilRotationProposalV2,
    process_create_council_rotation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_approve_council_rotation_proposal_v2,
    ApproveCouncilRotationProposalV2,
    process_approve_council_rotation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_cancel_council_rotation_proposal_v2,
    CancelCouncilRotationProposalV2,
    process_cancel_council_rotation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_expire_council_rotation_proposal_v2,
    ExpireCouncilRotationProposalV2,
    process_expire_council_rotation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_queue_council_rotation_proposal_v2,
    QueueCouncilRotationProposalV2,
    process_queue_council_rotation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_execute_council_rotation_proposal_v2,
    ExecuteCouncilRotationProposalV2,
    process_execute_council_rotation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_create_target_authority_handoff_proposal_v2,
    CreateTargetAuthorityHandoffProposalV2,
    process_create_target_authority_handoff_proposal_v2
);
governance_v2_dispatch!(
    dispatch_approve_target_authority_handoff_proposal_v2,
    ApproveTargetAuthorityHandoffProposalV2,
    process_approve_target_authority_handoff_proposal_v2
);
governance_v2_dispatch!(
    dispatch_cancel_target_authority_handoff_proposal_v2,
    CancelTargetAuthorityHandoffProposalV2,
    process_cancel_target_authority_handoff_proposal_v2
);
governance_v2_dispatch!(
    dispatch_expire_target_authority_handoff_proposal_v2,
    ExpireTargetAuthorityHandoffProposalV2,
    process_expire_target_authority_handoff_proposal_v2
);
governance_v2_dispatch!(
    dispatch_queue_target_authority_handoff_proposal_v2,
    QueueTargetAuthorityHandoffProposalV2,
    process_queue_target_authority_handoff_proposal_v2
);
governance_v2_dispatch!(
    dispatch_execute_target_authority_handoff_proposal_v2,
    ExecuteTargetAuthorityHandoffProposalV2,
    process_execute_target_authority_handoff_proposal_v2
);
governance_v2_dispatch!(
    dispatch_create_bootstrap_activation_proposal_v2,
    CreateBootstrapActivationProposalV2,
    process_create_bootstrap_activation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_approve_bootstrap_activation_proposal_v2,
    ApproveBootstrapActivationProposalV2,
    process_approve_bootstrap_activation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_cancel_bootstrap_activation_proposal_v2,
    CancelBootstrapActivationProposalV2,
    process_cancel_bootstrap_activation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_expire_bootstrap_activation_proposal_v2,
    ExpireBootstrapActivationProposalV2,
    process_expire_bootstrap_activation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_queue_bootstrap_activation_proposal_v2,
    QueueBootstrapActivationProposalV2,
    process_queue_bootstrap_activation_proposal_v2
);
governance_v2_dispatch!(
    dispatch_execute_bootstrap_activation_proposal_v2,
    ExecuteBootstrapActivationProposalV2,
    process_execute_bootstrap_activation_proposal_v2
);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.is_empty() || instruction_data.len() > MAX_CONTROLLER_INSTRUCTION_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    match instruction_data[0] {
        // Historical and policy-bypassing schemas stay regression-decodable in
        // their defining modules, but production dispatch rejects them before
        // payload decoding or account access. V2 owns rotation, authority
        // handoff, and bootstrap activation proposal liveness. Tag 18 remains
        // because V2 rotation reuses its immutable candidate council account.
        0..=17 | 19..=38 | 44..=52 => Err(ProgramError::InvalidInstructionData),
        instruction::CREATE_CANDIDATE_COUNCIL_SET_V1_TAG => {
            dispatch_create_candidate_council_set_v1(program_id, accounts, instruction_data)
        }
        release1_ceremony_instruction::BEGIN_PROGRAMDATA_OBSERVATION_V1_TAG => {
            dispatch_begin_programdata_observation_v1(program_id, accounts, instruction_data)
        }
        release1_ceremony_instruction::APPEND_PROGRAMDATA_OBSERVATION_CHUNK_V1_TAG => {
            dispatch_append_programdata_observation_chunk_v1(program_id, accounts, instruction_data)
        }
        release1_ceremony_instruction::VERIFY_OBSERVED_ARTIFACT_CHUNK_V1_TAG => {
            dispatch_verify_observed_artifact_chunk_v1(program_id, accounts, instruction_data)
        }
        release1_ceremony_instruction::FINALIZE_PROGRAMDATA_OBSERVATION_V1_TAG => {
            dispatch_finalize_programdata_observation_v1(program_id, accounts, instruction_data)
        }
        release1_authority_instruction::RECORD_CONTROLLER_IMMUTABILITY_V1_TAG => {
            dispatch_record_controller_immutability_v1(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::INITIALIZE_CONTROLLER_V2_TAG => {
            dispatch_initialize_controller_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::CREATE_PROPOSAL_V3_TAG => {
            dispatch_create_proposal_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::APPROVE_PROPOSAL_V3_TAG => {
            dispatch_approve_proposal_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::FINALIZE_GOVERNANCE_V3_TAG => {
            dispatch_finalize_governance_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::QUEUE_PROPOSAL_V3_TAG => {
            dispatch_queue_proposal_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::FREEZE_PROPOSAL_V3_TAG => {
            dispatch_freeze_proposal_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::CANCEL_PROPOSAL_V3_TAG => {
            dispatch_cancel_proposal_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::EXPIRE_PROPOSAL_V3_TAG => {
            dispatch_expire_proposal_v3(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::GUARDIAN_FREEZE_V2_TAG => {
            dispatch_guardian_freeze_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::CREATE_EMERGENCY_RESOLUTION_V2_TAG => {
            dispatch_create_emergency_resolution_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::APPROVE_EMERGENCY_RESOLUTION_V2_TAG => {
            dispatch_approve_emergency_resolution_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::QUEUE_EMERGENCY_RESOLUTION_V2_TAG => {
            dispatch_queue_emergency_resolution_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::EXECUTE_EMERGENCY_RESOLUTION_V2_TAG => {
            dispatch_execute_emergency_resolution_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::EXPIRE_EMERGENCY_RESOLUTION_V2_TAG => {
            dispatch_expire_emergency_resolution_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::CREATE_CHECKPOINT_V2_TAG => {
            dispatch_create_checkpoint_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::RECAST_CHECKPOINT_V2_TAG => {
            dispatch_recast_checkpoint_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::FINALIZE_CHECKPOINT_V2_TAG => {
            dispatch_finalize_checkpoint_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::BIND_PROGRAMDATA_VERIFICATION_V2_TAG => {
            dispatch_bind_programdata_verification_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::FINALIZE_PROGRAMDATA_VERIFICATION_V2_TAG => {
            dispatch_finalize_programdata_verification_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::OBSERVE_PROGRAMDATA_FAILURE_V2_TAG => {
            dispatch_observe_programdata_failure_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::APPROVE_UNFREEZE_V2_TAG => {
            dispatch_approve_unfreeze_v2(program_id, accounts, instruction_data)
        }
        release1_v3_instruction::EXECUTE_UNFREEZE_V2_TAG => {
            dispatch_execute_unfreeze_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::ADOPT_BUFFER_V2_TAG => {
            dispatch_adopt_buffer_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::VERIFY_BUFFER_CHUNK_V2_TAG => {
            dispatch_verify_buffer_chunk_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::FINALIZE_BUFFER_VERIFICATION_V2_TAG => {
            dispatch_finalize_buffer_verification_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::EXTEND_TARGET_V2_TAG => {
            dispatch_extend_target_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::EXECUTE_UPGRADE_V2_TAG => {
            dispatch_execute_upgrade_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::CLOSE_ABANDONED_BUFFER_V2_TAG => {
            dispatch_close_abandoned_buffer_v2(program_id, accounts, instruction_data)
        }
        release1_v3_custody_instruction::ACTIVATE_ROLLBACK_V2_TAG => {
            dispatch_activate_rollback_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::INITIALIZE_GOVERNANCE_LIFECYCLE_REGISTRY_V2_TAG => {
            dispatch_initialize_governance_lifecycle_registry_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::CREATE_GOVERNANCE_TIMING_PROFILE_V1_TAG => {
            dispatch_create_governance_timing_profile_v1(program_id, accounts, instruction_data)
        }
        release1_governance_v2::CREATE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
            dispatch_create_timing_policy_change_proposal_v1(program_id, accounts, instruction_data)
        }
        release1_governance_v2::APPROVE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
            dispatch_approve_timing_policy_change_proposal_v1(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::CANCEL_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
            dispatch_cancel_timing_policy_change_proposal_v1(program_id, accounts, instruction_data)
        }
        release1_governance_v2::EXPIRE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
            dispatch_expire_timing_policy_change_proposal_v1(program_id, accounts, instruction_data)
        }
        release1_governance_v2::QUEUE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
            dispatch_queue_timing_policy_change_proposal_v1(program_id, accounts, instruction_data)
        }
        release1_governance_v2::EXECUTE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
            dispatch_execute_timing_policy_change_proposal_v1(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::CREATE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
            dispatch_create_council_rotation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::APPROVE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
            dispatch_approve_council_rotation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::CANCEL_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
            dispatch_cancel_council_rotation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::EXPIRE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
            dispatch_expire_council_rotation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::QUEUE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
            dispatch_queue_council_rotation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::EXECUTE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
            dispatch_execute_council_rotation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::CREATE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
            dispatch_create_target_authority_handoff_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::APPROVE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
            dispatch_approve_target_authority_handoff_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::CANCEL_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
            dispatch_cancel_target_authority_handoff_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::EXPIRE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
            dispatch_expire_target_authority_handoff_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::QUEUE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
            dispatch_queue_target_authority_handoff_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::EXECUTE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
            dispatch_execute_target_authority_handoff_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::CREATE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
            dispatch_create_bootstrap_activation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::APPROVE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
            dispatch_approve_bootstrap_activation_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        release1_governance_v2::CANCEL_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
            dispatch_cancel_bootstrap_activation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::EXPIRE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
            dispatch_expire_bootstrap_activation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::QUEUE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
            dispatch_queue_bootstrap_activation_proposal_v2(program_id, accounts, instruction_data)
        }
        release1_governance_v2::EXECUTE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
            dispatch_execute_bootstrap_activation_proposal_v2(
                program_id,
                accounts,
                instruction_data,
            )
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::{
        ActivateRollbackV1, AdoptBufferV1, ApproveUnfreezeV1, CloseAbandonedBufferV1,
        EnvelopeExpectationV1, ExecuteUnfreezeV1, ExecuteUpgradeV1, ExtendTargetV1,
        FinalizeBufferVerificationV1, FinalizeProgramDataVerificationV1, FixedMerkleProofV1,
        ObserveProgramDataFailureV1, OptionalInstructionPubkeyV1, ProgramDataChunkPhaseV1,
        ProposalExpectationV2, RecordProposalApprovalV1, UnfreezeExpectationV1,
        VerifyBufferChunkV1, VerifyProgramDataChunkV1, MAX_FIXED_MERKLE_PROOF_NODES_V1,
    };
    use crate::release1_state::{
        BufferVerificationStatusV1, ProgramDataMismatchClassV1, ProgramDataVerificationStatusV1,
        ProposalStateV2, VERIFICATION_BITMAP_BYTES_V1,
    };
    use crate::state::GateStatusV1;

    const fn bytes(value: u8) -> [u8; 32] {
        [value; 32]
    }

    fn key(value: u8) -> Pubkey {
        Pubkey::new_from_array([value; 32])
    }

    fn proposal_expectation() -> ProposalExpectationV2 {
        ProposalExpectationV2 {
            expected_proposal_digest: bytes(1),
            expected_policy_version: 2,
            expected_policy_hash: bytes(3),
            expected_council_version: 4,
            expected_council_hash: bytes(5),
            expected_gate_status: GateStatusV1::FrozenForUpgrade,
            expected_gate_epoch: 6,
            expected_target_nonce: 7,
            expected_state: ProposalStateV2::Timelocked,
            expected_review_start_slot: 8,
            expected_review_end_slot: 9,
            expected_not_before_slot: 10,
            expected_expiry_slot: 11,
        }
    }

    fn unfreeze_expectation() -> UnfreezeExpectationV1 {
        UnfreezeExpectationV1 {
            expected_proposal_digest: bytes(12),
            expected_policy_version: 13,
            expected_policy_hash: bytes(14),
            expected_current_council_version: 15,
            expected_current_council_hash: bytes(16),
            expected_frozen_gate_epoch: 17,
            expected_target_nonce: 18,
            expected_proposal_state: ProposalStateV2::PoststateAccepted,
            expected_poststate_checkpoint_digest: bytes(19),
            expected_programdata_authority: key(20),
            expected_programdata_deployed_slot: 21,
            expected_programdata_capacity: 22,
            expected_raw_programdata_hash: bytes(23),
            expected_unfreeze_approval_bitset: 0b0_0011,
            expected_unfreeze_approval_count: 2,
            expected_programdata_verification_finalized_slot: 24,
        }
    }

    fn envelope() -> EnvelopeExpectationV1 {
        EnvelopeExpectationV1 {
            compute_unit_limit: 1_200_000,
            compute_unit_price_micro_lamports: 17,
            durable_nonce_account: OptionalInstructionPubkeyV1::some(key(25)).unwrap(),
            durable_nonce_authority: OptionalInstructionPubkeyV1::some(key(26)).unwrap(),
        }
    }

    fn merkle_proof() -> FixedMerkleProofV1 {
        let mut nodes = [[0; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1];
        nodes[0] = bytes(27);
        nodes[1] = bytes(28);
        FixedMerkleProofV1 {
            proof_len: 2,
            nodes,
        }
    }

    fn bitmap(value: u8) -> [u8; VERIFICATION_BITMAP_BYTES_V1] {
        [value; VERIFICATION_BITMAP_BYTES_V1]
    }

    fn custody_instruction_packets() -> Vec<Vec<u8>> {
        vec![
            AdoptBufferV1 {
                expected: proposal_expectation(),
            }
            .pack()
            .to_vec(),
            VerifyBufferChunkV1 {
                expected: proposal_expectation(),
                chunk_index: 1,
                proof: merkle_proof(),
                expected_verification_status: BufferVerificationStatusV1::Verifying,
                expected_verified_chunk_bitmap: bitmap(1),
                expected_verified_chunk_count: 2,
            }
            .pack()
            .to_vec(),
            FinalizeBufferVerificationV1 {
                expected: proposal_expectation(),
                expected_verification_status: BufferVerificationStatusV1::ReadyToFinalize,
                expected_verified_chunk_bitmap: bitmap(0xff),
                expected_verified_chunk_count: 128,
            }
            .pack()
            .to_vec(),
            ExtendTargetV1 {
                expected: proposal_expectation(),
                expected_prestate_checkpoint_digest: bytes(29),
                expected_current_capacity: 1_000,
                expected_extension_delta: 200,
                expected_post_capacity: 1_200,
                envelope: envelope(),
            }
            .pack()
            .to_vec(),
            ExecuteUpgradeV1 {
                expected: proposal_expectation(),
                expected_prestate_checkpoint_digest: bytes(30),
                expected_current_raw_programdata_hash: bytes(31),
                expected_sealed_buffer_header_hash: bytes(32),
                expected_counterpart_proposal_digest: bytes(33),
                expected_programdata_slot: 34,
                expected_capacity: 35,
                expected_verified_chunk_count: 128,
                expected_buffer_verification_status: BufferVerificationStatusV1::Verified,
                expected_counterpart_buffer_verification_status:
                    BufferVerificationStatusV1::Verified,
                envelope: envelope(),
            }
            .pack()
            .to_vec(),
            VerifyProgramDataChunkV1 {
                expected: proposal_expectation(),
                phase: ProgramDataChunkPhaseV1::Payload,
                chunk_index: 2,
                proof: merkle_proof(),
                expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
                expected_verified_payload_chunk_bitmap: bitmap(2),
                expected_verified_payload_chunk_count: 3,
                expected_verified_tail_chunk_bitmap: bitmap(0),
                expected_verified_tail_chunk_count: 0,
            }
            .pack()
            .to_vec(),
            FinalizeProgramDataVerificationV1 {
                expected: proposal_expectation(),
                expected_verification_status: ProgramDataVerificationStatusV1::ReadyToFinalize,
                expected_verified_payload_chunk_bitmap: bitmap(0xff),
                expected_verified_payload_chunk_count: 128,
                expected_verified_tail_chunk_bitmap: bitmap(0),
                expected_verified_tail_chunk_count: 0,
                expected_deployed_slot: 36,
                expected_capacity: 37,
            }
            .pack()
            .to_vec(),
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(),
            }
            .pack()
            .to_vec(),
            ExecuteUnfreezeV1 {
                expected: unfreeze_expectation(),
                linked_proposal: key(38),
                envelope: envelope(),
            }
            .pack()
            .to_vec(),
            CloseAbandonedBufferV1 {
                expected: proposal_expectation(),
                expected_verification_status: BufferVerificationStatusV1::Verified,
                expected_verified_chunk_bitmap: bitmap(0xff),
                expected_verified_chunk_count: 128,
                expected_buffer_verification_finalized_slot: 39,
            }
            .pack()
            .to_vec(),
            ActivateRollbackV1 {
                expected_primary: proposal_expectation(),
                expected_rollback: proposal_expectation(),
                expected_failure_evidence_digest: bytes(40),
                expected_primary_programdata_verification_status:
                    ProgramDataVerificationStatusV1::Verifying,
                expected_primary_programdata_verification_finalized_slot: 0,
                expected_rollback_buffer_verification_status: BufferVerificationStatusV1::Verified,
                expected_rollback_verified_chunk_bitmap: bitmap(0xff),
                expected_rollback_verified_chunk_count: 128,
            }
            .pack()
            .to_vec(),
            ObserveProgramDataFailureV1 {
                expected: proposal_expectation(),
                expected_program_owner: key(41),
                expected_program_executable: true,
                expected_program_data_length: 36,
                expected_program_header_present: true,
                expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(42)).unwrap(),
                expected_programdata_owner: key(43),
                expected_programdata_executable: false,
                expected_programdata_data_length: 4_141,
                expected_programdata_header_present: true,
                expected_programdata_slot: 44,
                expected_raw_hash_complete: true,
                expected_raw_programdata_hash: bytes(45),
                expected_capacity: 4_096,
                expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(46)).unwrap(),
                mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
                failing_chunk_index: 3,
                expected_leaf_hash: bytes(47),
                proof: merkle_proof(),
            }
            .pack()
            .to_vec(),
        ]
    }

    #[test]
    fn historical_approval_remains_non_executable() {
        let program_id = Pubkey::new_unique();
        let legacy = RecordProposalApprovalV1 {
            expected_proposal_digest: [7; 32],
            expected_council_version: 1,
        }
        .pack();
        assert_eq!(
            process_instruction(&program_id, &[], &legacy),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn deprecated_capacity_fragile_custody_surface_is_non_executable() {
        let program_id = Pubkey::new_unique();
        let packets = custody_instruction_packets();
        assert_eq!(packets.len(), 12);
        for (offset, packet) in packets.iter().enumerate() {
            let expected_tag = instruction::ADOPT_BUFFER_V1_TAG + offset as u8;
            assert_eq!(packet[0], expected_tag);
            assert_eq!(
                process_instruction(&program_id, &[], packet),
                Err(ProgramError::InvalidInstructionData),
                "deprecated tag {expected_tag} remained executable"
            );
        }
    }

    #[test]
    fn every_deprecated_lifecycle_tag_rejects_before_payload_decode() {
        for tag in (0..=17).chain(19..=38).chain(44..=52) {
            assert_eq!(
                process_instruction(&Pubkey::new_unique(), &[], &[tag]),
                Err(ProgramError::InvalidInstructionData),
                "deprecated tag {tag} did not retain generic early rejection"
            );
        }
    }

    #[test]
    fn unknown_tag_retains_generic_early_rejection() {
        assert_eq!(
            process_instruction(&Pubkey::new_unique(), &[], &[26]),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            process_instruction(&Pubkey::new_unique(), &[], &[255]),
            Err(ProgramError::InvalidInstructionData)
        );
    }
}
