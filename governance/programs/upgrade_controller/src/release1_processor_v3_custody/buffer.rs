use super::*;

/// Transfers one exact Loader buffer from the recorded uploader to the
/// controller PDA and creates its fixed verification record.
pub fn process_adopt_buffer_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: AdoptBufferV2,
) -> ProgramResult {
    let [payer, config_info, gate_info, proposal_info, capacity_info, deployment_info, buffer_info, uploader_authority, authority_info, verification_info, loader_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(payer, true, true, false)?;
    for account in [
        config_info,
        gate_info,
        capacity_info,
        deployment_info,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(buffer_info, true, false, false)?;
    validate_exact_privileges(uploader_authority, false, true, false)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        proposal_info,
        capacity_info,
        deployment_info,
        buffer_info,
        uploader_authority,
        authority_info,
        verification_info,
        loader_info,
        system_program_info,
    ])?;
    require_program_ids(loader_info, Some(system_program_info), None, None, None)?;

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::Draft {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    let deployment = load_current_deployment(
        program_id,
        deployment_info,
        config_info,
        capacity_info,
        &config,
        &capacity,
    )?;
    require_deployment_guard(&instruction.expected, &proposal, &deployment)?;
    if *buffer_info.key != proposal.buffer_pubkey
        || buffer_info.owner != &proposal.buffer_loader_owner
        || *uploader_authority.key != proposal.buffer_uploader_authority
        || *authority_info.key != proposal.buffer_final_authority
        || *authority_info.key != config.authority_pda
        || proposal.buffer_loader_owner != UPGRADEABLE_LOADER_ID
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let (expected_authority, authority_bump) =
        derive_authority_pda(program_id, &config.target_program);
    let (expected_verification, verification_bump) =
        derive_buffer_check_pda(program_id, proposal_info.key);
    if expected_authority != *authority_info.key
        || expected_verification != *verification_info.key
        || proposal.buffer_verification != *verification_info.key
        || verification_info.owner != &system_program::ID
        || verification_info.data_len() != 0
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    let header = validate_buffer_account(buffer_info, &config.upgradeable_loader)?;
    if header.authority != Some(*uploader_authority.key)
        || u64::try_from(header.payload_length).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != proposal.artifact_length
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let slot = current_preexpiry_slot(&proposal)?;
    let sealed_header = canonical_buffer_header(authority_info.key);
    let sealed_header_hash = hashv(&[&sealed_header]).to_bytes();

    let mut next_proposal = (*proposal).clone();
    next_proposal.state = ProposalStateV2::BufferAdopted;
    validate_upgrade_proposal_digest_v3(&next_proposal)?;
    let verification = BufferVerificationV1 {
        discriminator: BUFFER_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: verification_bump,
        initialized: true,
        status: BufferVerificationStatusV1::Adopted,
        controller_config: *config_info.key,
        proposal: *proposal_info.key,
        upgradeable_loader: config.upgradeable_loader,
        buffer: *buffer_info.key,
        expected_uploader_authority: *uploader_authority.key,
        controller_authority: *authority_info.key,
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_chunk_merkle_root: proposal.artifact_chunk_merkle_root,
        chunk_hash_domain: proposal.artifact_scheme_id,
        chunk_size: proposal.artifact_chunk_size,
        chunk_count: proposal.artifact_chunk_count,
        verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        verified_chunk_count: 0,
        adopted_slot: slot,
        finalized_slot: 0,
        sealed_buffer_header_hash: sealed_header_hash,
        terminal_slot: 0,
        reserved: [0; BUFFER_VERIFICATION_V1_RESERVED_LEN],
    };
    verification.validate_schema()?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV3::LEN)?;
    let verification_bytes = encode_fixed_account(&verification, BufferVerificationV1::LEN)?;

    let authority_bump_seed = [authority_bump];
    let authority_signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &authority_bump_seed,
    ];
    let set_authority =
        set_buffer_authority_checked(buffer_info.key, uploader_authority.key, authority_info.key);
    validate_set_buffer_authority_checked_cpi_shape(
        &set_authority,
        buffer_info.key,
        uploader_authority.key,
        authority_info.key,
    )?;
    invoke_signed(
        &set_authority,
        &[
            buffer_info.clone(),
            uploader_authority.clone(),
            authority_info.clone(),
            loader_info.clone(),
        ],
        &[authority_signer_seeds],
    )?;
    let sealed = validate_buffer_account(buffer_info, &config.upgradeable_loader)?;
    let buffer_data = buffer_info.try_borrow_data()?;
    let observed_header_hash = hashv(&[buffer_data
        .get(..LOADER_BUFFER_METADATA_LEN)
        .ok_or(GovernanceError::InvalidRelease1Account)?])
    .to_bytes();
    drop(buffer_data);
    if sealed.authority != Some(*authority_info.key)
        || sealed.payload_length != header.payload_length
        || observed_header_hash != sealed_header_hash
    {
        return Err(GovernanceError::InvalidAuthorityTransition.into());
    }
    let verification_bump_seed = [verification_bump];
    let verification_signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        BUFFER_CHECK_SEED,
        proposal_info.key.as_ref(),
        &verification_bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        verification_info,
        system_program_info,
        &Rent::get()?,
        BufferVerificationV1::LEN,
        verification_signer_seeds,
    )?;
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV3::LEN,
        verification_info,
        &verification_bytes,
        BufferVerificationV1::LEN,
    )
}

