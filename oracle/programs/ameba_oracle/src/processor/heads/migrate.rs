//! Selector 30, permissionless, dev builds only (`heads-migrate`; `localnet-dev-authority` has it
//! too): takes dev's heads accounts of an older layout to the current one, in place, so dev's
//! heads, contracts and markets carry on. Mainnet never had a heads account, so the production
//! artifact has no such path.
//!
//! Two kinds remain (round 8, after dev ran the round-7 migration):
//!
//! - A **version-2 Head of the round-6 program** carrying a renewal join or a left-out mark, or a
//!   prize day before its floor ([`retire_renewal`]). Accounts: 0 payer; 1 board; 2 the head;
//!   3 system program; `contracts` 0.
//! - A **board of version 1 or 2** ([`upgrade_board`]): its live entries' delegators and
//!   contract memberships. Accounts: 0 payer; 1 board; 2 the board again; 3 system program; then
//!   every live head of the board, then `contracts` ring contracts (every unsettled one).
//!
//! The older paths (the 2,688-byte Head grow, the version-1 Head sample rebuild and the version-1
//! and version-2 Delegation grow) are gone: dev has no account of those layouts any more, and
//! such an account is refused (`InvalidHeadsAccount`).

use super::accounts::{bad_account, bad_list, load_head_of, roles, system};
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::state::heads::{
    decode_head_fields, load_heads_account, store_heads_account, Head, HeadsBoard, HeadsContract,
    CAPTURE_JOINED, CAPTURE_RETIRED_EXCLUDED, HEADS_CONTRACT_RING, HEADS_MAX_LIVE_SOURCES,
    LIVE_MEMBER, RING_ACCUMULATING, RING_COLLECTING,
};

fn open_slot(board: &HeadsBoard, i: usize) -> bool {
    matches!(
        board.contract_status[i],
        RING_ACCUMULATING | RING_COLLECTING
    )
}

#[inline(never)]
pub(super) fn migrate_head(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    contracts: u8,
) -> ProgramResult {
    if accounts.len() < 4 {
        return Err(bad_list());
    }
    let (fixed, rest) = accounts.split_at(4);
    roles(fixed, b"Wwwr")?;
    let [_payer, board_info, head_info, system_info] = fixed else {
        return Err(bad_list());
    };
    system(system_info)?;
    // Only an account of this program in a layout this path takes; the full checks follow once
    // it is the current one.
    if head_info.owner != program_id || head_info.executable {
        return Err(bad_account());
    }
    let (kind, version) = {
        let data = head_info.try_data()?;
        let kind: [u8; 8] = data
            .get(..8)
            .and_then(|d| d.try_into().ok())
            .ok_or_else(bad_account)?;
        (kind, data.get(8).copied())
    };
    let len = head_info.data_len();
    if kind == HeadsBoard::DISCRIMINATOR && matches!(version, Some(1 | 2)) {
        // The board itself in the head's place (`instruction::heads::migrate_board`).
        if head_info.key != board_info.key || rest.len() < usize::from(contracts) {
            return Err(bad_list());
        }
        let (heads, contract_infos) = rest.split_at(rest.len() - usize::from(contracts));
        return upgrade_board(program_id, board_info, heads, contract_infos);
    }
    if kind == Head::DISCRIMINATOR && len == Head::LEN && version == Some(Head::VERSION) {
        return retire_renewal(program_id, board_info, head_info, contracts, rest);
    }
    Err(bad_account())
}

