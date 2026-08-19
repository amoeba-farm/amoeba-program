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
