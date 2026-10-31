//! The singleton, boards, items and components, AMBA stake, and burns: selectors 0, 1, 2, 4, 5,
//! 31, 32 and 33. Accounts: the matching builders in `instruction::heads`.

use super::accounts::{self as acc, add, signer, writable};
use super::token;
use super::transitions::now;
use super::*;
use crate::instruction::heads::{
    HeadsItemParams, InitBoardParams, InitHeadsConfigParams, SetBoardParamsParams,
    SetComponentParams, SetItemParams,
};
use crate::state::heads::{
    find_heads_authority_address, zeroed_box, HeadsBoard, HeadsConfig, HeadsParams,
    HEADS_BOARD_SEED, HEADS_CHANNELS, HEADS_CONFIG_SEED, HEADS_MAX_ITEMS,
};

fn set_item_fields(board: &mut HeadsBoard, index: usize, item: &HeadsItemParams) {
    board.item_bucket_ids[index] = item.bucket_id;
    board.item_weight_bps[index] = item.basket_weight_bps;
    board.item_mix_bps[index * HEADS_CHANNELS..(index + 1) * HEADS_CHANNELS]
        .copy_from_slice(&item.mix_bps);
    board.item_mix_basis[index] = item.mix_basis as u8;
}

/// Appends item `board.item_count` with its three components (sentences unset).
fn append_item(board: &mut HeadsBoard, item: &HeadsItemParams) -> ProgramResult {
    let index = usize::from(board.item_count);
    if index >= HEADS_MAX_ITEMS {
        return Err(OracleError::HeadsCapacityExhausted.into());
    }
    set_item_fields(board, index, item);
    for channel in 0..HEADS_CHANNELS {
        let component = index * HEADS_CHANNELS + channel;
        board.component_items[component] = index as u8;
        board.component_channels[component] = channel as u8;
    }
    board.item_count += 1;
    board.component_count += HEADS_CHANNELS as u8;
    Ok(())
}

/// Selector 0: validate the mint (`token::validate_amba_mint`) and the vault
/// (`token::validate_amba_vault`), create the config.
#[inline(never)]
pub(super) fn init_heads_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitHeadsConfigParams,
) -> ProgramResult {
    let [payer, admin_info, config_info, heads_config_info, authority_info, mint_info, vault_info, token_program_info, system_program] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::admin(program_id, admin_info, config_info)?;
    if !crate::pubkey_eq(mint_info.key, &params.amba_mint)
        || !crate::pubkey_eq(token_program_info.key, &params.amba_token_program)
    {
        return Err(OracleError::InvalidHeadsTokenAccount.into());
    }
    let mint = token::validate_amba_mint(mint_info, token_program_info.key)?;
    let (authority, authority_bump) = find_heads_authority_address(program_id);
    if !crate::pubkey_eq(authority_info.key, &authority) {
        return Err(OracleError::InvalidHeadsTokenAccount.into());
    }
    token::validate_amba_vault(
        vault_info,
        mint_info.key,
        token_program_info.key,
        &authority,
    )?;
    let now = now()?;
    let mut config = HeadsConfig {
        discriminator: HeadsConfig::DISCRIMINATOR,
        version: HeadsConfig::VERSION,
        authority_bump,
        amba_decimals: mint.base.decimals,
        amba_mint: *mint_info.key,
        amba_token_program: *token_program_info.key,
        amba_vault: *vault_info.key,
        created_ts: now,
        updated_ts: now,
        ..HeadsConfig::default()
    };
    acc::create(
        program_id,
        payer,
        heads_config_info,
        system_program,
        &[HEADS_CONFIG_SEED],
        &mut config,
        |v, bump| v.bump = bump,
    )
}

/// Selector 1: `HeadsParams::validate`, items with valid mixes, a new board whose items each bring
/// their three components.
#[inline(never)]
pub(super) fn init_board(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitBoardParams,
) -> ProgramResult {
    let [payer, admin_info, config_info, heads_config_info, board_info, system_program] = accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::admin(program_id, admin_info, config_info)?;
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    if !params.params.validate() || crate::bytes32_is_zero(&params.product_id) {
        return Err(OracleError::InvalidInstructionData.into());
    }
    let now = now()?;
    let mut board = zeroed_box::<HeadsBoard>();
    board.discriminator = HeadsBoard::DISCRIMINATOR;
    board.version = HeadsBoard::VERSION;
    board.product_id = params.product_id;
    board.params = params.params;
    board.params_version = 1;
    board.created_ts = now;
    board.updated_ts = now;
    for item in &params.items {
        append_item(&mut board, item)?;
    }
    acc::create(
        program_id,
        payer,
        board_info,
        system_program,
        &[HEADS_BOARD_SEED, &params.product_id],
        board.as_mut(),
        |v, bump| v.bump = bump,
    )?;
    config.board_count = config
        .board_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    config.updated_ts = now;
    acc::store(heads_config_info, config.as_ref())
}

