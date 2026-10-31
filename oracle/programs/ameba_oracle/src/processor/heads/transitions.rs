//! The state transitions both halves of the heads family share (`docs/heads.md`). Every function
//! here works on decoded accounts; loading, signer checks and storing stay with the handlers.
//!
//! # Time
//!
//! A head and each of its delegations keep a live-gated running integral of their weight-bearing
//! amount (`Head::delegation`; a delegation's `active + fresh`): amount times the overlap of the
//! held interval with the head's live interval `[live_ts, retired_ts)`. Every change of a
//! delegation's amount changes its head's `delegation` by the same amount in the same instruction
//! and at the same time, so the head's integral is always the sum of its delegations' integrals,
//! exactly. Before anything changes an amount, the caller brings both up to `now`
//! ([`sync_head`], [`sync_delegation`]): that also captures the integral at each open contract's
//! listing and freeze the first time it is touched after them (the amount was constant since the
//! last touch, so the value at the boundary is exact).
//!
//! # Strikes
//!
//! A strike is recorded on the head with the opening time of the case that caused it
//! ([`record_strike`]); each delegation applies the strikes it has not applied yet on its next
//! [`sync_delegation`], to the parts of it that were exposed when the case opened (`active`, a
//! `fresh` tranche added before the opening, `held` from an exit after it), at the time of
//! application: the slashed part leaves the head's delegation then, so the integrals stay exact.
//! What a strike slashes is burned ([`burn_to_pending`]). The proposer's part of a falling source
//! case is taken at resolution instead ([`slash_proposer`]).
//!
//! # Rewards
//!
//! The board's reward index grows with each funding over `live_delegation`, the delegation of
//! Live heads that are not duplicates. Each live entry encodes its head's own index
//! (`live_reward_offsets`), which advances only while the head earns, so duplicate flips never need
//! the flipped heads' accounts; a retiring head freezes its index into `reward_index_frozen`. A
//! delegation earns `rewards::earned` of its `active + fresh` over its head's index.
//!
//! # Order of a touch
//!
//! [`sync_delegation`] does, in order: captures and accruals of the head and the delegation,
//! rewards, pending strikes, the automatic cooldown of a retired or expired head's delegations,
//! and the merge of a `fresh` tranche nobody can strike any more. Handlers call it first, then
//! act ([`add_delegation`], [`undelegate`], [`release`], ...).

use super::*;
use crate::state::heads::{
    heads_backed, live_reset, Delegation, Head, HeadClaim, HeadClaimKind, HeadStatus, HeadsBoard,
    HeadsConfig, HeadsContract, CAPTURE_FREEZE, CAPTURE_JOINED, CAPTURE_LISTING, CAPTURE_MEMBER,
    HEADS_CONTRACT_RING, HEADS_MAX_COMPONENTS, HEADS_MAX_LIVE_SOURCES, HEAD_STRIKE_CAPACITY,
    HELD_COOLING, HELD_MASKS, LIVE_DUPLICATE, LIVE_MEMBER, RING_ACCUMULATING, RING_COLLECTING,
    RING_FREEZE_CAPTURED,
};
use ameba_oracle_heads::{channels, liability, rewards, thresholds, Params, BPS};

pub(super) use super::accounts::Res;
use super::accounts::{add, bad_state, overflow, sub};

fn full() -> ProgramError {
    OracleError::HeadsCapacityExhausted.into()
}

/// The current unix time from the clock sysvar.
pub(super) fn now() -> Res<u64> {
    u64::try_from(crate::compact_error::clock()?.unix_timestamp).map_err(|_| bad_state())
}

// Time and accruals
// ------------------------------------------------------------------------------------------

/// Seconds of `[from, to)` inside the live interval `[live_ts, retired_ts)` (`live_ts` 0: never
/// live; `retired_ts` 0: still live).
pub(super) fn live_overlap(from: u64, to: u64, live_ts: u64, retired_ts: u64) -> u64 {
    if live_ts == 0 {
        return 0;
    }
    let start = from.max(live_ts);
    let end = if retired_ts == 0 {
        to
    } else {
        to.min(retired_ts)
    };
    end.saturating_sub(start)
}

/// A live-gated integral `integral` of `amount` held since `since`, as of `at` (no earlier than
/// `since`).
pub(super) fn integral_at(
    integral: u128,
    amount: u64,
    since: u64,
    at: u64,
    live_ts: u64,
    retired_ts: u64,
) -> Res<u128> {
    let held = u128::from(amount)
        .checked_mul(u128::from(live_overlap(since, at, live_ts, retired_ts)))
        .ok_or_else(overflow)?;
    integral.checked_add(held).ok_or_else(overflow)
}

/// A delegation's weight-bearing amount: what counts in its head's `delegation`.
pub(super) fn weight_of(d: &Delegation) -> u64 {
    d.active.saturating_add(d.fresh)
}

/// The value of an integral at contract boundary `at` for an account last brought up to date at
/// `since` and created at `created`: exact when `since <= at`; zero when the account did not
/// exist yet at `at`; otherwise a boundary was missed, which never happens.
fn boundary_value(
    integral: u128,
    amount: u64,
    since: u64,
    created: u64,
    at: u64,
    live: (u64, u64),
) -> Res<u128> {
    if since <= at {
        integral_at(integral, amount, since, at, live.0, live.1)
    } else if created > at {
        Ok(0)
    } else {
        Err(bad_state())
    }
}

struct Captures<'a> {
    periods: &'a mut [Pubkey; HEADS_CONTRACT_RING],
    listing: &'a mut [u128; HEADS_CONTRACT_RING],
    freeze: &'a mut [u128; HEADS_CONTRACT_RING],
    flags: &'a mut [u8; HEADS_CONTRACT_RING],
}

#[allow(clippy::too_many_arguments)]
fn capture(
    board: &HeadsBoard,
    c: Captures<'_>,
    integral: u128,
    amount: u64,
    since: u64,
    created: u64,
    live: (u64, u64),
    now: u64,
) -> Res<()> {
    for i in 0..HEADS_CONTRACT_RING {
        let status = board.contract_status[i];
        if status != RING_ACCUMULATING && status != RING_COLLECTING {
            continue;
        }
        let period = board.contract_periods[i];
        if c.periods[i] != period {
            // The slot holds a later contract: the earlier one settled before it was reused.
            c.periods[i] = period;
            c.listing[i] = 0;
            c.freeze[i] = 0;
            c.flags[i] = 0;
        }
        for (flag, at, value) in [
            (
                CAPTURE_LISTING,
                board.contract_listing_ts[i],
                &mut c.listing[i],
            ),
            (
                CAPTURE_FREEZE,
                board.contract_freeze_ts[i],
                &mut c.freeze[i],
            ),
        ] {
            if c.flags[i] & flag == 0 && now >= at {
                *value = boundary_value(integral, amount, since, created, at, live)?;
                c.flags[i] |= flag;
            }
        }
    }
    Ok(())
}

/// Copies the membership of each ring contract its listing (or the head's joining) set on the
/// head's live entry onto the head (`CAPTURE_MEMBER`), so it outlives the entry (round 7 review,
/// B2). Its capture slots must be the board's (`capture`).
pub(super) fn mark_membership(board: &HeadsBoard, head: &mut Head) {
    if let Some(entry) = live_entry(board, head.head_id).filter(|_| head.status == HeadStatus::Live)
    {
        for i in 0..HEADS_CONTRACT_RING {
            if head.capture_periods[i] == board.contract_periods[i] && board.live_member(entry, i) {
                head.capture_flags[i] |= CAPTURE_MEMBER;
            }
        }
    }
}

