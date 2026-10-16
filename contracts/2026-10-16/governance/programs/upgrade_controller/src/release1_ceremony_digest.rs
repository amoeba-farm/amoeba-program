//! Domain-separated digests for Release 1 ceremony-closure accounts.
//!
//! Published V1/V2 digest meanings remain untouched. Ceremony digests hash a
//! fixed-width Borsh image after clearing only the digest itself. Governed
//! ceremony proposals additionally clear their lifecycle accumulator so the
//! immutable proposal digest survives approvals, queueing, and execution.

use borsh::BorshSerialize;
use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    release1_ceremony_state::{
        BootstrapActivationProposalV1, BootstrapActivationReceiptV1, CeremonyProposalStateV1,
        ControllerImmutabilityReceiptV1, ControllerReleaseCommitmentV1, CurrentDeploymentStateV1,
        ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1, ProgramDataObservationV1,
        TargetAuthorityHandoffProposalV1, TargetAuthorityHandoffReceiptV1,
    },
    state::GateStatusV1,
    GovernanceError, GovernanceResult,
};

pub const CAPACITY_POLICY_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_PROGRAMDATA_CAPACITY_POLICY_V1";
pub const CONTROLLER_RELEASE_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_CONTROLLER_RELEASE_COMMITMENT_V1";
pub const PROGRAMDATA_OBSERVATION_SUBJECT_DOMAIN_V1: &[u8] =
    b"AMOEBA_PROGRAMDATA_OBSERVATION_SUBJECT_V1";
pub const PROGRAMDATA_OBSERVATION_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_PROGRAMDATA_OBSERVATION_V1";
pub const CURRENT_DEPLOYMENT_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_CURRENT_DEPLOYMENT_STATE_V1";
pub const CONTROLLER_IMMUTABILITY_RECEIPT_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_CONTROLLER_IMMUTABILITY_RECEIPT_V1";
pub const TARGET_HANDOFF_PROPOSAL_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1";
pub const TARGET_HANDOFF_RECEIPT_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_TARGET_AUTHORITY_HANDOFF_RECEIPT_V1";
pub const BOOTSTRAP_ACTIVATION_PROPOSAL_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_BOOTSTRAP_ACTIVATION_PROPOSAL_V1";
pub const BOOTSTRAP_ACTIVATION_RECEIPT_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_BOOTSTRAP_ACTIVATION_RECEIPT_V1";
pub const BOOTSTRAP_ACTIVATION_DEPLOYMENT_PLAN_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_BOOTSTRAP_ACTIVATION_DEPLOYMENT_PLAN_V1";
pub const BOOTSTRAP_ACTIVATION_RECEIPT_PLAN_DIGEST_DOMAIN_V1: &[u8] =
    b"AMOEBA_BOOTSTRAP_ACTIVATION_RECEIPT_PLAN_V1";

#[allow(clippy::too_many_arguments)]
pub fn compute_programdata_observation_subject_digest_v1(
    controller_program: &Pubkey,
    controller_config: &Pubkey,
    observed_program: &Pubkey,
    observed_programdata: &Pubkey,
    purpose: ProgramDataObservationPurposeV1,
    subject: &Pubkey,
    generation: u64,
    protocol_gate: &Pubkey,
    gate_status: GateStatusV1,
    gate_epoch: u64,
    gate_active_proposal: &Pubkey,
    gate_freeze_slot: u64,
    gate_freeze_reason_code: u16,
    capacity_policy_digest: &[u8; 32],
    artifact_length: u64,
    artifact_sha256: &[u8; 32],
    artifact_merkle_root: &[u8; 32],
    artifact_scheme_id: &[u8; 32],
    minimum_required_capacity: u64,
) -> GovernanceResult<[u8; 32]> {
    if *controller_program == Pubkey::default()
        || *controller_config == Pubkey::default()
        || *observed_program == Pubkey::default()
        || *observed_programdata == Pubkey::default()
        || *subject == Pubkey::default()
        || generation == 0
        || *protocol_gate == Pubkey::default()
        || gate_epoch == 0
        || *capacity_policy_digest == [0; 32]
        || artifact_length == 0
        || *artifact_sha256 == [0; 32]
        || *artifact_merkle_root == [0; 32]
        || *artifact_scheme_id == [0; 32]
        || minimum_required_capacity == 0
    {
        return Err(GovernanceError::InvalidProgramDataObservation);
    }
    Ok(hashv(&[
        PROGRAMDATA_OBSERVATION_SUBJECT_DOMAIN_V1,
        controller_program.as_ref(),
        controller_config.as_ref(),
        observed_program.as_ref(),
        observed_programdata.as_ref(),
        &[purpose as u8],
        subject.as_ref(),
        &generation.to_le_bytes(),
        protocol_gate.as_ref(),
        &[gate_status as u8],
        &gate_epoch.to_le_bytes(),
        gate_active_proposal.as_ref(),
        &gate_freeze_slot.to_le_bytes(),
        &gate_freeze_reason_code.to_le_bytes(),
        capacity_policy_digest,
        &artifact_length.to_le_bytes(),
        artifact_sha256,
        artifact_merkle_root,
        artifact_scheme_id,
        &minimum_required_capacity.to_le_bytes(),
    ])
    .to_bytes())
}

