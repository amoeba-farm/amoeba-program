use super::*;

/// Verifies one deployed artifact or zero-tail chunk directly from canonical
/// ProgramData bytes and records the corresponding non-replayable bitmap bit.
pub fn process_verify_programdata_chunk_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: VerifyProgramDataChunkV1,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, target_program, target_programdata, authority_info, loader_info, verification_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    instruction.validate_phase_proof()?;
    for account in [
        config_info,
        gate_info,
        proposal_info,
        target_program,
        target_programdata,
        authority_info,
    ] {
        validate_exact_privileges(account, false, false, account.key == target_program.key)?;
    }
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        proposal_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        verification_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_frozen_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::UpgradeExecuted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_target_account_keys(&config, target_program, target_programdata, authority_info)?;
    let mut verification = load_programdata_verification(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_programdata_verification_expectation(
        &verification,
        instruction.expected_verification_status,
        &instruction.expected_verified_payload_chunk_bitmap,
        instruction.expected_verified_payload_chunk_count,
        &instruction.expected_verified_tail_chunk_bitmap,
        instruction.expected_verified_tail_chunk_count,
    )?;
    if verification.status != ProgramDataVerificationStatusV1::Verifying {
        return Err(GovernanceError::InvalidStateTransition.into());
    }

    let header =
        validate_current_programdata(target_program, target_programdata, &config, &verification)?;
    let data = target_programdata.try_borrow_data()?;
    let payload = data
        .get(header.payload_offset..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    match instruction.phase {
        ProgramDataChunkPhaseV1::Payload => {
            let exact = exact_region_chunk(
                payload,
                0,
                verification.artifact_length,
                verification.chunk_size,
                instruction.chunk_index,
            )?;
            let proof_len = usize::from(instruction.proof.proof_len);
            verify_artifact_chunk_proof(
                &verification.artifact_chunk_merkle_root,
                verification.artifact_length,
                verification.chunk_size,
                instruction.chunk_index,
                exact,
                &instruction.proof.nodes[..proof_len],
            )?;
            mark_bitmap_bit(
                &mut verification.verified_payload_chunk_bitmap,
                &mut verification.verified_payload_chunk_count,
                verification.payload_chunk_count,
                instruction.chunk_index,
            )?;
        }
        ProgramDataChunkPhaseV1::ZeroTail => {
            if !proposal.zero_tail_required {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
            let exact = exact_region_chunk(
                payload,
                verification.artifact_length,
                verification.tail_length,
                verification.chunk_size,
                instruction.chunk_index,
            )?;
            if exact.iter().any(|byte| *byte != 0) {
                return Err(GovernanceError::Release1DigestMismatch.into());
            }
            mark_bitmap_bit(
                &mut verification.verified_tail_chunk_bitmap,
                &mut verification.verified_tail_chunk_count,
                verification.tail_chunk_count,
                instruction.chunk_index,
            )?;
        }
    }
    let complete = verification.verified_payload_chunk_count == verification.payload_chunk_count
        && verification.verified_tail_chunk_count == verification.tail_chunk_count;
    verification.status = if complete {
        ProgramDataVerificationStatusV1::ReadyToFinalize
    } else {
        ProgramDataVerificationStatusV1::Verifying
    };
    verification.validate_schema()?;
    let bytes = encode_fixed_account(&*verification, ProgramDataVerificationV1::LEN)?;
    drop(data);
    commit_one_fixed_account(
        program_id,
        verification_info,
        &bytes,
        ProgramDataVerificationV1::LEN,
    )
}

/// Records one raw ProgramData hash after every artifact and zero-tail chunk
/// has been independently verified, then advances only to
/// `ProgramDataVerified`.
pub fn process_finalize_programdata_verification_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeProgramDataVerificationV1,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, target_program, target_programdata, authority_info, loader_info, verification_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, false, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(target_program, false, false, true)?;
    validate_exact_privileges(target_programdata, false, false, false)?;
    validate_exact_privileges(authority_info, false, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    require_distinct_accounts(&[
        config_info,
        gate_info,
        proposal_info,
        target_program,
        target_programdata,
        authority_info,
        loader_info,
        verification_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_frozen_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::UpgradeExecuted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let slot = current_post_execution_frozen_slot(&proposal)?;
    validate_target_account_keys(&config, target_program, target_programdata, authority_info)?;
    let mut verification = load_programdata_verification(
        program_id,
        verification_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_programdata_verification_expectation(
        &verification,
        instruction.expected_verification_status,
        &instruction.expected_verified_payload_chunk_bitmap,
        instruction.expected_verified_payload_chunk_count,
        &instruction.expected_verified_tail_chunk_bitmap,
        instruction.expected_verified_tail_chunk_count,
    )?;
    if verification.status != ProgramDataVerificationStatusV1::ReadyToFinalize
        || instruction.expected_deployed_slot != verification.deployed_slot
        || instruction.expected_capacity != verification.capacity
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let header =
        validate_current_programdata(target_program, target_programdata, &config, &verification)?;
    let data = target_programdata.try_borrow_data()?;
    if u64::try_from(data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
        > MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    if header.payload_offset != data.len().saturating_sub(header.capacity)
        || verification.verified_payload_chunk_count != verification.payload_chunk_count
        || verification.verified_tail_chunk_count != verification.tail_chunk_count
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    let raw_hash = loader_account_data_hash(&data);
    drop(data);

    verification.status = ProgramDataVerificationStatusV1::Verified;
    verification.zero_tail_verified = true;
    verification.raw_programdata_hash = raw_hash;
    verification.finalized_slot = slot;
    verification.validate_schema()?;
    proposal.state = ProposalStateV2::ProgramDataVerified;
    proposal.programdata_verified_slot = slot;
    validate_proposal_digest_v2(&proposal)?;
    let verification_bytes = encode_fixed_account(&*verification, ProgramDataVerificationV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&*proposal, UpgradeProposalV2::LEN)?;
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV2::LEN,
        verification_info,
        &verification_bytes,
        ProgramDataVerificationV1::LEN,
    )
}

fn load_programdata_verification(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
) -> Result<Box<ProgramDataVerificationV1>, ProgramError> {
    let verification = load_fixed_controller_account::<ProgramDataVerificationV1>(
        program_id,
        verification_info,
        ProgramDataVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_programdata_check_pda(program_id, proposal_info.key)
        != (*verification_info.key, verification.bump)
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.target_program != config.target_program
        || verification.target_programdata != config.target_programdata
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.controller_authority != config.authority_pda
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_chunk_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.chunk_hash_domain != proposal.chunk_hash_domain
        || verification.chunk_size != proposal.chunk_size
        || verification.payload_chunk_count != proposal.chunk_count
        || verification.capacity != proposal.expected_post_capacity
        || verification.deployed_slot != proposal.upgrade_executed_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

fn validate_current_programdata(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    verification: &ProgramDataVerificationV1,
) -> Result<crate::release1_loader_accounts::UpgradeableProgramDataHeaderV1, ProgramError> {
    let header = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if header.upgrade_authority != Some(config.authority_pda)
        || header.deployed_slot != verification.deployed_slot
        || u64::try_from(header.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != verification.capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(header)
}

fn require_programdata_verification_expectation(
    verification: &ProgramDataVerificationV1,
    status: ProgramDataVerificationStatusV1,
    payload_bitmap: &[u8; VERIFICATION_BITMAP_BYTES_V1],
    payload_count: u32,
    tail_bitmap: &[u8; VERIFICATION_BITMAP_BYTES_V1],
    tail_count: u32,
) -> ProgramResult {
    if verification.status != status
        || verification.verified_payload_chunk_bitmap != *payload_bitmap
        || verification.verified_payload_chunk_count != payload_count
        || verification.verified_tail_chunk_bitmap != *tail_bitmap
        || verification.verified_tail_chunk_count != tail_count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}

pub(super) fn mark_bitmap_bit(
    bitmap: &mut [u8; VERIFICATION_BITMAP_BYTES_V1],
    count: &mut u32,
    limit: u32,
    index: u32,
) -> ProgramResult {
    if index >= limit {
        return Err(GovernanceError::InvalidRelease1Bitmap.into());
    }
    let byte_index = usize::try_from(index / 8).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let bit = 1u8 << (index % 8);
    if bitmap[byte_index] & bit != 0 {
        return Err(GovernanceError::InvalidRelease1Bitmap.into());
    }
    bitmap[byte_index] |= bit;
    *count = count
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    Ok(())
}

pub(super) fn exact_region_chunk(
    payload: &[u8],
    region_start: u64,
    region_length: u64,
    chunk_size: u32,
    chunk_index: u32,
) -> Result<&[u8], ProgramError> {
    let relative_start = u64::from(chunk_index)
        .checked_mul(u64::from(chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let remaining = region_length
        .checked_sub(relative_start)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    let length = remaining.min(u64::from(chunk_size));
    if length == 0 {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    let start = region_start
        .checked_add(relative_start)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let end = start
        .checked_add(length)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let start = usize::try_from(start).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let end = usize::try_from(end).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    payload
        .get(start..end)
        .ok_or_else(|| GovernanceError::InvalidMerkleProof.into())
}

pub(super) fn chunk_count_allow_empty(length: u64, chunk_size: u32) -> Result<u32, ProgramError> {
    if length == 0 {
        return Ok(0);
    }
    let size = u64::from(chunk_size);
    let count = length
        .checked_add(size - 1)
        .ok_or(GovernanceError::ArithmeticOverflow)?
        / size;
    u32::try_from(count).map_err(|_| GovernanceError::InvalidMerkleParameters.into())
}
