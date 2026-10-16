use super::*;

pub(super) fn load_config(
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
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(policy)
}

fn load_council(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    policy: &GovernancePolicyV1,
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

pub(super) fn load_capacity_policy(
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
    crate::release1_ceremony_digest::validate_capacity_policy_digest_v1(&capacity)?;
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

pub(super) fn load_ceremony_context(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    council_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    slot: u64,
) -> Result<CeremonyContext, ProgramError> {
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    if slot == 0 || slot < policy.activation_slot {
        return Err(GovernanceError::InactivePolicy.into());
    }
    let council = load_council(program_id, council_info, config_info, &config, &policy)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    Ok(CeremonyContext {
        config,
        policy,
        council,
        gate,
        capacity,
    })
}

pub(super) fn load_controller_release(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    capacity: &ProgramDataCapacityPolicyV1,
    controller_programdata: &Pubkey,
    config: &ControllerConfigV1,
) -> Result<Box<ControllerReleaseCommitmentV1>, ProgramError> {
    let release = load_fixed_controller_account::<ControllerReleaseCommitmentV1>(
        program_id,
        info,
        ControllerReleaseCommitmentV1::LEN,
    )?;
    validate_controller_release_digest_v1(&release)?;
    let expected = derive_controller_release_commitment_pda(program_id, &config.target_program);
    if expected.0 != *info.key
        || expected.1 != release.bump
        || release.controller_program != *program_id
        || release.controller_programdata != *controller_programdata
        || release.upgradeable_loader != config.upgradeable_loader
        || release.capacity_policy != *capacity_info.key
        || release.capacity_policy_digest != capacity.policy_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(release)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_observation(
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
    let expected_subject_digest = compute_programdata_observation_subject_digest_v1(
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
        &expected_subject_digest,
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
        || observation.subject_digest != expected_subject_digest
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

pub(super) fn load_immutability_receipt(
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

#[allow(clippy::too_many_arguments)]
pub(super) fn load_handoff_proposal(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    context: &CeremonyContext,
) -> Result<Box<TargetAuthorityHandoffProposalV1>, ProgramError> {
    let proposal = load_fixed_controller_account::<TargetAuthorityHandoffProposalV1>(
        program_id,
        info,
        TargetAuthorityHandoffProposalV1::LEN,
    )?;
    validate_target_handoff_proposal_digest_v1(&proposal)?;
    let expected = derive_target_authority_handoff_pda(
        program_id,
        &context.config.target_program,
        context.council.version,
    );
    if expected.0 != *info.key
        || expected.1 != proposal.bump
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_info.key
        || proposal.governance_policy != *policy_info.key
        || proposal.governance_policy_hash != context.policy.policy_hash
        || proposal.capacity_policy != *capacity_info.key
        || proposal.capacity_policy_digest != context.capacity.policy_digest
        || proposal.gate != *gate_info.key
        || proposal.controller_immutability_receipt != *immutability_info.key
        || proposal.cluster_domain != context.config.cluster_domain
        || proposal.controller_authority != context.config.authority_pda
        || proposal.target_program != context.config.target_program
        || proposal.target_programdata != context.config.target_programdata
        || proposal.upgradeable_loader != context.config.upgradeable_loader
        || proposal.council_version != context.council.version
        || proposal.council_hash != context.council.set_hash
        || proposal.bootstrap_gate_epoch != context.gate.epoch
        || proposal.target_nonce != context.config.target_nonce
        || derive_major_timing(&context.config, proposal.creation_slot)?
            != (
                proposal.review_start_slot,
                proposal.review_end_slot,
                proposal.not_before_slot,
                proposal.expiry_slot,
            )
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
}

pub(super) fn load_handoff_receipt(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    context: &CeremonyContext,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn load_activation_proposal(
    program_id: &Pubkey,
    info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    policy_info: &AccountInfo<'_>,
    capacity_info: &AccountInfo<'_>,
    gate_info: &AccountInfo<'_>,
    immutability_info: &AccountInfo<'_>,
    handoff_receipt_info: &AccountInfo<'_>,
    context: &CeremonyContext,
) -> Result<Box<BootstrapActivationProposalV1>, ProgramError> {
    let proposal = load_fixed_controller_account::<BootstrapActivationProposalV1>(
        program_id,
        info,
        BootstrapActivationProposalV1::LEN,
    )?;
    validate_bootstrap_activation_proposal_digest_v1(&proposal)?;
    let expected = derive_bootstrap_activation_pda(
        program_id,
        &context.config.target_program,
        context.council.version,
    );
    if expected.0 != *info.key
        || expected.1 != proposal.bump
        || proposal.controller_program != *program_id
        || proposal.controller_programdata != derive_upgradeable_programdata_address(program_id).0
        || proposal.controller_config != *config_info.key
        || proposal.governance_policy != *policy_info.key
        || proposal.governance_policy_hash != context.policy.policy_hash
        || proposal.capacity_policy != *capacity_info.key
        || proposal.capacity_policy_digest != context.capacity.policy_digest
        || proposal.controller_immutability_receipt != *immutability_info.key
        || proposal.target_handoff_receipt != *handoff_receipt_info.key
        || proposal.gate != *gate_info.key
        || proposal.cluster_domain != context.config.cluster_domain
        || proposal.target_program != context.config.target_program
        || proposal.target_programdata != context.config.target_programdata
        || proposal.upgradeable_loader != context.config.upgradeable_loader
        || proposal.controller_authority != context.config.authority_pda
        || proposal.council_version != context.council.version
        || proposal.council_hash != context.council.set_hash
        || proposal.bootstrap_gate_epoch != context.gate.epoch
        || proposal.target_nonce != context.config.target_nonce
        || derive_major_timing(&context.config, proposal.creation_slot)?
            != (
                proposal.review_start_slot,
                proposal.review_end_slot,
                proposal.not_before_slot,
                proposal.expiry_slot,
            )
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(proposal)
}

pub(super) fn require_zero_initialized_destination(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    expected_len: usize,
) -> ProgramResult {
    if account.owner != program_id || account.data_len() != expected_len {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    if account.lamports() < Rent::get()?.minimum_balance(expected_len) {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }
    if account.try_borrow_data()?.iter().any(|byte| *byte != 0) {
        return Err(GovernanceError::CeremonyAlreadyFinalized.into());
    }
    Ok(())
}

/// Creates a one-time result PDA, or reuses its exact untouched preallocation after a council
/// rotation made the prior versioned proposal stale.  A reusable destination must already be the
/// canonical PDA, controller-owned, exact-size, rent-exempt, and byte-for-byte zero.
#[allow(clippy::too_many_arguments)]
pub(super) fn create_or_reuse_zero_fixed_pda<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    pda: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    rent: &Rent,
    space: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(pda, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    let expected = Pubkey::create_program_address(signer_seeds, program_id)
        .map_err(|_| GovernanceError::InvalidPda)?;
    if expected != *pda.key || *system_program_info.key != system_program::ID {
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

/// Borrows every destination before the first copy, so an account-borrow or length failure cannot
/// leave a partially written controller state in native tests.  Runtime transaction atomicity also
/// rolls back any earlier System/Loader CPI if this commit fails.
pub(super) fn commit_preencoded(
    program_id: &Pubkey,
    destinations: &[(&AccountInfo<'_>, &Vec<u8>)],
) -> ProgramResult {
    for (account, bytes) in destinations {
        if account.owner != program_id {
            return Err(GovernanceError::IncorrectAccountOwner.into());
        }
        if account.data_len() != bytes.len() {
            return Err(GovernanceError::InvalidAccountSize.into());
        }
    }
    let mut borrows = Vec::with_capacity(destinations.len());
    for (account, _) in destinations {
        borrows.push(account.try_borrow_mut_data()?);
    }
    for ((_, bytes), data) in destinations.iter().zip(borrows.iter_mut()) {
        data.copy_from_slice(bytes);
    }
    Ok(())
}
