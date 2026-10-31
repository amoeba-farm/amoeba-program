use super::*;
use crate::fixed_codec::{FixedStateEncode, ReferenceBorsh};

fn assert_codec_matches_borsh<T>(value: &T, wire_len: usize)
where
    T: BorshSerialize
        + BorshDeserialize
        + FixedStateEncode
        + ReferenceBorsh
        + core::fmt::Debug
        + PartialEq,
{
    let encoded = borsh::to_vec(value).unwrap();
    assert_eq!(encoded.len(), wire_len);
    assert_eq!(encoded, value.reference_borsh_bytes());
    let mut direct = vec![0; value.maximum_encoded_len()];
    value.encode_fixed(&mut direct);
    assert_eq!(direct, encoded);
    assert_eq!(&T::try_from_slice(&encoded).unwrap(), value);
    assert!(T::try_from_slice(&encoded[..encoded.len() - 1]).is_err());
}

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn header<T: HeadsAccount>() -> ([u8; 8], u8) {
    (T::DISCRIMINATOR, T::VERSION)
}

fn config() -> HeadsConfig {
    let (discriminator, version) = header::<HeadsConfig>();
    HeadsConfig {
        discriminator,
        version,
        bump: 254,
        authority_bump: 253,
        amba_decimals: 9,
        amba_mint: key(1),
        amba_token_program: crate::token_instruction::TOKEN_2022_PROGRAM_ID,
        amba_vault: key(2),
        amba_accounted: 7,
        pending_amba_burn: 8,
        amba_burned_total: 9,
        board_count: 2,
        created_ts: 10,
        updated_ts: 11,
    }
}

fn board() -> HeadsBoard {
    let (discriminator, version) = header::<HeadsBoard>();
    let mut b = HeadsBoard {
        discriminator,
        version,
        bump: 250,
        product_id: *b"RAMX\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        params: HeadsParams::spec_defaults(),
        params_version: 1,
        item_count: 2,
        component_count: 6,
        head_count: 9,
        total_delegation: 1_000,
        live_delegation: 600,
        live_count: 3,
        reward_index: u128::MAX / 3,
        contract_count: 1,
        ..HeadsBoard::default()
    };
    b.item_bucket_ids[0] = [4; 32];
    b.item_bucket_ids[1] = [5; 32];
    b.item_weight_bps[..2].copy_from_slice(&[6_000, 4_000]);
    b.item_mix_bps[..6].copy_from_slice(&[2_000, 5_000, 3_000, 0, 10_000, 0]);
    b.item_mix_basis[..2]
        .copy_from_slice(&[HeadsMixBasis::Assumed as u8, HeadsMixBasis::None as u8]);
    // Three components per item, at item * 3 + channel; item 1's LTA sentence is still unset.
    b.component_items[..6].copy_from_slice(&[0, 0, 0, 1, 1, 1]);
    b.component_channels[..6].copy_from_slice(&[0, 1, 2, 0, 1, 2]);
    for c in 0..5 {
        b.component_sentence_hashes[c] = [20 + c as u8; 32];
    }
    b.component_live_sources[..6].copy_from_slice(&[0, 3, 0, 0, 0, 0]);
    b.component_live_origins[..6].copy_from_slice(&[0, 2, 0, 0, 0, 0]);
    b.live_head_ids[..3].copy_from_slice(&[1, 4, 7]);
    b.live_components[..3].copy_from_slice(&[1, 1, 1]);
    b.live_flags[..3].copy_from_slice(&[0, 0, LIVE_DUPLICATE]);
    b.live_delegations[..3].copy_from_slice(&[300, 300, 100]);
    b.live_values[..3].copy_from_slice(&[100_000_000, 101_000_000, 99_000_000]);
    b.live_origin_ids[0] = [31; 32];
    b.live_origin_ids[1] = [32; 32];
    b.live_origin_ids[2] = [31; 32];
    b.live_since_ts[..3].copy_from_slice(&[50, 60, 70]);
    b.live_reward_offsets[..3].copy_from_slice(&[1, 2, u128::MAX]);
    b.contract_periods[1] = key(40);
    b.contract_listing_ts[1] = 100;
    b.contract_freeze_ts[1] = 200;
    b.contract_expiry_ts[1] = 300;
    b.contract_status[1] = RING_COLLECTING;
    b.contract_flags[1] = RING_FREEZE_CAPTURED;
    b.contract_freeze_origins[HEADS_MAX_COMPONENTS + 1] = 2;
    b
}

