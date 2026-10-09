//! Narrow session entrypoint; it accepts no arbitrary instruction, destination,
//! transfer amount outside owner funding, or signer substitution by a client.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::trading_session::{self as session, Action, TradingSessionV2};
use borsh::BorshSerialize;
mod classic_funding;
mod funding;

fn invalid() -> ProgramError {
    VaultError::InvalidAccountList.into()
}

pub(super) fn load(
    program: &Pubkey,
    info: &AccountInfo,
    owner: &Pubkey,
) -> Result<TradingSessionV2, ProgramError> {
    if info.owner != program
        || info.executable
        || info.data_len() != session::ACCOUNT_SIZE
        || *info.key != session::derive(program, owner).0
        || !info.is_writable
    {
        return Err(invalid());
    }
    let record = TradingSessionV2::try_from_slice(&info.try_data()?).map_err(|_| invalid())?;
    if record.owner != *owner || !record.valid(program, info.key) {
        return Err(invalid());
    }
    Ok(record)
}

fn store(info: &AccountInfo, record: &TradingSessionV2) -> ProgramResult {
    let bytes = record.try_to_vec().map_err(|_| invalid())?;
    if bytes.len() != session::ACCOUNT_SIZE || info.data_len() != bytes.len() {
        return Err(invalid());
    }
    info.try_data_mut()?.copy_from_slice(&bytes);
    Ok(())
}

