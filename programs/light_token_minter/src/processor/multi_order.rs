//! Typed all-or-none exchange of already backed canonical assets. A filler can
//! obtain assets from any venue; this handler accepts no arbitrary instructions.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::{multi_order as mo, regular_compressed_transfer as transfer};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::instruction::AccountMeta;

fn invalid() -> ProgramError {
    VaultError::InvalidAccountList.into()
}
fn checked<T>(v: Option<T>) -> Result<T, ProgramError> {
    v.ok_or_else(|| VaultError::ArithmeticOverflow.into())
}
fn magnitude(v: i128) -> Result<u64, ProgramError> {
    u64::try_from(v.unsigned_abs()).map_err(|_| invalid())
}

pub(in crate::processor) fn load(
    program: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
    nonce: &[u8; 32],
) -> Result<mo::Order, ProgramError> {
    load_checked(program, info, owner, nonce, true)
}
pub(in crate::processor) fn load_readonly(
    program: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
    nonce: &[u8; 32],
) -> Result<mo::Order, ProgramError> {
    load_checked(program, info, owner, nonce, false)
}
fn load_checked(
    program: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
    nonce: &[u8; 32],
    writable: bool,
) -> Result<mo::Order, ProgramError> {
    if info.owner != program
        || info.executable
        || writable && !info.is_writable
        || info.is_signer
        || info.data_len() > MAX_INSTRUCTION_DATA_BYTES
    {
        return Err(invalid());
    }
    let order = mo::Order::try_from_slice(&info.try_data()?).map_err(|_| invalid())?;
    let (key, bump) = mo::derive(program, &order.owner, nonce);
    if *info.key != key
        || order.magic != mo::MAGIC
        || order.version != mo::VERSION
        || order.bump != bump
        || order.custody_owner != *owner
        || order.custody_owner != order.owner
            && order.custody_owner != crate::trading_session::derive(program, &order.owner).0
        || order.nonce != *nonce
        || order.status > mo::EXPIRED
        || order.expiry_ts != order.legs.iter().map(|l| l.expiry_ts).min().unwrap_or(0)
        || mo::assets(&order.legs).is_none()
    {
        return Err(invalid());
    }
    Ok(order)
}
pub(in crate::processor) fn store(info: &AccountInfo, order: &mo::Order) -> ProgramResult {
    let bytes = order.try_to_vec().map_err(|_| invalid())?;
    if bytes.len() != info.data_len() {
        return Err(invalid());
    }
    info.try_data_mut()?.copy_from_slice(&bytes);
    Ok(())
}

