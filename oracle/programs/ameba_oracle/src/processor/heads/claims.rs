//! Claims and their challenges: selectors 11 to 17, and a standing claim's prize (24, routed
//! from `rewards`). Accounts: the matching builders in `instruction::heads`.
//!
//! Anyone may fill an Active head or update a Live one; there are no fetcher seats, votes or
//! stakes (the user, 2026-10-29: "agreed just have challenges deal w some of that stuff"). A
//! falling claim costs only its bond; claim challenges never strike the source or slash its
//! delegators (source challenges do, `sources`).
//!
//! A claim is committed with its bond, revealed in a later slot (v1's reveal window), and stays
//! open to bonded challenges until its window ends; a challenge in the last
//! `challenge_extension_secs` pushes the end out so it can be judged. Three current council seats
//! rule each challenge. Challenges per claim are unbounded, so the claim keeps the aggregates
//! (`challenge_bond_total`, `winning_bond_total`, `won_count`, `last_winner`) that reproduce
//! `outcome::resolve_claim`: `FinalizeClaim` resolves the claim once from them, and each
//! `SettleChallenge` pays one challenge its part, the index-last winner after every other so the
//! pro-rata remainder lands exactly where `outcome::pro_rata` puts it.

use super::accounts::{
    add, bad_account, bad_list, bad_state, credit_usdc, debit_usdc, evidence, expect_key,
    load_head_of, overflow, roles, sub, system, Res,
};
use super::transitions as t;
use super::*;
use crate::instruction::heads::{
    heads_claim_commitment, ChallengeClaimParams, CommitClaimParams, RevealClaimParams,
    RuleChallengeParams,
};
use crate::state::heads::{
    find_head_challenge_address, find_head_claim_address, find_heads_epoch_address,
    find_heads_stake_address, load_heads_account, store_heads_account, Head, HeadChallenge,
    HeadChallengeKind, HeadChallengeRuling, HeadClaim, HeadClaimKind, HeadClaimStatus, HeadStatus,
    HeadsBoard, HeadsChannel, HeadsConfig, HeadsEpoch, HeadsStake, DEFAULT_RULING_DEADLINE_SECS,
    HEADS_EPOCH_SEED, HEADS_MAX_LIVE_SOURCES, HEAD_CHALLENGE_SEED, HEAD_CLAIM_SEED,
    HEAD_PRINT_CAPACITY, SOURCE_CASE_FELL, SOURCE_CASE_OPEN, SOURCE_CASE_STOOD,
};
use ameba_oracle_heads::outcome::{self, Challenge};
use ameba_oracle_heads::{rewards, time};

/// A case's ruling deadline, fixed when it opens with its window ending at `window_end`: late
/// challenges extend the window only up to it, no ruling is taken from it, and from it the case
/// resolves on the rulings it has, each unruled challenge timing out alone (its bond back).
pub(super) fn ruling_deadline(board: &HeadsBoard, window_end: u64) -> Res<u64> {
    let secs = match board.ruling_deadline_secs {
        0 => DEFAULT_RULING_DEADLINE_SECS,
        secs => secs,
    };
    add(window_end, secs)
}

/// The least bond a challenge carries: the board's `challenge_bond`, and at least one atom so
/// every winner has a positive share of the pro-rata split (`outcome::pro_rata`).
pub(super) fn challenge_bond_floor(board: &HeadsBoard) -> u64 {
    board.params.challenge_bond.max(1)
}

/// The late-challenge rule: a challenge filed with less than `challenge_extension_secs` left
/// pushes the window's end to `extension` after it, so the council can judge it, but never past
/// the case's ruling `deadline` (early review B6: challenges cannot postpone it).
pub(super) fn extend_window(window_end: u64, now: u64, extension: u64, deadline: u64) -> Res<u64> {
    Ok(window_end.max(add(now, extension)?).min(deadline))
}

/// The challenges of a case as `outcome` takes them, from the aggregates the claim or the head
/// keeps: every loser's bond as one losing entry, every winner's as one winning entry. What
/// `resolve_claim` and `resolve_source` total (whether the case falls, what the claimant or
/// proposer gets, what burns, what the proposer pays) depends only on these sums; each winner's
/// own part is paid by `SettleChallenge` with the same pro-rata rule ([`winner_share`]).
pub(super) fn aggregate_challenges(total: u64, winning: u64, won: u16) -> Res<Vec<Challenge>> {
    let losing = sub(total, winning)?;
    let mut challenges = Vec::with_capacity(2);
    if losing > 0 || won == 0 {
        challenges.push(Challenge {
            bond: losing,
            won: false,
        });
    }
    if won > 0 {
        if winning == 0 {
            return Err(bad_state());
        }
        challenges.push(Challenge {
            bond: winning,
            won: true,
        });
    }
    Ok(challenges)
}

/// One winner's part of `pot` by its bond among `winning` (`outcome::pro_rata`): rounded down,
/// the last winner (by index) taking what the others left.
pub(super) fn winner_share(
    pot: u64,
    bond: u64,
    winning: u64,
    is_last: bool,
    paid_so_far: u64,
) -> Res<u64> {
    if is_last {
        return sub(pot, paid_so_far);
    }
    if winning == 0 {
        return Err(bad_state());
    }
    Ok((u128::from(pot) * u128::from(bond) / u128::from(winning)) as u64)
}

