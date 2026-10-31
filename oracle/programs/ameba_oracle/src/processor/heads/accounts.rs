//! Account plumbing the heads handlers share: signer and writable checks, the oracle config,
//! create-only PDAs at the canonical bump, and loaders that bind each account's stored identities
//! to the instruction's other accounts.

use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::state::heads::{
    find_head_delegation_address, find_heads_board_address, find_heads_config_address,
    find_heads_stake_address, load_heads_account, store_heads_account, Delegation, Head,
    HeadsAccount, HeadsBoard, HeadsConfig, HeadsStake, HEADS_EVIDENCE_NONE, HEADS_STAKE_SEED,
    HEAD_DELEGATION_SEED,
};

pub(super) type Res<T> = Result<T, ProgramError>;

pub(super) fn bad_list() -> ProgramError {
    OracleError::InvalidAccountList.into()
}

pub(super) fn bad_account() -> ProgramError {
    OracleError::InvalidHeadsAccount.into()
}

pub(super) fn bad_state() -> ProgramError {
    OracleError::InvalidHeadsState.into()
}

pub(super) fn overflow() -> ProgramError {
    OracleError::ArithmeticOverflow.into()
}

pub(super) fn add(a: u64, b: u64) -> Res<u64> {
    a.checked_add(b).ok_or_else(overflow)
}

pub(super) fn sub(a: u64, b: u64) -> Res<u64> {
    a.checked_sub(b).ok_or_else(overflow)
}

/// Checks a selector's fixed account list: `W` writable signer, `R` signer, `w` writable, `r`
/// any (read-only roles tolerate a writable duplicate of another role).
pub(super) fn roles(accounts: &[AccountInfo], spec: &[u8]) -> ProgramResult {
    if accounts.len() != spec.len() {
        return Err(bad_list());
    }
    for (info, role) in accounts.iter().zip(spec) {
        let ok = match role {
            b'W' => info.is_signer && info.is_writable,
            b'R' => info.is_signer,
            b'w' => info.is_writable,
            _ => true,
        };
        if !ok {
            return Err(bad_list());
        }
    }
    Ok(())
}

/// The system program in its own slot, or as the placeholder of a role that does not apply.
pub(super) fn system(info: &AccountInfo) -> ProgramResult {
    if !crate::is_system_program(info.key) {
        return Err(OracleError::InvalidSystemProgram.into());
    }
    Ok(())
}

pub(super) fn expect_key(info: &AccountInfo, expected: &Pubkey) -> ProgramResult {
    if !crate::pubkey_eq(info.key, expected) {
        return Err(bad_account());
    }
    Ok(())
}

/// Whether `info` is absent: a system-owned account without data (it may hold lamports).
pub(super) fn absent(info: &AccountInfo) -> bool {
    crate::is_system_program(info.owner) && info.data_len() == 0 && !info.executable
}

/// Debits `amount` USDC from the owner's `UserCollateral` into escrow (the tokens stay in the
/// bond vault). Nothing moves for zero.
pub(super) fn debit_usdc(
    program_id: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
    amount: u64,
) -> ProgramResult {
    let mut collateral = load_canonical_user_collateral(program_id, info, owner)?;
    if amount > 0 {
        usdc::debit_oracle_usdc_available(&mut collateral, amount)?;
        store_state(info, &collateral)?;
    }
    Ok(())
}

/// Credits `amount` USDC to the owner's `UserCollateral`.
pub(super) fn credit_usdc(
    program_id: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
    amount: u64,
) -> ProgramResult {
    let mut collateral = load_canonical_user_collateral(program_id, info, owner)?;
    if amount > 0 {
        usdc::credit_oracle_usdc_available(&mut collateral, amount)?;
        store_state(info, &collateral)?;
    }
    Ok(())
}

/// The sealed evidence object a claim or challenge references: the system program for none, else
/// a sealed object of the declared kind whose commitment is the declared one. Returns its address
/// (default for none) and its capture time (kinds 3 and 4).
pub(super) fn evidence(
    program_id: &Pubkey,
    info: &AccountInfo,
    kind: u8,
    commitment: &[u8; 32],
) -> Res<(Pubkey, Option<u64>)> {
    if kind == HEADS_EVIDENCE_NONE {
        system(info)?;
        return Ok((Pubkey::default(), None));
    }
    let sealed =
        crate::processor::oracle_evidence::sealed_evidence(program_id, info, kind, commitment)
            .map_err(|_| ProgramError::from(OracleError::InvalidHeadsEvidence))?;
    Ok((*info.key, sealed.capture_ts))
}

