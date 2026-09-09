//! Internal token delegation for owner-only settlement destinations.
use super::*;
use crate::scoped_settlement::derive_collective_settlement_delegate;

/// User token accounts may carry only this program's exact wallet/mint capability.
/// Vaults and other protocol custody retain the ordinary no-delegate validator.
pub(super) fn load_scoped_holder_token_account(
    program_id: &Pubkey,
    account: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<TokenAccount, ProgramError> {
    validate_light_token_account(account)?;
    validate_light_associated_token_address(owner, mint, account)?;
    let data = account.try_borrow_data()?;
    if !has_canonical_compressible_token_layout(&data) {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    let token = TokenAccount::unpack(&data[..TokenAccount::LEN])
        .map_err(|_| VaultError::InvalidLightTokenAccount)?;
    let delegation_ok = match token.delegate {
        COption::None => token.delegated_amount == 0,
        COption::Some(delegate) => {
            delegate == derive_collective_settlement_delegate(program_id, owner, mint).0
        }
    };
    if token.owner != *owner
        || token.mint != *mint
        || token.state != AccountState::Initialized
        || !delegation_ok
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(token)
}

/// Appended to the original owner-signed trade/bid. Future fills need no new signature.
pub(super) fn authorize_collective_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    revoke: bool,
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner = &accounts[0];
    let market = load_valid_market(program_id, &accounts[1])?;
    let mint = &accounts[2];
    let source = &accounts[3];
    let delegate = &accounts[4];
    let light = &accounts[5];
    if !owner.is_signer || !owner.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if market.long_contract_mint != Some(*mint.key)
        || *delegate.key != derive_collective_settlement_delegate(program_id, owner.key, mint.key).0
        || *light.key != light_token_program_id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_collateral_mint_account(mint, &spl_token_program_id())?;
    if !revoke && source.owner == &system_program::id() {
        load_or_create_light_associated_token_account(
            owner,
            owner,
            mint,
            source,
            light,
            &accounts[6],
            &accounts[7],
            &accounts[8],
        )?;
    }
    let _ = load_scoped_holder_token_account(program_id, source, owner.key, mint.key)?;
    let instruction = if revoke {
        light_token_instruction::revoke(source.key, owner.key)
    } else {
        light_token_instruction::approve(source.key, delegate.key, owner.key, u64::MAX)
    };
    invoke(
        &instruction,
        &[
            source.clone(),
            delegate.clone(),
            owner.clone(),
            light.clone(),
        ],
    )?;
    let token = load_scoped_holder_token_account(program_id, source, owner.key, mint.key)?;
    if (revoke && token.delegate != COption::None)
        || (!revoke
            && (token.delegate != COption::Some(*delegate.key)
                || token.delegated_amount != u64::MAX))
    {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(())
}