/// Verifies one exact, never-before-counted chunk from the locked buffer.
pub fn process_verify_buffer_chunk_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: VerifyBufferChunkV2,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, buffer_info, verification_info, authority_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    for account in [
        config_info,
        gate_info,
        proposal_info,
        buffer_info,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(verification_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        proposal_info,
        buffer_info,
        verification_info,
        authority_info,
        loader_info,
    ])?;
    require_program_ids(loader_info, None, None, None, None)?;
    instruction.proof.validate()?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::BufferAdopted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    current_preexpiry_slot(&proposal)?;
    let verification = load_buffer_verification(
        program_id,
        verification_info,
        proposal_info,
        buffer_info,
        authority_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_buffer_expectation(
        &verification,
        instruction.expected_verification_status,
        &instruction.expected_verified_chunk_bitmap,
        instruction.expected_verified_chunk_count,
    )?;
    validate_sealed_buffer(buffer_info, authority_info.key, &config, &verification)?;
    let buffer_data = buffer_info.try_borrow_data()?;
    let payload = buffer_data
        .get(LOADER_BUFFER_METADATA_LEN..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    let exact_chunk = exact_artifact_chunk(
        payload,
        proposal.artifact_length,
        proposal.artifact_chunk_size,
        instruction.chunk_index,
    )?;
    let next = verify_and_mark_chunk(
        &verification,
        instruction.chunk_index,
        exact_chunk,
        &instruction.proof.nodes[..usize::from(instruction.proof.proof_len)],
    )?;
    let bytes = encode_fixed_account(&next, BufferVerificationV1::LEN)?;
    drop(buffer_data);
    commit_one_fixed_account(
        program_id,
        verification_info,
        &bytes,
        BufferVerificationV1::LEN,
    )
}

/// Finalizes the complete chunk bitmap after every buffer byte has been bound
/// through the proposal's artifact Merkle root. The proposal SHA-256 remains a
/// release-identity commitment; this bounded instruction never one-shot hashes
/// a multi-megabyte Loader buffer.
pub fn process_finalize_buffer_verification_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeBufferVerificationV2,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, buffer_info, verification_info, authority_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, false, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(buffer_info, false, false, false)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    validate_exact_privileges(authority_info, false, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        proposal_info,
        buffer_info,
        verification_info,
        authority_info,
        loader_info,
    ])?;
    require_program_ids(loader_info, None, None, None, None)?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::BufferAdopted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let slot = current_preexpiry_slot(&proposal)?;
    let verification = load_buffer_verification(
        program_id,
        verification_info,
        proposal_info,
        buffer_info,
        authority_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_buffer_expectation(
        &verification,
        instruction.expected_verification_status,
        &instruction.expected_verified_chunk_bitmap,
        instruction.expected_verified_chunk_count,
    )?;
    if verification.sealed_buffer_header_hash != instruction.expected_sealed_buffer_header_hash {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_sealed_buffer(buffer_info, authority_info.key, &config, &verification)?;
    if verification.status != BufferVerificationStatusV1::ReadyToFinalize {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let mut next_verification = (*verification).clone();
    next_verification.status = BufferVerificationStatusV1::Verified;
    next_verification.finalized_slot = slot;
    next_verification.validate_schema()?;
    let mut next_proposal = (*proposal).clone();
    next_proposal.state = ProposalStateV2::BufferVerified;
    validate_upgrade_proposal_digest_v3(&next_proposal)?;
    let verification_bytes = encode_fixed_account(&next_verification, BufferVerificationV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV3::LEN)?;
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV3::LEN,
        verification_info,
        &verification_bytes,
        BufferVerificationV1::LEN,
    )
}

/// Closes only a terminal abandoned buffer to the one configured treasury.
pub fn process_close_abandoned_buffer_v2(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CloseAbandonedBufferV2,
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
    require_program_ids(loader_info, None, None, None, None)?;
    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_proposal_guard(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if !matches!(
        proposal.state,
        ProposalStateV2::Cancelled | ProposalStateV2::Expired | ProposalStateV2::Retired
    ) || (proposal.state == ProposalStateV2::Retired
        && proposal.proposal_class != ProposalClassV1::EmergencyRollback)
        || *treasury_info.key != config.canonical_spill_treasury
        || *treasury_info.key != proposal.canonical_spill_treasury
        || *buffer_info.key != proposal.buffer_pubkey
        || *verification_info.key != proposal.buffer_verification
        || *authority_info.key != config.authority_pda
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let mut verification = load_buffer_verification(
        program_id,
        verification_info,
        proposal_info,
        buffer_info,
        authority_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_buffer_expectation(
        &verification,
        instruction.expected_verification_status,
        &instruction.expected_verified_chunk_bitmap,
        instruction.expected_verified_chunk_count,
    )?;
    if verification.finalized_slot != instruction.expected_buffer_verification_finalized_slot
        || matches!(
            verification.status,
            BufferVerificationStatusV1::ConsumedByUpgrade
                | BufferVerificationStatusV1::ClosedAbandoned
        )
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_sealed_buffer(buffer_info, authority_info.key, &config, &verification)?;
    let slot = Clock::get()?.slot;
    if slot == 0 {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    verification.status = BufferVerificationStatusV1::ClosedAbandoned;
    verification.terminal_slot = slot;
    verification.validate_schema()?;
    let bytes = encode_fixed_account(&*verification, BufferVerificationV1::LEN)?;
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
        &bytes,
        BufferVerificationV1::LEN,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_buffer_verification(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    buffer_info: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV3,
) -> Result<Box<BufferVerificationV1>, ProgramError> {
    let verification = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        verification_info,
        BufferVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_buffer_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || proposal.buffer_verification != *verification_info.key
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.buffer != *buffer_info.key
        || verification.buffer != proposal.buffer_pubkey
        || verification.expected_uploader_authority != proposal.buffer_uploader_authority
        || verification.controller_authority != *authority_info.key
        || verification.controller_authority != config.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != proposal.artifact_scheme_id
        || verification.chunk_size != proposal.artifact_chunk_size
        || verification.chunk_count != proposal.artifact_chunk_count
    {
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
    proposal: &UpgradeProposalV3,
) -> Result<Box<BufferVerificationV1>, ProgramError> {
    let verification = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        verification_info,
        BufferVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_buffer_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || proposal.buffer_verification != *verification_info.key
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.buffer != proposal.buffer_pubkey
        || verification.expected_uploader_authority != proposal.buffer_uploader_authority
        || verification.controller_authority != config.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != proposal.artifact_scheme_id
        || verification.chunk_size != proposal.artifact_chunk_size
        || verification.chunk_count != proposal.artifact_chunk_count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

fn require_buffer_expectation(
    verification: &BufferVerificationV1,
    status: BufferVerificationStatusV1,
    bitmap: &[u8; VERIFICATION_BITMAP_BYTES_V1],
    count: u32,
) -> ProgramResult {
    if verification.status != status
        || verification.verified_chunk_bitmap != *bitmap
        || verification.verified_chunk_count != count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn validate_sealed_buffer(
    buffer_info: &AccountInfo<'_>,
    authority: &Pubkey,
    config: &ControllerConfigV1,
    verification: &BufferVerificationV1,
) -> ProgramResult {
    let header = validate_buffer_account(buffer_info, &config.upgradeable_loader)?;
    let data = buffer_info.try_borrow_data()?;
    let header_hash = hashv(&[data
        .get(..LOADER_BUFFER_METADATA_LEN)
        .ok_or(GovernanceError::InvalidRelease1Account)?])
    .to_bytes();
    if header.authority != Some(*authority)
        || u64::try_from(header.payload_length).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != verification.artifact_length
        || header_hash != verification.sealed_buffer_header_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

fn exact_artifact_chunk(
    payload: &[u8],
    artifact_length: u64,
    chunk_size: u32,
    chunk_index: u32,
) -> Result<&[u8], ProgramError> {
    if u64::try_from(payload.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
        != artifact_length
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    exact_region_chunk(payload, 0, artifact_length, chunk_size, chunk_index)
}

fn verify_and_mark_chunk(
    verification: &BufferVerificationV1,
    chunk_index: u32,
    exact_chunk: &[u8],
    proof: &[[u8; 32]],
) -> Result<BufferVerificationV1, ProgramError> {
    if !matches!(
        verification.status,
        BufferVerificationStatusV1::Adopted | BufferVerificationStatusV1::Verifying
    ) || chunk_index >= verification.chunk_count
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let byte_index =
        usize::try_from(chunk_index / 8).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let bit = 1u8 << (chunk_index % 8);
    if verification.verified_chunk_bitmap[byte_index] & bit != 0 {
        return Err(GovernanceError::InvalidRelease1Bitmap.into());
    }
    verify_artifact_chunk_proof(
        &verification.artifact_chunk_merkle_root,
        verification.artifact_length,
        verification.chunk_size,
        chunk_index,
        exact_chunk,
        proof,
    )?;
    let mut next = verification.clone();
    next.verified_chunk_bitmap[byte_index] |= bit;
    next.verified_chunk_count = next
        .verified_chunk_count
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    next.status = if next.verified_chunk_count == next.chunk_count {
        BufferVerificationStatusV1::ReadyToFinalize
    } else {
        BufferVerificationStatusV1::Verifying
    };
    next.validate_schema()?;
    Ok(next)
}

fn canonical_buffer_header(authority: &Pubkey) -> [u8; LOADER_BUFFER_METADATA_LEN] {
    let mut bytes = [0u8; LOADER_BUFFER_METADATA_LEN];
    bytes[..4].copy_from_slice(&1u32.to_le_bytes());
    bytes[4] = 1;
    bytes[5..].copy_from_slice(authority.as_ref());
    bytes
}