fn head() -> Head {
    let (discriminator, version) = header::<Head>();
    let mut h = Head {
        discriminator,
        version,
        bump: 251,
        board: key(6),
        head_id: 4,
        component: 2,
        channel: HeadsChannel::Lta,
        access_class: HeadAccessClass::OfflineQuote,
        status: HeadStatus::Live,
        name_len: 5,
        spec_hash: [7; 32],
        spec_object: key(8),
        proposer: key(9),
        delegation: 500,
        delegators: 2,
        accrual_integral: u128::MAX / 5,
        strike_count: 1,
        value: 101_000_000,
        origin_id: [10; 32],
        lta_effective_from_ts: 50,
        lta_next_reset_ts: 900,
        lta_floor: 90_000_000,
        lta_ceiling: 110_000_000,
        print_count: 33,
        claim_seq: 40,
        source_case: 1,
        source_case_status: SOURCE_CASE_STOOD,
        ..Head::default()
    };
    h.name[..5].copy_from_slice(b"Hynix");
    h.capture_periods[0] = key(40);
    h.capture_listing[0] = 12;
    h.capture_flags[0] = CAPTURE_LISTING;
    h.strike_opened_ts[0] = 77;
    h.strike_ts[0] = 78;
    h.sample_values[..2].copy_from_slice(&[1_000, 1_010]);
    h.sample_ts[..2].copy_from_slice(&[10_000, 10_001]);
    h.sample_resets[..2].copy_from_slice(&[0, 0]);
    h.reveals = 39;
    h.finalized_reveals = 39;
    h.restart_reveals[0] = 30;
    for i in 0..HEAD_PRINT_CAPACITY {
        h.print_claim_seq[i] = i as u32;
        h.print_values[i] = 1_000 + i as u64;
        h.print_source_ts[i] = 10_000 + i as u64;
        h.print_effective_from_ts[i] = 9_000 + i as u64;
        h.print_next_reset_ts[i] = 20_000 + i as u64;
        h.print_floor[i] = 900 + i as u64;
        h.print_ceiling[i] = 1_100 + i as u64;
    }
    h
}

fn delegation() -> Delegation {
    let (discriminator, version) = header::<Delegation>();
    let mut d = Delegation {
        discriminator,
        version,
        bump: 249,
        head: key(11),
        owner: key(12),
        board: key(6),
        is_proposer: true,
        active: 100,
        fresh: 5,
        held_principal: 40,
        held: 30,
        reward_index: 77,
        ..Delegation::default()
    };
    d.capture_periods[3] = key(41);
    d.capture_freeze[3] = 99;
    d.capture_flags[3] = CAPTURE_LISTING | CAPTURE_FREEZE;
    // 40 held principal: 10 weighed in nothing unsettled, 30 in slot 3's contract; 30 cooling.
    (d.held_exposed[0], d.held_exposed[1 << 3], d.held_gens[3]) = (10, 30, 7);
    (d.weighed[3], d.cooling[0], d.cooling_end[0]) = (50, 30, 500);
    d
}

fn claim() -> HeadClaim {
    let (discriminator, version) = header::<HeadClaim>();
    HeadClaim {
        discriminator,
        version,
        bump: 246,
        head: key(11),
        board: key(6),
        seq: 39,
        kind: HeadClaimKind::Update,
        status: HeadClaimStatus::Revealed,
        fetcher: key(13),
        commit_hash: [15; 32],
        value: 101_000_000,
        origin_id: [16; 32],
        lta_effective_from_ts: 1,
        lta_next_reset_ts: 2,
        lta_floor: 3,
        lta_ceiling: 4,
        evidence_kind: 4,
        evidence_commitment: [17; 32],
        evidence_object: key(18),
        bond: 1_000_000,
        challenge_count: 3,
        won_count: 1,
        last_winner: 2,
        epoch: 24_321,
        ..HeadClaim::default()
    }
}

fn challenge() -> HeadChallenge {
    let (discriminator, version) = header::<HeadChallenge>();
    HeadChallenge {
        discriminator,
        version,
        bump: 245,
        target: key(19),
        head: key(11),
        challenger: key(20),
        kind: HeadChallengeKind::Source,
        reason: HeadChallengeReason::DuplicateOrigin,
        index: 2,
        case_number: 1,
        bond: 1_000_000,
        reason_hash: [21; 32],
        ruling: HeadChallengeRuling::Won,
        payout_amba: 5,
        settled: true,
        ..HeadChallenge::default()
    }
}