/// Brings the head up to `now`: captures its integral at every open contract boundary it passed
/// since its last touch, starts the samples of a contract new to its ring slot
/// (`samples::restart`), then advances the integral. Call before changing `delegation`,
/// `live_ts`, `retired_ts`, `claim_seq`, `open_claims` or a claim's admission.
pub(super) fn sync_head(board: &HeadsBoard, head: &mut Head, now: u64) -> Res<()> {
    if now < head.accrual_since_ts {
        return Err(bad_state());
    }
    for i in 0..HEADS_CONTRACT_RING {
        let open = matches!(
            board.contract_status[i],
            RING_ACCUMULATING | RING_COLLECTING
        );
        if open && head.capture_periods[i] != board.contract_periods[i] {
            // A later contract reuses the slot; `capture` restarts its flags below.
            super::samples::restart(board, head, i);
        } else if !open {
            // Settled (or empty): no record waits on the slot any more, and nothing tracks its
            // count as claims close, so it goes (round 4 N1: it outlived the claims it counted).
            head.adm_open[i] = 0;
        }
    }
    let live = (head.live_ts, head.retired_ts);
    let (integral, amount, since, created) = (
        head.accrual_integral,
        head.delegation,
        head.accrual_since_ts,
        head.posted_ts,
    );
    capture(
        board,
        Captures {
            periods: &mut head.capture_periods,
            listing: &mut head.capture_listing,
            freeze: &mut head.capture_freeze,
            flags: &mut head.capture_flags,
        },
        integral,
        amount,
        since,
        created,
        live,
        now,
    )?;
    mark_membership(board, head);
    head.accrual_integral = integral_at(integral, amount, since, now, live.0, live.1)?;
    head.accrual_since_ts = now;
    Ok(())
}

/// A claim left the open set (finalized or refunded; [`sync_head`] first).
pub(super) fn close_claim(head: &mut Head, claim: &HeadClaim) -> Res<()> {
    head.open_claims = head.open_claims.checked_sub(1).ok_or_else(bad_state)?;
    if claim.kind == HeadClaimKind::Fill && head.open_fill && head.open_fill_seq == claim.seq {
        head.open_fill = false;
        head.open_fill_seq = 0;
    }
    Ok(())
}

/// Brings the delegation's integral and captures up to `now` against its head's live interval.
/// [`sync_delegation`] calls it.
pub(super) fn sync_delegation_accrual(
    board: &HeadsBoard,
    head: &Head,
    d: &mut Delegation,
    now: u64,
) -> Res<()> {
    if now < d.accrual_since_ts {
        return Err(bad_state());
    }
    let live = (head.live_ts, head.retired_ts);
    let (integral, amount, since, created) = (
        d.accrual_integral,
        weight_of(d),
        d.accrual_since_ts,
        d.created_ts,
    );
    capture(
        board,
        Captures {
            periods: &mut d.capture_periods,
            listing: &mut d.capture_listing,
            freeze: &mut d.capture_freeze,
            flags: &mut d.capture_flags,
        },
        integral,
        amount,
        since,
        created,
        live,
        now,
    )?;
    for i in 0..HEADS_CONTRACT_RING {
        if d.capture_periods[i] == board.contract_periods[i] {
            d.capture_gen[i] = board.contract_gens[i];
        }
    }
    d.accrual_integral = integral_at(integral, amount, since, now, live.0, live.1)?;
    d.accrual_since_ts = now;
    Ok(())
}

/// The head's integral over a contract's window without changing the head (SettleComponent
/// reads heads read-only): each boundary's capture when the head took it, else exactly what
/// [`sync_head`] would capture now (the same `boundary_value`). It is the sum of its delegations'
/// integrals, so it is the source's settlement weight (divided by the window).
pub(super) fn head_window_integral(head: &Head, contract: &HeadsContract) -> Res<u128> {
    let slot = usize::from(contract.ring_slot);
    let ours = head.capture_periods[slot] == contract.period;
    let live = (head.live_ts, head.retired_ts);
    let at = |flag: u8, boundary: u64| -> Res<u128> {
        if ours && head.capture_flags[slot] & flag != 0 {
            Ok(if flag == CAPTURE_LISTING {
                head.capture_listing[slot]
            } else {
                head.capture_freeze[slot]
            })
        } else {
            boundary_value(
                head.accrual_integral,
                head.delegation,
                head.accrual_since_ts,
                head.posted_ts,
                boundary,
                live,
            )
        }
    };
    let listing = at(CAPTURE_LISTING, contract.listing_ts)?;
    let freeze = at(CAPTURE_FREEZE, contract.freeze_ts)?;
    Ok(freeze.saturating_sub(listing))
}

// The board's live list
// ------------------------------------------------------------------------------------------

/// The live-list position of head `head_id`.
pub(super) fn live_entry(board: &HeadsBoard, head_id: u32) -> Option<usize> {
    board.live_head_ids[..usize::from(board.live_count)]
        .iter()
        .position(|id| *id == head_id)
}

fn is_duplicate(board: &HeadsBoard, entry: usize) -> bool {
    board.live_flags[entry] & LIVE_DUPLICATE != 0
}

/// A live entry's reward index (see `HeadsBoard::live_reward_offsets`).
fn entry_reward_index(board: &HeadsBoard, entry: usize) -> u128 {
    if is_duplicate(board, entry) {
        board.live_reward_offsets[entry]
    } else {
        board
            .reward_index
            .saturating_sub(board.live_reward_offsets[entry])
    }
}

/// Sets entry `entry`'s duplicate flag, re-encoding its reward index and moving its delegation
/// in or out of `live_delegation`.
pub(super) fn set_live_flags(board: &mut HeadsBoard, entry: usize, duplicate: bool) -> Res<()> {
    if is_duplicate(board, entry) == duplicate {
        return Ok(());
    }
    let index = entry_reward_index(board, entry);
    let delegation = board.live_delegations[entry];
    if duplicate {
        board.live_flags[entry] |= LIVE_DUPLICATE;
        board.live_reward_offsets[entry] = index;
        board.live_delegation = sub(board.live_delegation, delegation)?;
    } else {
        board.live_flags[entry] &= !LIVE_DUPLICATE;
        board.live_reward_offsets[entry] =
            board.reward_index.checked_sub(index).ok_or_else(overflow)?;
        board.live_delegation = add(board.live_delegation, delegation)?;
    }
    Ok(())
}

