use super::*;

/// Closes only a canonical controller-owned Loader-v3 Buffer belonging to a
/// Cancelled, Expired, or explicitly Retired rollback proposal, and transfers
/// every lamport only to the pinned treasury.
pub fn process_close_abandoned_buffer_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CloseAbandonedBufferV1,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, verification_info, buffer_info, treasury_info, authority_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [config_info, gate_info, proposal_info, authority_info] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(verification_info, true, false, false)?;
    validate_exact_privileges(buffer_info, true, false, false)?;
    validate_exact_privileges(treasury_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        proposal_info,
        verification_info,
        buffer_info,
        treasury_info,
        authority_info,
        loader_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    check_proposal_expectation_without_live_council(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
    )?;
    validate_abandoned_buffer_close_state(&proposal, proposal_info.key, &config, &gate)?;
    validate_close_identities(
        &config,
        &proposal,
        verification_info.key,
        buffer_info.key,
        treasury_info.key,
        authority_info.key,
    )?;

    let mut verification = load_buffer_verification(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        buffer_info,
        &config,
        &proposal,
    )?;
    if verification.status != instruction.expected_verification_status
        || verification.verified_chunk_bitmap != instruction.expected_verified_chunk_bitmap
        || verification.verified_chunk_count != instruction.expected_verified_chunk_count
        || verification.finalized_slot != instruction.expected_buffer_verification_finalized_slot
        || matches!(
            verification.status,
            BufferVerificationStatusV1::ConsumedByUpgrade
                | BufferVerificationStatusV1::ClosedAbandoned
        )
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let buffer_header = validate_buffer_account(buffer_info, &config.upgradeable_loader)?;
    let buffer_data = buffer_info.try_borrow_data()?;
    let header_hash = hashv(&[&buffer_data[..LOADER_BUFFER_METADATA_LEN]]).to_bytes();
    if buffer_header.authority != Some(config.authority_pda)
        || buffer_header.payload_length as u64 != proposal.artifact_length
        || header_hash != verification.sealed_buffer_header_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    drop(buffer_data);

    let slot = Clock::get()?.slot;
    if slot == 0 {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    verification.status = BufferVerificationStatusV1::ClosedAbandoned;
    verification.terminal_slot = slot;
    verification.validate_schema()?;
    let verification_bytes = encode_fixed_account(&*verification, BufferVerificationV1::LEN)?;

    let close_instruction = close(buffer_info.key, treasury_info.key, authority_info.key);
    validate_close_cpi_shape(
        &close_instruction,
        buffer_info.key,
        treasury_info.key,
        authority_info.key,
    )?;
    let authority_bump = derive_authority_pda(program_id, &config.target_program).1;
    let bump_seed = [authority_bump];
    let signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &bump_seed,
    ];
    invoke_signed(
        &close_instruction,
        &[
            buffer_info.clone(),
            treasury_info.clone(),
            authority_info.clone(),
            loader_info.clone(),
        ],
        &[signer_seeds],
    )?;
    if buffer_info.lamports() != 0 {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    commit_one_fixed_account(
        program_id,
        verification_info,
        &verification_bytes,
        BufferVerificationV1::LEN,
    )
}

pub(super) fn validate_close_identities(
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    verification: &Pubkey,
    buffer: &Pubkey,
    treasury: &Pubkey,
    authority: &Pubkey,
) -> ProgramResult {
    if *buffer != proposal.buffer_pubkey
        || *verification != proposal.buffer_verification
        || *treasury != config.canonical_spill_treasury
        || *treasury != proposal.canonical_spill_treasury
        || *authority != config.authority_pda
        || *authority != proposal.authority_pda
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn validate_abandoned_buffer_close_state(
    proposal: &UpgradeProposalV2,
    proposal_key: &Pubkey,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    let closable = matches!(
        proposal.state,
        ProposalStateV2::Cancelled | ProposalStateV2::Expired
    ) || proposal.state == ProposalStateV2::Retired
        && proposal.proposal_class == ProposalClassV1::EmergencyRollback;
    if !closable || gate.active_proposal == *proposal_key {
        return Err(GovernanceError::InvalidStateTransition.into());
    }

    // Cancelled/Expired is only a legitimate rollback terminal while its
    // linked primary has not crossed the freeze boundary. Once that exact
    // primary is the active frozen proposal and has consumed the nonce, its
    // sealed rollback buffer is mandatory recovery custody. The ordinary
    // successful unfreeze path retires the rollback atomically, after which
    // the buffer may be closed through the explicit Retired lane.
    if proposal.proposal_class == ProposalClassV1::EmergencyRollback
        && matches!(
            proposal.state,
            ProposalStateV2::Cancelled | ProposalStateV2::Expired
        )
    {
        let consumed_nonce = proposal
            .target_nonce
            .checked_add(1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        if proposal.primary_proposal.present
            && config.target_nonce == consumed_nonce
            && gate.status == GateStatusV1::FrozenForUpgrade
            && gate.active_proposal == proposal.primary_proposal.value
        {
            return Err(GovernanceError::InvalidStateTransition.into());
        }
    }
    Ok(())
}

fn load_buffer_verification(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    buffer_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
) -> Result<Box<BufferVerificationV1>, ProgramError> {
    let verification = load_buffer_verification_without_live_buffer(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        config,
        proposal,
    )?;
    if verification.buffer != *buffer_info.key {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

pub(super) fn load_buffer_verification_without_live_buffer(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
) -> Result<Box<BufferVerificationV1>, ProgramError> {
    let verification = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        verification_info,
        BufferVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_buffer_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || *verification_info.key != proposal.buffer_verification
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.buffer != proposal.buffer_pubkey
        || verification.expected_uploader_authority != proposal.buffer_uploader_authority
        || verification.controller_authority != config.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != proposal.chunk_hash_domain
        || verification.chunk_size != proposal.chunk_size
        || verification.chunk_count != proposal.chunk_count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}