fn contract() -> HeadsContract {
    let (discriminator, version) = header::<HeadsContract>();
    let b = board();
    let mut c = HeadsContract {
        discriminator,
        version,
        bump: 244,
        period: key(40),
        board: key(6),
        product_id: b.product_id,
        ring_slot: 1,
        status: HeadsContractStatus::Collecting,
        settlement_target: HeadsSettlementTarget::ContractChannel,
        params: HeadsParams::spec_defaults(),
        params_version: 1,
        base: 100_000_000,
        listing_ts: 100,
        freeze_ts: 200,
        expiry_ts: 300,
        terms_hash: [22; 32],
        item_count: b.item_count,
        item_bucket_ids: b.item_bucket_ids,
        item_weight_bps: b.item_weight_bps,
        item_mix_bps: b.item_mix_bps,
        item_mix_basis: b.item_mix_basis,
        component_count: b.component_count,
        component_items: b.component_items,
        component_channels: b.component_channels,
        component_sentence_hashes: b.component_sentence_hashes,
        listed_count: 3,
        index_change_ppm: -12_345,
        value: 98_765_432,
        listed_by: key(23),
        ..HeadsContract::default()
    };
    c.item_reference_flags[0] = REFERENCE_COMPUTED | REFERENCE_AVAILABLE | REFERENCE_ASSUMED;
    c.item_reference_flags[1] = REFERENCE_COMPUTED | REFERENCE_AVAILABLE;
    c.item_reference_ppm[1] = 2;
    c.item_degraded_channels[1] = 1 << HeadsChannel::Lta as u8;
    c.item_change_ppm[..2].copy_from_slice(&[-1, 2]);
    c.component_flags[1] = COMPONENT_SETTLED | COMPONENT_LIVE;
    c.component_flags[1] |= COMPONENT_FREEZE_CHECKED | COMPONENT_FREEZE_PASSED;
    c.component_change_ppm[1] = -4;
    c.component_listed_origins[1] = 3;
    c.component_freeze_origins[1] = 3;
    c.component_duplicates[1] = 1;
    c.component_flags[5] = COMPONENT_SETTLED | COMPONENT_LAST_VALID | COMPONENT_FREEZE_CHECKED;
    c.component_degraded[5] = DEGRADED_FREEZE_CHECK_FAILED;
    c.listed_head_ids[..3].copy_from_slice(&[1, 4, 7]);
    c.listed_components[..3].copy_from_slice(&[1, 1, 1]);
    c.listed_flags[2] = LIVE_DUPLICATE;
    c.listed_openings[..3].copy_from_slice(&[1, 2, 3]);
    c
}

fn epoch() -> HeadsEpoch {
    let (discriminator, version) = header::<HeadsEpoch>();
    HeadsEpoch {
        discriminator,
        version,
        bump: 243,
        board: key(6),
        month_id: 24_321,
        start_ts: 10,
        end_ts: 20,
        prize_pool: 5,
        ..HeadsEpoch::default()
    }
}

fn stake() -> HeadsStake {
    let (discriminator, version) = header::<HeadsStake>();
    HeadsStake {
        discriminator,
        version,
        bump: 242,
        owner: key(12),
        free: 9,
        delegation_count: 1,
        ..HeadsStake::default()
    }
}

#[test]
fn layouts_are_pinned() {
    // Consumers (the API, petri, ops, the chain) decode these; a change is a coordinated release.
    for (len, wire, expected_len, expected_wire) in [
        (HeadsParams::LEN, HeadsParams::LEN, 120, 120),
        (HeadsConfig::LEN, HeadsConfig::WIRE_LEN, 256, 152),
        (HeadsBoard::LEN, HeadsBoard::WIRE_LEN, 10_240, 10_194),
        (HeadsStake::LEN, HeadsStake::WIRE_LEN, 192, 110),
        (Head::LEN, Head::WIRE_LEN, 4_096, 3_855),
        (Delegation::LEN, Delegation::WIRE_LEN, 1_024, 969),
        (HeadClaim::LEN, HeadClaim::WIRE_LEN, 512, 478),
        (HeadChallenge::LEN, HeadChallenge::WIRE_LEN, 384, 309),
        (HeadsContract::LEN, HeadsContract::WIRE_LEN, 4_608, 3_902),
        (HeadsEpoch::LEN, HeadsEpoch::WIRE_LEN, 160, 118),
        (
            HeadsWeightsHeader::LEN,
            HeadsWeightsHeader::WIRE_LEN,
            192,
            147,
        ),
        (
            HeadsWeightSource::LEN,
            HeadsWeightSource::WIRE_LEN,
            160,
            141,
        ),
    ] {
        assert_eq!((len, wire), (expected_len, expected_wire));
        assert!(wire <= len);
    }
    assert_eq!(HEADS_MAX_ITEMS, 8);
    assert_eq!(HEADS_MAX_COMPONENTS, 24);
    assert_eq!(HEADS_MAX_LIVE_SOURCES, 96);
    assert_eq!(HEADS_CONTRACT_RING, 4);
    let discriminators = [
        HeadsConfig::DISCRIMINATOR,
        HeadsBoard::DISCRIMINATOR,
        HeadsStake::DISCRIMINATOR,
        Head::DISCRIMINATOR,
        Delegation::DISCRIMINATOR,
        HeadClaim::DISCRIMINATOR,
        HeadChallenge::DISCRIMINATOR,
        HeadsContract::DISCRIMINATOR,
        HeadsEpoch::DISCRIMINATOR,
        HeadsWeightsHeader::DISCRIMINATOR,
    ];
    for (i, a) in discriminators.iter().enumerate() {
        assert!(discriminators[..i].iter().all(|b| b != a));
    }
}

