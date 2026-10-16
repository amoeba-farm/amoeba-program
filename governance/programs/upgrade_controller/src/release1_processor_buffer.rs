//! Typed Release 1 buffer adoption and exact-byte verification.
//!
//! This module exposes only the three closed buffer operations.  Adoption can
//! perform one fixed Loader-v3 `SetAuthorityChecked` CPI; chunk verification
//! and finalization treat the sealed buffer as read-only and mutate only the
//! controller-owned fixed-width verification/proposal accounts.

use solana_loader_v3_interface::instruction::set_buffer_authority_checked;
use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult, hash::hashv,
    program::invoke_signed, program_error::ProgramError, pubkey::Pubkey, rent::Rent,
    sysvar::Sysvar,
};
use solana_sdk_ids::system_program;

use crate::{
    artifact_merkle::verify_artifact_chunk_proof,
    instruction::{
        AdoptBufferV1, FinalizeBufferVerificationV1, ProposalExpectationV2, VerifyBufferChunkV1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_gate_pda, derive_programdata_check_pda,
        derive_proposal_pda, derive_upgradeable_programdata_address, AUTHORITY_SEED,
        BUFFER_CHECK_SEED, UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, validate_exact_privileges,
    },
    release1_digest::validate_proposal_digest_v2,
    release1_loader_accounts::{validate_buffer_account, LOADER_BUFFER_METADATA_LEN},
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, ProposalStateV2, UpgradeProposalV2,
        BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1, BUFFER_VERIFICATION_V1_DISCRIMINATOR,
        BUFFER_VERIFICATION_V1_RESERVED_LEN, RELEASE1_ACCOUNT_VERSION_V1,
        VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, ProposalClassV1, ProtocolGateV1},
    GovernanceError,
};

/// Adopts the exact proposal buffer by transferring its Loader-v3 authority to
/// the controller PDA with `SetAuthorityChecked` and creating the immutable
/// verification record.
pub fn process_adopt_buffer_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: AdoptBufferV1,
) -> ProgramResult {
    let [payer, config_info, gate_info, proposal_info, buffer_info, uploader_authority, authority_info, verification_info, loader_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };

    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, false, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(buffer_info, true, false, false)?;
    validate_exact_privileges(uploader_authority, false, true, false)?;
    validate_exact_privileges(authority_info, false, false, false)?;
    validate_exact_privileges(verification_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        proposal_info,
        buffer_info,
        uploader_authority,
        authority_info,
        verification_info,
        loader_info,
        system_program_info,
    ])?;
    if *loader_info.key != UPGRADEABLE_LOADER_ID || *system_program_info.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_expectation_and_creation_gate(&instruction.expected, &proposal, &config, &gate)?;
    if proposal.state != ProposalStateV2::Draft {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
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
    if expected_authority != *authority_info.key {
        return Err(GovernanceError::InvalidPda.into());
    }
    let (expected_verification, verification_bump) =
        derive_buffer_check_pda(program_id, proposal_info.key);
    if expected_verification != *verification_info.key
        || proposal.buffer_verification != *verification_info.key
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    if verification_info.owner != &system_program::ID || verification_info.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
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
    validate_proposal_digest_v2(&next_proposal)?;
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
        chunk_hash_domain: proposal.chunk_hash_domain,
        chunk_size: proposal.chunk_size,
        chunk_count: proposal.chunk_count,
        verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        verified_chunk_count: 0,
        adopted_slot: slot,
        finalized_slot: 0,
        sealed_buffer_header_hash: sealed_header_hash,
        terminal_slot: 0,
        reserved: [0; BUFFER_VERIFICATION_V1_RESERVED_LEN],
    };
    verification.validate_schema()?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV2::LEN)?;
    let verification_bytes = encode_fixed_account(&verification, BufferVerificationV1::LEN)?;
    let rent = Rent::get()?;

    let authority_bump_seed = [authority_bump];
    let authority_signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &authority_bump_seed,
    ];
    let set_authority =
        set_buffer_authority_checked(buffer_info.key, uploader_authority.key, authority_info.key);
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

    // Bind the record only after re-reading the Loader-mutated account.  The
    // exact canonical header, authority, and payload length must all match.
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
        return Err(GovernanceError::CrossAccountMismatch.into());
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
        &rent,
        BufferVerificationV1::LEN,
        verification_signer_seeds,
    )?;
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV2::LEN,
        verification_info,
        &verification_bytes,
        BufferVerificationV1::LEN,
    )
}