/// A head bound to its board.
pub(super) fn load_head_of(
    program_id: &Pubkey,
    info: &AccountInfo,
    board: &Pubkey,
) -> Res<Box<Head>> {
    let head: Box<Head> = load_heads_account(program_id, info)?;
    if head.board != *board {
        return Err(bad_account());
    }
    Ok(head)
}

pub(super) fn signer(info: &AccountInfo) -> ProgramResult {
    if !info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}

pub(super) fn writable(info: &AccountInfo) -> ProgramResult {
    if !info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    Ok(())
}

/// The oracle config; `unpaused` refuses a paused one (the value flows, as the rest of the program
/// does).
pub(super) fn oracle_config(
    program_id: &Pubkey,
    info: &AccountInfo,
    unpaused: bool,
) -> Result<OracleConfig, ProgramError> {
    let config = load_canonical_vault_config(program_id, info)?;
    if unpaused && config.paused {
        return Err(OracleError::ContractPaused.into());
    }
    Ok(config)
}

/// The config admin, signing (setup is allowed while paused, as for the rest of the program's
/// create-only setup).
pub(super) fn admin(
    program_id: &Pubkey,
    admin_info: &AccountInfo,
    config_info: &AccountInfo,
) -> Result<OracleConfig, ProgramError> {
    signer(admin_info)?;
    let config = oracle_config(program_id, config_info, false)?;
    if !crate::pubkey_eq(admin_info.key, &config.admin) {
        return Err(OracleError::Unauthorized.into());
    }
    Ok(config)
}

/// Creates the PDA `[ameba-oracle-v1, seeds..]` at its canonical bump (create-only: an
/// initialized, foreign-owned or data-bearing address fails without mutation) and writes `value`
/// with that bump.
pub(super) fn create<'a, T: HeadsAccount>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    seeds: &[&[u8]],
    value: &mut T,
    set_bump: impl FnOnce(&mut T, u8),
) -> ProgramResult {
    signer(payer)?;
    writable(target)?;
    let mut all: Vec<&[u8]> = Vec::with_capacity(seeds.len() + 1);
    all.push(CURRENT_STATE_NAMESPACE_SEED);
    all.extend_from_slice(seeds);
    let (address, bump) = Pubkey::find_program_address(&all, program_id);
    if !crate::pubkey_eq(target.key, &address) {
        return Err(OracleError::InvalidPda.into());
    }
    let bump_seed = [bump];
    let mut signer_seeds: Vec<&[u8]> = seeds.to_vec();
    signer_seeds.push(&bump_seed);
    create_new_program_account(
        payer,
        target,
        system_program,
        program_id,
        T::LEN,
        &signer_seeds,
    )?;
    set_bump(value, bump);
    store_heads_account(target, value)
}

/// Loads and binds the heads config.
pub(super) fn heads_config(
    program_id: &Pubkey,
    info: &AccountInfo,
) -> Result<Box<HeadsConfig>, ProgramError> {
    let config = load_heads_account::<HeadsConfig>(program_id, info)?;
    if !crate::pubkey_eq(info.key, &find_heads_config_address(program_id).0) {
        return Err(bad_account());
    }
    Ok(config)
}

/// Loads a board (its address binds its product).
pub(super) fn board(
    program_id: &Pubkey,
    info: &AccountInfo,
) -> Result<Box<HeadsBoard>, ProgramError> {
    let board = load_heads_account::<HeadsBoard>(program_id, info)?;
    debug_assert_eq!(
        find_heads_board_address(program_id, &board.product_id).0,
        *info.key
    );
    Ok(board)
}

/// Loads the owner's stake.
pub(super) fn stake(
    program_id: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
) -> Result<Box<HeadsStake>, ProgramError> {
    let stake = load_heads_account::<HeadsStake>(program_id, info)?;
    if !crate::pubkey_eq(&stake.owner, owner) {
        return Err(bad_account());
    }
    Ok(stake)
}

/// Loads the owner's stake, or creates it (`payer` pays; a stake never closes) when the address
/// is still empty.
pub(super) fn stake_or_create<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    owner: &Pubkey,
    now: u64,
) -> Result<Box<HeadsStake>, ProgramError> {
    if info.owner == program_id {
        return stake(program_id, info, owner);
    }
    let mut value = Box::new(HeadsStake {
        discriminator: HeadsStake::DISCRIMINATOR,
        version: HeadsStake::VERSION,
        owner: *owner,
        created_ts: now,
        updated_ts: now,
        ..HeadsStake::default()
    });
    create(
        program_id,
        payer,
        info,
        system_program,
        &[HEADS_STAKE_SEED, owner.as_ref()],
        value.as_mut(),
        |v, bump| v.bump = bump,
    )?;
    debug_assert_eq!(find_heads_stake_address(program_id, owner).0, *info.key);
    Ok(value)
}

