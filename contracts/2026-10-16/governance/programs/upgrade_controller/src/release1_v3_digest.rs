//! Domain-separated fixed-image digests for capacity-safe Release 1 accounts.
//!
//! The published V1/V2 digest contracts remain unchanged.  These functions
//! operate only on the explicit V3/V2 account versions in `release1_v3_state`.
//! Every preimage is the account's exact fixed-width Borsh image after clearing
//! its stored digest and, where required, its mutable lifecycle accumulators.

use borsh::BorshSerialize;
use solana_program::hash::hashv;

use crate::{
    release1_state::{EmergencyFreezeResolutionStateV1, ProposalStateV2},
    release1_v3_state::{
        EmergencyFreezeObservationV2, EmergencyFreezeResolutionV2, ProgramDataFailureObservationV2,
        ProgramDataVerificationV2, StateCheckpointV2, UpgradeProposalV3,
        EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN,
        EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN,
        PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN,
        PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN, STATE_CHECKPOINT_V2_DIGEST_DOMAIN,
        UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN,
    },
    GovernanceError, GovernanceResult,
};

pub const UPGRADE_PROPOSAL_V3_DIGEST_IMAGE_LEN: usize = UpgradeProposalV3::LEN;
pub const PROGRAMDATA_VERIFICATION_V2_DIGEST_IMAGE_LEN: usize = ProgramDataVerificationV2::LEN;
pub const STATE_CHECKPOINT_V2_DIGEST_IMAGE_LEN: usize = StateCheckpointV2::LEN;
pub const EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_IMAGE_LEN: usize =
    EmergencyFreezeObservationV2::LEN;
pub const EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_IMAGE_LEN: usize = EmergencyFreezeResolutionV2::LEN;
pub const PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_IMAGE_LEN: usize =
    ProgramDataFailureObservationV2::LEN;

/// Computes the immutable proposal digest.  The proposal state, gate epoch
/// established at freeze, lifecycle slots, approval accumulators, cancellation
/// and terminal evidence are deliberately canonicalized.  All artifact,
/// capacity-policy, current-deployment, nonce, creation-epoch, timing, rollback,
/// and account-identity commitments remain in the image.
pub fn compute_upgrade_proposal_digest_v3(value: &UpgradeProposalV3) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    clear_upgrade_proposal_lifecycle_v3(&mut canonical);
    canonical.proposal_digest = [0; 32];
    hash_fixed_image(
        UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN,
        &canonical,
        UPGRADE_PROPOSAL_V3_DIGEST_IMAGE_LEN,
    )
}

pub fn validate_upgrade_proposal_digest_v3(value: &UpgradeProposalV3) -> GovernanceResult<()> {
    value.validate_schema()?;
    require_digest(
        compute_upgrade_proposal_digest_v3(value)?,
        value.proposal_digest,
    )
}

/// Verification generations form an explicit chain through
/// `verification_generation` and `previous_verification_digest`.  Those fields,
/// the exact ProgramData observation, status, and finalization evidence are all
/// hashed; only the stored digest itself is cleared.
pub fn compute_programdata_verification_digest_v2(
    value: &ProgramDataVerificationV2,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.verification_digest = [0; 32];
    hash_fixed_image(
        PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN,
        &canonical,
        PROGRAMDATA_VERIFICATION_V2_DIGEST_IMAGE_LEN,
    )
}

pub fn validate_programdata_verification_digest_v2(
    value: &ProgramDataVerificationV2,
) -> GovernanceResult<()> {
    value.validate_schema()?;
    require_digest(
        compute_programdata_verification_digest_v2(value)?,
        value.verification_digest,
    )
}

/// The checkpoint content, generation chain, selected council, observation,
/// capacity, roots, and donation accounting are immutable. The three-seat
/// approval accumulator, threshold-derived `accepted` flag, and permissionless
/// finalization slot are lifecycle evidence and are cleared so independent
/// seats can attest the same digest before the checkpoint account exists.
pub fn compute_state_checkpoint_digest_v2(value: &StateCheckpointV2) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.checkpoint_digest = [0; 32];
    canonical.approval_bitset = 0;
    canonical.approval_count = 0;
    canonical.accepted = false;
    canonical.finalized_slot = 0;
    hash_fixed_image(
        STATE_CHECKPOINT_V2_DIGEST_DOMAIN,
        &canonical,
        STATE_CHECKPOINT_V2_DIGEST_IMAGE_LEN,
    )
}

