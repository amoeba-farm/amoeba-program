//! Heads and delegations: selectors 6 to 10. Accounts: the matching builders in
//! `instruction::heads`.

use super::accounts::{self as acc, add, signer};
use super::transitions::{self as tr, now};
use super::*;
use crate::instruction::heads::PostHeadParams;
use crate::state::heads::{
    load_heads_account, zeroed_box, Delegation, Head, HeadStatus, HeadsChannel,
    HEAD_NAME_MAX_BYTES, HEAD_SEED,
};

/// Whether a head takes new delegation: posted and not retired or expired, an Open one only
/// until its time to live.
fn accepts_delegation(head: &Head, now: u64) -> bool {
    match head.status {
        HeadStatus::Open => now < head.expires_ts,
        HeadStatus::Active | HeadStatus::Live => true,
        HeadStatus::Retired | HeadStatus::Expired => false,
    }
}

/// Selector 6: the head on its component (fixing its channel) with the proposer's first
/// delegation of at least `min_post`.
#[inline(never)]
pub(super) fn post_head(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: PostHeadParams,
) -> ProgramResult {
    let [payer, proposer, heads_config_info, board_info, stake_info, head_info, delegation_info, spec_info, system_program, collateral_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(proposer)?;
    // A standing source case pays the proposer here: it exists before any case can need it.
    acc::collateral_or_create(
        program_id,
        payer,
        collateral_info,
        system_program,
        proposer.key,
    )?;
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    let mut stake = acc::stake(program_id, stake_info, proposer.key)?;
    let component = usize::from(params.component);
    if params.expected_head_id != board.head_count
        || component >= usize::from(board.component_count)
        || crate::bytes32_is_zero(&board.component_sentence_hashes[component])
        || params.name.is_empty()
        || params.name.len() > HEAD_NAME_MAX_BYTES
    {
        return Err(OracleError::InvalidHeadsState.into());
    }
    oracle_evidence::sealed_evidence(program_id, spec_info, 2, &params.spec_hash)
        .map_err(|_| ProgramError::from(OracleError::InvalidHeadsEvidence))?;
    let p = board.params.to_params();
    if params.amount < tr::min_post(&p, &board) {
        return Err(OracleError::InsufficientHeadsStake.into());
    }
    if params.amount > stake.free {
        return Err(OracleError::InsufficientHeadsStake.into());
    }
    let now = now()?;
    let mut head = zeroed_box::<Head>();
    head.discriminator = Head::DISCRIMINATOR;
    head.version = Head::VERSION;
    head.board = *board_info.key;
    head.head_id = params.expected_head_id;
    head.component = params.component;
    head.channel = HeadsChannel::from_byte(board.component_channels[component])
        .ok_or(OracleError::InvalidHeadsState)?;
    head.access_class = params.access_class;
    head.status = HeadStatus::Open;
    head.name_len = params.name.len() as u8;
    head.name[..params.name.len()].copy_from_slice(params.name.as_bytes());
    head.spec_hash = params.spec_hash;
    head.spec_object = *spec_info.key;
    head.proposer = *proposer.key;
    head.accrual_since_ts = now;
    head.posted_ts = now;
    head.expires_ts = add(now, p.head_ttl_secs)?;
    head.updated_ts = now;
    let id = params.expected_head_id.to_le_bytes();
    acc::create(
        program_id,
        payer,
        head_info,
        system_program,
        &[HEAD_SEED, board_info.key.as_ref(), &id],
        head.as_mut(),
        |v, bump| v.bump = bump,
    )?;
    let (mut d, _) = acc::delegation_or_create(
        program_id,
        payer,
        delegation_info,
        system_program,
        head_info.key,
        board_info.key,
        proposer.key,
        true,
        now,
    )?;
    tr::sync_delegation(&p, &mut config, &mut board, &mut head, &mut d, now)?;
    tr::add_delegation(&mut board, &mut head, &mut d, params.amount, now)?;
    stake.free -= params.amount;
    stake.delegated_total = add(stake.delegated_total, params.amount)?;
    stake.delegation_count = stake
        .delegation_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    stake.updated_ts = now;
    board.head_count = board
        .head_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    tr::maybe_activate(&p, &board, &mut head, now);
    board.updated_ts = now;
    d.updated_ts = now;
    acc::store(heads_config_info, config.as_ref())?;
    acc::store(board_info, board.as_ref())?;
    acc::store(stake_info, stake.as_ref())?;
    acc::store(head_info, head.as_ref())?;
    acc::store(delegation_info, d.as_ref())
}

/// Selector 7: free stake onto a head; an Open head that clears the threshold with enough
/// delegators activates.
#[inline(never)]
pub(super) fn delegate(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let [payer, owner, heads_config_info, board_info, head_info, stake_info, delegation_info, system_program] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(owner)?;
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    let mut head = acc::load_head_of(program_id, head_info, board_info.key)?;
    let mut stake = acc::stake(program_id, stake_info, owner.key)?;
    let now = now()?;
    if !accepts_delegation(&head, now) {
        return Err(OracleError::InvalidHeadsState.into());
    }
    if amount > stake.free {
        return Err(OracleError::InsufficientHeadsStake.into());
    }
    let (mut d, created) = acc::delegation_or_create(
        program_id,
        payer,
        delegation_info,
        system_program,
        head_info.key,
        board_info.key,
        owner.key,
        crate::pubkey_eq(owner.key, &head.proposer),
        now,
    )?;
    let p = board.params.to_params();
    tr::sync_delegation(&p, &mut config, &mut board, &mut head, &mut d, now)?;
    tr::add_delegation(&mut board, &mut head, &mut d, amount, now)?;
    stake.free -= amount;
    stake.delegated_total = add(stake.delegated_total, amount)?;
    if created {
        stake.delegation_count = stake
            .delegation_count
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?;
    }
    stake.updated_ts = now;
    tr::maybe_activate(&p, &board, &mut head, now);
    head.updated_ts = now;
    board.updated_ts = now;
    d.updated_ts = now;
    acc::store(heads_config_info, config.as_ref())?;
    acc::store(board_info, board.as_ref())?;
    acc::store(head_info, head.as_ref())?;
    acc::store(stake_info, stake.as_ref())?;
    acc::store(delegation_info, d.as_ref())
}

/// Selector 8: active delegation into cooldown; the escrow part stays held while exposed.
#[inline(never)]
pub(super) fn undelegate(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let [owner, heads_config_info, board_info, head_info, stake_info, delegation_info] = accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(owner)?;
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    let mut head = acc::load_head_of(program_id, head_info, board_info.key)?;
    acc::stake(program_id, stake_info, owner.key)?;
    let mut d = acc::delegation(program_id, delegation_info, head_info.key, owner.key)?;
    let now = now()?;
    let p = board.params.to_params();
    tr::sync_delegation(&p, &mut config, &mut board, &mut head, &mut d, now)?;
    tr::undelegate(&p, &mut board, &mut head, &mut d, amount, now)?;
    head.updated_ts = now;
    board.updated_ts = now;
    d.updated_ts = now;
    acc::store(heads_config_info, config.as_ref())?;
    acc::store(board_info, board.as_ref())?;
    acc::store(head_info, head.as_ref())?;
    acc::store(delegation_info, d.as_ref())
}

/// Selector 9, permissionless: strikes, captures, rewards, releases what the cooldown and the
/// exposure allow back to free stake; closes an empty, unexposed delegation, its rent going back to
/// whoever paid it (never to the owner, so a sponsor's rent cannot be farmed).
#[inline(never)]
pub(super) fn sync_delegation(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [heads_config_info, board_info, head_info, stake_info, delegation_info, rent_payer] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    let mut head = acc::load_head_of(program_id, head_info, board_info.key)?;
    let mut d = load_heads_account::<Delegation>(program_id, delegation_info)?;
    if !crate::pubkey_eq(&d.head, head_info.key) {
        return Err(acc::bad_account());
    }
    let owner = d.owner;
    let mut stake = acc::stake(program_id, stake_info, &owner)?;
    let now = now()?;
    let p = board.params.to_params();
    tr::sync_delegation(&p, &mut config, &mut board, &mut head, &mut d, now)?;
    let released = tr::release(&p, &board, &head, &mut d, now)?;
    if released > 0 {
        stake.free = add(stake.free, released)?;
        stake.returned_total = add(stake.returned_total, released)?;
    }
    let close = tr::closable(&board, &head, &d);
    if close {
        stake.delegation_count = stake.delegation_count.saturating_sub(1);
    }
    stake.updated_ts = now;
    head.updated_ts = now;
    board.updated_ts = now;
    acc::store(heads_config_info, config.as_ref())?;
    acc::store(board_info, board.as_ref())?;
    acc::store(head_info, head.as_ref())?;
    acc::store(stake_info, stake.as_ref())?;
    if close {
        acc::rent_recipient(rent_payer, &d.rent_payer, &owner)?;
        acc::close(program_id, delegation_info, rent_payer)
    } else {
        d.updated_ts = now;
        acc::store(delegation_info, d.as_ref())
    }
}

/// Selector 10, permissionless: an Open head past its time to live expires; on a full live list,
/// a Live head below activation backing retires (`transitions::expire_head`).
#[inline(never)]
pub(super) fn expire_head(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [board_info, head_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    let mut board = acc::board(program_id, board_info)?;
    let mut head = acc::load_head_of(program_id, head_info, board_info.key)?;
    let now = now()?;
    tr::expire_head(&mut board, &mut head, now)?;
    head.updated_ts = now;
    board.updated_ts = now;
    acc::store(board_info, board.as_ref())?;
    acc::store(head_info, head.as_ref())
}