/// Captures every open contract's freeze coverage at its freeze, the first time anything is
/// about to change it after the freeze (`HeadsBoard::contract_freeze_origins`): per component,
/// the contract's own members still backed with a price in effect at the freeze
/// (`HeadsBoard::member_coverage`, round 7 review, B1). Every change of a live entry's
/// membership, backing, origin or price calls this first, so the inputs are still the freeze's;
/// `SettleComponent` computes the same for a contract nothing touched since its freeze.
pub(super) fn capture_freeze_origins(board: &mut HeadsBoard, now: u64) {
    let due: [bool; HEADS_CONTRACT_RING] = core::array::from_fn(|slot| {
        matches!(
            board.contract_status[slot],
            RING_ACCUMULATING | RING_COLLECTING
        ) && board.contract_flags[slot] & RING_FREEZE_CAPTURED == 0
            && now >= board.contract_freeze_ts[slot]
    });
    if !due.contains(&true) {
        return;
    }
    // One grouping of a component's origins serves every contract captured now.
    for c in 0..usize::from(board.component_count) {
        let (members, entries) = board.origin_entries(c);
        let first = channels::first_of_origin(&entries);
        for slot in (0..HEADS_CONTRACT_RING).filter(|&slot| due[slot]) {
            let freeze = board.contract_freeze_ts[slot];
            let mut keep = Vec::with_capacity(members.len());
            for &e in &members {
                keep.push(
                    board.live_member(e, slot)
                        && board.live_backed(e)
                        && board.live_in_effect(e, freeze),
                );
            }
            board.contract_freeze_origins[slot * HEADS_MAX_COMPONENTS + c] =
                channels::count_origins(&first, &keep) as u16;
        }
    }
    for slot in (0..HEADS_CONTRACT_RING).filter(|&slot| due[slot]) {
        board.contract_flags[slot] |= RING_FREEZE_CAPTURED;
    }
}

/// Recomputes a component's duplicate flags (`channels::duplicate_flags` over its live entries:
/// earliest live, then lowest head id, keeps an origin) and its distinct live origins. Call after
/// any change of the component's live set or of an entry's origin. Heads whose flag flips need not
/// be loaded: their earning index lives in the board; a loaded head reads its flag back with
/// [`refresh_head_flags`].
pub(super) fn recompute_duplicates(board: &mut HeadsBoard, component: u8, now: u64) -> Res<()> {
    capture_freeze_origins(board, now);
    // Duplicates for rewards among all of them, as before; origins toward `min_live_sources`
    // only among those still backed (round 6 review, finding 1): one pass over the pairs.
    let (members, entries) = board.origin_entries(usize::from(component));
    let mut backed = Vec::with_capacity(members.len());
    for &e in &members {
        backed.push(board.live_backed(e));
    }
    let (flags, origins) = channels::duplicates_and_origins(&entries, &backed);
    for (&i, &duplicate) in members.iter().zip(&flags) {
        set_live_flags(board, i, duplicate)?;
    }
    board.component_live_origins[usize::from(component)] =
        u16::try_from(origins).map_err(|_| overflow())?;
    Ok(())
}

/// Copies the board's duplicate flag of a loaded head into `Head::duplicate`.
pub(super) fn refresh_head_flags(board: &HeadsBoard, head: &mut Head) {
    head.duplicate = head.status == HeadStatus::Live
        && live_entry(board, head.head_id).is_some_and(|e| is_duplicate(board, e));
}

/// Whether the head's delegation counts in the board's `total_delegation`.
fn counted(head: &Head) -> bool {
    matches!(
        head.status,
        HeadStatus::Open | HeadStatus::Active | HeadStatus::Live
    )
}

/// Changes the head's `delegation` by `+increase - decrease` (its `delegators` already counted),
/// with the board's totals and its live entry: a live entry that starts or stops meeting
/// activation backing changes its component's origins, the freeze's captured first (round 6
/// review, finding 1). The head (and the delegation that changed) must be synced to `now` first.
pub(super) fn change_head_delegation(
    board: &mut HeadsBoard,
    head: &mut Head,
    increase: u64,
    decrease: u64,
    now: u64,
) -> Res<()> {
    head.delegation = sub(add(head.delegation, increase)?, decrease)?;
    if counted(head) {
        board.total_delegation = sub(add(board.total_delegation, increase)?, decrease)?;
    }
    // Since when it has been unbacked (round 11 review, finding 3); with `floor_activate` and
    // `min_delegators` fixed once a head exists, only a delegation change moves its backing.
    if heads_backed(
        head.delegation,
        head.delegators,
        board.params.floor_activate,
        board.params.min_delegators,
    ) {
        head.unbacked_since_ts = 0;
    } else if head.unbacked_since_ts == 0 {
        head.unbacked_since_ts = now;
    }
    // A Live head off the list (`evict_head`) has no entry to follow.
    if let Some(entry) = live_entry(board, head.head_id).filter(|_| head.status == HeadStatus::Live)
    {
        capture_freeze_origins(board, now);
        let backed = board.live_backed(entry);
        board.live_delegations[entry] =
            sub(add(board.live_delegations[entry], increase)?, decrease)?;
        board.live_delegators[entry] = live_delegators(head);
        if !is_duplicate(board, entry) {
            board.live_delegation = sub(add(board.live_delegation, increase)?, decrease)?;
        }
        if board.live_backed(entry) != backed {
            recompute_duplicates(board, head.component, now)?;
        }
    }
    Ok(())
}

/// A head's delegators as its live entry keeps them (`HeadsBoard::live_delegators`).
fn live_delegators(head: &Head) -> u8 {
    u8::try_from(head.delegators).unwrap_or(u8::MAX)
}

// Head lifecycle
// ------------------------------------------------------------------------------------------

/// Makes an Active head Live at `now` (its fill stood): appends it to the board's live list with
/// its current `value` and `origin_id` (set them first), starts its reward index at zero, and
/// recomputes its component's duplicates. Syncs the head first.
pub(super) fn make_head_live(board: &mut HeadsBoard, head: &mut Head, now: u64) -> Res<()> {
    if head.status != HeadStatus::Active || live_entry(board, head.head_id).is_some() {
        return Err(bad_state());
    }
    sync_head(board, head, now)?;
    capture_freeze_origins(board, now);
    let entry = usize::from(board.live_count);
    if entry >= HEADS_MAX_LIVE_SOURCES {
        return Err(full());
    }
    let component = usize::from(head.component);
    board.live_head_ids[entry] = head.head_id;
    board.live_components[entry] = head.component;
    board.live_flags[entry] = 0;
    board.live_since_ts[entry] = now;
    board.live_delegations[entry] = head.delegation;
    board.live_delegators[entry] = live_delegators(head);
    board.live_values[entry] = head.value;
    board.live_origin_ids[entry] = head.origin_id;
    board.live_reset_ts[entry] = live_reset(head.lta_next_reset_ts);
    board.live_reward_offsets[entry] = board.reward_index;
    board.live_count += 1;
    board.live_delegation = add(board.live_delegation, head.delegation)?;
    board.component_live_sources[component] = board.component_live_sources[component]
        .checked_add(1)
        .ok_or_else(overflow)?;
    head.status = HeadStatus::Live;
    head.live_ts = now;
    for i in 0..HEADS_CONTRACT_RING {
        // A member of every contract still accumulating: it must be recorded at settlement.
        let at = i * HEADS_MAX_COMPONENTS + component;
        if board.contract_status[i] == RING_ACCUMULATING && now < board.contract_freeze_ts[i] {
            board.contract_joined[at] = board.contract_joined[at]
                .checked_add(1)
                .ok_or_else(overflow)?;
            head.capture_flags[i] |= CAPTURE_JOINED | CAPTURE_MEMBER;
            board.live_flags[entry] |= LIVE_MEMBER << i;
        }
    }
    recompute_duplicates(board, head.component, now)?;
    refresh_head_flags(board, head);
    Ok(())
}

