//! The board view over constructed accounts: shares under the caps, duplicate origins, the
//! changes against a contract's openings, the reference and the Stage 1 index, LTA levels, raw
//! weights and positions.
use super::*;
use crate::state::heads::{
    find_heads_stake_address, HeadsChannel, HeadsMixBasis, HeadsParams, HeadsSettlementTarget,
    HeadsStake, RING_ACCUMULATING,
};

const AMBA: u64 = 1_000_000_000;
const LISTING: u64 = 1_793_145_600;
const FREEZE: u64 = 1_803_254_400;
const EXPIRY: u64 = 1_803_859_200;
const NOW: u64 = 1_793_250_000;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn wide(n: u32) -> Pubkey {
    let mut bytes = [7u8; 32];
    bytes[..4].copy_from_slice(&n.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

fn product(symbol: &str) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..symbol.len()].copy_from_slice(symbol.as_bytes());
    id
}

/// RAMX with `items` (bucket byte, basket weight, mix, basis), three components each.
fn board(items: &[(u8, u16, [u16; 3], HeadsMixBasis)]) -> HeadsBoard {
    let mut b = HeadsBoard {
        discriminator: HeadsBoard::DISCRIMINATOR,
        version: HeadsBoard::VERSION,
        product_id: product("RAMX"),
        params: HeadsParams::spec_defaults(),
        params_version: 1,
        item_count: items.len() as u8,
        component_count: (items.len() * 3) as u8,
        ..HeadsBoard::default()
    };
    for (i, (bucket, weight, mix, basis)) in items.iter().enumerate() {
        b.item_bucket_ids[i] = [*bucket; 32];
        b.item_weight_bps[i] = *weight;
        b.item_mix_bps[i * 3..i * 3 + 3].copy_from_slice(mix);
        b.item_mix_basis[i] = *basis as u8;
        for ch in 0..3 {
            b.component_items[i * 3 + ch] = i as u8;
            b.component_channels[i * 3 + ch] = ch as u8;
            b.component_sentence_hashes[i * 3 + ch] = [(40 + i * 3 + ch) as u8; 32];
        }
    }
    b
}

/// A live head on `component` holding `delegation`, live since `live_ts`, whose fill and only
/// print are `value` at `live_ts`, declaring `origin`.
fn head(id: u32, component: u8, delegation: u64, value: u64, origin: u8, live_ts: u64) -> Head {
    let mut h = Head {
        discriminator: Head::DISCRIMINATOR,
        version: Head::VERSION,
        board: key(1),
        head_id: id,
        component,
        channel: HeadsChannel::from_byte(component % 3).unwrap(),
        status: HeadStatus::Live,
        proposer: key(2),
        delegation,
        accrual_since_ts: live_ts,
        posted_ts: live_ts - 100,
        live_ts,
        value,
        value_source_ts: live_ts,
        origin_id: [origin; 32],
        fill_value: value,
        fill_source_ts: live_ts,
        print_count: 1,
        ..Head::default()
    };
    h.print_values[0] = value;
    h.print_source_ts[0] = live_ts;
    h
}

/// Appends a print to `h` (its current value too).
fn print(h: &mut Head, ts: u64, value: u64) {
    let slot = h.print_count as usize % HEAD_PRINT_CAPACITY;
    h.print_values[slot] = value;
    h.print_source_ts[slot] = ts;
    h.print_count += 1;
    h.value = value;
    h.value_source_ts = ts;
}

fn delegation(head: Pubkey, owner: Pubkey, amount: u64) -> Delegation {
    Delegation {
        discriminator: Delegation::DISCRIMINATOR,
        version: Delegation::VERSION,
        head,
        owner,
        board: key(1),
        active: amount,
        created_ts: LISTING - 1_000,
        ..Delegation::default()
    }
}

