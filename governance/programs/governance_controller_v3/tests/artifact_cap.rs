//! 2 MiB artifact ceiling: bounds, 128-bit bitmap layout, Merkle parity and
//! the retired checked-extension tags. Pinned roots come from an independent
//! Python implementation of the Release 1 scheme, not from this crate.
use borsh::{BorshDeserialize, BorshSerialize};
use governance_controller_v3::{
    artifact::{self, CHUNK, MAX_ARTIFACT_BYTES, MAX_CHUNKS},
    instruction::Instruction,
    state::*,
};
use solana_program::{program_error::ProgramError, pubkey::Pubkey};
use upgrade_controller::artifact_merkle::{
    artifact_merkle_proof, artifact_merkle_root, MAX_ARTIFACT_BYTES_V1,
    RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
};

const PINNED: [(usize, &str); 4] = [
    (
        2_097_152,
        "209812d102a353ca780529085472fa3546bd104a5e60d965c35fced89027a854",
    ),
    (
        2_097_151,
        "4d0aa1710a4fcaa3470683e3dae310c33019554fe52ca94ae61d8a00f5701a96",
    ),
    (
        1_572_865,
        "01ab5fb8c5946f03f231b5113075d2fa1e702fb017068597b9fe5cfbd7646666",
    ),
    (
        1_572_864,
        "e9e6f420426a7a29f7f6b5516d9425223989871d5be59c27ab228eb2e3cdad7b",
    ),
];

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn upgrade_action(kind: u8, artifact_length: u64, capacity: u64) -> Action {
    let mut data = [0; ACTION_LEN];
    if kind == UPGRADE_CONTROLLER {
        let body = Upgrade {
            buffer: Pubkey::new_from_array([7; 32]),
            artifact_length,
            artifact_sha256: [1; 32],
            merkle_root: [2; 32],
            deployed_slot: 9,
            capacity,
            source_commitment: [3; 32],
            build_commitment: [4; 32],
            controller_release: CONTROLLER_RELEASE,
        };
        data.copy_from_slice(&body.try_to_vec().unwrap());
    } else {
        let body = TargetUpgrade {
            buffer: Pubkey::new_from_array([7; 32]),
            artifact_length,
            merkle_root: [2; 32],
            deployed_slot: 9,
            capacity,
            source_commitment: [3; 32],
            build_commitment: [4; 32],
            target: Pubkey::new_from_array([8; 32]),
            gate_epoch: 1,
        };
        data.copy_from_slice(&body.try_to_vec().unwrap());
    }
    Action { kind, data }
}

fn upgrade_proposal(artifact_length: u64) -> (Pubkey, Proposal) {
    let program = Pubkey::new_from_array([31; 32]);
    let timing = Timing::default();
    let (review_end, not_before, expires) = timing.boundaries(100).unwrap();
    let mut p = Proposal {
        discriminator: *b"AG3PRP01",
        version: 1,
        bump: proposal_pda(&program, 1).1,
        initialized: true,
        config: config_pda(&program).0,
        id: 1,
        council_epoch: 1,
        timing_version: 1,
        timing,
        created: 100,
        review_end,
        not_before,
        expires,
        action: upgrade_action(UPGRADE_TARGET, artifact_length, artifact_length),
        digest: [0; 32],
        approvals: 0,
        approval_count: 0,
        cancellations: 0,
        cancellation_count: 0,
        state: PENDING,
        verified_chunks: [0; 12],
        verified_count: 0,
        extension_slot: 0,
        extended_capacity: 0,
        executed_slot: 0,
        verified_chunks_high: [0; 4],
        reserved: [0; 3],
    };
    p.digest = p.compute_digest(&program).unwrap();
    (program, p)
}

#[test]
fn ceiling_is_exactly_two_mib_of_sixteen_kib_chunks() {
    assert_eq!(MAX_ARTIFACT_BYTES, 2_097_152);
    assert_eq!(MAX_CHUNKS, 128);
    assert_eq!(CHUNK, RELEASE1_ARTIFACT_CHUNK_SIZE_V1);
    // The historical Release 1 controller keeps its own pinned 1.5 MiB bound.
    assert_eq!(MAX_ARTIFACT_BYTES_V1, 1_572_864);
    for (len, chunks) in [
        (1, 1),
        (16_384, 1),
        (16_385, 2),
        (1_572_864, 96),
        (1_572_865, 97),
        (2_097_151, 128),
        (2_097_152, 128),
    ] {
        assert_eq!(artifact::chunk_count(len).unwrap(), chunks, "{len}");
    }
    for len in [0, 2_097_153, u64::MAX] {
        assert!(artifact::chunk_count(len).is_err(), "{len}");
    }
}