fn load_claim_of(program_id: &Pubkey, info: &AccountInfo, head: &Pubkey) -> Res<Box<HeadClaim>> {
    let claim: Box<HeadClaim> = load_heads_account(program_id, info)?;
    if claim.head != *head {
        return Err(bad_account());
    }
    Ok(claim)
}

/// A standing claim prints (the head synced, the claim still `Revealed`): the open contracts'
/// samples first (`samples::print`), then the ring (for display, in finalization order) and, when
/// it is the head's latest print by source time (then claim seq), its current value and origin:
/// whatever order claims finalize in, the head's value is its latest observation.
fn accept_print(board: &HeadsBoard, head: &mut Head, claim: &HeadClaim) -> Res<()> {
    super::samples::print(board, head, claim);
    let slot = head.print_count as usize % HEAD_PRINT_CAPACITY;
    head.print_values[slot] = claim.value;
    head.print_source_ts[slot] = claim.source_ts;
    head.print_effective_from_ts[slot] = claim.lta_effective_from_ts;
    head.print_next_reset_ts[slot] = claim.lta_next_reset_ts;
    head.print_floor[slot] = claim.lta_floor;
    head.print_ceiling[slot] = claim.lta_ceiling;
    head.print_claim_seq[slot] = claim.seq;
    head.print_count = head.print_count.checked_add(1).ok_or_else(overflow)?;
    if !super::samples::latest(head, claim) {
        return Ok(());
    }
    head.value = claim.value;
    head.value_source_ts = claim.source_ts;
    head.value_claim_seq = claim.seq;
    head.origin_id = claim.origin_id;
    head.lta_effective_from_ts = claim.lta_effective_from_ts;
    head.lta_next_reset_ts = claim.lta_next_reset_ts;
    head.lta_floor = claim.lta_floor;
    head.lta_ceiling = claim.lta_ceiling;
    Ok(())
}

/// The board's prize epoch for `month_id`, created (the payer funds it) when it does not exist.
pub(super) fn load_or_create_epoch<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    epoch_info: &AccountInfo<'a>,
    system_info: &AccountInfo<'a>,
    board: &Pubkey,
    month_id: u32,
    now: u64,
) -> Res<Box<HeadsEpoch>> {
    let (key, bump) = find_heads_epoch_address(program_id, board, month_id);
    expect_key(epoch_info, &key)?;
    if epoch_info.owner == program_id {
        return load_heads_account(program_id, epoch_info);
    }
    system(system_info)?;
    let month_seed = month_id.to_le_bytes();
    create_program_account(
        payer,
        epoch_info,
        system_info,
        program_id,
        HeadsEpoch::LEN,
        &[HEADS_EPOCH_SEED, board.as_ref(), &month_seed, &[bump]],
    )?;
    let next = month_id.checked_add(1).ok_or_else(overflow)?;
    Ok(Box::new(HeadsEpoch {
        discriminator: HeadsEpoch::DISCRIMINATOR,
        version: HeadsEpoch::VERSION,
        bump,
        board: *board,
        month_id,
        start_ts: time::month_start(month_id),
        end_ts: time::month_start(next),
        updated_ts: now,
        ..HeadsEpoch::default()
    }))
}

// Selector 11
// ------------------------------------------------------------------------------------------

/// Selector 11: anyone commits a fill (an Active head with no fill open) or an update (a Live
/// head) with the board's `claim_bond`.
#[inline(never)]
pub(super) fn commit_claim(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CommitClaimParams,
) -> ProgramResult {
    roles(accounts, b"WRrrwwwr")?;
    let [payer_info, fetcher_info, config_info, board_info, head_info, claim_info, collateral_info, system_info] =
        accounts
    else {
        return Err(bad_list());
    };
    system(system_info)?;
    load_active_oracle_vault_config(program_id, config_info)?;
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    if params.expected_seq != head.claim_seq
        || params.bond != board.params.claim_bond
        || crate::bytes32_is_zero(&params.commit_hash)
    {
        return Err(bad_state());
    }
    let open = match params.kind {
        HeadClaimKind::Fill => head.status == HeadStatus::Active && !head.open_fill,
        HeadClaimKind::Update => head.status == HeadStatus::Live,
    };
    if !open {
        return Err(bad_state());
    }
    let now = t::now()?;
    // Capture the claims at any expiry passed since the head's last touch, before this one counts.
    t::sync_head(&board, &mut head, now)?;
    let seq = head.claim_seq;
    let (claim_key, claim_bump) = find_head_claim_address(program_id, head_info.key, seq);
    expect_key(claim_info, &claim_key)?;
    validate_create_only_program_account_target(program_id, claim_info)?;
    let slot = crate::compact_error::slot()?;
    debit_usdc(program_id, collateral_info, fetcher_info.key, params.bond)?;
    let seq_seed = seq.to_le_bytes();
    create_program_account(
        payer_info,
        claim_info,
        system_info,
        program_id,
        HeadClaim::LEN,
        &[
            HEAD_CLAIM_SEED,
            head_info.key.as_ref(),
            &seq_seed,
            &[claim_bump],
        ],
    )?;
    let reveal_deadline_slot = slot
        .checked_add(ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS)
        .and_then(|s| s.checked_add(ORACLE_UPDATE_REVEAL_WINDOW_SLOTS))
        .ok_or_else(overflow)?;
    let claim = HeadClaim {
        discriminator: HeadClaim::DISCRIMINATOR,
        version: HeadClaim::VERSION,
        bump: claim_bump,
        head: *head_info.key,
        board: *board_info.key,
        seq,
        kind: params.kind,
        status: HeadClaimStatus::Committed,
        fetcher: *fetcher_info.key,
        commit_hash: params.commit_hash,
        commit_slot: slot,
        commit_ts: now,
        reveal_deadline_slot,
        bond: params.bond,
        rent_payer: *payer_info.key,
        ..HeadClaim::default()
    };
    head.claim_seq = seq.checked_add(1).ok_or_else(overflow)?;
    head.open_claims = head.open_claims.checked_add(1).ok_or_else(overflow)?;
    if params.kind == HeadClaimKind::Fill {
        head.open_fill = true;
        head.open_fill_seq = seq;
    }
    head.updated_ts = now;
    store_heads_account(claim_info, &claim)?;
    store_heads_account(head_info, head.as_ref())
}

