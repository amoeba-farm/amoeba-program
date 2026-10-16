//! On-chain processors for the additive governance-liveness V2 surface (tags 82-95).
//!
//! V1 account layouts and processors remain untouched. Every V2 creation path
//! records the runtime `Clock` slot, binds one immutable timing profile, and
//! consumes one monotonic lifecycle-registry proposal id. Terminal proposals
//! are never reused; cancellation and expiry only make a fresh id admissible.

use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
};
use solana_sdk_ids::system_program;

use crate::{
    council::{validate_council_guardian_separation, validate_council_set},
    pda::{
        derive_authority_pda, derive_controller_config_pda, derive_council_pda, derive_gate_pda,
        derive_policy_pda, derive_upgradeable_programdata_address, UPGRADEABLE_LOADER_ID,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, store_fixed_controller_account, validate_exact_privileges,
    },
    release1_governance_v2::{
        compute_council_rotation_digest_v2, compute_governance_timing_profile_hash_v1,
        compute_timing_policy_change_digest_v1, derive_governance_action_proposal_v2,
        derive_governance_lifecycle_registry_v2, derive_governance_timing_profile_v1,
        derive_proposal_timing_v2, nominal_governance_timing_profile_v1,
        record_governance_approval_v2, record_governance_cancellation_v2,
        require_governance_executable_v2, require_governance_expirable_v2,
        require_governance_queueable_v2, ApproveCouncilRotationProposalV2,
        ApproveTimingPolicyChangeProposalV1, CancelCouncilRotationProposalV2,
        CancelTimingPolicyChangeProposalV1, CouncilRotationProposalV2,
        CreateCouncilRotationProposalV2, CreateGovernanceTimingProfileV1,
        CreateTimingPolicyChangeProposalV1, ExecuteCouncilRotationProposalV2,
        ExecuteTimingPolicyChangeProposalV1, ExpireCouncilRotationProposalV2,
        ExpireTimingPolicyChangeProposalV1, GovernanceActionGuardV2, GovernanceActionKindV2,
        GovernanceLifecycleRegistryV2, GovernanceLifecycleStateV2, GovernanceTimingClassV1,
        GovernanceTimingProfileV1, InitializeGovernanceLifecycleRegistryV2,
        QueueCouncilRotationProposalV2, QueueTimingPolicyChangeProposalV1,
        TimingPolicyChangeProposalV1, COUNCIL_ROTATION_PROPOSAL_V2_DISCRIMINATOR,
        COUNCIL_ROTATION_PROPOSAL_V2_RESERVED_LEN, EQUAL_SEAT_THRESHOLD_V2,
        GOVERNANCE_ACTION_PROPOSAL_V2_SEED, GOVERNANCE_LIFECYCLE_REGISTRY_V2_DISCRIMINATOR,
        GOVERNANCE_LIFECYCLE_REGISTRY_V2_RESERVED_LEN, GOVERNANCE_LIFECYCLE_REGISTRY_V2_SEED,
        GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION, GOVERNANCE_TIMING_PROFILE_V1_DISCRIMINATOR,
        GOVERNANCE_TIMING_PROFILE_V1_RESERVED_LEN, GOVERNANCE_TIMING_PROFILE_V1_SEED,
        GOVERNANCE_V2_ACCOUNT_VERSION, GOVERNANCE_V2_SEED_PREFIX, MINIMUM_DELAY_SLOTS_V1,
        MINIMUM_EXECUTION_MARGIN_SLOTS_V1, MINIMUM_REVIEW_SLOTS_V1,
        TIMING_POLICY_CHANGE_PROPOSAL_V1_DISCRIMINATOR,
        TIMING_POLICY_CHANGE_PROPOSAL_V1_RESERVED_LEN,
    },
    release1_loader_accounts::validate_program_programdata_linkage,
    release1_state::BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
    state::{
        ControllerConfigV1, GateStatusV1, GovernanceCouncilSetV1, GovernancePolicyV1,
        ProtocolGateV1,
    },
    GovernanceError,
};

const GOVERNANCE_COMPLETED_REASON_V2: u16 = 1;
const GOVERNANCE_EXPIRED_REASON_V2: u16 = 2;

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

fn validate_readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, false)
}

fn validate_writable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, false, false)
}

fn validate_signer_readonly(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, true, false)
}

fn validate_signer_writable(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, true, true, false)
}