#[test]
fn upgrade_actions_admit_two_mib_and_reject_one_byte_more() {
    for kind in [UPGRADE_CONTROLLER, UPGRADE_TARGET] {
        upgrade_action(kind, 2_097_152, 2_097_152)
            .validate()
            .unwrap();
        upgrade_action(kind, 2_097_151, 2_097_152)
            .validate()
            .unwrap();
        upgrade_action(kind, 1_572_865, 1_572_865)
            .validate()
            .unwrap();
        // Capacity below length is an informational snapshot: anyone may grow
        // ProgramData top-level before installation.
        upgrade_action(kind, 2_097_152, 1_457_072)
            .validate()
            .unwrap();
        assert!(upgrade_action(kind, 2_097_153, 2_097_152)
            .validate()
            .is_err());
        assert!(upgrade_action(kind, 0, 1).validate().is_err());
        assert!(upgrade_action(kind, 1, 0).validate().is_err());
    }
}

#[test]
fn programdata_grown_by_anyone_never_invalidates_an_action() {
    // Anyone can grow ProgramData with the permissionless top-level Loader-v3
    // ExtendProgram, up to the loader's 10 MiB account limit. The capacity
    // snapshot is informational and has no upper bound.
    for kind in [UPGRADE_CONTROLLER, UPGRADE_TARGET] {
        for capacity in [1_572_865, 2_097_153, 10 * 1024 * 1024 - 45, u64::MAX] {
            upgrade_action(kind, 2_097_152, capacity)
                .validate()
                .unwrap();
        }
    }
}

#[test]
fn controller_actions_bind_the_controller_release_in_former_zero_bytes() {
    assert_eq!(CONTROLLER_RELEASE, 2, "bump on every controller release");
    let action = upgrade_action(UPGRADE_CONTROLLER, 224_776, 224_776);
    assert_eq!(&action.data[184..], &CONTROLLER_RELEASE.to_le_bytes());
    assert_eq!(action.upgrade().unwrap().controller_release, 2);
    // A proposal created by the deployed controller carries zeros here. It
    // still decodes as release 0, so it can expire and release its buffer.
    let mut legacy = action.clone();
    legacy.data[184..].fill(0);
    legacy.validate().unwrap();
    assert_eq!(legacy.upgrade().unwrap().controller_release, 0);
    assert!(upgrade_action(UPGRADE_TARGET, 224_776, 224_776)
        .upgrade()
        .is_err());
}

#[test]
fn bitmap_extends_into_former_reserved_bytes_without_moving_any_field() {
    let (program, mut p) = upgrade_proposal(2_097_152);
    let key = proposal_pda(&program, 1).0;
    p.validate(&program, &key).unwrap();
    let empty = p.try_to_vec().unwrap();
    assert_eq!(empty.len(), PROPOSAL_LEN);
    // A proposal written by the 1.5 MiB controller has seven zero reserved
    // bytes at 393..400; it decodes unchanged with an empty high bitmap.
    assert_eq!(&empty[393..], &[0; 7]);
    let old = Proposal::try_from_slice(&empty).unwrap();
    assert_eq!(old.verified_chunks_high, [0; 4]);
    old.validate(&program, &key).unwrap();

    for index in 0..MAX_CHUNKS as u32 {
        p.mark_chunk_verified(index).unwrap();
        assert!(p.mark_chunk_verified(index).is_err(), "duplicate {index}");
    }
    assert!(p.mark_chunk_verified(128).is_err());
    assert!(p.mark_chunk_verified(u32::MAX).is_err());
    assert_eq!(p.verified_count, 128);
    assert_eq!(p.verified_bitmap(), [0xff; 16]);
    p.validate(&program, &key).unwrap();
    let full = p.try_to_vec().unwrap();
    assert_eq!(full.len(), PROPOSAL_LEN);
    // Low twelve bytes keep their offset; high four sit after executed_slot.
    assert_eq!(&full[353..365], &[0xff; 12]);
    assert_eq!(&full[365..369], &128u32.to_le_bytes());
    assert_eq!(&full[393..397], &[0xff; 4]);
    assert_eq!(&full[397..], &[0; 3]);
    // Every other byte matches the empty proposal.
    for (i, (a, b)) in empty.iter().zip(&full).enumerate() {
        if !(353..369).contains(&i) && !(393..397).contains(&i) {
            assert_eq!(a, b, "byte {i}");
        }
    }
    // Remaining reserved bytes stay zero, and the count must match every bit.
    for i in 397..400 {
        let mut bad = full.clone();
        bad[i] = 1;
        let decoded = Proposal::try_from_slice(&bad).unwrap();
        assert!(decoded.validate(&program, &key).is_err());
    }
    let mut bad = full.clone();
    bad[396] = 0x7f;
    assert!(Proposal::try_from_slice(&bad)
        .unwrap()
        .validate(&program, &key)
        .is_err());
    // Policy proposals can never carry verified bits in either half.
    let (_, mut policy) = upgrade_proposal(16_384);
    let mut timing = [0; ACTION_LEN];
    timing[..24].copy_from_slice(&Timing::default().try_to_vec().unwrap());
    policy.action = Action {
        kind: SET_TIMING,
        data: timing,
    };
    policy.digest = policy.compute_digest(&program).unwrap();
    policy.validate(&program, &key).unwrap();
    policy.verified_chunks_high[3] = 0x80;
    policy.verified_count = 1;
    assert!(policy.validate(&program, &key).is_err());
}