pub(super) fn process(program: &Pubkey, a: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    let action: Action = decode_instruction_payload(payload)?;
    match action {
        Action::DirectWalletCloseAtomicPosition { params } => {
            if a.len() < 2 + crate::atomic_option_route::COMMON
                || !a[0].is_signer
                || a[1].is_signer
                || !crate::is_system_program(a[1].key)
                || a[2].key != a[0].key
                || !a[2].is_signer
            {
                return Err(invalid());
            }
            writer_sleeve::process_atomic_position_close(program, &a[2..], params, *a[0].key)
        }
        Action::CloseAtomicPosition { generation, params } => {
            if a.len() < 2 + crate::atomic_option_route::COMMON
                || a[0].is_signer
                || !a[1].is_signer
                || a[2].is_signer
            {
                return Err(invalid());
            }
            let record = load(program, &a[2], a[0].key)?;
            if record.sponsor != *a[4].key
                || record.quote_mint != *a[6].key
                || !record.authorized(a[1].key, generation, current_unix_timestamp()?)
            {
                return Err(VaultError::Unauthorized.into());
            }
            writer_sleeve::process_atomic_position_close(program, &a[2..], params, record.owner)
        }
        Action::OwnerCloseAtomicPosition { params } => {
            if a.len() < 2 + crate::atomic_option_route::COMMON
                || !a[0].is_signer
                || a[1].is_signer
                || !crate::is_system_program(a[1].key)
            {
                return Err(invalid());
            }
            if a[2].key != a[0].key {
                if a[2].is_signer {
                    return Err(invalid());
                }
                let record = load(program, &a[2], a[0].key)?;
                if record.sponsor != *a[4].key || record.quote_mint != *a[6].key {
                    return Err(invalid());
                }
            } else if !a[2].is_signer {
                return Err(invalid());
            }
            writer_sleeve::process_atomic_position_close(program, &a[2..], params, *a[0].key)
        }
        Action::DirectWalletTradeStrip {
            params,
            classic_quote_amount,
        } => {
            if a.len() < crate::capped_strip::COMMON
                || !a[0].is_signer
                || a[1].is_signer
                || !crate::is_system_program(a[1].key)
                || a[2].key != a[0].key
                || !a[2].is_signer
            {
                return Err(invalid());
            }
            writer_sleeve::process_direct_wallet_strip_trade(
                program,
                a,
                params,
                *a[0].key,
                classic_quote_amount,
            )
        }
        Action::Order { generation, action } => order(program, a, action, Some(generation)),
        Action::OwnerOrder { action } => order(program, a, action, None),
        Action::TradeStrip { generation, params } => {
            if a.len() < crate::capped_strip::COMMON
                || a[0].is_signer
                || !a[1].is_signer
                || a[2].is_signer
            {
                return Err(invalid());
            }
            let record = load(program, &a[2], a[0].key)?;
            if record.sponsor != *a[3].key || record.quote_mint != *a[13].key {
                return Err(invalid());
            }
            if !record.authorized(a[1].key, generation, current_unix_timestamp()?) {
                return Err(VaultError::Unauthorized.into());
            }
            writer_sleeve::process_shared_strip_trade(program, a, params, record.owner)
        }
        Action::OwnerTradeStrip { params } => {
            if a.len() < crate::capped_strip::COMMON
                || !a[0].is_signer
                || a[1].is_signer
                || a[2].is_signer
                || !crate::is_system_program(a[1].key)
            {
                return Err(invalid());
            }
            let record = load(program, &a[2], a[0].key)?;
            if record.sponsor != *a[3].key || record.quote_mint != *a[13].key {
                return Err(invalid());
            }
            writer_sleeve::process_shared_strip_trade(program, a, params, record.owner)
        }
        Action::Enable(grant) => enable(program, a, grant),
        Action::DepositClassic { amount } => classic_funding::deposit(program, a, amount),
        Action::Deposit(movement) => funding::owner_move(program, a, movement, false),
        Action::Withdraw(movement) => funding::owner_move(program, a, movement, true),
        Action::Revoke => {
            if a.len() != 2 || !a[0].is_signer || a[1].is_signer {
                return Err(invalid());
            }
            let mut record = load(program, &a[1], a[0].key)?;
            record.revoked = true;
            if record.generation > 0 {
                record.generation = record
                    .generation
                    .checked_add(1)
                    .ok_or(VaultError::ArithmeticOverflow)?;
            }
            store(&a[1], &record)
        }
        Action::Trade { generation, params } => {
            if a.len() < 3 + 42
                || a[0].is_signer
                || !a[1].is_signer
                || a[2].is_signer
                || a[3].key != a[2].key
            {
                return Err(invalid());
            }
            let record = load(program, &a[2], a[0].key)?;
            let base = &a[3..];
            let sponsor_index = 31 + usize::from(params.page_count) + 8;
            if base.get(sponsor_index).map(|i| *i.key) != Some(record.sponsor)
                || *base[10].key != record.quote_mint
            {
                return Err(invalid());
            }
            if !record.authorized(a[1].key, generation, current_unix_timestamp()?) {
                return Err(VaultError::Unauthorized.into());
            }
            // This signer bit is private to an authenticated PDA capability.
            // Every Light CPI signs with exactly this PDA's seeds.
            let mut internal = base.to_vec();
            internal[0].is_signer = true;
            ameba_dlmm::process_trading_session_compressed_swap(
                program,
                &internal,
                params,
                record.owner,
            )
        }
        Action::Close { generation, claim } => {
            if a.len() != 3 + 29
                || a[0].is_signer
                || !a[1].is_signer
                || a[2].is_signer
                || a[4].key != a[2].key
            {
                return Err(invalid());
            }
            let record = load(program, &a[2], a[0].key)?;
            let base = &a[3..];
            if *base[0].key != record.sponsor
                || *base[28].key != record.sponsor
                || *base[10].key != record.quote_mint
            {
                return Err(invalid());
            }
            if !record.authorized(a[1].key, generation, current_unix_timestamp()?) {
                return Err(VaultError::Unauthorized.into());
            }
            let mut internal = base.to_vec();
            internal[1].is_signer = true;
            writer_sleeve::process_trading_session_close(
                program,
                &internal,
                claim,
                record.owner,
                true,
            )
        }
        Action::OwnerClose { claim } => {
            if a.len() != 2 + 28 || !a[0].is_signer || a[1].is_signer || a[3].key != a[1].key {
                return Err(invalid());
            }
            let record = load(program, &a[1], a[0].key)?;
            let base = &a[2..];
            if *base[0].key != record.sponsor || *base[10].key != record.quote_mint {
                return Err(invalid());
            }
            let mut internal = base.to_vec();
            internal[1].is_signer = true;
            writer_sleeve::process_trading_session_close(
                program,
                &internal,
                claim,
                record.owner,
                false,
            )
        }
        Action::OwnerTrade { params } => {
            if a.len() < 2 + 42 || !a[0].is_signer || a[1].is_signer || a[2].key != a[1].key {
                return Err(invalid());
            }
            let record = load(program, &a[1], a[0].key)?;
            let base = &a[2..];
            let sponsor_index = 31 + usize::from(params.page_count) + 8;
            if base.get(sponsor_index).map(|i| *i.key) != Some(record.sponsor)
                || *base[10].key != record.quote_mint
            {
                return Err(invalid());
            }
            let mut internal = base.to_vec();
            internal[0].is_signer = true;
            ameba_dlmm::process_trading_session_compressed_swap(
                program,
                &internal,
                params,
                record.owner,
            )
        }
    }
}