/// Common accounts: original owner, action actor, SOL payer, order PDA, config,
/// quote mint, Light Token, its CPI authority, System, Light System, registered
/// program, compression authority, compression program, SPL Token, classic quote
/// ATA (System when unused), quote interface (System when unused). Each distinct
/// option then has Market, mint, owner settlement delegate, actor delegate;
/// packed Merkle accounts follow. Governance is removed by the central dispatcher.
fn validate_base(a: &[AccountInfo], asset_count: usize, merkle: u8, place: bool) -> ProgramResult {
    let end = checked(mo::COMMON.checked_add(checked(asset_count.checked_mul(4))?))?;
    if a.len() != end + usize::from(merkle)
        || a.len() > 255
        || merkle < 2
        || !a[1].is_signer
        || !a[2].is_signer
        || !a[2].is_writable
        || !a[3].is_writable
        || a[3].is_signer
        || (place && (!a[0].is_signer || a[0].key != a[1].key))
        || a[0].key == a[3].key
        || a[1].key == a[3].key
        || a[2].key == a[3].key
        || !crate::light_token_instruction::is_program(a[6].key)
        || !crate::light_token_instruction::is_cpi_authority(a[7].key)
        || !crate::is_system_program(a[8].key)
        || !crate::token_instruction::check_id(a[13].key)
        || !crate::light_token_instruction::is_light_system_program(a[9].key)
        || !crate::light_token_instruction::is_registered_program(a[10].key)
        || !crate::light_token_instruction::is_compression_authority(a[11].key)
        || !crate::light_token_instruction::is_compression_program(a[12].key)
        || a[end..].iter().enumerate().any(|(i, v)| {
            v.is_signer
                || v.executable
                || !v.is_writable
                || a[end..end + i].iter().any(|p| p.key == v.key)
        })
    {
        return Err(invalid());
    }
    Ok(())
}
fn optional_delegate(program: &Pubkey, info: &AccountInfo, owner: &Pubkey, mint: &Pubkey) -> bool {
    crate::is_system_program(info.key)
        || *info.key
            == crate::scoped_settlement::derive_collective_settlement_delegate(program, owner, mint)
                .0
}
fn bind_assets(
    program: &Pubkey,
    a: &[AccountInfo],
    order: &mo::Order,
    assets: &[mo::Asset],
    mode: u8,
    session_authority: bool,
) -> ProgramResult {
    let actor_owner = if session_authority {
        a[0].key
    } else {
        a[1].key
    };
    if mode >= 2 {
        // The authenticated immutable order is sufficient for a refund. Market
        // cleanup, paused trading, and expiry cannot revoke custody recovery.
        if *a[5].key != order.quote_mint {
            return Err(invalid());
        }
        for (i, asset) in assets.iter().enumerate() {
            let n = mo::COMMON + 4 * i;
            if *a[n].key != asset.market
                || *a[n + 1].key != asset.mint
                || !optional_delegate(program, &a[n + 2], &order.custody_owner, &asset.mint)
                || !optional_delegate(program, &a[n + 3], actor_owner, &asset.mint)
            {
                return Err(invalid());
            }
        }
        return Ok(());
    }
    let config = load_canonical_vault_config(program, &a[4])?;
    if config.usdc_mint != order.quote_mint || *a[5].key != order.quote_mint {
        return Err(invalid());
    }
    validate_collateral_mint_account(&a[5], a[13].key)?;
    for (i, asset) in assets.iter().enumerate() {
        let n = mo::COMMON + 4 * i;
        let mut market = load_valid_market(program, &a[n])?;
        if *a[n].key != asset.market
            || *a[n + 1].key != asset.mint
            || market.long_contract_mint != Some(asset.mint)
            || market.collateral_mint != order.quote_mint
            || asset.mint == order.quote_mint
            || order
                .legs
                .iter()
                .filter(|l| l.market == asset.market)
                .any(|l| l.expiry_ts != market.instrument.expiry_ts)
            || !optional_delegate(program, &a[n + 2], &order.custody_owner, &asset.mint)
            || !optional_delegate(program, &a[n + 3], actor_owner, &asset.mint)
        {
            return Err(invalid());
        }
        validate_canonical_market_mint(&a[n], &mut market, &a[n + 1], 0)?;
        if mode == 1 {
            ensure_market_value_flow_unpaused(&config, &market)?;
        }
    }
    Ok(())
}

pub(super) fn process(program: &Pubkey, a: &[AccountInfo], action: mo::Action) -> ProgramResult {
    if let mo::Action::ReclaimProof = action {
        return super::atomic_proof::reclaim(program, a);
    }
    if let mo::Action::PrepareRouter(wire) = action {
        return super::market_router::prepare(program, a, wire);
    }
    if let mo::Action::FillFromOptionRoutes(wire) = action {
        return super::writer_sleeve::fill_multi_order_from_option_routes(program, a, wire);
    }
    if let mo::Action::UploadProof(wire) = action {
        return super::atomic_proof::upload(program, a, wire);
    }
    if let mo::Action::ConsolidateWriterCash(wire) = action {
        return super::writer_cash_consolidation::consolidate(program, a, wire);
    }
    if let mo::Action::PrepareProjection(wire) = action {
        return super::atomic_projection::process(program, a, wire);
    }
    process_inner(program, a, action, None, false)
}

pub(super) fn process_sponsored_cancel(
    program: &Pubkey,
    a: &[AccountInfo],
    exit: mo::Exit,
) -> ProgramResult {
    process_inner(program, a, mo::Action::Cancel(exit), None, true)
}