/// The board's accounts with `heads`, each head's delegation split evenly over `owners`
/// distinct owners (so the address cap does not bind).
fn accounts(board: HeadsBoard, heads: Vec<Head>, owners: u32) -> BoardAccounts {
    let mut next = 0u32;
    let mut delegations = Vec::new();
    let heads: Vec<(Pubkey, Box<Head>)> = heads
        .into_iter()
        .map(|h| {
            let address = wide(10_000 + h.head_id);
            for _ in 0..owners {
                next += 1;
                delegations.push((
                    wide(next),
                    Box::new(delegation(
                        address,
                        wide(1_000_000 + next),
                        h.delegation / u64::from(owners),
                    )),
                ));
            }
            (address, Box::new(h))
        })
        .collect();
    BoardAccounts {
        address: key(1),
        board: Box::new(board),
        heads,
        delegations,
        contracts: Vec::new(),
        weights: Vec::new(),
    }
}

/// A contract on the Contract channel alone over `accounts`' board, listed with `listed` (head
/// id, component, opening).
fn contract(accounts: &BoardAccounts, listed: &[(u32, u8, u64)]) -> HeadsContract {
    let b = &accounts.board;
    let mut k = HeadsContract {
        discriminator: HeadsContract::DISCRIMINATOR,
        version: HeadsContract::VERSION,
        period: key(9),
        board: accounts.address,
        product_id: b.product_id,
        status: HeadsContractStatus::Accumulating,
        settlement_target: HeadsSettlementTarget::ContractChannel,
        params: b.params,
        params_version: 1,
        base: 100_000_000,
        listing_ts: LISTING,
        freeze_ts: FREEZE,
        expiry_ts: EXPIRY,
        item_count: b.item_count,
        item_bucket_ids: b.item_bucket_ids,
        item_weight_bps: b.item_weight_bps,
        item_mix_bps: b.item_mix_bps,
        item_mix_basis: b.item_mix_basis,
        component_count: b.component_count,
        component_items: b.component_items,
        component_channels: b.component_channels,
        component_sentence_hashes: b.component_sentence_hashes,
        listed_count: listed.len() as u16,
        ..HeadsContract::default()
    };
    for (i, (id, component, open)) in listed.iter().enumerate() {
        k.listed_head_ids[i] = *id;
        k.listed_components[i] = *component;
        k.listed_openings[i] = *open;
    }
    k
}

const RAMX_ONE: [(u8, u16, [u16; 3], HeadsMixBasis); 1] =
    [(4, 10_000, [1_000, 5_000, 4_000], HeadsMixBasis::Assumed)];

#[test]
fn shares_add_up_by_largest_remainder() {
    assert_eq!(shares_bps(&[1, 1, 1]), vec![3_334, 3_333, 3_333]);
    assert_eq!(shares_bps(&[0, 0]), vec![0, 0]);
    assert_eq!(shares_bps(&[5, 0]), vec![10_000, 0]);
    let shares = shares_bps(&[27_500, 27_500, 18_000, 9_500]);
    assert_eq!(shares, vec![3_333, 3_333, 2_182, 1_152]);
    assert_eq!(shares.iter().map(|s| u32::from(*s)).sum::<u32>(), 10_000);
}

