//! Governance-liveness V2 target-authority handoff and bootstrap activation.
//!
//! Tags 96-107 reuse the finalized Release 1 controller, Loader-v3, observation,
//! receipt, and transaction-envelope evidence while replacing the V1 proposal
//! identity/timing layer. Proposal creation records `Clock::get()?.slot`,
//! consumes the lifecycle registry's monotonic id, and allocates only the
//! proposal. Singleton ceremony receipts are created only by their successful
//! execution transaction.

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
    council::{validate_council_guardian_separation, validate_council_set},
    pda::{
        derive_authority_pda, derive_bootstrap_activation_receipt_pda, derive_capacity_policy_pda,
        derive_controller_config_pda, derive_controller_immutability_receipt_pda,
        derive_council_pda, derive_current_deployment_state_pda, derive_gate_pda,
        derive_policy_pda, derive_programdata_observation_pda, derive_target_handoff_receipt_pda,
        derive_upgradeable_programdata_address, AUTHORITY_SEED, BOOTSTRAP_ACTIVATION_RECEIPT_SEED,
        DEPLOYMENT_STATE_SEED, TARGET_HANDOFF_RECEIPT_SEED, UPGRADEABLE_LOADER_ID,
        UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, validate_exact_privileges,
    },
    release1_authority_instruction::{
        CeremonyEnvelopeV1, MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1,
        MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    release1_ceremony_digest::{
        compute_bootstrap_activation_deployment_plan_digest_v1,
        compute_bootstrap_activation_receipt_digest_v1,
        compute_bootstrap_activation_receipt_plan_digest_v1, compute_current_deployment_digest_v1,
        compute_programdata_observation_subject_digest_v1,
        compute_target_handoff_receipt_digest_v1, validate_bootstrap_activation_receipt_digest_v1,
        validate_capacity_policy_digest_v1, validate_controller_immutability_receipt_digest_v1,
        validate_current_deployment_digest_v1, validate_programdata_observation_digest_v1,
        validate_target_handoff_receipt_digest_v1,
    },
    release1_ceremony_state::{
        BootstrapActivationReceiptV1, ControllerImmutabilityReceiptV1, CurrentDeploymentStateV1,
        ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1,
        ProgramDataObservationStatusV1, ProgramDataObservationV1, TargetAuthorityHandoffReceiptV1,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN, CEREMONY_ACCOUNT_VERSION_V1,
        CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR, CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN,
        TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN,
    },
    release1_governance_v2::{
        compute_bootstrap_activation_digest_v2, compute_target_authority_handoff_digest_v2,
        derive_governance_action_proposal_v2, derive_governance_lifecycle_registry_v2,
        derive_governance_timing_profile_v1, derive_proposal_timing_v2,
        record_governance_approval_v2, record_governance_cancellation_v2,
        require_governance_executable_v2, require_governance_expirable_v2,
        require_governance_queueable_v2, ApproveBootstrapActivationProposalV2,
        ApproveTargetAuthorityHandoffProposalV2, BootstrapActivationProposalV2,
        CancelBootstrapActivationProposalV2, CancelTargetAuthorityHandoffProposalV2,
        CreateBootstrapActivationProposalV2, CreateTargetAuthorityHandoffProposalV2,
        ExecuteBootstrapActivationProposalV2, ExecuteTargetAuthorityHandoffProposalV2,
        ExpireBootstrapActivationProposalV2, ExpireTargetAuthorityHandoffProposalV2,
        GovernanceActionGuardV2, GovernanceActionKindV2, GovernanceLifecycleRegistryV2,
        GovernanceLifecycleStateV2, GovernanceTimingClassV1, GovernanceTimingProfileV1,
        QueueBootstrapActivationProposalV2, QueueTargetAuthorityHandoffProposalV2,
        TargetAuthorityHandoffProposalV2, BOOTSTRAP_ACTIVATION_PROPOSAL_V2_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_PROPOSAL_V2_RESERVED_LEN, EQUAL_SEAT_THRESHOLD_V2,
        GOVERNANCE_ACTION_PROPOSAL_V2_SEED, GOVERNANCE_V2_ACCOUNT_VERSION,
        GOVERNANCE_V2_SEED_PREFIX, TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_RESERVED_LEN,
    },
    release1_loader_accounts::{
        parse_upgradeable_programdata, validate_program_programdata_linkage,
        LOADER_PROGRAMDATA_METADATA_LEN,
    },
    release1_state::BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
    state::{
        ControllerConfigV1, GateStatusV1, GovernanceCouncilSetV1, GovernancePolicyV1,
        OptionalPubkeyV1, ProtocolGateV1,
    },
    GovernanceError,
};

pub const CREATE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT: usize = 20;
pub const APPROVE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT: usize = 18;
pub const CANCEL_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT: usize = 8;
pub const EXPIRE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT: usize = 4;
pub const QUEUE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT: usize = 17;
pub const EXECUTE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT: usize = 21;
pub const CREATE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT: usize = 20;
pub const APPROVE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT: usize = 18;
pub const CANCEL_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT: usize = 8;
pub const EXPIRE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT: usize = 4;
pub const QUEUE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT: usize = 17;
pub const EXECUTE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT: usize = 22;

const GOVERNANCE_COMPLETED_REASON_V2: u16 = 1;
const GOVERNANCE_EXPIRED_REASON_V2: u16 = 2;

struct CeremonyContextV2 {
    config: Box<ControllerConfigV1>,
    policy: Box<GovernancePolicyV1>,
    council: Box<GovernanceCouncilSetV1>,
    gate: Box<ProtocolGateV1>,
    capacity: Box<ProgramDataCapacityPolicyV1>,
    registry: Box<GovernanceLifecycleRegistryV2>,
    profile: Box<GovernanceTimingProfileV1>,
}

struct HandoffEvidenceV2 {
    context: CeremonyContextV2,
    immutable: Box<ControllerImmutabilityReceiptV1>,
    observation: Box<ProgramDataObservationV1>,
}

struct ActivationEvidenceV2 {
    context: CeremonyContextV2,
    immutable: Box<ControllerImmutabilityReceiptV1>,
    handoff: Box<TargetAuthorityHandoffReceiptV1>,
    observation: Box<ProgramDataObservationV1>,
}

fn current_slot() -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
}

fn exact_account_count(accounts: &[AccountInfo<'_>], expected: usize) -> ProgramResult {
    if accounts.len() != expected {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    Ok(())
}

fn all_distinct(accounts: &[AccountInfo<'_>]) -> ProgramResult {
    require_distinct_accounts(&accounts.iter().collect::<Vec<_>>())
}

fn ro(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, false)
}

fn rw(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, false, false)
}

fn rs(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, true, false)
}

fn ws(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, true, false)
}

fn executable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, true)
}