// Selector 12
// ------------------------------------------------------------------------------------------

/// Selector 12: reveal value, source time, origin, LTA terms and evidence in a later slot inside
/// the reveal window; the challenge window starts.
#[inline(never)]
pub(super) fn reveal_claim(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RevealClaimParams,
) -> ProgramResult {
    roles(accounts, b"Rrwwr")?;
    let [fetcher_info, board_info, head_info, claim_info, evidence_info] = accounts else {
        return Err(bad_list());
    };
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
    let now = t::now()?;
    let slot = crate::compact_error::slot()?;
    let earliest = claim
        .commit_slot
        .checked_add(ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS)
        .ok_or_else(overflow)?;
    let head_open = match claim.kind {
        HeadClaimKind::Fill => head.status == HeadStatus::Active,
        HeadClaimKind::Update => head.status == HeadStatus::Live,
    };
    if claim.status != HeadClaimStatus::Committed
        || claim.fetcher != *fetcher_info.key
        || slot < earliest
        || slot > claim.reveal_deadline_slot
        || !head_open
        || params.value == 0
        || params.source_ts == 0
        || params.source_ts > now
        // Observed by the commit: a claim committed before an expiry cannot carry an observation
        // from after it (re-review N3). Any earlier observation reveals in its window, whatever
        // was revealed since: no reveal shuts out another (round 3 R1).
        || params.source_ts > claim.commit_ts
        || (head.channel != HeadsChannel::Lta && !params.lta.is_none())
        // An LTA price counts from its effective date, never ahead of it: its claim is a price in
        // effect at its source time (an announcement is captured again once in effect), so no
        // print waits on a schedule (round 3 R3, R4, R6).
        || (head.channel == HeadsChannel::Lta && !params.lta.in_effect_at(params.source_ts))
    {
        return Err(bad_state());
    }
    if heads_claim_commitment(head_info.key, claim.seq, fetcher_info.key, &params)
        != claim.commit_hash
    {
        return Err(bad_state());
    }
    let (object, capture) = evidence(
        program_id,
        evidence_info,
        params.evidence_kind,
        &params.evidence_commitment,
    )?;
    if capture.is_some_and(|ts| ts != params.source_ts) {
        return Err(OracleError::InvalidHeadsEvidence.into());
    }
    claim.status = HeadClaimStatus::Revealed;
    claim.value = params.value;
    claim.source_ts = params.source_ts;
    claim.origin_id = params.origin_id;
    claim.lta_effective_from_ts = params.lta.effective_from_ts;
    claim.lta_next_reset_ts = params.lta.next_reset_ts;
    claim.lta_floor = params.lta.floor;
    claim.lta_ceiling = params.lta.ceiling;
    claim.evidence_kind = params.evidence_kind;
    claim.evidence_commitment = params.evidence_commitment;
    claim.evidence_object = object;
    claim.revealed_ts = now;
    claim.window_end_ts = add(now, board.params.challenge_window_secs)?;
    // Its place among the head's reveals: a ring slot that starts later waits for it.
    t::sync_head(&board, &mut head, now)?;
    claim.reveal_index = head.reveals;
    head.reveals = head.reveals.checked_add(1).ok_or_else(overflow)?;
    super::samples::track(&board, &mut head, [false; 4], &claim)?;
    head.updated_ts = now;
    store_heads_account(head_info, head.as_ref())?;
    store_heads_account(claim_info, claim.as_ref())
}

// Selector 13
// ------------------------------------------------------------------------------------------

