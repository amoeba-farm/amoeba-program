//! Release 1 controller-immutability, checked authority-handoff, and bootstrap-activation
//! processors.
//!
//! The surface is deliberately closed and typed.  It contains no arbitrary CPI data, no
//! caller-selected program IDs, and no production execution helper.  The checked handoff is
//! finalized atomically by tag 47; the historical tag 48 codec is intentionally not consumed
//! here because a second full ProgramData observation after handoff cannot fit in the same
//! transaction and is not required to prove the typed Loader header transition.

use solana_loader_v3_interface::instruction::set_upgrade_authority_checked;
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    instruction::Instruction,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::{instructions, Sysvar},
};
use solana_sdk_ids::{compute_budget, system_program, sysvar as sysvar_ids};
use solana_system_interface::instruction as system_instruction;

use crate::{
    authorization::validate_seat_authority,
    council::{
        record_seat_approval, validate_council_guardian_separation, validate_council_set,
        VALID_APPROVAL_MASK,
    },
    pda::{
        derive_authority_pda, derive_bootstrap_activation_pda,
        derive_bootstrap_activation_receipt_pda, derive_capacity_policy_pda,
        derive_controller_config_pda, derive_controller_immutability_receipt_pda,
        derive_controller_release_commitment_pda, derive_council_pda,
        derive_current_deployment_state_pda, derive_gate_pda, derive_policy_pda,
        derive_programdata_observation_pda,
        derive_target_handoff_pda as derive_target_authority_handoff_pda,
        derive_target_handoff_receipt_pda, derive_upgradeable_programdata_address, AUTHORITY_SEED,
        BOOTSTRAP_ACTIVATION_RECEIPT_SEED, BOOTSTRAP_ACTIVATION_SEED, CONTROLLER_IMMUTABILITY_SEED,
        DEPLOYMENT_STATE_SEED, TARGET_HANDOFF_RECEIPT_SEED, TARGET_HANDOFF_SEED,
        UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, validate_exact_privileges,
    },
    release1_authority_instruction::{
        AcceptTargetAuthorityCheckedV1, ApproveBootstrapActivationV1,
        ApproveTargetAuthorityHandoffV1, CeremonyEnvelopeV1, CreateBootstrapActivationV1,
        CreateTargetAuthorityHandoffV1, ExecuteBootstrapActivationV1, QueueBootstrapActivationV1,
        QueueTargetAuthorityHandoffV1, RecordControllerImmutabilityV1,
        MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1, MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    release1_ceremony_digest::{
        compute_bootstrap_activation_deployment_plan_digest_v1,
        compute_bootstrap_activation_proposal_digest_v1,
        compute_bootstrap_activation_receipt_digest_v1,
        compute_bootstrap_activation_receipt_plan_digest_v1,
        compute_controller_immutability_receipt_digest_v1, compute_current_deployment_digest_v1,
        compute_programdata_observation_subject_digest_v1,
        compute_target_handoff_proposal_digest_v1, compute_target_handoff_receipt_digest_v1,
        validate_bootstrap_activation_proposal_digest_v1,
        validate_controller_immutability_receipt_digest_v1, validate_controller_release_digest_v1,
        validate_current_deployment_digest_v1, validate_programdata_observation_digest_v1,
        validate_target_handoff_proposal_digest_v1, validate_target_handoff_receipt_digest_v1,
    },
    release1_ceremony_state::{
        BootstrapActivationProposalV1, BootstrapActivationReceiptV1, CeremonyProposalStateV1,
        ControllerImmutabilityReceiptV1, ControllerReleaseCommitmentV1, CurrentDeploymentStateV1,
        ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1,
        ProgramDataObservationStatusV1, ProgramDataObservationV1, TargetAuthorityHandoffProposalV1,
        TargetAuthorityHandoffReceiptV1, BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN, CEREMONY_ACCOUNT_VERSION_V1,
        CEREMONY_PROPOSAL_COMPLETED_REASON_V1, CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
        CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN, CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN,
        TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN,
        TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN,
    },
    release1_loader_accounts::{
        parse_upgradeable_programdata, validate_program_programdata_linkage,
        LOADER_PROGRAMDATA_METADATA_LEN,
    },
    release1_state::{BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1, RELEASE1_APPROVAL_THRESHOLD},
    state::{
        ControllerConfigV1, GateStatusV1, GovernanceCouncilSetV1, GovernancePolicyV1,
        OptionalPubkeyV1, ProtocolGateV1,
    },
    GovernanceError,
};

pub const RECORD_CONTROLLER_IMMUTABILITY_V1_ACCOUNT_COUNT: usize = 11;
pub const CREATE_TARGET_AUTHORITY_HANDOFF_V1_ACCOUNT_COUNT: usize = 18;
pub const APPROVE_TARGET_AUTHORITY_HANDOFF_V1_ACCOUNT_COUNT: usize = 16;
pub const QUEUE_TARGET_AUTHORITY_HANDOFF_V1_ACCOUNT_COUNT: usize = 15;
pub const ACCEPT_TARGET_AUTHORITY_CHECKED_V1_ACCOUNT_COUNT: usize = 19;
pub const CREATE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT: usize = 20;
pub const APPROVE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT: usize = 16;
pub const QUEUE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT: usize = 15;
pub const EXECUTE_BOOTSTRAP_ACTIVATION_V1_ACCOUNT_COUNT: usize = 18;

struct CeremonyContext {
    config: Box<ControllerConfigV1>,
    policy: Box<GovernancePolicyV1>,
    council: Box<GovernanceCouncilSetV1>,
    gate: Box<ProtocolGateV1>,
    capacity: Box<ProgramDataCapacityPolicyV1>,
}

struct HandoffReviewState {
    context: CeremonyContext,
    proposal: Box<TargetAuthorityHandoffProposalV1>,
}

struct ActivationEvidence {
    context: CeremonyContext,
    immutable: Box<ControllerImmutabilityReceiptV1>,
    handoff: Box<TargetAuthorityHandoffReceiptV1>,
    observation: Box<ProgramDataObservationV1>,
}

#[cfg(test)]
mod tests;

mod account_context;
mod activation;
mod approval;
mod evidence;
mod handoff;
mod immutability;
mod loader_contract;

use account_context::{
    commit_preencoded, create_or_reuse_zero_fixed_pda, load_activation_proposal,
    load_capacity_policy, load_ceremony_context, load_config, load_controller_release,
    load_handoff_proposal, load_handoff_receipt, load_immutability_receipt, load_observation,
    require_zero_initialized_destination,
};
pub use activation::{
    process_approve_bootstrap_activation_v1, process_create_bootstrap_activation_v1,
    process_execute_bootstrap_activation_v1, process_queue_bootstrap_activation_v1,
};
#[cfg(test)]
use approval::derive_major_timing_from_values;
use approval::{
    derive_major_timing, reject_guardian, require_active_seat, require_approval_window,
    require_approval_window_activation, require_exact_quorum,
};
use evidence::{
    require_live_observation, validate_activation_evidence, validate_activation_proposal_evidence,
    validate_activation_review_privileges, validate_bootstrap_gate,
    validate_controller_immutability_transition, validate_controller_still_immutable,
    validate_handoff_proposal_evidence, validate_handoff_review_privileges,
    validate_handoff_review_state, validate_target_graph,
};
pub use handoff::{
    process_accept_target_authority_checked_v1, process_approve_target_authority_handoff_v1,
    process_create_target_authority_handoff_v1, process_queue_target_authority_handoff_v1,
};
pub use immutability::process_record_controller_immutability_v1;
use loader_contract::{
    copy_program_header, copy_programdata_header, expected_handoff_post_header, optional_matches,
    validate_canonical_envelope, validate_checked_handoff_cpi_shape,
    validate_some_to_none_header_delta, validate_some_to_some_header_delta,
};