fn validate_system_program(account: &AccountInfo<'_>) -> ProgramResult {
    validate_exact_privileges(account, false, false, true)?;
    if *account.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn load_config(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        account,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    let (expected, bump) = derive_controller_config_pda(program_id, &config.target_program);
    let (programdata, _) = derive_upgradeable_programdata_address(&config.target_program);
    let (authority, _) = derive_authority_pda(program_id, &config.target_program);
    let (gate, _) = derive_gate_pda(program_id, &config.target_program);
    if *account.key != expected
        || config.bump != bump
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || config.target_programdata != programdata
        || config.authority_pda != authority
        || config.gate_pda != gate
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

fn load_policy(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    slot: u64,
) -> Result<Box<GovernancePolicyV1>, ProgramError> {
    let policy = load_fixed_controller_account::<GovernancePolicyV1>(
        program_id,
        account,
        GovernancePolicyV1::LEN,
    )?;
    validate_policy_against_config(&policy, config)?;
    let (expected, bump) = derive_policy_pda(
        program_id,
        &config.target_program,
        config.current_policy_version,
    );
    if *account.key != expected
        || policy.bump != bump
        || policy.controller_config != *config_key
        || policy.activation_slot > slot
    {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(policy)
}

fn load_current_council(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
    slot: u64,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        account,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&council, policy)?;
    validate_council_guardian_separation(&council, &config.guardian)?;
    let (expected, bump) = derive_council_pda(
        program_id,
        &config.target_program,
        config.current_council_version,
    );
    if *account.key != expected
        || council.bump != bump
        || council.controller_config != *config_key
        || council.version != config.current_council_version
        || !council.active_at(slot)
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(council)
}

fn load_gate(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate =
        load_fixed_controller_account::<ProtocolGateV1>(program_id, account, ProtocolGateV1::LEN)?;
    gate.validate_static()?;
    let (expected, bump) = derive_gate_pda(program_id, &config.target_program);
    if *account.key != expected
        || *account.key != config.gate_pda
        || gate.bump != bump
        || gate.controller_config != *config_key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

fn require_active_seat(
    council: &GovernanceCouncilSetV1,
    authority: &Pubkey,
    slot: u64,
) -> Result<usize, ProgramError> {
    council
        .seats
        .iter()
        .position(|seat| seat.seat_authority == *authority && seat.term_covers(slot))
        .ok_or_else(|| GovernanceError::InactiveCouncilSeat.into())
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

fn load_registry(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<GovernanceLifecycleRegistryV2>, ProgramError> {
    let registry = load_fixed_controller_account::<GovernanceLifecycleRegistryV2>(
        program_id,
        account,
        GovernanceLifecycleRegistryV2::LEN,
    )?;
    registry.validate_static()?;
    let (expected, bump) =
        derive_governance_lifecycle_registry_v2(program_id, &config.target_program);
    if *account.key != expected
        || registry.bump != bump
        || registry.controller_program != *program_id
        || registry.controller_config != *config_key
        || registry.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(registry)
}

fn load_timing_profile(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<GovernanceTimingProfileV1>, ProgramError> {
    let profile = load_fixed_controller_account::<GovernanceTimingProfileV1>(
        program_id,
        account,
        GovernanceTimingProfileV1::LEN,
    )?;
    profile.validate_static()?;
    let (expected, bump) = derive_governance_timing_profile_v1(
        program_id,
        &config.target_program,
        profile.profile_version,
    );
    if *account.key != expected
        || profile.bump != bump
        || profile.controller_config != *config_key
        || profile.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(profile)
}

fn require_registry_profile(
    registry: &GovernanceLifecycleRegistryV2,
    profile_key: &Pubkey,
    profile: &GovernanceTimingProfileV1,
) -> ProgramResult {
    if registry.current_timing_profile != *profile_key
        || registry.current_timing_profile_version != profile.profile_version
        || registry.current_timing_profile_hash != profile.profile_hash
    {
        return Err(GovernanceError::InactivePolicy.into());
    }
    Ok(())
}

fn load_candidate_council(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let candidate = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        account,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&candidate, policy)?;
    validate_council_guardian_separation(&candidate, &config.guardian)?;
    let (expected, bump) =
        derive_council_pda(program_id, &config.target_program, candidate.version);
    if *account.key != expected
        || candidate.bump != bump
        || candidate.controller_config != *config_key
        || candidate.target_program != config.target_program
        || candidate.version <= config.current_council_version
        || candidate.version == u64::MAX
        || candidate.deactivation_slot != 0
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(candidate)
}

fn load_historical_council(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
) -> Result<Box<GovernanceCouncilSetV1>, ProgramError> {
    let council = load_fixed_controller_account::<GovernanceCouncilSetV1>(
        program_id,
        account,
        GovernanceCouncilSetV1::LEN,
    )?;
    validate_council_set(&council, policy)?;
    validate_council_guardian_separation(&council, &config.guardian)?;
    let (expected, bump) = derive_council_pda(program_id, &config.target_program, council.version);
    if *account.key != expected
        || council.bump != bump
        || council.controller_config != *config_key
        || council.target_program != config.target_program
        || council.version == u64::MAX
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(council)
}

fn require_candidate_council_active(
    candidate: &GovernanceCouncilSetV1,
    slot: u64,
) -> ProgramResult {
    if !candidate.active_at(slot) || candidate.seats.iter().any(|seat| !seat.term_covers(slot)) {
        return Err(GovernanceError::InactiveCouncilSeat.into());
    }
    Ok(())
}

fn load_timing_policy_proposal(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<TimingPolicyChangeProposalV1>, ProgramError> {
    let proposal = load_fixed_controller_account::<TimingPolicyChangeProposalV1>(
        program_id,
        account,
        TimingPolicyChangeProposalV1::LEN,
    )?;
    proposal.validate_static()?;
    let (expected, bump) = derive_governance_action_proposal_v2(
        program_id,
        &config.target_program,
        GovernanceActionKindV2::TimingPolicyChange,
        proposal.proposal_id,
    );
    if *account.key != expected
        || proposal.bump != bump
        || proposal.controller_config != *config_key
        || proposal.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
}

fn load_rotation_proposal(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    config_key: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<CouncilRotationProposalV2>, ProgramError> {
    let proposal = load_fixed_controller_account::<CouncilRotationProposalV2>(
        program_id,
        account,
        CouncilRotationProposalV2::LEN,
    )?;
    proposal.validate_static()?;
    let (expected, bump) = derive_governance_action_proposal_v2(
        program_id,
        &config.target_program,
        GovernanceActionKindV2::CouncilRotation,
        proposal.proposal_id,
    );
    if *account.key != expected
        || proposal.bump != bump
        || proposal.controller_config != *config_key
        || proposal.target_program != config.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
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

fn validate_profile_predecessor(
    candidate_key: &Pubkey,
    candidate: &GovernanceTimingProfileV1,
    governing_key: &Pubkey,
    governing: &GovernanceTimingProfileV1,
) -> ProgramResult {
    if candidate_key == governing_key
        || candidate.profile_version <= governing.profile_version
        || candidate.predecessor_profile != *governing_key
        || candidate.predecessor_profile_hash != governing.profile_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_timing_policy_graph(
    registry_key: &Pubkey,
    registry: &GovernanceLifecycleRegistryV2,
    council_key: &Pubkey,
    council: &GovernanceCouncilSetV1,
    governing_profile_key: &Pubkey,
    governing_profile: &GovernanceTimingProfileV1,
    candidate_profile_key: &Pubkey,
    candidate_profile: &GovernanceTimingProfileV1,
    proposal: &TimingPolicyChangeProposalV1,
    require_live_governors: bool,
) -> ProgramResult {
    validate_profile_predecessor(
        candidate_profile_key,
        candidate_profile,
        governing_profile_key,
        governing_profile,
    )?;
    if proposal.lifecycle_registry != *registry_key
        || proposal.governing_timing_profile != *governing_profile_key
        || proposal.governing_timing_profile_version != governing_profile.profile_version
        || proposal.governing_timing_profile_hash != governing_profile.profile_hash
        || proposal.candidate_timing_profile != *candidate_profile_key
        || proposal.candidate_timing_profile_version != candidate_profile.profile_version
        || proposal.candidate_timing_profile_hash != candidate_profile.profile_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if require_live_governors {
        require_registry_profile(registry, governing_profile_key, governing_profile)?;
        if proposal.creation_council != *council_key
            || proposal.creation_council_version != council.version
            || proposal.creation_council_hash != council.set_hash
        {
            return Err(GovernanceError::StaleCouncilVersion.into());
        }
    }
    Ok(())
}

fn validate_timing_policy_guard(
    guard: &GovernanceActionGuardV2,
    proposal: &TimingPolicyChangeProposalV1,
) -> ProgramResult {
    validate_guard(
        guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.creation_council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_rotation_graph(
    registry_key: &Pubkey,
    registry: &GovernanceLifecycleRegistryV2,
    current_council_key: &Pubkey,
    current_council: &GovernanceCouncilSetV1,
    candidate_council_key: &Pubkey,
    candidate_council: &GovernanceCouncilSetV1,
    profile_key: &Pubkey,
    profile: &GovernanceTimingProfileV1,
    proposal: &CouncilRotationProposalV2,
    require_live_governors: bool,
) -> ProgramResult {
    if proposal.lifecycle_registry != *registry_key
        || proposal.governing_timing_profile != *profile_key
        || proposal.governing_timing_profile_version != profile.profile_version
        || proposal.governing_timing_profile_hash != profile.profile_hash
        || proposal.candidate_council != *candidate_council_key
        || proposal.candidate_council_version != candidate_council.version
        || proposal.candidate_council_hash != candidate_council.set_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if require_live_governors
        && (proposal.current_council != *current_council_key
            || proposal.current_council_version != current_council.version
            || proposal.current_council_hash != current_council.set_hash
            || proposal.rotation_nonce != registry.rotation_nonce)
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    Ok(())
}

fn validate_rotation_guard(
    guard: &GovernanceActionGuardV2,
    proposal: &CouncilRotationProposalV2,
) -> ProgramResult {
    validate_guard(
        guard,
        proposal.proposal_id,
        &proposal.proposal_digest,
        proposal.current_council_version,
        proposal.governing_timing_profile_version,
        &proposal.governing_timing_profile_hash,
    )
}

struct LoadedRotationContextV2 {
    config: Box<ControllerConfigV1>,
    current_council: Box<GovernanceCouncilSetV1>,
    candidate_council: Box<GovernanceCouncilSetV1>,
    registry: Box<GovernanceLifecycleRegistryV2>,
    proposal: Box<CouncilRotationProposalV2>,
}

#[allow(clippy::too_many_arguments)]
fn load_rotation_context_v2(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    current_council_info: &AccountInfo<'_>,
    candidate_council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    profile_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    slot: u64,
    require_live_governors: bool,
) -> Result<LoadedRotationContextV2, ProgramError> {
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let current_council = load_current_council(
        program_id,
        current_council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let candidate_council = if require_live_governors {
        load_candidate_council(
            program_id,
            candidate_council_info,
            config_info.key,
            &config,
            &policy,
        )?
    } else {
        load_historical_council(
            program_id,
            candidate_council_info,
            config_info.key,
            &config,
            &policy,
        )?
    };
    let _gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let registry = load_registry(program_id, registry_info, config_info.key, &config)?;
    let profile = load_timing_profile(program_id, profile_info, config_info.key, &config)?;
    let proposal = load_rotation_proposal(program_id, proposal_info, config_info.key, &config)?;
    validate_rotation_graph(
        registry_info.key,
        &registry,
        current_council_info.key,
        &current_council,
        candidate_council_info.key,
        &candidate_council,
        profile_info.key,
        &profile,
        &proposal,
        require_live_governors,
    )?;
    Ok(LoadedRotationContextV2 {
        config,
        current_council,
        candidate_council,
        registry,
        proposal,
    })
}

struct LoadedTimingPolicyContext {
    config: Box<ControllerConfigV1>,
    council: Box<GovernanceCouncilSetV1>,
    registry: Box<GovernanceLifecycleRegistryV2>,
    proposal: Box<TimingPolicyChangeProposalV1>,
}

#[allow(clippy::too_many_arguments)]
fn load_timing_policy_context(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    registry_info: &AccountInfo<'_>,
    governing_profile_info: &AccountInfo<'_>,
    candidate_profile_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    slot: u64,
    require_live_governors: bool,
) -> Result<LoadedTimingPolicyContext, ProgramError> {
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let _gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let registry = load_registry(program_id, registry_info, config_info.key, &config)?;
    let governing_profile =
        load_timing_profile(program_id, governing_profile_info, config_info.key, &config)?;
    let candidate_profile =
        load_timing_profile(program_id, candidate_profile_info, config_info.key, &config)?;
    let proposal =
        load_timing_policy_proposal(program_id, proposal_info, config_info.key, &config)?;
    validate_timing_policy_graph(
        registry_info.key,
        &registry,
        council_info.key,
        &council,
        governing_profile_info.key,
        &governing_profile,
        candidate_profile_info.key,
        &candidate_profile,
        &proposal,
        require_live_governors,
    )?;
    Ok(LoadedTimingPolicyContext {
        config,
        council,
        registry,
        proposal,
    })
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
    for (account, bytes) in values {
        account.try_borrow_mut_data()?.copy_from_slice(bytes);
    }
    Ok(())
}

fn create_registry_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    target_program: &Pubkey,
    bump: u8,
) -> ProgramResult {
    let bump_seed = [bump];
    create_fixed_pda_account(
        program_id,
        payer,
        account,
        system_program_info,
        &Rent::get()?,
        GovernanceLifecycleRegistryV2::LEN,
        &[
            GOVERNANCE_V2_SEED_PREFIX,
            GOVERNANCE_LIFECYCLE_REGISTRY_V2_SEED,
            target_program.as_ref(),
            &bump_seed,
        ],
    )
}

fn create_timing_profile_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    target_program: &Pubkey,
    profile_version: u64,
    bump: u8,
) -> ProgramResult {
    let version_seed = profile_version.to_le_bytes();
    let bump_seed = [bump];
    create_fixed_pda_account(
        program_id,
        payer,
        account,
        system_program_info,
        &Rent::get()?,
        GovernanceTimingProfileV1::LEN,
        &[
            GOVERNANCE_V2_SEED_PREFIX,
            GOVERNANCE_TIMING_PROFILE_V1_SEED,
            target_program.as_ref(),
            &version_seed,
            &bump_seed,
        ],
    )
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

/// Initializes the additive V2 lifecycle registry and the immutable nominal
/// timing profile. The controller's current ProgramData upgrade authority is
/// the sole bootstrap capability; once that authority is removed this path is
/// mechanically unreachable.
pub fn process_initialize_governance_lifecycle_registry_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: InitializeGovernanceLifecycleRegistryV2,
) -> ProgramResult {
    exact_account_count(accounts, 11)?;
    all_distinct(accounts)?;
    let [payer, initializer, controller_program, controller_programdata, config_info, policy_info, council_info, gate_info, registry_info, initial_profile_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };

    validate_signer_writable(payer)?;
    validate_signer_readonly(initializer)?;
    validate_exact_privileges(controller_program, false, false, true)?;
    validate_readonly(controller_programdata)?;
    for readonly in [config_info, policy_info, council_info, gate_info] {
        validate_readonly(readonly)?;
    }
    validate_writable(registry_info)?;
    validate_writable(initial_profile_info)?;
    validate_system_program(system_program_info)?;
    if *controller_program.key != *program_id {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let programdata = validate_program_programdata_linkage(
        controller_program,
        controller_programdata,
        &UPGRADEABLE_LOADER_ID,
    )?;
    if programdata.upgrade_authority != Some(*initializer.key) {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    if gate.status != GateStatusV1::EmergencyFrozen
        || gate.epoch != 1
        || gate.active_proposal != Pubkey::default()
        || gate.freeze_slot == 0
        || gate.freeze_slot > slot
        || gate.freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || gate.last_completed_proposal != Pubkey::default()
    {
        return Err(GovernanceError::InvalidGateState.into());
    }

    let (registry_pda, registry_bump) =
        derive_governance_lifecycle_registry_v2(program_id, &config.target_program);
    let (profile_pda, profile_bump) =
        derive_governance_timing_profile_v1(program_id, &config.target_program, 1);
    if *registry_info.key != registry_pda
        || *initial_profile_info.key != profile_pda
        || instruction.expected_initial_timing_profile_version != 1
        || instruction.expected_initial_next_proposal_id != config.next_proposal_id
        || instruction.expected_initial_next_proposal_id == u64::MAX
        || instruction.expected_initial_rotation_nonce != 1
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let profile = nominal_governance_timing_profile_v1(
        profile_bump,
        *config_info.key,
        config.target_program,
        council.version,
        slot,
    )?;
    if instruction.expected_initial_timing_profile_hash != profile.profile_hash {
        return Err(GovernanceError::PolicyHashMismatch.into());
    }
    let registry = GovernanceLifecycleRegistryV2 {
        discriminator: GOVERNANCE_LIFECYCLE_REGISTRY_V2_DISCRIMINATOR,
        version: GOVERNANCE_V2_ACCOUNT_VERSION,
        bump: registry_bump,
        initialized: true,
        controller_program: *program_id,
        controller_config: *config_info.key,
        target_program: config.target_program,
        current_timing_profile: *initial_profile_info.key,
        current_timing_profile_version: profile.profile_version,
        current_timing_profile_hash: profile.profile_hash,
        next_proposal_id: instruction.expected_initial_next_proposal_id,
        next_timing_profile_version: 2,
        rotation_nonce: instruction.expected_initial_rotation_nonce,
        last_policy_change_proposal: Pubkey::default(),
        creation_slot: slot,
        reserved: [0; GOVERNANCE_LIFECYCLE_REGISTRY_V2_RESERVED_LEN],
    };
    registry.validate_static()?;

    let registry_bytes = encode_fixed_account(&registry, GovernanceLifecycleRegistryV2::LEN)?;
    let profile_bytes = encode_fixed_account(&profile, GovernanceTimingProfileV1::LEN)?;
    create_registry_account(
        program_id,
        payer,
        registry_info,
        system_program_info,
        &config.target_program,
        registry_bump,
    )?;
    create_timing_profile_account(
        program_id,
        payer,
        initial_profile_info,
        system_program_info,
        &config.target_program,
        profile.profile_version,
        profile_bump,
    )?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (initial_profile_info, profile_bytes.as_slice()),
        ],
    )
}

/// Creates a finalized immutable candidate profile. Creation consumes the next
/// profile version even though activation remains a separately governed act.
pub fn process_create_governance_timing_profile_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateGovernanceTimingProfileV1,
) -> ProgramResult {
    exact_account_count(accounts, 10)?;
    all_distinct(accounts)?;
    let [payer, creator, config_info, policy_info, council_info, gate_info, registry_info, current_profile_info, candidate_profile_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };

    validate_signer_writable(payer)?;
    validate_signer_readonly(creator)?;
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        current_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(registry_info)?;
    validate_writable(candidate_profile_info)?;
    validate_system_program(system_program_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    reject_guardian(&config, creator.key)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    require_active_seat(&council, creator.key, slot)?;
    let _gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let mut registry = load_registry(program_id, registry_info, config_info.key, &config)?;
    let current_profile =
        load_timing_profile(program_id, current_profile_info, config_info.key, &config)?;
    require_registry_profile(&registry, current_profile_info.key, &current_profile)?;
    if instruction.predecessor_profile_hash != current_profile.profile_hash {
        return Err(GovernanceError::PolicyHashMismatch.into());
    }
    registry.consume_timing_profile_version(instruction.profile_version)?;

    let (candidate_pda, candidate_bump) = derive_governance_timing_profile_v1(
        program_id,
        &config.target_program,
        instruction.profile_version,
    );
    if *candidate_profile_info.key != candidate_pda {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut candidate = GovernanceTimingProfileV1 {
        discriminator: GOVERNANCE_TIMING_PROFILE_V1_DISCRIMINATOR,
        version: GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION,
        bump: candidate_bump,
        initialized: true,
        controller_config: *config_info.key,
        target_program: config.target_program,
        profile_version: instruction.profile_version,
        predecessor_profile: *current_profile_info.key,
        predecessor_profile_hash: current_profile.profile_hash,
        creation_council_version: council.version,
        emergency_rollback: instruction.emergency_rollback,
        routine: instruction.routine,
        major: instruction.major,
        constitutional: instruction.constitutional,
        hard_minimum_review_slots: MINIMUM_REVIEW_SLOTS_V1,
        hard_minimum_delay_slots: MINIMUM_DELAY_SLOTS_V1,
        hard_minimum_execution_margin_slots: MINIMUM_EXECUTION_MARGIN_SLOTS_V1,
        profile_hash: [0; 32],
        creation_slot: slot,
        finalized: true,
        reserved: [0; GOVERNANCE_TIMING_PROFILE_V1_RESERVED_LEN],
    };
    candidate.profile_hash = compute_governance_timing_profile_hash_v1(&candidate)?;
    candidate.validate_static()?;

    let registry_bytes = encode_fixed_account(&*registry, GovernanceLifecycleRegistryV2::LEN)?;
    let candidate_bytes = encode_fixed_account(&candidate, GovernanceTimingProfileV1::LEN)?;
    create_timing_profile_account(
        program_id,
        payer,
        candidate_profile_info,
        system_program_info,
        &config.target_program,
        candidate.profile_version,
        candidate_bump,
    )?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (candidate_profile_info, candidate_bytes.as_slice()),
        ],
    )
}

pub fn process_create_timing_policy_change_proposal_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateTimingPolicyChangeProposalV1,
) -> ProgramResult {
    exact_account_count(accounts, 11)?;
    all_distinct(accounts)?;
    let [payer, creator, config_info, policy_info, council_info, gate_info, registry_info, governing_profile_info, candidate_profile_info, proposal_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };

    validate_signer_writable(payer)?;
    validate_signer_readonly(creator)?;
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        governing_profile_info,
        candidate_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(registry_info)?;
    validate_writable(proposal_info)?;
    validate_system_program(system_program_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    reject_guardian(&config, creator.key)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let council = load_current_council(
        program_id,
        council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    require_active_seat(&council, creator.key, slot)?;
    let _gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let mut registry = load_registry(program_id, registry_info, config_info.key, &config)?;
    let governing_profile =
        load_timing_profile(program_id, governing_profile_info, config_info.key, &config)?;
    let candidate_profile =
        load_timing_profile(program_id, candidate_profile_info, config_info.key, &config)?;
    require_registry_profile(&registry, governing_profile_info.key, &governing_profile)?;
    validate_profile_predecessor(
        candidate_profile_info.key,
        &candidate_profile,
        governing_profile_info.key,
        &governing_profile,
    )?;
    if instruction.expected_current_timing_profile_version != governing_profile.profile_version
        || instruction.expected_current_timing_profile_hash != governing_profile.profile_hash
        || instruction.candidate_timing_profile_version != candidate_profile.profile_version
        || instruction.candidate_timing_profile_hash != candidate_profile.profile_hash
        || instruction.expected_council_version != council.version
        || instruction.expected_council_hash != council.set_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let proposal_id = registry.consume_proposal_id(instruction.expected_proposal_id)?;
    let (proposal_pda, proposal_bump) = derive_governance_action_proposal_v2(
        program_id,
        &config.target_program,
        GovernanceActionKindV2::TimingPolicyChange,
        proposal_id,
    );
    if *proposal_info.key != proposal_pda {
        return Err(GovernanceError::InvalidPda.into());
    }
    let timing =
        derive_proposal_timing_v2(&governing_profile, GovernanceTimingClassV1::Major, slot)?;
    let mut proposal = TimingPolicyChangeProposalV1 {
        discriminator: TIMING_POLICY_CHANGE_PROPOSAL_V1_DISCRIMINATOR,
        version: GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION,
        bump: proposal_bump,
        initialized: true,
        state: GovernanceLifecycleStateV2::Draft,
        proposal_id,
        controller_config: *config_info.key,
        target_program: config.target_program,
        lifecycle_registry: *registry_info.key,
        creation_council: *council_info.key,
        creation_council_version: council.version,
        creation_council_hash: council.set_hash,
        governing_timing_profile: *governing_profile_info.key,
        governing_timing_profile_version: governing_profile.profile_version,
        governing_timing_profile_hash: governing_profile.profile_hash,
        candidate_timing_profile: *candidate_profile_info.key,
        candidate_timing_profile_version: candidate_profile.profile_version,
        candidate_timing_profile_hash: candidate_profile.profile_hash,
        review_duration_slots: timing.review_duration_slots,
        delay_duration_slots: timing.delay_duration_slots,
        expiry_duration_slots: timing.expiry_duration_slots,
        creation_slot: slot,
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
        reserved: [0; TIMING_POLICY_CHANGE_PROPOSAL_V1_RESERVED_LEN],
    };
    proposal.proposal_digest = compute_timing_policy_change_digest_v1(&proposal)?;
    proposal.validate_static()?;

    let registry_bytes = encode_fixed_account(&*registry, GovernanceLifecycleRegistryV2::LEN)?;
    let proposal_bytes = encode_fixed_account(&proposal, TimingPolicyChangeProposalV1::LEN)?;
    create_proposal_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &config.target_program,
        GovernanceActionKindV2::TimingPolicyChange,
        proposal_id,
        proposal_bump,
        TimingPolicyChangeProposalV1::LEN,
    )?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
        ],
    )
}

pub fn process_approve_timing_policy_change_proposal_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveTimingPolicyChangeProposalV1,
) -> ProgramResult {
    exact_account_count(accounts, 9)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, governing_profile_info, candidate_profile_info, proposal_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;
    validate_signer_readonly(seat_authority)?;

    let slot = current_slot()?;
    let LoadedTimingPolicyContext {
        config,
        council,
        mut proposal,
        ..
    } = load_timing_policy_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
        proposal_info,
        slot,
        true,
    )?;
    reject_guardian(&config, seat_authority.key)?;
    validate_timing_policy_guard(&instruction.guard, &proposal)?;
    if proposal.state != GovernanceLifecycleStateV2::Draft {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_mask_active(
        &council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    let seat_index = u8::try_from(require_active_seat(&council, seat_authority.key, slot)?)
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let first_approval = proposal.approval_count == 0;
    let crossed = record_governance_approval_v2(
        &mut proposal.approval_bitset,
        &mut proposal.approval_count,
        seat_index,
        slot,
        proposal.review_start_slot,
        proposal.review_end_slot,
        proposal.expiry_slot,
    )?;
    if first_approval {
        proposal.first_approval_slot = slot;
    }
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    require_mask_active(
        &council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        TimingPolicyChangeProposalV1::LEN,
    )
}

pub fn process_cancel_timing_policy_change_proposal_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelTimingPolicyChangeProposalV1,
) -> ProgramResult {
    exact_account_count(accounts, 9)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, governing_profile_info, candidate_profile_info, proposal_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;
    validate_signer_readonly(seat_authority)?;
    if instruction.cancellation_reason_code == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let slot = current_slot()?;
    let LoadedTimingPolicyContext {
        config,
        council,
        mut proposal,
        ..
    } = load_timing_policy_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
        proposal_info,
        slot,
        true,
    )?;
    reject_guardian(&config, seat_authority.key)?;
    validate_timing_policy_guard(&instruction.guard, &proposal)?;
    require_mask_active(
        &council,
        proposal.cancellation_bitset,
        proposal.cancellation_count,
        slot,
    )?;
    let seat_index = u8::try_from(require_active_seat(&council, seat_authority.key, slot)?)
        .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let crossed = record_governance_cancellation_v2(
        proposal.state,
        &mut proposal.cancellation_bitset,
        &mut proposal.cancellation_count,
        seat_index,
        slot,
        proposal.expiry_slot,
    )?;
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::Cancelled;
        proposal.terminal_slot = slot;
        proposal.terminal_reason_code = instruction.cancellation_reason_code;
    }
    require_mask_active(
        &council,
        proposal.cancellation_bitset,
        proposal.cancellation_count,
        slot,
    )?;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        TimingPolicyChangeProposalV1::LEN,
    )
}

pub fn process_expire_timing_policy_change_proposal_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireTimingPolicyChangeProposalV1,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, governing_profile_info, candidate_profile_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;

    let slot = current_slot()?;
    let LoadedTimingPolicyContext { mut proposal, .. } = load_timing_policy_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
        proposal_info,
        slot,
        false,
    )?;
    validate_timing_policy_guard(&instruction.guard, &proposal)?;
    require_governance_expirable_v2(proposal.state, slot, proposal.expiry_slot)?;
    proposal.state = GovernanceLifecycleStateV2::Expired;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_EXPIRED_REASON_V2;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        TimingPolicyChangeProposalV1::LEN,
    )
}