/// Selector 13: a bonded challenge of a revealed claim inside its window; a late one extends the
/// window. It opens no case on the head: a claim challenge never strikes the source.
#[inline(never)]
pub(super) fn challenge_claim(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ChallengeClaimParams,
) -> ProgramResult {
    roles(accounts, b"WRrrwwwwrr")?;
    let [payer_info, challenger_info, config_info, board_info, head_info, claim_info, challenge_info, collateral_info, evidence_info, system_info] =
        accounts
    else {
        return Err(bad_list());
    };
    system(system_info)?;
    load_active_oracle_vault_config(program_id, config_info)?;
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
    let now = t::now()?;
    if claim.status != HeadClaimStatus::Revealed
        || now >= claim.window_end_ts
        || params.bond < challenge_bond_floor(&board)
    {
        return Err(bad_state());
    }
    let (object, capture) = evidence(
        program_id,
        evidence_info,
        params.evidence_kind,
        &params.evidence_commitment,
    )?;
    if params.alternative_value != 0 && capture.is_some_and(|ts| ts != params.alternative_source_ts)
    {
        return Err(OracleError::InvalidHeadsEvidence.into());
    }
    let (challenge_key, challenge_bump) =
        find_head_challenge_address(program_id, claim_info.key, challenger_info.key);
    expect_key(challenge_info, &challenge_key)?;
    validate_create_only_program_account_target(program_id, challenge_info)?;
    debit_usdc(
        program_id,
        collateral_info,
        challenger_info.key,
        params.bond,
    )?;
    create_program_account(
        payer_info,
        challenge_info,
        system_info,
        program_id,
        HeadChallenge::LEN,
        &[
            HEAD_CHALLENGE_SEED,
            claim_info.key.as_ref(),
            challenger_info.key.as_ref(),
            &[challenge_bump],
        ],
    )?;
    let index = claim.challenge_count;
    if index == 0 {
        // The case's deadline, fixed as it opens (early review B6).
        claim.ruling_deadline_ts = ruling_deadline(&board, claim.window_end_ts)?;
    }
    // Unruled, the claim is admitted to no contract; ruled by a contract's cutoff it may be again.
    t::sync_head(&board, &mut head, now)?;
    let before = super::samples::admissions(&board, &head, &claim);
    claim.challenge_count = index.checked_add(1).ok_or_else(overflow)?;
    claim.challenge_bond_total = add(claim.challenge_bond_total, params.bond)?;
    claim.window_end_ts = extend_window(
        claim.window_end_ts,
        now,
        board.params.challenge_extension_secs,
        claim.ruling_deadline_ts,
    )?;
    super::samples::track(&board, &mut head, before, &claim)?;
    head.updated_ts = now;
    store_heads_account(head_info, head.as_ref())?;
    let challenge = HeadChallenge {
        discriminator: HeadChallenge::DISCRIMINATOR,
        version: HeadChallenge::VERSION,
        bump: challenge_bump,
        target: *claim_info.key,
        head: *head_info.key,
        challenger: *challenger_info.key,
        kind: HeadChallengeKind::Claim,
        reason: params.reason,
        index,
        case_number: claim.seq,
        bond: params.bond,
        alternative_value: params.alternative_value,
        alternative_source_ts: params.alternative_source_ts,
        evidence_kind: params.evidence_kind,
        evidence_commitment: params.evidence_commitment,
        evidence_object: object,
        reason_hash: params.reason_hash,
        opened_ts: now,
        rent_payer: *payer_info.key,
        ..HeadChallenge::default()
    };
    store_heads_account(challenge_info, &challenge)?;
    store_heads_account(claim_info, claim.as_ref())
}

// Selector 14
// ------------------------------------------------------------------------------------------

/// Three distinct current seats of the council Config sign (as tag 130/0 checks them), for the
/// epoch the ruling names. Returns the epoch.
fn council_ruling(
    config_info: &AccountInfo,
    seats: [&AccountInfo; 3],
    expected_epoch: u64,
) -> Res<u64> {
    let view = crate::processor::council::council(config_info)?;
    let mut mask = 0u8;
    for seat in seats {
        let index = view
            .seats
            .iter()
            .position(|key| key == seat.key)
            .filter(|_| seat.is_signer)
            .ok_or(OracleError::HeadsCouncilRequired)?;
        mask |= 1 << index;
    }
    if mask.count_ones() != 3 || view.epoch != expected_epoch {
        return Err(OracleError::HeadsCouncilRequired.into());
    }
    Ok(view.epoch)
}