#[test]
fn a_component_shares_its_weight_by_delegation() {
    // The web fixture's DDR5 16Gb: 42,000 / 31,500 / 18,000 / 9,500 AMBA on four sources. No
    // cap (the user, 2026-10-29): each share is its delegation's.
    let heads = vec![
        head(1, 1, 42_000 * AMBA, 7_940_000, 1, LISTING - 10),
        head(2, 1, 31_500 * AMBA, 8_120_000, 2, LISTING - 9),
        head(3, 1, 18_000 * AMBA, 7_650_000, 3, LISTING - 8),
        head(4, 1, 9_500 * AMBA, 7_400_000, 4, LISTING - 7),
    ];
    let a = accounts(board(&RAMX_ONE), heads, 6);
    let v = view(&a, NOW);
    let c = v.component(1).unwrap();
    assert_eq!(
        (c.channel, c.settles, c.live_sources, c.live_origins),
        (Channel::Contract, true, 4, 4)
    );
    assert_eq!(
        c.sources.iter().map(|s| s.share_bps).collect::<Vec<_>>(),
        vec![4_158, 3_119, 1_782, 941]
    );
    assert_eq!(v.share_bps(4), Some(941));
    assert_eq!(v.share_bps(99), None);
    // No contract: nothing to measure a change from, and the reference says so.
    assert_eq!(c.reading, Reading::NoContract);
    assert_eq!(
        v.front_index(),
        IndexReading::Unavailable(IndexGap::NoContract)
    );
    // No contract: the reference has nothing to measure against either; the board reads on the
    // molds' target, so every channel the mix weights settles.
    assert_eq!(
        v.items[0].reference,
        ReferenceView::Unavailable(ReferenceGap::NoContract)
    );
    assert_eq!(v.settlement_target, SettlementTarget::FixedMixReference);
    assert!(v.components.iter().all(|c| c.settles));
    // A mix basis of none is the reference's own reason, contract or not.
    let none = [(4, 10_000, [1_000, 5_000, 4_000], HeadsMixBasis::None)];
    let v_none = view(&accounts(board(&none), vec![], 1), NOW);
    assert_eq!(
        v_none.items[0].reference,
        ReferenceView::Unavailable(ReferenceGap::Reference(Unavailable::MixBasisNone))
    );
    // Thresholds: alpha * D / n and beta * the median live delegation.
    let mut b = (*a.board).clone();
    b.total_delegation = 101_000 * AMBA;
    let v = view(
        &BoardAccounts {
            board: Box::new(b),
            ..a
        },
        NOW,
    );
    assert_eq!(v.thresholds.min_post, 101_000 * AMBA / 4 / 4);
    assert_eq!(v.thresholds.activation, (18_000 + 31_500) * AMBA / 2 / 2);
}

#[test]
fn shares_do_not_depend_on_how_many_owners_delegate() {
    let heads = vec![
        head(1, 0, 3_000 * AMBA, 7_000_000, 1, LISTING - 10),
        head(2, 0, 1_000 * AMBA, 7_100_000, 2, LISTING - 9),
    ];
    // No address cap: one owner each or two, the shares are the delegations'.
    for owners in [1, 2] {
        let v = view(&accounts(board(&RAMX_ONE), heads.clone(), owners), NOW);
        let spot = v.component(0).unwrap();
        assert_eq!(
            spot.sources.iter().map(|s| s.share_bps).collect::<Vec<_>>(),
            vec![7_500, 2_500]
        );
    }
}

#[test]
fn a_repeated_origin_carries_no_weight_and_counts_once() {
    // Heads 2 and 3 declare one origin; 2 went live first, so 3 is the duplicate.
    let heads = vec![
        head(1, 1, 5_000 * AMBA, 7_000_000, 1, LISTING - 10),
        head(3, 1, 5_000 * AMBA, 7_200_000, 2, LISTING - 5),
        head(2, 1, 5_000 * AMBA, 7_100_000, 2, LISTING - 9),
        head(4, 1, 5_000 * AMBA, 7_300_000, 4, LISTING - 7),
    ];
    let v = view(&accounts(board(&RAMX_ONE), heads, 6), NOW);
    let c = v.component(1).unwrap();
    assert_eq!((c.live_sources, c.live_origins), (4, 3));
    let by_id: Vec<(u32, bool, u16)> = c
        .sources
        .iter()
        .map(|s| (s.head_id, s.duplicate, s.share_bps))
        .collect();
    assert_eq!(
        by_id,
        vec![
            (1, false, 3_334),
            (2, false, 3_333),
            (3, true, 0),
            (4, false, 3_333)
        ]
    );
    assert_eq!(v.weighted_sources, 3);
}

