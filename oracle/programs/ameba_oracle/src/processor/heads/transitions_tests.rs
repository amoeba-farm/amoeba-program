use super::*;
use crate::state::heads::{HeadsChannel, HeadsContract, HeadsParams, RING_EMPTY, RING_SETTLED};

const DAY: u64 = 86_400;

fn params() -> Params {
    Params::spec_defaults()
}

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

fn board() -> HeadsBoard {
    // Any delegation backs a live source here (round 6 review, finding 1: only backed live
    // sources are origins).
    let mut params = HeadsParams::spec_defaults();
    (params.floor_activate, params.min_delegators) = (0, 1);
    let mut b = HeadsBoard {
        params,
        params_version: 1,
        item_count: 1,
        component_count: 3,
        ..HeadsBoard::default()
    };
    b.item_weight_bps[0] = 10_000;
    b.item_mix_bps[..3].copy_from_slice(&[2_000, 5_000, 3_000]);
    b.component_items[..3].copy_from_slice(&[0, 0, 0]);
    b.component_channels[..3].copy_from_slice(&[0, 1, 2]);
    b
}

fn config() -> HeadsConfig {
    HeadsConfig {
        amba_accounted: 1_000_000,
        ..HeadsConfig::default()
    }
}

fn head(id: u32, component: u8, now: u64) -> Head {
    Head {
        board: key(1),
        head_id: id,
        component,
        channel: HeadsChannel::Contract,
        proposer: key(2),
        posted_ts: now,
        expires_ts: now + 14 * DAY,
        accrual_since_ts: now,
        ..Head::default()
    }
}

fn delegation(owner: u8, now: u64) -> Delegation {
    Delegation {
        head: key(3),
        owner: key(owner),
        created_ts: now,
        accrual_since_ts: now,
        ..Delegation::default()
    }
}

/// A delegation joins at `now` with `amount`.
fn join(b: &mut HeadsBoard, h: &mut Head, d: &mut Delegation, amount: u64, now: u64) {
    let mut c = config();
    sync_delegation(&params(), &mut c, b, h, d, now).unwrap();
    add_delegation(b, h, d, amount, now).unwrap();
}

fn go_live(b: &mut HeadsBoard, h: &mut Head, origin: u8, now: u64) {
    h.status = HeadStatus::Active;
    h.delegators = h.delegators.max(1);
    h.value = 100_000_000;
    h.origin_id = [origin; 32];
    make_head_live(b, h, now).unwrap();
}

/// What an account contributed to ring slot `slot` (`freeze - listing` of its captures) once both
/// are captured for `period`.
fn contribution(
    periods: &[Pubkey; HEADS_CONTRACT_RING],
    listing: &[u128; HEADS_CONTRACT_RING],
    freeze: &[u128; HEADS_CONTRACT_RING],
    flags: &[u8; HEADS_CONTRACT_RING],
    slot: usize,
    period: &Pubkey,
) -> Option<u128> {
    let both = CAPTURE_LISTING | CAPTURE_FREEZE;
    (periods[slot] == *period && flags[slot] & both == both)
        .then(|| freeze[slot].saturating_sub(listing[slot]))
}

fn delegation_contribution(d: &Delegation, slot: usize, period: &Pubkey) -> Option<u128> {
    contribution(
        &d.capture_periods,
        &d.capture_listing,
        &d.capture_freeze,
        &d.capture_flags,
        slot,
        period,
    )
}

fn head_contribution(h: &Head, slot: usize, period: &Pubkey) -> Option<u128> {
    contribution(
        &h.capture_periods,
        &h.capture_listing,
        &h.capture_freeze,
        &h.capture_flags,
        slot,
        period,
    )
}

/// A source case opens on the head as `ChallengeSource` opens one: the next case number, and the
/// generations of the ring contracts unsettled now.
fn open(b: &HeadsBoard, h: &mut Head, now: u64) -> u64 {
    h.source_case += 1;
    for i in 0..HEADS_CONTRACT_RING {
        h.source_case_open_gens[i] = if b.contract_status[i] == RING_ACCUMULATING {
            b.contract_gens[i]
        } else {
            0
        };
    }
    open_case(h, now).unwrap()
}