#[test]
fn fixed_codecs_are_byte_identical_to_borsh() {
    assert_codec_matches_borsh(&HeadsParams::spec_defaults(), HeadsParams::LEN);
    assert_codec_matches_borsh(&config(), HeadsConfig::WIRE_LEN);
    assert_codec_matches_borsh(&board(), HeadsBoard::WIRE_LEN);
    assert_codec_matches_borsh(&stake(), HeadsStake::WIRE_LEN);
    assert_codec_matches_borsh(&head(), Head::WIRE_LEN);
    assert_codec_matches_borsh(&delegation(), Delegation::WIRE_LEN);
    assert_codec_matches_borsh(&claim(), HeadClaim::WIRE_LEN);
    assert_codec_matches_borsh(&challenge(), HeadChallenge::WIRE_LEN);
    assert_codec_matches_borsh(&contract(), HeadsContract::WIRE_LEN);
    assert_codec_matches_borsh(&epoch(), HeadsEpoch::WIRE_LEN);
    assert_codec_matches_borsh(&HeadsWeightsHeader::default(), HeadsWeightsHeader::WIRE_LEN);
    assert_codec_matches_borsh(&HeadsWeightSource::default(), HeadsWeightSource::WIRE_LEN);
}

#[test]
fn params_mirror_the_shared_crate() {
    let spec = ameba_oracle_heads::Params::spec_defaults();
    let mirror = HeadsParams::from(&spec);
    assert_eq!(mirror.to_params(), spec);
    assert!(mirror.validate());
    let mut too_many = mirror;
    too_many.max_strikes = HEAD_STRIKE_CAPACITY as u8 + 1;
    assert!(!too_many.validate());
    assert!(!HeadsParams::zero().validate());
}

/// Packs, unpacks, and checks every reserved byte, length and header byte rejects.
fn assert_pack_roundtrip<T>(value: &T)
where
    T: HeadsAccount + core::fmt::Debug + PartialEq,
{
    let mut data = vec![0xa5u8; T::LEN];
    value.pack(&mut data).unwrap();
    assert_eq!(&*T::unpack(&data).unwrap(), value);
    let wire = T::WIRE_LEN;
    assert!(data[wire..].iter().all(|b| *b == 0));
    for index in [wire, T::LEN - 1] {
        let mut tail = data.clone();
        tail[index] = 1;
        assert!(T::unpack(&tail).is_err(), "reserved byte {index}");
    }
    for index in 0..9 {
        let mut header = data.clone();
        header[index] ^= 0x40;
        assert!(T::unpack(&header).is_err(), "header byte {index}");
    }
    assert!(T::unpack(&data[..T::LEN - 1]).is_err());
    let mut longer = data.clone();
    longer.push(0);
    assert!(T::unpack(&longer).is_err());
    assert!(value.pack(&mut vec![0u8; T::LEN + 1]).is_err());
}

#[test]
fn every_account_packs_into_its_allocation_and_rejects_corruption() {
    assert_pack_roundtrip(&config());
    assert_pack_roundtrip(&board());
    assert_pack_roundtrip(&stake());
    assert_pack_roundtrip(&head());
    assert_pack_roundtrip(&delegation());
    assert_pack_roundtrip(&claim());
    assert_pack_roundtrip(&challenge());
    assert_pack_roundtrip(&contract());
    assert_pack_roundtrip(&epoch());
}

fn rejects<T: HeadsAccount>(value: &T) -> bool {
    let mut data = vec![0u8; T::LEN];
    value.pack(&mut data).unwrap();
    T::unpack(&data).is_err()
}

#[test]
fn enum_bytes_outside_their_domain_reject() {
    let mut data = vec![0u8; Head::LEN];
    head().pack(&mut data).unwrap();
    // `channel` follows the header (10), board (32), head_id (4) and component (1).
    let channel = 10 + 32 + 4 + 1;
    assert_eq!(data[channel], HeadsChannel::Lta as u8);
    data[channel] = 3;
    assert!(Head::unpack(&data).is_err());
}

