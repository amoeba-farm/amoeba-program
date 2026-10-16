use solana_program::{pubkey, pubkey::Pubkey};

use crate::state::CheckpointPhaseV1;

pub const UPGRADE_SEED_DOMAIN_V1: &[u8] = b"ameba-upgrade-v1";
pub const TARGET_SEED: &[u8] = b"target";
pub const AUTHORITY_SEED: &[u8] = b"authority";
pub const GATE_SEED: &[u8] = b"gate";
pub const POLICY_SEED: &[u8] = b"policy";
pub const COUNCIL_SEED: &[u8] = b"council";
pub const PROPOSAL_SEED: &[u8] = b"proposal";
pub const CHECKPOINT_SEED: &[u8] = b"checkpoint";
pub const BUFFER_CHECK_SEED: &[u8] = b"buffer-check";
pub const PROGRAMDATA_CHECK_SEED: &[u8] = b"programdata-check";
pub const EMERGENCY_RESOLUTION_SEED: &[u8] = b"emergency-resolution";
pub const EMERGENCY_CHECKPOINT_SEED: &[u8] = b"emergency-checkpoint";
pub const EMERGENCY_RESOLUTION_V2_SEED: &[u8] = b"emergency-resolution-v2";
pub const EMERGENCY_CHECKPOINT_V2_SEED: &[u8] = b"emergency-checkpoint-v2";
pub const COUNCIL_ROTATION_SEED: &[u8] = b"council-rotation";
pub const EMERGENCY_FREEZE_OBSERVATION_SEED: &[u8] = b"emergency-observation";
pub const PROGRAMDATA_FAILURE_OBSERVATION_SEED: &[u8] = b"programdata-failure";
pub const CHECKPOINT_ATTESTATION_SEED: &[u8] = b"checkpoint-attestation";
pub const CAPACITY_POLICY_SEED: &[u8] = b"capacity-policy";
pub const CONTROLLER_RELEASE_SEED: &[u8] = b"controller-release";
pub const PROGRAMDATA_OBSERVATION_SEED: &[u8] = b"programdata-observation";
pub const DEPLOYMENT_STATE_SEED: &[u8] = b"deployment-state";
pub const CONTROLLER_IMMUTABILITY_SEED: &[u8] = b"controller-immutability";
pub const TARGET_HANDOFF_SEED: &[u8] = b"handoff";
pub const TARGET_HANDOFF_RECEIPT_SEED: &[u8] = b"handoff-receipt";
pub const BOOTSTRAP_ACTIVATION_SEED: &[u8] = b"bootstrap-activation";
pub const BOOTSTRAP_ACTIVATION_RECEIPT_SEED: &[u8] = b"activation-receipt";
pub const UPGRADEABLE_LOADER_ID: Pubkey = pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

pub fn derive_upgradeable_programdata_address(target_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[target_program.as_ref()], &UPGRADEABLE_LOADER_ID)
}

pub fn derive_controller_config_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[UPGRADE_SEED_DOMAIN_V1, TARGET_SEED, target_program.as_ref()],
        controller_program,
    )
}

pub fn derive_authority_pda(controller_program: &Pubkey, target_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            AUTHORITY_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_gate_pda(controller_program: &Pubkey, target_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[UPGRADE_SEED_DOMAIN_V1, GATE_SEED, target_program.as_ref()],
        controller_program,
    )
}

pub fn derive_policy_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    policy_version: u64,
) -> (Pubkey, u8) {
    let version = policy_version.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            POLICY_SEED,
            target_program.as_ref(),
            &version,
        ],
        controller_program,
    )
}

pub fn derive_council_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    council_version: u64,
) -> (Pubkey, u8) {
    let version = council_version.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            COUNCIL_SEED,
            target_program.as_ref(),
            &version,
        ],
        controller_program,
    )
}

pub fn derive_proposal_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    proposal_id: u64,
) -> (Pubkey, u8) {
    let id = proposal_id.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            PROPOSAL_SEED,
            target_program.as_ref(),
            &id,
        ],
        controller_program,
    )
}

pub fn derive_checkpoint_pda(
    controller_program: &Pubkey,
    proposal: &Pubkey,
    phase: CheckpointPhaseV1,
) -> (Pubkey, u8) {
    let phase = [phase as u8];
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CHECKPOINT_SEED,
            proposal.as_ref(),
            &phase,
        ],
        controller_program,
    )
}