fn list(b: &mut HeadsBoard, slot: usize, period: u8, listing: u64, freeze: u64) {
    b.contract_count += 1;
    b.contract_gens[slot] = b.contract_count;
    b.contract_periods[slot] = key(period);
    b.contract_listing_ts[slot] = listing;
    b.contract_freeze_ts[slot] = freeze;
    b.contract_expiry_ts[slot] = freeze + 10 * DAY;
    b.contract_status[slot] = RING_ACCUMULATING;
    // The listing makes the live sources in effect its members.
    for e in 0..usize::from(b.live_count) {
        b.live_flags[e] &= !(LIVE_MEMBER << slot);
        if b.live_in_effect(e, listing) {
            b.live_flags[e] |= LIVE_MEMBER << slot;
        }
    }
}

#[test]
fn live_overlap_clips_to_the_live_interval() {
    assert_eq!(live_overlap(0, 100, 0, 0), 0);
    assert_eq!(live_overlap(0, 100, 50, 0), 50);
    assert_eq!(live_overlap(60, 100, 50, 0), 40);
    assert_eq!(live_overlap(0, 100, 50, 80), 30);
    assert_eq!(live_overlap(90, 100, 50, 80), 0);
    assert_eq!(integral_at(7, 3, 0, 100, 50, 80).unwrap(), 7 + 90);
}

#[test]
fn the_head_integral_is_the_sum_of_its_delegations_at_every_boundary() {
    let (mut b, mut h) = (board(), head(0, 1, 1_000));
    let mut d1 = delegation(10, 1_000);
    let mut d2 = delegation(11, 1_100);
    join(&mut b, &mut h, &mut d1, 100, 1_000);
    join(&mut b, &mut h, &mut d2, 50, 1_100);
    assert_eq!(h.delegation, 150);
    assert_eq!(h.delegators, 2);
    go_live(&mut b, &mut h, 7, 1_200);
    list(&mut b, 0, 40, 1_300, 2_300);
    let mut c = config();
    // d1 leaves 40 mid-window, a new delegation joins, d2 adds 25 later; nobody touches anything
    // at the boundaries.
    sync_delegation(&params(), &mut c, &mut b, &mut h, &mut d1, 1_500).unwrap();
    undelegate(&params(), &mut b, &mut h, &mut d1, 40, 1_500).unwrap();
    let mut late = delegation(12, 1_800);
    join(&mut b, &mut h, &mut late, 10, 1_800);
    join(&mut b, &mut h, &mut d2, 25, 2_000);
    for d in [&mut d1, &mut d2, &mut late] {
        sync_delegation(&params(), &mut c, &mut b, &mut h, d, 2_500).unwrap();
    }
    let period = key(40);
    let one = delegation_contribution(&d1, 0, &period).unwrap();
    let two = delegation_contribution(&d2, 0, &period).unwrap();
    // A delegation made after the listing captured zero at it.
    let three = delegation_contribution(&late, 0, &period).unwrap();
    assert_eq!(one, 100 * 200 + 60 * 800);
    assert_eq!(two, 50 * 700 + 75 * 300);
    assert_eq!(three, 10 * 500);
    assert_eq!(head_contribution(&h, 0, &period), Some(one + two + three));
    // A missed boundary is a structural error, never a silent zero.
    let mut stale = delegation(13, 1_000);
    stale.accrual_since_ts = 2_400;
    assert!(sync_delegation_accrual(&b, &h, &mut stale, 2_600).is_err());
}