/// Records an accepted print's value and origin on the board's live entry (the head's fields are
/// set by the caller), recomputing duplicates when the origin changed.
pub(super) fn update_live_print(board: &mut HeadsBoard, head: &mut Head, now: u64) -> Res<()> {
    // An evicted head prints for its contracts (its samples) and is on no list.
    let Some(entry) = live_entry(board, head.head_id) else {
        return Ok(());
    };
    capture_freeze_origins(board, now);
    board.live_values[entry] = head.value;
    board.live_reset_ts[entry] = live_reset(head.lta_next_reset_ts);
    if board.live_origin_ids[entry] != head.origin_id {
        board.live_origin_ids[entry] = head.origin_id;
        recompute_duplicates(board, head.component, now)?;
    }
    refresh_head_flags(board, head);
    Ok(())
}

fn remove_live_entry(board: &mut HeadsBoard, entry: usize) {
    let last = usize::from(board.live_count) - 1;
    board.live_head_ids[entry] = board.live_head_ids[last];
    board.live_components[entry] = board.live_components[last];
    board.live_flags[entry] = board.live_flags[last];
    board.live_since_ts[entry] = board.live_since_ts[last];
    board.live_delegations[entry] = board.live_delegations[last];
    board.live_values[entry] = board.live_values[last];
    board.live_origin_ids[entry] = board.live_origin_ids[last];
    board.live_reward_offsets[entry] = board.live_reward_offsets[last];
    board.live_reset_ts[entry] = board.live_reset_ts[last];
    board.live_delegators[entry] = board.live_delegators[last];
    board.live_head_ids[last] = 0;
    board.live_components[last] = 0;
    board.live_flags[last] = 0;
    board.live_since_ts[last] = 0;
    board.live_delegations[last] = 0;
    board.live_values[last] = 0;
    board.live_origin_ids[last] = [0; 32];
    board.live_reward_offsets[last] = 0;
    board.live_reset_ts[last] = 0;
    board.live_delegators[last] = 0;
    board.live_count -= 1;
}

/// Retires the head at `now` (`max_strikes` reached): no new claims; it leaves the live list
/// (its reward index frozen) and the board's totals, its component's duplicates are recomputed,
/// and its delegations cool on their next sync. Open liabilities stay with the delegations.
pub(super) fn retire_head(board: &mut HeadsBoard, head: &mut Head, now: u64) -> Res<()> {
    if matches!(head.status, HeadStatus::Retired | HeadStatus::Expired) {
        return Err(bad_state());
    }
    leave_live_list(board, head, now)?;
    leave_unfrozen_contracts(board, head, now);
    board.total_delegation = sub(board.total_delegation, head.delegation)?;
    head.status = HeadStatus::Retired;
    head.retired_ts = now;
    head.duplicate = false;
    Ok(())
}

/// Takes a Live head off the live list, if it is on it (its reward index frozen, its component's
/// duplicates recomputed), synced and with the freeze coverage captured first.
fn leave_live_list(board: &mut HeadsBoard, head: &mut Head, now: u64) -> Res<()> {
    sync_head(board, head, now)?;
    capture_freeze_origins(board, now);
    let Some(entry) = live_entry(board, head.head_id).filter(|_| head.status == HeadStatus::Live)
    else {
        return Ok(());
    };
    head.reward_index_frozen = entry_reward_index(board, entry);
    if !is_duplicate(board, entry) {
        board.live_delegation = sub(board.live_delegation, board.live_delegations[entry])?;
    }
    remove_live_entry(board, entry);
    let component = usize::from(head.component);
    board.component_live_sources[component] =
        sub(u64::from(board.component_live_sources[component]), 1)? as u16;
    head.duplicate = false;
    recompute_duplicates(board, head.component, now)
}

/// A source that joined a contract's window and retires before its freeze can never count there
/// (not eligible at the cutoff, not live at the freeze), so it leaves the membership it took
/// (round 5 item 2): the record settlement needs never holds it, and a replacement can join.
/// Members are then sources live at the freeze or still live, so a contract's joiners never
/// outnumber the live list (96), however many joined and retired before.
fn leave_unfrozen_contracts(board: &mut HeadsBoard, head: &mut Head, now: u64) {
    let component = usize::from(head.component);
    for i in 0..HEADS_CONTRACT_RING {
        let at = i * HEADS_MAX_COMPONENTS + component;
        if head.capture_flags[i] & CAPTURE_JOINED != 0
            && head.capture_periods[i] == board.contract_periods[i]
            && board.contract_status[i] == RING_ACCUMULATING
            && now < board.contract_freeze_ts[i]
        {
            head.capture_flags[i] &= !(CAPTURE_JOINED | CAPTURE_MEMBER);
            board.contract_joined[at] = board.contract_joined[at].saturating_sub(1);
        }
    }
}

/// Expires an Open head past `expires_ts`: it leaves the board's totals and its delegations cool
/// on their next sync (an Open head never had a claim, so they carry no liability). On a full
/// live list it also frees the slot of a Live head no longer backed as an activation needs (round
/// 8 review, finding 1). One that nothing could still answer through (round 9 review, findings 1
/// and 2: no member of an unsettled contract, no case open) retires. Any other, once unbacked for
/// `cooldown_secs` without a break (round 11 review, finding 3), only leaves the list (round 12
/// review, N1 and N2): it stays Live for everything but the list, a member of its contracts with
/// its eligibility and weight there (a joiner leaves a contract whose freeze is ahead: round 13
/// review, finding 1), its samples printed, its delegations uncooled, open to source cases; it is
/// no origin and joins nothing, and never comes back (`Head::evicted_ts`). Once none
/// of its contracts is unsettled and no case is open, this retires it. A head unbacked since
/// before round 11 has no time yet: this call starts it.
pub(super) fn expire_head(board: &mut HeadsBoard, head: &mut Head, now: u64) -> Res<()> {
    if head.status == HeadStatus::Live {
        let bound = (0..HEADS_CONTRACT_RING)
            .any(|i| board.weighs_in(head, i, board.contract_gens[i]))
            || head.unresolved_cases > 0;
        let Some(entry) = live_entry(board, head.head_id) else {
            // Evicted: it retires once nothing holds it.
            return if bound {
                Err(bad_state())
            } else {
                retire_head(board, head, now)
            };
        };
        if usize::from(board.live_count) < HEADS_MAX_LIVE_SOURCES || board.live_backed(entry) {
            return Err(bad_state());
        }
        let member = (0..HEADS_CONTRACT_RING)
            .any(|i| board.live_member(entry, i) && board.unsettled(i, board.contract_gens[i]));
        if !member && head.unresolved_cases == 0 {
            return retire_head(board, head, now);
        }
        if head.unbacked_since_ts == 0 {
            head.unbacked_since_ts = now;
            return Ok(());
        }
        if now
            < head
                .unbacked_since_ts
                .saturating_add(board.params.cooldown_secs)
        {
            return Err(bad_state());
        }
        leave_live_list(board, head, now)?;
        // A joiner leaves a contract whose freeze is still ahead, as on retirement (round 13
        // review, finding 1): its join is free for a replacement, and a contract's members at
        // its freeze stay its listed sources and the joiners then live (at most 96 each).
        leave_unfrozen_contracts(board, head, now);
        head.evicted_ts = now;
        return Ok(());
    }
    if head.status != HeadStatus::Open || now < head.expires_ts {
        return Err(bad_state());
    }
    sync_head(board, head, now)?;
    board.total_delegation = sub(board.total_delegation, head.delegation)?;
    head.status = HeadStatus::Expired;
    head.retired_ts = now;
    Ok(())
}