#[test]
fn against_a_contract_the_component_moves_by_its_weighted_median_and_the_index_follows() {
    // Item 0 (60%) and item 1 (40%). Every channel of item 0 has three origins; item 1 has only
    // its Contract channel.
    let items = [
        (4, 6_000, [2_000, 5_000, 3_000], HeadsMixBasis::Assumed),
        (5, 4_000, [1_000, 6_000, 3_000], HeadsMixBasis::Assumed),
    ];
    let mut heads = Vec::new();
    let mut listed = Vec::new();
    let mut id = 0;
    for component in [0u8, 1, 2, 4] {
        for n in 0..3u32 {
            id += 1;
            let opening = 10_000_000;
            let mut h = head(
                id,
                component,
                (3_000 + 1_000 * u64::from(n)) * AMBA,
                opening,
                id as u8,
                LISTING - 50 + u64::from(id),
            );
            // Each source rises by 1%, 2% and 3% (by its position).
            print(
                &mut h,
                LISTING + 3_600,
                opening + opening / 100 * u64::from(n + 1),
            );
            heads.push(h);
            listed.push((id, component, opening));
        }
    }
    let mut a = accounts(board(&items), heads, 6);
    a.contracts.push((key(8), Box::new(contract(&a, &listed))));
    let v = view(&a, NOW);
    assert_eq!(v.front, Some(0));
    let changes: Vec<Option<i64>> = v
        .components
        .iter()
        .map(|c| c.reading.live_change_ppm())
        .collect();
    // Weights 3,000 / 4,000 / 5,000 at the 1/3 cap: equal, so the median is the middle change.
    assert_eq!(
        changes,
        vec![
            Some(20_000),
            Some(20_000),
            Some(20_000),
            None,
            Some(20_000),
            None
        ]
    );
    assert_eq!(
        v.components[3].reading,
        Reading::Degraded {
            reason: settle::Degraded::TooFewOrigins
        }
    );
    // Item 0's reference: every channel at +2%, so +2%, flagged as an assumed scenario.
    assert_eq!(
        v.items[0].reference,
        ReferenceView::Available {
            change_ppm: 20_000,
            assumed: true,
            degraded: [false; 3],
            recorded: false
        }
    );
    // Item 1's spot and LTA are on their degraded path: each keeps its share at its listing
    // value, 0, so 60% x 2% = 1.2%; nothing moves onto the Contract channel.
    assert_eq!(
        v.items[1].reference,
        ReferenceView::Available {
            change_ppm: 12_000,
            assumed: true,
            degraded: [true, false, true],
            recorded: false
        }
    );
    // On the Contract channel alone: 60% x 2% + 40% x 2%.
    match v.front_index() {
        IndexReading::Indicative {
            change_ppm,
            value,
            degraded,
        } => {
            assert_eq!(change_ppm, 20_000);
            assert_eq!(value, Some(102_000_000));
            assert!(degraded.is_empty());
        }
        other => panic!("{other:?}"),
    }
    // On the fixed mix: 60% x 2% + 40% x 1.2%, with item 1's spot and LTA (3 and 5) degraded.
    a.contracts[0].1.settlement_target = HeadsSettlementTarget::FixedMixReference;
    match view(&a, NOW).front_index() {
        IndexReading::Indicative {
            change_ppm,
            value,
            degraded,
        } => {
            assert_eq!(change_ppm, 16_800);
            assert_eq!(value, Some(101_680_000));
            assert_eq!(degraded, vec![3, 5]);
        }
        other => panic!("{other:?}"),
    }
    // Once recorded, the reference reads back from the contract, degraded channels included.
    let k = &mut a.contracts[0].1;
    k.item_reference_flags[1] = REFERENCE_COMPUTED | REFERENCE_AVAILABLE | REFERENCE_ASSUMED;
    k.item_reference_ppm[1] = 12_000;
    k.item_degraded_channels[1] = 0b101;
    assert_eq!(
        view(&a, NOW).items[1].reference,
        ReferenceView::Available {
            change_ppm: 12_000,
            assumed: true,
            degraded: [true, false, true],
            recorded: true
        }
    );
    // A source that has no print in effect has no level; its share stays.
    let c = &v.components[1];
    assert!(c
        .sources
        .iter()
        .all(|s| s.opening == Some(10_000_000) && s.level.is_some()));
}

#[test]
fn an_lta_print_past_its_reset_is_no_level() {
    let items = [(4, 10_000, [0, 5_000, 5_000], HeadsMixBasis::Assumed)];
    let mut h = head(1, 2, 1_000 * AMBA, 9_000_000, 1, LISTING - 10);
    h.print_next_reset_ts[0] = NOW - 1;
    h.lta_next_reset_ts = NOW - 1;
    let mut current = head(2, 2, 1_000 * AMBA, 9_100_000, 2, LISTING - 10);
    current.print_next_reset_ts[0] = NOW + 86_400;
    let v = view(&accounts(board(&items), vec![h, current], 1), NOW);
    let lta = v.component(2).unwrap();
    assert_eq!(
        lta.sources.iter().map(|s| s.level).collect::<Vec<_>>(),
        vec![None, Some(9_100_000)]
    );
}

