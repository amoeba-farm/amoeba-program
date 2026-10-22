use borsh::{BorshDeserialize, BorshSerialize};
use governance_controller_v3::{instruction::Instruction, state::*};
use solana_program::pubkey::Pubkey;

fn sample() -> (Pubkey, Config, Proposal) {
    let program = Pubkey::new_from_array([31; 32]);
    let (config_key, config_bump) = config_pda(&program);
    let config = Config {
        discriminator: *b"AG3CFG01",
        version: 1,
        bump: config_bump,
        initialized: true,
        controller: program,
        programdata: Pubkey::find_program_address(
            &[program.as_ref()],
            &solana_sdk_ids::bpf_loader_upgradeable::ID,
        )
        .0,
        authority: authority_pda(&program).0,
        treasury: Pubkey::new_from_array([32; 32]),
        seats: std::array::from_fn(|i| Pubkey::new_from_array([40 + i as u8; 32])),
        council_epoch: 1,
        timing_version: 1,
        timing: Timing::default(),
        next_id: 2,
        reserved: [0; 37],
    };
    let mut data = [0; 192];
    data[..24].copy_from_slice(&Timing::default().try_to_vec().unwrap());
    let (review_end, not_before, expires) = config.timing.boundaries(100).unwrap();
    let mut p = Proposal {
        discriminator: *b"AG3PRP01",
        version: 1,
        bump: proposal_pda(&program, 1).1,
        initialized: true,
        config: config_key,
        id: 1,
        council_epoch: 1,
        timing_version: 1,
        timing: config.timing,
        created: 100,
        review_end,
        not_before,
        expires,
        action: Action {
            kind: SET_TIMING,
            data,
        },
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
    (program, config, p)
}

#[test]
fn one_week_approval_is_independent_from_execution_delay_and_cannot_be_shortened() {
    let (_, mut config, mut p) = sample();
    assert_eq!(p.review_end - p.created, WEEK_SLOTS);
    assert_eq!(p.not_before - p.created, MIN_DELAY_SLOTS);
    for i in 0..3 {
        p.approve(i, 100 + MIN_DELAY_SLOTS, false).unwrap();
    }
    assert!(p.executable(&config, p.not_before - 1).is_err());
    p.executable(&config, p.not_before).unwrap();
    p.approve(3, 100 + WEEK_SLOTS, false).unwrap();
    assert!(p.approve(4, 100 + WEEK_SLOTS + 1, false).is_err());
    let frozen = p.timing;
    config.timing.review_slots = 2 * WEEK_SLOTS;
    config.timing.expiry_slots = 5_184_000;
    config.timing_version = 2;
    assert_eq!(p.timing, frozen);
    p.executable(&config, p.not_before).unwrap();
    let mut weak = Timing {
        review_slots: WEEK_SLOTS - 1,
        ..Timing::default()
    };
    assert!(weak.validate().is_err());
    weak.review_slots = WEEK_SLOTS;
    weak.expiry_slots = WEEK_SLOTS + EXECUTION_MARGIN_SLOTS;
    assert!(weak.validate().is_err());
    assert!(Timing::default().boundaries(u64::MAX - 5).is_err());
}

#[test]
fn every_three_seat_coalition_and_no_two_can_execute_and_stale_council_fails() {
    let (_, config, base) = sample();
    for mask in 0u8..32 {
        let mut p = base.clone();
        for i in 0..5 {
            if mask & (1 << i) != 0 {
                p.approve(i, p.created, false).unwrap();
            }
        }
        assert_eq!(
            p.executable(&config, p.not_before).is_ok(),
            mask.count_ones() >= 3
        );
    }
    let mut p = base;
    p.approve(0, p.created, false).unwrap();
    assert!(p.approve(0, p.created, false).is_err());
    p.approve(1, p.created, false).unwrap();
    p.approve(2, p.created, false).unwrap();
    let mut rotated = config.clone();
    rotated.council_epoch += 1;
    assert!(p.executable(&rotated, p.not_before).is_err());
    for i in 0..3 {
        p.approve(i, p.not_before, true).unwrap();
    }
    assert_eq!(p.state, CANCELLED);
    assert!(p.executable(&config, p.not_before).is_err());
}

#[test]
fn fixed_accounts_and_digest_reject_timing_action_padding_and_trailing_drift() {
    let (program, config, p) = sample();
    // The development profile binds a different genesis, timing snapshot,
    // action payload and derived proposal boundaries into the same wire digest.
    #[cfg(feature = "localnet-governance")]
    let expected_digest = "d22b414920bbdb103a13c4f04327da93ae291f92a8596c9fec4e1e48b56e245e";
    #[cfg(not(feature = "localnet-governance"))]
    let expected_digest = "b9efe77bf526500f8777e10bd8884dda326989bca4a930b133ca0dee669554d3";
    assert_eq!(
        p.digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        expected_digest
    );
    config.validate(&program, &config_pda(&program).0).unwrap();
    p.validate(&program, &proposal_pda(&program, 1).0).unwrap();
    assert_eq!(config.try_to_vec().unwrap().len(), 384);
    assert_eq!(p.try_to_vec().unwrap().len(), 400);
    let mut altered = p.clone();
    altered.review_end += 1;
    assert!(altered
        .validate(&program, &proposal_pda(&program, 1).0)
        .is_err());
    altered = p.clone();
    altered.action.data[191] = 1;
    assert!(altered.action.validate().is_err());
    let mut encoded = p.try_to_vec().unwrap();
    encoded.push(0);
    assert!(Proposal::try_from_slice(&encoded).is_err());
    let mut duplicate = config;
    duplicate.seats[4] = duplicate.seats[0];
    assert!(duplicate
        .validate(&program, &config_pda(&program).0)
        .is_err());
}

#[test]
fn closed_dispatch_rejects_every_unknown_tag_before_accounts() {
    let program = ameba_governance_interface::PINNED_CONTROLLER_PROGRAM_ID;
    // Tags 10 and 13 (retired checked extension) fail exactly like unknown tags.
    for tag in [10u8, 13].into_iter().chain(20..=255u8) {
        assert_eq!(
            governance_controller_v3::process_instruction(&program, &[], &[tag]),
            Err(solana_program::program_error::ProgramError::InvalidInstructionData)
        );
    }
    assert!(Instruction::unpack(&vec![0; MAX_DATA + 1]).is_err());
    assert!(Instruction::unpack(&[6, 0]).is_err());
    assert!(Instruction::unpack(&[]).is_err());
}

#[test]
fn verification_policy_fixed_wire_and_existing_governance_rules() {
    use governance_controller_v3::verification::*;
    let (program, config, mut proposal) = sample();
    let policy = VerificationPolicy {
        target: program,
        deployed_slot: 123,
        commit: [0xab; 20],
    };
    let body = policy.try_to_vec().unwrap();
    assert_eq!(body.len(), 60);
    proposal.action = Action {
        kind: PUBLISH_VERIFICATION,
        data: [0; 192],
    };
    proposal.action.data[..60].copy_from_slice(&body);
    assert_eq!(
        proposal.action.try_to_vec().unwrap(),
        [
            vec![6],
            vec![31; 32],
            123u64.to_le_bytes().to_vec(),
            vec![0xab; 20],
            vec![0; 132]
        ]
        .concat()
    );
    proposal.action.validate().unwrap();
    assert!(!proposal.action.is_upgrade());
    for offset in 60..192 {
        let mut bad = proposal.action.clone();
        bad.data[offset] = 1;
        assert!(bad.validate().is_err());
    }
    let params = policy.params(&program).unwrap();
    assert_eq!(params.len(), 311);
    assert_eq!(
        solana_program::hash::hash(&params).to_bytes(),
        [
            0xe6, 0x23, 0xee, 0x1b, 0x4b, 0x62, 0xa0, 0xc9, 0xb0, 0x3e, 0xc5, 0xb8, 0xc8, 0x15,
            0xa2, 0x53, 0x97, 0x55, 0x87, 0xce, 0x60, 0xcd, 0xe6, 0xeb, 0xad, 0x43, 0xf3, 0xda,
            0x34, 0x88, 0xd1, 0x67,
        ]
    );
    assert!(params
        .windows(b"https://github.com/amoeba-farm/amoeba-governance".len())
        .any(|w| w == b"https://github.com/amoeba-farm/amoeba-governance"));
    assert!(!params.windows(8).any(|w| w == b"api-key="));
    let foreign = VerificationPolicy {
        target: Pubkey::new_unique(),
        ..policy.clone()
    };
    assert!(foreign.params(&program).is_err());
    let zero_commit = VerificationPolicy {
        commit: [0; 20],
        ..policy.clone()
    };
    assert!(zero_commit.validate().is_err());
    let target = VerificationPolicy {
        target: DEVNET_SPREAD,
        ..policy.clone()
    };
    let target_params = target.params(&program).unwrap();
    assert_eq!(target_params.len(), 434);
    assert_eq!(
        solana_program::hash::hash(&target_params).to_bytes(),
        [
            0xc7, 0xa8, 0xfb, 0xcd, 0x45, 0xc2, 0x37, 0xe9, 0x11, 0xd6, 0xf4, 0xb7, 0x29, 0x7b,
            0x3b, 0x13, 0x17, 0xb8, 0xcb, 0x34, 0x82, 0x3f, 0x5d, 0x91, 0x0b, 0xb8, 0xcc, 0x35,
            0xa0, 0xd2, 0xd9, 0xcc,
        ]
    );
    let encoded = Instruction::PublishVerification { digest: [42; 32] }
        .try_to_vec()
        .unwrap();
    assert_eq!(encoded, [vec![16], vec![42; 32]].concat());
    assert!(Instruction::unpack(&encoded).is_ok());
    assert!(Instruction::unpack(&encoded[..32]).is_err());
    assert!(Instruction::unpack(&[encoded, vec![0]].concat()).is_err());
    proposal.digest = proposal.compute_digest(&program).unwrap();
    let key = proposal_pda(&program, proposal.id).0;
    proposal.validate(&program, &key).unwrap();
    let digest = proposal.digest;
    for offset in [0, 32, 40] {
        let mut changed = proposal.clone();
        changed.action.data[offset] ^= 1;
        assert_ne!(changed.compute_digest(&program).unwrap(), digest);
    }
    for mask in 0..32u8 {
        let mut p = proposal.clone();
        p.approvals = mask;
        p.approval_count = mask.count_ones() as u8;
        assert_eq!(
            p.executable(&config, p.not_before).is_ok(),
            mask.count_ones() >= 3
        );
        assert!(p.executable(&config, p.not_before - 1).is_err());
        assert!(p.executable(&config, p.expires).is_err());
        let mut newer = config.clone();
        newer.council_epoch += 1;
        assert!(p.executable(&newer, p.not_before).is_err());
    }
}

#[test]
fn spread_light_policy_has_fixed_bytes_and_rejects_reserved_or_foreign_targets() {
    let policy = SpreadLightConfigPolicy {
        target: DEVNET_SPREAD,
        expected_epoch: 3,
        deployed_slot: 10,
        compression_authority: Pubkey::new_from_array([42; 32]),
        rent: LightRentPolicy {
            base_rent: 123,
            compression_cost: 456,
            lamports_per_byte_per_epoch: 7,
            max_funded_epochs: 8,
            max_top_up: 901,
        },
        write_top_up: 2345,
        address_tree: Pubkey::new_from_array([43; 32]),
    };
    let body = policy.try_to_vec().unwrap();
    assert_eq!(body.len(), 124);
    assert_eq!(
        &body[32..48],
        &[3, 0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(&body[80..92], &[123, 0, 200, 1, 7, 8, 133, 3, 41, 9, 0, 0]);
    let mut action = Action {
        kind: INITIALIZE_SPREAD_LIGHT_CONFIG,
        data: [0; 192],
    };
    action.data[..124].copy_from_slice(&body);
    action.validate().unwrap();
    assert_eq!(action.try_to_vec().unwrap().len(), 193);
    assert!(!action.is_upgrade());
    action.data[124] = 1;
    assert!(action.validate().is_err());
    action.data[124] = 0;
    action.data[0] ^= 1;
    assert!(action.validate().is_err());
    action.data[0] ^= 1;
    action.data[32..40].fill(0);
    assert!(action.validate().is_err());
    let encoded = Instruction::ExecuteSpreadLightConfig { digest: [23; 32] }
        .try_to_vec()
        .unwrap();
    assert_eq!(encoded, [vec![15], vec![23; 32]].concat());
    Instruction::unpack(&encoded).unwrap();
    assert!(Instruction::unpack(&[encoded, vec![0]].concat()).is_err());
}

#[test]
fn retirement_policy_is_fixed_and_preserves_quorum_and_timing() {
    let (program, config, mut p) = sample();
    let policy = RetireTargetPolicy {
        target: Pubkey::new_from_array([42; 32]),
        expected_epoch: 44,
        deployed_slot: 400,
        payload_sha256: [43; 32],
        recovery_commitment: [44; 32],
    };
    let bytes = policy.try_to_vec().unwrap();
    assert_eq!(bytes.len(), 112);
    assert_eq!(
        bytes,
        [
            vec![42; 32],
            44u64.to_le_bytes().to_vec(),
            400u64.to_le_bytes().to_vec(),
            vec![43; 32],
            vec![44; 32]
        ]
        .concat()
    );
    p.action = Action {
        kind: RETIRE_TARGET,
        data: [0; 192],
    };
    p.action.data[..112].copy_from_slice(&bytes);
    p.digest = p.compute_digest(&program).unwrap();
    p.validate(&program, &proposal_pda(&program, p.id).0)
        .unwrap();
    for mask in 0..32u8 {
        let mut candidate = p.clone();
        candidate.approvals = mask;
        candidate.approval_count = mask.count_ones() as u8;
        assert_eq!(
            candidate.executable(&config, candidate.not_before).is_ok(),
            mask.count_ones() >= 3
        );
        assert!(candidate
            .executable(&config, candidate.not_before - 1)
            .is_err());
        assert!(candidate.executable(&config, candidate.expires).is_err());
    }
    for offset in [0, 32, 40, 48, 80] {
        let mut changed = p.clone();
        changed.action.data[offset] ^= 1;
        assert_ne!(changed.compute_digest(&program).unwrap(), p.digest);
    }
    let mut bad = p.action.clone();
    bad.data[112] = 1;
    assert!(bad.validate().is_err());
    for ix in [
        Instruction::RetireTarget { digest: [42; 32] },
        Instruction::ReclaimRetiredTargetRecords {
            target: policy.target,
        },
    ] {
        let encoded = ix.try_to_vec().unwrap();
        assert_eq!(encoded.len(), 33);
        assert!(matches!(encoded[0], 17..=19));
        assert!(Instruction::unpack(&encoded).is_ok());
        assert!(Instruction::unpack(&encoded[..32]).is_err());
        assert!(Instruction::unpack(&[encoded, vec![0]].concat()).is_err());
    }
}