/// The live sources that count for thresholds (duplicates excluded): their number `n` and
/// delegations.
pub(super) fn independent_live(board: &HeadsBoard) -> (usize, Vec<u64>) {
    let delegations: Vec<u64> = (0..usize::from(board.live_count))
        .filter(|&i| !is_duplicate(board, i))
        .map(|i| board.live_delegations[i])
        .collect();
    (delegations.len(), delegations)
}

/// `min_post` of the board now (`thresholds::min_post`).
pub(super) fn min_post(params: &Params, board: &HeadsBoard) -> u64 {
    let (n, _) = independent_live(board);
    thresholds::min_post(
        params.floor_post,
        params.alpha_bps,
        board.total_delegation,
        n,
    )
}

/// The activation threshold of the board now (`thresholds::activation_threshold`).
pub(super) fn activation_threshold(params: &Params, board: &HeadsBoard) -> u64 {
    let (_, mut delegations) = independent_live(board);
    thresholds::activation_threshold(params.floor_activate, params.beta_bps, &mut delegations)
}

/// Activates an Open head that clears the threshold with enough delegators (it clears it once:
/// an Active head never falls back). Returns whether it activated.
pub(super) fn maybe_activate(
    params: &Params,
    board: &HeadsBoard,
    head: &mut Head,
    now: u64,
) -> bool {
    if head.status != HeadStatus::Open || now >= head.expires_ts {
        return false;
    }
    let threshold = activation_threshold(params, board);
    if heads_backed(
        head.delegation,
        head.delegators,
        threshold,
        params.min_delegators,
    ) {
        head.status = HeadStatus::Active;
        head.active_ts = now;
        return true;
    }
    false
}

// Cases and strikes
// ------------------------------------------------------------------------------------------

/// A source case opens on the head (its first challenge): delegation added from now on goes to
/// `fresh` until it closes. Returns the opening time to record (`source_case_opened_ts`).
pub(super) fn open_case(head: &mut Head, now: u64) -> Res<u64> {
    if head.unresolved_cases == 0 {
        head.unresolved_since_ts = now;
    }
    head.unresolved_cases = head.unresolved_cases.checked_add(1).ok_or_else(overflow)?;
    Ok(now)
}

/// A case resolved (record its strike, if any, first).
pub(super) fn close_case(head: &mut Head) -> Res<()> {
    head.unresolved_cases = head.unresolved_cases.checked_sub(1).ok_or_else(bad_state)?;
    if head.unresolved_cases == 0 {
        head.unresolved_since_ts = 0;
    }
    Ok(())
}

/// Records a strike from a lost source case opened at `opened_ts` (the only strikes: claim
/// challenges never strike); at `max_strikes` the head retires. Delegations apply it on their next
/// sync. A head that already has `max_strikes` records nothing more (a source can cost its
/// delegators at most `max_strikes` slashes); the case still resolves. Returns whether the head
/// retired.
pub(super) fn record_strike(
    params: &Params,
    board: &mut HeadsBoard,
    head: &mut Head,
    opened_ts: u64,
    now: u64,
) -> Res<bool> {
    if head.strike_count >= params.max_strikes {
        return Ok(false);
    }
    let slot = usize::from(head.strike_count);
    if slot >= HEAD_STRIKE_CAPACITY {
        return Err(full());
    }
    head.strike_opened_ts[slot] = opened_ts;
    head.strike_ts[slot] = now;
    head.strike_case[slot] = head.source_case;
    head.strike_open_gens[slot * HEADS_CONTRACT_RING..(slot + 1) * HEADS_CONTRACT_RING]
        .copy_from_slice(&head.source_case_open_gens);
    head.strike_count += 1;
    if head.strike_count >= params.max_strikes
        && matches!(head.status, HeadStatus::Live | HeadStatus::Active)
    {
        retire_head(board, head, now)?;
        return Ok(true);
    }
    Ok(false)
}

/// Applies the strikes of `head` this delegation has not applied yet, each to the parts exposed
/// when its case opened. A proposer skips them (it paid `proposer_slash_bps` at resolution). Returns
/// `(burned, weight_removed)`: everything slashed is burned; `weight_removed` is what left
/// `active + fresh` (the caller moves it out of the head with [`change_head_delegation`]).
pub(super) fn apply_pending_strikes(
    params: &Params,
    board: &HeadsBoard,
    head: &Head,
    d: &mut Delegation,
) -> Res<(u64, u64)> {
    let (mut burned, mut removed) = (0u64, 0u64);
    let _ = board;
    for s in usize::from(d.applied_strikes)..usize::from(head.strike_count) {
        if d.is_proposer {
            continue;
        }
        // Case numbers order the case against the stake, whatever the timestamps (re-review N7).
        let case = head.strike_case[s];
        let rate = params.slash_rate_bps;
        // What weighed in each contract is the oldest stake: `active` before `fresh`.
        let (on_active, on_fresh) = (d.active_since_case < case, d.fresh_since_case < case);
        for i in 0..HEADS_CONTRACT_RING {
            let older = d.weighed[i].min(d.active);
            let newer = d.weighed[i] - older;
            let off = if on_active {
                liability::slash(older, rate)
            } else {
                0
            } + if on_fresh {
                liability::slash(newer, rate)
            } else {
                0
            };
            d.weighed[i] -= off;
        }
        if d.active > 0 && d.active_since_case < case {
            let taken = liability::slash(d.active, rate);
            d.active -= taken;
            burned = add(burned, taken)?;
            removed = add(removed, taken)?;
        }
        if d.fresh > 0 && d.fresh_since_case < case {
            let taken = liability::slash(d.fresh, rate);
            d.fresh -= taken;
            burned = add(burned, taken)?;
            removed = add(removed, taken)?;
        }
        // Held stake answers for a case opened while a contract it weighed in was unsettled, or
        // that it left during (escrow is slashable as if still delegated): by contract
        // generation and case number, never time or ring slot (re-review N5, N6), each
        // exposure for itself (round 6 review, findings 3, 4 and 6).
        let gens = &head.strike_open_gens[s * HEADS_CONTRACT_RING..(s + 1) * HEADS_CONTRACT_RING];
        burned = add(burned, strike_held(d, case, gens, rate)?)?;
    }
    d.applied_strikes = head.strike_count;
    if d.case_ref != 0 && (head.unresolved_cases == 0 || head.source_case != d.case_ref) {
        // Its case resolved and any strike of it applied: nothing answers by number any more.
        d.held_case_adjust = [0; HELD_MASKS];
        d.case_ref = 0;
    }
    settle_held(d);
    d.slashed_total = add(d.slashed_total, burned)?;
    Ok((burned, removed))
}

/// What the head's open source case can take from the proposer if it falls (the delegation
/// synced): everything still delegated, and of its held principal what answers the case, as for
/// any delegator (`Delegation::held_answering`: a part naming a contract the head was a member of
/// when the case opened, or that left during it). Held principal answering nothing is not the
/// case's, synced and released or not (round 15 review: a lazy proposer lost what an eager one
/// had back).
pub(super) fn proposer_principal(d: &Delegation, head: &Head) -> u64 {
    let gens = &head.source_case_open_gens;
    (0..HELD_MASKS)
        .map(|m| d.held_answering(m, head.source_case, gens))
        .fold(weight_of(d), u64::saturating_add)
}

