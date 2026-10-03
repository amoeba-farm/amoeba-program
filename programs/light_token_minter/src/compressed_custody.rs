//! Accounted regular compressed-token custody for an existing DLMM pool or order book.
//! The sidecar is an inventory ledger, not a second price or obligation ledger.
use crate::ProgramError;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

use crate::constants::CURRENT_STATE_NAMESPACE_SEED;

pub const COMPRESSED_CUSTODY_SEED: &[u8] = b"dlmm-compressed-custody";
pub const COMPRESSED_CUSTODY_VERSION: u8 = 1;
pub const COMPRESSED_CUSTODY_DISCRIMINATOR: [u8; 8] = *b"DLMCCST1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
#[repr(u8)]
pub enum CustodyKind {
    Pool = 0,
    OrderBook = 1,
    WriterCash = 2,
}

/// The derived Borsh tag is the declaration index (equal to the discriminants above).
impl crate::fixed_codec::CursorField for CustodyKind {
    #[inline(never)]
    fn read(input: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
        match input.u8() {
            0 => Self::Pool,
            1 => Self::OrderBook,
            2 => Self::WriterCash,
            _ => {
                input.invalid = true;
                Self::Pool
            }
        }
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct CompressedCustodyV1 {
        pub version: u8,
        pub bump: u8,
        pub kind: CustodyKind,
        pub parent: Pubkey,
        pub option_mint: Pubkey,
        pub quote_mint: Pubkey,
        pub option_atoms: u64,
        pub quote_atoms: u64,
    }
}

impl CompressedCustodyV1 {
    pub const LEN: usize = 3 + 32 * 3 + 8 * 2;
    pub const ACCOUNT_LEN: usize = 8 + Self::LEN;

    pub fn new(
        kind: CustodyKind,
        parent: Pubkey,
        option_mint: Pubkey,
        quote_mint: Pubkey,
        bump: u8,
    ) -> Self {
        Self {
            version: COMPRESSED_CUSTODY_VERSION,
            bump,
            kind,
            parent,
            option_mint,
            quote_mint,
            option_atoms: 0,
            quote_atoms: 0,
        }
    }

    pub fn matches(
        &self,
        kind: CustodyKind,
        parent: &Pubkey,
        option_mint: &Pubkey,
        quote_mint: &Pubkey,
        bump: u8,
    ) -> bool {
        self.version == COMPRESSED_CUSTODY_VERSION
            && self.bump == bump
            && self.kind == kind
            && self.parent == *parent
            && self.option_mint == *option_mint
            && self.quote_mint == *quote_mint
    }

    pub fn backs(
        &self,
        hot_option: u64,
        hot_quote: u64,
        option_obligation: u64,
        quote_obligation: u64,
    ) -> bool {
        hot_option
            .checked_add(self.option_atoms)
            .is_some_and(|total| total >= option_obligation)
            && hot_quote
                .checked_add(self.quote_atoms)
                .is_some_and(|total| total >= quote_obligation)
    }
}

/// Missing sidecars are the legacy hot-only mode. A present account must be the
/// exact program-owned PDA for the requested pool or book; arbitrary sidecar
/// bytes must never be accepted as reserve backing.
pub fn load(
    program: &Pubkey,
    account: Option<&AccountInfo>,
    kind: CustodyKind,
    parent: &Pubkey,
    option_mint: &Pubkey,
    quote_mint: &Pubkey,
) -> Result<Option<CompressedCustodyV1>, ProgramError> {
    let Some(account) = account else {
        return Ok(None);
    };
    let (expected, bump) = derive_compressed_custody(program, kind, parent);
    if account.key != &expected
        || account.owner != program
        || account.data_len() != CompressedCustodyV1::ACCOUNT_LEN
        || !account.is_writable
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let data = account.try_borrow_data()?;
    if data[..8] != COMPRESSED_CUSTODY_DISCRIMINATOR {
        return Err(ProgramError::InvalidAccountData);
    }
    let state = CompressedCustodyV1::try_from_slice(&data[8..])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if !state.matches(kind, parent, option_mint, quote_mint, bump) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(Some(state))
}

pub fn store(account: &AccountInfo, state: &CompressedCustodyV1) -> Result<(), ProgramError> {
    if !account.is_writable || account.data_len() != CompressedCustodyV1::ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut data = account.try_borrow_mut_data()?;
    data[..8].copy_from_slice(&COMPRESSED_CUSTODY_DISCRIMINATOR);
    state
        .serialize(&mut &mut data[8..])
        .map_err(|_| ProgramError::InvalidAccountData)
}

pub fn backs(
    sidecar: Option<&CompressedCustodyV1>,
    hot_option: u64,
    hot_quote: u64,
    option_obligation: u64,
    quote_obligation: u64,
) -> bool {
    match sidecar {
        Some(sidecar) => sidecar.backs(hot_option, hot_quote, option_obligation, quote_obligation),
        None => hot_option >= option_obligation && hot_quote >= quote_obligation,
    }
}

pub fn derive_compressed_custody(
    program: &Pubkey,
    kind: CustodyKind,
    parent: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            COMPRESSED_CUSTODY_SEED,
            &[kind as u8],
            parent.as_ref(),
        ],
        program,
    )
}