#[test]
fn layout_invariants_reject_inconsistent_items_components_and_lists() {
    let mut b = board();
    b.item_mix_bps[0] += 1;
    assert!(rejects(&b), "mix off 10,000");
    let mut b = board();
    b.item_mix_basis[0] = 3;
    assert!(rejects(&b), "mix basis domain");
    let mut b = board();
    b.component_items[3] = 0;
    assert!(rejects(&b), "component off the mold's item order");
    let mut b = board();
    b.component_channels[3] = 3;
    assert!(rejects(&b), "channel domain");
    let mut b = board();
    b.component_channels[2] = 1;
    assert!(rejects(&b), "component off the mold's channel order");
    let mut b = board();
    b.component_count = 5;
    b.component_items[5] = 0;
    b.component_channels[5] = 0;
    assert!(rejects(&b), "three components per item");
    let mut b = board();
    b.component_sentence_hashes[6] = [1; 32];
    assert!(rejects(&b), "sentence past the count");
    let mut b = board();
    b.live_origin_ids[3] = [1; 32];
    assert!(rejects(&b), "live entry past the count");
    let mut b = board();
    b.live_components[0] = 6;
    assert!(rejects(&b), "live entry on a missing component");
    let mut b = board();
    b.live_since_ts[3] = 1;
    assert!(rejects(&b), "live time past the count");
    let mut b = board();
    b.contract_flags[1] = 2;
    assert!(rejects(&b), "unknown ring flag");
    let mut b = board();
    b.contract_freeze_origins[0] = 1;
    assert!(rejects(&b), "freeze origins on an empty ring slot");
    let mut b = board();
    b.live_flags[0] = 32;
    assert!(rejects(&b), "unknown live flag");
    let mut b = board();
    b.item_count = 0;
    assert!(rejects(&b), "a board has an item");
    let mut b = board();
    b.contract_status[1] = RING_EMPTY;
    assert!(rejects(&b), "an empty ring slot names no period");

    let mut h = head();
    h.name[..5].copy_from_slice(&[0xff; 5]);
    assert!(rejects(&h), "name not UTF-8");
    let mut h = head();
    h.name[5] = b'x';
    assert!(rejects(&h), "name byte past its length");
    let mut h = head();
    h.channel = HeadsChannel::Contract;
    assert!(rejects(&h), "LTA terms off the LTA channel");
    let mut h = head();
    h.channel = HeadsChannel::Spot;
    h.lta_effective_from_ts = 0;
    h.lta_next_reset_ts = 0;
    h.lta_floor = 0;
    h.lta_ceiling = 0;
    h.print_effective_from_ts = [0; HEAD_PRINT_CAPACITY];
    h.print_next_reset_ts = [0; HEAD_PRINT_CAPACITY];
    h.print_floor = [0; HEAD_PRINT_CAPACITY];
    assert!(rejects(&h), "a print ceiling off the LTA channel");
    h.print_ceiling = [0; HEAD_PRINT_CAPACITY];
    assert!(!rejects(&h), "a spot head without LTA terms");
    let mut h = head();
    h.strike_ts[1] = 1;
    assert!(rejects(&h), "strike past the count");
    let mut h = head();
    h.sample_values[2] = 1;
    assert!(rejects(&h), "a sample level without its print");
    let mut h = head();
    h.samples_reserved[1] = 1;
    assert!(rejects(&h), "the reserved sample bytes");
    let mut h = head();
    h.adm_open[0] = h.open_claims + 1;
    assert!(rejects(&h), "more admitted claims open than open claims");
    let mut h = head();
    h.finalized_reveals = h.reveals + 1;
    assert!(rejects(&h), "more finalized than revealed");
    let mut h = head();
    h.print_claim_seq[3] = h.claim_seq;
    assert!(rejects(&h), "a print from a claim not committed yet");
    let mut h = head();
    h.capture_flags[1] = CAPTURE_LISTING;
    assert!(rejects(&h), "capture without a period");

    let mut c = claim();
    c.lta_floor = 5;
    assert!(rejects(&c), "floor above ceiling");
    let mut c = claim();
    c.evidence_kind = 1;
    assert!(rejects(&c), "locator evidence");
    let mut c = claim();
    c.evidence_object = Pubkey::default();
    assert!(rejects(&c), "evidence without its object");

    let mut d = delegation();
    d.held = d.held_principal + 1;
    assert!(rejects(&d), "holding above its principal");
    let mut d = delegation();
    d.held_exposed[1] = 1;
    assert!(rejects(&d), "principal other than its masks'");
    let mut d = delegation();
    (d.held_gens[3], d.weighed[3]) = (0, 0);
    assert!(rejects(&d), "a mask naming a slot with no contract");
    let mut d = delegation();
    d.held_case_adjust[0] = 1;
    assert!(rejects(&d), "a case adjustment with no case");
    let mut d = delegation();
    d.weighed[3] = d.active + d.fresh + 1;
    assert!(rejects(&d), "weighed above what is delegated");
    let mut d = delegation();
    d.cooling_end[1] = 9;
    assert!(rejects(&d), "a cooling end with nothing cooling");
    let mut d = delegation();
    (d.held, d.held_principal, d.held_exposed) = (0, 0, [0; 16]);
    d.case_ref = 3;
    assert!(rejects(&d), "a case with nothing held");

    let mut k = contract();
    k.item_reference_reason[0] = REFERENCE_REASON_INVALID_MIX + 1;
    assert!(rejects(&k), "reference reason domain");
    // Codes 1 and 3 are retired.
    for retired in [1, 3] {
        let mut k = contract();
        k.item_reference_reason[0] = retired;
        assert!(rejects(&k), "retired reference reason {retired}");
    }
    let mut k = contract();
    k.item_reference_reason[0] = REFERENCE_REASON_MIX_BASIS_NONE;
    assert!(!rejects(&k));
    let mut k = contract();
    k.item_degraded_channels[0] = 0b1000;
    assert!(rejects(&k), "degraded channels are three bits");
    let mut k = contract();
    k.item_degraded_channels[2] = 1;
    assert!(rejects(&k), "degraded channels past the item count");
    let mut k = contract();
    k.component_degraded[1] = DEGRADED_TOO_LITTLE_WEIGHT + 1;
    assert!(rejects(&k), "degraded reason domain");
    let mut k = contract();
    k.component_freeze_origins[6] = 1;
    assert!(rejects(&k), "freeze origins past the count");
    let mut k = contract();
    k.listed_openings[3] = 1;
    assert!(rejects(&k), "listed entry past the count");
    let mut k = contract();
    k.component_change_ppm[6] = 1;
    assert!(rejects(&k), "component result past the count");
}