/// Selector 14: three current council seats rule one pending challenge, of an open claim or of
/// the head's open source case, before the case's ruling deadline (early review B5: from it the
/// case resolves on the rulings it has, so a late ruling cannot race that). The ruling is final.
#[inline(never)]
pub(super) fn rule_challenge(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RuleChallengeParams,
) -> ProgramResult {
    roles(accounts, b"rRRRwwrr")?;
    let [config_info, first, second, third, head_info, challenge_info, claim_info, board_info] =
        accounts
    else {
        return Err(bad_list());
    };
    let epoch = council_ruling(config_info, [first, second, third], params.expected_epoch)?;
    let mut head: Box<Head> = load_heads_account(program_id, head_info)?;
    let mut challenge: Box<HeadChallenge> = load_heads_account(program_id, challenge_info)?;
    if challenge.head != *head_info.key
        || challenge.kind != params.kind
        || challenge.ruling != HeadChallengeRuling::Pending
    {
        return Err(bad_state());
    }
    let won = params.ruling == HeadChallengeRuling::Won;
    let now = t::now()?;
    match params.kind {
        HeadChallengeKind::Claim => {
            if !claim_info.is_writable {
                return Err(bad_list());
            }
            let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
            if challenge.target != *claim_info.key
                || claim.status != HeadClaimStatus::Revealed
                || now >= claim.ruling_deadline_ts
            {
                return Err(bad_state());
            }
            // The last ruling's time decides which contracts may admit the claim.
            let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
            if claim.board != *board_info.key {
                return Err(bad_account());
            }
            t::sync_head(&board, &mut head, now)?;
            let before = super::samples::admissions(&board, &head, &claim);
            claim.ruled_count = claim.ruled_count.checked_add(1).ok_or_else(overflow)?;
            claim.ruled_bond_total = add(claim.ruled_bond_total, challenge.bond)?;
            claim.last_ruled_ts = now;
            if won {
                claim.last_winner = if claim.won_count == 0 {
                    challenge.index
                } else {
                    claim.last_winner.max(challenge.index)
                };
                claim.won_count = claim.won_count.checked_add(1).ok_or_else(overflow)?;
                claim.winning_bond_total = add(claim.winning_bond_total, challenge.bond)?;
            }
            super::samples::track(&board, &mut head, before, &claim)?;
            head.updated_ts = now;
            store_heads_account(head_info, head.as_ref())?;
            store_heads_account(claim_info, claim.as_ref())?;
        }
        HeadChallengeKind::Source => {
            system(claim_info)?;
            if challenge.target != *head_info.key
                || head.source_case_status != SOURCE_CASE_OPEN
                || challenge.case_number != head.source_case
                || now >= head.source_case_deadline_ts
            {
                return Err(bad_state());
            }
            head.source_case_ruled = head.source_case_ruled.checked_add(1).ok_or_else(overflow)?;
            head.source_case_ruled_bonds = add(head.source_case_ruled_bonds, challenge.bond)?;
            if won {
                head.source_case_last_winner = if head.source_case_won == 0 {
                    challenge.index
                } else {
                    head.source_case_last_winner.max(challenge.index)
                };
                head.source_case_won = head.source_case_won.checked_add(1).ok_or_else(overflow)?;
                head.source_case_winning_bonds =
                    add(head.source_case_winning_bonds, challenge.bond)?;
            }
            head.updated_ts = now;
            store_heads_account(head_info, head.as_ref())?;
        }
    }
    challenge.ruling = params.ruling;
    challenge.ruled_ts = now;
    challenge.ruled_epoch = epoch;
    store_heads_account(challenge_info, challenge.as_ref())
}

// Selector 15
// ------------------------------------------------------------------------------------------