/// Verifies one exact payload chunk against the proposal's canonical Merkle
/// root and atomically records one previously-unset bitmap bit.
pub fn process_verify_buffer_chunk_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: VerifyBufferChunkV1,
) -> ProgramResult {
    let [config_info, gate_info, proposal_info, buffer_info, verification_info, authority_info, loader_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    instruction.proof.validate()?;
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
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_expectation_and_creation_gate(&instruction.expected, &proposal, &config, &gate)?;
    if proposal.state != ProposalStateV2::BufferAdopted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    current_preexpiry_slot(&proposal)?;
    let verification = load_verification(
        program_id,
        verification_info,
        proposal_info,
        buffer_info,
        authority_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_verification_expectation(
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
        proposal.chunk_size,
        instruction.chunk_index,
    )?;
    let proof_len = usize::from(instruction.proof.proof_len);
    let next_verification = verify_and_mark_chunk(
        &verification,
        instruction.chunk_index,
        exact_chunk,
        &instruction.proof.nodes[..proof_len],
    )?;
    let encoded = encode_fixed_account(&next_verification, BufferVerificationV1::LEN)?;
    drop(buffer_data);
    commit_one_fixed_account(
        program_id,
        verification_info,
        &encoded,
        BufferVerificationV1::LEN,
    )
}

/// Recomputes the full sealed payload SHA-256 only after every canonical chunk
/// has been verified, then advances both records to their verified states.
pub fn process_finalize_buffer_verification_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: FinalizeBufferVerificationV1,
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
    if *loader_info.key != UPGRADEABLE_LOADER_ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_expectation_and_creation_gate(&instruction.expected, &proposal, &config, &gate)?;
    if proposal.state != ProposalStateV2::BufferAdopted {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    let slot = current_preexpiry_slot(&proposal)?;
    let verification = load_verification(
        program_id,
        verification_info,
        proposal_info,
        buffer_info,
        authority_info,
        config_info,
        &config,
        &proposal,
    )?;
    require_verification_expectation(
        &verification,
        instruction.expected_verification_status,
        &instruction.expected_verified_chunk_bitmap,
        instruction.expected_verified_chunk_count,
    )?;
    validate_sealed_buffer(buffer_info, authority_info.key, &config, &verification)?;
    if verification.status != BufferVerificationStatusV1::ReadyToFinalize {
        return Err(GovernanceError::InvalidStateTransition.into());
    }

    let buffer_data = buffer_info.try_borrow_data()?;
    let payload = buffer_data
        .get(LOADER_BUFFER_METADATA_LEN..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    let observed_sha256 = hashv(&[payload]).to_bytes();
    if observed_sha256 != proposal.artifact_sha256 {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    let next_verification = finalize_buffer_record(&verification, &observed_sha256, slot)?;
    let mut next_proposal = (*proposal).clone();
    next_proposal.state = ProposalStateV2::BufferVerified;
    validate_proposal_digest_v2(&next_proposal)?;
    let verification_bytes = encode_fixed_account(&next_verification, BufferVerificationV1::LEN)?;
    let proposal_bytes = encode_fixed_account(&next_proposal, UpgradeProposalV2::LEN)?;
    drop(buffer_data);
    commit_two_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV2::LEN,
        verification_info,
        &verification_bytes,
        BufferVerificationV1::LEN,
    )
}

fn load_config(
    program_id: &Pubkey,
    config_info: &AccountInfo<'_>,
) -> Result<Box<ControllerConfigV1>, ProgramError> {
    let config = load_fixed_controller_account::<ControllerConfigV1>(
        program_id,
        config_info,
        ControllerConfigV1::LEN,
    )?;
    config.validate_static()?;
    let expected_config = derive_controller_config_pda(program_id, &config.target_program);
    let expected_authority = derive_authority_pda(program_id, &config.target_program);
    let expected_gate = derive_gate_pda(program_id, &config.target_program);
    if expected_config != (*config_info.key, config.bump)
        || expected_authority.0 != config.authority_pda
        || expected_gate.0 != config.gate_pda
        || config.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || derive_upgradeable_programdata_address(&config.target_program).0
            != config.target_programdata
    {
        return Err(GovernanceError::InvalidPda.into());
    }
    Ok(config)
}

fn load_gate(
    program_id: &Pubkey,
    gate_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<ProtocolGateV1>, ProgramError> {
    let gate = load_fixed_controller_account::<ProtocolGateV1>(
        program_id,
        gate_info,
        ProtocolGateV1::LEN,
    )?;
    gate.validate_static()?;
    let expected = derive_gate_pda(program_id, &config.target_program);
    if expected != (*gate_info.key, gate.bump)
        || *gate_info.key != config.gate_pda
        || gate.controller_config != *config_info.key
        || gate.target_program != config.target_program
        || gate.target_programdata != config.target_programdata
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(gate)
}

fn load_proposal(
    program_id: &Pubkey,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
) -> Result<Box<UpgradeProposalV2>, ProgramError> {
    let proposal = load_fixed_controller_account::<UpgradeProposalV2>(
        program_id,
        proposal_info,
        UpgradeProposalV2::LEN,
    )?;
    validate_proposal_digest_v2(&proposal)?;
    let expected = derive_proposal_pda(program_id, &config.target_program, proposal.proposal_id);
    if expected != (*proposal_info.key, proposal.bump)
        || proposal.controller_program != *program_id
        || proposal.controller_config != *config_info.key
        || proposal.protocol_gate != config.gate_pda
        || proposal.cluster_domain != config.cluster_domain
        || proposal.target_program != config.target_program
        || proposal.target_programdata != config.target_programdata
        || proposal.upgradeable_loader != config.upgradeable_loader
        || proposal.authority_pda != config.authority_pda
        || proposal.canonical_spill_treasury != config.canonical_spill_treasury
        || proposal.policy_version != config.current_policy_version
        || proposal.proposal_id >= config.next_proposal_id
        || proposal.buffer_verification != derive_buffer_check_pda(program_id, proposal_info.key).0
        || proposal.programdata_verification
            != derive_programdata_check_pda(program_id, proposal_info.key).0
        || proposal.prestate_checkpoint
            != derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Prestate).0
        || proposal.required_poststate_checkpoint
            != derive_checkpoint_pda(program_id, proposal_info.key, CheckpointPhaseV1::Poststate).0
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    verify_exact_proposal_timing(&proposal, config)?;
    Ok(proposal)
}

fn validate_expectation_and_creation_gate(
    expected: &ProposalExpectationV2,
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
    gate: &ProtocolGateV1,
) -> ProgramResult {
    if expected.expected_proposal_digest != proposal.proposal_digest
        || expected.expected_policy_version != proposal.policy_version
        || expected.expected_policy_hash != proposal.policy_hash
        || expected.expected_council_version != proposal.creation_council_version
        || expected.expected_council_hash != proposal.creation_council_hash
        || expected.expected_gate_status != gate.status
        || expected.expected_gate_epoch != gate.epoch
        || expected.expected_target_nonce != config.target_nonce
        || expected.expected_state != proposal.state
        || expected.expected_review_start_slot != proposal.review_start_slot
        || expected.expected_review_end_slot != proposal.review_end_slot
        || expected.expected_not_before_slot != proposal.not_before_slot
        || expected.expected_expiry_slot != proposal.expiry_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if proposal.target_nonce != config.target_nonce
        || proposal.creation_gate_status != gate.status
        || proposal.creation_gate_epoch != gate.epoch
        || gate.status == GateStatusV1::FrozenForUpgrade
        || gate.status == GateStatusV1::EmergencyFrozen
            && gate.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
    {
        return Err(GovernanceError::InvalidProposalEpoch.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn load_verification(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    buffer_info: &AccountInfo<'_>,
    authority_info: &AccountInfo<'_>,
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
    let expected = derive_buffer_check_pda(program_id, proposal_info.key);
    if expected != (*verification_info.key, verification.bump)
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
        || verification.chunk_hash_domain != proposal.chunk_hash_domain
        || verification.chunk_size != proposal.chunk_size
        || verification.chunk_count != proposal.chunk_count
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(verification)
}

fn validate_sealed_buffer(
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

fn require_verification_expectation(
    verification: &BufferVerificationV1,
    expected_status: BufferVerificationStatusV1,
    expected_bitmap: &[u8; VERIFICATION_BITMAP_BYTES_V1],
    expected_count: u32,
) -> ProgramResult {
    if verification.status != expected_status
        || verification.verified_chunk_bitmap != *expected_bitmap
        || verification.verified_chunk_count != expected_count
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
    let start = u64::from(chunk_index)
        .checked_mul(u64::from(chunk_size))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let remaining = artifact_length
        .checked_sub(start)
        .ok_or(GovernanceError::InvalidMerkleProof)?;
    let length = remaining.min(u64::from(chunk_size));
    if length == 0 {
        return Err(GovernanceError::InvalidMerkleProof.into());
    }
    let end = start
        .checked_add(length)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let start = usize::try_from(start).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let end = usize::try_from(end).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    payload
        .get(start..end)
        .ok_or(GovernanceError::InvalidMerkleProof.into())
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

fn finalize_buffer_record(
    verification: &BufferVerificationV1,
    observed_sha256: &[u8; 32],
    slot: u64,
) -> Result<BufferVerificationV1, ProgramError> {
    if verification.status != BufferVerificationStatusV1::ReadyToFinalize {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    if observed_sha256 != &verification.artifact_sha256 {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    if slot == 0 || slot < verification.adopted_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let mut next = verification.clone();
    next.status = BufferVerificationStatusV1::Verified;
    next.finalized_slot = slot;
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

fn current_preexpiry_slot(proposal: &UpgradeProposalV2) -> Result<u64, ProgramError> {
    let slot = Clock::get()?.slot;
    if slot == 0 || slot >= proposal.expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(slot)
}

fn derive_exact_timing(
    config: &ControllerConfigV1,
    class: ProposalClassV1,
    creation_slot: u64,
) -> Result<(u64, u64, u64, u64), ProgramError> {
    let delay = match class {
        ProposalClassV1::EmergencyRollback => config.rollback_delay_slots,
        ProposalClassV1::RoutineUpgrade => config.routine_delay_slots,
        ProposalClassV1::EconomicChange | ProposalClassV1::ConstitutionalChange => {
            config.major_delay_slots
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    };
    let review_start = creation_slot
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let review_end = review_start
        .checked_add(config.council_review_slots())
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let not_before = review_end
        .checked_add(delay)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expiry = creation_slot
        .checked_add(config.proposal_expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if not_before >= expiry {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok((review_start, review_end, not_before, expiry))
}

fn verify_exact_proposal_timing(
    proposal: &UpgradeProposalV2,
    config: &ControllerConfigV1,
) -> ProgramResult {
    if derive_exact_timing(config, proposal.proposal_class, proposal.creation_slot)?
        != (
            proposal.review_start_slot,
            proposal.review_end_slot,
            proposal.not_before_slot,
            proposal.expiry_slot,
        )
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}

fn commit_one_fixed_account(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    bytes: &[u8],
    expected_len: usize,
) -> ProgramResult {
    if account.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut data = account.try_borrow_mut_data()?;
    if data.len() != expected_len || bytes.len() != expected_len {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    data.copy_from_slice(bytes);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn commit_two_fixed_accounts(
    program_id: &Pubkey,
    first: &AccountInfo<'_>,
    first_bytes: &[u8],
    first_len: usize,
    second: &AccountInfo<'_>,
    second_bytes: &[u8],
    second_len: usize,
) -> ProgramResult {
    if first.owner != program_id || second.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut first_data = first.try_borrow_mut_data()?;
    let mut second_data = second.try_borrow_mut_data()?;
    if first_data.len() != first_len
        || second_data.len() != second_len
        || first_bytes.len() != first_len
        || second_bytes.len() != second_len
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    first_data.copy_from_slice(first_bytes);
    second_data.copy_from_slice(second_bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact_merkle::{
        artifact_chunk_count, artifact_merkle_proof, artifact_merkle_root,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    };

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn adopted_verification(artifact: &[u8]) -> BufferVerificationV1 {
        let chunk_size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1;
        BufferVerificationV1 {
            discriminator: BUFFER_VERIFICATION_V1_DISCRIMINATOR,
            account_version: RELEASE1_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            status: BufferVerificationStatusV1::Adopted,
            controller_config: key(1),
            proposal: key(2),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            buffer: key(3),
            expected_uploader_authority: key(4),
            controller_authority: key(5),
            artifact_length: artifact.len() as u64,
            artifact_sha256: hashv(&[artifact]).to_bytes(),
            artifact_chunk_merkle_root: artifact_merkle_root(artifact, chunk_size).unwrap(),
            chunk_hash_domain: crate::artifact_merkle::ARTIFACT_MERKLE_SCHEME_ID,
            chunk_size,
            chunk_count: artifact_chunk_count(artifact.len() as u64, chunk_size).unwrap(),
            verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
            verified_chunk_count: 0,
            adopted_slot: 10,
            finalized_slot: 0,
            sealed_buffer_header_hash: [9; 32],
            terminal_slot: 0,
            reserved: [0; BUFFER_VERIFICATION_V1_RESERVED_LEN],
        }
    }

    fn proof(artifact: &[u8], index: u32) -> Vec<[u8; 32]> {
        artifact_merkle_proof(artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, index).unwrap()
    }

    #[test]
    fn first_middle_and_final_partial_chunks_bind_exact_bytes() {
        let chunk = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
        let artifact = (0..chunk * 2 + 123)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let mut verification = adopted_verification(&artifact);
        let original_buffer = artifact.clone();

        for index in [0u32, 1, 2] {
            let exact = exact_artifact_chunk(
                &artifact,
                artifact.len() as u64,
                RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
                index,
            )
            .unwrap();
            verification =
                verify_and_mark_chunk(&verification, index, exact, &proof(&artifact, index))
                    .unwrap();
        }
        assert_eq!(
            artifact, original_buffer,
            "verification has no buffer write path"
        );
        assert_eq!(verification.verified_chunk_count, 3);
        assert_eq!(
            verification.status,
            BufferVerificationStatusV1::ReadyToFinalize
        );
        let finalized = finalize_buffer_record(
            &verification,
            &hashv(&[&artifact]).to_bytes(),
            verification.adopted_slot + 1,
        )
        .unwrap();
        assert_eq!(finalized.status, BufferVerificationStatusV1::Verified);
        assert_eq!(
            verification.status,
            BufferVerificationStatusV1::ReadyToFinalize
        );
        assert_eq!(
            exact_artifact_chunk(
                &artifact,
                artifact.len() as u64,
                RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
                2,
            )
            .unwrap()
            .len(),
            123
        );
    }

    #[test]
    fn duplicate_wrong_proof_and_wrong_length_are_failure_atomic() {
        let chunk = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
        let artifact = vec![7; chunk + 17];
        let base = adopted_verification(&artifact);
        let first = exact_artifact_chunk(
            &artifact,
            artifact.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            0,
        )
        .unwrap();
        let after_first = verify_and_mark_chunk(&base, 0, first, &proof(&artifact, 0)).unwrap();

        let snapshot = after_first.clone();
        assert!(verify_and_mark_chunk(&after_first, 0, first, &proof(&artifact, 0)).is_err());
        assert_eq!(after_first, snapshot);

        let mut wrong_proof = proof(&artifact, 1);
        wrong_proof[0][0] ^= 1;
        let final_chunk = &artifact[chunk..];
        assert!(verify_and_mark_chunk(&after_first, 1, final_chunk, &wrong_proof).is_err());
        assert_eq!(after_first, snapshot);

        assert!(verify_and_mark_chunk(
            &after_first,
            1,
            &artifact[chunk..artifact.len() - 1],
            &proof(&artifact, 1),
        )
        .is_err());
        assert_eq!(after_first, snapshot);
    }

    #[test]
    fn missing_chunk_and_bitmap_count_mismatch_cannot_finalize() {
        let chunk = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
        let artifact = vec![11; chunk + 9];
        let mut verification = adopted_verification(&artifact);
        let first = &artifact[..chunk];
        verification =
            verify_and_mark_chunk(&verification, 0, first, &proof(&artifact, 0)).unwrap();
        let snapshot = verification.clone();
        assert_ne!(
            verification.status,
            BufferVerificationStatusV1::ReadyToFinalize
        );
        assert!(verification.validate_schema().is_ok());
        assert!(finalize_buffer_record(
            &verification,
            &hashv(&[&artifact]).to_bytes(),
            verification.adopted_slot + 1,
        )
        .is_err());
        assert_eq!(verification, snapshot);

        let mut malformed = verification.clone();
        malformed.verified_chunk_count += 1;
        assert_eq!(
            malformed.validate_schema(),
            Err(GovernanceError::InvalidRelease1Bitmap)
        );
        assert_eq!(verification, snapshot);
    }

    #[test]
    fn sealed_header_is_exact_loader_v3_encoding() {
        let authority = key(88);
        let header = canonical_buffer_header(&authority);
        assert_eq!(&header[..4], &1u32.to_le_bytes());
        assert_eq!(header[4], 1);
        assert_eq!(&header[5..], authority.as_ref());
        let mut changed = header;
        changed[36] ^= 1;
        assert_ne!(hashv(&[&header]).to_bytes(), hashv(&[&changed]).to_bytes());
    }

    #[test]
    fn adoption_cpi_is_the_fixed_checked_loader_instruction() {
        let buffer = key(90);
        let uploader = key(91);
        let controller = key(92);
        let instruction = set_buffer_authority_checked(&buffer, &uploader, &controller);
        assert_eq!(instruction.program_id, UPGRADEABLE_LOADER_ID);
        assert_eq!(instruction.accounts.len(), 3);
        assert_eq!(instruction.accounts[0].pubkey, buffer);
        assert!(instruction.accounts[0].is_writable);
        assert!(!instruction.accounts[0].is_signer);
        assert_eq!(instruction.accounts[1].pubkey, uploader);
        assert!(!instruction.accounts[1].is_writable);
        assert!(instruction.accounts[1].is_signer);
        assert_eq!(instruction.accounts[2].pubkey, controller);
        assert!(!instruction.accounts[2].is_writable);
        assert!(instruction.accounts[2].is_signer);
    }
}