#[test]
fn stored_identities_derive_the_canonical_pdas() {
    let program = Pubkey::new_unique();
    let product = board().product_id;
    let (board_key, board_bump) = find_heads_board_address(&program, &product);
    let (head_key, head_bump) = find_head_address(&program, &board_key, 4);
    let owner = key(12);
    let fetcher = key(13);
    let (config_key, config_bump) = find_heads_config_address(&program);
    let (stake_key, stake_bump) = find_heads_stake_address(&program, &owner);
    let (delegation_key, delegation_bump) =
        find_head_delegation_address(&program, &head_key, &owner);
    let (claim_key, claim_bump) = find_head_claim_address(&program, &head_key, 39);
    let (challenge_key, challenge_bump) =
        find_head_challenge_address(&program, &claim_key, &fetcher);
    let period = key(40);
    let (contract_key, contract_bump) = find_heads_contract_address(&program, &period);
    let (epoch_key, epoch_bump) = find_heads_epoch_address(&program, &board_key, 24_321);
    let (weights_key, weights_bump) = find_heads_weights_address(&program, &contract_key, 23);

    let mut c = config();
    c.bump = config_bump;
    assert_eq!(c.address(&program), Some(config_key));
    let mut b = board();
    b.bump = board_bump;
    assert_eq!(b.address(&program), Some(board_key));
    let mut s = stake();
    s.bump = stake_bump;
    assert_eq!(s.address(&program), Some(stake_key));
    let mut h = head();
    h.board = board_key;
    h.bump = head_bump;
    assert_eq!(h.address(&program), Some(head_key));
    let mut d = delegation();
    d.head = head_key;
    d.owner = owner;
    d.bump = delegation_bump;
    assert_eq!(d.address(&program), Some(delegation_key));
    let mut k = claim();
    k.head = head_key;
    k.seq = 39;
    k.bump = claim_bump;
    assert_eq!(k.address(&program), Some(claim_key));
    let mut x = challenge();
    x.target = claim_key;
    x.challenger = fetcher;
    x.bump = challenge_bump;
    assert_eq!(x.address(&program), Some(challenge_key));
    let mut t = contract();
    t.period = period;
    t.bump = contract_bump;
    assert_eq!(t.address(&program), Some(contract_key));
    let mut e = epoch();
    e.board = board_key;
    e.bump = epoch_bump;
    assert_eq!(e.address(&program), Some(epoch_key));
    let w = HeadsWeightsHeader {
        contract: contract_key,
        component: 23,
        bump: weights_bump,
        ..HeadsWeightsHeader::default()
    };
    assert_eq!(w.address(&program), Some(weights_key));
    // Another identity, another address.
    h.head_id = 5;
    assert_ne!(h.address(&program), Some(head_key));
    // Every seed starts with the namespace.
    assert_eq!(
        find_heads_authority_address(&program).0,
        Pubkey::find_program_address(&[b"ameba-oracle-v1", b"heads-authority"], &program).0
    );
}

#[test]
fn loaders_check_owner_length_and_stored_bump_address() {
    let program = Pubkey::new_unique();
    let (address, bump) = find_heads_stake_address(&program, &key(12));
    let mut value = stake();
    value.bump = bump;
    let mut data = vec![0u8; HeadsStake::LEN];
    value.pack(&mut data).unwrap();
    let load = |key: Pubkey, owner: Pubkey, data: &mut Vec<u8>| {
        let mut lamports = 1;
        let info = AccountInfo::new(&key, false, true, &mut lamports, data, &owner, false, 0);
        load_heads_account::<HeadsStake>(&program, &info).map(|v| *v)
    };
    assert_eq!(load(address, program, &mut data.clone()).unwrap(), value);
    let invalid = Err(ProgramError::from(OracleError::InvalidHeadsAccount));
    assert_eq!(
        load(address, Pubkey::new_unique(), &mut data.clone()),
        invalid
    );
    assert_eq!(
        load(Pubkey::new_unique(), program, &mut data.clone()),
        invalid
    );
    let mut short = data[..HeadsStake::LEN - 1].to_vec();
    assert_eq!(load(address, program, &mut short), invalid);
    // A wrong stored bump derives another address (or none).
    let mut other = value.clone();
    other.bump = bump.wrapping_sub(1);
    let mut other_data = vec![0u8; HeadsStake::LEN];
    other.pack(&mut other_data).unwrap();
    assert_eq!(load(address, program, &mut other_data), invalid);

    // Store writes the same bytes back.
    let mut lamports = 1;
    let mut target = vec![0xffu8; HeadsStake::LEN];
    let owner = program;
    let info = AccountInfo::new(
        &address,
        false,
        true,
        &mut lamports,
        &mut target,
        &owner,
        false,
        0,
    );
    store_heads_account(&info, &value).unwrap();
    drop(info);
    assert_eq!(target, data);
}

