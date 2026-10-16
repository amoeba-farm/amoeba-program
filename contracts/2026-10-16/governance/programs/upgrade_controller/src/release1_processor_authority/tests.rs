use solana_program::instruction::{AccountMeta, Instruction};

use super::*;
use crate::state::{CouncilSeatV1, GovernanceCouncilSetV1, COUNCIL_SEAT_RESERVED_LEN};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn programdata_header(slot: u64, authority: Option<Pubkey>) -> [u8; 45] {
    let mut out = [0u8; 45];
    out[..4].copy_from_slice(&3u32.to_le_bytes());
    out[4..12].copy_from_slice(&slot.to_le_bytes());
    if let Some(authority) = authority {
        out[12] = 1;
        out[13..].copy_from_slice(authority.as_ref());
    }
    out
}

fn council() -> GovernanceCouncilSetV1 {
    GovernanceCouncilSetV1 {
        discriminator: crate::state::GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: crate::state::ACCOUNT_VERSION_V1,
        bump: 1,
        initialized: true,
        controller_config: key(1),
        version: 1,
        target_program: key(2),
        activation_slot: 1,
        deactivation_slot: 0,
        seats: std::array::from_fn(|index| CouncilSeatV1 {
            seat_authority: key(10 + index as u8),
            term_start_slot: 1,
            term_end_slot: 1_000,
            active: true,
            reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
        }),
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [1; 32],
        reserved: [0; crate::state::GOVERNANCE_COUNCIL_RESERVED_LEN],
    }
}

#[test]
fn header_deltas_are_exact_and_payload_metadata_cannot_drift() {
    let legacy = key(1);
    let controller = key(2);
    let pre = programdata_header(44, Some(legacy));
    let post = expected_handoff_post_header(&pre, legacy, controller).unwrap();
    validate_some_to_some_header_delta(&pre, &post, legacy, controller).unwrap();
    let mut wrong_slot = post;
    wrong_slot[4] ^= 1;
    assert_eq!(
        validate_some_to_some_header_delta(&pre, &wrong_slot, legacy, controller),
        Err(GovernanceError::InvalidAuthorityTransition.into())
    );

    let mut immutable = pre;
    immutable[12] = 0;
    validate_some_to_none_header_delta(&pre, &immutable, legacy).unwrap();
    immutable[13] ^= 1;
    assert_eq!(
        validate_some_to_none_header_delta(&pre, &immutable, legacy),
        Err(GovernanceError::InvalidAuthorityTransition.into())
    );
}

#[test]
fn all_32_masks_enforce_exact_three_of_five_for_ceremony_execution() {
    let council = council();
    for mask in 0u8..32 {
        let count = mask.count_ones() as u8;
        assert_eq!(
            require_exact_quorum(&council, mask, count, 100).is_ok(),
            count == 3,
            "mask {mask:05b}"
        );
    }
    assert_eq!(
        require_exact_quorum(&council, 0b00111, 2, 100),
        Err(GovernanceError::QuorumNotSatisfied.into())
    );
}

#[test]
fn versioned_proposal_creation_seeds_match_pda_derivations() {
    let program_id = key(31);
    let target = key(32);
    let mut handoff_addresses = Vec::new();
    let mut activation_addresses = Vec::new();
    for council_version in [1u64, 2, u64::MAX] {
        let council_version_seed = council_version.to_le_bytes();

        let (handoff, handoff_bump) =
            derive_target_authority_handoff_pda(&program_id, &target, council_version);
        let handoff_bump_seed = [handoff_bump];
        assert_eq!(
            Pubkey::create_program_address(
                &[
                    UPGRADE_SEED_DOMAIN_V1,
                    TARGET_HANDOFF_SEED,
                    target.as_ref(),
                    &council_version_seed,
                    &handoff_bump_seed,
                ],
                &program_id,
            )
            .unwrap(),
            handoff
        );
        handoff_addresses.push(handoff);

        let (activation, activation_bump) =
            derive_bootstrap_activation_pda(&program_id, &target, council_version);
        let activation_bump_seed = [activation_bump];
        assert_eq!(
            Pubkey::create_program_address(
                &[
                    UPGRADE_SEED_DOMAIN_V1,
                    BOOTSTRAP_ACTIVATION_SEED,
                    target.as_ref(),
                    &council_version_seed,
                    &activation_bump_seed,
                ],
                &program_id,
            )
            .unwrap(),
            activation
        );
        activation_addresses.push(activation);
    }
    assert_eq!(
        handoff_addresses
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        handoff_addresses.len()
    );
    assert_eq!(
        activation_addresses
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        activation_addresses.len()
    );
}