#[test]
fn merkle_scheme_is_unchanged_and_pinned_through_two_mib() {
    let full = pattern(2_097_152);
    for (len, expected) in PINNED {
        assert_eq!(hex(&artifact::merkle_root(&full[..len]).unwrap()), expected);
    }
    // Byte-identical to the Release 1 implementation wherever both apply.
    for len in [1, 16_383, 16_384, 16_385, 1_000_000, 1_572_863, 1_572_864] {
        let data = &full[..len];
        assert_eq!(
            artifact::merkle_root(data).unwrap(),
            artifact_merkle_root(data, CHUNK).unwrap(),
            "{len}"
        );
        let last = artifact::chunk_count(len as u64).unwrap() - 1;
        for index in [0, last] {
            assert_eq!(
                artifact::merkle_proof(data, index).unwrap(),
                artifact_merkle_proof(data, CHUNK, index).unwrap()
            );
        }
    }
    assert!(artifact_merkle_root(&full[..1_572_865], CHUNK).is_err());
    assert!(artifact::merkle_root(&pattern(2_097_153)).is_err());
    assert!(artifact::merkle_root(&[]).is_err());
}

#[test]
fn every_chunk_verifies_at_the_new_boundaries_and_forgeries_fail() {
    let full = pattern(2_097_152);
    for len in [2_097_152usize, 2_097_151, 1_572_865, 1_589_249] {
        let data = &full[..len];
        let root = artifact::merkle_root(data).unwrap();
        let count = artifact::chunk_count(len as u64).unwrap();
        for index in 0..count {
            let start = index as usize * CHUNK as usize;
            let chunk = &data[start..(start + CHUNK as usize).min(len)];
            let proof = artifact::merkle_proof(data, index).unwrap();
            assert_eq!(proof.len(), 7);
            artifact::verify_chunk(&root, len as u64, index, chunk, &proof).unwrap();
        }
        let last = count - 1;
        let start = last as usize * CHUNK as usize;
        let chunk = &data[start..];
        let proof = artifact::merkle_proof(data, last).unwrap();
        // Out-of-range index, short/long chunk, short proof, wrong root.
        assert!(artifact::verify_chunk(&root, len as u64, count, chunk, &proof).is_err());
        assert!(artifact::verify_chunk(&root, len as u64, last, &chunk[1..], &proof).is_err());
        assert!(artifact::verify_chunk(&root, len as u64, last, chunk, &proof[..6]).is_err());
        assert!(artifact::verify_chunk(&[9; 32], len as u64, last, chunk, &proof).is_err());
        // A length-bound root cannot be reused for a different length.
        assert!(artifact::verify_chunk(&root, len as u64 + 1, last, chunk, &proof).is_err());
        for level in 0..7 {
            let mut forged = proof.clone();
            forged[level][0] ^= 1;
            assert!(
                artifact::verify_chunk(&root, len as u64, last, chunk, &forged).is_err(),
                "{len} level {level}"
            );
        }
    }
    assert!(artifact::verify_chunk(&[0; 32], 2_097_153, 0, &[0; 16_384], &[[0; 32]; 7]).is_err());
}

#[test]
fn retired_checked_extension_tags_fail_like_unknown_tags() {
    // `enable_extend_program_checked` was cancelled upstream, so tags 10
    // (ExtendController) and 13 (ExtendTarget) stay closed exactly as in the
    // deployed controller: rejected before Borsh decoding or account access,
    // even with a well-formed 32-byte payload.
    let program = ameba_governance_interface::PINNED_CONTROLLER_PROGRAM_ID;
    for tag in [10u8, 13] {
        let mut data = vec![tag];
        data.extend_from_slice(&[5; 32]);
        assert!(matches!(
            Instruction::unpack(&data),
            Err(ProgramError::InvalidInstructionData)
        ));
        assert_eq!(
            governance_controller_v3::process_instruction(&program, &[], &data),
            Err(ProgramError::InvalidInstructionData)
        );
    }
    for tag in 17..=255u8 {
        assert!(Instruction::unpack(&[tag]).is_err());
    }
    // Every live tag still decodes at its original Borsh position.
    for tag in [8u8, 9, 11, 12, 14, 15, 16] {
        let mut data = vec![tag];
        if tag != 11 {
            data.extend_from_slice(&[5; 32]);
        }
        let decoded = Instruction::unpack(&data).unwrap();
        assert_eq!(decoded.try_to_vec().unwrap(), data);
    }
}
