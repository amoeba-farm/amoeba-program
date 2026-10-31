//! A contract's levels from fixed samples (re-review N1, N2, limits (a) and (b); round 3 R1).
//!
//! Each contract has ten fixed instants: five in its freeze level's window
//! `[settlement_window_start(freeze), freeze]` and five in its settlement window
//! `[freeze, expiry]`, evenly spaced, the last at each window's end ([`instants`]). A sample is the
//! source's level at its instant: the latest print the contract admits with a source time at or
//! before it, latest by source time and then by what it says ([`Key`]), unless past its own
//! reset (an LTA print; every LTA print is in effect at its source time, `claims::reveal_claim`:
//! round 3 R3, R4, R6). A level is the median of its window's samples, so how often a page is captured cannot
//! move it, and the 32-print ring (kept for display) never decides it.
//!
//! Each ring slot keeps, per instant, the print deciding it so far: its source time, value and
//! reset (`sample_ts`, `sample_values`, `sample_resets`; its level at the instant follows). A print the contract admits ([`admitted`]: committed
//! before the expiry, its window ended and every challenge ruled by the contract's cutoff K,
//! standing) takes every instant at or after its source time whose deciding print is earlier
//! ([`print`]). The latest is a maximum, so the order claims finalize in never matters (re-review
//! N1), and a claim that never prints (it fell, or stood unvetted at its ruling deadline) moves
//! nothing: there is no watermark and no queue, and any committed claim reveals in its window
//! whatever was revealed since (round 3 R1). A sample is final once nothing the contract may admit
//! is open: settlement waits for those ([`waits`]), and a claim ruled after K never counts.

use super::accounts::{overflow, Res};
pub(super) use crate::state::heads::heads_sample_instants as instants;
use crate::state::heads::{
    heads_rebuild_slot, Head, HeadClaim, HeadClaimStatus, HeadsBoard, HEADS_CONTRACT_RING,
    HEAD_SLOT_SAMPLES, HEAD_WINDOW_SAMPLES, RING_ACCUMULATING, RING_COLLECTING,
};
#[cfg(test)]
use crate::state::heads::{heads_sampled_levels as levels, HeadsContract};

/// Samples per window.
pub(super) const SAMPLES: usize = HEAD_WINDOW_SAMPLES;
/// Samples per contract slot: the freeze window's, then the settlement window's.
pub(super) const SLOT_SAMPLES: usize = HEAD_SLOT_SAMPLES;

fn board_instants(board: &HeadsBoard, i: usize) -> [u64; SLOT_SAMPLES] {
    instants(
        board.contract_freeze_window_ts[i],
        board.contract_freeze_ts[i],
        board.contract_expiry_ts[i],
    )
}

fn open(board: &HeadsBoard, head: &Head, i: usize) -> bool {
    matches!(
        board.contract_status[i],
        RING_ACCUMULATING | RING_COLLECTING
    ) && head.capture_periods[i] == board.contract_periods[i]
}

/// An LTA reset as a sample keeps it (`Head::sample_resets`): unix seconds, at most `u32::MAX`
/// (2106, past every instant a contract has); 0: none.
fn sample_reset(reset: u64) -> u32 {
    u32::try_from(reset).unwrap_or(u32::MAX)
}

/// A print as an instant orders it: by source time, then by what it says that decides the sample
/// (its value, then its reset). Two prints with one key make the same sample, so no order of
/// arrival can choose between them; at the freeze and expiry instants their origins break the
/// tie too (the record carries those). A total order on what prints say, not a hash: no two
/// different observations compare equal by accident, and a replay compares equal to the
/// original, so it displaces nothing (round 4 N3; round 5, item 1).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    ts: u64,
    value: u64,
    reset: u32,
}

fn claim_key(claim: &HeadClaim) -> Key {
    Key {
        ts: claim.source_ts,
        value: claim.value,
        reset: sample_reset(claim.lta_next_reset_ts),
    }
}

/// Whether `claim` is the head's latest print (its `value` then): by source time, then by
/// everything it says, so the head's value is the same in every order prints come in.
pub(super) fn latest(head: &Head, claim: &HeadClaim) -> bool {
    (
        claim.source_ts,
        claim.value,
        claim.lta_next_reset_ts,
        claim.origin_id,
        claim.lta_effective_from_ts,
        claim.lta_floor,
        claim.lta_ceiling,
    ) > (
        head.value_source_ts,
        head.value,
        head.lta_next_reset_ts,
        head.origin_id,
        head.lta_effective_from_ts,
        head.lta_floor,
        head.lta_ceiling,
    )
}

fn set_sample(head: &mut Head, k: usize, key: Key) {
    head.sample_ts[k] = key.ts;
    head.sample_values[k] = key.value;
    head.sample_resets[k] = key.reset;
}

