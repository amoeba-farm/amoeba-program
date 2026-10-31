//! AMBA custody for the heads family: the mint and vault checks, and the token CPIs the handlers
//! share.
//!
//! AMBA lives in one vault: the classic or Token-2022 associated token account of the
//! `heads-authority` PDA for the configured mint and token program. The client creates it with
//! the associated-token program; the program validates it and never creates it. Stake moves in
//! by `TransferChecked` (what arrives is credited, a Token-2022 transfer fee off), out by
//! `TransferChecked` signed by the authority, and slashes burn by `BurnChecked`. Burned USDC
//! (losing challengers' bonds when the claim falls anyway) burns from the classic bond vault,
//! signed by the config PDA.

use super::*;
use crate::associated_token::get_associated_token_address_with_program_id;
use crate::compact_error::CompactAccountInfo;
use crate::state::heads::{heads_authority_address, HeadsConfig, HEADS_AUTHORITY_SEED};
use crate::token_state::token2022::{
    unpack_mint_any, unpack_token_account_any, ExtendedMint, ExtendedTokenAccount,
};

fn bad() -> ProgramError {
    OracleError::InvalidHeadsTokenAccount.into()
}

/// Byte range of a token account's amount (the base layout, classic and Token-2022 alike).
const AMOUNT: core::ops::Range<usize> = 64..72;

/// The AMBA mint: owned by `token_program` (classic SPL Token or Token-2022), initialized, and
/// with no extension that lets anyone but the vault's owner move or freeze its balance
/// (`ExtendedMint::is_custody_safe`).
pub(super) fn validate_amba_mint(
    mint_info: &AccountInfo,
    token_program: &Pubkey,
) -> Result<ExtendedMint, ProgramError> {
    if !token_instruction::is_token_program(token_program)
        || mint_info.owner != token_program
        || mint_info.executable
    {
        return Err(bad());
    }
    let mint = unpack_mint_any(token_program, &mint_info.try_data()?).ok_or_else(bad)?;
    if !mint.is_custody_safe() {
        return Err(bad());
    }
    Ok(mint)
}

/// The AMBA vault: exactly `ATA(authority, mint, token_program)`, owned by that token program, an
/// initialized (not frozen, not native) account of `mint` owned by `authority`, with no delegate
/// and no close authority.
pub(super) fn validate_amba_vault(
    vault_info: &AccountInfo,
    mint: &Pubkey,
    token_program: &Pubkey,
    authority: &Pubkey,
) -> Result<ExtendedTokenAccount, ProgramError> {
    let expected = get_associated_token_address_with_program_id(authority, mint, token_program);
    if !crate::pubkey_eq(vault_info.key, &expected)
        || vault_info.owner != token_program
        || vault_info.executable
    {
        return Err(bad());
    }
    let account =
        unpack_token_account_any(token_program, &vault_info.try_data()?).ok_or_else(bad)?;
    let base = &account.base;
    if base.mint != *mint
        || base.owner != *authority
        || base.delegate.is_some()
        || base.close_authority.is_some()
        || base.state != AccountState::Initialized
        || base.is_native.is_some()
    {
        return Err(bad());
    }
    Ok(account)
}

/// The custody accounts a stored config names: its token program, mint and vault, the heads
/// authority at its stored bump (when the instruction passes it), and the vault's state.
pub(super) fn validate_amba_custody(
    program_id: &Pubkey,
    config: &HeadsConfig,
    token_program_info: &AccountInfo,
    mint_info: &AccountInfo,
    vault_info: &AccountInfo,
    authority_info: Option<&AccountInfo>,
) -> Result<ExtendedTokenAccount, ProgramError> {
    let authority = heads_authority_address(program_id, config.authority_bump).ok_or_else(bad)?;
    if !crate::pubkey_eq(token_program_info.key, &config.amba_token_program)
        || !crate::pubkey_eq(mint_info.key, &config.amba_mint)
        || !crate::pubkey_eq(vault_info.key, &config.amba_vault)
        || mint_info.owner != &config.amba_token_program
        || authority_info.is_some_and(|info| !crate::pubkey_eq(info.key, &authority))
    {
        return Err(bad());
    }
    validate_amba_vault(
        vault_info,
        &config.amba_mint,
        &config.amba_token_program,
        &authority,
    )
}