/// Selector 15, in any order (the samples take each print at its own source time, round 3 R1):
/// once the window ended with every challenge ruled, or at the claim's ruling deadline, `outcome::resolve_claim` runs once from the claim's ruled
/// challenges (an unruled one timed out: its bond goes back at `SettleChallenge`, early review B2).
/// A standing claim pays the fetcher its bond and every losing challenger's, earns its prize
/// points in the month's epoch, and makes the head Live (a fill, while its delegation still meets
/// the activation floor, `floor_activate` and `min_delegators`, and the live list has room; else
/// it stands for its bonds and the head stays Active) or prints (an update); one with
/// a challenge timed out stands for its bonds alone (nothing prints, no prize). A falling claim's
/// bond goes to the winners (`SettleChallenge`) and the losers' bonds burn; nothing else happens
/// to the head or its delegators.
#[inline(never)]
pub(super) fn finalize_claim(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    roles(accounts, b"Wrwwwwwr")?;
    let [payer_info, config_info, board_info, head_info, claim_info, collateral_info, epoch_info, system_info] =
        accounts
    else {
        return Err(bad_list());
    };
    system(system_info)?;
    load_current_canonical_vault_config(program_id, config_info)?;
    let mut board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
    let now = t::now()?;
    if claim.status != HeadClaimStatus::Revealed || now < claim.window_end_ts {
        return Err(bad_state());
    }
    // Every challenge ruled, or the deadline reached (the unruled ones time out alone).
    let ruled = claim.ruled_count == claim.challenge_count;
    if !ruled && now < claim.ruling_deadline_ts {
        return Err(bad_state());
    }
    t::sync_head(&board, &mut head, now)?;
    let before = super::samples::admissions(&board, &head, &claim);
    head.finalized_reveals = head.finalized_reveals.checked_add(1).ok_or_else(overflow)?;
    let params = board.params.to_params();
    let resolution = outcome::resolve_claim(
        claim.bond,
        &aggregate_challenges(
            claim.ruled_bond_total,
            claim.winning_bond_total,
            claim.won_count,
        )?,
    );
    if !resolution.falls {
        credit_usdc(
            program_id,
            collateral_info,
            &claim.fetcher,
            resolution.to_claimant,
        )?;
        claim.to_claimant = resolution.to_claimant;
        // A full live-source list cannot take the head: the fill stands for its bonds and the
        // head stays Active for a later one (nothing waits on room).
        let room = usize::from(board.live_count) < HEADS_MAX_LIVE_SOURCES;
        // Still backed as an activation needs, at its static floor: delegation withdrawn since
        // the head activated cannot leave a Live source behind it (an origin toward coverage with
        // no weight). The fill stands for its bonds and the head stays Active, as on a full board.
        let backed = crate::state::heads::heads_backed(
            head.delegation,
            head.delegators,
            params.floor_activate,
            params.min_delegators,
        );
        // One update prize per head per UTC day of source time (re-review limit (a)): repeat
        // captures of a page earn nothing more, and the samples do not count them.
        let day = (claim.source_ts / 86_400) as u32;
        let prize = match (claim.kind, head.status) {
            // A challenge timed out unruled: the claim stands for its bonds, never vetted.
            _ if !ruled => 0,
            (HeadClaimKind::Fill, HeadStatus::Active) if room && backed => {
                accept_print(&board, &mut head, &claim)?;
                head.fill_value = claim.value;
                head.fill_source_ts = claim.source_ts;
                // Prizes are for days from the fill's and the posting's on (round 6 review,
                // finding 5; round 7 review, B3): an observation from before either, however late
                // it is captured, earns nothing.
                head.last_prize_day = head.last_prize_day.max(head.prize_floor());
                t::make_head_live(&mut board, &mut head, now)?;
                rewards::fill_prize(&params, head.delegation)
            }
            (HeadClaimKind::Update, HeadStatus::Live) => {
                accept_print(&board, &mut head, &claim)?;
                t::update_live_print(&mut board, &mut head, now)?;
                // A replay of the fill's observation earns nothing; another one of its day does
                // (round 4 N7, round 5 item 5). Off the list (evicted), it prints for its
                // contracts and earns nothing.
                let no_prize = head.evicted_ts != 0
                    || (claim.source_ts == head.fill_source_ts && claim.value == head.fill_value);
                if day > head.last_prize_day && !no_prize {
                    head.last_prize_day = day;
                    rewards::update_prize(&params, head.delegation)
                } else {
                    0
                }
            }
            // A source retired after a contract's cutoff stays eligible there: its update still
            // prints for it (re-review N4), with no prize.
            (HeadClaimKind::Update, HeadStatus::Retired) => {
                accept_print(&board, &mut head, &claim)?;
                0
            }
            // The head retired or expired while the claim was open, or no live source fits:
            // its bonds settle, but it prints nothing.
            _ => 0,
        };
        claim.status = HeadClaimStatus::Stood;
        claim.epoch = time::month_id(now);
        if prize > 0 {
            claim.prize_points = prize;
            let mut epoch = load_or_create_epoch(
                program_id,
                payer_info,
                epoch_info,
                system_info,
                board_info.key,
                claim.epoch,
                now,
            )?;
            epoch.prize_points = add(epoch.prize_points, prize)?;
            epoch.claims = epoch.claims.checked_add(1).ok_or_else(overflow)?;
            epoch.updated_ts = now;
            store_heads_account(epoch_info, epoch.as_ref())?;
        }
    } else {
        // The collateral slot is still the fetcher's, though it receives nothing.
        load_canonical_user_collateral(program_id, collateral_info, &claim.fetcher)?;
        claim.status = HeadClaimStatus::Fell;
        claim.bonds_burned = resolution.burned;
        board.pending_usdc_burn = add(board.pending_usdc_burn, resolution.burned)?;
    }
    super::samples::track(&board, &mut head, before, &claim)?;
    t::close_claim(&mut head, &claim)?;
    claim.finalized_ts = now;
    head.updated_ts = now;
    board.updated_ts = now;
    store_heads_account(claim_info, claim.as_ref())?;
    store_heads_account(head_info, head.as_ref())?;
    store_heads_account(board_info, board.as_ref())
}

// Selector 16
// ------------------------------------------------------------------------------------------

