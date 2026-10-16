use super::*;

/// A valid controller Config for `controller`, with `seats` at `epoch`.
fn seeded_config(controller: &Pubkey, seats: [Pubkey; 5], epoch: u64) -> (Pubkey, Vec<u8>) {
    let (key, bump) =
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], controller);
    let programdata = Pubkey::find_program_address(
        &[controller.as_ref()],
        &solana_sdk_ids::bpf_loader_upgradeable::ID,
    )
    .0;
    let authority =
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"authority"], controller).0;
    let mut data = vec![0u8; 384];
    data[..8].copy_from_slice(b"AG3CFG01");
    data[8] = 1;
    data[9] = bump;
    data[10] = 1;
    data[11..43].copy_from_slice(controller.as_ref());
    data[43..75].copy_from_slice(programdata.as_ref());
    data[75..107].copy_from_slice(authority.as_ref());
    data[107..139].copy_from_slice(Pubkey::new_unique().as_ref());
    for (index, seat) in seats.iter().enumerate() {
        data[139 + 32 * index..171 + 32 * index].copy_from_slice(seat.as_ref());
    }
    for (offset, value) in [
        (299, epoch),
        (307, 1),
        (315, ameba_governance_interface::MIN_REVIEW_SLOTS),
        (323, ameba_governance_interface::MIN_DELAY_SLOTS),
        (
            331,
            ameba_governance_interface::MIN_REVIEW_SLOTS
                .max(ameba_governance_interface::MIN_DELAY_SLOTS)
                + ameba_governance_interface::EXECUTION_MARGIN_SLOTS
                + 1,
        ),
        (339, 1),
    ] {
        data[offset..offset + 8].copy_from_slice(&u64::to_le_bytes(value));
    }
    (key, data)
}

#[test]
fn controller_config_decodes_only_with_its_exact_identity_layout_and_seats() {
    let controller = Pubkey::new_unique();
    let seats: [Pubkey; 5] = core::array::from_fn(|_| Pubkey::new_unique());
    let (key, data) = seeded_config(&controller, seats, 7);
    let view = decode_council(&controller, &key, &controller, &data).unwrap();
    assert_eq!(view.epoch, 7);
    assert_eq!(view.seats, seats);
    assert_eq!(
        view.digest,
        hashv(&[b"amoeba-council-seats-v1", &data[139..299]]).to_bytes()
    );

    // Owner, address and length bind the controller.
    assert!(decode_council(&controller, &key, &Pubkey::new_unique(), &data).is_err());
    assert!(decode_council(&controller, &Pubkey::new_unique(), &controller, &data).is_err());
    assert!(decode_council(&controller, &key, &controller, &data[..383]).is_err());
    let mut longer = data.clone();
    longer.push(0);
    assert!(decode_council(&controller, &key, &controller, &longer).is_err());

    let mutate = |at: usize, value: &[u8]| {
        let mut changed = data.clone();
        changed[at..at + value.len()].copy_from_slice(value);
        decode_council(&controller, &key, &controller, &changed)
    };
    for (at, value) in [
        (0usize, b"AG3CFG02".to_vec()),
        (8, vec![2]),
        (9, vec![data[9].wrapping_add(1)]),
        (10, vec![0]),
        (11, Pubkey::new_unique().to_bytes().to_vec()),
        (43, Pubkey::new_unique().to_bytes().to_vec()),
        (75, Pubkey::new_unique().to_bytes().to_vec()),
        (107, [0; 32].to_vec()),
        (107, key.to_bytes().to_vec()),
        (299, 0u64.to_le_bytes().to_vec()),
        (307, 0u64.to_le_bytes().to_vec()),
        (
            315,
            (ameba_governance_interface::MIN_REVIEW_SLOTS - 1)
                .to_le_bytes()
                .to_vec(),
        ),
        (
            323,
            (ameba_governance_interface::MIN_DELAY_SLOTS - 1)
                .to_le_bytes()
                .to_vec(),
        ),
        (
            331,
            (ameba_governance_interface::MIN_REVIEW_SLOTS
                .max(ameba_governance_interface::MIN_DELAY_SLOTS)
                + ameba_governance_interface::EXECUTION_MARGIN_SLOTS)
                .to_le_bytes()
                .to_vec(),
        ),
        (339, 0u64.to_le_bytes().to_vec()),
        (347, vec![1]),
        (383, vec![1]),
        // A default seat, a repeated seat, and a seat equal to the controller authority.
        (139, [0; 32].to_vec()),
        (171, seats[0].to_bytes().to_vec()),
        (139, data[75..107].to_vec()),
    ] {
        assert!(mutate(at, &value).is_err(), "byte {at}");
    }
}

#[test]
fn council_decision_is_an_absolute_three_seat_threshold() {
    for n in 0..1024 {
        let mut x = n;
        let mut ballots = [255; 5];
        for ballot in &mut ballots {
            let t = x % 4;
            *ballot = if t == 3 { 255 } else { t as u8 };
            x /= 4;
        }
        for fallback in 0..3 {
            let (out, mask, majority) = council_decision(&ballots, 3, fallback).unwrap();
            let expected = (0..3).find(|v| ballots.iter().filter(|b| **b == *v).count() >= 3);
            assert_eq!(out, expected.unwrap_or(fallback));
            assert_eq!(majority, expected.is_some());
            assert_eq!(mask.count_ones() >= 3, majority);
        }
    }
    assert!(council_decision(&[255; 5], 4, 0).is_err());
    assert!(council_decision(&[255; 5], 2, 2).is_err());
    assert!(council_decision(&[2, 255, 255, 255, 255], 2, 0).is_err());
}