pub fn process_queue_timing_policy_change_proposal_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueTimingPolicyChangeProposalV1,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, governing_profile_info, candidate_profile_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;

    let slot = current_slot()?;
    let LoadedTimingPolicyContext {
        council,
        mut proposal,
        ..
    } = load_timing_policy_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
        proposal_info,
        slot,
        true,
    )?;
    validate_timing_policy_guard(&instruction.guard, &proposal)?;
    require_governance_queueable_v2(proposal.state, slot, proposal.expiry_slot)?;
    require_mask_active(
        &council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    if proposal.approval_count < EQUAL_SEAT_THRESHOLD_V2 {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    proposal.state = GovernanceLifecycleStateV2::Timelocked;
    proposal.queued_slot = slot;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        TimingPolicyChangeProposalV1::LEN,
    )
}

pub fn process_execute_timing_policy_change_proposal_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteTimingPolicyChangeProposalV1,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, council_info, gate_info, registry_info, governing_profile_info, candidate_profile_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        council_info,
        gate_info,
        governing_profile_info,
        candidate_profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(registry_info)?;
    validate_writable(proposal_info)?;

    let slot = current_slot()?;
    let LoadedTimingPolicyContext {
        council,
        mut registry,
        mut proposal,
        ..
    } = load_timing_policy_context(
        program_id,
        config_info,
        policy_info,
        council_info,
        gate_info,
        registry_info,
        governing_profile_info,
        candidate_profile_info,
        proposal_info,
        slot,
        true,
    )?;
    validate_timing_policy_guard(&instruction.guard, &proposal)?;
    require_governance_executable_v2(
        proposal.state,
        slot,
        proposal.not_before_slot,
        proposal.expiry_slot,
    )?;
    require_mask_active(
        &council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    if proposal.approval_count < EQUAL_SEAT_THRESHOLD_V2 {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }

    registry.current_timing_profile = proposal.candidate_timing_profile;
    registry.current_timing_profile_version = proposal.candidate_timing_profile_version;
    registry.current_timing_profile_hash = proposal.candidate_timing_profile_hash;
    registry.last_policy_change_proposal = *proposal_info.key;
    registry.validate_static()?;
    proposal.state = GovernanceLifecycleStateV2::Completed;
    proposal.executed_slot = slot;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_COMPLETED_REASON_V2;
    proposal.validate_static()?;

    let registry_bytes = encode_fixed_account(&*registry, GovernanceLifecycleRegistryV2::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, TimingPolicyChangeProposalV1::LEN)?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
        ],
    )
}