/// Takes `amount` (`outcome::resolve_source`'s `proposer_slashed`) from the proposer's delegation,
/// `active`, then `fresh`, then the held principal answering the open case
/// ([`proposer_principal`]), for the winning challengers (not burned). Returns what it actually
/// took: the winners are awarded that and never more (principal already released backs nothing).
/// Sync the delegation first, call before [`record_strike`], then set `applied_strikes` to the
/// head's new `strike_count` so the proposer skips that strike.
pub(super) fn slash_proposer(
    board: &mut HeadsBoard,
    head: &mut Head,
    d: &mut Delegation,
    amount: u64,
    now: u64,
) -> Res<u64> {
    if !d.is_proposer || amount > proposer_principal(d, head) {
        return Err(bad_state());
    }
    let before = weight_of(d);
    let mut left = amount;
    let from_active = left.min(d.active);
    d.active -= from_active;
    left -= from_active;
    let from_fresh = left.min(d.fresh);
    d.fresh -= from_fresh;
    left -= from_fresh;
    // Then off the held principal answering the case, each part in proportion, and out of what
    // is held.
    let gens = head.source_case_open_gens;
    let (off, from_held) = take_held(d, head.source_case, &gens, BPS as u16, left)?;
    let taken = from_active + from_fresh + from_held;
    if left > off {
        return Err(bad_state());
    }
    // What stays delegated keeps all it weighed in.
    for i in 0..HEADS_CONTRACT_RING {
        d.weighed[i] = d.weighed[i].min(weight_of(d));
    }
    settle_held(d);
    d.proposer_slashed = add(d.proposer_slashed, taken)?;
    after_weight_change(board, head, d, before, now)?;
    Ok(taken)
}

/// Moves `burned` AMBA from what the family owes into its pending burn (`BurnPending` burns it).
pub(super) fn burn_to_pending(config: &mut HeadsConfig, burned: u64) -> Res<()> {
    if burned == 0 {
        return Ok(());
    }
    config.amba_accounted = sub(config.amba_accounted, burned)?;
    config.pending_amba_burn = add(config.pending_amba_burn, burned)?;
    Ok(())
}

/// Moves the head's delegation by the delegation's weight change. On a board needing two
/// delegators or more, a delegation counts as one of the head's delegators (toward
/// `min_delegators`) only while it holds at least its share of the activation floor,
/// `floor_activate / min_delegators` (round 9 review, dust delegators): an essential delegator
/// commits real, exposed stake. Needing one, every positive delegation counts, so stake pooled
/// over wallets activates a source (the user's decision, 2026-10-31). `Delegation::dust` keeps
/// whether it counted, so a later change of the parameters never miscounts it.
fn after_weight_change(
    board: &mut HeadsBoard,
    head: &mut Head,
    d: &mut Delegation,
    before: u64,
    now: u64,
) -> Res<()> {
    let after = weight_of(d);
    let share = board.params.floor_activate / u64::from(board.params.min_delegators.max(1));
    let counted = before > 0 && !d.dust;
    d.dust = board.params.min_delegators >= 2 && after > 0 && after < share;
    match (counted, after > 0 && !d.dust) {
        (true, false) => head.delegators = head.delegators.saturating_sub(1),
        // Needing one delegator, backing never reads the count (`heads_backed`): it saturates, so
        // no number of delegations exhausts it (round 18 review, N1).
        (false, true) if board.params.min_delegators < 2 => {
            head.delegators = head.delegators.saturating_add(1);
        }
        (false, true) => {
            head.delegators = head.delegators.checked_add(1).ok_or_else(overflow)?;
        }
        _ => {}
    }
    if after >= before {
        change_head_delegation(board, head, after - before, 0, now)
    } else {
        change_head_delegation(board, head, 0, before - after, now)
    }
}

// Rewards
// ------------------------------------------------------------------------------------------

/// The head's delegator reward index now: its live entry's while Live, frozen once retired, zero
/// before it ever went live.
pub(super) fn head_reward_index(board: &HeadsBoard, head: &Head) -> u128 {
    match live_entry(board, head.head_id) {
        Some(entry) if head.status == HeadStatus::Live => entry_reward_index(board, entry),
        _ => head.reward_index_frozen,
    }
}

/// Accrues what the delegation's `active + fresh` earned since its last accrual (duplicates and
/// heads that are not Live earn nothing: their index does not move).
pub(super) fn accrue_rewards(board: &HeadsBoard, head: &Head, d: &mut Delegation) -> Res<()> {
    let index = head_reward_index(board, head);
    let earned = rewards::earned(weight_of(d), d.reward_index, index);
    d.rewards_accrued = add(d.rewards_accrued, earned)?;
    d.reward_index = index;
    Ok(())
}

// The touch
// ------------------------------------------------------------------------------------------

/// Everything a delegation owes before anything acts on it at `now`, in order: the head's and the
/// delegation's captures and accruals, its rewards, its pending strikes (burned to the config's
/// pending burn), the cooldown of what a retired or expired head still holds active, and the
/// merge of a `fresh` tranche no unresolved case can strike any more.
pub(super) fn sync_delegation(
    params: &Params,
    config: &mut HeadsConfig,
    board: &mut HeadsBoard,
    head: &mut Head,
    d: &mut Delegation,
    now: u64,
) -> Res<()> {
    // The proposer's delegation is the one its proposer owns, however it was made (a closed and
    // recreated one, or one from before this rule).
    d.is_proposer = crate::pubkey_eq(&d.owner, &head.proposer);
    sync_head(board, head, now)?;
    let since = d.accrual_since_ts;
    sync_delegation_accrual(board, head, d, now)?;
    accrue_rewards(board, head, d)?;
    let before = weight_of(d);
    let (burned, _) = apply_pending_strikes(params, board, head, d)?;
    burn_to_pending(config, burned)?;
    after_weight_change(board, head, d, before, now)?;
    normalize_held(board, head, d)?;
    weigh(board, head, d, since, now);
    if matches!(head.status, HeadStatus::Retired | HeadStatus::Expired) && weight_of(d) > 0 {
        let cooldown_end = head
            .retired_ts
            .checked_add(params.cooldown_secs)
            .ok_or_else(overflow)?;
        cool(
            board,
            head,
            d,
            weight_of(d),
            head.retired_ts,
            cooldown_end,
            now,
        )?;
    }
    merge_fresh(head, d);
    Ok(())
}

fn merge_fresh(head: &Head, d: &mut Delegation) {
    // Still exempt from the open case, the one it was added during.
    if d.fresh == 0 || (head.unresolved_cases != 0 && head.source_case <= d.fresh_since_case) {
        return;
    }
    if d.active == 0 {
        d.active_since_ts = d.fresh_since_ts;
        d.active_since_case = d.fresh_since_case;
    }
    d.active += d.fresh;
    d.fresh = 0;
    d.fresh_since_ts = 0;
}