#[test]
fn council_round_and_carry_registry_share_ocr_but_never_a_length() {
    assert_eq!(CouncilRound::LEN, 83);
    let round = CouncilRound {
        discriminator: *b"OCR",
        version: 1,
        initialized: true,
        bump: 255,
        case: Pubkey::new_unique(),
        epoch: 1,
        seats_hash: [1; 32],
        ballots: [255; 5],
    };
    let bytes = borsh::to_vec(&round).unwrap();
    assert_eq!(bytes.len(), CouncilRound::LEN);
    let program = Pubkey::new_unique();
    let key = Pubkey::new_unique();
    // A 78-byte "OCR" account (the carry registry's length) never loads as a round.
    let mut registry_sized = bytes.clone();
    registry_sized.truncate(78);
    let mut lamports = 1;
    let info = AccountInfo::new(
        &key,
        false,
        false,
        &mut lamports,
        &mut registry_sized,
        &program,
        false,
        0,
    );
    assert!(load_council_state::<CouncilRound>(
        &info,
        &program,
        CouncilRound::LEN,
        OracleError::InvalidOracleEmergencyDispute
    )
    .is_err());
}

#[test]
fn real_controller_config_and_profile_authorize_council() {
    use governance_controller_v3::state::{authority_pda, config_pda, Config, Timing};
    let controller = authority::pinned_controller().unwrap();
    let (key, bump) = config_pda(&controller);
    let seats = core::array::from_fn(|_| Pubkey::new_unique());
    let config = Config {
        discriminator: *b"AG3CFG01",
        version: 1,
        bump,
        initialized: true,
        controller,
        programdata: Pubkey::find_program_address(
            &[controller.as_ref()],
            &solana_sdk_ids::bpf_loader_upgradeable::ID,
        )
        .0,
        authority: authority_pda(&controller).0,
        treasury: Pubkey::new_unique(),
        seats,
        council_epoch: 2,
        timing_version: 1,
        timing: Timing::default(),
        next_id: 1,
        reserved: [0; 37],
    };
    config.validate(&controller, &key).unwrap();
    let mut data = config.try_to_vec().unwrap();
    let mut lamports = 1;
    let info = solana_program::account_info::AccountInfo::new(
        &key,
        false,
        false,
        &mut lamports,
        &mut data,
        &controller,
        false,
        0,
    );
    let view = authority::council(&info).unwrap();
    assert_eq!(view.epoch, 2);
    assert_eq!(view.seats, seats);
    assert_eq!(council_decision(&[1, 1, 1, 255, 255], 2, 0).unwrap().0, 1);
}

#[test]
#[cfg(feature = "localnet-governance")]
fn governed_localnet_never_reads_an_oracle_owned_dev_roster() {
    let seats = core::array::from_fn(|_| Pubkey::new_unique());
    let controller = crate::id();
    let (key, mut data) = seeded_config(&controller, seats, 1);
    let mut lamports = 1;
    let info = solana_program::account_info::AccountInfo::new(
        &key,
        false,
        false,
        &mut lamports,
        &mut data,
        &controller,
        false,
        0,
    );
    assert!(authority::council(&info).is_err());
}

#[test]
#[cfg(feature = "localnet-governance")]
fn governed_localnet_rejects_legacy_roster_instruction_before_accounts() {
    assert_eq!(
        crate::process_instruction(&crate::id(), &[], &[243]),
        Err(solana_program::program_error::ProgramError::Custom(
            OracleError::InvalidInstructionData as u32
        ))
    );
}

#[test]
#[cfg(feature = "synthetic-governance-controller")]
fn synthetic_initialization_uses_only_its_pinned_fixture_roster() {
    let controller = crate::governance_gate::PINNED_CONTROLLER_PROGRAM_ID;
    let seats = core::array::from_fn(|_| Pubkey::new_unique());
    let (config, data) = seeded_config(&controller, seats, 1);
    let keys = [config, seats[0], seats[1], seats[2]];
    let owners = [
        controller,
        solana_sdk_ids::system_program::ID,
        solana_sdk_ids::system_program::ID,
        solana_sdk_ids::system_program::ID,
    ];
    let mut lamports = [1; 4];
    let mut data = [data, vec![], vec![], vec![]];
    let accounts = keys
        .iter()
        .zip(owners.iter())
        .zip(lamports.iter_mut())
        .zip(data.iter_mut())
        .enumerate()
        .map(|(i, (((key, owner), balance), bytes))| {
            solana_program::account_info::AccountInfo::new(
                key,
                i != 0,
                false,
                balance,
                bytes,
                owner,
                false,
                0,
            )
        })
        .collect::<Vec<_>>();
    super::super::config::require_bootstrap_council(&accounts).unwrap();
    let mut missing = accounts.clone();
    missing[3].is_signer = false;
    assert!(super::super::config::require_bootstrap_council(&missing).is_err());
    let duplicate = [
        accounts[0].clone(),
        accounts[1].clone(),
        accounts[1].clone(),
        accounts[3].clone(),
    ];
    assert!(super::super::config::require_bootstrap_council(&duplicate).is_err());
}