/// A head of the round-6 program (version 2, before the round-6 review): its renewed LTA opening
/// (`join_opening`, the 32 bytes past the current wire) and its left-out marks
/// (`CAPTURE_RETIRED_EXCLUDED`) go, since renewal joins are gone (round 6 review, finding 2).
/// A slot it joined on a renewal stays a member, counted in `contract_joined`, and opens at its
/// fill's value as every other joiner does. Its prize day starts at the day before its fill's
/// (round 6 review, finding 5), as a fill now sets it. Nothing else changes; a head with none of
/// these is refused, as a migrated account is.
fn retire_renewal(
    program_id: &Pubkey,
    board_info: &AccountInfo,
    head_info: &AccountInfo,
    contracts: u8,
    rest: &[AccountInfo],
) -> ProgramResult {
    let renewed = head_info.try_data()?[Head::WIRE_LEN..]
        .iter()
        .any(|b| *b != 0);
    head_info.try_data_mut()?[Head::WIRE_LEN..].fill(0);
    let mut head = decode_head_fields(&head_info.try_data()?)?;
    let marked = head
        .capture_flags
        .iter()
        .any(|f| f & CAPTURE_RETIRED_EXCLUDED != 0);
    let floor = head.prize_floor();
    if !renewed && !marked && head.last_prize_day >= floor {
        return Err(bad_account());
    }
    if head.board != *board_info.key || contracts != 0 || !rest.is_empty() {
        return Err(bad_list());
    }
    for flags in &mut head.capture_flags {
        *flags &= !CAPTURE_RETIRED_EXCLUDED;
    }
    head.last_prize_day = head.last_prize_day.max(floor);
    store_heads_account(head_info, head.as_ref())?;
    load_head_of(program_id, head_info, board_info.key)?;
    Ok(())
}

/// A board of version 1 (before the round-6 review) or 2 (the round-7 program): its live list
/// takes each entry's delegators (`live_delegators`), read from its heads (every live one,
/// passed in any order), and each entry's membership of the unsettled ring contracts (every one
/// passed after the heads): listed by it, or joined (round 7 review, B1). Then the freeze coverage of a frozen
/// contract not captured yet is taken from its members, and each component's origins count only
/// the entries still backed (finding 1). Its heads must load (a round-6 head with a renewal join
/// is migrated first).
fn upgrade_board(
    program_id: &Pubkey,
    board_info: &AccountInfo,
    heads: &[AccountInfo],
    contract_infos: &[AccountInfo],
) -> ProgramResult {
    if board_info.owner != program_id {
        return Err(bad_account());
    }
    *board_info
        .try_data_mut()?
        .get_mut(8)
        .ok_or_else(bad_account)? = HeadsBoard::VERSION;
    let mut board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    if heads.len() != usize::from(board.live_count) {
        return Err(bad_list());
    }
    let mut seen = [false; HEADS_MAX_LIVE_SOURCES];
    // Per entry, the ring slots whose current contract it joined.
    let mut joined = [0u8; HEADS_MAX_LIVE_SOURCES];
    for info in heads {
        let head = load_head_of(program_id, info, board_info.key)?;
        let entry = super::transitions::live_entry(&board, head.head_id).ok_or_else(bad_list)?;
        if seen[entry] {
            return Err(bad_list());
        }
        seen[entry] = true;
        board.live_delegators[entry] = u8::try_from(head.delegators).unwrap_or(u8::MAX);
        for i in 0..HEADS_CONTRACT_RING {
            if head.capture_flags[i] & CAPTURE_JOINED != 0
                && head.capture_periods[i] == board.contract_periods[i]
            {
                joined[entry] |= 1 << i;
            }
        }
    }
    let mut slots = [false; HEADS_CONTRACT_RING];
    for info in contract_infos {
        let k: Box<HeadsContract> = load_heads_account(program_id, info)?;
        let slot = usize::from(k.ring_slot);
        if k.board != *board_info.key || board.contract_periods[slot] != k.period || slots[slot] {
            return Err(bad_account());
        }
        slots[slot] = true;
        let listed = &k.listed_head_ids[..usize::from(k.listed_count)];
        for e in 0..usize::from(board.live_count) {
            if listed.contains(&board.live_head_ids[e]) || joined[e] & (1 << slot) != 0 {
                board.live_flags[e] |= LIVE_MEMBER << slot;
            }
        }
    }
    if (0..HEADS_CONTRACT_RING).any(|i| open_slot(&board, i) && !slots[i]) {
        return Err(bad_list());
    }
    let now = super::transitions::now()?;
    super::transitions::capture_freeze_origins(&mut board, now);
    for component in 0..board.component_count {
        super::transitions::recompute_duplicates(&mut board, component, now)?;
    }
    store_heads_account(board_info, board.as_ref())
}