#[test]
fn a_reused_ring_slot_restarts_its_captures() {
    let (mut b, mut h) = (board(), head(0, 1, 1_000));
    let mut d = delegation(10, 1_000);
    join(&mut b, &mut h, &mut d, 100, 1_000);
    go_live(&mut b, &mut h, 7, 1_000);
    list(&mut b, 0, 40, 1_100, 1_200);
    let mut c = config();
    sync_delegation(&params(), &mut c, &mut b, &mut h, &mut d, 1_300).unwrap();
    list(&mut b, 0, 41, 1_400, 1_500);
    sync_delegation(&params(), &mut c, &mut b, &mut h, &mut d, 1_600).unwrap();
    assert_eq!(d.capture_periods[0], key(41));
    assert_eq!(d.capture_flags[0], CAPTURE_LISTING | CAPTURE_FREEZE);
    assert_eq!(delegation_contribution(&d, 0, &key(41)), Some(100 * 100));
    assert_eq!(delegation_contribution(&d, 0, &key(40)), None);
}

#[test]
fn strikes_slash_what_was_exposed_when_the_case_opened() {
    let p = params();
    let (mut b, mut h) = (board(), head(0, 1, 1_000));
    let mut active = delegation(10, 1_000);
    let mut left = delegation(11, 1_000);
    let mut late = delegation(12, 1_000);
    join(&mut b, &mut h, &mut active, 100, 1_000);
    join(&mut b, &mut h, &mut left, 100, 1_000);
    go_live(&mut b, &mut h, 7, 1_200);
    let opened = open(&b, &mut h, 1_400);
    // While the case is open, new delegation is fresh; the leaver's held stake stays exposed.
    join(&mut b, &mut h, &mut late, 50, 1_450);
    assert_eq!((late.active, late.fresh), (0, 50));
    let mut c = config();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut left, 1_500).unwrap();
    undelegate(&p, &mut b, &mut h, &mut left, 100, 1_500).unwrap();
    assert!(!record_strike(&p, &mut b, &mut h, opened, 1_600).unwrap());
    close_case(&mut h).unwrap();
    for d in [&mut active, &mut left, &mut late] {
        sync_delegation(&p, &mut c, &mut b, &mut h, d, 1_700).unwrap();
    }
    assert_eq!(active.active, 90);
    assert_eq!((left.held_principal, left.held), (90, 90));
    // The fresh stake came after the opening: untouched, and merged now nothing can strike it.
    assert_eq!((late.active, late.fresh), (50, 0));
    assert_eq!(c.pending_amba_burn, 20);
    assert_eq!(c.amba_accounted, 1_000_000 - 20);
    assert_eq!(h.delegation, 140);
    assert_eq!(b.total_delegation, 140);
    assert_eq!(b.live_delegation, 140);
    assert_eq!(active.slashed_total + left.slashed_total, 20);
    // Applied once only.
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut active, 1_800).unwrap();
    assert_eq!(active.active, 90);
}

#[test]
fn max_strikes_retire_the_head_and_its_delegations_cool() {
    let p = params();
    let (mut b, mut h) = (board(), head(0, 1, 1_000));
    let mut d = delegation(10, 1_000);
    join(&mut b, &mut h, &mut d, 100, 1_000);
    go_live(&mut b, &mut h, 7, 1_100);
    assert_eq!(b.live_count, 1);
    for (i, at) in [(0u64, 2_000u64), (1, 3_000)] {
        let opened = open(&b, &mut h, at - 10);
        let retired = record_strike(&p, &mut b, &mut h, opened, at).unwrap();
        close_case(&mut h).unwrap();
        assert_eq!(retired, i == 1);
    }
    assert_eq!(h.status, HeadStatus::Retired);
    assert_eq!(h.retired_ts, 3_000);
    assert_eq!(
        (b.live_count, b.live_delegation, b.total_delegation),
        (0, 0, 0)
    );
    assert_eq!(b.component_live_sources[1], 0);
    // A third strike records nothing: liability is bounded by max_strikes.
    assert!(!record_strike(&p, &mut b, &mut h, 3_100, 3_200).unwrap());
    assert_eq!(h.strike_count, 2);
    let mut c = config();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut d, 4_000).unwrap();
    // 100 -> 90 -> 81, then everything active cools from the retirement.
    assert_eq!((d.active, d.held, d.held_principal), (0, 81, 81));
    assert_eq!(d.cooldown_end_ts, 3_000 + p.cooldown_secs);
    assert_eq!(d.exit_ts, 3_000);
    assert_eq!(h.delegation, 0);
    assert_eq!(h.delegators, 0);
    // No strike is left, so nothing is escrowed: all of it comes back after the cooldown.
    assert_eq!(releasable(&p, &b, &h, &d, 3_000 + p.cooldown_secs - 1), 0);
    assert_eq!(releasable(&p, &b, &h, &d, 3_000 + p.cooldown_secs), 81);
}