#[test]
fn weights_accounts_lay_out_their_sources_from_the_capacity() {
    let program = Pubkey::new_unique();
    let contract = key(40);
    let (address, bump) = find_heads_weights_address(&program, &contract, 2);
    let header = HeadsWeightsHeader {
        discriminator: HeadsWeightsHeader::DISCRIMINATOR,
        version: HeadsWeightsHeader::VERSION,
        bump,
        contract,
        board: key(6),
        component: 2,
        channel: HeadsChannel::Lta,
        window_secs: 100,
        source_count: 2,
        source_capacity: 3,
        ..HeadsWeightsHeader::default()
    };
    let layout = HeadsWeightsLayout::of(&header);
    assert_eq!(layout.sources_offset(), 192);
    assert_eq!(layout.len(), 192 + 3 * 160);
    let mut data = vec![0u8; layout.len()];
    header.pack_account(&mut data).unwrap();
    let source = HeadsWeightSource {
        head_id: 7,
        flags: WEIGHT_SOURCE_LISTED | WEIGHT_SOURCE_DUPLICATE,
        origin_id: [9; 32],
        opening: 100,
        integral: u128::MAX / 7,
        weight: 5,
        change_ppm: -3,
        ..HeadsWeightSource::default()
    };
    source.pack(&mut data[layout.source_range(1)]).unwrap();

    let mut lamports = 1;
    let mut account = data.clone();
    let info = AccountInfo::new(
        &address,
        false,
        true,
        &mut lamports,
        &mut account,
        &program,
        false,
        0,
    );
    let (read, read_layout) = load_heads_weights_header(&program, &info).unwrap();
    assert_eq!(read, header);
    assert_eq!(read_layout, layout);
    drop(info);
    assert_eq!(
        HeadsWeightSource::unpack(&data[layout.source_range(1)]).unwrap(),
        source
    );
    // The flags byte sits where the completing call reads it alone.
    assert_eq!(
        data[layout.source_range(1).start + HEADS_WEIGHT_SOURCE_FLAGS_OFFSET],
        source.flags
    );
    // A record's padding is reserved.
    let mut dirty = data[layout.source_range(1)].to_vec();
    dirty[HeadsWeightSource::LEN - 1] = 1;
    assert!(HeadsWeightSource::unpack(&dirty).is_err());
    // The length must be exactly what the capacities lay out, and counts within them.
    assert!(HeadsWeightsHeader::unpack_account(&data[..data.len() - 1]).is_err());
    let mut over = header.clone();
    over.source_count = 4;
    let mut over_data = data.clone();
    over.pack_account(&mut over_data).unwrap();
    assert!(HeadsWeightsHeader::unpack_account(&over_data).is_err());
    let mut component = header.clone();
    component.component = HEADS_MAX_COMPONENTS as u8;
    let mut component_data = data.clone();
    component.pack_account(&mut component_data).unwrap();
    assert!(HeadsWeightsHeader::unpack_account(&component_data).is_err());
    // Another component's address does not load.
    let mut lamports = 1;
    let mut account = data;
    let other = find_heads_weights_address(&program, &contract, 3).0;
    let info = AccountInfo::new(
        &other,
        false,
        true,
        &mut lamports,
        &mut account,
        &program,
        false,
        0,
    );
    assert!(load_heads_weights_header(&program, &info).is_err());
}

#[test]
fn head_prints_read_the_ring_oldest_first() {
    let mut h = head();
    h.print_count = 3;
    for i in 3..HEAD_PRINT_CAPACITY {
        h.print_values[i] = 0;
        h.print_source_ts[i] = 0;
    }
    assert_eq!(
        h.prints(),
        vec![(10_000, 1_000), (10_001, 1_001), (10_002, 1_002)]
    );
    let h = head();
    // 33 prints: the ring kept prints 1..=32; print 32 overwrote slot 0.
    let prints = h.prints();
    assert_eq!(prints.len(), HEAD_PRINT_CAPACITY);
    assert_eq!(prints[0], (10_001, 1_001));
    assert_eq!(prints[31], (10_000, 1_000));
    assert_eq!(head().name(), "Hynix");
}

#[test]
fn evidence_lta_and_mix_rules() {
    assert!(heads_evidence_is_valid(0, &[0; 32]));
    assert!(!heads_evidence_is_valid(0, &[1; 32]));
    for kind in [2, 3, 4] {
        assert!(heads_evidence_is_valid(kind, &[1; 32]));
        assert!(!heads_evidence_is_valid(kind, &[0; 32]));
    }
    for kind in [1, 5, 255] {
        assert!(!heads_evidence_is_valid(kind, &[1; 32]));
    }
    assert!(heads_lta_terms_are_valid(0, 0, 0, 0));
    assert!(heads_lta_terms_are_valid(5, 0, 9, 0));
    assert!(heads_lta_terms_are_valid(5, 6, 9, 9));
    assert!(!heads_lta_terms_are_valid(6, 6, 0, 0));
    assert!(!heads_lta_terms_are_valid(0, 0, 10, 9));
    assert!(heads_mix_is_valid(&[0, 10_000, 0]));
    assert!(heads_mix_is_valid(&[2_500, 5_000, 2_500]));
    assert!(!heads_mix_is_valid(&[2_500, 5_000, 2_499]));
    assert!(!heads_mix_is_valid(&[u16::MAX, u16::MAX, 0]));
    assert_eq!(HeadsChannel::from_byte(2), Some(HeadsChannel::Lta));
    assert_eq!(HeadsChannel::from_byte(3), None);
    assert_eq!(HeadsMixBasis::from_byte(2), Some(HeadsMixBasis::Matched));
    assert_eq!(HeadsMixBasis::from_byte(3), None);
    assert_eq!(heads_component_index(0, HeadsChannel::Spot), 0);
    assert_eq!(heads_component_index(1, HeadsChannel::Contract), 4);
    assert_eq!(
        heads_component_index(7, HeadsChannel::Lta),
        HEADS_MAX_COMPONENTS - 1
    );
}

