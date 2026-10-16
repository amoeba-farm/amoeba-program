use super::*;

pub(super) fn expected_unextended_raw_programdata_hash(
    proposal: &UpgradeProposalV2,
    prestate: &StateCheckpointV1,
) -> Result<[u8; 32], ProgramError> {
    let expected = match proposal.proposal_class {
        ProposalClassV1::EmergencyRollback => prestate.target_raw_programdata_commitment,
        ProposalClassV1::RoutineUpgrade
        | ProposalClassV1::EconomicChange
        | ProposalClassV1::ConstitutionalChange => {
            if prestate.target_raw_programdata_commitment != proposal.current_raw_programdata_hash {
                return Err(GovernanceError::CrossAccountMismatch.into());
            }
            proposal.current_raw_programdata_hash
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass.into());
        }
    };
    if expected == [0; 32] {
        return Err(GovernanceError::InvalidProposalCommitment.into());
    }
    Ok(expected)
}

/// Performs the sole maximum-size SHA-256 pass admitted in an extension or
/// upgrade transaction. Header/linkage/capacity are validated separately so a
/// caller cannot turn a differently shaped account into the hashed subject.
pub(super) fn require_raw_programdata_hash(
    target_programdata: &AccountInfo<'_>,
    expected_raw_hash: &[u8; 32],
) -> ProgramResult {
    let data = target_programdata.try_borrow_data()?;
    if u64::try_from(data.len()).map_err(|_| GovernanceError::ArithmeticOverflow)?
        > MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
    {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    if loader_account_data_hash(&data) != *expected_raw_hash {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    Ok(())
}

/// Checked extension preserves the existing prefix by Loader-v3 contract. The
/// controller re-reads only the appended region here and requires every byte
/// of the exact delta to be zero, avoiding a second full-account hash pass.
pub(super) fn validate_zero_appended_extension(
    target_programdata: &AccountInfo<'_>,
    payload_offset: usize,
    previous_capacity: u64,
    extension_delta: u64,
) -> ProgramResult {
    let previous_capacity =
        usize::try_from(previous_capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let extension_delta =
        usize::try_from(extension_delta).map_err(|_| GovernanceError::ArithmeticOverflow)?;
    let data = target_programdata.try_borrow_data()?;
    let payload = data
        .get(payload_offset..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    let expected_capacity = previous_capacity
        .checked_add(extension_delta)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if payload.len() != expected_capacity {
        return Err(GovernanceError::InvalidCapacityPlan.into());
    }
    let appended = payload
        .get(previous_capacity..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    if appended.len() != extension_delta || appended.iter().any(|byte| *byte != 0) {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    Ok(())
}
