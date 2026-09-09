//! One-position cleanup permission, captured by the original owner-signed LP operation.
use super::*;
use crate::scoped_settlement::{
    derive_position_settlement_authority, POSITION_SETTLEMENT_AUTHORITY_DISCRIMINATOR as MAGIC,
    POSITION_SETTLEMENT_AUTHORITY_LEN as LEN, POSITION_SETTLEMENT_AUTHORITY_SEED as SEED,
};

/// Constructed only after checking the owner-created marker and final pool settlement.
pub(super) struct ScopedCleanup<'a, 'info> {
    keeper: &'a AccountInfo<'info>,
    owner: Pubkey,
    position: Pubkey,
    pool: Pubkey,
}
impl<'a, 'info> ScopedCleanup<'a, 'info> {
    pub(super) fn payer(&self) -> &'a AccountInfo<'info> {
        self.keeper
    }
    pub(super) fn require_binding(
        &self,
        owner: &Pubkey,
        pool: &Pubkey,
        position: &Pubkey,
    ) -> ProgramResult {
        if self.owner != *owner || self.pool != *pool || self.position != *position {
            return Err(VaultError::InvalidAmoebaDlmmPosition.into());
        }
        Ok(())
    }
}

fn marker_bytes(owner: &Pubkey, pool: &Pubkey, position: &Pubkey, bump: u8) -> [u8; LEN] {
    let mut bytes = [0; LEN];
    bytes[..8].copy_from_slice(&MAGIC);
    bytes[8] = bump;
    bytes[9..41].copy_from_slice(owner.as_ref());
    bytes[41..73].copy_from_slice(pool.as_ref());
    bytes[73..105].copy_from_slice(position.as_ref());
    bytes
}

fn validate_marker(
    program_id: &Pubkey,
    marker: &AccountInfo,
    owner: &Pubkey,
    pool: &Pubkey,
    position: &Pubkey,
) -> ProgramResult {
    let (address, bump) = derive_position_settlement_authority(program_id, owner, position);
    if *marker.key != address
        || marker.owner != program_id
        || marker.executable
        || marker.data_len() != LEN
        || *marker.try_borrow_data()? != marker_bytes(owner, pool, position, bump).as_slice()
    {
        return Err(VaultError::InvalidAmoebaDlmmPosition.into());
    }
    Ok(())
}

pub(in crate::processor) fn process_scoped_position_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    action: u8,
) -> ProgramResult {
    if action == 0 || action == 2 {
        if accounts.len() != 5 {
            return Err(VaultError::InvalidAccountList.into());
        }
        let owner = &accounts[0];
        let pool_info = &accounts[1];
        let position_info = &accounts[2];
        let marker = &accounts[3];
        let pool = load_pool(program_id, pool_info)?;
        let position = load_position(program_id, pool_info.key, position_info)?;
        validate_position_owner_and_nonce(owner, &pool, &position, position.position_nonce)?;
        if !owner.is_writable || !marker.is_writable {
            return Err(VaultError::InvalidAccountList.into());
        }
        let (address, bump) =
            derive_position_settlement_authority(program_id, owner.key, position_info.key);
        if *marker.key != address {
            return Err(VaultError::InvalidPda.into());
        }
        if action == 2 {
            validate_marker(
                program_id,
                marker,
                owner.key,
                pool_info.key,
                position_info.key,
            )?;
            return close_program_account(program_id, marker, owner);
        }
        if marker.owner == program_id {
            return validate_marker(
                program_id,
                marker,
                owner.key,
                pool_info.key,
                position_info.key,
            );
        }
        validate_create_only_program_account_target(program_id, marker)?;
        create_program_account(
            owner,
            marker,
            &accounts[4],
            program_id,
            LEN,
            &[
                SEED,
                owner.key.as_ref(),
                position_info.key.as_ref(),
                &[bump],
            ],
        )?;
        marker.try_borrow_mut_data()?.copy_from_slice(&marker_bytes(
            owner.key,
            pool_info.key,
            position_info.key,
            bump,
        ));
        return Ok(());
    }
    // The ordinary liquidity account prefix/page pairs, followed by permission and fee payer.
    if action != 1 || accounts.len() < 18 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let end = accounts.len() - 2;
    let marker = &accounts[end];
    let keeper = &accounts[end + 1];
    let owner = &accounts[0];
    let pool_info = &accounts[1];
    let position_info = &accounts[2];
    if !keeper.is_signer || !keeper.is_writable || !owner.is_writable || !marker.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut pool = load_pool(program_id, pool_info)?;
    let mut position = load_position(program_id, pool_info.key, position_info)?;
    validate_marker(
        program_id,
        marker,
        owner.key,
        pool_info.key,
        position_info.key,
    )?;
    if pool.status != AmoebaDlmmPoolStatus::Settled
        || current_unix_timestamp()? < pool.expiry_ts
        || position.owner != *owner.key
        || pool.liquidity_manager != *owner.key
    {
        return Err(VaultError::InvalidAmoebaDlmmStatusTransition.into());
    }
    if position.is_empty() {
        if end != 16 || !pool_info.is_writable || !position_info.is_writable {
            return Err(VaultError::InvalidAccountList.into());
        }
        position.is_initialized = false;
        pool.position_count = pool
            .position_count
            .checked_sub(1)
            .ok_or(VaultError::AmoebaDlmmInvariantViolation)?;
        position.last_updated_slot = Clock::get()?.slot;
        pool.last_updated_slot = position.last_updated_slot;
        store_light_state(position_info, &position)?;
        store_light_state(pool_info, &pool)?;
    } else {
        let permit = ScopedCleanup {
            keeper,
            owner: *owner.key,
            pool: *pool_info.key,
            position: *position_info.key,
        };
        let mut entries = Vec::new();
        for index in 0..usize::from(position.bin_count) {
            let shares = position.liquidity_shares[index];
            if shares != 0 {
                entries.push(crate::ameba_dlmm_instruction::AmoebaDlmmRemoveEntry {
                    bin_id: position
                        .lower_bin_id
                        .checked_add(index as u16)
                        .ok_or(VaultError::ArithmeticOverflow)?,
                    shares,
                    minimum_option_out: 0,
                    minimum_quote_out: 0,
                });
            }
        }
        process_scoped_liquidity_cleanup(
            program_id,
            &accounts[..end],
            RemoveAmoebaDlmmLiquidityV1Params {
                position_nonce: position.position_nonce,
                entries,
                close_position_when_empty: true,
            },
            &permit,
        )?;
    }
    close_program_account(program_id, marker, owner)
}