pub fn derive_buffer_check_pda(controller_program: &Pubkey, proposal: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[UPGRADE_SEED_DOMAIN_V1, BUFFER_CHECK_SEED, proposal.as_ref()],
        controller_program,
    )
}

pub fn derive_programdata_check_pda(
    controller_program: &Pubkey,
    proposal: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            PROGRAMDATA_CHECK_SEED,
            proposal.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_emergency_resolution_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    frozen_epoch: u64,
) -> (Pubkey, u8) {
    let epoch = frozen_epoch.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            EMERGENCY_RESOLUTION_SEED,
            target_program.as_ref(),
            &epoch,
        ],
        controller_program,
    )
}

pub fn derive_emergency_checkpoint_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    frozen_epoch: u64,
) -> (Pubkey, u8) {
    let epoch = frozen_epoch.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            EMERGENCY_CHECKPOINT_SEED,
            target_program.as_ref(),
            &epoch,
        ],
        controller_program,
    )
}

/// Capacity-safe emergency resolutions are council-versioned so a council
/// rotation cannot strand the protocol in a frozen epoch. The prior V1 PDA
/// derivation remains unchanged for regression decoding.
pub fn derive_emergency_resolution_v2_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    frozen_epoch: u64,
    council_version: u64,
) -> (Pubkey, u8) {
    let epoch = frozen_epoch.to_le_bytes();
    let council = council_version.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            EMERGENCY_RESOLUTION_V2_SEED,
            target_program.as_ref(),
            &epoch,
            &council,
        ],
        controller_program,
    )
}

/// Each council-versioned resolution receives its own checkpoint PDA. A stale
/// resolution and its approvals remain immutable historical evidence while a
/// newly rotated council can start a fresh resolution for the same gate epoch.
pub fn derive_emergency_checkpoint_v2_pda(
    controller_program: &Pubkey,
    emergency_resolution: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            EMERGENCY_CHECKPOINT_V2_SEED,
            emergency_resolution.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_council_rotation_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    candidate_council_version: u64,
) -> (Pubkey, u8) {
    let version = candidate_council_version.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            COUNCIL_ROTATION_SEED,
            target_program.as_ref(),
            &version,
        ],
        controller_program,
    )
}

pub fn derive_emergency_freeze_observation_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    frozen_epoch: u64,
) -> (Pubkey, u8) {
    let epoch = frozen_epoch.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            EMERGENCY_FREEZE_OBSERVATION_SEED,
            target_program.as_ref(),
            &epoch,
        ],
        controller_program,
    )
}

pub fn derive_programdata_failure_observation_pda(
    controller_program: &Pubkey,
    primary_proposal: &Pubkey,
    frozen_epoch: u64,
) -> (Pubkey, u8) {
    let epoch = frozen_epoch.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            PROGRAMDATA_FAILURE_OBSERVATION_SEED,
            primary_proposal.as_ref(),
            &epoch,
        ],
        controller_program,
    )
}

pub fn derive_checkpoint_attestation_pda(
    controller_program: &Pubkey,
    checkpoint: &Pubkey,
    council_version: u64,
    seat_index: u8,
) -> (Pubkey, u8) {
    let version = council_version.to_le_bytes();
    let seat = [seat_index];
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CHECKPOINT_ATTESTATION_SEED,
            checkpoint.as_ref(),
            &version,
            &seat,
        ],
        controller_program,
    )
}

pub fn derive_capacity_policy_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CAPACITY_POLICY_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_controller_release_commitment_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CONTROLLER_RELEASE_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_programdata_observation_pda(
    controller_program: &Pubkey,
    observed_program: &Pubkey,
    purpose: u8,
    subject_digest: &[u8; 32],
    generation: u64,
) -> (Pubkey, u8) {
    let purpose = [purpose];
    let generation = generation.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            PROGRAMDATA_OBSERVATION_SEED,
            observed_program.as_ref(),
            &purpose,
            subject_digest,
            &generation,
        ],
        controller_program,
    )
}

pub fn derive_current_deployment_state_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            DEPLOYMENT_STATE_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_controller_immutability_receipt_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CONTROLLER_IMMUTABILITY_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_target_handoff_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    council_version: u64,
) -> (Pubkey, u8) {
    let council_version_seed = council_version.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            TARGET_HANDOFF_SEED,
            target_program.as_ref(),
            &council_version_seed,
        ],
        controller_program,
    )
}