pub fn process_create_council_rotation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateCouncilRotationProposalV2,
) -> ProgramResult {
    exact_account_count(accounts, 11)?;
    all_distinct(accounts)?;
    let [payer, creator, config_info, policy_info, current_council_info, candidate_council_info, gate_info, registry_info, profile_info, proposal_info, system_program_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    validate_signer_writable(payer)?;
    validate_signer_readonly(creator)?;
    for readonly in [
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(registry_info)?;
    validate_writable(proposal_info)?;
    validate_system_program(system_program_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    reject_guardian(&config, creator.key)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let current_council = load_current_council(
        program_id,
        current_council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    require_active_seat(&current_council, creator.key, slot)?;
    let candidate_council = load_candidate_council(
        program_id,
        candidate_council_info,
        config_info.key,
        &config,
        &policy,
    )?;
    let _gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let mut registry = load_registry(program_id, registry_info, config_info.key, &config)?;
    let profile = load_timing_profile(program_id, profile_info, config_info.key, &config)?;
    require_registry_profile(&registry, profile_info.key, &profile)?;
    if instruction.expected_current_council_version != current_council.version
        || instruction.expected_current_council_hash != current_council.set_hash
        || instruction.candidate_council_version != candidate_council.version
        || instruction.candidate_council_hash != candidate_council.set_hash
        || instruction.expected_rotation_nonce != registry.rotation_nonce
        || instruction.expected_rotation_nonce == u64::MAX
        || instruction.expected_timing_profile_version != profile.profile_version
        || instruction.expected_timing_profile_hash != profile.profile_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let proposal_id = registry.consume_proposal_id(instruction.expected_proposal_id)?;
    let timing = derive_proposal_timing_v2(&profile, GovernanceTimingClassV1::Major, slot)?;
    let earliest_activation = candidate_council
        .activation_slot
        .max(timing.not_before_slot);
    let last_executable_slot = timing
        .expiry_slot
        .checked_sub(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if earliest_activation >= timing.expiry_slot
        || !candidate_council.active_at(earliest_activation)
        || candidate_council.seats.iter().any(|seat| {
            !seat.term_covers(earliest_activation) || !seat.term_covers(last_executable_slot)
        })
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let (proposal_pda, proposal_bump) = derive_governance_action_proposal_v2(
        program_id,
        &config.target_program,
        GovernanceActionKindV2::CouncilRotation,
        proposal_id,
    );
    if *proposal_info.key != proposal_pda {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut proposal = CouncilRotationProposalV2 {
        discriminator: COUNCIL_ROTATION_PROPOSAL_V2_DISCRIMINATOR,
        version: GOVERNANCE_V2_ACCOUNT_VERSION,
        bump: proposal_bump,
        initialized: true,
        state: GovernanceLifecycleStateV2::Draft,
        proposal_id,
        controller_config: *config_info.key,
        target_program: config.target_program,
        lifecycle_registry: *registry_info.key,
        governing_timing_profile: *profile_info.key,
        governing_timing_profile_version: profile.profile_version,
        governing_timing_profile_hash: profile.profile_hash,
        current_council: *current_council_info.key,
        current_council_version: current_council.version,
        current_council_hash: current_council.set_hash,
        candidate_council: *candidate_council_info.key,
        candidate_council_version: candidate_council.version,
        candidate_council_hash: candidate_council.set_hash,
        rotation_nonce: registry.rotation_nonce,
        review_duration_slots: timing.review_duration_slots,
        delay_duration_slots: timing.delay_duration_slots,
        expiry_duration_slots: timing.expiry_duration_slots,
        creation_slot: slot,
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
        reserved: [0; COUNCIL_ROTATION_PROPOSAL_V2_RESERVED_LEN],
    };
    proposal.proposal_digest = compute_council_rotation_digest_v2(&proposal)?;
    proposal.validate_static()?;

    let registry_bytes = encode_fixed_account(&*registry, GovernanceLifecycleRegistryV2::LEN)?;
    let proposal_bytes = encode_fixed_account(&proposal, CouncilRotationProposalV2::LEN)?;
    create_proposal_account(
        program_id,
        payer,
        proposal_info,
        system_program_info,
        &config.target_program,
        GovernanceActionKindV2::CouncilRotation,
        proposal_id,
        proposal_bump,
        CouncilRotationProposalV2::LEN,
    )?;
    commit_preencoded(
        program_id,
        &[
            (registry_info, registry_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
        ],
    )
}

pub fn process_approve_council_rotation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ApproveCouncilRotationProposalV2,
) -> ProgramResult {
    exact_account_count(accounts, 9)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_council_info, gate_info, registry_info, profile_info, proposal_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;
    validate_signer_readonly(seat_authority)?;

    let slot = current_slot()?;
    let LoadedRotationContextV2 {
        config,
        current_council,
        mut proposal,
        ..
    } = load_rotation_context_v2(
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
        proposal_info,
        slot,
        true,
    )?;
    reject_guardian(&config, seat_authority.key)?;
    validate_rotation_guard(&instruction.guard, &proposal)?;
    if proposal.state != GovernanceLifecycleStateV2::Draft {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    require_mask_active(
        &current_council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    let seat_index = u8::try_from(require_active_seat(
        &current_council,
        seat_authority.key,
        slot,
    )?)
    .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let first_approval = proposal.approval_count == 0;
    let crossed = record_governance_approval_v2(
        &mut proposal.approval_bitset,
        &mut proposal.approval_count,
        seat_index,
        slot,
        proposal.review_start_slot,
        proposal.review_end_slot,
        proposal.expiry_slot,
    )?;
    if first_approval {
        proposal.first_approval_slot = slot;
    }
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::CouncilApproved;
        proposal.council_approved_slot = slot;
    }
    require_mask_active(
        &current_council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        CouncilRotationProposalV2::LEN,
    )
}

pub fn process_cancel_council_rotation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CancelCouncilRotationProposalV2,
) -> ProgramResult {
    exact_account_count(accounts, 9)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_council_info, gate_info, registry_info, profile_info, proposal_info, seat_authority] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;
    validate_signer_readonly(seat_authority)?;
    if instruction.cancellation_reason_code == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let slot = current_slot()?;
    let LoadedRotationContextV2 {
        config,
        current_council,
        mut proposal,
        ..
    } = load_rotation_context_v2(
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
        proposal_info,
        slot,
        true,
    )?;
    reject_guardian(&config, seat_authority.key)?;
    validate_rotation_guard(&instruction.guard, &proposal)?;
    require_mask_active(
        &current_council,
        proposal.cancellation_bitset,
        proposal.cancellation_count,
        slot,
    )?;
    let seat_index = u8::try_from(require_active_seat(
        &current_council,
        seat_authority.key,
        slot,
    )?)
    .map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let crossed = record_governance_cancellation_v2(
        proposal.state,
        &mut proposal.cancellation_bitset,
        &mut proposal.cancellation_count,
        seat_index,
        slot,
        proposal.expiry_slot,
    )?;
    if crossed {
        proposal.state = GovernanceLifecycleStateV2::Cancelled;
        proposal.terminal_slot = slot;
        proposal.terminal_reason_code = instruction.cancellation_reason_code;
    }
    require_mask_active(
        &current_council,
        proposal.cancellation_bitset,
        proposal.cancellation_count,
        slot,
    )?;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        CouncilRotationProposalV2::LEN,
    )
}

pub fn process_expire_council_rotation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExpireCouncilRotationProposalV2,
) -> ProgramResult {
    // Candidate council is deliberately not required: a competing rotation may
    // already have made it current, and expiry must remain permissionless.
    exact_account_count(accounts, 7)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, gate_info, registry_info, profile_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        current_council_info,
        gate_info,
        registry_info,
        profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let _current_council = load_current_council(
        program_id,
        current_council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let _gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    let registry = load_registry(program_id, registry_info, config_info.key, &config)?;
    let profile = load_timing_profile(program_id, profile_info, config_info.key, &config)?;
    let mut proposal = load_rotation_proposal(program_id, proposal_info, config_info.key, &config)?;
    if proposal.lifecycle_registry != *registry_info.key
        || proposal.governing_timing_profile != *profile_info.key
        || proposal.governing_timing_profile_version != profile.profile_version
        || proposal.governing_timing_profile_hash != profile.profile_hash
        || registry.target_program != proposal.target_program
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_rotation_guard(&instruction.guard, &proposal)?;
    require_governance_expirable_v2(proposal.state, slot, proposal.expiry_slot)?;
    proposal.state = GovernanceLifecycleStateV2::Expired;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_EXPIRED_REASON_V2;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        CouncilRotationProposalV2::LEN,
    )
}

pub fn process_queue_council_rotation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: QueueCouncilRotationProposalV2,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_council_info, gate_info, registry_info, profile_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    for readonly in [
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(proposal_info)?;

    let slot = current_slot()?;
    let LoadedRotationContextV2 {
        current_council,
        mut proposal,
        ..
    } = load_rotation_context_v2(
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
        proposal_info,
        slot,
        true,
    )?;
    validate_rotation_guard(&instruction.guard, &proposal)?;
    require_governance_queueable_v2(proposal.state, slot, proposal.expiry_slot)?;
    require_mask_active(
        &current_council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    if proposal.approval_count < EQUAL_SEAT_THRESHOLD_V2 {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    proposal.state = GovernanceLifecycleStateV2::Timelocked;
    proposal.queued_slot = slot;
    proposal.validate_static()?;
    store_fixed_controller_account(
        program_id,
        proposal_info,
        &*proposal,
        CouncilRotationProposalV2::LEN,
    )
}

pub fn process_execute_council_rotation_proposal_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteCouncilRotationProposalV2,
) -> ProgramResult {
    exact_account_count(accounts, 8)?;
    all_distinct(accounts)?;
    let [config_info, policy_info, current_council_info, candidate_council_info, gate_info, registry_info, profile_info, proposal_info] =
        accounts
    else {
        unreachable!("account count checked")
    };
    validate_writable(config_info)?;
    for readonly in [
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        profile_info,
    ] {
        validate_readonly(readonly)?;
    }
    validate_writable(registry_info)?;
    validate_writable(proposal_info)?;

    let slot = current_slot()?;
    let LoadedRotationContextV2 {
        mut config,
        current_council,
        candidate_council,
        mut registry,
        mut proposal,
    } = load_rotation_context_v2(
        program_id,
        config_info,
        policy_info,
        current_council_info,
        candidate_council_info,
        gate_info,
        registry_info,
        profile_info,
        proposal_info,
        slot,
        true,
    )?;
    validate_rotation_guard(&instruction.guard, &proposal)?;
    require_governance_executable_v2(
        proposal.state,
        slot,
        proposal.not_before_slot,
        proposal.expiry_slot,
    )?;
    require_mask_active(
        &current_council,
        proposal.approval_bitset,
        proposal.approval_count,
        slot,
    )?;
    if proposal.approval_count < EQUAL_SEAT_THRESHOLD_V2 {
        return Err(GovernanceError::QuorumNotSatisfied.into());
    }
    require_candidate_council_active(&candidate_council, slot)?;
    if registry.rotation_nonce != proposal.rotation_nonce {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }

    config.current_council_version = candidate_council.version;
    config.validate_static()?;
    registry.rotation_nonce = registry
        .rotation_nonce
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    registry.validate_static()?;
    proposal.state = GovernanceLifecycleStateV2::Completed;
    proposal.executed_slot = slot;
    proposal.terminal_slot = slot;
    proposal.terminal_reason_code = GOVERNANCE_COMPLETED_REASON_V2;
    proposal.validate_static()?;

    let config_bytes = encode_fixed_account(&*config, ControllerConfigV1::LEN)?;
    let registry_bytes = encode_fixed_account(&*registry, GovernanceLifecycleRegistryV2::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, CouncilRotationProposalV2::LEN)?;
    commit_preencoded(
        program_id,
        &[
            (config_info, config_bytes.as_slice()),
            (registry_info, registry_bytes.as_slice()),
            (proposal_info, proposal_bytes.as_slice()),
        ],
    )
}