pub fn validate_state_checkpoint_digest_v2(value: &StateCheckpointV2) -> GovernanceResult<()> {
    value.validate_schema()?;
    require_digest(
        compute_state_checkpoint_digest_v2(value)?,
        value.checkpoint_digest,
    )
}

pub fn compute_emergency_freeze_observation_digest_v2(
    value: &EmergencyFreezeObservationV2,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.observation_digest = [0; 32];
    hash_fixed_image(
        EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN,
        &canonical,
        EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_IMAGE_LEN,
    )
}

pub fn validate_emergency_freeze_observation_digest_v2(
    value: &EmergencyFreezeObservationV2,
) -> GovernanceResult<()> {
    value.validate_schema()?;
    require_digest(
        compute_emergency_freeze_observation_digest_v2(value)?,
        value.observation_digest,
    )
}

/// Resolution identity is immutable across council approval, queueing,
/// execution, and expiry.  The selected council version/hash and threshold stay
/// committed; only the accumulator and terminal lifecycle are canonicalized.
pub fn compute_emergency_freeze_resolution_digest_v2(
    value: &EmergencyFreezeResolutionV2,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    clear_emergency_resolution_lifecycle_v2(&mut canonical);
    canonical.resolution_digest = [0; 32];
    hash_fixed_image(
        EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN,
        &canonical,
        EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_IMAGE_LEN,
    )
}

pub fn validate_emergency_freeze_resolution_digest_v2(
    value: &EmergencyFreezeResolutionV2,
) -> GovernanceResult<()> {
    value.validate_schema()?;
    require_digest(
        compute_emergency_freeze_resolution_digest_v2(value)?,
        value.resolution_digest,
    )
}

pub fn compute_programdata_failure_observation_digest_v2(
    value: &ProgramDataFailureObservationV2,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.failure_digest = [0; 32];
    hash_fixed_image(
        PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN,
        &canonical,
        PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_IMAGE_LEN,
    )
}

pub fn validate_programdata_failure_observation_digest_v2(
    value: &ProgramDataFailureObservationV2,
) -> GovernanceResult<()> {
    value.validate_schema()?;
    require_digest(
        compute_programdata_failure_observation_digest_v2(value)?,
        value.failure_digest,
    )
}

fn clear_upgrade_proposal_lifecycle_v3(value: &mut UpgradeProposalV3) {
    value.state = ProposalStateV2::Draft;
    value.freeze_gate_epoch = 0;
    value.first_approval_slot = 0;
    value.council_approved_slot = 0;
    value.governance_satisfied_slot = 0;
    value.queued_slot = 0;
    value.frozen_slot = 0;
    value.extension_executed_slot = 0;
    value.upgrade_executed_slot = 0;
    value.programdata_verified_slot = 0;
    value.poststate_accepted_slot = 0;
    value.unfreeze_approved_slot = 0;
    value.terminal_slot = 0;
    value.council_approval_bitset = 0;
    value.council_approval_count = 0;
    value.cancellation_council_version = 0;
    value.cancellation_council_hash = [0; 32];
    value.cancellation_approval_bitset = 0;
    value.cancellation_approval_count = 0;
    value.unfreeze_council_version = 0;
    value.unfreeze_council_hash = [0; 32];
    value.unfreeze_approval_bitset = 0;
    value.unfreeze_approval_count = 0;
    value.cancellation_reason_code = 0;
    value.terminal_reason_code = 0;
}

fn clear_emergency_resolution_lifecycle_v2(value: &mut EmergencyFreezeResolutionV2) {
    value.state = EmergencyFreezeResolutionStateV1::Draft;
    // The canonical checkpoint PDA is committed at resolution creation, but
    // its digest can exist only after the checkpoint binds this resolution
    // digest. Treat the accepted checkpoint digest as lifecycle evidence so
    // the two fixed accounts do not form an impossible hash cycle.
    value.emergency_checkpoint_digest = [0; 32];
    value.approval_bitset = 0;
    value.approval_count = 0;
    value.first_approval_slot = 0;
    value.council_approved_slot = 0;
    value.queued_slot = 0;
    value.executed_slot = 0;
    value.terminal_slot = 0;
    value.terminal_reason_code = 0;
}

