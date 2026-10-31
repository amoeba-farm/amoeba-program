//! Source challenges: selectors 22 and 23. Accounts: the matching builders in
//! `instruction::heads`.
//!
//! A source challenge disputes the source itself (it does not exist, cannot be filled, or
//! publishes fabricated numbers). The head holds one case at a time: the first challenge opens it
//! and its window, later ones join it (a late one extends the window, never past the case's ruling
//! deadline, fixed as it opens), and the case resolves once (`outcome::resolve_source`) when the
//! window ended with every challenge ruled, or at the deadline on the rulings it has (each
//! unruled challenge timing out alone, its bond back: early review B2, B5, B6). A falling case
//! takes `proposer_slash_bps` of the proposer's delegation for the winners (paid one by one by
//! `SettleChallenge`), burns the losers' bonds and strikes the source, whoever fetched; a standing
//! one pays every bond to the proposer. A new case opens only once every winner of the last is
//! paid, so the head's aggregates always describe one case.

use super::accounts::{
    absent, add, bad_list, bad_state, credit_usdc, debit_usdc, evidence, expect_key, load_head_of,
    overflow, roles, stake_or_create, system,
};
use super::claims::{aggregate_challenges, challenge_bond_floor, extend_window};
use super::transitions as t;
use super::*;
use crate::instruction::heads::ChallengeSourceParams;
use crate::state::heads::{
    find_head_challenge_address, find_head_delegation_address, load_heads_account,
    store_heads_account, Delegation, HeadChallenge, HeadChallengeKind, HeadStatus, HeadsBoard,
    HeadsConfig, HEADS_CONTRACT_RING, HEAD_CHALLENGE_SEED, SOURCE_CASE_FELL, SOURCE_CASE_NONE,
    SOURCE_CASE_OPEN, SOURCE_CASE_STOOD,
};
use ameba_oracle_heads::outcome;

/// Selector 18: a bonded challenge of an Active or Live source. It opens the head's next case
/// when none is open (the last one's winners all paid), or joins the open one inside its window.
#[inline(never)]
pub(super) fn challenge_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ChallengeSourceParams,
) -> ProgramResult {
    roles(accounts, b"WRrrwwwrrw")?;
    let [payer_info, challenger_info, config_info, board_info, head_info, challenge_info, collateral_info, evidence_info, system_info, stake_info] =
        accounts
    else {
        return Err(bad_list());
    };
    system(system_info)?;
    // A winner is paid AMBA into its stake: it exists before the case can need it.
    stake_or_create(
        program_id,
        payer_info,
        stake_info,
        system_info,
        challenger_info.key,
        t::now()?,
    )?;
    load_active_oracle_vault_config(program_id, config_info)?;
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    let now = t::now()?;
    if !matches!(head.status, HeadStatus::Active | HeadStatus::Live)
        || params.bond < challenge_bond_floor(&board)
    {
        return Err(bad_state());
    }
    let (object, _) = evidence(
        program_id,
        evidence_info,
        params.evidence_kind,
        &params.evidence_commitment,
    )?;
    let (challenge_key, challenge_bump) =
        find_head_challenge_address(program_id, head_info.key, challenger_info.key);
    expect_key(challenge_info, &challenge_key)?;
    validate_create_only_program_account_target(program_id, challenge_info)?;
    match head.source_case_status {
        SOURCE_CASE_OPEN => {
            if now >= head.source_case_window_end_ts {
                return Err(bad_state());
            }
            head.source_case_window_end_ts = extend_window(
                head.source_case_window_end_ts,
                now,
                board.params.challenge_extension_secs,
                head.source_case_deadline_ts,
            )?;
        }
        SOURCE_CASE_NONE | SOURCE_CASE_STOOD | SOURCE_CASE_FELL => {
            if head.source_case_status == SOURCE_CASE_FELL
                && head.source_case_settled_winners < head.source_case_won
            {
                return Err(bad_state());
            }
            head.source_case = head.source_case.checked_add(1).ok_or_else(overflow)?;
            head.source_case_status = SOURCE_CASE_OPEN;
            head.source_case_opened_ts = t::open_case(&mut head, now)?;
            head.source_case_window_end_ts = add(now, board.params.challenge_window_secs)?;
            head.source_case_deadline_ts =
                super::claims::ruling_deadline(&board, head.source_case_window_end_ts)?;
            head.source_case_ruled_bonds = 0;
            head.source_case_challenges = 0;
            head.source_case_ruled = 0;
            head.source_case_won = 0;
            head.source_case_last_winner = 0;
            head.source_case_settled_winners = 0;
            head.source_case_bond_total = 0;
            head.source_case_winning_bonds = 0;
            head.source_case_proposer_slashed = 0;
            head.source_case_amba_paid = 0;
            // The unsettled contracts the head is a member of now, by generation: escrow kept for
            // one answers for it. Synced first, so its memberships are the board's (round 14
            // review, N1: a contract it left answers for no case opened after, whenever its
            // delegations sync).
            t::sync_head(&board, &mut head, now)?;
            for i in 0..HEADS_CONTRACT_RING {
                let gen = board.contract_gens[i];
                head.source_case_open_gens[i] = if board.weighs_in(&head, i, gen) {
                    gen
                } else {
                    0
                };
            }
        }
        _ => return Err(bad_state()),
    }
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
            head_info.key.as_ref(),
            challenger_info.key.as_ref(),
            &[challenge_bump],
        ],
    )?;
    let index = head.source_case_challenges;
    head.source_case_challenges = index.checked_add(1).ok_or_else(overflow)?;
    head.source_case_bond_total = add(head.source_case_bond_total, params.bond)?;
    head.updated_ts = now;
    let challenge = HeadChallenge {
        discriminator: HeadChallenge::DISCRIMINATOR,
        version: HeadChallenge::VERSION,
        bump: challenge_bump,
        target: *head_info.key,
        head: *head_info.key,
        challenger: *challenger_info.key,
        kind: HeadChallengeKind::Source,
        reason: params.reason,
        index,
        case_number: head.source_case,
        bond: params.bond,
        evidence_kind: params.evidence_kind,
        evidence_commitment: params.evidence_commitment,
        evidence_object: object,
        reason_hash: params.reason_hash,
        opened_ts: now,
        rent_payer: *payer_info.key,
        ..HeadChallenge::default()
    };
    store_heads_account(challenge_info, &challenge)?;
    store_heads_account(head_info, head.as_ref())
}