#[test]
fn escrow_holds_what_the_remaining_strikes_can_take_while_exposed() {
    let p = params();
    let (mut b, mut h) = (board(), head(0, 1, 1_000));
    let mut d = delegation(10, 1_000);
    // The head's proposer (key 2) owns it: that makes it the proposer's delegation.
    let mut proposer = delegation(2, 1_000);
    join(&mut b, &mut h, &mut d, 1_000, 1_000);
    join(&mut b, &mut h, &mut proposer, 1_000, 1_000);
    go_live(&mut b, &mut h, 7, 1_000);
    list(&mut b, 0, 40, 1_100, 1_000 + 30 * DAY);
    let mut c = config();
    for x in [&mut d, &mut proposer] {
        sync_delegation(&p, &mut c, &mut b, &mut h, x, 2_000).unwrap();
        undelegate(&p, &mut b, &mut h, x, 1_000, 2_000).unwrap();
    }
    let after = 2_000 + p.cooldown_secs;
    assert_eq!(releasable(&p, &b, &h, &d, after - 1), 0);
    // It weighed in the unsettled contract: 19% stays (max_liability at two strikes left).
    for x in [&mut d, &mut proposer] {
        sync_delegation(&p, &mut c, &mut b, &mut h, x, after).unwrap();
    }
    assert!(exposed(&b, &h, &d));
    assert_eq!(required_escrow(&p, &b, &h, &d), 190);
    assert_eq!(releasable(&p, &b, &h, &d, after), 810);
    // The proposer's escrow is all of it.
    assert_eq!(required_escrow(&p, &b, &h, &proposer), 1_000);
    assert_eq!(releasable(&p, &b, &h, &proposer, after), 0);
    assert_eq!(release(&p, &b, &h, &mut d, after).unwrap(), 810);
    // The base stays: one strike takes 10% of the principal, leaving what one more could take.
    assert_eq!((d.held, d.held_principal), (190, 1_000));
    let opened = open(&b, &mut h, after);
    record_strike(&p, &mut b, &mut h, opened, after + 1).unwrap();
    close_case(&mut h).unwrap();
    // The case opened after the exit, while the escrow was kept for the unsettled contract: the
    // held stake answers for it as if still delegated (review 2026-10-30 #9).
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut d, after + 2).unwrap();
    assert_eq!((d.held, d.held_principal), (90, 900));
    assert_eq!(required_escrow(&p, &b, &h, &d), 90);
    assert_eq!(releasable(&p, &b, &h, &d, after + 2), 0);
    // Once the contract settles, nothing is exposed and the rest comes back.
    b.contract_status[0] = RING_SETTLED;
    assert!(!exposed(&b, &h, &d));
    assert_eq!(releasable(&p, &b, &h, &d, after + 3), 90);
    // A case opened after the settlement is not the escrow's.
    let opened = open(&b, &mut h, after + 4);
    assert!(!exposed(&b, &h, &d));
    record_strike(&p, &mut b, &mut h, opened, after + 5).unwrap();
    close_case(&mut h).unwrap();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut d, after + 6).unwrap();
    assert_eq!(d.held, 90);
    assert_eq!(release(&p, &b, &h, &mut d, after + 6).unwrap(), 90);
    assert_eq!((d.held, d.held_principal), (0, 0));
    assert!(closable(&b, &h, &d));
    // A held stake from before a case's opening is exposed to it.
    let mut e = delegation(12, 1_000);
    (e.held, e.held_principal, e.held_exposed[0]) = (10, 10, 10);
    open(&b, &mut h, 4_000);
    // It left during the open case: it answers it by number.
    (e.case_ref, e.held_case_adjust[0]) = (h.source_case, 10);
    assert!(exposed(&b, &h, &e));
    let mut f = e.clone();
    (f.case_ref, f.held_case_adjust[0]) = (0, 0);
    assert!(!exposed(&b, &h, &f));
}

