//! Rent and proof transport only. Native financial projection seals the payload.
use super::*;
use crate::atomic_projection as cache;
use crate::compact_error::CompactAccountInfo;
use borsh::{BorshDeserialize, BorshSerialize};

// Solana's account-data limit and per-instruction realloc growth, respectively.
const MAX_DATA: usize = 10 * 1024 * 1024;
const GROWTH: usize = 10 * 1024;
fn invalid() -> ProgramError {
    ProgramError::InvalidAccountData
}
pub(super) fn process(program: &Pubkey, a: &[AccountInfo], action: cache::Action) -> ProgramResult {
    if let cache::Action::Project { action } = action {
        return super::writer_sleeve::prepare_atomic_projection(program, a, action);
    }
    if a.len() != cache::COMMON
        || !a[0].is_writable
        || a[0].executable
        || !a[2].is_writable
        || a[2].is_signer
        || a[2].executable
        || !crate::is_system_program(a[3].key)
    {
        return Err(invalid());
    }
    let identity = match &action {
        cache::Action::Reserve { identity, .. }
        | cache::Action::ProofChunk { identity, .. }
        | cache::Action::Reclaim { identity } => identity.clone(),
        cache::Action::Project { .. } => unreachable!(),
    };
    if *a[1].key != identity.order {
        return Err(invalid());
    }
    let (key, bump) = cache::derive(program, a[0].key, &identity);
    if *a[2].key != key {
        return Err(invalid());
    }
    if let cache::Action::Reclaim { .. } = action {
        if a[2].owner != program {
            return Err(invalid());
        }
        let h = cache::decode_header(program, a[2].key, &a[2].try_data()?)?;
        if h.payer != *a[0].key
            || h.identity != identity
            || !a[0].is_signer && h.status != cache::CONSUMED
        {
            return Err(invalid());
        }
        // Rent cleanup is independent of paused/expired/deleted trading sources.
        return close_program_account(program, &a[2], &a[0]);
    }
    if !a[0].is_signer {
        return Err(invalid());
    }
    let order =
        crate::multi_order::Order::try_from_slice(&a[1].try_data()?).map_err(|_| invalid())?;
    super::multi_order::load_readonly(program, &a[1], &order.custody_owner, &order.nonce)?;
    match action {
        cache::Action::Reserve {
            identity,
            proof_count,
            allocation_bytes,
        } => {
            let requested = allocation_bytes as usize;
            let initial = cache::Header {
                magic: cache::MAGIC,
                version: cache::VERSION,
                bump,
                status: cache::RESERVED,
                projector_version: cache::PROJECTOR_VERSION,
                payer: *a[0].key,
                identity,
                proof_count,
                payload_bytes: 0,
                payload_hash: [0; 32],
            };
            if !(cache::HEADER_LEN..=MAX_DATA).contains(&requested)
                || initial.payload_start()? > MAX_DATA
            {
                return Err(invalid());
            }
            if a[2].owner == program {
                let h = cache::decode_header(program, a[2].key, &a[2].try_data()?)?;
                if h.identity != initial.identity
                    || h.payer != initial.payer
                    || h.proof_count != proof_count
                {
                    return Err(invalid());
                }
                if requested <= a[2].data_len() {
                    return Ok(());
                }
                if h.status != cache::RESERVED {
                    return Err(invalid());
                }
                let size = requested.min(a[2].data_len().checked_add(GROWTH).ok_or_else(invalid)?);
                let shortfall = crate::compact_error::rent()?
                    .minimum_balance(size)
                    .saturating_sub(a[2].lamports());
                if shortfall != 0 {
                    invoke_system_transfer(&a[0], &a[2], &a[3], shortfall, &[])?;
                }
                a[2].resize(size)?;
            } else {
                if !crate::is_system_program(a[2].owner) || !a[2].data_is_empty() {
                    return Err(invalid());
                }
                let bump = [bump];
                let id = &initial.identity;
                invoke_create_or_allocate_account(
                    &a[0],
                    &a[2],
                    &a[3],
                    program,
                    requested.min(GROWTH),
                    &[
                        CURRENT_STATE_NAMESPACE_SEED,
                        cache::SEED,
                        a[0].key.as_ref(),
                        id.order.as_ref(),
                        &id.action_hash,
                        &id.source_hash,
                        &id.proofs_hash,
                        &bump,
                    ],
                )?;
                initial
                    .serialize(&mut &mut a[2].try_data_mut()?[..cache::HEADER_LEN])
                    .map_err(|_| invalid())?;
            }
            Ok(())
        }
        cache::Action::ProofChunk {
            identity,
            start,
            proofs,
        } => {
            if a[2].owner != program {
                return Err(invalid());
            }
            let h = cache::decode_header(program, a[2].key, &a[2].try_data()?)?;
            if h.identity != identity
                || h.status != cache::RESERVED
                || proofs.is_empty()
                || (start as usize)
                    .checked_add(proofs.len())
                    .is_none_or(|end| end > h.proof_count as usize)
            {
                return Err(invalid());
            }
            let mut data = a[2].try_data_mut()?;
            for (offset, proof) in proofs.iter().enumerate() {
                let ordinal = start as usize + offset;
                let position = h
                    .proofs_start()?
                    .checked_add(ordinal * 128)
                    .ok_or_else(invalid)?;
                if position.checked_add(128).is_none_or(|end| end > data.len()) {
                    return Err(ProgramError::AccountDataTooSmall);
                }
                let ready = data[cache::HEADER_LEN + ordinal / 8] & (1 << (ordinal % 8)) != 0;
                if ready && data[position..position + 128] != proof[..] {
                    return Err(invalid());
                }
                data[position..position + 128].copy_from_slice(proof);
                data[cache::HEADER_LEN + ordinal / 8] |= 1 << (ordinal % 8);
            }
            Ok(())
        }
        _ => Err(invalid()),
    }
}