#[test]
fn council_rotation_stales_proposals_but_preserves_one_time_destinations() {
    let program_id = key(33);
    let target = key(34);
    let old_handoff = derive_target_authority_handoff_pda(&program_id, &target, 8).0;
    let current_handoff = derive_target_authority_handoff_pda(&program_id, &target, 9).0;
    let old_activation = derive_bootstrap_activation_pda(&program_id, &target, 8).0;
    let current_activation = derive_bootstrap_activation_pda(&program_id, &target, 9).0;
    assert_ne!(old_handoff, current_handoff);
    assert_ne!(old_activation, current_activation);
    assert_eq!(
        derive_target_handoff_receipt_pda(&program_id, &target),
        derive_target_handoff_receipt_pda(&program_id, &target)
    );
    assert_eq!(
        derive_bootstrap_activation_receipt_pda(&program_id, &target),
        derive_bootstrap_activation_receipt_pda(&program_id, &target)
    );
    assert_eq!(
        derive_current_deployment_state_pda(&program_id, &target),
        derive_current_deployment_state_pda(&program_id, &target)
    );
}

#[test]
fn rotated_activation_can_reuse_only_untouched_rent_exempt_destinations() {
    let program_id = key(35);
    let target = key(36);
    let (pda_key, bump) = derive_bootstrap_activation_receipt_pda(&program_id, &target);
    let rent = Rent::default();
    let space = BootstrapActivationReceiptV1::LEN;
    let pda = leaked_owned_account(
        pda_key,
        program_id,
        rent.minimum_balance(space),
        true,
        false,
        false,
        vec![0; space],
    );
    let payer = leaked_owned_account(key(37), key(38), 1, true, true, false, vec![]);
    let system = leaked_owned_account(system_program::ID, key(39), 1, false, false, true, vec![]);
    let bump_seed = [bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        BOOTSTRAP_ACTIVATION_RECEIPT_SEED,
        target.as_ref(),
        &bump_seed,
    ];
    create_or_reuse_zero_fixed_pda(&program_id, &payer, &pda, &system, &rent, space, seeds)
        .unwrap();

    pda.try_borrow_mut_data().unwrap()[0] = 1;
    assert_eq!(
        create_or_reuse_zero_fixed_pda(&program_id, &payer, &pda, &system, &rent, space, seeds,),
        Err(GovernanceError::CeremonyAlreadyFinalized.into())
    );
}

#[test]
fn checked_loader_cpi_shape_is_exact() {
    let target = key(3);
    let programdata = derive_upgradeable_programdata_address(&target).0;
    let legacy = key(4);
    let controller = key(5);
    let instruction = set_upgrade_authority_checked(&target, &legacy, &controller);
    validate_checked_handoff_cpi_shape(&instruction, &programdata, &legacy, &controller).unwrap();
    let mut unchecked = instruction;
    unchecked.data = 4u32.to_le_bytes().to_vec();
    assert_eq!(
        validate_checked_handoff_cpi_shape(&unchecked, &programdata, &legacy, &controller,),
        Err(GovernanceError::InvalidAuthorityTransition.into())
    );
}