/// The session entrypoint authenticates the wallet grant before calling this.
pub(super) fn process_session(
    program: &Pubkey,
    a: &[AccountInfo],
    action: mo::Action,
    owner: Pubkey,
    sponsored: bool,
) -> ProgramResult {
    if a.len() < mo::COMMON || *a[0].key != crate::trading_session::derive(program, &owner).0 {
        return Err(invalid());
    }
    process_inner(program, a, action, Some(owner), sponsored)
}
fn process_inner(
    program: &Pubkey,
    a: &[AccountInfo],
    action: mo::Action,
    session_owner: Option<Pubkey>,
    sponsored: bool,
) -> ProgramResult {
    if a.len() < mo::COMMON {
        return Err(invalid());
    }
    let now = current_unix_timestamp()?;
    let (mut order, merkle, batches, mode, quote_delta, classic) = match action {
        mo::Action::PrepareRouter(_)
        | mo::Action::FillFromOptionRoutes(_)
        | mo::Action::UploadProof(_)
        | mo::Action::ConsolidateWriterCash(_)
        | mo::Action::ReclaimProof
        | mo::Action::PrepareProjection(_) => return Err(invalid()),
        mo::Action::Place(p) => {
            if p.legs.is_empty() {
                return Err(invalid());
            }
            let mut bound = Vec::with_capacity(p.legs.len());
            let mut markets: Vec<Pubkey> = Vec::new();
            for leg in &p.legs {
                let index = usize::from(leg.market_index);
                if index > markets.len() {
                    return Err(invalid());
                }
                let n = mo::COMMON + 4 * index;
                if n + 3 >= a.len() {
                    return Err(invalid());
                }
                let market = load_valid_market(program, &a[n])?;
                if index == markets.len() {
                    if markets.contains(a[n].key) {
                        return Err(invalid());
                    }
                    markets.push(*a[n].key);
                } else if markets[index] != *a[n].key {
                    return Err(invalid());
                }
                if leg.side > 1 || leg.quantity == 0 {
                    return Err(invalid());
                }
                bound.push(mo::BoundLeg {
                    market: *a[n].key,
                    mint: market.long_contract_mint.ok_or_else(invalid)?,
                    expiry_ts: market.instrument.expiry_ts,
                    side: leg.side,
                    quantity: leg.quantity,
                });
            }
            let expiry_ts = bound
                .iter()
                .map(|l| l.expiry_ts)
                .min()
                .ok_or_else(invalid)?;
            if now >= expiry_ts || p.classic_quote_amount > p.quote_bound.escrow() {
                return Err(invalid());
            }
            let owner = session_owner.unwrap_or(*a[0].key);
            let (key, bump) = mo::derive(program, &owner, &p.nonce);
            if *a[3].key != key
                || !crate::is_system_program(a[3].owner)
                || a[3].data_len() != 0
                || a[3].executable
            {
                return Err(invalid());
            }
            (
                mo::Order {
                    magic: mo::MAGIC,
                    version: mo::VERSION,
                    bump,
                    status: mo::OPEN,
                    owner,
                    custody_owner: *a[0].key,
                    nonce: p.nonce,
                    quote_mint: *a[5].key,
                    quote_bound: p.quote_bound,
                    settlement_delegate: p.settlement_delegate,
                    expiry_ts,
                    filled_quote_delta: 0,
                    filled_slot: 0,
                    legs: bound,
                },
                p.merkle_accounts,
                p.batches,
                0,
                0,
                p.classic_quote_amount,
            )
        }
        mo::Action::Fill(p) => (
            load(program, &a[3], a[0].key, &p.nonce)?,
            p.merkle_accounts,
            p.batches,
            1,
            p.quote_delta,
            0,
        ),
        mo::Action::Cancel(p) => (
            load(program, &a[3], a[0].key, &p.nonce)?,
            p.merkle_accounts,
            p.batches,
            2,
            0,
            0,
        ),
        mo::Action::Expire(p) => (
            load(program, &a[3], a[0].key, &p.nonce)?,
            p.merkle_accounts,
            p.batches,
            3,
            0,
            0,
        ),
    };
    if order.status != mo::OPEN
        || mode == 1 && (now >= order.expiry_ts || !order.quote_bound.admits(quote_delta))
        || mode == 3 && now < order.expiry_ts
    {
        return Err(invalid());
    }
    if session_owner.is_some_and(|wallet| order.owner != wallet)
        || session_owner.is_some() && classic != 0
    {
        return Err(VaultError::Unauthorized.into());
    }
    if mode == 2
        && session_owner.is_none()
        && (!a[1].is_signer
            || *a[1].key != order.owner
                && *a[1].key != load_canonical_vault_config(program, &a[4])?.admin)
    {
        return Err(VaultError::Unauthorized.into());
    }
    magnitude(quote_delta)?;
    let assets = mo::assets(&order.legs).ok_or_else(invalid)?;
    validate_base(
        a,
        assets.len(),
        merkle,
        mode == 0 && session_owner.is_none(),
    )?;
    bind_assets(program, a, &order, &assets, mode, session_owner.is_some())?;
    if mode == 0 {
        let bytes = order.try_to_vec().map_err(|_| invalid())?;
        if bytes.len() > MAX_INSTRUCTION_DATA_BYTES {
            return Err(invalid());
        }
        invoke_create_or_allocate_account(
            &a[2],
            &a[3],
            &a[8],
            program,
            bytes.len(),
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                mo::SEED,
                order.owner.as_ref(),
                &order.nonce,
                &[order.bump],
            ],
        )?;
    }
    settle(
        program,
        a,
        &order,
        &assets,
        merkle,
        &batches,
        mode,
        quote_delta,
        classic,
        session_owner.is_some(),
        if sponsored {
            crate::trading_session::SPONSOR_FEE_ATOMS
        } else {
            0
        },
    )?;
    order.status = match mode {
        0 => mo::OPEN,
        1 => mo::FILLED,
        2 => mo::CANCELLED,
        _ => mo::EXPIRED,
    };
    if mode == 1 {
        order.filled_quote_delta = quote_delta;
        order.filled_slot = crate::compact_error::slot()?;
    }
    store(&a[3], &order)
}