#[test]
fn rewards_follow_the_live_non_duplicate_heads() {
    let p = params();
    let mut b = board();
    let (mut first, mut copy, mut open) = (head(1, 1, 1_000), head(2, 1, 1_000), head(3, 1, 1_000));
    let (mut d1, mut d2, mut d3) = (
        delegation(10, 1_000),
        delegation(11, 1_000),
        delegation(12, 1_000),
    );
    join(&mut b, &mut first, &mut d1, 300, 1_000);
    join(&mut b, &mut copy, &mut d2, 100, 1_000);
    join(&mut b, &mut open, &mut d3, 500, 1_000);
    list(&mut b, 0, 40, 900, 1_250);
    go_live(&mut b, &mut first, 7, 1_200);
    // Same origin, later: a duplicate. The freeze capture took the counts before it changed them.
    go_live(&mut b, &mut copy, 7, 1_300);
    assert!(copy.duplicate && !first.duplicate);
    assert_eq!(b.component_live_sources[1], 2);
    assert_eq!(b.component_live_origins[1], 1);
    assert_eq!(b.contract_freeze_origins[1], 1);
    assert_ne!(b.contract_flags[0] & RING_FREEZE_CAPTURED, 0);
    assert_eq!(b.live_delegation, 300);
    // 300 USDC atoms over 300 live delegation: one per atom.
    b.reward_index = rewards::index_after(b.reward_index, 300, b.live_delegation).unwrap();
    let mut c = config();
    for (h, d) in [
        (&mut first, &mut d1),
        (&mut copy, &mut d2),
        (&mut open, &mut d3),
    ] {
        sync_delegation(&p, &mut c, &mut b, h, d, 1_400).unwrap();
    }
    assert_eq!(
        (d1.rewards_accrued, d2.rewards_accrued, d3.rewards_accrued),
        (300, 0, 0)
    );
    // The first source retires: the copy keeps the origin and earns from now on.
    retire_head(&mut b, &mut first, 1_500).unwrap();
    refresh_head_flags(&b, &mut copy);
    assert!(!copy.duplicate);
    assert_eq!(b.live_delegation, 100);
    assert_eq!(b.component_live_origins[1], 1);
    b.reward_index = rewards::index_after(b.reward_index, 50, b.live_delegation).unwrap();
    for (h, d) in [(&mut first, &mut d1), (&mut copy, &mut d2)] {
        sync_delegation(&p, &mut c, &mut b, h, d, 1_600).unwrap();
    }
    assert_eq!((d1.rewards_accrued, d2.rewards_accrued), (300, 50));
    assert_eq!(first.reward_index_frozen, head_reward_index(&b, &first));
}

#[test]
fn a_new_origin_on_an_update_recomputes_duplicates() {
    let mut b = board();
    let (mut a, mut c) = (head(1, 1, 1_000), head(2, 1, 1_000));
    // Backed (needing one delegator: a positive total, round 18 review).
    join(&mut b, &mut a, &mut delegation(10, 1_000), 1, 1_000);
    join(&mut b, &mut c, &mut delegation(11, 1_000), 1, 1_000);
    go_live(&mut b, &mut a, 7, 1_100);
    go_live(&mut b, &mut c, 7, 1_200);
    assert_eq!(b.component_live_origins[1], 1);
    c.origin_id = [8; 32];
    update_live_print(&mut b, &mut c, 1_300).unwrap();
    assert!(!c.duplicate);
    assert_eq!(b.component_live_origins[1], 2);
    assert_eq!(b.live_flags[..2], [0, 0]);
}

