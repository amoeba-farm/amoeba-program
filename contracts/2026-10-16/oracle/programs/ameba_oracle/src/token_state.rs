use solana_program::{program_option::COption, pubkey::Pubkey};

use crate::fixed_codec::FixedCursor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountState {
    Initialized,
    Frozen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenAccount {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub delegate: COption<Pubkey>,
    pub state: AccountState,
    pub is_native: COption<u64>,
    pub delegated_amount: u64,
    pub close_authority: COption<Pubkey>,
}

impl TokenAccount {
    pub const LEN: usize = 165;

    pub fn unpack(data: &[u8]) -> Result<Self, ()> {
        unpack_token_account(data).ok_or(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Mint {
    pub mint_authority: COption<Pubkey>,
    pub supply: u64,
    pub decimals: u8,
    pub is_initialized: bool,
    pub freeze_authority: COption<Pubkey>,
}

impl Mint {
    pub const LEN: usize = 82;

    pub fn unpack(data: &[u8]) -> Result<Self, ()> {
        unpack_mint(data).ok_or(())
    }
}

#[inline(never)]
fn read_optional_pubkey(input: &mut FixedCursor<'_>) -> COption<Pubkey> {
    let tag = input.u32();
    let value = input.pubkey();
    match tag {
        0 => COption::None,
        1 => COption::Some(value),
        _ => {
            input.invalid = true;
            COption::None
        }
    }
}

#[inline(always)]
fn read_optional_u64(input: &mut FixedCursor<'_>) -> COption<u64> {
    let tag = input.u32();
    let value = input.u64();
    match tag {
        0 => COption::None,
        1 => COption::Some(value),
        _ => {
            input.invalid = true;
            COption::None
        }
    }
}

pub(crate) fn unpack_token_account(data: &[u8]) -> Option<TokenAccount> {
    if data.len() != 165 {
        return None;
    }
    let mut input = FixedCursor::new(data);
    let mint = input.pubkey();
    let owner = input.pubkey();
    let amount = input.u64();
    let delegate = read_optional_pubkey(&mut input);
    let state = match input.u8() {
        1 => AccountState::Initialized,
        2 => AccountState::Frozen,
        _ => return None,
    };
    let value = TokenAccount {
        mint,
        owner,
        amount,
        delegate,
        state,
        is_native: read_optional_u64(&mut input),
        delegated_amount: input.u64(),
        close_authority: read_optional_pubkey(&mut input),
    };
    if input.invalid || input.offset != TokenAccount::LEN {
        return None;
    }
    Some(value)
}

pub(crate) fn unpack_mint(data: &[u8]) -> Option<Mint> {
    if data.len() != 82 || data[45] != 1 {
        return None;
    }
    let mut input = FixedCursor::new(data);
    let mint_authority = read_optional_pubkey(&mut input);
    let supply = input.u64();
    let decimals = input.u8();
    let is_initialized = input.bool();
    let freeze_authority = read_optional_pubkey(&mut input);
    if input.invalid || input.offset != Mint::LEN || !is_initialized {
        return None;
    }
    Some(Mint {
        mint_authority,
        supply,
        decimals,
        is_initialized,
        freeze_authority,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::program_pack::Pack;
    use spl_token::state::{
        Account as SplTokenAccount, AccountState as SplAccountState, Mint as SplMint,
    };

    fn assert_mint_acceptance_matches_spl(data: &[u8]) {
        assert_eq!(
            Mint::unpack(data).is_ok(),
            <SplMint as Pack>::unpack(data).is_ok()
        );
    }

    fn assert_token_acceptance_matches_spl(data: &[u8]) {
        assert_eq!(
            TokenAccount::unpack(data).is_ok(),
            <SplTokenAccount as Pack>::unpack(data).is_ok()
        );
    }

    #[test]
    fn fixed_mint_codec_matches_spl_pack() {
        let authority = Pubkey::new_unique();
        let freeze = Pubkey::new_unique();
        for mint_authority in [COption::None, COption::Some(authority)] {
            for freeze_authority in [COption::None, COption::Some(freeze)] {
                let expected = SplMint {
                    mint_authority,
                    supply: 91,
                    decimals: 6,
                    is_initialized: true,
                    freeze_authority,
                };
                let mut data = [0_u8; SplMint::LEN];
                SplMint::pack(expected, &mut data).unwrap();
                let decoded = Mint::unpack(&data).unwrap();
                assert_eq!(decoded.mint_authority, mint_authority);
                assert_eq!(decoded.supply, 91);
                assert_eq!(decoded.decimals, 6);
                assert!(decoded.is_initialized);
                assert_eq!(decoded.freeze_authority, freeze_authority);
                assert_mint_acceptance_matches_spl(&data);
            }
        }

        let mut data = [0_u8; SplMint::LEN];
        SplMint::pack(
            SplMint {
                mint_authority: COption::None,
                supply: 91,
                decimals: 6,
                is_initialized: false,
                freeze_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        assert_mint_acceptance_matches_spl(&data);
        assert_mint_acceptance_matches_spl(&data[..data.len() - 1]);
        let mut oversized = data.to_vec();
        oversized.push(0);
        assert_mint_acceptance_matches_spl(&oversized);
        for tag_offset in [0, 46] {
            data[45] = 1;
            data[tag_offset..tag_offset + 4].copy_from_slice(&2_u32.to_le_bytes());
            assert_mint_acceptance_matches_spl(&data);
            data[tag_offset..tag_offset + 4].copy_from_slice(&0_u32.to_le_bytes());
        }
        data[45] = 2;
        assert_mint_acceptance_matches_spl(&data);
    }

    #[test]
    fn fixed_token_codec_matches_spl_pack() {
        let mint = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let delegate = Pubkey::new_unique();
        let close_authority = Pubkey::new_unique();
        for delegate_option in [COption::None, COption::Some(delegate)] {
            for state in [
                (SplAccountState::Initialized, AccountState::Initialized),
                (SplAccountState::Frozen, AccountState::Frozen),
            ] {
                for is_native in [COption::None, COption::Some(11)] {
                    for close_option in [COption::None, COption::Some(close_authority)] {
                        let expected = SplTokenAccount {
                            mint,
                            owner,
                            amount: 73,
                            delegate: delegate_option,
                            state: state.0,
                            is_native,
                            delegated_amount: 7,
                            close_authority: close_option,
                        };
                        let mut data = [0_u8; SplTokenAccount::LEN];
                        SplTokenAccount::pack(expected, &mut data).unwrap();
                        let decoded = TokenAccount::unpack(&data).unwrap();
                        assert_eq!(decoded.mint, mint);
                        assert_eq!(decoded.owner, owner);
                        assert_eq!(decoded.amount, 73);
                        assert_eq!(decoded.delegate, delegate_option);
                        assert_eq!(decoded.state, state.1);
                        assert_eq!(decoded.is_native, is_native);
                        assert_eq!(decoded.delegated_amount, 7);
                        assert_eq!(decoded.close_authority, close_option);
                        assert_token_acceptance_matches_spl(&data);
                    }
                }
            }
        }

        let mut data = [0_u8; SplTokenAccount::LEN];
        SplTokenAccount::pack(
            SplTokenAccount {
                mint,
                owner,
                amount: 73,
                delegate: COption::None,
                state: SplAccountState::Uninitialized,
                is_native: COption::None,
                delegated_amount: 7,
                close_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        assert_token_acceptance_matches_spl(&data);
        assert_token_acceptance_matches_spl(&data[..data.len() - 1]);
        let mut oversized = data.to_vec();
        oversized.push(0);
        assert_token_acceptance_matches_spl(&oversized);
        data[108] = 3;
        assert_token_acceptance_matches_spl(&data);
        data[108] = 1;
        for tag_offset in [72, 109, 129] {
            data[tag_offset..tag_offset + 4].copy_from_slice(&2_u32.to_le_bytes());
            assert_token_acceptance_matches_spl(&data);
            data[tag_offset..tag_offset + 4].copy_from_slice(&0_u32.to_le_bytes());
        }
    }
}