// Keep the existing accounting interface and its explicit inputs.
#[allow(clippy::too_many_arguments)]
fn settle<'info>(
    program: &Pubkey,
    a: &[AccountInfo<'info>],
    order: &mo::Order,
    assets: &[mo::Asset],
    _merkle_count: u8,
    batches: &[mo::Batch],
    mode: u8,
    quote_delta: i128,
    classic: u64,
    session_authority: bool,
    sponsor_fee: u64,
) -> ProgramResult {
    let count = assets.len().div_ceil(mo::OPTIONS_PER_BATCH);
    if batches.len() != count {
        return Err(invalid());
    }
    let end = mo::COMMON + 4 * assets.len();
    let merkle = &a[end..];
    let mut consumed: Vec<(u8, u8, u32)> = Vec::new();
    let escrow_quote = order.quote_bound.escrow();
    for (batch_index, batch) in batches.iter().enumerate() {
        let start = batch_index * mo::OPTIONS_PER_BATCH;
        let stop = (start + mo::OPTIONS_PER_BATCH).min(assets.len());
        if batch.output_tree == batch.output_queue
            || usize::from(batch.output_tree) >= merkle.len()
            || usize::from(batch.output_queue) >= merkle.len()
        {
            return Err(invalid());
        }
        let mut infos = vec![
            a[9].clone(),
            a[2].clone(),
            a[7].clone(),
            a[10].clone(),
            a[11].clone(),
            a[12].clone(),
            a[8].clone(),
        ];
        infos.extend(merkle.iter().cloned());
        let mut metas = infos
            .iter()
            .enumerate()
            .map(|(i, v)| AccountMeta {
                pubkey: *v.key,
                is_signer: i == 1,
                is_writable: i == 1 || i >= 7,
            })
            .collect::<Vec<_>>();
        let mut add = |role: usize, signer: bool, writable: bool| -> Result<u8, ProgramError> {
            let packed = u8::try_from(infos.len() - 7).map_err(|_| invalid())?;
            infos.push(a[role].clone());
            metas.push(AccountMeta {
                pubkey: *a[role].key,
                is_signer: signer,
                is_writable: writable,
            });
            Ok(packed)
        };
        let owner = add(0, session_authority, false)?;
        let actor = add(1, true, false)?;
        let escrow = add(3, true, false)?;
        let sponsor = add(2, true, true)?;
        let quote = add(5, false, false)?;
        let classic_ata = add(14, false, classic > 0)?;
        let quote_interface = add(15, false, classic > 0)?;
        let spl = add(13, false, false)?;
        let _ = spl;
        let mut roles = vec![(0, quote, 0, 0)];
        for i in start..stop {
            let n = mo::COMMON + 4 * i;
            roles.push((
                i + 1,
                add(n + 1, false, false)?,
                add(n + 2, false, false)?,
                add(n + 3, false, false)?,
            ));
        }
        let mut inputs = Vec::with_capacity(batch.inputs.len());
        let mut actor_total = vec![0u64; roles.len()];
        let mut escrow_total = vec![0u64; roles.len()];
        for input in &batch.inputs {
            let w = input.witness;
            let slot = roles
                .iter()
                .position(|r| r.0 == usize::from(input.asset))
                .ok_or_else(invalid)?;
            if input.party > 1
                || mode != 1
                    && input.party != if mode == 0 { 0 } else { 1 }
                    && !(mode == 2 && sponsor_fee > 0 && input.party == 0 && input.asset == 0)
                || input.amount == 0
                || input.asset == 0 && batch_index != 0
                || input.delegated && (input.party == 1 || input.asset == 0)
                || input.delegated
                    && crate::is_system_program(infos[usize::from(roles[slot].3) + 7].key)
                || usize::from(w.tree_index) >= merkle.len()
                || usize::from(w.queue_index) >= merkle.len()
                || w.tree_index == w.queue_index
                || consumed.contains(&(w.tree_index, w.queue_index, w.leaf_index))
            {
                return Err(invalid());
            }
            consumed.push((w.tree_index, w.queue_index, w.leaf_index));
            let totals = if input.party == 0 {
                &mut actor_total
            } else {
                &mut escrow_total
            };
            totals[slot] = checked(totals[slot].checked_add(input.amount))?;
            inputs.push(transfer::InputLeaf {
                owner: if input.party == 0 {
                    if session_authority {
                        owner
                    } else {
                        actor
                    }
                } else {
                    escrow
                },
                amount: input.amount,
                has_delegate: input.delegated,
                delegate: roles[slot].3,
                mint: roles[slot].1,
                tree: w.tree_index,
                queue: w.queue_index,
                leaf_index: w.leaf_index,
                prove_by_index: w.prove_by_index,
                root_index: w.root_index,
            });
        }
        let mut outputs = Vec::new();
        let mut push = |who: u8, amount: u64, mint: u8, delegate: Option<u8>| -> ProgramResult {
            if amount > 0 {
                if delegate.is_some_and(|d| crate::is_system_program(infos[usize::from(d) + 7].key))
                {
                    return Err(invalid());
                }
                outputs.push(transfer::OutputLeaf {
                    owner: who,
                    amount,
                    mint,
                    has_delegate: delegate.is_some(),
                    delegate: delegate.unwrap_or(0),
                });
            }
            Ok(())
        };
        for (slot, &(asset, mint, owner_delegate, actor_delegate)) in roles.iter().enumerate() {
            if asset == 0 && batch_index != 0 {
                continue;
            }
            let delta = if asset == 0 {
                quote_delta
            } else {
                assets[asset - 1].delta
            };
            let escrow_required = if asset == 0 {
                escrow_quote
            } else if delta < 0 {
                magnitude(delta)?
            } else {
                0
            };
            let own_delegate = if asset > 0 && order.settlement_delegate {
                Some(owner_delegate)
            } else {
                None
            };
            let actor_has_delegate = batch
                .inputs
                .iter()
                .any(|v| usize::from(v.asset) == asset && v.party == 0 && v.delegated);
            let act_delegate = if actor_has_delegate {
                Some(actor_delegate)
            } else {
                None
            };
            if mode == 0 {
                let hot = if asset == 0 { classic } else { 0 };
                let total = checked(actor_total[slot].checked_add(hot))?;
                let fee = if asset == 0 { sponsor_fee } else { 0 };
                let required = checked(escrow_required.checked_add(fee))?;
                if total < required || escrow_total[slot] != 0 {
                    return Err(invalid());
                }
                push(escrow, escrow_required, mint, None)?;
                push(owner, total - required, mint, own_delegate)?;
                push(sponsor, fee, mint, None)?;
            } else {
                if escrow_total[slot] != escrow_required {
                    return Err(invalid());
                }
                if mode == 1 {
                    let from_actor = if delta > 0 { magnitude(delta)? } else { 0 };
                    let to_actor = if delta < 0 { magnitude(delta)? } else { 0 };
                    if actor_total[slot] < from_actor || to_actor > escrow_required {
                        return Err(invalid());
                    }
                    push(owner, from_actor, mint, own_delegate)?;
                    push(
                        actor,
                        checked((actor_total[slot] - from_actor).checked_add(to_actor))?,
                        mint,
                        act_delegate,
                    )?;
                    push(owner, escrow_required - to_actor, mint, own_delegate)?;
                } else {
                    let fee = if asset == 0 { sponsor_fee } else { 0 };
                    let total = checked(escrow_required.checked_add(actor_total[slot]))?;
                    if actor_total[slot] != 0 && !(asset == 0 && sponsor_fee > 0) || total < fee {
                        return Err(invalid());
                    }
                    push(owner, total - fee, mint, own_delegate)?;
                    push(sponsor, fee, mint, None)?;
                }
            }
        }
        let mut hot = Vec::new();
        let mut classic_before = None;
        if mode == 0 && classic > 0 && batch_index == 0 {
            let canonical = crate::associated_token::get_associated_token_address_with_program_id(
                &order.owner,
                &order.quote_mint,
                &spl_token_program_id(),
            );
            if *a[14].key != canonical || !a[14].is_writable || !a[15].is_writable {
                return Err(invalid());
            }
            classic_before =
                Some(validate_vault_token_account(&a[14], &order.quote_mint, &order.owner)?.amount);
            let (_, bump) = validate_spl_interface_account_with_bump(&order.quote_mint, &a[15])?;
            hot.push(transfer::HotCompression {
                amount: classic,
                mint: quote,
                source: classic_ata,
                authority: actor,
                pool_account_index: quote_interface,
                pool_index: 0,
                bump,
                decimals: 6,
            });
        }
        if inputs.is_empty() && hot.is_empty() {
            if !outputs.is_empty() {
                return Err(invalid());
            }
            continue;
        }
        let ix = transfer::instruction_with_compressions(
            *a[6].key,
            metas,
            batch.output_queue,
            batch.proof,
            &inputs,
            &hot,
            &outputs,
        )?;
        infos.push(a[6].clone());
        let session_bump = [crate::trading_session::derive(program, &order.owner).1];
        let session_seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::trading_session::SEED,
            order.owner.as_ref(),
            &session_bump,
        ];
        let order_bump = [order.bump];
        let order_seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            mo::SEED,
            order.owner.as_ref(),
            &order.nonce,
            &order_bump,
        ];
        let mut signers = vec![order_seeds];
        if session_authority {
            signers.push(session_seeds);
        }
        invoke_signed(&ix, &infos, &signers)?;
        if let Some(before) = classic_before {
            let after =
                validate_vault_token_account(&a[14], &order.quote_mint, &order.owner)?.amount;
            if before.checked_sub(after) != Some(classic) {
                return Err(VaultError::AmoebaDlmmInvariantViolation.into());
            }
        }
    }
    let _ = program;
    Ok(())
}