/// A token account's amount.
pub(super) fn token_amount(info: &AccountInfo) -> Result<u64, ProgramError> {
    let data = info.try_data()?;
    let bytes: [u8; 8] = data
        .get(AMOUNT)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(bad)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Moves `amount` from the owner's token account into the vault (the owner signs the
/// transaction) and returns what arrived: a Token-2022 transfer fee is withheld in the vault and
/// never credited.
#[allow(clippy::too_many_arguments)]
pub(super) fn transfer_into_vault<'a>(
    token_program_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    vault_info: &AccountInfo<'a>,
    owner_info: &AccountInfo<'a>,
    amount: u64,
    decimals: u8,
) -> Result<u64, ProgramError> {
    let before = token_amount(vault_info)?;
    let instruction = token_instruction::transfer_checked_any(
        token_program_info.key,
        source_info.key,
        mint_info.key,
        vault_info.key,
        owner_info.key,
        amount,
        decimals,
    )?;
    invoke(
        &instruction,
        &[
            source_info.clone(),
            mint_info.clone(),
            vault_info.clone(),
            owner_info.clone(),
            token_program_info.clone(),
        ],
    )?;
    token_amount(vault_info)?
        .checked_sub(before)
        .ok_or_else(|| OracleError::ArithmeticOverflow.into())
}

/// Moves `amount` out of the vault, signed by the heads authority at `authority_bump`.
#[allow(clippy::too_many_arguments)]
pub(super) fn transfer_from_vault<'a>(
    token_program_info: &AccountInfo<'a>,
    vault_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    authority_bump: u8,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    let instruction = token_instruction::transfer_checked_any(
        token_program_info.key,
        vault_info.key,
        mint_info.key,
        destination_info.key,
        authority_info.key,
        amount,
        decimals,
    )?;
    invoke_signed(
        &instruction,
        &[
            vault_info.clone(),
            mint_info.clone(),
            destination_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        &[&[
            CURRENT_STATE_NAMESPACE_SEED,
            HEADS_AUTHORITY_SEED,
            &[authority_bump],
        ]],
    )
}

/// Burns `amount` of AMBA from the vault, signed by the heads authority at `authority_bump`.
pub(super) fn burn_from_vault<'a>(
    token_program_info: &AccountInfo<'a>,
    vault_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    authority_bump: u8,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    let instruction = token_instruction::burn_checked(
        token_program_info.key,
        vault_info.key,
        mint_info.key,
        authority_info.key,
        amount,
        decimals,
    )?;
    invoke_signed(
        &instruction,
        &[
            vault_info.clone(),
            mint_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        &[&[
            CURRENT_STATE_NAMESPACE_SEED,
            HEADS_AUTHORITY_SEED,
            &[authority_bump],
        ]],
    )
}

