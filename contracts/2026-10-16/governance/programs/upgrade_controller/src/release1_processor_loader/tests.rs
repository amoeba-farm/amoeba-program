use super::*;
use crate::artifact_merkle::{
    artifact_merkle_proof, artifact_merkle_root, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn leaked_account(
    account_key: Pubkey,
    writable: bool,
    signer: bool,
    executable: bool,
    data: Vec<u8>,
) -> AccountInfo<'static> {
    let key_ref = Box::leak(Box::new(account_key));
    let owner = Box::leak(Box::new(key(250)));
    let lamports = Box::leak(Box::new(1u64));
    let data = Box::leak(data.into_boxed_slice());
    AccountInfo::new(
        key_ref, signer, writable, lamports, data, owner, executable, 0,
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
    let current_index_offset = data.len().checked_sub(2).unwrap();
    data[current_index_offset..].copy_from_slice(&current_index.to_le_bytes());
    leaked_account(sysvar_ids::instructions::ID, false, false, false, data)
}

fn compute_limit_instruction(envelope: &EnvelopeExpectationV1) -> Instruction {
    let mut data = [0u8; 5];
    data[0] = 2;
    data[1..].copy_from_slice(&envelope.compute_unit_limit.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

fn compute_price_instruction(envelope: &EnvelopeExpectationV1) -> Instruction {
    let mut data = [0u8; 9];
    data[0] = 3;
    data[1..].copy_from_slice(&envelope.compute_unit_price_micro_lamports.to_le_bytes());
    Instruction {
        program_id: compute_budget::ID,
        accounts: vec![],
        data: data.to_vec(),
    }
}

#[test]
fn bitmap_rejects_duplicate_and_out_of_range_without_mutation() {
    let mut bitmap = [0u8; VERIFICATION_BITMAP_BYTES_V1];
    let mut count = 0;
    mark_bitmap_bit(&mut bitmap, &mut count, 9, 8).unwrap();
    let before = bitmap;
    assert_eq!(count, 1);
    assert_eq!(
        mark_bitmap_bit(&mut bitmap, &mut count, 9, 8),
        Err(GovernanceError::InvalidRelease1Bitmap.into())
    );
    assert_eq!(bitmap, before);
    assert_eq!(count, 1);
    assert_eq!(
        mark_bitmap_bit(&mut bitmap, &mut count, 9, 9),
        Err(GovernanceError::InvalidRelease1Bitmap.into())
    );
    assert_eq!(bitmap, before);
    assert_eq!(count, 1);
}

#[test]
fn post_execution_verification_remains_live_after_proposal_expiry() {
    let proposal = UpgradeProposalV2 {
        frozen_slot: 10,
        upgrade_executed_slot: 20,
        expiry_slot: 30,
        ..UpgradeProposalV2::default()
    };

    assert_eq!(
        validate_post_execution_frozen_slot(&proposal, 19),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    validate_post_execution_frozen_slot(&proposal, 20).unwrap();
    validate_post_execution_frozen_slot(&proposal, proposal.expiry_slot).unwrap();
    validate_post_execution_frozen_slot(&proposal, 1_000).unwrap();
}

#[test]
fn extension_preserves_a_strictly_later_preexpiry_upgrade_slot() {
    let mut proposal = UpgradeProposalV2 {
        expiry_slot: 30,
        ..UpgradeProposalV2::default()
    };
    let before = proposal.clone();

    require_extension_execution_runway(&proposal, 28).unwrap();
    assert_eq!(
        require_extension_execution_runway(&proposal, 29),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    proposal.expiry_slot = u64::MAX;
    assert_eq!(
        require_extension_execution_runway(&proposal, u64::MAX),
        Err(GovernanceError::ArithmeticOverflow.into())
    );
    proposal.expiry_slot = before.expiry_slot;
    assert_eq!(proposal, before);
}

#[test]
fn primary_execution_preserves_complete_rollback_recovery_runway() {
    // 83 + five-slot delay + ten-slot checkpoint review + one execution
    // slot = 99, which is the final safe horizon before expiry 100.
    require_primary_execution_rollback_runway(83, 5, 10, 100).unwrap();
    assert_eq!(
        require_primary_execution_rollback_runway(84, 5, 10, 100),
        Err(GovernanceError::InvalidProposalTiming.into())
    );
    assert_eq!(
        require_primary_execution_rollback_runway(u64::MAX - 2, 1, 1, u64::MAX),
        Err(GovernanceError::ArithmeticOverflow.into())
    );
}

#[test]
fn execute_pair_requires_identical_checkpoint_schema_and_policy() {
    let primary = UpgradeProposalV2 {
        checkpoint_schema_id: [1; 32],
        checkpoint_policy_hash: [2; 32],
        ..UpgradeProposalV2::default()
    };
    let mut rollback = primary.clone();
    let primary_before = primary.clone();
    validate_reciprocal_checkpoint_commitments(&primary, &rollback).unwrap();

    rollback.checkpoint_schema_id[0] ^= 1;
    assert_eq!(
        validate_reciprocal_checkpoint_commitments(&primary, &rollback),
        Err(GovernanceError::InvalidProposalCommitment.into())
    );
    rollback.checkpoint_schema_id = primary.checkpoint_schema_id;
    rollback.checkpoint_policy_hash[0] ^= 1;
    assert_eq!(
        validate_reciprocal_checkpoint_commitments(&primary, &rollback),
        Err(GovernanceError::InvalidProposalCommitment.into())
    );
    assert_eq!(primary, primary_before);
}

#[test]
fn exact_region_chunk_covers_first_middle_and_final_partial_edges() {
    let size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
    let mut payload = vec![0u8; size * 2 + 17 + 11];
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte = index as u8;
    }
    assert_eq!(
        exact_region_chunk(&payload, 11, (size * 2 + 17) as u64, size as u32, 0)
            .unwrap()
            .len(),
        size
    );
    assert_eq!(
        exact_region_chunk(&payload, 11, (size * 2 + 17) as u64, size as u32, 1)
            .unwrap()
            .len(),
        size
    );
    assert_eq!(
        exact_region_chunk(&payload, 11, (size * 2 + 17) as u64, size as u32, 2)
            .unwrap()
            .len(),
        17
    );
    assert_eq!(
        exact_region_chunk(&payload, 11, (size * 2 + 17) as u64, size as u32, 3),
        Err(GovernanceError::InvalidMerkleProof.into())
    );
}

#[test]
fn deployed_chunk_proofs_bind_first_middle_and_final_partial_bytes() {
    let size = RELEASE1_ARTIFACT_CHUNK_SIZE_V1 as usize;
    let artifact: Vec<u8> = (0..size * 2 + 17)
        .map(|index| (index.wrapping_mul(31) & 0xff) as u8)
        .collect();
    let root = artifact_merkle_root(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1).unwrap();
    for index in [0u32, 1, 2] {
        let chunk = exact_region_chunk(
            &artifact,
            0,
            artifact.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            index,
        )
        .unwrap();
        let proof =
            artifact_merkle_proof(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, index).unwrap();
        verify_artifact_chunk_proof(
            &root,
            artifact.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            index,
            chunk,
            &proof,
        )
        .unwrap();
    }

    let final_chunk = exact_region_chunk(
        &artifact,
        0,
        artifact.len() as u64,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
        2,
    )
    .unwrap();
    let proof = artifact_merkle_proof(&artifact, RELEASE1_ARTIFACT_CHUNK_SIZE_V1, 2).unwrap();
    let mut wrong_chunk = final_chunk.to_vec();
    wrong_chunk[0] ^= 1;
    assert_eq!(
        verify_artifact_chunk_proof(
            &root,
            artifact.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            2,
            &wrong_chunk,
            &proof,
        ),
        Err(GovernanceError::InvalidMerkleProof)
    );
    assert_eq!(
        verify_artifact_chunk_proof(
            &root,
            artifact.len() as u64,
            RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
            2,
            &final_chunk[..final_chunk.len() - 1],
            &proof,
        ),
        Err(GovernanceError::InvalidMerkleProof)
    );
}

#[test]
fn raw_programdata_and_appended_zero_region_fail_closed_on_drift() {
    let raw = vec![3u8; 97];
    let expected = loader_account_data_hash(&raw);
    let raw_info = leaked_account(key(10), false, false, false, raw);
    require_raw_programdata_hash(&raw_info, &expected).unwrap();
    raw_info.try_borrow_mut_data().unwrap()[48] ^= 1;
    assert_eq!(
        require_raw_programdata_hash(&raw_info, &expected),
        Err(GovernanceError::Release1DigestMismatch.into())
    );

    let header_len = 45usize;
    let old_capacity = 23usize;
    let delta = 11usize;
    let mut extended = vec![7u8; header_len + old_capacity];
    extended.resize(header_len + old_capacity + delta, 0);
    let extended_info = leaked_account(key(11), false, false, false, extended);
    validate_zero_appended_extension(
        &extended_info,
        header_len,
        old_capacity as u64,
        delta as u64,
    )
    .unwrap();
    extended_info.try_borrow_mut_data().unwrap()[header_len + old_capacity + delta - 1] = 1;
    assert_eq!(
        validate_zero_appended_extension(
            &extended_info,
            header_len,
            old_capacity as u64,
            delta as u64,
        ),
        Err(GovernanceError::Release1DigestMismatch.into())
    );
    assert_eq!(
        validate_zero_appended_extension(
            &extended_info,
            header_len,
            old_capacity as u64,
            (delta - 1) as u64,
        ),
        Err(GovernanceError::InvalidCapacityPlan.into())
    );
}

#[test]
fn canonical_envelope_rejects_siblings_reordering_and_wrong_current_index() {
    let program_id = key(12);
    let current_data = vec![30, 1, 2, 3];
    let envelope = envelope();
    let current = Instruction {
        program_id,
        accounts: vec![],
        data: current_data.clone(),
    };
    let valid = vec![
        compute_limit_instruction(&envelope),
        compute_price_instruction(&envelope),
        current.clone(),
    ];
    let valid_sysvar = instructions_sysvar(&valid, 2);
    validate_canonical_envelope(&program_id, &[], &valid_sysvar, &current_data, &envelope).unwrap();

    let mut sibling = valid.clone();
    sibling.push(Instruction {
        program_id: key(13),
        accounts: vec![],
        data: vec![1],
    });
    let sibling_sysvar = instructions_sysvar(&sibling, 2);
    assert_eq!(
        validate_canonical_envelope(&program_id, &[], &sibling_sysvar, &current_data, &envelope,),
        Err(GovernanceError::InvalidAccountCount.into())
    );

    let reordered = vec![
        compute_price_instruction(&envelope),
        compute_limit_instruction(&envelope),
        current,
    ];
    let reordered_sysvar = instructions_sysvar(&reordered, 2);
    assert_eq!(
        validate_canonical_envelope(
            &program_id,
            &[],
            &reordered_sysvar,
            &current_data,
            &envelope,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
    let wrong_index_sysvar = instructions_sysvar(&valid, 1);
    assert_eq!(
        validate_canonical_envelope(
            &program_id,
            &[],
            &wrong_index_sysvar,
            &current_data,
            &envelope,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn canonical_envelope_accepts_only_exact_optional_nonce_prefix() {
    let program_id = key(14);
    let nonce = key(15);
    let nonce_authority = key(16);
    let mut envelope = envelope();
    envelope.durable_nonce_account =
        crate::instruction::OptionalInstructionPubkeyV1::some(nonce).unwrap();
    envelope.durable_nonce_authority =
        crate::instruction::OptionalInstructionPubkeyV1::some(nonce_authority).unwrap();
    let current_data = vec![31, 9];
    let current = Instruction {
        program_id,
        accounts: vec![],
        data: current_data.clone(),
    };
    let valid = vec![
        system_instruction::advance_nonce_account(&nonce, &nonce_authority),
        compute_limit_instruction(&envelope),
        compute_price_instruction(&envelope),
        current,
    ];
    let valid_sysvar = instructions_sysvar(&valid, 3);
    validate_canonical_envelope(&program_id, &[], &valid_sysvar, &current_data, &envelope).unwrap();

    let mut wrong_nonce = valid;
    wrong_nonce[0] = system_instruction::advance_nonce_account(&key(17), &nonce_authority);
    let wrong_nonce_sysvar = instructions_sysvar(&wrong_nonce, 3);
    assert_eq!(
        validate_canonical_envelope(
            &program_id,
            &[],
            &wrong_nonce_sysvar,
            &current_data,
            &envelope,
        ),
        Err(GovernanceError::CrossAccountMismatch.into())
    );
}

#[test]
fn typed_loader_cpi_shapes_are_exact_and_closed() {
    let program = key(1);
    let programdata = derive_upgradeable_programdata_address(&program).0;
    let authority = key(2);
    let payer = key(3);
    let system = system_program::ID;
    let extend = extend_program_checked(&program, &authority, Some(&payer), 4096);
    validate_extend_cpi_shape(&extend, &programdata, &program, &authority, &system, &payer)
        .unwrap();
    let buffer = key(4);
    let spill = key(5);
    let upgrade = upgrade(&program, &buffer, &authority, &spill);
    validate_upgrade_cpi_shape(
        &upgrade,
        &programdata,
        &program,
        &buffer,
        &spill,
        &sysvar_ids::rent::ID,
        &sysvar_ids::clock::ID,
        &authority,
    )
    .unwrap();
    assert_eq!(upgrade.program_id, UPGRADEABLE_LOADER_ID);
    assert_eq!(upgrade.accounts.len(), 7);
}

#[test]
fn all_exports_reject_wrong_account_count_before_any_state_access() {
    let program_id = key(99);
    let expected = Err(GovernanceError::InvalidAccountCount.into());
    assert_eq!(
        process_extend_target_v1(&program_id, &[], extend_instruction()),
        expected
    );
    assert_eq!(
        process_execute_upgrade_v1(&program_id, &[], execute_instruction()),
        expected
    );
    assert_eq!(
        process_verify_programdata_chunk_v1(&program_id, &[], chunk_instruction()),
        expected
    );
    assert_eq!(
        process_finalize_programdata_verification_v1(&program_id, &[], finalize_instruction()),
        expected
    );
}

#[test]
fn every_export_rejects_privilege_drift_and_aliases_without_mutation() {
    let program_id = key(99);

    let mut extend = extend_accounts();
    extend[0].is_signer = false;
    assert_failed_without_mutation(
        &extend,
        Err(GovernanceError::InvalidAccountPrivileges.into()),
        |accounts| process_extend_target_v1(&program_id, accounts, extend_instruction()),
    );
    let mut extend_alias = extend_accounts();
    extend_alias[2] = extend_alias[1].clone();
    assert_failed_without_mutation(
        &extend_alias,
        Err(GovernanceError::CrossAccountMismatch.into()),
        |accounts| process_extend_target_v1(&program_id, accounts, extend_instruction()),
    );

    let mut execute = execute_accounts();
    execute[0].is_signer = false;
    assert_failed_without_mutation(
        &execute,
        Err(GovernanceError::InvalidAccountPrivileges.into()),
        |accounts| process_execute_upgrade_v1(&program_id, accounts, execute_instruction()),
    );
    let mut execute_alias = execute_accounts();
    execute_alias[2] = execute_alias[1].clone();
    assert_failed_without_mutation(
        &execute_alias,
        Err(GovernanceError::CrossAccountMismatch.into()),
        |accounts| process_execute_upgrade_v1(&program_id, accounts, execute_instruction()),
    );

    let mut chunk = chunk_accounts();
    chunk[0].is_writable = true;
    assert_failed_without_mutation(
        &chunk,
        Err(GovernanceError::InvalidAccountPrivileges.into()),
        |accounts| process_verify_programdata_chunk_v1(&program_id, accounts, chunk_instruction()),
    );
    let mut chunk_alias = chunk_accounts();
    chunk_alias[1] = chunk_alias[0].clone();
    assert_failed_without_mutation(
        &chunk_alias,
        Err(GovernanceError::CrossAccountMismatch.into()),
        |accounts| process_verify_programdata_chunk_v1(&program_id, accounts, chunk_instruction()),
    );

    let mut finalize = finalize_accounts();
    finalize[0].is_writable = true;
    assert_failed_without_mutation(
        &finalize,
        Err(GovernanceError::InvalidAccountPrivileges.into()),
        |accounts| {
            process_finalize_programdata_verification_v1(
                &program_id,
                accounts,
                finalize_instruction(),
            )
        },
    );
    let mut finalize_alias = finalize_accounts();
    finalize_alias[1] = finalize_alias[0].clone();
    assert_failed_without_mutation(
        &finalize_alias,
        Err(GovernanceError::CrossAccountMismatch.into()),
        |accounts| {
            process_finalize_programdata_verification_v1(
                &program_id,
                accounts,
                finalize_instruction(),
            )
        },
    );
}

fn proposal_expectation(state: ProposalStateV2) -> ProposalExpectationV2 {
    ProposalExpectationV2 {
        expected_proposal_digest: [1; 32],
        expected_policy_version: 1,
        expected_policy_hash: [2; 32],
        expected_council_version: 1,
        expected_council_hash: [3; 32],
        expected_gate_status: GateStatusV1::FrozenForUpgrade,
        expected_gate_epoch: 2,
        expected_target_nonce: 2,
        expected_state: state,
        expected_review_start_slot: 2,
        expected_review_end_slot: 3,
        expected_not_before_slot: 4,
        expected_expiry_slot: 5,
    }
}

fn extend_instruction() -> ExtendTargetV1 {
    ExtendTargetV1 {
        expected: proposal_expectation(ProposalStateV2::Frozen),
        expected_prestate_checkpoint_digest: [1; 32],
        expected_current_capacity: 1,
        expected_extension_delta: 1,
        expected_post_capacity: 2,
        envelope: envelope(),
    }
}

fn execute_instruction() -> ExecuteUpgradeV1 {
    ExecuteUpgradeV1 {
        expected: proposal_expectation(ProposalStateV2::Frozen),
        expected_prestate_checkpoint_digest: [1; 32],
        expected_current_raw_programdata_hash: [2; 32],
        expected_sealed_buffer_header_hash: [3; 32],
        expected_counterpart_proposal_digest: [4; 32],
        expected_programdata_slot: 1,
        expected_capacity: 1,
        expected_verified_chunk_count: 1,
        expected_buffer_verification_status: BufferVerificationStatusV1::Verified,
        expected_counterpart_buffer_verification_status: BufferVerificationStatusV1::Verified,
        envelope: envelope(),
    }
}

fn chunk_instruction() -> VerifyProgramDataChunkV1 {
    VerifyProgramDataChunkV1 {
        expected: proposal_expectation(ProposalStateV2::UpgradeExecuted),
        phase: ProgramDataChunkPhaseV1::ZeroTail,
        chunk_index: 0,
        proof: crate::instruction::FixedMerkleProofV1::empty(),
        expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
        expected_verified_payload_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_payload_chunk_count: 0,
        expected_verified_tail_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_tail_chunk_count: 0,
    }
}

fn finalize_instruction() -> FinalizeProgramDataVerificationV1 {
    FinalizeProgramDataVerificationV1 {
        expected: proposal_expectation(ProposalStateV2::UpgradeExecuted),
        expected_verification_status: ProgramDataVerificationStatusV1::ReadyToFinalize,
        expected_verified_payload_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_payload_chunk_count: 0,
        expected_verified_tail_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_tail_chunk_count: 0,
        expected_deployed_slot: 1,
        expected_capacity: 1,
    }
}

fn account_set(flags: &[(bool, bool, bool)], first_key: u8) -> Vec<AccountInfo<'static>> {
    flags
        .iter()
        .enumerate()
        .map(|(index, (writable, signer, executable))| {
            leaked_account(
                key(first_key.wrapping_add(index as u8)),
                *writable,
                *signer,
                *executable,
                vec![index as u8],
            )
        })
        .collect()
}

fn extend_accounts() -> Vec<AccountInfo<'static>> {
    account_set(
        &[
            (true, true, false),
            (false, false, false),
            (false, false, false),
            (true, false, false),
            (false, false, false),
            (true, false, false),
            (true, false, true),
            (true, false, false),
            (false, false, true),
            (false, false, true),
            (false, false, false),
            (false, false, false),
        ],
        20,
    )
}

fn execute_accounts() -> Vec<AccountInfo<'static>> {
    account_set(
        &[
            (true, true, false),
            (false, false, false),
            (false, false, false),
            (false, false, false),
            (true, false, false),
            (false, false, false),
            (false, false, false),
            (false, false, false),
            (true, false, false),
            (true, false, false),
            (true, false, false),
            (true, false, true),
            (true, false, false),
            (true, false, false),
            (false, false, false),
            (false, false, false),
            (false, false, false),
            (false, false, true),
            (false, false, true),
            (false, false, false),
        ],
        50,
    )
}

fn chunk_accounts() -> Vec<AccountInfo<'static>> {
    account_set(
        &[
            (false, false, false),
            (false, false, false),
            (false, false, false),
            (false, false, true),
            (false, false, false),
            (false, false, false),
            (false, false, true),
            (true, false, false),
        ],
        80,
    )
}

fn finalize_accounts() -> Vec<AccountInfo<'static>> {
    account_set(
        &[
            (false, false, false),
            (false, false, false),
            (true, false, false),
            (false, false, true),
            (false, false, false),
            (false, false, false),
            (false, false, true),
            (true, false, false),
        ],
        100,
    )
}

fn assert_failed_without_mutation(
    accounts: &[AccountInfo<'_>],
    expected: ProgramResult,
    call: impl FnOnce(&[AccountInfo<'_>]) -> ProgramResult,
) {
    let before: Vec<Vec<u8>> = accounts
        .iter()
        .map(|account| account.try_borrow_data().unwrap().to_vec())
        .collect();
    assert_eq!(call(accounts), expected);
    let after: Vec<Vec<u8>> = accounts
        .iter()
        .map(|account| account.try_borrow_data().unwrap().to_vec())
        .collect();
    assert_eq!(after, before);
}

fn envelope() -> EnvelopeExpectationV1 {
    EnvelopeExpectationV1 {
        compute_unit_limit: 1_000_000,
        compute_unit_price_micro_lamports: 1,
        durable_nonce_account: crate::instruction::OptionalInstructionPubkeyV1::none(),
        durable_nonce_authority: crate::instruction::OptionalInstructionPubkeyV1::none(),
    }
}
