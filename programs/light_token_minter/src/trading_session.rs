//! Wallet-bound custody and bounded Ed25519 trading authority. Custody never
//! changes owner when a grant expires, is revoked, or is replaced.
use crate::{
    ameba_dlmm_instruction::SwapCollectiveCompressedExactInV1Params,
    compressed_option_settlement::CompressedCashOptionClaim,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const TAG: u8 = 32;
pub const SEED: &[u8] = b"trading-session-v1";
pub const MAGIC: [u8; 8] = *b"TRDSESV2";
pub const DURATION_SECONDS: u64 = 90 * 24 * 60 * 60;
pub const SPONSOR_FEE_ATOMS: u64 = 10_000;
pub const ACCOUNT_SIZE: usize = 155;

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct TradingSessionV2 {
        pub magic: [u8; 8],
        pub version: u8,
        pub bump: u8,
        pub owner: Pubkey,
        pub session: Pubkey,
        pub quote_mint: Pubkey,
        pub sponsor: Pubkey,
        pub generation: u64,
        pub expires_at: u64,
        pub revoked: bool,
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Grant {
        pub session: Pubkey,
        pub sponsor: Pubkey,
    }
}

crate::fixed_codec::compact_borsh_struct! {
    /// Wallet/PDA USDC leaf movement. Both destinations are constructed by the
    /// handler; the caller supplies no recipient or fee field.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Funding {
        pub input_amount: u64,
        pub amount: u64,
        pub leaf_index: u32,
        pub root_index: u16,
        pub prove_by_index: bool,
        pub proof: Option<[u8; 128]>,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    TradeStrip {
        generation: u64,
        params: crate::capped_strip::Trade,
    },
    OwnerTradeStrip {
        params: crate::capped_strip::Trade,
    },
    Enable(Grant),
    Revoke,
    Deposit(Funding),
    Withdraw(Funding),
    Trade {
        generation: u64,
        params: SwapCollectiveCompressedExactInV1Params,
    },
    Close {
        generation: u64,
        claim: CompressedCashOptionClaim,
    },
    OwnerClose {
        claim: CompressedCashOptionClaim,
    },
    OwnerTrade {
        params: SwapCollectiveCompressedExactInV1Params,
    },
    DepositClassic {
        amount: u64,
    },
}
// Explicit selectors preserve existing owner funding routes while retiring 8.
impl BorshSerialize for Action {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Self::TradeStrip { generation, params } => {
                10u8.serialize(writer)?;
                generation.serialize(writer)?;
                params.serialize(writer)
            }
            Self::OwnerTradeStrip { params } => {
                11u8.serialize(writer)?;
                params.serialize(writer)
            }
            Self::Enable(g) => {
                0u8.serialize(writer)?;
                g.serialize(writer)
            }
            Self::Revoke => 1u8.serialize(writer),
            Self::Deposit(f) => {
                2u8.serialize(writer)?;
                f.serialize(writer)
            }
            Self::Withdraw(f) => {
                3u8.serialize(writer)?;
                f.serialize(writer)
            }
            Self::Trade { generation, params } => {
                4u8.serialize(writer)?;
                generation.serialize(writer)?;
                params.serialize(writer)
            }
            Self::Close { generation, claim } => {
                5u8.serialize(writer)?;
                generation.serialize(writer)?;
                claim.serialize(writer)
            }
            Self::OwnerClose { claim } => {
                6u8.serialize(writer)?;
                claim.serialize(writer)
            }
            Self::OwnerTrade { params } => {
                7u8.serialize(writer)?;
                params.serialize(writer)
            }
            Self::DepositClassic { amount } => {
                9u8.serialize(writer)?;
                amount.serialize(writer)
            }
        }
    }
}
impl BorshDeserialize for Action {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            10 => Self::TradeStrip {
                generation: u64::deserialize_reader(reader)?,
                params: crate::capped_strip::Trade::deserialize_reader(reader)?,
            },
            11 => Self::OwnerTradeStrip {
                params: crate::capped_strip::Trade::deserialize_reader(reader)?,
            },
            0 => Self::Enable(Grant::deserialize_reader(reader)?),
            1 => Self::Revoke,
            2 => Self::Deposit(Funding::deserialize_reader(reader)?),
            3 => Self::Withdraw(Funding::deserialize_reader(reader)?),
            4 => Self::Trade {
                generation: u64::deserialize_reader(reader)?,
                params: SwapCollectiveCompressedExactInV1Params::deserialize_reader(reader)?,
            },
            5 => Self::Close {
                generation: u64::deserialize_reader(reader)?,
                claim: CompressedCashOptionClaim::deserialize_reader(reader)?,
            },
            6 => Self::OwnerClose {
                claim: CompressedCashOptionClaim::deserialize_reader(reader)?,
            },
            7 => Self::OwnerTrade {
                params: SwapCollectiveCompressedExactInV1Params::deserialize_reader(reader)?,
            },
            9 => Self::DepositClassic {
                amount: u64::deserialize_reader(reader)?,
            },
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unknown trading action",
                ))
            }
        })
    }

    #[inline]
    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_deserialize(buf)
    }

    #[inline]
    fn try_from_slice(data: &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_from_slice(data)
    }
}

