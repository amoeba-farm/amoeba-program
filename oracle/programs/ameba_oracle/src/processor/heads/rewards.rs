//! Money: selectors 3, 29 and 30. Accounts: the matching builders in `instruction::heads`.

use super::accounts::{self as acc, add, signer};
use super::transitions::{self as tr, now};
use super::*;
use crate::state::heads::{load_heads_account, zeroed_box, HeadsEpoch, HEADS_EPOCH_SEED};
use ameba_oracle_heads::{rewards, time};

/// The board's prize epoch for the month of `now`, created (the payer pays) when it is new.
pub(super) fn epoch_or_create<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    board: &Pubkey,
    now: u64,
) -> Result<Box<HeadsEpoch>, ProgramError> {
    let month = time::month_id(now);
    if info.owner == program_id {
        let epoch = load_heads_account::<HeadsEpoch>(program_id, info)?;
        if !crate::pubkey_eq(&epoch.board, board) || epoch.month_id != month {
            return Err(acc::bad_account());
        }
        return Ok(epoch);
    }
    let mut epoch = zeroed_box::<HeadsEpoch>();
    epoch.discriminator = HeadsEpoch::DISCRIMINATOR;
    epoch.version = HeadsEpoch::VERSION;
    epoch.board = *board;
    epoch.month_id = month;
    epoch.start_ts = time::month_start(month);
    epoch.end_ts = time::month_start(month + 1);
    epoch.updated_ts = now;
    let month_seed = month.to_le_bytes();
    acc::create(
        program_id,
        payer,
        info,
        system_program,
        &[HEADS_EPOCH_SEED, board.as_ref(), &month_seed],
        epoch.as_mut(),
        |v, bump| v.bump = bump,
    )?;
    Ok(epoch)
}

/// Selector 3: USDC from the funder's ledger into the board: `prize_cut` to this month's prize
/// epoch, the rest through the reward index over the live, non-duplicate delegation (kept in the
/// reserve while there is none).
#[inline(never)]
pub(super) fn fund_board(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let [payer, funder, config_info, collateral_info, board_info, epoch_info, system_program] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(funder)?;
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    acc::oracle_config(program_id, config_info, true)?;
    let mut collateral = acc::collateral(program_id, collateral_info, funder.key)?;
    usdc::debit_oracle_usdc_available(&mut collateral, amount)?;
    let mut board = acc::board(program_id, board_info)?;
    let now = now()?;
    let mut epoch = epoch_or_create(
        program_id,
        payer,
        epoch_info,
        system_program,
        board_info.key,
        now,
    )?;
    let (prizes, share) = rewards::split_inflow(amount, board.params.prize_cut_bps);
    epoch.prize_pool = add(epoch.prize_pool, prizes)?;
    epoch.updated_ts = now;
    board.prize_funded_total = add(board.prize_funded_total, prizes)?;
    board.delegator_funded_total = add(board.delegator_funded_total, share)?;
    let pool = add(share, board.reserve)?;
    match rewards::index_after(board.reward_index, pool, board.live_delegation) {
        Some(index) => {
            board.reward_index = index;
            board.reserve = 0;
        }
        None => board.reserve = pool,
    }
    board.updated_ts = now;
    acc::store_collateral(collateral_info, &collateral)?;
    acc::store(board_info, board.as_ref())?;
    acc::store(epoch_info, epoch.as_ref())
}

/// Selector 23: the delegation's accrued rewards into its owner's `UserCollateral`.
#[inline(never)]
pub(super) fn claim_rewards(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [owner, config_info, board_info, head_info, delegation_info, collateral_info] = accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(owner)?;
    acc::oracle_config(program_id, config_info, true)?;
    let mut board = acc::board(program_id, board_info)?;
    let head = acc::load_head_of(program_id, head_info, board_info.key)?;
    let mut d = acc::delegation(program_id, delegation_info, head_info.key, owner.key)?;
    let mut collateral = acc::collateral(program_id, collateral_info, owner.key)?;
    tr::accrue_rewards(&board, &head, &mut d)?;
    let amount = d.rewards_accrued;
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    let now = now()?;
    d.rewards_accrued = 0;
    d.rewards_claimed = add(d.rewards_claimed, amount)?;
    d.updated_ts = now;
    board.delegator_paid_total = add(board.delegator_paid_total, amount)?;
    board.updated_ts = now;
    usdc::credit_oracle_usdc_available(&mut collateral, amount)?;
    acc::store_collateral(collateral_info, &collateral)?;
    acc::store(board_info, board.as_ref())?;
    acc::store(delegation_info, d.as_ref())
}

/// Selector 29, permissionless: once a month ended, its prize money that no prize can claim
/// (`rewards::prize_surplus`) joins the current month's pool. Every prize earned stays payable
/// exactly as before, so it runs once (afterwards there is no surplus).
#[inline(never)]
pub(super) fn roll_over_prizes(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [payer, board_info, ended_info, epoch_info, system_program] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::board(program_id, board_info)?;
    let mut ended = load_heads_account::<HeadsEpoch>(program_id, ended_info)?;
    let now = now()?;
    if !crate::pubkey_eq(&ended.board, board_info.key) || now < ended.end_ts {
        return Err(OracleError::InvalidHeadsState.into());
    }
    let surplus = rewards::prize_surplus(
        ended.prize_pool,
        ended.prize_points,
        ended.prize_paid,
        ended.claims == ended.claims_paid,
    );
    if surplus == 0 {
        return Err(OracleError::InvalidHeadsState.into());
    }
    let mut epoch = epoch_or_create(
        program_id,
        payer,
        epoch_info,
        system_program,
        board_info.key,
        now,
    )?;
    ended.prize_pool -= surplus;
    ended.rolled_out = add(ended.rolled_out, surplus)?;
    ended.updated_ts = now;
    epoch.prize_pool = add(epoch.prize_pool, surplus)?;
    epoch.rolled_in = add(epoch.rolled_in, surplus)?;
    epoch.updated_ts = now;
    acc::store(ended_info, ended.as_ref())?;
    acc::store(epoch_info, epoch.as_ref())
}

/// Selector 24: a standing claim's prize after its month (with the claims, `claims::claim_prize`).
#[inline(never)]
pub(super) fn claim_prize(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    super::claims::claim_prize(program_id, accounts)
}