pub fn compute_capacity_policy_digest_v1(
    value: &ProgramDataCapacityPolicyV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.policy_digest = [0; 32];
    // The initialization transaction supplies the immutable policy identity,
    // while the controller records the actual execution slot on chain. The
    // digest therefore excludes chronology so a signer never has to predict a
    // future bank slot.
    canonical.creation_slot = 0;
    hash_fixed(CAPACITY_POLICY_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_capacity_policy_digest_v1(
    value: &ProgramDataCapacityPolicyV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_capacity_policy_digest_v1(value)?,
        value.policy_digest,
    )
}

pub fn compute_controller_release_digest_v1(
    value: &ControllerReleaseCommitmentV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.release_digest = [0; 32];
    canonical.creation_slot = 0;
    hash_fixed(CONTROLLER_RELEASE_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_controller_release_digest_v1(
    value: &ControllerReleaseCommitmentV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_controller_release_digest_v1(value)?,
        value.release_digest,
    )
}

pub fn compute_programdata_observation_digest_v1(
    value: &ProgramDataObservationV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.observation_digest = [0; 32];
    hash_fixed(PROGRAMDATA_OBSERVATION_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_programdata_observation_digest_v1(
    value: &ProgramDataObservationV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_programdata_observation_digest_v1(value)?,
        value.observation_digest,
    )
}

pub fn compute_current_deployment_digest_v1(
    value: &CurrentDeploymentStateV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.deployment_digest = [0; 32];
    hash_fixed(CURRENT_DEPLOYMENT_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_current_deployment_digest_v1(
    value: &CurrentDeploymentStateV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_current_deployment_digest_v1(value)?,
        value.deployment_digest,
    )
}

/// Commits the complete static deployment image without requiring an operator
/// to predict the bank slot in which activation will land. The controller
/// computes the ordinary deployment digest from the actual slot after this
/// plan commitment matches.
pub fn compute_bootstrap_activation_deployment_plan_digest_v1(
    value: &CurrentDeploymentStateV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.deployment_digest = [0; 32];
    canonical.last_updated_slot = 0;
    hash_fixed(
        BOOTSTRAP_ACTIVATION_DEPLOYMENT_PLAN_DIGEST_DOMAIN_V1,
        &canonical,
    )
}

pub fn compute_controller_immutability_receipt_digest_v1(
    value: &ControllerImmutabilityReceiptV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.receipt_digest = [0; 32];
    hash_fixed(CONTROLLER_IMMUTABILITY_RECEIPT_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_controller_immutability_receipt_digest_v1(
    value: &ControllerImmutabilityReceiptV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_controller_immutability_receipt_digest_v1(value)?,
        value.receipt_digest,
    )
}

pub fn compute_target_handoff_proposal_digest_v1(
    value: &TargetAuthorityHandoffProposalV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    clear_ceremony_proposal_lifecycle(
        &mut canonical.state,
        &mut canonical.approval_bitset,
        &mut canonical.approval_count,
        &mut canonical.first_approval_slot,
        &mut canonical.council_approved_slot,
        &mut canonical.queued_slot,
        &mut canonical.executed_slot,
        &mut canonical.terminal_slot,
        &mut canonical.terminal_reason_code,
    );
    canonical.proposal_digest = [0; 32];
    hash_fixed(TARGET_HANDOFF_PROPOSAL_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_target_handoff_proposal_digest_v1(
    value: &TargetAuthorityHandoffProposalV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_target_handoff_proposal_digest_v1(value)?,
        value.proposal_digest,
    )
}

pub fn compute_target_handoff_receipt_digest_v1(
    value: &TargetAuthorityHandoffReceiptV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.receipt_digest = [0; 32];
    hash_fixed(TARGET_HANDOFF_RECEIPT_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_target_handoff_receipt_digest_v1(
    value: &TargetAuthorityHandoffReceiptV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_target_handoff_receipt_digest_v1(value)?,
        value.receipt_digest,
    )
}

pub fn compute_bootstrap_activation_proposal_digest_v1(
    value: &BootstrapActivationProposalV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    clear_ceremony_proposal_lifecycle(
        &mut canonical.state,
        &mut canonical.approval_bitset,
        &mut canonical.approval_count,
        &mut canonical.first_approval_slot,
        &mut canonical.council_approved_slot,
        &mut canonical.queued_slot,
        &mut canonical.executed_slot,
        &mut canonical.terminal_slot,
        &mut canonical.terminal_reason_code,
    );
    canonical.proposal_digest = [0; 32];
    hash_fixed(BOOTSTRAP_ACTIVATION_PROPOSAL_DIGEST_DOMAIN_V1, &canonical)
}

pub fn validate_bootstrap_activation_proposal_digest_v1(
    value: &BootstrapActivationProposalV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_bootstrap_activation_proposal_digest_v1(value)?,
        value.proposal_digest,
    )
}

pub fn compute_bootstrap_activation_receipt_digest_v1(
    value: &BootstrapActivationReceiptV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.receipt_digest = [0; 32];
    hash_fixed(BOOTSTRAP_ACTIVATION_RECEIPT_DIGEST_DOMAIN_V1, &canonical)
}

/// Commits the complete static activation-receipt image while excluding only
/// the actual landing slot and the slot-dependent deployment/receipt digests.
/// Those final digests are derived and stored by the controller after the plan
/// has matched.
pub fn compute_bootstrap_activation_receipt_plan_digest_v1(
    value: &BootstrapActivationReceiptV1,
) -> GovernanceResult<[u8; 32]> {
    let mut canonical = value.clone();
    canonical.current_deployment_digest = [0; 32];
    canonical.finalized_slot = 0;
    canonical.receipt_digest = [0; 32];
    hash_fixed(
        BOOTSTRAP_ACTIVATION_RECEIPT_PLAN_DIGEST_DOMAIN_V1,
        &canonical,
    )
}

pub fn validate_bootstrap_activation_receipt_digest_v1(
    value: &BootstrapActivationReceiptV1,
) -> GovernanceResult<()> {
    value.validate_static()?;
    require_digest(
        compute_bootstrap_activation_receipt_digest_v1(value)?,
        value.receipt_digest,
    )
}

#[allow(clippy::too_many_arguments)]
fn clear_ceremony_proposal_lifecycle(
    state: &mut CeremonyProposalStateV1,
    approval_bitset: &mut u8,
    approval_count: &mut u8,
    first_approval_slot: &mut u64,
    council_approved_slot: &mut u64,
    queued_slot: &mut u64,
    executed_slot: &mut u64,
    terminal_slot: &mut u64,
    terminal_reason_code: &mut u16,
) {
    *state = CeremonyProposalStateV1::Draft;
    *approval_bitset = 0;
    *approval_count = 0;
    *first_approval_slot = 0;
    *council_approved_slot = 0;
    *queued_slot = 0;
    *executed_slot = 0;
    *terminal_slot = 0;
    *terminal_reason_code = 0;
}

const MAX_CEREMONY_DIGEST_IMAGE_LEN: usize = 1280;

fn hash_fixed<T: BorshSerialize>(domain: &[u8], value: &T) -> GovernanceResult<[u8; 32]> {
    // The SBF allocator is a bump arena: dropping each temporary `Vec` does
    // not reclaim heap during the instruction. Ceremony execution validates
    // several fixed accounts and previously exhausted the default heap even
    // though no individual digest was large. Serialize into a bounded stack
    // image instead. Every ceremony account is fixed at no more than 1,280
    // bytes, and slice-backed Borsh serialization is byte-identical to
    // `try_to_vec` while failing closed if a future account exceeds the cap.
    let mut bytes = [0u8; MAX_CEREMONY_DIGEST_IMAGE_LEN];
    let remaining = {
        let mut output = &mut bytes[..];
        value
            .serialize(&mut output)
            .map_err(|_| GovernanceError::InvalidRelease1Account)?;
        output.len()
    };
    let encoded_len = MAX_CEREMONY_DIGEST_IMAGE_LEN
        .checked_sub(remaining)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    Ok(hashv(&[domain, &bytes[..encoded_len]]).to_bytes())
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

    use super::*;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    #[test]
    fn every_ceremony_digest_domain_is_unique() {
        let domains = [
            CAPACITY_POLICY_DIGEST_DOMAIN_V1,
            CONTROLLER_RELEASE_DIGEST_DOMAIN_V1,
            PROGRAMDATA_OBSERVATION_SUBJECT_DOMAIN_V1,
            PROGRAMDATA_OBSERVATION_DIGEST_DOMAIN_V1,
            CURRENT_DEPLOYMENT_DIGEST_DOMAIN_V1,
            CONTROLLER_IMMUTABILITY_RECEIPT_DIGEST_DOMAIN_V1,
            TARGET_HANDOFF_PROPOSAL_DIGEST_DOMAIN_V1,
            TARGET_HANDOFF_RECEIPT_DIGEST_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_PROPOSAL_DIGEST_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_RECEIPT_DIGEST_DOMAIN_V1,
        ];
        assert_eq!(
            domains.iter().copied().collect::<BTreeSet<_>>().len(),
            domains.len()
        );
    }

    #[test]
    fn observation_subject_is_purpose_generation_artifact_and_capacity_bound() {
        let args = |purpose, generation, artifact_length, artifact_scheme, capacity| {
            compute_programdata_observation_subject_digest_v1(
                &key(1),
                &key(2),
                &key(3),
                &key(4),
                purpose,
                &key(5),
                generation,
                &key(9),
                GateStatusV1::FrozenForUpgrade,
                10,
                &key(11),
                12,
                13,
                &[6; 32],
                artifact_length,
                &[7; 32],
                &[8; 32],
                &artifact_scheme,
                capacity,
            )
            .unwrap()
        };
        let base = args(
            ProgramDataObservationPurposeV1::TargetHandoffBridge,
            1,
            1_024,
            [10; 32],
            2_048,
        );
        assert_ne!(
            base,
            args(
                ProgramDataObservationPurposeV1::BootstrapActivation,
                1,
                1_024,
                [10; 32],
                2_048,
            )
        );
        assert_ne!(
            base,
            args(
                ProgramDataObservationPurposeV1::TargetHandoffBridge,
                2,
                1_024,
                [10; 32],
                2_048,
            )
        );
        assert_ne!(
            base,
            args(
                ProgramDataObservationPurposeV1::TargetHandoffBridge,
                1,
                1_025,
                [10; 32],
                2_048,
            )
        );
        assert_ne!(
            base,
            args(
                ProgramDataObservationPurposeV1::TargetHandoffBridge,
                1,
                1_024,
                [11; 32],
                2_048,
            )
        );
        assert_ne!(
            base,
            args(
                ProgramDataObservationPurposeV1::TargetHandoffBridge,
                1,
                1_024,
                [10; 32],
                2_049,
            )
        );
    }
}