#[test]
fn activation_needs_the_threshold_and_two_delegators() {
    let p = params();
    let mut b = board();
    let mut h = head(1, 1, 1_000);
    let (mut d1, mut d2) = (delegation(10, 1_000), delegation(11, 1_000));
    join(&mut b, &mut h, &mut d1, p.floor_activate, 1_000);
    assert!(!maybe_activate(&p, &b, &mut h, 1_000));
    join(&mut b, &mut h, &mut d2, 1, 1_000);
    assert!(maybe_activate(&p, &b, &mut h, 1_001));
    assert_eq!((h.status, h.active_ts), (HeadStatus::Active, 1_001));
    assert_eq!(min_post(&p, &b), p.floor_post);
    // Expiry: an Open head past its time to live.
    let mut late = head(2, 1, 1_000);
    assert!(expire_head(&mut b, &mut late, 1_000 + 14 * DAY - 1).is_err());
    expire_head(&mut b, &mut late, 1_000 + 14 * DAY).unwrap();
    assert_eq!(late.status, HeadStatus::Expired);
}

#[test]
fn the_proposer_pays_a_falling_source_case_once() {
    let p = params();
    let mut b = board();
    let mut h = head(1, 1, 1_000);
    let mut proposer = delegation(2, 1_000);
    let mut other = delegation(11, 1_000);
    join(&mut b, &mut h, &mut proposer, 1_000, 1_000);
    join(&mut b, &mut h, &mut other, 1_000, 1_000);
    go_live(&mut b, &mut h, 7, 1_000);
    let opened = open(&b, &mut h, 1_500);
    let mut c = config();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut proposer, 2_000).unwrap();
    let take = liability::slash(proposer_principal(&proposer, &h), p.proposer_slash_bps);
    slash_proposer(&mut b, &mut h, &mut proposer, take, 0).unwrap();
    record_strike(&p, &mut b, &mut h, opened, 2_000).unwrap();
    proposer.applied_strikes = h.strike_count;
    close_case(&mut h).unwrap();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut other, 2_100).unwrap();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut proposer, 2_100).unwrap();
    assert_eq!(weight_of(&proposer), 0);
    assert_eq!(proposer.proposer_slashed, 1_000);
    assert_eq!(other.active, 900);
    // Only the ordinary slash is burned; the proposer's AMBA goes to the winners.
    assert_eq!(c.pending_amba_burn, 100);
    assert_eq!(h.delegation, 900);
    assert_eq!(h.delegators, 1);
}

#[test]
fn an_empty_ring_slot_is_never_captured() {
    let (b, mut h) = (board(), head(0, 1, 1_000));
    assert_eq!(b.contract_status, [RING_EMPTY; 4]);
    sync_head(&b, &mut h, 2_000).unwrap();
    assert_eq!(h.capture_flags, [0; 4]);
    assert!(sync_head(&b, &mut h, 1_999).is_err());
}

#[test]
fn the_read_only_window_integral_agrees_with_the_captures() {
    let (mut b, mut h) = (board(), head(0, 1, 1_000));
    let mut d1 = delegation(10, 1_000);
    let mut d2 = delegation(11, 1_000);
    join(&mut b, &mut h, &mut d1, 100, 1_000);
    go_live(&mut b, &mut h, 7, 1_100);
    list(&mut b, 2, 40, 1_300, 2_300);
    let contract = HeadsContract {
        period: key(40),
        ring_slot: 2,
        listing_ts: 1_300,
        freeze_ts: 2_300,
        ..HeadsContract::default()
    };
    // Untouched since before the listing: computed from the running integral.
    assert_eq!(head_window_integral(&h, &contract).unwrap(), 100 * 1_000);
    // Touched inside the window: the listing captured, the freeze computed.
    join(&mut b, &mut h, &mut d2, 50, 1_800);
    assert_eq!(
        head_window_integral(&h, &contract).unwrap(),
        100 * 1_000 + 50 * 500
    );
    // Touched after the freeze: both captured; and it equals what sync_head captured.
    let mut synced = h.clone();
    sync_head(&b, &mut synced, 2_600).unwrap();
    let captured = head_contribution(&synced, 2, &key(40)).unwrap();
    assert_eq!(head_window_integral(&h, &contract).unwrap(), captured);
    assert_eq!(head_window_integral(&synced, &contract).unwrap(), captured);
    // A head posted after the window began counts from zero; a missed boundary is an error.
    let late = head(1, 1, 2_000);
    assert_eq!(head_window_integral(&late, &contract).unwrap(), 0);
    let mut stale = h.clone();
    stale.capture_flags = [0; 4];
    stale.accrual_since_ts = 2_400;
    assert!(head_window_integral(&stale, &contract).is_err());
}