/// `Action::deserialize_reader` on a cursor, selector for selector; an unknown selector marks
/// the cursor invalid exactly where the reader returns its error.
impl crate::fixed_codec::CursorField for Action {
    #[inline(never)]
    fn read(c: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
        match c.u8() {
            10 => Self::TradeStrip {
                generation: c.u64(),
                params: crate::capped_strip::Trade::read(c),
            },
            11 => Self::OwnerTradeStrip {
                params: crate::capped_strip::Trade::read(c),
            },
            0 => Self::Enable(Grant::read(c)),
            1 => Self::Revoke,
            2 => Self::Deposit(Funding::read(c)),
            3 => Self::Withdraw(Funding::read(c)),
            4 => Self::Trade {
                generation: c.u64(),
                params: SwapCollectiveCompressedExactInV1Params::read(c),
            },
            5 => Self::Close {
                generation: c.u64(),
                claim: CompressedCashOptionClaim::read(c),
            },
            6 => Self::OwnerClose {
                claim: CompressedCashOptionClaim::read(c),
            },
            7 => Self::OwnerTrade {
                params: SwapCollectiveCompressedExactInV1Params::read(c),
            },
            9 => Self::DepositClassic { amount: c.u64() },
            _ => {
                c.invalid = true;
                Self::Revoke
            }
        }
    }
}

pub fn derive(program: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            owner.as_ref(),
        ],
        program,
    )
}

/// solana-pubkey 2.4.0 leaves is_on_curve unimplemented on SBF. Use the
/// Edwards syscall as solana-curve25519 2.3.13/src/edwards.rs does, preserving
/// transaction Ed25519 authority and rejecting another program's signing PDA.
fn ed25519_key(key: &Pubkey) -> bool {
    if *key == Pubkey::default() {
        return false;
    }
    #[cfg(target_os = "solana")]
    {
        let mut validation_result = 0u8;
        // Curve id 0 is CURVE25519_EDWARDS. The syscall reads exactly 32 key
        // bytes and writes one result byte; successful validation returns 0.
        unsafe {
            solana_program::syscalls::sol_curve_validate_point(
                0,
                key.as_ref().as_ptr(),
                &mut validation_result,
            ) == 0
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        key.is_on_curve()
    }
}

impl TradingSessionV2 {
    pub fn valid(&self, program: &Pubkey, address: &Pubkey) -> bool {
        let (key, bump) = derive(program, &self.owner);
        self.magic == MAGIC
            && self.version == 2
            && self.bump == bump
            && key == *address
            && ed25519_key(&self.owner)
            && self.owner != self.session
            && self.sponsor != self.owner
            && self.sponsor != self.session
            && self.sponsor != key
            && self.sponsor != Pubkey::default()
            && self.quote_mint != Pubkey::default()
            && ((self.session == Pubkey::default()
                && self.generation == 0
                && self.expires_at == 0
                && self.revoked)
                || (ed25519_key(&self.session) && self.generation > 0 && self.expires_at > 0))
    }

    pub fn dormant(owner: Pubkey, quote_mint: Pubkey, sponsor: Pubkey, bump: u8) -> Option<Self> {
        if !ed25519_key(&owner)
            || sponsor == owner
            || sponsor == Pubkey::default()
            || quote_mint == Pubkey::default()
        {
            return None;
        }
        Some(Self {
            magic: MAGIC,
            version: 2,
            bump,
            owner,
            session: Pubkey::default(),
            quote_mint,
            sponsor,
            generation: 0,
            expires_at: 0,
            revoked: true,
        })
    }

    pub fn grant(
        owner: Pubkey,
        quote_mint: Pubkey,
        bump: u8,
        previous_generation: u64,
        grant: Grant,
        now: u64,
    ) -> Option<Self> {
        if !ed25519_key(&owner)
            || !ed25519_key(&grant.session)
            || owner == grant.session
            || grant.sponsor == owner
            || grant.sponsor == grant.session
            || grant.sponsor == Pubkey::default()
            || quote_mint == Pubkey::default()
        {
            return None;
        }
        Some(Self {
            magic: MAGIC,
            version: 2,
            bump,
            owner,
            session: grant.session,
            quote_mint,
            sponsor: grant.sponsor,
            generation: previous_generation.checked_add(1)?,
            expires_at: now.checked_add(DURATION_SECONDS)?,
            revoked: false,
        })
    }

    /// Authority only. The canonical native swap/Light inputs enforce funded
    /// balances and fixed fees; turnover never consumes an authorization quota.
    pub fn authorized(&self, signer: &Pubkey, generation: u64, now: u64) -> bool {
        !self.revoked
            && self.session == *signer
            && self.generation == generation
            && now < self.expires_at
    }
}