/// Selector 19: once the case's window ended with every challenge ruled, or at its ruling
/// deadline, `outcome::resolve_source` runs once from the head's ruled challenges (an unruled one
/// timed out: its bond back at `SettleChallenge`; a winning ruling keeps the falling outcome,
/// early review B2). Standing: every losing bond to the proposer's `UserCollateral`. Falling: the proposer's delegation (synced first, so its pending
/// strikes apply before) pays `proposer_slash_bps` of what it holds to the winners, the losers'
/// bonds burn, and the source takes one strike (the proposer skips it: it paid at resolution).
#[inline(never)]
pub(super) fn finalize_source_challenges(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    roles(accounts, b"rwwwww")?;
    let [config_info, heads_config_info, board_info, head_info, delegation_info, collateral_info] =
        accounts
    else {
        return Err(bad_list());
    };
    load_current_canonical_vault_config(program_id, config_info)?;
    let mut heads_config: Box<HeadsConfig> = load_heads_account(program_id, heads_config_info)?;
    let mut board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut head = load_head_of(program_id, head_info, board_info.key)?;
    let now = t::now()?;
    if head.source_case_status != SOURCE_CASE_OPEN
        || now < head.source_case_window_end_ts
        || (head.source_case_ruled != head.source_case_challenges
            && now < head.source_case_deadline_ts)
    {
        return Err(bad_state());
    }
    let proposer = head.proposer;
    expect_key(
        delegation_info,
        &find_head_delegation_address(program_id, head_info.key, &proposer).0,
    )?;
    let params = board.params.to_params();
    let challenges = aggregate_challenges(
        head.source_case_ruled_bonds,
        head.source_case_winning_bonds,
        head.source_case_won,
    )?;
    if head.source_case_won == 0 {
        let resolution = outcome::resolve_source(&challenges, 0, params.proposer_slash_bps);
        credit_usdc(
            program_id,
            collateral_info,
            &proposer,
            resolution.to_proposer,
        )?;
        head.source_case_status = SOURCE_CASE_STOOD;
    } else {
        // The proposer's UserCollateral receives nothing, so it is not needed. A proposer who
        // fully exited and closed holds nothing to take.
        let mut proposer_delegation: Option<Box<Delegation>> = if absent(delegation_info) {
            None
        } else {
            let mut d: Box<Delegation> = load_heads_account(program_id, delegation_info)?;
            if d.head != *head_info.key || d.owner != proposer {
                return Err(OracleError::InvalidHeadsAccount.into());
            }
            t::sync_delegation(
                &params,
                &mut heads_config,
                &mut board,
                &mut head,
                &mut d,
                now,
            )?;
            Some(d)
        };
        let principal = proposer_delegation
            .as_deref()
            .map_or(0, |d| t::proposer_principal(d, &head));
        let resolution = outcome::resolve_source(&challenges, principal, params.proposer_slash_bps);
        // The winners share what the slash actually took, never more.
        let mut taken = 0;
        if let Some(d) = proposer_delegation.as_mut() {
            if resolution.proposer_slashed > 0 {
                taken =
                    t::slash_proposer(&mut board, &mut head, d, resolution.proposer_slashed, now)?;
            }
        }
        board.pending_usdc_burn = add(board.pending_usdc_burn, resolution.burned)?;
        head.source_case_proposer_slashed = taken;
        head.source_case_status = SOURCE_CASE_FELL;
        let opened = head.source_case_opened_ts;
        t::record_strike(&params, &mut board, &mut head, opened, now)?;
        if let Some(d) = proposer_delegation.as_mut() {
            d.applied_strikes = head.strike_count;
            d.updated_ts = now;
            store_heads_account(delegation_info, d.as_ref())?;
        }
    }
    t::close_case(&mut head)?;
    head.updated_ts = now;
    board.updated_ts = now;
    heads_config.updated_ts = now;
    store_heads_account(head_info, head.as_ref())?;
    store_heads_account(board_info, board.as_ref())?;
    store_heads_account(heads_config_info, heads_config.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_case_pays_every_bond_to_the_proposer() {
        let challenges = aggregate_challenges(12, 0, 0).unwrap();
        let resolution = outcome::resolve_source(&challenges, 5_000, 10_000);
        assert!(!resolution.falls);
        assert_eq!(resolution.to_proposer, 12);
        assert_eq!(resolution.proposer_slashed, 0);
    }
}