/// Selector 16: pays one resolved challenge and closes it (its rent to the challenger). A claim
/// challenge: a winner of a fallen claim gets its bond back and its pro-rata part of the claim
/// bond; everyone else nothing (a standing claim's challengers already paid the fetcher; a fallen
/// claim's losers burned). A source challenge: a winner of a fallen case gets its bond back in
/// USDC and its pro-rata part of the proposer's slashed delegation as free AMBA stake.
#[inline(never)]
pub(super) fn settle_challenge(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    // The claim's slot is the read-only system program for a source challenge.
    roles(accounts, b"rwwwrwww")?;
    let [config_info, heads_config_info, head_info, challenge_info, claim_info, collateral_info, stake_info, rent_payer_info] =
        accounts
    else {
        return Err(bad_list());
    };
    load_current_canonical_vault_config(program_id, config_info)?;
    let mut heads_config: Box<HeadsConfig> = load_heads_account(program_id, heads_config_info)?;
    let mut head: Box<Head> = load_heads_account(program_id, head_info)?;
    let challenge: Box<HeadChallenge> = load_heads_account(program_id, challenge_info)?;
    if challenge.head != *head_info.key || challenge.settled {
        return Err(bad_state());
    }
    expect_key(
        stake_info,
        &find_heads_stake_address(program_id, &challenge.challenger).0,
    )?;
    let now = t::now()?;
    let won = challenge.ruling == HeadChallengeRuling::Won;
    let mut usdc = 0u64;
    match challenge.kind {
        HeadChallengeKind::Claim => {
            if !claim_info.is_writable {
                return Err(bad_list());
            }
            let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
            if challenge.target != *claim_info.key
                || !matches!(claim.status, HeadClaimStatus::Stood | HeadClaimStatus::Fell)
            {
                return Err(bad_state());
            }
            if challenge.ruling == HeadChallengeRuling::Pending {
                // Timed out unruled at the claim's deadline: its bond back.
                usdc = challenge.bond;
            }
            if claim.status == HeadClaimStatus::Fell && won {
                let is_last = challenge.index == claim.last_winner;
                if is_last && claim.settled_winners.checked_add(1) != Some(claim.won_count) {
                    // The remainder goes to the last winner after every other is paid.
                    return Err(bad_state());
                }
                let share = winner_share(
                    claim.bond,
                    challenge.bond,
                    claim.winning_bond_total,
                    is_last,
                    claim.shares_paid,
                )?;
                claim.shares_paid = add(claim.shares_paid, share)?;
                if claim.shares_paid > claim.bond {
                    return Err(bad_state());
                }
                claim.settled_winners =
                    claim.settled_winners.checked_add(1).ok_or_else(overflow)?;
                usdc = add(challenge.bond, share)?;
            }
            claim.settled_count = claim.settled_count.checked_add(1).ok_or_else(overflow)?;
            store_heads_account(claim_info, claim.as_ref())?;
        }
        HeadChallengeKind::Source => {
            system(claim_info)?;
            let current = challenge.case_number == head.source_case;
            if challenge.target != *head_info.key
                || challenge.case_number > head.source_case
                || (current
                    && !matches!(
                        head.source_case_status,
                        SOURCE_CASE_STOOD | SOURCE_CASE_FELL
                    ))
                // An earlier case's winners were all paid before the next case opened.
                || (!current && won)
            {
                return Err(bad_state());
            }
            if challenge.ruling == HeadChallengeRuling::Pending {
                // Timed out unruled at its case's deadline: its bond back.
                usdc = challenge.bond;
            }
            if current && won && head.source_case_status == SOURCE_CASE_FELL {
                let is_last = challenge.index == head.source_case_last_winner;
                if is_last
                    && head.source_case_settled_winners.checked_add(1) != Some(head.source_case_won)
                {
                    return Err(bad_state());
                }
                let amba = winner_share(
                    head.source_case_proposer_slashed,
                    challenge.bond,
                    head.source_case_winning_bonds,
                    is_last,
                    head.source_case_amba_paid,
                )?;
                head.source_case_amba_paid = add(head.source_case_amba_paid, amba)?;
                if head.source_case_amba_paid > head.source_case_proposer_slashed {
                    return Err(bad_state());
                }
                head.source_case_settled_winners = head
                    .source_case_settled_winners
                    .checked_add(1)
                    .ok_or_else(overflow)?;
                usdc = challenge.bond;
                if amba > 0 {
                    let mut stake: Box<HeadsStake> = load_heads_account(program_id, stake_info)?;
                    if stake.owner != challenge.challenger {
                        return Err(bad_account());
                    }
                    stake.free = add(stake.free, amba)?;
                    stake.awarded_total = add(stake.awarded_total, amba)?;
                    stake.updated_ts = now;
                    store_heads_account(stake_info, stake.as_ref())?;
                }
                head.updated_ts = now;
                store_heads_account(head_info, head.as_ref())?;
            }
        }
    }
    credit_usdc(program_id, collateral_info, &challenge.challenger, usdc)?;
    heads_config.updated_ts = now;
    store_heads_account(heads_config_info, heads_config.as_ref())?;
    // The rent goes back to whoever paid it (an account from before the field: its challenger).
    let rent_payer = if crate::pubkey_is_default(&challenge.rent_payer) {
        challenge.challenger
    } else {
        challenge.rent_payer
    };
    expect_key(rent_payer_info, &rent_payer)?;
    close_program_account(program_id, challenge_info, rent_payer_info)
}

// Selector 17
// ------------------------------------------------------------------------------------------

/// Selector 17: a commit nobody revealed by its deadline gets its bond back; the head's next fill
/// may proceed.
#[inline(never)]
pub(super) fn refund_expired_commit(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    roles(accounts, b"rrwww")?;
    let [config_info, board_info, head_info, claim_info, collateral_info] = accounts else {
        return Err(bad_list());
    };
    load_current_canonical_vault_config(program_id, config_info)?;
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
    let slot = crate::compact_error::slot()?;
    if claim.status != HeadClaimStatus::Committed || slot <= claim.reveal_deadline_slot {
        return Err(bad_state());
    }
    let now = t::now()?;
    t::sync_head(&board, &mut head, now)?;
    credit_usdc(program_id, collateral_info, &claim.fetcher, claim.bond)?;
    claim.status = HeadClaimStatus::Expired;
    claim.finalized_ts = now;
    t::close_claim(&mut head, &claim)?;
    head.updated_ts = now;
    store_heads_account(claim_info, claim.as_ref())?;
    store_heads_account(head_info, head.as_ref())
}

// Selector 24
// ------------------------------------------------------------------------------------------