/// The parameters fixed once a head is posted: what recorded state is read with at a later sync,
/// strike, release, eviction or finalization, so that a change would apply in the order of
/// permissionless calls (round 10 review N2; round 11 review, findings 1 and 2). The share of the
/// floor (`floor_activate`, `min_delegators`) behind every cached delegator count and backing; the
/// strike policy (`slash_rate_bps`, `max_strikes`, `proposer_slash_bps`) behind recorded strikes
/// and escrow; `cooldown_secs`, how long a member must be unbacked to give up its slot; the
/// prizes of claims already in their windows; and the dispute terms of claims and cases already
/// submitted (`challenge_window_secs`, `challenge_extension_secs`, `challenge_bond`: round 12
/// review, N3, a window pushed past a contract's cutoff). The rest apply only to what comes after
/// (heads.md, "Board parameters").
fn fixed_once_posted(p: &HeadsParams) -> [u64; 13] {
    [
        p.challenge_window_secs,
        p.challenge_extension_secs,
        p.challenge_bond,
        p.floor_activate,
        u64::from(p.min_delegators),
        u64::from(p.slash_rate_bps),
        u64::from(p.max_strikes),
        u64::from(p.proposer_slash_bps),
        p.cooldown_secs,
        p.base_fill_prize,
        p.fill_prize_slope_ppb,
        p.base_update_prize,
        p.update_prize_slope_ppb,
    ]
}