/// Adds `amount` of the owner's free stake to the delegation at `now` (sync it first): to
/// `active`, or to `fresh` while the head has an unresolved case.
pub(super) fn add_delegation(
    board: &mut HeadsBoard,
    head: &mut Head,
    d: &mut Delegation,
    amount: u64,
    now: u64,
) -> Res<()> {
    let before = weight_of(d);
    if head.unresolved_cases > 0 {
        if d.fresh == 0 {
            d.fresh_since_ts = now;
            d.fresh_since_case = head.source_case;
        }
        d.fresh = add(d.fresh, amount)?;
    } else {
        if d.active == 0 {
            d.active_since_ts = now;
            d.active_since_case = head.source_case;
        }
        d.active = add(d.active, amount)?;
    }
    after_weight_change(board, head, d, before, now)
}

// Held stake: exposure by contract (round 6 review, findings 3, 4 and 6)
// ------------------------------------------------------------------------------------------
//
// Held principal is kept by the set of ring contracts it weighed in (`held_exposed`, one entry
// per subset of the ring), never joined to another exposure. Strikes and escrow work on one base
// for the delegation, so rounding is never split. `weighed[i]` is the delegated stake that
// weighed in slot `i`'s contract: always the oldest, since a withdrawal takes the newest first.

/// Clears everything held once nothing is: no strike can take anything any more.
pub(super) fn settle_held(d: &mut Delegation) {
    if d.held == 0 {
        d.held_principal = 0;
        d.held_exposed = [0; HELD_MASKS];
        d.held_case_adjust = [0; HELD_MASKS];
        d.case_ref = 0;
        d.cooling = [0; HELD_COOLING];
        d.cooling_end = [0; HELD_COOLING];
    }
}

/// Drops from the held masks every ring slot whose contract settled (or whose slot a later one
/// took), or of which the head is no member (round 7 review, B2): no later case can reach it
/// through that contract. What the open case reached only through it answers that case by
/// number from then on (`held_case_adjust` of the smaller mask), so it answers exactly what it
/// did. Call after the pending strikes.
pub(super) fn normalize_held(board: &HeadsBoard, head: &Head, d: &mut Delegation) -> Res<()> {
    let open = head.unresolved_cases > 0;
    let gens = head.source_case_open_gens;
    for i in 0..HEADS_CONTRACT_RING {
        if d.held_gens[i] == 0 || board.weighs_in(head, i, d.held_gens[i]) {
            continue;
        }
        let bit = 1 << i;
        for m in (0..HELD_MASKS).filter(|m| m & bit != 0) {
            let into = m & !bit;
            let mut adjust = d.held_case_adjust[m];
            if open && d.mask_meets(m, &gens) && !d.mask_meets(into, &gens) {
                adjust = d.held_answering(m, head.source_case, &gens);
                d.case_ref = head.source_case;
            }
            d.held_exposed[into] = add(d.held_exposed[into], d.held_exposed[m])?;
            d.held_case_adjust[into] = add(d.held_case_adjust[into], adjust)?;
            d.held_exposed[m] = 0;
            d.held_case_adjust[m] = 0;
        }
        d.held_gens[i] = 0;
        d.weighed[i] = 0;
    }
    Ok(())
}

/// Weighs the delegated stake in each unsettled contract the head is a member of whose window
/// ran, with the head live, since the delegation's last touch `since`: all of `active + fresh`
/// was delegated then. A contract the head is no member of is never weighed in (round 7 review,
/// B2).
fn weigh(board: &HeadsBoard, head: &Head, d: &mut Delegation, since: u64, now: u64) {
    let live_end = if head.retired_ts == 0 {
        u64::MAX
    } else {
        head.retired_ts
    };
    for i in 0..HEADS_CONTRACT_RING {
        let gen = board.contract_gens[i];
        if !board.weighs_in(head, i, gen) {
            continue;
        }
        d.held_gens[i] = gen;
        let start = since.max(board.contract_listing_ts[i]).max(head.live_ts);
        let end = now.min(board.contract_freeze_ts[i]).min(live_end);
        if head.live_ts != 0 && start < end {
            d.weighed[i] = weight_of(d);
        }
    }
}

/// Splits `total` over `parts` (summing to `base >= total`) in proportion, to the atom.
fn spread(parts: &[u64; HELD_MASKS], total: u64, base: u64) -> [u64; HELD_MASKS] {
    let mut out = [0u64; HELD_MASKS];
    if base == 0 {
        return out;
    }
    let mut left = total;
    for m in 0..HELD_MASKS {
        out[m] = (u128::from(total) * u128::from(parts[m]) / u128::from(base)) as u64;
        left -= out[m];
    }
    for m in 0..HELD_MASKS {
        if left > 0 && out[m] < parts[m] {
            out[m] += 1;
            left -= 1;
        }
    }
    out
}

/// One strike of case `case` (opened with `gens` unsettled) on the held stake: it takes
/// `slash_rate` of the principal answering it ([`take_held`]). Returns the burn.
fn strike_held(d: &mut Delegation, case: u32, gens: &[u32], rate: u16) -> Res<u64> {
    Ok(take_held(d, case, gens, rate, u64::MAX)?.1)
}

/// Takes `rate` of the held principal answering case `case` (opened with `gens` unsettled;
/// `Delegation::held_answering`, one base), at most `cap`, off each answering part's principal in
/// proportion, and out of what is held. Returns that principal and what left `held`. One instance
/// for strikes and the proposer's slash (its rate 100%, its cap what is left to take).
fn take_held(d: &mut Delegation, case: u32, gens: &[u32], rate: u16, cap: u64) -> Res<(u64, u64)> {
    let parts: [u64; HELD_MASKS] = core::array::from_fn(|m| d.held_answering(m, case, gens));
    let base = parts.iter().sum::<u64>();
    let struck = liability::slash(base, rate).min(cap);
    let cut = spread(&parts, struck, base);
    let by_number = d.case_ref == case;
    for m in 0..HELD_MASKS {
        d.held_exposed[m] -= cut[m];
        if by_number && !d.mask_meets(m, gens) {
            d.held_case_adjust[m] -= cut[m];
        }
    }
    d.held_principal = sub(d.held_principal, struck)?;
    let burned = struck.min(d.held);
    d.held -= burned;
    Ok((struck, burned))
}

/// Adds `amount` cooling until `end`: to the entry of that end, else a free one; with all four
/// taken by other ends, the two closest ends join at the later one.
pub(super) fn add_cooling(d: &mut Delegation, amount: u64, end: u64) -> Res<()> {
    let k = (0..HELD_COOLING)
        .find(|&k| d.cooling_end[k] == end)
        .or_else(|| (0..HELD_COOLING).find(|&k| d.cooling_end[k] == 0));
    if let Some(k) = k {
        d.cooling[k] = add(d.cooling[k], amount)?;
        d.cooling_end[k] = end;
        return Ok(());
    }
    let mut all = [(end, amount); HELD_COOLING + 1];
    for k in 0..HELD_COOLING {
        all[k] = (d.cooling_end[k], d.cooling[k]);
    }
    sort_few(&mut all, |e| e.0);
    let j = (0..HELD_COOLING)
        .min_by_key(|&j| all[j + 1].0 - all[j].0)
        .ok_or_else(full)?;
    all[j + 1].1 = add(all[j + 1].1, all[j].1)?;
    let kept = all.iter().enumerate().filter(|(i, _)| *i != j);
    for (k, (_, (end, amount))) in kept.enumerate() {
        d.cooling_end[k] = *end;
        d.cooling[k] = *amount;
    }
    Ok(())
}

