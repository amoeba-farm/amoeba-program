use super::*;

pub(super) fn load_buffer_verification(
    program_id: &Pubkey,
    verification_info: &AccountInfo<'_>,
    proposal_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    buffer_info: &AccountInfo<'_>,
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
        || verification.controller_config != *config_info.key
        || verification.proposal != *proposal_info.key
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.buffer != *buffer_info.key
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

pub(super) fn validate_live_sealed_buffer(
    buffer_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    verification: &BufferVerificationV1,
) -> ProgramResult {
    if buffer_info.owner != &config.upgradeable_loader || buffer_info.executable {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let data = buffer_info.try_borrow_data()?;
    let header = parse_upgradeable_buffer(&data)?;
    let header_hash = hashv(&[data
        .get(..LOADER_BUFFER_METADATA_LEN)
        .ok_or(GovernanceError::InvalidRelease1Account)?])
    .to_bytes();
    if header.authority != Some(config.authority_pda)
        || u64::try_from(header.payload_length).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != proposal.artifact_length
        || header_hash != verification.sealed_buffer_header_hash
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    Ok(())
}