#[test]
fn codes_match_the_shared_channel_arithmetic() {
    // The on-chain bytes are `ameba_oracle_heads::channels`' codes (`docs/lta.md`).
    assert_eq!(
        [
            HeadsChannel::Spot as u8,
            HeadsChannel::Contract as u8,
            HeadsChannel::Lta as u8
        ],
        [0, 1, 2]
    );
    assert_eq!(
        [
            HeadsMixBasis::None as u8,
            HeadsMixBasis::Assumed as u8,
            HeadsMixBasis::Matched as u8
        ],
        [0, 1, 2]
    );
    assert_eq!(
        [
            HeadsSettlementTarget::ContractChannel as u8,
            HeadsSettlementTarget::FixedMixReference as u8
        ],
        [0, 1]
    );
    assert_eq!(
        [
            HeadChallengeReason::ValueOrOther as u8,
            HeadChallengeReason::OffChannel as u8,
            HeadChallengeReason::DuplicateOrigin as u8,
            HeadChallengeReason::ExpiredObservation as u8
        ],
        [0, 1, 2, 3]
    );
    assert_eq!(
        [
            REFERENCE_REASON_NONE,
            REFERENCE_REASON_MIX_BASIS_NONE,
            REFERENCE_REASON_INVALID_MIX
        ],
        [0, 2, 4]
    );
    assert_eq!(
        [
            DEGRADED_FREEZE_CHECK_FAILED,
            DEGRADED_TOO_FEW_ORIGINS,
            DEGRADED_TOO_LITTLE_WEIGHT
        ],
        [1, 2, 3]
    );
}

#[test]
fn zeroed_boxes_equal_the_defaults() {
    assert_eq!(*zeroed_box::<HeadsBoard>(), HeadsBoard::default());
    assert_eq!(*zeroed_box::<Head>(), Head::default());
    assert_eq!(*zeroed_box::<HeadsContract>(), HeadsContract::default());
    assert_eq!(*zeroed_box::<Delegation>(), Delegation::default());
    assert_eq!(*zeroed_box::<HeadClaim>(), HeadClaim::default());
    assert_eq!(*zeroed_box::<HeadChallenge>(), HeadChallenge::default());
    assert_eq!(
        *zeroed_box::<HeadsWeightsHeader>(),
        HeadsWeightsHeader::default()
    );
    assert_eq!(*zeroed_box::<HeadsStake>(), HeadsStake::default());
    assert_eq!(*zeroed_box::<HeadsConfig>(), HeadsConfig::default());
    assert_eq!(*zeroed_box::<HeadsEpoch>(), HeadsEpoch::default());
}

/// The program's mix commitment is commodity-spec's, and the listing terms document carries it
/// as the field `ListContract` looks for, for each quarter.
#[test]
fn the_listing_mix_commitment_is_the_documents() {
    use ameba_commodity_spec::{label_id32, molds, Product};
    let mold = Product::Ramx.mold();
    let buckets: Vec<[u8; 32]> = mold.items.iter().map(|i| i.bucket.id32()).collect();
    let product = label_id32("RAMX").unwrap();
    // Q4 2026 (2027-01-01) and Q1 2027 (2027-04-01): different mixes, different commitments.
    let mut seen = Vec::new();
    for expiry in [1_798_761_600i64, 1_806_537_600] {
        let mixes: Vec<_> = molds::listing_mixes(Product::Ramx, expiry)
            .into_iter()
            .map(|(mix, basis)| (mix, HeadsMixBasis::from_byte(basis.code()).unwrap()))
            .collect();
        let (year, quarter) = ameba_oracle_heads::time::contract_quarter(expiry as u64);
        let ours = heads_listing_mix_commitment(
            &product,
            ameba_oracle_heads::time::quarter_start(year, quarter),
            &buckets,
            &mixes,
        );
        assert_eq!(ours, molds::listing_mix_commitment(Product::Ramx, expiry));
        let field = heads_mix_commitment_field(&ours);
        let document = molds::listing_terms_document(Product::Ramx, expiry);
        assert!(document.windows(field.len()).any(|w| w == field));
        seen.push(ours);
    }
    assert_ne!(seen[0], seen[1]);
    assert_eq!(
        &heads_mix_commitment_field(&[0xab; 32])[..20],
        b"\"mixCommitment\":\"aba"
    );
}