/// A board needing one delegator, its activation floor 1,000.
fn pooled_board() -> HeadsBoard {
    let mut b = board();
    (b.params.floor_activate, b.params.min_delegators) = (1_000, 1);
    b
}

/// Round 18 review, N1: needing one delegator, every positive delegation counts, but backing never
/// reads the count, so it saturates: 65,536 one-atom wallets cannot exhaust it, an honest wallet
/// still joins, leaves and comes back, and the head's backing is its total alone (before, the
/// 65,536th delegation failed on the counter's overflow, and every one after it).
#[test]
fn one_atom_wallets_never_exhaust_a_pooled_heads_delegators() {
    let p = params();
    let mut b = pooled_board();
    let mut h = head(1, 1, 1_000);
    for _ in 0..=usize::from(u16::MAX) {
        join(&mut b, &mut h, &mut delegation(10, 1_000), 1, 1_000);
    }
    assert_eq!(h.delegators, u16::MAX);
    assert_eq!(h.delegation, u64::from(u16::MAX) + 1);
    let mut honest = delegation(11, 1_000);
    join(&mut b, &mut h, &mut honest, 1_000, 1_000);
    assert!(heads_backed(h.delegation, h.delegators, 1_000, 1));
    let mut c = config();
    sync_delegation(&p, &mut c, &mut b, &mut h, &mut honest, 2_000).unwrap();
    undelegate(&p, &mut b, &mut h, &mut honest, 1_000, 2_000).unwrap();
    // An attacker's next atom takes the place it left; the honest wallet comes back all the same.
    join(&mut b, &mut h, &mut delegation(12, 2_000), 1, 2_000);
    let mut back = delegation(13, 2_000);
    join(&mut b, &mut h, &mut back, 1_000, 2_000);
    assert_eq!(weight_of(&back), 1_000);
    assert!(heads_backed(h.delegation, h.delegators, 1_000, 1));
}

/// Round 18 review, N2: on a board needing one delegator, a delegation flagged dust under the
/// earlier rule left the head's count, and the live list's mirror, at none; a one-atom delegator
/// arriving and leaving then toggled the head's backing. Backing is the total alone there, so
/// the Live head stays backed throughout and never starts an unbacked time.
#[test]
fn a_stale_dust_flag_cannot_toggle_a_pooled_heads_backing() {
    let mut b = pooled_board();
    let mut h = head(1, 1, 1_000);
    let mut honest = delegation(11, 1_000);
    join(&mut b, &mut h, &mut honest, 1_000, 1_000);
    go_live(&mut b, &mut h, 7, 1_100);
    // The earlier rule's state: the honest delegation flagged dust, counted nowhere.
    let entry = live_entry(&b, h.head_id).unwrap();
    (honest.dust, h.delegators, b.live_delegators[entry]) = (true, 0, 0);
    assert!(b.live_backed(entry));
    let mut atom = delegation(12, 1_200);
    join(&mut b, &mut h, &mut atom, 1, 1_200);
    assert!(b.live_backed(entry) && h.unbacked_since_ts == 0);
    let mut c = config();
    sync_delegation(&params(), &mut c, &mut b, &mut h, &mut atom, 1_300).unwrap();
    undelegate(&params(), &mut b, &mut h, &mut atom, 1, 1_300).unwrap();
    assert!(b.live_backed(entry) && h.unbacked_since_ts == 0);
    assert_eq!(b.component_live_origins[1], 1);
}