/// Sorts a handful of entries by `key` in place: an insertion sort, so the program links no
/// general-purpose sort for these few.
fn sort_few<T, K: PartialOrd>(v: &mut [T], key: impl Fn(&T) -> K) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && key(&v[j - 1]) > key(&v[j]) {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Forgets cooling that ended by `now`.
fn cooled(d: &mut Delegation, now: u64) {
    for k in 0..HELD_COOLING {
        if d.cooling_end[k] <= now {
            d.cooling[k] = 0;
            d.cooling_end[k] = 0;
        }
    }
}

/// Moves `amount` of `active + fresh` (the newest first: `fresh`, then the latest `active`) into
/// held stake, cooling until `cooldown_end`. Each part keeps the contracts it weighed in: the
/// oldest `weighed[i]` of the delegated stake weighed in slot `i`'s contract, so of what leaves,
/// the last `amount - (weight - weighed[i])` did. During a case, what came from `fresh` stays
/// exempt from it and what came from `active` answers it by number (`held_case_adjust`); a
/// proposer's `fresh` answers it like its `active` (round 16 review, N1: the proposer answers for
/// everything it delegated during its case, withdrawn or not). Never refused for room: every
/// exposure has its own entry. Sync first.
#[allow(clippy::too_many_arguments)]
fn cool(
    board: &mut HeadsBoard,
    head: &mut Head,
    d: &mut Delegation,
    amount: u64,
    exit_ts: u64,
    cooldown_end: u64,
    now: u64,
) -> Res<()> {
    let before = weight_of(d);
    let from_fresh = amount.min(d.fresh);
    // Positions in what leaves, newest first: position p weighed in slot i from `cut[i]` on.
    let cut: [u64; HEADS_CONTRACT_RING] = core::array::from_fn(|i| {
        if d.held_gens[i] != 0 && d.weighed[i] > 0 {
            (before - d.weighed[i]).min(amount)
        } else {
            amount
        }
    });
    let mut points = [0u64; HEADS_CONTRACT_RING + 3];
    points[..HEADS_CONTRACT_RING].copy_from_slice(&cut);
    points[HEADS_CONTRACT_RING..].copy_from_slice(&[0, from_fresh, amount]);
    sort_few(&mut points, |p| *p);
    let open = head.unresolved_cases > 0;
    let gens = head.source_case_open_gens;
    for w in points.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        if lo == hi {
            continue;
        }
        let m = (0..HEADS_CONTRACT_RING)
            .filter(|&i| cut[i] <= lo)
            .fold(0usize, |m, i| m | 1 << i);
        d.held_exposed[m] = add(d.held_exposed[m], hi - lo)?;
        if open {
            // Positions below `from_fresh` came from `fresh` (none exempt for the proposer).
            let fresh = if d.is_proposer {
                0
            } else {
                from_fresh.min(hi).saturating_sub(lo)
            };
            let adjust = if d.mask_meets(m, &gens) {
                fresh
            } else {
                hi - lo - fresh
            };
            d.held_case_adjust[m] = add(d.held_case_adjust[m], adjust)?;
            d.case_ref = head.source_case;
        }
    }
    for i in 0..HEADS_CONTRACT_RING {
        d.weighed[i] = d.weighed[i].min(before - amount);
    }
    cooled(d, now);
    add_cooling(d, amount, cooldown_end)?;
    d.held = add(d.held, amount)?;
    d.held_principal = add(d.held_principal, amount)?;
    d.cooldown_end_ts = d.cooldown_end_ts.max(cooldown_end);
    d.exit_case = d.exit_case.max(head.source_case);
    d.fresh -= from_fresh;
    if d.fresh == 0 {
        d.fresh_since_ts = 0;
    }
    d.active = sub(d.active, amount - from_fresh)?;
    if d.active == 0 {
        d.active_since_ts = 0;
    }
    d.exit_ts = d.exit_ts.max(exit_ts);
    after_weight_change(board, head, d, before, now)
}

/// Undelegates `amount` at `now` (sync first): it stops carrying weight and earning, and is held
/// until the cooldown ends; the escrow part stays held while exposure lasts ([`release`]).
pub(super) fn undelegate(
    params: &Params,
    board: &mut HeadsBoard,
    head: &mut Head,
    d: &mut Delegation,
    amount: u64,
    now: u64,
) -> Res<()> {
    if amount == 0 || amount > weight_of(d) {
        return Err(OracleError::InsufficientHeadsStake.into());
    }
    let cooldown_end = now.checked_add(params.cooldown_secs).ok_or_else(overflow)?;
    cool(board, head, d, amount, now, cooldown_end, now)
}

// Escrow and release
// ------------------------------------------------------------------------------------------

/// Whether the delegation still bears exposure (spec section 5): an unsettled contract of the
/// ring it contributed weight to, or held principal a strike can still reach. The delegation
/// must be synced to now.
pub(super) fn exposed(board: &HeadsBoard, head: &Head, d: &Delegation) -> bool {
    // Only a contract the head is a member of (round 8 review, finding 2).
    let contracts = (0..HEADS_CONTRACT_RING).any(|i| {
        board.weighs_in(head, i, board.contract_gens[i])
            && d.capture_periods[i] == board.contract_periods[i]
            && contributed(d, i)
    });
    contracts || (d.held > 0 && d.held_exposure(board, head) > 0)
}

/// Whether the delegation's captures in ring slot `i` show weight it contributed to that slot's
/// contract (the window started and its integral grew since).
pub(super) fn contributed(d: &Delegation, i: usize) -> bool {
    if crate::pubkey_is_default(&d.capture_periods[i]) || d.capture_flags[i] & CAPTURE_LISTING == 0
    {
        // The window has not started; held stake contributes nothing to it.
        return false;
    }
    let end = if d.capture_flags[i] & CAPTURE_FREEZE != 0 {
        d.capture_freeze[i]
    } else {
        d.accrual_integral
    };
    end > d.capture_listing[i]
}

/// The escrow the delegation must keep held (`Delegation::held_escrow`: what the head's remaining
/// strikes can take of the held principal they can reach).
pub(super) fn required_escrow(
    params: &Params,
    board: &HeadsBoard,
    head: &Head,
    d: &Delegation,
) -> u64 {
    d.held_escrow(params, board, head)
}

/// What may go back to free stake at `now`: what is held above its escrow and its cooling.
pub(super) fn releasable(
    params: &Params,
    board: &HeadsBoard,
    head: &Head,
    d: &Delegation,
    now: u64,
) -> u64 {
    d.held_releasable(params, board, head, now)
}

/// Releases everything [`releasable`] at `now` and returns it for the owner's free stake. The
/// held principal stays (the base the remaining strikes slash and the escrow is sized on) until
/// nothing is held.
pub(super) fn release(
    params: &Params,
    board: &HeadsBoard,
    head: &Head,
    d: &mut Delegation,
    now: u64,
) -> Res<u64> {
    cooled(d, now);
    let out = d.held_releasable(params, board, head, now);
    d.held -= out;
    settle_held(d);
    Ok(out)
}

/// Whether the delegation can close: nothing active, fresh, held or unclaimed, no exposure, and
/// no uncollected contribution to an unsettled contract.
pub(super) fn closable(board: &HeadsBoard, head: &Head, d: &Delegation) -> bool {
    weight_of(d) == 0 && d.held == 0 && d.rewards_accrued == 0 && !exposed(board, head, d)
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod tests;