/// Burns `amount` of USDC from the classic bond vault, signed by the config PDA at
/// `config_bump`. The burned bonds were already off every `UserCollateral` ledger.
pub(super) fn burn_from_bond_vault<'a>(
    token_program_info: &AccountInfo<'a>,
    bond_vault_info: &AccountInfo<'a>,
    usdc_mint_info: &AccountInfo<'a>,
    config_info: &AccountInfo<'a>,
    config_bump: u8,
    amount: u64,
) -> ProgramResult {
    if !token_instruction::check_id(token_program_info.key) {
        return Err(OracleError::InvalidTokenProgram.into());
    }
    let instruction = token_instruction::burn_checked(
        token_program_info.key,
        bond_vault_info.key,
        usdc_mint_info.key,
        config_info.key,
        amount,
        COLLATERAL_MINT_DECIMALS,
    )?;
    invoke_signed(
        &instruction,
        &[
            bond_vault_info.clone(),
            usdc_mint_info.clone(),
            config_info.clone(),
            token_program_info.clone(),
        ],
        &[&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED, &[config_bump]]],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::heads::{find_heads_amba_vault_address, find_heads_authority_address};
    use crate::token_instruction::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
    use solana_program::program_pack::Pack;
    use spl_token::state::{Account as SplAccount, AccountState as SplState, Mint as SplMint};

    fn mint_bytes(extensions: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; SplMint::LEN];
        SplMint::pack(
            SplMint {
                mint_authority: COption::Some(Pubkey::new_unique()),
                supply: 10,
                decimals: 9,
                is_initialized: true,
                freeze_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        if !extensions.is_empty() {
            data.resize(165, 0);
            data.push(1);
            data.extend_from_slice(extensions);
        }
        data
    }

    fn vault_bytes(mint: Pubkey, owner: Pubkey, edit: impl FnOnce(&mut SplAccount)) -> Vec<u8> {
        let mut account = SplAccount {
            mint,
            owner,
            amount: 1_000,
            delegate: COption::None,
            state: SplState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        };
        edit(&mut account);
        let mut data = vec![0u8; SplAccount::LEN];
        SplAccount::pack(account, &mut data).unwrap();
        // A Token-2022 associated account carries the immutable-owner extension.
        data.push(2);
        data.extend_from_slice(&[7, 0, 0, 0]);
        data
    }

    fn check_vault(
        program_id: &Pubkey,
        key: Pubkey,
        owner: Pubkey,
        mut data: Vec<u8>,
        mint: &Pubkey,
    ) -> Result<ExtendedTokenAccount, ProgramError> {
        let mut lamports = 1;
        let info = AccountInfo::new(
            &key,
            false,
            true,
            &mut lamports,
            &mut data,
            &owner,
            false,
            0,
        );
        let authority = find_heads_authority_address(program_id).0;
        validate_amba_vault(&info, mint, &TOKEN_2022_PROGRAM_ID, &authority)
    }

    #[test]
    fn the_vault_is_the_authority_ata_with_no_delegate_or_close_authority() {
        let program_id = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let authority = find_heads_authority_address(&program_id).0;
        let vault = find_heads_amba_vault_address(&program_id, &mint, &TOKEN_2022_PROGRAM_ID);
        let good = vault_bytes(mint, authority, |_| {});
        let parsed = check_vault(
            &program_id,
            vault,
            TOKEN_2022_PROGRAM_ID,
            good.clone(),
            &mint,
        )
        .unwrap();
        assert_eq!(parsed.base.amount, 1_000);
        assert_eq!(parsed.extensions, vec![7]);

        let invalid = Err(ProgramError::from(OracleError::InvalidHeadsTokenAccount));
        // Another address, another token program owner, another owner or mint.
        assert_eq!(
            check_vault(
                &program_id,
                Pubkey::new_unique(),
                TOKEN_2022_PROGRAM_ID,
                good.clone(),
                &mint
            ),
            invalid
        );
        assert_eq!(
            check_vault(&program_id, vault, TOKEN_PROGRAM_ID, good.clone(), &mint),
            invalid
        );
        let stranger = vault_bytes(mint, Pubkey::new_unique(), |_| {});
        assert_eq!(
            check_vault(&program_id, vault, TOKEN_2022_PROGRAM_ID, stranger, &mint),
            invalid
        );
        let other_mint = vault_bytes(Pubkey::new_unique(), authority, |_| {});
        assert_eq!(
            check_vault(&program_id, vault, TOKEN_2022_PROGRAM_ID, other_mint, &mint),
            invalid
        );
        // A delegate, a close authority, a frozen or native account.
        for edit in [
            (|a: &mut SplAccount| {
                a.delegate = COption::Some(Pubkey::new_unique());
                a.delegated_amount = 1;
            }) as fn(&mut SplAccount),
            |a| a.close_authority = COption::Some(Pubkey::new_unique()),
            |a| a.state = SplState::Frozen,
            |a| a.is_native = COption::Some(1),
        ] {
            let data = vault_bytes(mint, authority, edit);
            assert_eq!(
                check_vault(&program_id, vault, TOKEN_2022_PROGRAM_ID, data, &mint),
                invalid
            );
        }
        let mut lamports = 1;
        let mut data = good;
        let info = AccountInfo::new(
            &vault,
            false,
            true,
            &mut lamports,
            &mut data,
            &TOKEN_2022_PROGRAM_ID,
            false,
            0,
        );
        assert_eq!(token_amount(&info), Ok(1_000));
    }

    #[test]
    fn the_mint_must_be_a_custody_safe_mint_of_its_token_program() {
        let key = Pubkey::new_unique();
        let check = |owner: Pubkey, program: Pubkey, mut data: Vec<u8>| {
            let mut lamports = 1;
            let info = AccountInfo::new(
                &key,
                false,
                false,
                &mut lamports,
                &mut data,
                &owner,
                false,
                0,
            );
            validate_amba_mint(&info, &program).map(|m| m.base.decimals)
        };
        // A transfer fee (25 bp, as AMBA on dev) is fine.
        let mut fee = vec![1u8, 0, 108, 0];
        fee.extend_from_slice(&[0u8; 108]);
        assert_eq!(
            check(
                TOKEN_2022_PROGRAM_ID,
                TOKEN_2022_PROGRAM_ID,
                mint_bytes(&fee)
            ),
            Ok(9)
        );
        assert_eq!(
            check(TOKEN_PROGRAM_ID, TOKEN_PROGRAM_ID, mint_bytes(&[])),
            Ok(9)
        );
        let invalid = Err(ProgramError::from(OracleError::InvalidHeadsTokenAccount));
        // A permanent delegate could drain the vault.
        let mut delegate = vec![12u8, 0, 32, 0];
        delegate.extend_from_slice(&[9u8; 32]);
        assert_eq!(
            check(
                TOKEN_2022_PROGRAM_ID,
                TOKEN_2022_PROGRAM_ID,
                mint_bytes(&delegate)
            ),
            invalid
        );
        // Owner and program must agree, and be a token program.
        assert_eq!(
            check(TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, mint_bytes(&[])),
            invalid
        );
        let other = Pubkey::new_unique();
        assert_eq!(check(other, other, mint_bytes(&[])), invalid);
    }
}