/// Selector 24: a standing claim's prize once its month ended, `rewards::prize_payout` of the
/// epoch's pool (in full while the pool covers every prize, else pro rata).
#[inline(never)]
pub(super) fn claim_prize(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    roles(accounts, b"Rrwrwww")?;
    let [fetcher_info, config_info, board_info, head_info, claim_info, epoch_info, collateral_info] =
        accounts
    else {
        return Err(bad_list());
    };
    load_current_canonical_vault_config(program_id, config_info)?;
    let mut board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    load_head_of(program_id, head_info, board_info.key)?;
    let mut claim = load_claim_of(program_id, claim_info, head_info.key)?;
    let mut epoch: Box<HeadsEpoch> = load_heads_account(program_id, epoch_info)?;
    let now = t::now()?;
    if claim.fetcher != *fetcher_info.key
        || claim.status != HeadClaimStatus::Stood
        || claim.prize_claimed
        || claim.prize_points == 0
        || epoch.board != *board_info.key
        || epoch.month_id != claim.epoch
        || now < epoch.end_ts
    {
        return Err(bad_state());
    }
    let payout = rewards::prize_payout(claim.prize_points, epoch.prize_points, epoch.prize_pool);
    epoch.prize_paid = add(epoch.prize_paid, payout)?;
    if epoch.prize_paid > epoch.prize_pool {
        return Err(bad_state());
    }
    epoch.claims_paid = epoch.claims_paid.checked_add(1).ok_or_else(overflow)?;
    epoch.updated_ts = now;
    board.prize_paid_total = add(board.prize_paid_total, payout)?;
    board.updated_ts = now;
    claim.prize_paid = payout;
    claim.prize_claimed = true;
    credit_usdc(program_id, collateral_info, fetcher_info.key, payout)?;
    store_heads_account(claim_info, claim.as_ref())?;
    store_heads_account(epoch_info, epoch.as_ref())?;
    store_heads_account(board_info, board.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The aggregates reproduce `resolve_claim` on the full list, and paying every winner its
    /// `winner_share` (the last one last) gives exactly what `pro_rata` hands out.
    #[test]
    fn aggregates_and_shares_reproduce_the_outcome() {
        let cases: &[(u64, &[(u64, bool)])] = &[
            (1_000_000, &[]),
            (1_000_000, &[(5, false), (7, false)]),
            (1_000_001, &[(3, true), (5, false), (7, true), (11, true)]),
            (999, &[(1, true), (1, true), (1, true)]),
            (10, &[(4, false), (9, true)]),
        ];
        for &(bond, list) in cases {
            let full: Vec<Challenge> = list
                .iter()
                .map(|&(bond, won)| Challenge { bond, won })
                .collect();
            let expected = outcome::resolve_claim(bond, &full);
            let total: u64 = list.iter().map(|c| c.0).sum();
            let winning: u64 = list.iter().filter(|c| c.1).map(|c| c.0).sum();
            let won = list.iter().filter(|c| c.1).count() as u16;
            let got =
                outcome::resolve_claim(bond, &aggregate_challenges(total, winning, won).unwrap());
            assert_eq!(
                (got.falls, got.to_claimant, got.burned),
                (expected.falls, expected.to_claimant, expected.burned)
            );
            if !expected.falls {
                continue;
            }
            let last = list.iter().rposition(|c| c.1).unwrap();
            let mut paid = 0;
            let mut parts = vec![0u64; list.len()];
            for (i, &(b, won)) in list.iter().enumerate() {
                if won && i != last {
                    let share = winner_share(bond, b, winning, false, paid).unwrap();
                    paid += share;
                    parts[i] = b + share;
                }
            }
            parts[last] =
                list[last].0 + winner_share(bond, list[last].0, winning, true, paid).unwrap();
            assert_eq!(parts, expected.to_challengers);
        }
    }

    /// The same for a source case: AMBA parts of the proposer's slash.
    #[test]
    fn source_aggregates_reproduce_resolve_source() {
        let list = [(2u64, true), (3, false), (5, true)];
        let full: Vec<Challenge> = list
            .iter()
            .map(|&(bond, won)| Challenge { bond, won })
            .collect();
        let expected = outcome::resolve_source(&full, 1_000_003, 10_000);
        let got =
            outcome::resolve_source(&aggregate_challenges(10, 7, 2).unwrap(), 1_000_003, 10_000);
        assert_eq!(
            (got.falls, got.proposer_slashed, got.to_proposer, got.burned),
            (
                expected.falls,
                expected.proposer_slashed,
                expected.to_proposer,
                expected.burned
            )
        );
        let first = winner_share(got.proposer_slashed, 2, 7, false, 0).unwrap();
        let last = winner_share(got.proposer_slashed, 5, 7, true, first).unwrap();
        assert_eq!(vec![first, 0, last], expected.amba_to_challengers);
    }

    #[test]
    fn late_challenges_extend_the_window_to_the_extension() {
        assert_eq!(extend_window(1_000, 100, 50, 5_000).unwrap(), 1_000);
        assert_eq!(extend_window(1_000, 990, 50, 5_000).unwrap(), 1_040);
        // Never past the case's deadline (early review B6).
        assert_eq!(extend_window(4_990, 4_980, 50, 5_000).unwrap(), 5_000);
    }
}
