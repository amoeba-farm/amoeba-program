use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_reciprocal_counterpart(
    program_id: &Pubkey,
    counterpart_info: &AccountInfo<'_>,
    counterpart_verification_info: &AccountInfo<'_>,
    config_info: &AccountInfo<'_>,
    config: &ControllerConfigV1,
    proposal: &UpgradeProposalV2,
    proposal_key: &Pubkey,
    instruction: &ExecuteUpgradeV1,
    slot: u64,
) -> ProgramResult {
    let counterpart = load_proposal(program_id, counterpart_info, config_info, config)?;
    if counterpart.proposal_digest != instruction.expected_counterpart_proposal_digest {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_reciprocal_checkpoint_commitments(proposal, &counterpart)?;
    let verification = load_fixed_controller_account::<BufferVerificationV1>(
        program_id,
        counterpart_verification_info,
        BufferVerificationV1::LEN,
    )?;
    verification.validate_schema()?;
    if derive_buffer_check_pda(program_id, counterpart_info.key)
        != (*counterpart_verification_info.key, verification.bump)
        || verification.controller_config != *config_info.key
        || verification.proposal != *counterpart_info.key
        || verification.buffer != counterpart.buffer_pubkey
        || verification.controller_authority != config.authority_pda
        || verification.upgradeable_loader != config.upgradeable_loader
        || verification.artifact_length != counterpart.artifact_length
        || verification.artifact_sha256 != counterpart.artifact_sha256
        || verification.artifact_chunk_merkle_root != counterpart.artifact_chunk_merkle_root
        || verification.status != instruction.expected_counterpart_buffer_verification_status
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    match proposal.proposal_class {
        ProposalClassV1::EmergencyRollback => {
            if !proposal.primary_proposal.present
                || proposal.primary_proposal.value != *counterpart_info.key
                || counterpart.rollback_proposal.value != *proposal_key
                || counterpart.proposal_class == ProposalClassV1::EmergencyRollback
                || verification.status != BufferVerificationStatusV1::ConsumedByUpgrade
                || !matches!(
                    counterpart.state,
                    ProposalStateV2::UpgradeExecuted
                        | ProposalStateV2::ProgramDataVerified
                        | ProposalStateV2::PoststateAccepted
                )
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
        }
        ProposalClassV1::RoutineUpgrade
        | ProposalClassV1::EconomicChange
        | ProposalClassV1::ConstitutionalChange => {
            require_primary_execution_rollback_runway(
                slot,
                config.rollback_delay_slots,
                config.council_review_slots(),
                counterpart.expiry_slot,
            )?;
            if !proposal.rollback_proposal.present
                || proposal.rollback_proposal.value != *counterpart_info.key
                || counterpart.primary_proposal.value != *proposal_key
                || counterpart.proposal_class != ProposalClassV1::EmergencyRollback
                || counterpart.state != ProposalStateV2::Timelocked
                || counterpart.not_before_slot > slot
                || counterpart.expiry_slot <= slot
                || verification.status != BufferVerificationStatusV1::Verified
                || proposal.rollback_buffer.value != counterpart.buffer_pubkey
                || proposal.rollback_artifact_sha256 != counterpart.artifact_sha256
                || proposal.rollback_artifact_chunk_root != counterpart.artifact_chunk_merkle_root
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    }
    Ok(())
}

pub(super) fn validate_reciprocal_checkpoint_commitments(
    proposal: &UpgradeProposalV2,
    counterpart: &UpgradeProposalV2,
) -> ProgramResult {
    if counterpart.checkpoint_schema_id != proposal.checkpoint_schema_id
        || counterpart.checkpoint_policy_hash != proposal.checkpoint_policy_hash
    {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    Ok(())
}

pub(super) fn require_primary_execution_rollback_runway(
    slot: u64,
    rollback_delay_slots: u64,
    council_review_slots: u64,
    rollback_expiry_slot: u64,
) -> ProgramResult {
    let recovery_horizon = slot
        .checked_add(rollback_delay_slots)
        .and_then(|value| value.checked_add(council_review_slots))
        .and_then(|value| value.checked_add(1))
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if recovery_horizon >= rollback_expiry_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    Ok(())
}