/// A slot starts on the contract the head first sees there ([`super::transitions::sync_head`]):
/// every instant from the head's latest print then (every print syncs the head first, and a
/// contract lists no later than its first instant, so that print decides every instant from
/// the listing on), waiting for every claim revealed and still open then: the contract may admit
/// them.
pub(super) fn restart(board: &HeadsBoard, head: &mut Head, i: usize) {
    rebuild(board, head, i);
    head.restart_reveals[i] = head.reveals;
    // Open revealed claims: no more than `open_claims`.
    head.adm_open[i] =
        u16::try_from(head.reveals.saturating_sub(head.finalized_reveals)).unwrap_or(u16::MAX);
}

/// Slot `i`'s samples and origins from the head's latest print alone: what a slot starting now
/// has, and what the dev migration rebuilds a version-1 head's samples to while no instant of the
/// contract came before that print.
pub(super) fn rebuild(board: &HeadsBoard, head: &mut Head, i: usize) {
    heads_rebuild_slot(head, i, &board_instants(board, i));
}

/// Whether settlement must wait for one of `head`'s claims before recording slot `i`'s contract
/// (the head's slot is on it): one revealed before the slot started is open, or one the contract
/// admits.
pub(super) fn waits(head: &Head, i: usize) -> bool {
    head.adm_open[i] > 0
}

/// Whether ring slot `i`'s contract admits `claim`, on facts that do not change after its cutoff
/// K: an open claim the record waits for, or a standing one as it finalizes.
pub(super) fn admitted(board: &HeadsBoard, i: usize, claim: &HeadClaim) -> bool {
    let cutoff = board.contract_cutoff_ts[i];
    claim.status == HeadClaimStatus::Revealed
        && claim.commit_ts < board.contract_expiry_ts[i]
        && claim.window_end_ts <= cutoff
        && claim.won_count == 0
        && claim.ruled_count == claim.challenge_count
        && (claim.challenge_count == 0 || claim.last_ruled_ts <= cutoff)
}

/// The slots that count `claim` in `adm_open` now: revealed before the slot started and open,
/// or revealed since and admitted. [`track`] compares them before and after a change.
pub(super) fn admissions(board: &HeadsBoard, head: &Head, claim: &HeadClaim) -> [bool; 4] {
    core::array::from_fn(|i| {
        open(board, head, i)
            && claim.status == HeadClaimStatus::Revealed
            && (claim.reveal_index < head.restart_reveals[i] || admitted(board, i, claim))
    })
}

/// Moves the head's `adm_open` from `before` (`admissions` before the change) to now.
pub(super) fn track(
    board: &HeadsBoard,
    head: &mut Head,
    before: [bool; 4],
    claim: &HeadClaim,
) -> Res<()> {
    let after = admissions(board, head, claim);
    for i in 0..HEADS_CONTRACT_RING {
        if before[i] && !after[i] {
            head.adm_open[i] = head.adm_open[i].saturating_sub(1);
        } else if after[i] && !before[i] {
            head.adm_open[i] = head.adm_open[i].checked_add(1).ok_or_else(overflow)?;
        }
    }
    Ok(())
}

