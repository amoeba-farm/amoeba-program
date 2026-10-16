use super::*;

pub(super) fn terminalize_unfreeze_pair_v3(
    proposal: &UpgradeProposalV3,
    proposal_key: &Pubkey,
    linked: &mut UpgradeProposalV3,
    linked_key: &Pubkey,
    slot: u64,
) -> ProgramResult {
    match proposal.proposal_class {
        ProposalClassV1::EmergencyRollback => {
            if !proposal.primary_proposal.present
                || proposal.primary_proposal.value != *linked_key
                || linked.proposal_class == ProposalClassV1::EmergencyRollback
                || !linked.rollback_proposal.present
                || linked.rollback_proposal.value != *proposal_key
                || !linked.rollback_buffer.present
                || linked.rollback_buffer.value != proposal.buffer_pubkey
                || linked.rollback_artifact_length != proposal.artifact_length
                || linked.rollback_artifact_sha256 != proposal.artifact_sha256
                || linked.rollback_artifact_chunk_root != proposal.artifact_chunk_merkle_root
                || linked.rollback_artifact_scheme_id != proposal.artifact_scheme_id
                || linked.target_nonce != proposal.target_nonce
                || linked.checkpoint_schema_id != proposal.checkpoint_schema_id
                || linked.checkpoint_policy_hash != proposal.checkpoint_policy_hash
                || !matches!(
                    linked.state,
                    ProposalStateV2::UpgradeExecuted | ProposalStateV2::ProgramDataVerified
                )
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
            linked.state = ProposalStateV2::SupersededByRollback;
            linked.terminal_slot = slot;
            linked.terminal_reason_code = PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1;
        }
        ProposalClassV1::RoutineUpgrade
        | ProposalClassV1::EconomicChange
        | ProposalClassV1::ConstitutionalChange => {
            if !proposal.rollback_proposal.present
                || proposal.rollback_proposal.value != *linked_key
                || !proposal.rollback_buffer.present
                || proposal.rollback_buffer.value != linked.buffer_pubkey
                || proposal.rollback_artifact_length != linked.artifact_length
                || proposal.rollback_artifact_sha256 != linked.artifact_sha256
                || proposal.rollback_artifact_chunk_root != linked.artifact_chunk_merkle_root
                || proposal.rollback_artifact_scheme_id != linked.artifact_scheme_id
                || linked.proposal_class != ProposalClassV1::EmergencyRollback
                || !linked.primary_proposal.present
                || linked.primary_proposal.value != *proposal_key
                || linked.rollback_proposal.present
                || linked.rollback_buffer.present
                || linked.target_nonce != proposal.target_nonce
                || linked.checkpoint_schema_id != proposal.checkpoint_schema_id
                || linked.checkpoint_policy_hash != proposal.checkpoint_policy_hash
                || linked.state != ProposalStateV2::Timelocked
            {
                return Err(GovernanceError::InvalidProposalCommitment.into());
            }
            linked.state = ProposalStateV2::Retired;
            linked.terminal_slot = slot;
            linked.terminal_reason_code = PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1;
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    }
    Ok(())
}

pub(super) fn apply_activated_deployment_v2(
    current: &CurrentDeploymentStateV1,
    evidence: &ActivatedDeploymentEvidenceV2,
    active_gate_epoch: u64,
    slot: u64,
) -> GovernanceResult<CurrentDeploymentStateV1> {
    if active_gate_epoch == 0 || active_gate_epoch == u64::MAX || slot == 0 {
        return Err(GovernanceError::CrossAccountMismatch);
    }
    let mut next = current.clone();
    next.artifact_length = evidence.artifact_length;
    next.artifact_sha256 = evidence.artifact_sha256;
    next.artifact_merkle_root = evidence.artifact_merkle_root;
    next.artifact_scheme_id = evidence.artifact_scheme_id;
    next.actual_programdata_capacity = evidence.actual_capacity;
    next.programdata_observation = evidence.programdata_observation;
    next.observation_generation = evidence.observation_generation;
    next.observation_root = evidence.observation_root;
    next.observation_digest = evidence.observation_digest;
    next.deployed_slot = evidence.deployed_slot;
    next.installed_authority = evidence.installed_authority;
    next.source_commitment = evidence.source_commitment;
    next.build_inputs_commitment = evidence.build_inputs_commitment;
    next.package_commitment = evidence.package_commitment;
    next.release_manifest_commitment = evidence.release_manifest_commitment;
    next.release_commitment = evidence.release_commitment;
    next.release_commitment_digest = evidence.release_commitment_digest;
    next.activation_receipt = OptionalPubkeyV1::none();
    next.completed_proposal = OptionalPubkeyV1::some(evidence.release_commitment)?;
    next.gate_epoch_at_activation = active_gate_epoch;
    next.deployment_generation = checked_nonterminal_increment(current.deployment_generation)?;
    next.last_updated_slot = slot;
    next.deployment_digest = [0; 32];
    next.deployment_digest = compute_current_deployment_digest_v1(&next)?;
    validate_current_deployment_digest_v1(&next)?;
    Ok(next)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_activated_deployment_v2(
    current: &CurrentDeploymentStateV1,
    config: &ControllerConfigV1,
    proposal_key: &Pubkey,
    proposal: &UpgradeProposalV3,
    verification: &ProgramDataVerificationV2,
    active_gate_epoch: u64,
    slot: u64,
) -> GovernanceResult<CurrentDeploymentStateV1> {
    if active_gate_epoch == 0
        || active_gate_epoch == u64::MAX
        || slot == 0
        || verification.status != ProgramDataVerificationStatusV2::Verified
        || !verification.zero_tail_verified
        || verification.finalized_slot == 0
        || verification.finalized_slot > slot
        || verification.proposal != *proposal_key
        || verification.proposal_digest != proposal.proposal_digest
        || verification.artifact_length != proposal.artifact_length
        || verification.artifact_sha256 != proposal.artifact_sha256
        || verification.artifact_merkle_root != proposal.artifact_chunk_merkle_root
        || verification.artifact_scheme_id != proposal.artifact_scheme_id
        || verification.observed_authority != OptionalPubkeyV1::some(config.authority_pda)?
    {
        return Err(GovernanceError::CrossAccountMismatch);
    }
    let evidence = ActivatedDeploymentEvidenceV2 {
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_merkle_root: proposal.artifact_chunk_merkle_root,
        artifact_scheme_id: proposal.artifact_scheme_id,
        actual_capacity: verification.actual_capacity,
        programdata_observation: verification.programdata_observation,
        observation_generation: verification.observation_generation,
        observation_root: verification.observation_root,
        observation_digest: verification.observation_digest,
        deployed_slot: verification.observed_deployed_slot,
        installed_authority: config.authority_pda,
        source_commitment: proposal.source_commit_hash,
        build_inputs_commitment: proposal.build_input_inventory_hash,
        package_commitment: proposal.package_receipt_hash,
        release_manifest_commitment: proposal.release_intent_hash,
        release_commitment: *proposal_key,
        release_commitment_digest: proposal.proposal_digest,
    };
    apply_activated_deployment_v2(current, &evidence, active_gate_epoch, slot)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_four_fixed_accounts(
    program_id: &Pubkey,
    first: &AccountInfo<'_>,
    first_bytes: &[u8],
    first_len: usize,
    second: &AccountInfo<'_>,
    second_bytes: &[u8],
    second_len: usize,
    third: &AccountInfo<'_>,
    third_bytes: &[u8],
    third_len: usize,
    fourth: &AccountInfo<'_>,
    fourth_bytes: &[u8],
    fourth_len: usize,
) -> ProgramResult {
    if first.owner != program_id
        || second.owner != program_id
        || third.owner != program_id
        || fourth.owner != program_id
    {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let mut first_data = first.try_borrow_mut_data()?;
    let mut second_data = second.try_borrow_mut_data()?;
    let mut third_data = third.try_borrow_mut_data()?;
    let mut fourth_data = fourth.try_borrow_mut_data()?;
    if first_data.len() != first_len
        || second_data.len() != second_len
        || third_data.len() != third_len
        || fourth_data.len() != fourth_len
        || first_bytes.len() != first_len
        || second_bytes.len() != second_len
        || third_bytes.len() != third_len
        || fourth_bytes.len() != fourth_len
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    first_data.copy_from_slice(first_bytes);
    second_data.copy_from_slice(second_bytes);
    third_data.copy_from_slice(third_bytes);
    fourth_data.copy_from_slice(fourth_bytes);
    Ok(())
}