#[inline(never)]
fn hash_fixed_image<T: BorshSerialize>(
    domain: &[u8],
    value: &T,
    expected_image_len: usize,
) -> GovernanceResult<[u8; 32]> {
    // The on-chain allocator is a bump arena, so repeated fixed-account digest
    // checks cannot rely on dropped `Vec`s returning heap space.  Keep one
    // bounded image in this callee's stack frame; the largest V3 image is the
    // 2,048-byte proposal and every smaller account must consume its exact
    // published fixed length.
    let mut image = [0u8; UPGRADE_PROPOSAL_V3_DIGEST_IMAGE_LEN];
    if expected_image_len > image.len() {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    let remaining = {
        let mut output = &mut image[..expected_image_len];
        value
            .serialize(&mut output)
            .map_err(|_| GovernanceError::InvalidRelease1Account)?;
        output.len()
    };
    if remaining != 0 {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(hashv(&[domain, &image[..expected_image_len]]).to_bytes())
}

fn require_digest(actual: [u8; 32], expected: [u8; 32]) -> GovernanceResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(GovernanceError::Release1DigestMismatch)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use borsh::BorshDeserialize;

    use super::*;
    use crate::{
        release1_ceremony_state::ProgramDataObservationPurposeV1,
        release1_state::{ProposalStateV2, StateCheckpointPhaseV1},
        release1_v3_state::ProgramDataVerificationStatusV2,
    };

    fn zero_image<T: BorshDeserialize>(len: usize) -> T {
        T::try_from_slice(&vec![0; len]).expect("all-zero image has only zero-valued enums/bools")
    }

    #[test]
    fn digest_domains_are_unique_and_images_are_exact_account_lengths() {
        let domains = [
            UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN,
            PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN,
            STATE_CHECKPOINT_V2_DIGEST_DOMAIN,
            EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN,
            EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN,
            PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN,
        ];
        assert_eq!(
            domains.iter().copied().collect::<BTreeSet<_>>().len(),
            domains.len()
        );
        assert_eq!(UPGRADE_PROPOSAL_V3_DIGEST_IMAGE_LEN, 2_048);
        assert_eq!(PROGRAMDATA_VERIFICATION_V2_DIGEST_IMAGE_LEN, 1_024);
        assert_eq!(STATE_CHECKPOINT_V2_DIGEST_IMAGE_LEN, 1_280);
        assert_eq!(EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_IMAGE_LEN, 768);
        assert_eq!(EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_IMAGE_LEN, 1_152);
        assert_eq!(PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_IMAGE_LEN, 1_024);
    }

    #[test]
    fn proposal_digest_is_invariant_across_every_lifecycle_accumulator() {
        let mut value: UpgradeProposalV3 = zero_image(UpgradeProposalV3::LEN);
        value.proposal_digest = [1; 32];
        let expected = compute_upgrade_proposal_digest_v3(&value).unwrap();
        value.state = ProposalStateV2::Completed;
        value.freeze_gate_epoch = 7;
        value.first_approval_slot = 8;
        value.council_approved_slot = 9;
        value.governance_satisfied_slot = 10;
        value.queued_slot = 11;
        value.frozen_slot = 12;
        value.extension_executed_slot = 13;
        value.upgrade_executed_slot = 14;
        value.programdata_verified_slot = 15;
        value.poststate_accepted_slot = 16;
        value.unfreeze_approved_slot = 17;
        value.terminal_slot = 18;
        value.council_approval_bitset = 7;
        value.council_approval_count = 3;
        value.cancellation_council_version = 19;
        value.cancellation_council_hash = [20; 32];
        value.cancellation_approval_bitset = 7;
        value.cancellation_approval_count = 3;
        value.unfreeze_council_version = 21;
        value.unfreeze_council_hash = [22; 32];
        value.unfreeze_approval_bitset = 7;
        value.unfreeze_approval_count = 3;
        value.cancellation_reason_code = 23;
        value.terminal_reason_code = 24;
        value.proposal_digest = [25; 32];
        assert_eq!(
            compute_upgrade_proposal_digest_v3(&value).unwrap(),
            expected
        );

        value.minimum_required_capacity = 1;
        assert_ne!(
            compute_upgrade_proposal_digest_v3(&value).unwrap(),
            expected
        );
    }

    #[test]
    fn checkpoint_approvals_and_resolution_lifecycle_do_not_rekey_the_subject() {
        let mut checkpoint: StateCheckpointV2 = zero_image(StateCheckpointV2::LEN);
        checkpoint.phase = StateCheckpointPhaseV1::Poststate;
        checkpoint.observation_purpose = ProgramDataObservationPurposeV1::PostUpgrade;
        let checkpoint_digest = compute_state_checkpoint_digest_v2(&checkpoint).unwrap();
        checkpoint.approval_bitset = 7;
        checkpoint.approval_count = 3;
        checkpoint.accepted = true;
        checkpoint.finalized_slot = 77;
        checkpoint.checkpoint_digest = [9; 32];
        assert_eq!(
            compute_state_checkpoint_digest_v2(&checkpoint).unwrap(),
            checkpoint_digest
        );
        checkpoint.checkpoint_generation = 1;
        assert_ne!(
            compute_state_checkpoint_digest_v2(&checkpoint).unwrap(),
            checkpoint_digest
        );

        let mut resolution: EmergencyFreezeResolutionV2 =
            zero_image(EmergencyFreezeResolutionV2::LEN);
        let resolution_digest = compute_emergency_freeze_resolution_digest_v2(&resolution).unwrap();
        resolution.state = EmergencyFreezeResolutionStateV1::Executed;
        resolution.approval_bitset = 7;
        resolution.approval_count = 3;
        resolution.first_approval_slot = 1;
        resolution.council_approved_slot = 2;
        resolution.queued_slot = 3;
        resolution.executed_slot = 4;
        resolution.terminal_slot = 4;
        resolution.terminal_reason_code = 5;
        resolution.emergency_checkpoint_digest = [7; 32];
        resolution.resolution_digest = [6; 32];
        assert_eq!(
            compute_emergency_freeze_resolution_digest_v2(&resolution).unwrap(),
            resolution_digest
        );
        resolution.approval_council_version = 1;
        assert_ne!(
            compute_emergency_freeze_resolution_digest_v2(&resolution).unwrap(),
            resolution_digest
        );
    }

    #[test]
    fn generation_chain_and_finalization_evidence_are_digest_bound() {
        let mut verification: ProgramDataVerificationV2 =
            zero_image(ProgramDataVerificationV2::LEN);
        let first = compute_programdata_verification_digest_v2(&verification).unwrap();
        verification.verification_digest = [1; 32];
        assert_eq!(
            compute_programdata_verification_digest_v2(&verification).unwrap(),
            first
        );
        verification.verification_generation = 2;
        verification.previous_verification_digest = first;
        assert_ne!(
            compute_programdata_verification_digest_v2(&verification).unwrap(),
            first
        );
        let second = compute_programdata_verification_digest_v2(&verification).unwrap();
        verification.status = ProgramDataVerificationStatusV2::Verified;
        verification.zero_tail_verified = true;
        verification.finalized_slot = 3;
        assert_ne!(
            compute_programdata_verification_digest_v2(&verification).unwrap(),
            second
        );

        let mut observation: EmergencyFreezeObservationV2 =
            zero_image(EmergencyFreezeObservationV2::LEN);
        let initial = compute_emergency_freeze_observation_digest_v2(&observation).unwrap();
        observation.observation_digest = [4; 32];
        assert_eq!(
            compute_emergency_freeze_observation_digest_v2(&observation).unwrap(),
            initial
        );
        observation.actual_capacity = 5;
        assert_ne!(
            compute_emergency_freeze_observation_digest_v2(&observation).unwrap(),
            initial
        );

        let mut failure: ProgramDataFailureObservationV2 =
            zero_image(ProgramDataFailureObservationV2::LEN);
        let failure_digest = compute_programdata_failure_observation_digest_v2(&failure).unwrap();
        failure.failure_digest = [6; 32];
        assert_eq!(
            compute_programdata_failure_observation_digest_v2(&failure).unwrap(),
            failure_digest
        );
        failure.observation_generation = 1;
        assert_ne!(
            compute_programdata_failure_observation_digest_v2(&failure).unwrap(),
            failure_digest
        );
    }
}