/// Selector 2: compare-and-set `params_version`; contracts copy the parameters when they list.
/// Once a head is posted, [`fixed_once_posted`] parameters do not change.
#[inline(never)]
pub(super) fn set_board_params(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetBoardParamsParams,
) -> ProgramResult {
    let [admin_info, config_info, board_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::admin(program_id, admin_info, config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    if params.expected_params_version != board.params_version {
        return Err(OracleError::InvalidHeadsState.into());
    }
    if !params.params.validate() {
        return Err(OracleError::InvalidInstructionData.into());
    }
    if board.head_count > 0 && fixed_once_posted(&params.params) != fixed_once_posted(&board.params)
    {
        return Err(OracleError::InvalidHeadsState.into());
    }
    let now = now()?;
    // The freeze coverage of a frozen contract is the old parameters' (round 7 review, B1).
    super::transitions::capture_freeze_origins(&mut board, now);
    board.params = params.params;
    board.params_version = board
        .params_version
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    // Activation backing follows the parameters: each component's origins again, the freeze's
    // captured first (round 6 review, finding 1).
    for component in 0..board.component_count {
        super::transitions::recompute_duplicates(&mut board, component, now)?;
    }
    board.updated_ts = now;
    acc::store(board_info, board.as_ref())
}

/// Selector 31: the board's ruling deadline (`HeadsBoard::ruling_deadline_secs`).
#[inline(never)]
pub(super) fn set_ruling_deadline(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    secs: u64,
) -> ProgramResult {
    let [admin_info, config_info, board_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::admin(program_id, admin_info, config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    // Fixed once a head is posted, as the dispute terms are (round 13 review, finding 4): a claim
    // takes its ruling deadline at its first challenge.
    if board.head_count > 0 {
        return Err(OracleError::InvalidHeadsState.into());
    }
    board.ruling_deadline_secs = secs;
    board.updated_ts = now()?;
    acc::store(board_info, board.as_ref())
}

/// Selector 26: set an item's weight, mix and basis, or append an item. An existing item's bucket
/// never changes (its components' heads measure it); weight 0 takes it out of the basket.
#[inline(never)]
pub(super) fn set_item(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetItemParams,
) -> ProgramResult {
    let [admin_info, config_info, board_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::admin(program_id, admin_info, config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    let index = usize::from(params.index);
    let count = usize::from(board.item_count);
    if index < count {
        if board.item_bucket_ids[index] != params.item.bucket_id {
            return Err(OracleError::InvalidHeadsState.into());
        }
        set_item_fields(&mut board, index, &params.item);
    } else if index == count {
        append_item(&mut board, &params.item)?;
    } else {
        return Err(OracleError::InvalidHeadsState.into());
    }
    board.updated_ts = now()?;
    acc::store(board_info, board.as_ref())
}

/// Selector 27: set a component's sentence from its sealed kind-2 object.
#[inline(never)]
pub(super) fn set_component(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetComponentParams,
) -> ProgramResult {
    let [admin_info, config_info, board_info, sentence_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    acc::admin(program_id, admin_info, config_info)?;
    let mut board = acc::board(program_id, board_info)?;
    let index = usize::from(params.index);
    if index >= usize::from(board.component_count)
        || board.component_items[index] != params.item
        || board.component_channels[index] != params.channel as u8
    {
        return Err(OracleError::InvalidHeadsState.into());
    }
    oracle_evidence::sealed_evidence(program_id, sentence_info, 2, &params.sentence_hash)
        .map_err(|_| ProgramError::from(OracleError::InvalidHeadsEvidence))?;
    board.component_sentence_hashes[index] = params.sentence_hash;
    board.updated_ts = now()?;
    acc::store(board_info, board.as_ref())
}

/// Selector 4: AMBA into the vault; the owner's free stake is credited with what arrived.
#[inline(never)]
pub(super) fn stake(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let [payer, owner, heads_config_info, stake_info, source_info, vault_info, mint_info, token_program_info, system_program] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(owner)?;
    writable(source_info)?;
    writable(vault_info)?;
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    token::validate_amba_custody(
        program_id,
        &config,
        token_program_info,
        mint_info,
        vault_info,
        None,
    )?;
    let now = now()?;
    let mut stake = acc::stake_or_create(
        program_id,
        payer,
        stake_info,
        system_program,
        owner.key,
        now,
    )?;
    let received = token::transfer_into_vault(
        token_program_info,
        source_info,
        mint_info,
        vault_info,
        owner,
        amount,
        config.amba_decimals,
    )?;
    if received == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    stake.free = add(stake.free, received)?;
    stake.staked_in_total = add(stake.staked_in_total, received)?;
    stake.updated_ts = now;
    config.amba_accounted = add(config.amba_accounted, received)?;
    config.updated_ts = now;
    acc::store(stake_info, stake.as_ref())?;
    acc::store(heads_config_info, config.as_ref())
}

/// Selector 5: free stake out of the vault, signed by the heads authority.
#[inline(never)]
pub(super) fn unstake(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let [owner, heads_config_info, stake_info, authority_info, vault_info, destination_info, mint_info, token_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    signer(owner)?;
    writable(vault_info)?;
    writable(destination_info)?;
    if amount == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    token::validate_amba_custody(
        program_id,
        &config,
        token_program_info,
        mint_info,
        vault_info,
        Some(authority_info),
    )?;
    let mut stake = acc::stake(program_id, stake_info, owner.key)?;
    if amount > stake.free {
        return Err(OracleError::InsufficientHeadsStake.into());
    }
    let now = now()?;
    stake.free -= amount;
    stake.unstaked_out_total = add(stake.unstaked_out_total, amount)?;
    stake.updated_ts = now;
    config.amba_accounted = config
        .amba_accounted
        .checked_sub(amount)
        .ok_or(OracleError::ArithmeticOverflow)?;
    config.updated_ts = now;
    acc::store(stake_info, stake.as_ref())?;
    acc::store(heads_config_info, config.as_ref())?;
    token::transfer_from_vault(
        token_program_info,
        vault_info,
        mint_info,
        destination_info,
        authority_info,
        config.authority_bump,
        amount,
        config.amba_decimals,
    )
}

/// Selector 25: burns the pending AMBA from the vault and the board's pending USDC from the bond
/// vault.
#[inline(never)]
pub(super) fn burn_pending(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [heads_config_info, authority_info, vault_info, amba_mint_info, amba_token_program_info, board_info, config_info, bond_vault_info, usdc_mint_info, usdc_token_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    let mut config = acc::heads_config(program_id, heads_config_info)?;
    token::validate_amba_custody(
        program_id,
        &config,
        amba_token_program_info,
        amba_mint_info,
        vault_info,
        Some(authority_info),
    )?;
    let mut board = acc::board(program_id, board_info)?;
    let oracle = acc::oracle_config(program_id, config_info, false)?;
    if !crate::pubkey_eq(bond_vault_info.key, &oracle.bond_vault)
        || !crate::pubkey_eq(usdc_mint_info.key, &oracle.usdc_mint)
    {
        return Err(OracleError::InvalidTokenAccount.into());
    }
    let (amba, usdc) = (config.pending_amba_burn, board.pending_usdc_burn);
    if amba == 0 && usdc == 0 {
        return Err(OracleError::AmountMustBePositive.into());
    }
    let now = now()?;
    config.pending_amba_burn = 0;
    config.amba_burned_total = add(config.amba_burned_total, amba)?;
    config.updated_ts = now;
    board.pending_usdc_burn = 0;
    board.usdc_burned_total = add(board.usdc_burned_total, usdc)?;
    board.updated_ts = now;
    acc::store(heads_config_info, config.as_ref())?;
    acc::store(board_info, board.as_ref())?;
    if amba > 0 {
        writable(amba_mint_info)?;
        token::burn_from_vault(
            amba_token_program_info,
            vault_info,
            amba_mint_info,
            authority_info,
            config.authority_bump,
            amba,
            config.amba_decimals,
        )?;
    }
    if usdc > 0 {
        writable(bond_vault_info)?;
        writable(usdc_mint_info)?;
        token::burn_from_bond_vault(
            usdc_token_program_info,
            bond_vault_info,
            usdc_mint_info,
            config_info,
            oracle.bump,
            usdc,
        )?;
    }
    Ok(())
}