/// A standing claim prints (the head synced, the claim still `Revealed`): each open contract that
/// admits it takes it at every instant at or after its source time whose deciding print orders
/// before it ([`Key`]; at the freeze and expiry instants a tie goes by origin).
pub(super) fn print(board: &HeadsBoard, head: &mut Head, claim: &HeadClaim) {
    let key = claim_key(claim);
    for i in 0..HEADS_CONTRACT_RING {
        if !open(board, head, i) || !admitted(board, i, claim) {
            continue;
        }
        for (j, &tau) in board_instants(board, i).iter().enumerate() {
            let k = i * SLOT_SAMPLES + j;
            if tau < key.ts {
                continue;
            }
            let held = Key {
                ts: head.sample_ts[k],
                value: head.sample_values[k],
                reset: head.sample_resets[k],
            };
            let origin = match j {
                _ if j == SAMPLES - 1 => Some(head.freeze_origin[i]),
                _ if j == SLOT_SAMPLES - 1 => Some(head.standing_origin[i]),
                _ => None,
            };
            let takes = match key.cmp(&held) {
                core::cmp::Ordering::Greater => true,
                core::cmp::Ordering::Equal => origin.is_some_and(|o| claim.origin_id > o),
                core::cmp::Ordering::Less => false,
            };
            if !takes {
                continue;
            }
            set_sample(head, k, key);
            if j == SAMPLES - 1 {
                head.freeze_origin[i] = claim.origin_id;
            }
            if j == SLOT_SAMPLES - 1 {
                head.standing_origin[i] = claim.origin_id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    const DAY: u64 = 86_400;

    /// A board with one open contract in slot 0: freeze window from day 10, freeze day 20,
    /// expiry day 30, cutoff day 31.
    fn board() -> HeadsBoard {
        let mut b = HeadsBoard::default();
        b.contract_periods[0] = Pubkey::new_from_array([7; 32]);
        b.contract_status[0] = RING_ACCUMULATING;
        b.contract_listing_ts[0] = DAY;
        b.contract_freeze_window_ts[0] = 10 * DAY;
        b.contract_freeze_ts[0] = 20 * DAY;
        b.contract_expiry_ts[0] = 30 * DAY;
        b.contract_cutoff_ts[0] = 31 * DAY;
        b
    }

    fn contract(b: &HeadsBoard) -> HeadsContract {
        HeadsContract {
            period: b.contract_periods[0],
            freeze_ts: 20 * DAY,
            expiry_ts: 30 * DAY,
            ..HeadsContract::default()
        }
    }

    fn claim(seq: u32, value: u64, source_ts: u64, reset: u64) -> HeadClaim {
        HeadClaim {
            status: HeadClaimStatus::Revealed,
            seq,
            value,
            source_ts,
            commit_ts: source_ts,
            window_end_ts: source_ts + DAY,
            lta_effective_from_ts: if reset == 0 { 0 } else { source_ts },
            lta_next_reset_ts: reset,
            origin_id: [seq as u8; 32],
            ..HeadClaim::default()
        }
    }

    /// A head on the board's contract, its slot started from a print of `value` on day 5.
    fn head(b: &HeadsBoard, value: u64) -> Head {
        let mut h = Head {
            value,
            value_source_ts: 5 * DAY,
            value_claim_seq: 0,
            origin_id: [0; 32],
            reveals: 1,
            finalized_reveals: 1,
            ..Head::default()
        };
        h.capture_periods[0] = b.contract_periods[0];
        restart(b, &mut h, 0);
        h
    }

    #[test]
    fn instants_end_each_window() {
        let at = instants(10 * DAY, 20 * DAY, 30 * DAY);
        assert_eq!(at[0], 12 * DAY);
        assert_eq!(at[SAMPLES - 1], 20 * DAY);
        assert_eq!(at[SAMPLES], 22 * DAY);
        assert_eq!(at[SLOT_SAMPLES - 1], 30 * DAY);
    }

    /// LTA: a print is the level from its source time until its own reset, after which it is
    /// none; an identical recapture changes nothing (round 3 R3).
    #[test]
    fn an_lta_print_lapses_at_its_reset() {
        let b = board();
        for recapture in [false, true] {
            let mut h = head(&b, 500);
            // In effect from day 23 (captured then), resetting on day 27.
            print(&b, &mut h, &claim(1, 580, 23 * DAY, 27 * DAY));
            if recapture {
                print(&b, &mut h, &claim(2, 580, 24 * DAY, 27 * DAY));
            }
            assert_eq!(&h.sample_values[..6], &[500; 6]);
            // Settlement instants: days 22 (500), 24 (580), 26 (580), 28 and 30 (none).
            let (level, freeze, ..) = levels(&h, &contract(&b), 10 * DAY);
            assert_eq!((level, freeze), (Some(580), Some(500)));
        }
    }

    /// Round 3 R1: the samples are the same in every order the prints come in, an earlier
    /// observation arriving after a later one included; equal source times go by rank.
    #[test]
    fn every_order_of_prints_gives_the_same_samples() {
        let b = board();
        let prints = [
            claim(1, 100, 13 * DAY, 0),
            claim(2, 200, 21 * DAY, 0),
            claim(3, 300, 21 * DAY, 0),
            claim(4, 400, 26 * DAY, 0),
            claim(5, 150, 11 * DAY, 0),
        ];
        let mut first: Option<Head> = None;
        let mut order: Vec<usize> = (0..prints.len()).collect();
        // Every permutation of five (Heap's algorithm, iteratively).
        let mut c = [0usize; 5];
        let mut check = |order: &[usize]| {
            let mut h = head(&b, 50);
            for &n in order {
                print(&b, &mut h, &prints[n]);
            }
            match &first {
                None => first = Some(h),
                Some(f) => assert_eq!(
                    (
                        &f.sample_values,
                        &f.sample_ts,
                        &f.sample_resets,
                        f.standing_origin
                    ),
                    (
                        &h.sample_values,
                        &h.sample_ts,
                        &h.sample_resets,
                        h.standing_origin
                    )
                ),
            }
        };
        check(&order);
        let mut i = 0;
        while i < order.len() {
            if c[i] < i {
                order.swap(if i % 2 == 0 { 0 } else { c[i] }, i);
                check(&order);
                c[i] += 1;
                i = 0;
            } else {
                c[i] = 0;
                i += 1;
            }
        }
        let h = first.unwrap();
        // Day 21's two observations: the one saying the higher value decides.
        let tie = 300;
        // Freeze window 12, 14, 16, 18, 20; settlement 22, 24, 26, 28, 30.
        assert_eq!(
            &h.sample_values[..10],
            &[150, 100, 100, 100, 100, tie, tie, 400, 400, 400]
        );
        assert_eq!(h.standing_origin[0], [4; 32]);
        assert_eq!(h.freeze_origin[0], [1; 32]);
    }

    /// Round 4 N3, round 5 item 1: one second's observations order by what they say, never by
    /// claim or arrival, and a replay (what it says unchanged, a later claim) compares equal to
    /// the original, so it displaces nothing. The values the reviewer found colliding under the
    /// round-4 32-bit rank, 1,000,081 and 1,129,816 (origin [1; 32], no LTA terms), order too.
    #[test]
    fn one_seconds_observations_order_by_what_they_say() {
        let b = board();
        for (low, high) in [(100, 200), (1_000_081, 1_129_816)] {
            for flip in [false, true] {
                let mut h = head(&b, 50);
                let mut first = claim(1, low, 23 * DAY, 0);
                let mut second = claim(2, high, 23 * DAY, 0);
                first.origin_id = [1; 32];
                second.origin_id = [1; 32];
                if flip {
                    core::mem::swap(&mut first, &mut second);
                }
                print(&b, &mut h, &first);
                print(&b, &mut h, &second);
                assert_eq!(h.sample_values[SLOT_SAMPLES - 1], high);
                let before = (h.sample_values, h.sample_ts, h.sample_resets);
                for replayed in [&first, &second] {
                    let mut replay = replayed.clone();
                    replay.seq += 10;
                    print(&b, &mut h, &replay);
                    assert_eq!((h.sample_values, h.sample_ts, h.sample_resets), before);
                }
            }
        }
    }

    /// A claim the contract does not admit (committed at the expiry) prints for nothing in it.
    #[test]
    fn only_admitted_prints_decide_samples() {
        let b = board();
        let mut h = head(&b, 50);
        let mut late = claim(1, 900, 29 * DAY, 0);
        late.commit_ts = 30 * DAY;
        print(&b, &mut h, &late);
        assert_eq!(&h.sample_values[..10], &[50; 10]);
    }

    /// Admission is decided on facts as of the cutoff: committed after the expiry, its window
    /// ending after K, a challenge pending or ruled after K, or a winning challenge: out.
    #[test]
    fn admission_follows_the_cutoff() {
        let b = board();
        let ok = claim(1, 1, 25 * DAY, 0);
        assert!(admitted(&b, 0, &ok));
        let mut c = ok.clone();
        c.commit_ts = 30 * DAY;
        assert!(!admitted(&b, 0, &c), "committed at the expiry");
        let mut c = ok.clone();
        c.window_end_ts = 31 * DAY + 1;
        assert!(!admitted(&b, 0, &c), "window past K");
        let mut c = ok.clone();
        c.challenge_count = 1;
        assert!(!admitted(&b, 0, &c), "unruled");
        c.ruled_count = 1;
        c.last_ruled_ts = 31 * DAY;
        assert!(admitted(&b, 0, &c), "ruled at K");
        c.last_ruled_ts = 31 * DAY + 1;
        assert!(!admitted(&b, 0, &c), "ruled after K");
        c.last_ruled_ts = 26 * DAY;
        c.won_count = 1;
        assert!(!admitted(&b, 0, &c), "fallen");
    }

    /// A slot starting with claims revealed and open waits for every one of them, admitted or
    /// not, until each finalizes; later reveals count while admitted.
    #[test]
    fn a_slot_waits_for_the_claims_open_when_it_started() {
        let b = board();
        let mut h = Head {
            reveals: 3,
            finalized_reveals: 1,
            ..Head::default()
        };
        h.capture_periods[0] = b.contract_periods[0];
        restart(&b, &mut h, 0);
        assert_eq!(h.adm_open[0], 2);
        let mut early = claim(1, 1, 3 * DAY, 0);
        early.reveal_index = 2;
        early.commit_ts = 40 * DAY; // admitted nowhere, still waited for
        let before = admissions(&b, &h, &early);
        assert!(before[0]);
        early.status = HeadClaimStatus::Stood;
        track(&b, &mut h, before, &early).unwrap();
        assert_eq!(h.adm_open[0], 1);
        let mut later = claim(2, 1, 25 * DAY, 0);
        later.reveal_index = 3;
        track(&b, &mut h, [false; 4], &later).unwrap();
        assert_eq!(h.adm_open[0], 2);
        assert!(waits(&h, 0));
    }
}