pub fn derive_target_handoff_receipt_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            TARGET_HANDOFF_RECEIPT_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_bootstrap_activation_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    council_version: u64,
) -> (Pubkey, u8) {
    let council_version_seed = council_version.to_le_bytes();
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_SEED,
            target_program.as_ref(),
            &council_version_seed,
        ],
        controller_program,
    )
}

pub fn derive_bootstrap_activation_receipt_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            UPGRADE_SEED_DOMAIN_V1,
            BOOTSTRAP_ACTIVATION_RECEIPT_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_pairwise_separated() {
        let controller = Pubkey::new_from_array([17; 32]);
        let target = Pubkey::new_from_array([29; 32]);
        let proposal = derive_proposal_pda(&controller, &target, 7).0;
        let addresses = [
            derive_upgradeable_programdata_address(&target).0,
            derive_controller_config_pda(&controller, &target).0,
            derive_authority_pda(&controller, &target).0,
            derive_gate_pda(&controller, &target).0,
            derive_policy_pda(&controller, &target, 7).0,
            derive_council_pda(&controller, &target, 7).0,
            proposal,
            derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Prestate).0,
            derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Poststate).0,
            derive_buffer_check_pda(&controller, &proposal).0,
            derive_programdata_check_pda(&controller, &proposal).0,
            derive_emergency_resolution_pda(&controller, &target, 7).0,
            derive_emergency_checkpoint_pda(&controller, &target, 7).0,
            derive_emergency_resolution_v2_pda(&controller, &target, 7, 1).0,
            derive_emergency_checkpoint_v2_pda(
                &controller,
                &derive_emergency_resolution_v2_pda(&controller, &target, 7, 1).0,
            )
            .0,
            derive_council_rotation_pda(&controller, &target, 8).0,
            derive_emergency_freeze_observation_pda(&controller, &target, 7).0,
            derive_programdata_failure_observation_pda(&controller, &proposal, 7).0,
            derive_checkpoint_attestation_pda(&controller, &proposal, 7, 2).0,
            derive_capacity_policy_pda(&controller, &target).0,
            derive_controller_release_commitment_pda(&controller, &target).0,
            derive_programdata_observation_pda(&controller, &target, 1, &[91; 32], 7).0,
            derive_current_deployment_state_pda(&controller, &target).0,
            derive_controller_immutability_receipt_pda(&controller, &target).0,
            derive_target_handoff_pda(&controller, &target, 7).0,
            derive_target_handoff_receipt_pda(&controller, &target).0,
            derive_bootstrap_activation_pda(&controller, &target, 7).0,
            derive_bootstrap_activation_receipt_pda(&controller, &target).0,
        ];
        for (index, address) in addresses.iter().enumerate() {
            assert!(
                addresses[..index].iter().all(|prior| prior != address),
                "PDA domain collision at index {index}"
            );
        }
    }

    #[test]
    fn numeric_seeds_are_little_endian_and_order_sensitive() {
        let controller = Pubkey::new_from_array([31; 32]);
        let target = Pubkey::new_from_array([37; 32]);
        let version = 0x0102_0304_0506_0708u64;
        let (actual, bump) = derive_policy_pda(&controller, &target, version);
        let bump_seed = [bump];
        let expected = Pubkey::create_program_address(
            &[
                UPGRADE_SEED_DOMAIN_V1,
                POLICY_SEED,
                target.as_ref(),
                &version.to_le_bytes(),
                &bump_seed,
            ],
            &controller,
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_ne!(
            actual,
            derive_policy_pda(&controller, &target, version.swap_bytes()).0
        );

        let (resolution, bump) = derive_emergency_resolution_pda(&controller, &target, version);
        let bump_seed = [bump];
        let expected = Pubkey::create_program_address(
            &[
                UPGRADE_SEED_DOMAIN_V1,
                EMERGENCY_RESOLUTION_SEED,
                target.as_ref(),
                &version.to_le_bytes(),
                &bump_seed,
            ],
            &controller,
        )
        .unwrap();
        assert_eq!(resolution, expected);
        assert_ne!(
            resolution,
            derive_emergency_resolution_pda(&controller, &target, version.swap_bytes()).0
        );

        let (observation, bump) =
            derive_emergency_freeze_observation_pda(&controller, &target, version);
        let bump_seed = [bump];
        let expected = Pubkey::create_program_address(
            &[
                UPGRADE_SEED_DOMAIN_V1,
                EMERGENCY_FREEZE_OBSERVATION_SEED,
                target.as_ref(),
                &version.to_le_bytes(),
                &bump_seed,
            ],
            &controller,
        )
        .unwrap();
        assert_eq!(observation, expected);
        assert_ne!(
            observation,
            derive_emergency_freeze_observation_pda(&controller, &target, version.swap_bytes()).0
        );

        let proposal = derive_proposal_pda(&controller, &target, 9).0;
        let (failure, bump) =
            derive_programdata_failure_observation_pda(&controller, &proposal, version);
        let bump_seed = [bump];
        let expected = Pubkey::create_program_address(
            &[
                UPGRADE_SEED_DOMAIN_V1,
                PROGRAMDATA_FAILURE_OBSERVATION_SEED,
                proposal.as_ref(),
                &version.to_le_bytes(),
                &bump_seed,
            ],
            &controller,
        )
        .unwrap();
        assert_eq!(failure, expected);
        assert_ne!(
            failure,
            derive_programdata_failure_observation_pda(
                &controller,
                &proposal,
                version.swap_bytes()
            )
            .0
        );

        let checkpoint =
            derive_checkpoint_pda(&controller, &proposal, CheckpointPhaseV1::Prestate).0;
        let (attestation, bump) =
            derive_checkpoint_attestation_pda(&controller, &checkpoint, version, 3);
        let bump_seed = [bump];
        let version_seed = version.to_le_bytes();
        let seat_seed = [3];
        let expected = Pubkey::create_program_address(
            &[
                UPGRADE_SEED_DOMAIN_V1,
                CHECKPOINT_ATTESTATION_SEED,
                checkpoint.as_ref(),
                &version_seed,
                &seat_seed,
                &bump_seed,
            ],
            &controller,
        )
        .unwrap();
        assert_eq!(attestation, expected);
        assert_ne!(
            attestation,
            derive_checkpoint_attestation_pda(&controller, &checkpoint, version, 4).0
        );
        assert_ne!(
            attestation,
            derive_checkpoint_attestation_pda(&controller, &checkpoint, version.swap_bytes(), 3).0
        );

        let purpose = 5u8;
        let subject_digest = [53; 32];
        let (observation, bump) = derive_programdata_observation_pda(
            &controller,
            &target,
            purpose,
            &subject_digest,
            version,
        );
        let bump_seed = [bump];
        let purpose_seed = [purpose];
        let generation_seed = version.to_le_bytes();
        let expected = Pubkey::create_program_address(
            &[
                UPGRADE_SEED_DOMAIN_V1,
                PROGRAMDATA_OBSERVATION_SEED,
                target.as_ref(),
                &purpose_seed,
                &subject_digest,
                &generation_seed,
                &bump_seed,
            ],
            &controller,
        )
        .unwrap();
        assert_eq!(observation, expected);
        assert_ne!(
            observation,
            derive_programdata_observation_pda(
                &controller,
                &target,
                purpose,
                &subject_digest,
                version.swap_bytes(),
            )
            .0
        );
        assert_ne!(
            observation,
            derive_programdata_observation_pda(
                &controller,
                &target,
                purpose + 1,
                &subject_digest,
                version,
            )
            .0
        );
        assert_ne!(
            observation,
            derive_programdata_observation_pda(&controller, &target, purpose, &[54; 32], version,)
                .0
        );
    }

    #[test]
    fn emergency_v2_rotation_gets_a_fresh_resolution_and_checkpoint() {
        let controller = Pubkey::new_from_array([41; 32]);
        let target = Pubkey::new_from_array([43; 32]);
        let epoch = 9;
        let first = derive_emergency_resolution_v2_pda(&controller, &target, epoch, 1).0;
        let rotated = derive_emergency_resolution_v2_pda(&controller, &target, epoch, 2).0;
        assert_ne!(first, rotated);
        assert_ne!(
            derive_emergency_checkpoint_v2_pda(&controller, &first).0,
            derive_emergency_checkpoint_v2_pda(&controller, &rotated).0
        );
        assert_ne!(
            first,
            derive_emergency_resolution_pda(&controller, &target, epoch).0
        );
    }
}