fn order(
    program: &Pubkey,
    a: &[AccountInfo],
    action: crate::dlmm_order_state::DlmmOrderAction,
    generation: Option<u64>,
) -> ProgramResult {
    use crate::dlmm_order_state::DlmmOrderAction as O;
    if let O::MultiOrder(action) = action {
        let prefix = if generation.is_some() { 3 } else { 2 };
        if a.len() < prefix + crate::multi_order::COMMON
            || (generation.is_some() && (a[0].is_signer || !a[1].is_signer))
            || (generation.is_none() && !a[0].is_signer)
            || a[prefix - 1].is_signer
        {
            return Err(invalid());
        }
        if generation.is_none() && crate::is_system_program(a[1].key) {
            let base = &a[2..];
            if base[0].key != a[0].key || base[1].key != a[0].key {
                return Err(invalid());
            }
            if let crate::multi_order::Action::Cancel(exit) = action {
                return super::multi_order::process_sponsored_cancel(program, base, exit);
            }
            return Err(invalid());
        }
        let record = load(program, &a[prefix - 1], a[0].key)?;
        let base = &a[prefix..];
        let sponsored =
            generation.is_some() || matches!(action, crate::multi_order::Action::Cancel(_));
        if base[0].key != a[prefix - 1].key
            || *base[5].key != record.quote_mint
            || (sponsored && *base[2].key != record.sponsor)
            || base[1].key
                != if generation.is_some() {
                    a[1].key
                } else {
                    a[0].key
                }
        {
            return Err(invalid());
        }
        if let Some(generation) = generation {
            if !record.authorized(a[1].key, generation, current_unix_timestamp()?) {
                return Err(VaultError::Unauthorized.into());
            }
        }
        if !matches!(
            action,
            crate::multi_order::Action::Place(_) | crate::multi_order::Action::Cancel(_)
        ) {
            return Err(invalid());
        }
        return super::multi_order::process_session(program, base, action, record.owner, sponsored);
    }
    let (records, pages) = match &action {
        O::PlaceCompressedEscrow {
            record_count,
            page_count,
            funding_mode: 0,
            ..
        } => (*record_count, *page_count),
        O::CancelCompressedEscrow { params } | O::ClaimCompressedEscrow { params } => {
            (params.record_count, 0)
        }
        O::SwapCompressedOrders { params } => (params.record_count, params.params.page_count),
        _ => return Err(invalid()),
    };
    let prefix = if generation.is_some() { 3 } else { 2 };
    if a.len() < prefix + 34 + usize::from(records) + usize::from(pages) + 7
        || a[prefix - 1].is_signer
        || a[prefix].key != a[prefix - 1].key
        || (generation.is_some() && (a[0].is_signer || !a[1].is_signer))
        || (generation.is_none() && !a[0].is_signer)
    {
        return Err(invalid());
    }
    let record = load(program, &a[prefix - 1], a[0].key)?;
    let base = &a[prefix..];
    if *base[34 + usize::from(records) + usize::from(pages) + 1].key != record.sponsor
        || *base[10].key != record.quote_mint
    {
        return Err(invalid());
    }
    if let Some(generation) = generation {
        if !record.authorized(a[1].key, generation, current_unix_timestamp()?) {
            return Err(VaultError::Unauthorized.into());
        }
    }
    let mut internal = base.to_vec();
    internal[0].is_signer = true;
    ameba_dlmm::orders::process_with_authority(program, &internal, action, record.owner)
}

/// Enable changes only authority. It works before funding and preserves every
/// existing custody balance on re-enable; expiry uses the actual execution clock.
fn enable(program: &Pubkey, a: &[AccountInfo], grant: session::Grant) -> ProgramResult {
    if a.len() != 6
        || !a[0].is_signer
        || a[0].is_writable
        || !a[1].is_writable
        || a[1].is_signer
        || a[2].is_signer
        || a[2].is_writable
        || a[3].is_signer
        || a[3].is_writable
        || !a[4].is_signer
        || !a[4].is_writable
        || *a[4].key != grant.sponsor
        || a[5].is_signer
        || a[5].is_writable
        || !crate::is_system_program(a[5].key)
        || *a[1].key != session::derive(program, a[0].key).0
    {
        return Err(invalid());
    }
    let config = load_canonical_vault_config(program, &a[2])?;
    if config.usdc_mint != *a[3].key {
        return Err(invalid());
    }
    validate_collateral_mint_account(&a[3], &spl_token_program_id())?;
    let existing = if a[1].owner == program {
        Some(load(program, &a[1], a[0].key)?)
    } else if crate::is_system_program(a[1].owner) && a[1].data_is_empty() && !a[1].executable {
        None
    } else {
        return Err(invalid());
    };
    if existing
        .as_ref()
        .is_some_and(|r| r.quote_mint != *a[3].key || r.sponsor != grant.sponsor)
    {
        return Err(invalid());
    }
    let bump = session::derive(program, a[0].key).1;
    let record = TradingSessionV2::grant(
        *a[0].key,
        *a[3].key,
        bump,
        existing.map_or(0, |r| r.generation),
        grant,
        current_unix_timestamp()?,
    )
    .ok_or(VaultError::InvalidInstructionData)?;
    if record.sponsor == *a[1].key {
        return Err(invalid());
    }
    create_program_account(
        &a[4],
        &a[1],
        &a[5],
        program,
        session::ACCOUNT_SIZE,
        &[session::SEED, a[0].key.as_ref(), &[bump]],
    )?;
    store(&a[1], &record)
}