#[test]
fn raw_weight_is_the_live_delegation_averaged_over_the_window_so_far() {
    let k = HeadsContract {
        period: key(9),
        listing_ts: 1_000,
        freeze_ts: 10_000,
        ..HeadsContract::default()
    };
    // Live since before the listing, constant: its delegation.
    let mut h = head(1, 1, 4_000, 1, 1, 500);
    assert_eq!(head_raw_weight(&h, &k, 3_000), Some(4_000));
    // Live from halfway through the window so far: half.
    h.live_ts = 2_000;
    h.accrual_since_ts = 500;
    assert_eq!(head_raw_weight(&h, &k, 3_000), Some(2_000));
    // Past the freeze: the window ends there.
    assert_eq!(head_raw_weight(&h, &k, 50_000), Some(4_000 * 8_000 / 9_000));
    // An accrual after the listing without a capture: unknown.
    h.accrual_since_ts = 1_500;
    assert_eq!(head_raw_weight(&h, &k, 3_000), None);
    // With the listing captured, known again.
    h.capture_periods[0] = key(9);
    h.capture_flags[0] = CAPTURE_LISTING;
    h.capture_listing[0] = 0;
    assert_eq!(head_raw_weight(&h, &k, 3_000), Some(2_000));
}

#[test]
fn an_undelegation_keeps_what_the_remaining_strikes_can_take_while_exposed() {
    let mut b = board(&RAMX_ONE);
    let mut h = head(1, 1, 0, 1, 1, LISTING - 10);
    // A member of the contract of ring slot 0 (round 7 review, B2).
    h.capture_periods[0] = key(9);
    h.capture_flags[0] = crate::state::heads::CAPTURE_MEMBER;
    let mut d = delegation(key(3), key(4), 0);
    d.held_principal = 1_000 * AMBA;
    d.held = 1_000 * AMBA;
    d.cooldown_end_ts = NOW + 86_400;
    d.exit_ts = NOW - 10;
    // Held principal that weighed in the contract of ring slot 0 (generation 1), cooling a day.
    (d.held_exposed[1], d.held_gens[0]) = (1_000 * AMBA, 1);
    (d.cooling[0], d.cooling_end[0]) = (1_000 * AMBA, NOW + 86_400);
    // No open contract and no case: nothing is exposed, all of it cools.
    let p = position(&b, &h, &d);
    assert_eq!(
        (p.amount, p.cooling, p.escrow, p.cooldown_ends_at),
        (0, 1_000 * AMBA, 0, Some(NOW + 86_400))
    );
    // A contract it weighed in is open: 19% (two strikes of 10%) stays in escrow.
    b.contract_periods[0] = key(9);
    b.contract_status[0] = RING_ACCUMULATING;
    b.contract_listing_ts[0] = LISTING;
    b.contract_freeze_ts[0] = FREEZE;
    b.contract_gens[0] = 1;
    let p = position(&b, &h, &d);
    assert_eq!((p.cooling, p.escrow), (810 * AMBA, 190 * AMBA));
    // The proposer keeps all of it (a lost source challenge takes 100%).
    d.is_proposer = true;
    assert_eq!(position(&b, &h, &d).escrow, 1_000 * AMBA);
}

