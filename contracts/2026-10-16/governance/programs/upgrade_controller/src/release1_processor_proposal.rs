//! Release 1 proposal, freeze, and emergency-resolution processors.
//!
//! This module deliberately contains no loader mutation.  It validates the
//! current loader graph read-only, prepares exact fixed-width account bytes off
//! account borrows, and commits only after every fallible validation succeeds.

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

use crate::{
    artifact_merkle::{
        artifact_chunk_count, ARTIFACT_MERKLE_SCHEME_ID, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    authorization::validate_seat_authority,
    council::{
        record_seat_approval, validate_council_guardian_separation, validate_council_set,
        VALID_APPROVAL_MASK,
    },
    instruction::{
        ApproveEmergencyResolutionV1, ApproveProposalV2, CancelProposalV2,
        ConvertEmergencyFreezeV2, CreateEmergencyResolutionV1, CreateProposalV2,
        EmergencyResolutionExpectationV1, ExecuteEmergencyResolutionV1, ExpireProposalV2,
        FinalizeGovernanceV2, FreezeProposalV2, GuardianFreezeV1, ProposalExpectationV2,
        QueueEmergencyResolutionV1, QueueProposalV2, MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1,
        MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_council_pda, derive_emergency_checkpoint_pda,
        derive_emergency_freeze_observation_pda, derive_emergency_resolution_pda, derive_gate_pda,
        derive_policy_pda, derive_programdata_check_pda, derive_proposal_pda,
        derive_upgradeable_programdata_address, EMERGENCY_FREEZE_OBSERVATION_SEED,
        EMERGENCY_RESOLUTION_SEED, PROPOSAL_SEED, UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, store_fixed_controller_account, validate_exact_privileges,
    },
    release1_digest::{
        compute_emergency_freeze_observation_digest_v1, compute_emergency_resolution_digest_v1,
        compute_proposal_digest_v2, validate_emergency_freeze_observation_digest_v1,
        validate_emergency_resolution_digest_v1, validate_proposal_digest_v2,
        validate_state_checkpoint_digest_v1,
    },
    release1_loader_accounts::{
        loader_account_data_hash, parse_upgradeable_program, parse_upgradeable_programdata,
        validate_buffer_account, validate_program_programdata_linkage, LOADER_BUFFER_METADATA_LEN,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, EmergencyFreezeObservationV1,
        EmergencyFreezeResolutionKindV1, EmergencyFreezeResolutionStateV1,
        EmergencyFreezeResolutionV1, ProposalStateV2, StateCheckpointPhaseV1, StateCheckpointV1,
        UpgradeProposalV2, ACCOUNT_VERSION_V2, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
        EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN, EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
        EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN,
        EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1,
        MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, PROPOSAL_EXPIRED_TERMINAL_REASON_V1,
        RELEASE1_ACCOUNT_VERSION_V1, RELEASE1_APPROVAL_THRESHOLD,
        UPGRADE_PROPOSAL_V2_DISCRIMINATOR, UPGRADE_PROPOSAL_V2_RESERVED_LEN,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProtocolGateV1, VoteRequirementV1,
    },
    GovernanceError, GovernanceResult,
};

/// Canonical reason used only for a council-approved proposal freeze.  The
/// bootstrap reason remains 1 and guardian reasons are carried verbatim.
pub const GOVERNED_UPGRADE_FREEZE_REASON_V1: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProgramDataSnapshotV1 {
    deployed_slot: u64,
    capacity: u64,
    raw_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeObservationV1 {
    program_owner: Pubkey,
    program_executable: bool,
    program_data_length: u64,
    program_header_present: bool,
    linked_programdata: OptionalPubkeyV1,
    programdata_owner: Pubkey,
    programdata_executable: bool,
    programdata_data_length: u64,
    programdata_header_present: bool,
    programdata_slot: u64,
    raw_hash_complete: bool,
    raw_programdata_hash: [u8; 32],
    capacity: u64,
    authority: OptionalPubkeyV1,
}

struct FreezeInputs<'a, 'info> {
    config_info: &'a AccountInfo<'info>,
    policy_info: &'a AccountInfo<'info>,
    council_info: &'a AccountInfo<'info>,
    gate_info: &'a AccountInfo<'info>,
    proposal_info: &'a AccountInfo<'info>,
    target_program: &'a AccountInfo<'info>,
    target_programdata: &'a AccountInfo<'info>,
    loader: &'a AccountInfo<'info>,
    authority: &'a AccountInfo<'info>,
    rollback_info: &'a AccountInfo<'info>,
    rollback_verification_info: &'a AccountInfo<'info>,
    rollback_buffer: &'a AccountInfo<'info>,
    emergency_observation: Option<&'a AccountInfo<'info>>,
}

#[cfg(test)]
mod tests;

mod account_context;
mod emergency;
mod emergency_guards;
mod lifecycle;
mod observations;
mod proposal_guards;
mod proposals;

use account_context::{
    load_config, load_current_council, load_emergency_observation, load_gate, load_pinned_council,
    load_policy, load_proposal, load_resolution,
};
pub use emergency::{
    process_approve_emergency_resolution_v1, process_convert_emergency_freeze_v2,
    process_create_emergency_resolution_v1, process_execute_emergency_resolution_v1,
    process_guardian_freeze_v1, process_queue_emergency_resolution_v1,
};
use emergency_guards::{
    check_emergency_expectation, require_emergency_binding,
    validate_bounded_emergency_resolution_envelope,
};
use lifecycle::freeze_or_convert;
#[cfg(test)]
use lifecycle::require_freeze_execution_runway;
pub use lifecycle::{
    process_cancel_proposal_v2, process_expire_proposal_v2, process_freeze_proposal_v2,
};
use observations::{
    capture_runtime_observation, compare_execute_instruction_observation,
    compare_guardian_instruction_observation, compare_resolution_instruction_observation,
    optional_pubkey, read_canonical_programdata_snapshot, require_canonical_unchanged_runtime,
    require_resolution_observation_match,
};
#[cfg(test)]
use proposal_guards::require_creation_gate_values;
use proposal_guards::{
    check_proposal_expectation, check_proposal_expectation_without_policy, checked_freeze_counters,
    derive_proposal_timing, is_pre_freeze_state, reject_guardian_authority,
    reject_locked_reciprocal_rollback, require_active_seat, require_approval_window,
    require_creation_gate, require_exact_recorded_quorum, require_policy_active,
    validate_loader_identity, validate_readonly_state_accounts, verify_exact_proposal_timing,
};
pub use proposals::{
    process_approve_proposal_v2, process_create_proposal_v2, process_finalize_governance_v2,
    process_queue_proposal_v2,
};