fn system_program_account(account: &AccountInfo<'_>) -> ProgramResult {
    executable(account)?;
    if *account.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn load_config(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        info,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    let expected = derive_controller_config_pda(program_id, &config.target_program);
    if expected.0 != *info.key
        || expected.1 != config.bump
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
        || derive_authority_pda(program_id, &config.target_program).0 != config.authority_pda
        || derive_gate_pda(program_id, &config.target_program).0 != config.gate_pda
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

fn load_policy(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    slot: u64,
) -> Result<Box<GovernancePolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<GovernancePolicyV1>(
        program_id,
        info,
        GovernancePolicyV1::LEN,
    )?;
    validate_policy_against_config(&policy, config)?;
    let expected = derive_policy_pda(program_id, &config.target_program, policy.version);
    if expected.0 != *info.key
        || expected.1 != policy.bump
        || policy.controller_config != *config_info.key
        || policy.version != config.current_policy_version
        || policy.activation_slot > slot
    {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(policy)
}

fn load_council(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    slot: u64,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        info,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&council, policy)?;
    validate_council_guardian_separation(&council, &config.guardian)?;
    let expected = derive_council_pda(program_id, &config.target_program, council.version);
    if expected.0 != *info.key
        || expected.1 != council.bump
        || council.controller_config != *config_info.key
        || council.target_program != config.target_program
        || council.version != config.current_council_version
        || !council.active_at(slot)
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(council)
}

fn load_gate(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate =
        load_fixed_controller_account::<ProtocolGateV1>(program_id, info, ProtocolGateV1::LEN)?;
    gate.validate_static()?;
    let expected = derive_gate_pda(program_id, &config.target_program);
    if expected.0 != *info.key
        || expected.1 != gate.bump
        || *info.key != config.gate_pda
        || gate.controller_config != *config_info.key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

fn load_capacity(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProgramDataCapacityPolicyV1>, ProgramError> {
    let capacity = load_fixed_controller_account::<ProgramDataCapacityPolicyV1>(
        program_id,
        info,
        ProgramDataCapacityPolicyV1::LEN,
    )?;
    validate_capacity_policy_digest_v1(&capacity)?;
    let expected = derive_capacity_policy_pda(program_id, &config.target_program);
    if expected.0 != *info.key
        || expected.1 != capacity.bump
        || capacity.controller_program != *program_id
        || capacity.controller_config != *config_info.key
        || capacity.target_program != config.target_program
        || capacity.target_programdata != config.target_programdata
        || capacity.upgradeable_loader != config.upgradeable_loader
    {
        return Err(GovernanceError::CapacityPolicyMismatch.into());
    }
    Ok(capacity)
}

fn load_registry(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<GovernanceLifecycleRegistryV2>, ProgramError> {
    let registry = load_fixed_controller_account::<GovernanceLifecycleRegistryV2>(
        program_id,
        info,
        GovernanceLifecycleRegistryV2::LEN,
    )?;
    registry.validate_static()?;
    let expected = derive_governance_lifecycle_registry_v2(program_id, &config.target_program);
    if expected.0 != *info.key
        || expected.1 != registry.bump
        || registry.controller_program != *program_id
        || registry.controller_config != *config_info.key
        || registry.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(registry)
}

fn load_profile(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<GovernanceTimingProfileV1>, ProgramError> {
    let profile = load_fixed_controller_account::<GovernanceTimingProfileV1>(
        program_id,
        info,
        GovernanceTimingProfileV1::LEN,
    )?;
    profile.validate_static()?;
    let expected = derive_governance_timing_profile_v1(
        program_id,
        &config.target_program,
        profile.profile_version,
    );
    if expected.0 != *info.key
        || expected.1 != profile.bump
        || profile.controller_config != *config_info.key
        || profile.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(profile)
}

fn require_registry_profile(
    registry: &GovernanceLifecycleRegistryV2,
    profile_info: &AccountInfo<'_>,
    profile: &GovernanceTimingProfileV1,
) -> ProgramResult {
    if registry.current_timing_profile != *profile_info.key
        || registry.current_timing_profile_version != profile.profile_version
        || registry.current_timing_profile_hash != profile.profile_hash
    {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(())
}

fn validate_bootstrap_gate(gate: &ProtocolGateV1, config: &ControllerConfigV1) -> ProgramResult {
    if gate.status != GateStatusV1::EmergencyFrozen
        || gate.epoch == 0
        || gate.active_proposal != Pubkey::default()
        || gate.freeze_slot == 0
        || gate.freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::InvalidGateState.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn load_context(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    profile_info: &AccountInfo<'_>,
    slot: u64,
) -> Result<CeremonyContextV2, ProgramError> {
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config, slot)?;
    let council = load_council(
        program_id,
        council_info,
        config_info,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    validate_bootstrap_gate(&gate, &config)?;
    let capacity = load_capacity(program_id, capacity_info, config_info, &config)?;
    let registry = load_registry(program_id, registry_info, config_info, &config)?;
    let profile = load_profile(program_id, profile_info, config_info, &config)?;
    Ok(CeremonyContextV2 {
        config,
        policy,
        council,
        gate,
        capacity,
        registry,
        profile,
    })
}

#[allow(clippy::too_many_arguments)]
fn load_observation(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    capacity_info: &AccountInfo<'_>,
    capacity: &ProgramDataCapacityPolicyV1,
    observed_program: &Pubkey,
    purpose: ProgramDataObservationPurposeV1,
    subject: &Pubkey,
) -> Result<Box<ProgramDataObservationV1>, ProgramError> {
    let observation = load_fixed_controller_account::<ProgramDataObservationV1>(
        program_id,
        info,
        ProgramDataObservationV1::LEN,
    )?;
    validate_programdata_observation_digest_v1(&observation)?;
    let subject_digest = compute_programdata_observation_subject_digest_v1(
        program_id,
        config_info.key,
        observed_program,
        &observation.target_programdata,
        purpose,
        subject,
        observation.generation,
        &observation.protocol_gate,
        observation.gate_status,
        observation.gate_epoch,
        &observation.gate_active_proposal,
        observation.gate_freeze_slot,
        observation.gate_freeze_reason_code,
        &capacity.policy_digest,
        observation.expected_artifact_length,
        &observation.expected_artifact_sha256,
        &observation.expected_artifact_merkle_root,
        &observation.expected_artifact_scheme_id,
        observation.minimum_required_capacity,
    )?;
    let expected = derive_programdata_observation_pda(
        program_id,
        observed_program,
        purpose as u8,
        &subject_digest,
        observation.generation,
    );
    if expected.0 != *info.key
        || expected.1 != observation.bump
        || observation.controller_program != *program_id
        || observation.controller_config != *config_info.key
        || observation.capacity_policy != *capacity_info.key
        || observation.capacity_policy_digest != capacity.policy_digest
        || observation.purpose != purpose
        || observation.subject != *subject
        || observation.subject_digest != subject_digest
        || observation.target_program != *observed_program
        || observation.target_programdata
            != derive_upgradeable_programdata_address(observed_program).0
        || observation.upgradeable_loader != config.upgradeable_loader
        || observation.status != ProgramDataObservationStatusV1::Finalized
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(observation)
}

fn load_immutability_receipt(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    capacity: &ProgramDataCapacityPolicyV1,
    config: &ControllerConfigV1,
) -> Result<Box<ControllerImmutabilityReceiptV1>, ProgramError> {
    let receipt = load_fixed_controller_account::<ControllerImmutabilityReceiptV1>(
        program_id,
        info,
        ControllerImmutabilityReceiptV1::LEN,
    )?;
    validate_controller_immutability_receipt_digest_v1(&receipt)?;
    let expected = derive_controller_immutability_receipt_pda(program_id, &config.target_program);
    if expected.0 != *info.key
        || expected.1 != receipt.bump
        || receipt.controller_program != *program_id
        || receipt.controller_programdata != derive_upgradeable_programdata_address(program_id).0
        || receipt.upgradeable_loader != config.upgradeable_loader
        || receipt.capacity_policy != *capacity_info.key
        || receipt.capacity_policy_digest != capacity.policy_digest
    {
        return Err(GovernanceError::ControllerNotImmutable.into());
    }
    Ok(receipt)
}

fn load_handoff_receipt(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    context: &CeremonyContextV2,
) -> Result<Box<TargetAuthorityHandoffReceiptV1>, ProgramError> {
    let receipt = load_fixed_controller_account::<TargetAuthorityHandoffReceiptV1>(
        program_id,
        info,
        TargetAuthorityHandoffReceiptV1::LEN,
    )?;
    validate_target_handoff_receipt_digest_v1(&receipt)?;
    let expected = derive_target_handoff_receipt_pda(program_id, &context.config.target_program);
    if expected.0 != *info.key
        || expected.1 != receipt.bump
        || receipt.controller_program != *program_id
        || receipt.controller_config != *config_info.key
        || receipt.controller_authority != context.config.authority_pda
        || receipt.controller_immutability_receipt != *immutability_info.key
        || receipt.target_program != context.config.target_program
        || receipt.target_programdata != context.config.target_programdata
        || receipt.upgradeable_loader != context.config.upgradeable_loader
        || receipt.bootstrap_gate_epoch != context.gate.epoch
        || receipt.target_nonce != context.config.target_nonce
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(receipt)
}

fn load_handoff_proposal(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<TargetAuthorityHandoffProposalV2>, ProgramError> {
    let proposal = load_fixed_controller_account::<TargetAuthorityHandoffProposalV2>(
        program_id,
        info,
        TargetAuthorityHandoffProposalV2::LEN,
    )?;
    proposal.validate_static()?;
    let expected = derive_governance_action_proposal_v2(
        program_id,
        &config.target_program,
        GovernanceActionKindV2::TargetAuthorityHandoff,
        proposal.proposal_id,
    );
    if expected.0 != *info.key
        || expected.1 != proposal.bump
        || proposal.controller_program != *program_id
        || proposal.controller_config
            != derive_controller_config_pda(program_id, &config.target_program).0
        || proposal.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
}

fn load_activation_proposal(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<BootstrapActivationProposalV2>, ProgramError> {
    let proposal = load_fixed_controller_account::<BootstrapActivationProposalV2>(
        program_id,
        info,
        BootstrapActivationProposalV2::LEN,
    )?;
    proposal.validate_static()?;
    let expected = derive_governance_action_proposal_v2(
        program_id,
        &config.target_program,
        GovernanceActionKindV2::BootstrapActivation,
        proposal.proposal_id,
    );
    if expected.0 != *info.key
        || expected.1 != proposal.bump
        || proposal.controller_program != *program_id
        || proposal.controller_config
            != derive_controller_config_pda(program_id, &config.target_program).0
        || proposal.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
}

fn validate_controller_still_immutable(
    program_id: &Pubkey,
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    receipt: &ControllerImmutabilityReceiptV1,
) -> ProgramResult {
    if *controller_program.key != *program_id
        || *controller_programdata.key != receipt.controller_programdata
    {
        return Err(GovernanceError::ControllerNotImmutable.into());
    }
    let header = validate_program_programdata_linkage(
        controller_program,
        controller_programdata,
        &receipt.upgradeable_loader,
    )?;
    if header.upgrade_authority.is_some()
        || header.deployed_slot != receipt.deployed_slot
        || u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != receipt.programdata_capacity
    {
        return Err(GovernanceError::ControllerNotImmutable.into());
    }
    Ok(())
}

fn copy_program_header(program: &AccountInfo<'_>) -> Result<[u8; 36], ProgramError> {
    let data = program.try_borrow_data()?;
    data.get(..36)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account.into())
}

fn copy_programdata_header(
    programdata: &AccountInfo<'_>,
) -> Result<[u8; LOADER_PROGRAMDATA_METADATA_LEN], ProgramError> {
    let data = programdata.try_borrow_data()?;
    data.get(..LOADER_PROGRAMDATA_METADATA_LEN)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account.into())
}

fn optional_matches(value: &OptionalPubkeyV1, expected: Option<Pubkey>) -> bool {
    match expected {
        Some(key) => value.present && value.value == key,
        None => !value.present && value.value == Pubkey::default(),
    }
}

fn require_live_observation(
    program: &AccountInfo<'_>,
    programdata: &AccountInfo<'_>,
    observation: &ProgramDataObservationV1,
    expected_authority: Option<Pubkey>,
) -> ProgramResult {
    if *program.key != observation.target_program
        || *programdata.key != observation.target_programdata
        || program.owner != &observation.program_owner
        || programdata.owner != &observation.programdata_owner
        || program.executable != observation.program_executable
        || programdata.executable != observation.programdata_executable
        || u64::try_from(program.data_len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.program_data_length
        || u64::try_from(programdata.data_len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.raw_data_length
        || copy_program_header(program)? != observation.program_header_snapshot
        || copy_programdata_header(programdata)? != observation.programdata_header_snapshot
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    let header = validate_program_programdata_linkage(
        program,
        programdata,
        &observation.upgradeable_loader,
    )?;
    if header.deployed_slot != observation.deployed_slot
        || u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != observation.actual_capacity
        || header.upgrade_authority != expected_authority
        || !optional_matches(&observation.upgrade_authority, expected_authority)
    {
        return Err(GovernanceError::StaleProgramDataObservation.into());
    }
    Ok(())
}

fn validate_target_graph(
    config: &ControllerConfigV1,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority: &AccountInfo<'_>,
    loader: &AccountInfo<'_>,
) -> ProgramResult {
    if *target_program.key != config.target_program
        || *target_programdata.key != config.target_programdata
        || *authority.key != config.authority_pda
        || *loader.key != config.upgradeable_loader
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn require_active_seat(
    council: &GovernanceCouncilSetV1,
    authority: &Pubkey,
    slot: u64,
) -> Result<u8, ProgramError> {
    let index = council
        .seats
        .iter()
        .position(|seat| seat.seat_authority == *authority && seat.term_covers(slot))
        .ok_or(GovernanceError::InactiveCouncilSeat)?;
    u8::try_from(index).map_err(|_| GovernanceError::ArithmeticOverflow.into())
}

fn reject_guardian(config: &ControllerConfigV1, authority: &Pubkey) -> ProgramResult {
    if *authority == config.guardian {
        return Err(GovernanceError::UnknownSeatAuthority.into());
    }
    Ok(())
}

fn require_mask_active(
    council: &GovernanceCouncilSetV1,
    bitset: u8,
    count: u8,
    slot: u64,
) -> ProgramResult {
    if bitset & !0b1_1111 != 0 || bitset.count_ones() as u8 != count {
        return Err(GovernanceError::ApprovalCountMismatch.into());
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if bitset & (1u8 << index) != 0 && !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat.into());
        }
    }
    Ok(())
}

fn require_exact_quorum(
    council: &GovernanceCouncilSetV1,
    bitset: u8,
    count: u8,
    slot: u64,
) -> ProgramResult {
    require_mask_active(council, bitset, count, slot)?;
    if count != EQUAL_SEAT_THRESHOLD_V2 || !council.active_at(slot) {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    Ok(())
}

fn validate_guard(
    guard: &GovernanceActionGuardV2,
    proposal_id: u64,
    proposal_digest: &[u8; 32],
    council_version: u64,
    profile_version: u64,
    profile_hash: &[u8; 32],
) -> ProgramResult {
    guard.validate()?;
    if guard.proposal_id != proposal_id
        || guard.expected_proposal_digest != *proposal_digest
        || guard.expected_council_version != council_version
        || guard.expected_timing_profile_version != profile_version
        || guard.expected_timing_profile_hash != *profile_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn validate_handoff_identity_graph(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    profile_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    profile: &GovernanceTimingProfileV1,
    proposal: &TargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    let timing = derive_proposal_timing_v2(
        profile,
        GovernanceTimingClassV1::Constitutional,
        proposal.creation_slot,
    )?;
    if proposal.lifecycle_registry != *registry_info.key
        || proposal.governing_timing_profile != *profile_info.key
        || proposal.governing_timing_profile_version != profile.profile_version
        || proposal.governing_timing_profile_hash != profile.profile_hash
        || proposal.cluster_domain != config.cluster_domain
        || proposal.controller_program != *program_id
        || proposal.controller_programdata != derive_upgradeable_programdata_address(program_id).0
        || proposal.controller_config != *config_info.key
        || proposal.controller_authority != config.authority_pda
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.review_duration_slots != timing.review_duration_slots
        || proposal.delay_duration_slots != timing.delay_duration_slots
        || proposal.expiry_duration_slots != timing.expiry_duration_slots
        || proposal.review_start_slot != timing.review_start_slot
        || proposal.review_end_slot != timing.review_end_slot
        || proposal.not_before_slot != timing.not_before_slot
        || proposal.expiry_slot != timing.expiry_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_handoff_live_governors(
    registry_info: &AccountInfo<'_>,
    _profile_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    context: &CeremonyContextV2,
    proposal: &TargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    if proposal.lifecycle_registry != *registry_info.key
        || proposal.governance_policy != *policy_info.key
        || proposal.governance_policy_hash != context.policy.policy_hash
        || proposal.gate != *gate_info.key
        || proposal.bootstrap_gate_status != context.gate.status
        || proposal.bootstrap_gate_epoch != context.gate.epoch
        || proposal.bootstrap_freeze_reason_code != context.gate.freeze_reason_code
        || proposal.bootstrap_freeze_slot != context.gate.freeze_slot
        || proposal.target_nonce != context.config.target_nonce
        || proposal.council_version != context.council.version
        || proposal.council_hash != context.council.set_hash
        || *council_info.key
            != derive_council_pda(
                &proposal.controller_program,
                &proposal.target_program,
                proposal.council_version,
            )
            .0
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(())
}

fn validate_activation_identity_graph(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    profile_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    profile: &GovernanceTimingProfileV1,
    proposal: &BootstrapActivationProposalV2,
) -> ProgramResult {
    let timing = derive_proposal_timing_v2(
        profile,
        GovernanceTimingClassV1::Routine,
        proposal.creation_slot,
    )?;
    if proposal.lifecycle_registry != *registry_info.key
        || proposal.governing_timing_profile != *profile_info.key
        || proposal.governing_timing_profile_version != profile.profile_version
        || proposal.governing_timing_profile_hash != profile.profile_hash
        || proposal.cluster_domain != config.cluster_domain
        || proposal.controller_program != *program_id
        || proposal.controller_programdata != derive_upgradeable_programdata_address(program_id).0
        || proposal.controller_config != *config_info.key
        || proposal.controller_authority != config.authority_pda
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.review_duration_slots != timing.review_duration_slots
        || proposal.delay_duration_slots != timing.delay_duration_slots
        || proposal.expiry_duration_slots != timing.expiry_duration_slots
        || proposal.review_start_slot != timing.review_start_slot
        || proposal.review_end_slot != timing.review_end_slot
        || proposal.not_before_slot != timing.not_before_slot
        || proposal.expiry_slot != timing.expiry_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_activation_live_governors(
    registry_info: &AccountInfo<'_>,
    _profile_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    context: &CeremonyContextV2,
    proposal: &BootstrapActivationProposalV2,
) -> ProgramResult {
    if proposal.lifecycle_registry != *registry_info.key
        || proposal.governance_policy != *policy_info.key
        || proposal.governance_policy_hash != context.policy.policy_hash
        || proposal.gate != *gate_info.key
        || proposal.bootstrap_gate_status != context.gate.status
        || proposal.bootstrap_gate_epoch != context.gate.epoch
        || proposal.bootstrap_freeze_reason_code != context.gate.freeze_reason_code
        || proposal.bootstrap_freeze_slot != context.gate.freeze_slot
        || proposal.target_nonce != context.config.target_nonce
        || proposal.council_version != context.council.version
        || proposal.council_hash != context.council.set_hash
        || *council_info.key
            != derive_council_pda(
                &proposal.controller_program,
                &proposal.target_program,
                proposal.council_version,
            )
            .0
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_handoff_proposal_evidence(
    proposal: &TargetAuthorityHandoffProposalV2,
    context: &CeremonyContextV2,
    capacity_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    immutable: &ControllerImmutabilityReceiptV1,
    observation_info: &AccountInfo<'_>,
    observation: &ProgramDataObservationV1,
    legacy_authority: &Pubkey,
) -> ProgramResult {
    let same_generation = proposal.bridge_observation_generation == observation.generation;
    if proposal.capacity_policy != *capacity_info.key
        || proposal.capacity_policy_digest != context.capacity.policy_digest
        || proposal.controller_immutability_receipt != *immutability_info.key
        || proposal.controller_immutability_digest != immutable.receipt_digest
        || observation.generation < proposal.bridge_observation_generation
        || (same_generation
            && (proposal.bridge_observation != *observation_info.key
                || proposal.bridge_observation_root != observation.final_raw_merkle_root
                || proposal.bridge_observation_digest != observation.observation_digest))
        || proposal.legacy_target_authority != *legacy_authority
        || proposal.bridge_artifact_length != observation.expected_artifact_length
        || proposal.bridge_artifact_sha256 != observation.expected_artifact_sha256
        || proposal.bridge_artifact_merkle_root != observation.expected_artifact_merkle_root
        || proposal.bridge_artifact_scheme_id != observation.expected_artifact_scheme_id
        || proposal.minimum_target_deployed_slot > observation.deployed_slot
        || proposal.minimum_target_capacity > observation.actual_capacity
        || proposal.minimum_target_raw_length > observation.raw_data_length
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn validate_activation_proposal_evidence(
    proposal: &BootstrapActivationProposalV2,
    evidence: &ActivationEvidenceV2,
    capacity_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    handoff_receipt_info: &AccountInfo<'_>,
) -> ProgramResult {
    let same_generation = proposal.bridge_observation_generation == evidence.observation.generation;
    if proposal.capacity_policy != *capacity_info.key
        || proposal.capacity_policy_digest != evidence.context.capacity.policy_digest
        || proposal.controller_immutability_receipt != *immutability_info.key
        || proposal.controller_immutability_digest != evidence.immutable.receipt_digest
        || proposal.target_handoff_receipt != *handoff_receipt_info.key
        || proposal.target_handoff_digest != evidence.handoff.receipt_digest
        || evidence.observation.generation < proposal.bridge_observation_generation
        || (same_generation
            && (proposal.bridge_observation != *observation_info.key
                || proposal.bridge_observation_root != evidence.observation.final_raw_merkle_root
                || proposal.bridge_observation_digest != evidence.observation.observation_digest))
        || proposal.bridge_artifact_length != evidence.handoff.artifact_length
        || proposal.bridge_artifact_sha256 != evidence.handoff.artifact_sha256
        || proposal.bridge_artifact_merkle_root != evidence.handoff.artifact_merkle_root
        || proposal.bridge_artifact_scheme_id != evidence.handoff.artifact_scheme_id
        || proposal.bridge_source_commitment != evidence.handoff.bridge_source_commitment
        || proposal.bridge_build_inputs_commitment
            != evidence.handoff.bridge_build_inputs_commitment
        || proposal.bridge_package_commitment != evidence.handoff.bridge_package_commitment
        || proposal.bridge_release_manifest_commitment
            != evidence.handoff.bridge_release_manifest_commitment
        || proposal.minimum_target_deployed_slot > evidence.observation.deployed_slot
        || proposal.minimum_target_capacity > evidence.observation.actual_capacity
        || proposal.minimum_target_raw_length > evidence.observation.raw_data_length
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn load_handoff_evidence(
    program_id: &Pubkey,
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    legacy_authority: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    profile_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    slot: u64,
) -> Result<HandoffEvidenceV2, ProgramError> {
    let context = load_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        registry_info,
        profile_info,
        slot,
    )?;
    let immutable = load_immutability_receipt(
        program_id,
        immutability_info,
        capacity_info,
        &context.capacity,
        &context.config,
    )?;
    validate_controller_still_immutable(
        program_id,
        controller_program,
        controller_programdata,
        &immutable,
    )?;
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        &context.config,
        capacity_info,
        &context.capacity,
        target_program.key,
        ProgramDataObservationPurposeV1::TargetHandoffBridge,
        immutability_info.key,
    )?;
    validate_target_graph(
        &context.config,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    )?;
    require_live_observation(
        target_program,
        target_programdata,
        &observation,
        Some(*legacy_authority.key),
    )?;
    Ok(HandoffEvidenceV2 {
        context,
        immutable,
        observation,
    })
}

#[allow(clippy::too_many_arguments)]
fn load_activation_evidence(
    program_id: &Pubkey,
    controller_program: &AccountInfo<'_>,
    controller_programdata: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    handoff_receipt_info: &AccountInfo<'_>,
    observation_info: &AccountInfo<'_>,
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    profile_info: &AccountInfo<'_>,
    loader_info: &AccountInfo<'_>,
    slot: u64,
) -> Result<ActivationEvidenceV2, ProgramError> {
    let context = load_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        registry_info,
        profile_info,
        slot,
    )?;
    let immutable = load_immutability_receipt(
        program_id,
        immutability_info,
        capacity_info,
        &context.capacity,
        &context.config,
    )?;
    validate_controller_still_immutable(
        program_id,
        controller_program,
        controller_programdata,
        &immutable,
    )?;
    let handoff = load_handoff_receipt(
        program_id,
        handoff_receipt_info,
        config_info,
        immutability_info,
        &context,
    )?;
    if handoff.controller_immutability_digest != immutable.receipt_digest {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let observation = load_observation(
        program_id,
        observation_info,
        config_info,
        &context.config,
        capacity_info,
        &context.capacity,
        target_program.key,
        ProgramDataObservationPurposeV1::BootstrapActivation,
        handoff_receipt_info.key,
    )?;
    validate_target_graph(
        &context.config,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
    )?;
    require_live_observation(
        target_program,
        target_programdata,
        &observation,
        Some(context.config.authority_pda),
    )?;
    if observation.expected_artifact_length != handoff.artifact_length
        || observation.expected_artifact_sha256 != handoff.artifact_sha256
        || observation.expected_artifact_merkle_root != handoff.artifact_merkle_root
        || observation.expected_artifact_scheme_id != handoff.artifact_scheme_id
        || observation.deployed_slot < handoff.deployed_slot
        || observation.actual_capacity < handoff.programdata_capacity
        || observation.raw_data_length < handoff.raw_programdata_length
        || *authority_info.key != handoff.controller_authority
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(ActivationEvidenceV2 {
        context,
        immutable,
        handoff,
        observation,
    })
}

fn expected_handoff_post_header(
    pre: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    legacy: Pubkey,
    controller: Pubkey,
) -> Result<[u8; LOADER_PROGRAMDATA_METADATA_LEN], ProgramError> {
    if parse_upgradeable_programdata(pre)?.upgrade_authority != Some(legacy) {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    let mut post = *pre;
    post[12] = 1;
    post[13..45].copy_from_slice(controller.as_ref());
    Ok(post)
}

fn validate_some_to_some_header_delta(
    pre: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    post: &[u8; LOADER_PROGRAMDATA_METADATA_LEN],
    legacy: Pubkey,
    controller: Pubkey,
) -> ProgramResult {
    if parse_upgradeable_programdata(pre)?.upgrade_authority != Some(legacy)
        || parse_upgradeable_programdata(post)?.upgrade_authority != Some(controller)
        || pre[..12] != post[..12]
        || pre[12] != 1
        || post[12] != 1
        || post[13..45] != controller.to_bytes()
        || pre[13..45] != legacy.to_bytes()
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    Ok(())
}

fn validate_checked_handoff_cpi_shape(
    instruction: &Instruction,
    programdata: &Pubkey,
    legacy: &Pubkey,
    controller: &Pubkey,
) -> ProgramResult {
    if instruction.program_id != UPGRADEABLE_LOADER_ID
        || instruction.accounts.len() != 3
        || instruction.accounts[0].pubkey != *programdata
        || !instruction.accounts[0].is_writable
        || instruction.accounts[0].is_signer
        || instruction.accounts[1].pubkey != *legacy
        || instruction.accounts[1].is_writable
        || !instruction.accounts[1].is_signer
        || instruction.accounts[2].pubkey != *controller
        || instruction.accounts[2].is_writable
        || !instruction.accounts[2].is_signer
        || instruction.data != 7u32.to_le_bytes()
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    Ok(())
}

fn validate_canonical_envelope(
    program_id: &Pubkey,
    current_accounts: &[AccountInfo<'_>],
    instructions_info: &AccountInfo<'_>,
    expected_current_data: &[u8],
    envelope: &CeremonyEnvelopeV1,
) -> ProgramResult {
    envelope.validate()?;
    if envelope.compute_unit_limit == 0
        || envelope.compute_unit_limit > MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1
        || envelope.compute_unit_price_micro_lamports
            > MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1
        || *instructions_info.key != sysvar_ids::instructions::ID
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let nonce = envelope
        .durable_nonce_account
        .present
        .then_some(envelope.durable_nonce_account.value);
    let nonce_authority = envelope
        .durable_nonce_authority
        .present
        .then_some(envelope.durable_nonce_authority.value);
    if nonce.is_some() != nonce_authority.is_some() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let current_index = if nonce.is_some() { 3usize } else { 2usize };
    if usize::from(instructions::load_current_index_checked(instructions_info)?) != current_index {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let mut index = 0usize;
    if let (Some(nonce), Some(authority)) = (nonce, nonce_authority) {
        if instructions::load_instruction_at_checked(index, instructions_info)?
            != system_instruction::advance_nonce_account(&nonce, &authority)
        {
            return Err(GovernanceError::CrossAccountMismatch.into());
        }
        index += 1;
    }
    let limit = instructions::load_instruction_at_checked(index, instructions_info)?;
    let mut limit_data = [0u8; 5];
    limit_data[0] = 2;
    limit_data[1..].copy_from_slice(&envelope.compute_unit_limit.to_le_bytes());
    if limit.program_id != compute_budget::ID
        || !limit.accounts.is_empty()
        || limit.data != limit_data
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    index += 1;
    let price = instructions::load_instruction_at_checked(index, instructions_info)?;
    let mut price_data = [0u8; 9];
    price_data[0] = 3;
    price_data[1..].copy_from_slice(&envelope.compute_unit_price_micro_lamports.to_le_bytes());
    if price.program_id != compute_budget::ID
        || !price.accounts.is_empty()
        || price.data != price_data
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    index += 1;
    let current = instructions::load_instruction_at_checked(index, instructions_info)?;
    if current.program_id != *program_id
        || current.data != expected_current_data
        || current.accounts.len() != current_accounts.len()
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    for (meta, info) in current.accounts.iter().zip(current_accounts) {
        if meta.pubkey != *info.key
            || meta.is_signer != info.is_signer
            || meta.is_writable != info.is_writable
        {
            return Err(GovernanceError::InvalidAccountPrivileges.into());
        }
    }
    if instructions::load_instruction_at_checked(index + 1, instructions_info).is_ok() {
        return Err(GovernanceError::InvalidAccountCount.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_proposal_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    target_program: &Pubkey,
    kind: GovernanceActionKindV2,
    proposal_id: u64,
    bump: u8,
    len: usize,
) -> ProgramResult {
    let kind_seed = [kind as u8];
    let proposal_seed = proposal_id.to_le_bytes();
    let bump_seed = [bump];
    create_fixed_pda_account(
        program_id,
        payer,
        account,
        system_program_info,
        &Rent::get()?,
        len,
        &[
            GOVERNANCE_V2_SEED_PREFIX,
            GOVERNANCE_ACTION_PROPOSAL_V2_SEED,
            target_program.as_ref(),
            &kind_seed,
            &proposal_seed,
            &bump_seed,
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn create_or_reuse_zero_fixed_pda<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    pda: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    rent: &Rent,
    space: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    ws(payer)?;
    rw(pda)?;
    system_program_account(system_program_info)?;
    let expected = Pubkey::create_program_address(signer_seeds, program_id)
        .map_err(|_| GovernanceError::InvalidPda)?;
    if expected != *pda.key {
        return Err(GovernanceError::InvalidPda.into());
    }
    if pda.owner == program_id {
        if pda.data_len() != space || pda.lamports() < rent.minimum_balance(space) {
            return Err(GovernanceError::InvalidRelease1Account.into());
        }
        if pda.try_borrow_data()?.iter().any(|byte| *byte != 0) {
            return Err(GovernanceError::CeremonyAlreadyFinalized.into());
        }
        return Ok(());
    }
    if pda.owner != &system_program::ID || pda.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    create_fixed_pda_account(
        program_id,
        payer,
        pda,
        system_program_info,
        rent,
        space,
        signer_seeds,
    )
}

fn commit_preencoded(program_id: &Pubkey, values: &[(&AccountInfo<'_>, &[u8])]) -> ProgramResult {
    for (account, bytes) in values {
        if account.owner != program_id {
            return Err(GovernanceError::IncorrectAccountOwner.into());
        }
        if account.data_len() != bytes.len() {
            return Err(GovernanceError::InvalidAccountSize.into());
        }
    }
    let mut borrows = Vec::with_capacity(values.len());
    for (account, _) in values {
        borrows.push(account.try_borrow_mut_data()?);
    }
    for ((_, bytes), data) in values.iter().zip(borrows.iter_mut()) {
        data.copy_from_slice(bytes);
    }
    Ok(())
}

/// Creates a unique, monotonic V2 checked-authority handoff proposal.
///
/// Accounts: payer W/S; creator seat RO/S; controller Program RO/X; controller ProgramData RO;
/// config, policy, council, gate, capacity, immutability receipt, bridge observation RO;
/// target Program RO/X; target ProgramData, legacy authority, controller authority RO;
/// lifecycle registry W; timing profile RO; proposal W; Loader and System Program RO/X.
pub fn process_create_target_authority_handoff_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateTargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        CREATE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [payer, creator, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, registry_info, profile_info, proposal_info, loader_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    ws(payer)?;
    rs(creator)?;
    validate_seat_authority(creator)?;
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_programdata,
        legacy_authority,
        authority_info,
        profile_info,
    ] {
        ro(account)?;
    }
    executable(target_program)?;
    rw(registry_info)?;
    rw(proposal_info)?;
    executable(loader_info)?;
    system_program_account(system_program_info)?;

    let slot = current_slot()?;
    let mut evidence = load_handoff_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    require_registry_profile(
        &evidence.context.registry,
        profile_info,
        &evidence.context.profile,
    )?;
    reject_guardian(&evidence.context.config, creator.key)?;
    require_active_seat(&evidence.context.council, creator.key, slot)?;
    if *legacy_authority.key == Pubkey::default()
        || *legacy_authority.key == evidence.context.config.guardian
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
        || instruction.expected_council_version != evidence.context.council.version
        || instruction.expected_timing_profile_version != evidence.context.profile.profile_version
        || instruction.expected_timing_profile_hash != evidence.context.profile.profile_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let proposal_id = evidence
        .context
        .registry
        .consume_proposal_id(instruction.expected_proposal_id)?;
    let timing = derive_proposal_timing_v2(
        &evidence.context.profile,
        GovernanceTimingClassV1::Constitutional,
        slot,
    )?;
    let (expected_proposal, bump) = derive_governance_action_proposal_v2(
        program_id,
        &evidence.context.config.target_program,
        GovernanceActionKindV2::TargetAuthorityHandoff,
        proposal_id,
    );
    if *proposal_info.key != expected_proposal {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut proposal = TargetAuthorityHandoffProposalV2 {
        discriminator: TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_DISCRIMINATOR,
        version: GOVERNANCE_V2_ACCOUNT_VERSION,
        bump,
        initialized: true,
        state: GovernanceLifecycleStateV2::Draft,
        proposal_id,
        lifecycle_registry: *registry_info.key,
        governing_timing_profile: *profile_info.key,
        governing_timing_profile_version: evidence.context.profile.profile_version,
        governing_timing_profile_hash: evidence.context.profile.profile_hash,
        cluster_domain: evidence.context.config.cluster_domain,
        controller_program: *program_id,
        controller_programdata: *controller_programdata.key,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: evidence.immutable.receipt_digest,
        controller_config: *config_info.key,
        governance_policy: *policy_info.key,
        governance_policy_hash: evidence.context.policy.policy_hash,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: evidence.context.capacity.policy_digest,
        gate: *gate_info.key,
        controller_authority: evidence.context.config.authority_pda,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        legacy_target_authority: *legacy_authority.key,
        bridge_artifact_length: evidence.observation.expected_artifact_length,
        bridge_artifact_sha256: evidence.observation.expected_artifact_sha256,
        bridge_artifact_merkle_root: evidence.observation.expected_artifact_merkle_root,
        bridge_artifact_scheme_id: evidence.observation.expected_artifact_scheme_id,
        bridge_source_commitment: instruction.bridge_source_commitment,
        bridge_build_inputs_commitment: instruction.bridge_build_inputs_commitment,
        bridge_package_commitment: instruction.bridge_package_commitment,
        bridge_release_manifest_commitment: instruction.bridge_release_manifest_commitment,
        bridge_observation: *observation_info.key,
        bridge_observation_generation: evidence.observation.generation,
        bridge_observation_root: evidence.observation.final_raw_merkle_root,
        bridge_observation_digest: evidence.observation.observation_digest,
        minimum_target_deployed_slot: evidence.observation.deployed_slot,
        minimum_target_capacity: evidence.observation.actual_capacity,
        minimum_target_raw_length: evidence.observation.raw_data_length,
        bootstrap_gate_status: evidence.context.gate.status,
        bootstrap_gate_epoch: evidence.context.gate.epoch,
        bootstrap_freeze_reason_code: evidence.context.gate.freeze_reason_code,
        bootstrap_freeze_slot: evidence.context.gate.freeze_slot,
        target_nonce: evidence.context.config.target_nonce,
        council_version: evidence.context.council.version,
        council_hash: evidence.context.council.set_hash,
        review_duration_slots: timing.review_duration_slots,
        delay_duration_slots: timing.delay_duration_slots,
        expiry_duration_slots: timing.expiry_duration_slots,
        review_start_slot: timing.review_start_slot,
        review_end_slot: timing.review_end_slot,
        not_before_slot: timing.not_before_slot,
        expiry_slot: timing.expiry_slot,
        approval_bitset: 0,
        approval_count: 0,
        approval_threshold: EQUAL_SEAT_THRESHOLD_V2,
        cancellation_bitset: 0,
        cancellation_count: 0,
        cancellation_threshold: EQUAL_SEAT_THRESHOLD_V2,
        first_approval_slot: 0,
        council_approved_slot: 0,
        queued_slot: 0,
        executed_slot: 0,
        terminal_slot: 0,
        terminal_reason_code: 0,
        proposal_digest: [0; 32],
        creation_slot: slot,
        reserved: [0; TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_RESERVED_LEN],
    };
    proposal.proposal_digest = compute_target_authority_handoff_digest_v2(&proposal)?;
    proposal.validate_static()?;
    evidence.context.registry.validate_static()?;
    let registry_bytes = encode_fixed_account(
        &*evidence.context.registry,
        GovernanceLifecycleRegistryV2::LEN,
    )?;
    let proposal_bytes = encode_fixed_account(&proposal, TargetAuthorityHandoffProposalV2::LEN)?;
    create_proposal_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &evidence.context.config.target_program,
        GovernanceActionKindV2::TargetAuthorityHandoff,
        proposal_id,
        bump,
        TargetAuthorityHandoffProposalV2::LEN,
    )?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
        ],
    )
}

pub fn process_approve_target_authority_handoff_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveTargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        APPROVE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, registry_info, profile_info, proposal_info, loader_info, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_programdata,
        legacy_authority,
        authority_info,
        registry_info,
        profile_info,
    ] {
        ro(account)?;
    }
    executable(target_program)?;
    rw(proposal_info)?;
    executable(loader_info)?;
    rs(seat)?;
    validate_seat_authority(seat)?;

    let slot = current_slot()?;
    let evidence = load_handoff_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    let mut proposal = load_handoff_proposal(program_id, proposal_info, &evidence.context.config)?;
    validate_handoff_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &evidence.context.config,
        &evidence.context.profile,
        &proposal,
    )?;
    validate_handoff_live_governors(
        registry_info,
        profile_info,
        policy_info,
        council_info,
        gate_info,
        &evidence.context,
        &proposal,
    )?;
    validate_handoff_proposal_evidence(
        &proposal,
        &evidence.context,
        capacity_info,
        immutability_info,
        &evidence.immutable,
        observation_info,
        &evidence.observation,
        legacy_authority.key,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    if proposal.state != GovernanceLifecycleStateV2::Draft {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    reject_guardian(&evidence.context.config, seat.key)?;
    let seat_index = require_active_seat(&evidence.context.council, seat.key, slot)?;
    let crossed = record_governance_approval_v2(
        &mut proposal.approval_bitset,
        &mut proposal.approval_count,
        seat_index,
        slot,
        proposal.review_start_slot,
        proposal.review_end_slot,
        proposal.expiry_slot,
    )?;
    if proposal.first_approval_slot == 0 {
        proposal.first_approval_slot = slot;
    }
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, TargetAuthorityHandoffProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

pub fn process_cancel_target_authority_handoff_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelTargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        CANCEL_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, profile_info, proposal_info, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for account in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        profile_info,
    ] {
        ro(account)?;
    }
    rw(proposal_info)?;
    rs(seat)?;
    validate_seat_authority(seat)?;
    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config, slot)?;
    let council = load_council(
        program_id,
        council_info,
        config_info,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    validate_bootstrap_gate(&gate, &config)?;
    let _registry = load_registry(program_id, registry_info, config_info, &config)?;
    let profile = load_profile(program_id, profile_info, config_info, &config)?;
    let mut proposal = load_handoff_proposal(program_id, proposal_info, &config)?;
    validate_handoff_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &config,
        &profile,
        &proposal,
    )?;
    if proposal.governance_policy != *policy_info.key
        || proposal.governance_policy_hash != policy.policy_hash
        || proposal.gate != *gate_info.key
        || proposal.bootstrap_gate_status != gate.status
        || proposal.bootstrap_gate_epoch != gate.epoch
        || proposal.bootstrap_freeze_reason_code != gate.freeze_reason_code
        || proposal.bootstrap_freeze_slot != gate.freeze_slot
        || proposal.target_nonce != config.target_nonce
        || proposal.council_version != council.version
        || proposal.council_hash != council.set_hash
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    reject_guardian(&config, seat.key)?;
    let seat_index = require_active_seat(&council, seat.key, slot)?;
    let crossed = record_governance_cancellation_v2(
        proposal.state,
        &mut proposal.cancellation_bitset,
        &mut proposal.cancellation_count,
        seat_index,
        slot,
        proposal.expiry_slot,
    )?;
    require_mask_active(
        &council,
        proposal.cancellation_bitset,
        proposal.cancellation_count,
        slot,
    )?;
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::Cancelled;
        proposal.terminal_slot = slot;
        proposal.terminal_reason_code = instruction.cancellation_reason_code;
    }
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, TargetAuthorityHandoffProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

pub fn process_expire_target_authority_handoff_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireTargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        EXPIRE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [config_info, registry_info, profile_info, proposal_info] = accounts else {
        unreachable!("account count checked")
    };
    ro(config_info)?;
    ro(registry_info)?;
    ro(profile_info)?;
    rw(proposal_info)?;
    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let _registry = load_registry(program_id, registry_info, config_info, &config)?;
    let profile = load_profile(program_id, profile_info, config_info, &config)?;
    let mut proposal = load_handoff_proposal(program_id, proposal_info, &config)?;
    validate_handoff_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &config,
        &profile,
        &proposal,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    require_governance_expirable_v2(proposal.state, slot, proposal.expiry_slot)?;
    proposal.state = GovernanceLifecycleStateV2::Expired;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_EXPIRED_REASON_V2;
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, TargetAuthorityHandoffProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

pub fn process_queue_target_authority_handoff_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueTargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        QUEUE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, registry_info, profile_info, proposal_info, loader_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_programdata,
        legacy_authority,
        authority_info,
        registry_info,
        profile_info,
    ] {
        ro(account)?;
    }
    executable(target_program)?;
    rw(proposal_info)?;
    executable(loader_info)?;
    let slot = current_slot()?;
    let evidence = load_handoff_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    let mut proposal = load_handoff_proposal(program_id, proposal_info, &evidence.context.config)?;
    validate_handoff_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &evidence.context.config,
        &evidence.context.profile,
        &proposal,
    )?;
    validate_handoff_live_governors(
        registry_info,
        profile_info,
        policy_info,
        council_info,
        gate_info,
        &evidence.context,
        &proposal,
    )?;
    validate_handoff_proposal_evidence(
        &proposal,
        &evidence.context,
        capacity_info,
        immutability_info,
        &evidence.immutable,
        observation_info,
        &evidence.observation,
        legacy_authority.key,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    require_governance_queueable_v2(proposal.state, slot, proposal.expiry_slot)?;
    require_exact_quorum(
        &evidence.context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    proposal.state = GovernanceLifecycleStateV2::Timelocked;
    proposal.queued_slot = slot;
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, TargetAuthorityHandoffProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

/// Executes exactly one checked Loader-v3 authority transfer and finalizes the singleton receipt.
pub fn process_execute_target_authority_handoff_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteTargetAuthorityHandoffProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        EXECUTE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [payer, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, registry_info, profile_info, proposal_info, observation_info, target_program, target_programdata, legacy_authority, authority_info, loader_info, receipt_info, system_program_info, instructions_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    ws(payer)?;
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        registry_info,
        profile_info,
        observation_info,
        authority_info,
        instructions_info,
    ] {
        ro(account)?;
    }
    rw(proposal_info)?;
    executable(target_program)?;
    rw(target_programdata)?;
    rs(legacy_authority)?;
    executable(loader_info)?;
    rw(receipt_info)?;
    system_program_account(system_program_info)?;
    if *instructions_info.key != sysvar_ids::instructions::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let slot = current_slot()?;
    let evidence = load_handoff_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        observation_info,
        target_program,
        target_programdata,
        legacy_authority,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    let mut proposal = load_handoff_proposal(program_id, proposal_info, &evidence.context.config)?;
    validate_handoff_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &evidence.context.config,
        &evidence.context.profile,
        &proposal,
    )?;
    validate_handoff_live_governors(
        registry_info,
        profile_info,
        policy_info,
        council_info,
        gate_info,
        &evidence.context,
        &proposal,
    )?;
    validate_handoff_proposal_evidence(
        &proposal,
        &evidence.context,
        capacity_info,
        immutability_info,
        &evidence.immutable,
        observation_info,
        &evidence.observation,
        legacy_authority.key,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    if instruction.expected_bridge_observation_digest != evidence.observation.observation_digest
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
        || *legacy_authority.key == evidence.context.config.guardian
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    require_governance_executable_v2(
        proposal.state,
        slot,
        proposal.not_before_slot,
        proposal.expiry_slot,
    )?;
    require_exact_quorum(
        &evidence.context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    let expected_current_data = instruction.pack()?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &expected_current_data,
        &instruction.envelope,
    )?;

    let (expected_receipt, receipt_bump) =
        derive_target_handoff_receipt_pda(program_id, &evidence.context.config.target_program);
    if *receipt_info.key != expected_receipt {
        return Err(GovernanceError::InvalidPda.into());
    }
    let pre_header = evidence.observation.programdata_header_snapshot;
    let expected_post = expected_handoff_post_header(
        &pre_header,
        *legacy_authority.key,
        evidence.context.config.authority_pda,
    )?;
    validate_some_to_some_header_delta(
        &pre_header,
        &expected_post,
        *legacy_authority.key,
        evidence.context.config.authority_pda,
    )?;

    let authority_bump =
        derive_authority_pda(program_id, &evidence.context.config.target_program).1;
    let authority_bump_seed = [authority_bump];
    let authority_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        evidence.context.config.target_program.as_ref(),
        &authority_bump_seed,
    ];
    let cpi =
        set_upgrade_authority_checked(target_program.key, legacy_authority.key, authority_info.key);
    validate_checked_handoff_cpi_shape(
        &cpi,
        target_programdata.key,
        legacy_authority.key,
        authority_info.key,
    )?;
    invoke_signed(
        &cpi,
        &[
            target_programdata.clone(),
            legacy_authority.clone(),
            authority_info.clone(),
            loader_info.clone(),
        ],
        &[authority_seeds],
    )?;

    let post_linkage = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &evidence.context.config.upgradeable_loader,
    )?;
    let post_header = copy_programdata_header(target_programdata)?;
    validate_some_to_some_header_delta(
        &pre_header,
        &post_header,
        *legacy_authority.key,
        evidence.context.config.authority_pda,
    )?;
    if post_linkage.upgrade_authority != Some(evidence.context.config.authority_pda)
        || post_linkage.deployed_slot != evidence.observation.deployed_slot
        || u64::try_from(post_linkage.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != evidence.observation.actual_capacity
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }

    proposal.state = GovernanceLifecycleStateV2::Completed;
    proposal.executed_slot = slot;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_COMPLETED_REASON_V2;
    proposal.validate_static()?;
    let mut receipt = TargetAuthorityHandoffReceiptV1 {
        discriminator: TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: receipt_bump,
        initialized: true,
        proposal: *proposal_info.key,
        proposal_digest: proposal.proposal_digest,
        controller_program: *program_id,
        controller_config: *config_info.key,
        controller_authority: evidence.context.config.authority_pda,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: evidence.immutable.receipt_digest,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        pre_observation: *observation_info.key,
        pre_observation_generation: evidence.observation.generation,
        pre_observation_root: evidence.observation.final_raw_merkle_root,
        pre_observation_digest: evidence.observation.observation_digest,
        pre_upgrade_authority: OptionalPubkeyV1::some(*legacy_authority.key)?,
        pre_programdata_header_snapshot: pre_header,
        post_programdata_header_snapshot: post_header,
        post_upgrade_authority: OptionalPubkeyV1::some(evidence.context.config.authority_pda)?,
        deployed_slot: evidence.observation.deployed_slot,
        raw_programdata_length: evidence.observation.raw_data_length,
        programdata_capacity: evidence.observation.actual_capacity,
        artifact_length: proposal.bridge_artifact_length,
        artifact_sha256: proposal.bridge_artifact_sha256,
        artifact_merkle_root: proposal.bridge_artifact_merkle_root,
        artifact_scheme_id: proposal.bridge_artifact_scheme_id,
        bridge_source_commitment: proposal.bridge_source_commitment,
        bridge_build_inputs_commitment: proposal.bridge_build_inputs_commitment,
        bridge_package_commitment: proposal.bridge_package_commitment,
        bridge_release_manifest_commitment: proposal.bridge_release_manifest_commitment,
        bootstrap_gate_epoch: proposal.bootstrap_gate_epoch,
        target_nonce: proposal.target_nonce,
        council_version: proposal.council_version,
        accepted_slot: slot,
        receipt_digest: [0; 32],
        finalized: true,
        reserved: [0; TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN],
    };
    receipt.receipt_digest = compute_target_handoff_receipt_digest_v1(&receipt)?;
    validate_target_handoff_receipt_digest_v1(&receipt)?;
    let proposal_bytes = encode_fixed_account(&*proposal, TargetAuthorityHandoffProposalV2::LEN)?;
    let receipt_bytes = encode_fixed_account(&receipt, TargetAuthorityHandoffReceiptV1::LEN)?;
    let receipt_bump_seed = [receipt_bump];
    create_or_reuse_zero_fixed_pda(
        program_id,
        payer,
        receipt_info,
        system_program_info,
        &Rent::get()?,
        TargetAuthorityHandoffReceiptV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            TARGET_HANDOFF_RECEIPT_SEED,
            evidence.context.config.target_program.as_ref(),
            &receipt_bump_seed,
        ],
    )?;
    commit_preencoded(
        program_id,
        &[
            (proposal_info, proposal_bytes.as_slice()),
            (receipt_info, receipt_bytes.as_slice()),
        ],
    )
}

/// Creates a unique V2 bootstrap-activation proposal. No receipt or deployment PDA is allocated.
pub fn process_create_bootstrap_activation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateBootstrapActivationProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        CREATE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [payer, creator, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, observation_info, target_program, target_programdata, authority_info, registry_info, profile_info, proposal_info, loader_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    ws(payer)?;
    rs(creator)?;
    validate_seat_authority(creator)?;
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_programdata,
        authority_info,
        profile_info,
    ] {
        ro(account)?;
    }
    executable(target_program)?;
    rw(registry_info)?;
    rw(proposal_info)?;
    executable(loader_info)?;
    system_program_account(system_program_info)?;
    let slot = current_slot()?;
    let mut evidence = load_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    require_registry_profile(
        &evidence.context.registry,
        profile_info,
        &evidence.context.profile,
    )?;
    reject_guardian(&evidence.context.config, creator.key)?;
    require_active_seat(&evidence.context.council, creator.key, slot)?;
    if instruction.expected_controller_immutability_digest != evidence.immutable.receipt_digest
        || instruction.expected_handoff_receipt_digest != evidence.handoff.receipt_digest
        || instruction.expected_bridge_observation_digest != evidence.observation.observation_digest
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
        || instruction.expected_council_version != evidence.context.council.version
        || instruction.expected_timing_profile_version != evidence.context.profile.profile_version
        || instruction.expected_timing_profile_hash != evidence.context.profile.profile_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let proposal_id = evidence
        .context
        .registry
        .consume_proposal_id(instruction.expected_proposal_id)?;
    let timing = derive_proposal_timing_v2(
        &evidence.context.profile,
        GovernanceTimingClassV1::Routine,
        slot,
    )?;
    let (expected_proposal, bump) = derive_governance_action_proposal_v2(
        program_id,
        &evidence.context.config.target_program,
        GovernanceActionKindV2::BootstrapActivation,
        proposal_id,
    );
    if *proposal_info.key != expected_proposal {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut proposal = BootstrapActivationProposalV2 {
        discriminator: BOOTSTRAP_ACTIVATION_PROPOSAL_V2_DISCRIMINATOR,
        version: GOVERNANCE_V2_ACCOUNT_VERSION,
        bump,
        initialized: true,
        state: GovernanceLifecycleStateV2::Draft,
        proposal_id,
        lifecycle_registry: *registry_info.key,
        governing_timing_profile: *profile_info.key,
        governing_timing_profile_version: evidence.context.profile.profile_version,
        governing_timing_profile_hash: evidence.context.profile.profile_hash,
        cluster_domain: evidence.context.config.cluster_domain,
        controller_program: *program_id,
        controller_programdata: *controller_programdata.key,
        controller_config: *config_info.key,
        governance_policy: *policy_info.key,
        governance_policy_hash: evidence.context.policy.policy_hash,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: evidence.context.capacity.policy_digest,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: evidence.immutable.receipt_digest,
        target_handoff_receipt: *handoff_receipt_info.key,
        target_handoff_digest: evidence.handoff.receipt_digest,
        gate: *gate_info.key,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        controller_authority: evidence.context.config.authority_pda,
        bridge_artifact_length: evidence.handoff.artifact_length,
        bridge_artifact_sha256: evidence.handoff.artifact_sha256,
        bridge_artifact_merkle_root: evidence.handoff.artifact_merkle_root,
        bridge_artifact_scheme_id: evidence.handoff.artifact_scheme_id,
        bridge_source_commitment: evidence.handoff.bridge_source_commitment,
        bridge_build_inputs_commitment: evidence.handoff.bridge_build_inputs_commitment,
        bridge_package_commitment: evidence.handoff.bridge_package_commitment,
        bridge_release_manifest_commitment: evidence.handoff.bridge_release_manifest_commitment,
        bridge_observation: *observation_info.key,
        bridge_observation_generation: evidence.observation.generation,
        bridge_observation_root: evidence.observation.final_raw_merkle_root,
        bridge_observation_digest: evidence.observation.observation_digest,
        minimum_target_deployed_slot: evidence.observation.deployed_slot,
        minimum_target_capacity: evidence.observation.actual_capacity,
        minimum_target_raw_length: evidence.observation.raw_data_length,
        bootstrap_gate_status: evidence.context.gate.status,
        bootstrap_gate_epoch: evidence.context.gate.epoch,
        bootstrap_freeze_reason_code: evidence.context.gate.freeze_reason_code,
        bootstrap_freeze_slot: evidence.context.gate.freeze_slot,
        target_nonce: evidence.context.config.target_nonce,
        council_version: evidence.context.council.version,
        council_hash: evidence.context.council.set_hash,
        review_duration_slots: timing.review_duration_slots,
        delay_duration_slots: timing.delay_duration_slots,
        expiry_duration_slots: timing.expiry_duration_slots,
        review_start_slot: timing.review_start_slot,
        review_end_slot: timing.review_end_slot,
        not_before_slot: timing.not_before_slot,
        expiry_slot: timing.expiry_slot,
        approval_bitset: 0,
        approval_count: 0,
        approval_threshold: EQUAL_SEAT_THRESHOLD_V2,
        cancellation_bitset: 0,
        cancellation_count: 0,
        cancellation_threshold: EQUAL_SEAT_THRESHOLD_V2,
        first_approval_slot: 0,
        council_approved_slot: 0,
        queued_slot: 0,
        executed_slot: 0,
        terminal_slot: 0,
        terminal_reason_code: 0,
        proposal_digest: [0; 32],
        creation_slot: slot,
        reserved: [0; BOOTSTRAP_ACTIVATION_PROPOSAL_V2_RESERVED_LEN],
    };
    proposal.proposal_digest = compute_bootstrap_activation_digest_v2(&proposal)?;
    proposal.validate_static()?;
    evidence.context.registry.validate_static()?;
    let registry_bytes = encode_fixed_account(
        &*evidence.context.registry,
        GovernanceLifecycleRegistryV2::LEN,
    )?;
    let proposal_bytes = encode_fixed_account(&proposal, BootstrapActivationProposalV2::LEN)?;
    create_proposal_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &evidence.context.config.target_program,
        GovernanceActionKindV2::BootstrapActivation,
        proposal_id,
        bump,
        BootstrapActivationProposalV2::LEN,
    )?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
        ],
    )
}

pub fn process_approve_bootstrap_activation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveBootstrapActivationProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        APPROVE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, observation_info, target_program, target_programdata, authority_info, registry_info, profile_info, proposal_info, loader_info, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_programdata,
        authority_info,
        registry_info,
        profile_info,
    ] {
        ro(account)?;
    }
    executable(target_program)?;
    rw(proposal_info)?;
    executable(loader_info)?;
    rs(seat)?;
    validate_seat_authority(seat)?;
    let slot = current_slot()?;
    let evidence = load_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    let mut proposal =
        load_activation_proposal(program_id, proposal_info, &evidence.context.config)?;
    validate_activation_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &evidence.context.config,
        &evidence.context.profile,
        &proposal,
    )?;
    validate_activation_live_governors(
        registry_info,
        profile_info,
        policy_info,
        council_info,
        gate_info,
        &evidence.context,
        &proposal,
    )?;
    validate_activation_proposal_evidence(
        &proposal,
        &evidence,
        capacity_info,
        observation_info,
        immutability_info,
        handoff_receipt_info,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    if proposal.state != GovernanceLifecycleStateV2::Draft {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    reject_guardian(&evidence.context.config, seat.key)?;
    let seat_index = require_active_seat(&evidence.context.council, seat.key, slot)?;
    let crossed = record_governance_approval_v2(
        &mut proposal.approval_bitset,
        &mut proposal.approval_count,
        seat_index,
        slot,
        proposal.review_start_slot,
        proposal.review_end_slot,
        proposal.expiry_slot,
    )?;
    if proposal.first_approval_slot == 0 {
        proposal.first_approval_slot = slot;
    }
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, BootstrapActivationProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

pub fn process_cancel_bootstrap_activation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelBootstrapActivationProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        CANCEL_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, profile_info, proposal_info, seat] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for account in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        profile_info,
    ] {
        ro(account)?;
    }
    rw(proposal_info)?;
    rs(seat)?;
    validate_seat_authority(seat)?;
    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config, slot)?;
    let council = load_council(
        program_id,
        council_info,
        config_info,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    validate_bootstrap_gate(&gate, &config)?;
    let _registry = load_registry(program_id, registry_info, config_info, &config)?;
    let profile = load_profile(program_id, profile_info, config_info, &config)?;
    let mut proposal = load_activation_proposal(program_id, proposal_info, &config)?;
    validate_activation_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &config,
        &profile,
        &proposal,
    )?;
    if proposal.governance_policy != *policy_info.key
        || proposal.governance_policy_hash != policy.policy_hash
        || proposal.gate != *gate_info.key
        || proposal.bootstrap_gate_status != gate.status
        || proposal.bootstrap_gate_epoch != gate.epoch
        || proposal.bootstrap_freeze_reason_code != gate.freeze_reason_code
        || proposal.bootstrap_freeze_slot != gate.freeze_slot
        || proposal.target_nonce != config.target_nonce
        || proposal.council_version != council.version
        || proposal.council_hash != council.set_hash
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    reject_guardian(&config, seat.key)?;
    let seat_index = require_active_seat(&council, seat.key, slot)?;
    let crossed = record_governance_cancellation_v2(
        proposal.state,
        &mut proposal.cancellation_bitset,
        &mut proposal.cancellation_count,
        seat_index,
        slot,
        proposal.expiry_slot,
    )?;
    require_mask_active(
        &council,
        proposal.cancellation_bitset,
        proposal.cancellation_count,
        slot,
    )?;
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::Cancelled;
        proposal.terminal_slot = slot;
        proposal.terminal_reason_code = instruction.cancellation_reason_code;
    }
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, BootstrapActivationProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

pub fn process_expire_bootstrap_activation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireBootstrapActivationProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        EXPIRE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [config_info, registry_info, profile_info, proposal_info] = accounts else {
        unreachable!("account count checked")
    };
    ro(config_info)?;
    ro(registry_info)?;
    ro(profile_info)?;
    rw(proposal_info)?;
    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let _registry = load_registry(program_id, registry_info, config_info, &config)?;
    let profile = load_profile(program_id, profile_info, config_info, &config)?;
    let mut proposal = load_activation_proposal(program_id, proposal_info, &config)?;
    validate_activation_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &config,
        &profile,
        &proposal,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    require_governance_expirable_v2(proposal.state, slot, proposal.expiry_slot)?;
    proposal.state = GovernanceLifecycleStateV2::Expired;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_EXPIRED_REASON_V2;
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, BootstrapActivationProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

pub fn process_queue_bootstrap_activation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueBootstrapActivationProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        QUEUE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, observation_info, target_program, target_programdata, authority_info, registry_info, profile_info, proposal_info, loader_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_programdata,
        authority_info,
        registry_info,
        profile_info,
    ] {
        ro(account)?;
    }
    executable(target_program)?;
    rw(proposal_info)?;
    executable(loader_info)?;
    let slot = current_slot()?;
    let evidence = load_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    let mut proposal =
        load_activation_proposal(program_id, proposal_info, &evidence.context.config)?;
    validate_activation_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &evidence.context.config,
        &evidence.context.profile,
        &proposal,
    )?;
    validate_activation_live_governors(
        registry_info,
        profile_info,
        policy_info,
        council_info,
        gate_info,
        &evidence.context,
        &proposal,
    )?;
    validate_activation_proposal_evidence(
        &proposal,
        &evidence,
        capacity_info,
        observation_info,
        immutability_info,
        handoff_receipt_info,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    require_governance_queueable_v2(proposal.state, slot, proposal.expiry_slot)?;
    require_exact_quorum(
        &evidence.context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    proposal.state = GovernanceLifecycleStateV2::Timelocked;
    proposal.queued_slot = slot;
    proposal.validate_static()?;
    let bytes = encode_fixed_account(&*proposal, BootstrapActivationProposalV2::LEN)?;
    commit_preencoded(program_id, &[(proposal_info, bytes.as_slice())])
}

/// Activates the bootstrap gate and creates the activation receipt/deployment state atomically.
pub fn process_execute_bootstrap_activation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteBootstrapActivationProposalV2,
) -> ProgramResult {
    exact_account_count(
        accounts,
        EXECUTE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_ACCOUNT_COUNT,
    )?;
    all_distinct(accounts)?;
    let [payer, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, capacity_info, immutability_info, handoff_receipt_info, registry_info, profile_info, proposal_info, observation_info, target_program, target_programdata, authority_info, loader_info, activation_receipt_info, deployment_info, system_program_info, instructions_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    ws(payer)?;
    executable(controller_program)?;
    for account in [
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        registry_info,
        profile_info,
        observation_info,
        target_programdata,
        authority_info,
        instructions_info,
    ] {
        ro(account)?;
    }
    rw(gate_info)?;
    rw(proposal_info)?;
    executable(target_program)?;
    executable(loader_info)?;
    rw(activation_receipt_info)?;
    rw(deployment_info)?;
    system_program_account(system_program_info)?;
    if *instructions_info.key != sysvar_ids::instructions::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let slot = current_slot()?;
    let evidence = load_activation_evidence(
        program_id,
        controller_program,
        controller_programdata,
        config_info,
        policy_info,
        council_info,
        gate_info,
        capacity_info,
        immutability_info,
        handoff_receipt_info,
        observation_info,
        target_program,
        target_programdata,
        authority_info,
        registry_info,
        profile_info,
        loader_info,
        slot,
    )?;
    let mut proposal =
        load_activation_proposal(program_id, proposal_info, &evidence.context.config)?;
    validate_activation_identity_graph(
        program_id,
        config_info,
        registry_info,
        profile_info,
        &evidence.context.config,
        &evidence.context.profile,
        &proposal,
    )?;
    validate_activation_live_governors(
        registry_info,
        profile_info,
        policy_info,
        council_info,
        gate_info,
        &evidence.context,
        &proposal,
    )?;
    validate_activation_proposal_evidence(
        &proposal,
        &evidence,
        capacity_info,
        observation_info,
        immutability_info,
        handoff_receipt_info,
    )?;
    validate_guard(
        &instruction.guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )?;
    if instruction.expected_bridge_observation_digest != evidence.observation.observation_digest
        || instruction.expected_gate_epoch != evidence.context.gate.epoch
        || instruction.expected_target_nonce != evidence.context.config.target_nonce
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    require_governance_executable_v2(
        proposal.state,
        slot,
        proposal.not_before_slot,
        proposal.expiry_slot,
    )?;
    require_exact_quorum(
        &evidence.context.council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    let expected_current_data = instruction.pack()?;
    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &expected_current_data,
        &instruction.envelope,
    )?;

    let (expected_receipt, receipt_bump) = derive_bootstrap_activation_receipt_pda(
        program_id,
        &evidence.context.config.target_program,
    );
    let (expected_deployment, deployment_bump) =
        derive_current_deployment_state_pda(program_id, &evidence.context.config.target_program);
    if *activation_receipt_info.key != expected_receipt
        || *deployment_info.key != expected_deployment
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    let next_epoch = evidence
        .context
        .gate
        .epoch
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let mut next_gate = (*evidence.context.gate).clone();
    next_gate.status = GateStatusV1::Active;
    next_gate.epoch = next_epoch;
    next_gate.active_proposal = Pubkey::default();
    next_gate.freeze_slot = 0;
    next_gate.freeze_reason_code = 0;
    next_gate.last_completed_proposal = *proposal_info.key;
    next_gate.validate_static()?;

    proposal.state = GovernanceLifecycleStateV2::Completed;
    proposal.executed_slot = slot;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_COMPLETED_REASON_V2;
    proposal.validate_static()?;

    let mut deployment = CurrentDeploymentStateV1 {
        discriminator: CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: deployment_bump,
        initialized: true,
        controller_program: *program_id,
        controller_config: *config_info.key,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: evidence.context.capacity.policy_digest,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        controller_authority: evidence.context.config.authority_pda,
        artifact_length: evidence.handoff.artifact_length,
        artifact_sha256: evidence.handoff.artifact_sha256,
        artifact_merkle_root: evidence.handoff.artifact_merkle_root,
        artifact_scheme_id: evidence.handoff.artifact_scheme_id,
        actual_programdata_capacity: evidence.observation.actual_capacity,
        programdata_observation: *observation_info.key,
        observation_generation: evidence.observation.generation,
        observation_root: evidence.observation.final_raw_merkle_root,
        observation_digest: evidence.observation.observation_digest,
        deployed_slot: evidence.observation.deployed_slot,
        installed_authority: evidence.context.config.authority_pda,
        source_commitment: evidence.handoff.bridge_source_commitment,
        build_inputs_commitment: evidence.handoff.bridge_build_inputs_commitment,
        package_commitment: evidence.handoff.bridge_package_commitment,
        release_manifest_commitment: evidence.handoff.bridge_release_manifest_commitment,
        release_commitment: *handoff_receipt_info.key,
        release_commitment_digest: evidence.handoff.receipt_digest,
        activation_receipt: OptionalPubkeyV1::some(*activation_receipt_info.key)?,
        completed_proposal: OptionalPubkeyV1::none(),
        gate_epoch_at_activation: next_epoch,
        deployment_generation: 1,
        deployment_digest: [0; 32],
        last_updated_slot: slot,
        reserved: [0; CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN],
    };
    if compute_bootstrap_activation_deployment_plan_digest_v1(&deployment)?
        != instruction.expected_deployment_plan_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    deployment.deployment_digest = compute_current_deployment_digest_v1(&deployment)?;
    validate_current_deployment_digest_v1(&deployment)?;

    let mut receipt = BootstrapActivationReceiptV1 {
        discriminator: BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: receipt_bump,
        initialized: true,
        proposal: *proposal_info.key,
        proposal_digest: proposal.proposal_digest,
        controller_program: *program_id,
        controller_config: *config_info.key,
        governance_policy: *policy_info.key,
        governance_policy_hash: evidence.context.policy.policy_hash,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: evidence.context.capacity.policy_digest,
        controller_immutability_receipt: *immutability_info.key,
        controller_immutability_digest: evidence.immutable.receipt_digest,
        target_handoff_receipt: *handoff_receipt_info.key,
        target_handoff_digest: evidence.handoff.receipt_digest,
        gate: *gate_info.key,
        target_program: evidence.context.config.target_program,
        target_programdata: evidence.context.config.target_programdata,
        upgradeable_loader: evidence.context.config.upgradeable_loader,
        controller_authority: evidence.context.config.authority_pda,
        bridge_observation: *observation_info.key,
        bridge_observation_generation: evidence.observation.generation,
        bridge_observation_root: evidence.observation.final_raw_merkle_root,
        bridge_observation_digest: evidence.observation.observation_digest,
        bridge_artifact_length: evidence.handoff.artifact_length,
        bridge_artifact_sha256: evidence.handoff.artifact_sha256,
        bridge_artifact_merkle_root: evidence.handoff.artifact_merkle_root,
        bridge_artifact_scheme_id: evidence.handoff.artifact_scheme_id,
        actual_target_capacity: evidence.observation.actual_capacity,
        target_deployed_slot: evidence.observation.deployed_slot,
        previous_gate_status: GateStatusV1::EmergencyFrozen,
        previous_gate_epoch: evidence.context.gate.epoch,
        previous_freeze_reason_code: evidence.context.gate.freeze_reason_code,
        previous_freeze_slot: evidence.context.gate.freeze_slot,
        activated_gate_status: GateStatusV1::Active,
        activated_gate_epoch: next_epoch,
        target_nonce: evidence.context.config.target_nonce,
        council_version: evidence.context.council.version,
        council_hash: evidence.context.council.set_hash,
        current_deployment_state: *deployment_info.key,
        current_deployment_digest: deployment.deployment_digest,
        deployment_generation: deployment.deployment_generation,
        finalized_slot: slot,
        receipt_digest: [0; 32],
        finalized: true,
        reserved: [0; BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN],
    };
    if compute_bootstrap_activation_receipt_plan_digest_v1(&receipt)?
        != instruction.expected_receipt_plan_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    receipt.receipt_digest = compute_bootstrap_activation_receipt_digest_v1(&receipt)?;
    validate_bootstrap_activation_receipt_digest_v1(&receipt)?;

    let gate_bytes = encode_fixed_account(&next_gate, ProtocolGateV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, BootstrapActivationProposalV2::LEN)?;
    let receipt_bytes = encode_fixed_account(&receipt, BootstrapActivationReceiptV1::LEN)?;
    let deployment_bytes = encode_fixed_account(&deployment, CurrentDeploymentStateV1::LEN)?;
    let rent = Rent::get()?;
    let receipt_bump_seed = [receipt_bump];
    let deployment_bump_seed = [deployment_bump];
    create_or_reuse_zero_fixed_pda(
        program_id,
        payer,
        activation_receipt_info,
        system_program_info,
        &rent,
        BootstrapActivationReceiptV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_RECEIPT_SEED,
            evidence.context.config.target_program.as_ref(),
            &receipt_bump_seed,
        ],
    )?;
    create_or_reuse_zero_fixed_pda(
        program_id,
        payer,
        deployment_info,
        system_program_info,
        &rent,
        CurrentDeploymentStateV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            DEPLOYMENT_STATE_SEED,
            evidence.context.config.target_program.as_ref(),
            &deployment_bump_seed,
        ],
    )?;
    commit_preencoded(
        program_id,
        &[
            (gate_info, gate_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
            (activation_receipt_info, receipt_bytes.as_slice()),
            (deployment_info, deployment_bytes.as_slice()),
        ],
    )
}