#[test]
fn rewards_are_accrued_plus_what_the_index_added_while_live() {
    let scale = rewards::INDEX_SCALE;
    let mut b = board(&RAMX_ONE);
    b.reward_index = 3 * scale / 1_000;
    let h = head(1, 1, 1_000, 1, 1, LISTING);
    let live = usize::from(b.live_count);
    let entry = (0..live)
        .find(|&i| b.live_head_ids[i] == 1)
        .unwrap_or_else(|| {
            b.live_head_ids[live] = 1;
            b.live_components[live] = 1;
            b.live_count += 1;
            live
        });
    // The head went live when the board's index stood at 1 / 1,000: its own index is 2 / 1,000.
    b.live_flags[entry] = 0;
    b.live_reward_offsets[entry] = scale / 1_000;
    assert_eq!(head_reward_index(&b, &h), 2 * scale / 1_000);
    let mut d = delegation(key(3), key(4), 1_000);
    d.rewards_accrued = 5;
    assert_eq!(position(&b, &h, &d).rewards, 5 + 2);
    d.reward_index = scale / 1_000;
    assert_eq!(position(&b, &h, &d).rewards, 5 + 1);
    // A duplicate's index stands still at its frozen value: it earns nothing more.
    b.live_flags[entry] = crate::state::heads::LIVE_DUPLICATE;
    b.live_reward_offsets[entry] = scale / 1_000;
    assert_eq!(position(&b, &h, &d).rewards, 5, "a duplicate earns nothing");
    // A head that left the list keeps its frozen index.
    let mut retired = h.clone();
    retired.status = HeadStatus::Retired;
    retired.reward_index_frozen = 4 * scale / 1_000;
    assert_eq!(position(&b, &retired, &d).rewards, 5 + 3);
}

#[test]
fn decode_takes_only_the_canonical_account() {
    let program = key(200);
    let owner = key(5);
    let (address, bump) = find_heads_stake_address(&program, &owner);
    let stake = HeadsStake {
        discriminator: HeadsStake::DISCRIMINATOR,
        version: HeadsStake::VERSION,
        bump,
        owner,
        free: 7,
        ..HeadsStake::default()
    };
    let mut data = vec![0; HeadsStake::LEN];
    HeadsAccount::pack(&stake, &mut data).unwrap();
    assert_eq!(
        decode::<HeadsStake>(&program, &address, &program, &data)
            .unwrap()
            .free,
        7
    );
    assert!(
        decode::<HeadsStake>(&program, &key(6), &program, &data).is_none(),
        "another address"
    );
    assert!(
        decode::<HeadsStake>(&program, &address, &key(7), &data).is_none(),
        "another owner"
    );
    assert!(
        decode::<HeadsStake>(&program, &address, &program, &data[1..]).is_none(),
        "short"
    );
    assert_eq!(product_symbol(&product("NANDX")).as_deref(), Some("NANDX"));
    assert_eq!(product_symbol(&[0; 32]), None);
}

/// Round 13 review, finding 3: against a contract the indication prices its members, what its
/// settlement records: A (listed, evicted, +10% and most of the weight) counts; N (live since the
/// listing but no member of it, +100% on far more stake) does not. The live list's counts stay the
/// list's: B, C and N.
#[test]
fn a_contract_is_priced_by_its_members_evicted_or_not() {
    let mut a = head(1, 1, 3_000 * AMBA, 10_000_000, 1, LISTING - 10);
    print(&mut a, LISTING + 3_600, 11_000_000);
    a.evicted_ts = LISTING + 7_200;
    let b = head(2, 1, 1_000 * AMBA, 10_000_000, 2, LISTING - 10);
    let c = head(3, 1, 1_000 * AMBA, 10_000_000, 3, LISTING - 10);
    let mut n = head(4, 1, 50_000 * AMBA, 10_000_000, 4, LISTING + 100);
    print(&mut n, LISTING + 3_600, 20_000_000);
    let mut accounts = accounts(board(&RAMX_ONE), vec![a, b, c, n], 1);
    let listed = [(1, 1, 10_000_000), (2, 1, 10_000_000), (3, 1, 10_000_000)];
    let k = contract(&accounts, &listed);
    accounts.contracts.push((key(8), Box::new(k)));
    let v = view(&accounts, NOW);
    let component = &v.contracts[0].components[1];
    let priced: Vec<u32> = component.sources.iter().map(|s| s.head_id).collect();
    assert_eq!(priced, [1, 2, 3]);
    assert_eq!(component.live_sources, 3);
    match component.reading {
        Reading::Live { change_ppm, .. } => assert_eq!(change_ppm, 100_000),
        ref other => panic!("{other:?}"),
    }
}