/// Loads the owner's delegation on `head`.
pub(super) fn delegation(
    program_id: &Pubkey,
    info: &AccountInfo,
    head: &Pubkey,
    owner: &Pubkey,
) -> Result<Box<Delegation>, ProgramError> {
    let d = load_heads_account::<Delegation>(program_id, info)?;
    if !crate::pubkey_eq(&d.head, head) || !crate::pubkey_eq(&d.owner, owner) {
        return Err(bad_account());
    }
    Ok(d)
}

/// Loads the owner's delegation on `head`, or creates an empty one (`payer` pays and is recorded:
/// the rent goes back to it when the delegation closes).
#[allow(clippy::too_many_arguments)]
pub(super) fn delegation_or_create<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    head: &Pubkey,
    board: &Pubkey,
    owner: &Pubkey,
    is_proposer: bool,
    now: u64,
) -> Result<(Box<Delegation>, bool), ProgramError> {
    if info.owner == program_id {
        return Ok((delegation(program_id, info, head, owner)?, false));
    }
    let mut value = Box::new(Delegation {
        discriminator: Delegation::DISCRIMINATOR,
        version: Delegation::VERSION,
        head: *head,
        owner: *owner,
        board: *board,
        is_proposer,
        accrual_since_ts: now,
        created_ts: now,
        updated_ts: now,
        rent_payer: *payer.key,
        ..Delegation::default()
    });
    create(
        program_id,
        payer,
        info,
        system_program,
        &[HEAD_DELEGATION_SEED, head.as_ref(), owner.as_ref()],
        value.as_mut(),
        |v, bump| v.bump = bump,
    )?;
    debug_assert_eq!(
        find_head_delegation_address(program_id, head, owner).0,
        *info.key
    );
    Ok((value, true))
}

/// Stores a heads account (it must be writable).
pub(super) fn store<T: HeadsAccount>(info: &AccountInfo, value: &T) -> ProgramResult {
    writable(info)?;
    store_heads_account(info, value)
}

/// Makes sure the owner's `UserCollateral` ledger exists, creating it empty (as
/// `InitUserCollateral` does; the payer funds the rent) when it does not: an account a later
/// payout needs exists before anyone can depend on it (a ledger never closes).
pub(super) fn collateral_or_create<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    owner: &Pubkey,
) -> ProgramResult {
    if info.owner == program_id {
        return load_canonical_user_collateral(program_id, info, owner).map(|_| ());
    }
    signer(payer)?;
    writable(info)?;
    let (key, bump) = derive_user_collateral_pda(program_id, owner);
    expect_key(info, &key)?;
    create_program_account(
        payer,
        info,
        system_program,
        program_id,
        UserCollateral::LEN,
        &[USER_COLLATERAL_PDA_SEED, owner.as_ref(), &[bump]],
    )?;
    store_state(
        info,
        &UserCollateral {
            is_initialized: true,
            bump,
            owner: *owner,
            ..UserCollateral::default()
        },
    )
}

/// The owner's `UserCollateral` ledger.
pub(super) fn collateral(
    program_id: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
) -> Result<UserCollateral, ProgramError> {
    writable(info)?;
    load_canonical_user_collateral(program_id, info, owner)
}

/// Stores a `UserCollateral` ledger back.
pub(super) fn store_collateral(info: &AccountInfo, value: &UserCollateral) -> ProgramResult {
    store_state(info, value)
}

/// Closes a program account into `recipient`.
pub(super) fn close(
    program_id: &Pubkey,
    target: &AccountInfo,
    recipient: &AccountInfo,
) -> ProgramResult {
    let _ = target.try_data()?;
    close_program_account(program_id, target, recipient)
}

/// Where a closing account's rent goes: the payer it recorded (an account from before the field
/// recorded none: its owner paid). Checks `recipient` is that key.
pub(super) fn rent_recipient(
    recipient: &AccountInfo,
    recorded: &Pubkey,
    owner: &Pubkey,
) -> ProgramResult {
    let payer = if crate::pubkey_is_default(recorded) {
        owner
    } else {
        recorded
    };
    writable(recipient)?;
    expect_key(recipient, payer)
}