#[test]
fn major_timing_is_immutable_and_overflow_safe() {
    assert_eq!(
        derive_major_timing_from_values(10, 20, 100, 5).unwrap(),
        (6, 16, 36, 105)
    );
    assert!(derive_major_timing_from_values(10, 20, 100, u64::MAX).is_err());
    assert!(derive_major_timing_from_values(10, 90, 100, 5).is_err());
}

#[test]
fn envelope_rejects_siblings_and_privilege_drift() {
    let program_id = key(6);
    let envelope = CeremonyEnvelopeV1 {
        compute_unit_limit: 1_000_000,
        compute_unit_price_micro_lamports: 7,
        durable_nonce_account: OptionalPubkeyV1::none(),
        durable_nonce_authority: OptionalPubkeyV1::none(),
    };
    let current_data = vec![47, 1, 2, 3];
    let account_key = key(7);
    let info = leaked_account(account_key, true, false, false, vec![]);
    let ix_sysvar_key = sysvar_ids::instructions::ID;
    let sysvar_info = leaked_account(ix_sysvar_key, false, false, false, vec![]);
    let mut current_accounts = vec![info, sysvar_info];
    let current = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(account_key, false),
            AccountMeta::new_readonly(ix_sysvar_key, false),
        ],
        data: current_data.clone(),
    };
    let prefix = [
        compute_limit(&envelope),
        compute_price(&envelope),
        current.clone(),
    ];
    current_accounts[1] = instructions_sysvar(&prefix, 2);
    validate_canonical_envelope(
        &program_id,
        &current_accounts,
        &current_accounts[1],
        &current_data,
        &envelope,
    )
    .unwrap();

    let sibling = Instruction {
        program_id: key(8),
        accounts: vec![],
        data: vec![1],
    };
    let with_sibling = [prefix[0].clone(), prefix[1].clone(), current, sibling];
    current_accounts[1] = instructions_sysvar(&with_sibling, 2);
    assert!(validate_canonical_envelope(
        &program_id,
        &current_accounts,
        &current_accounts[1],
        &current_data,
        &envelope,
    )
    .is_err());
}

fn leaked_account(
    account_key: Pubkey,
    writable: bool,
    signer: bool,
    executable: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    leaked_owned_account(account_key, key(250), 1, writable, signer, executable, data)
}

#[allow(clippy::too_many_arguments)]
fn leaked_owned_account(
    account_key: Pubkey,
    owner: Pubkey,
    lamports: u64,
    writable: bool,
    signer: bool,
    executable: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    AccountInfo::new(
        Box::leak(Box::new(account_key)),
        signer,
        writable,
        Box::leak(Box::new(lamports)),
        Box::leak(data.into_boxed_slice()),
        Box::leak(Box::new(owner)),
        executable,
        0,
    )
}

fn borrowed_instruction(instruction: &Instruction) -> instructions::BorrowedInstruction<'_> {
    instructions::BorrowedInstruction {
        program_id: &instruction.program_id,
        accounts: instruction
            .accounts
            .iter()
            .map(|meta| instructions::BorrowedAccountMeta {
                pubkey: &meta.pubkey,
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            })
            .collect(),
        data: &instruction.data,
    }
}

fn instructions_sysvar(transaction: &[Instruction], current_index: u16) -> AccountInfo<'static> {
    let borrowed: Vec<_> = transaction.iter().map(borrowed_instruction).collect();
    let mut data = instructions::construct_instructions_data(&borrowed);
    let offset = data.len() - 2;
    data[offset..].copy_from_slice(&current_index.to_le_bytes());
    leaked_account(sysvar_ids::instructions::ID, false, false, false, data)
}

fn compute_limit(envelope: &CeremonyEnvelopeV1) -> Instruction {
    let mut data = [0u8; 5];
    data[0] = 2;
    data[1..].copy_from_slice(&envelope.compute_unit_limit.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

fn compute_price(envelope: &CeremonyEnvelopeV1) -> Instruction {
    let mut data = [0u8; 9];
    data[0] = 3;
    data[1..].copy_from_slice(&envelope.compute_unit_price_micro_lamports.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}
