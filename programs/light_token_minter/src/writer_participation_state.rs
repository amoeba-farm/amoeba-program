//! Versioned, owner-controlled receipts. No fungible Flat is minted for a lot.
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use crate::fixed_codec::{
    fixed_state_deserialize_flat, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use crate::state::WriterSleeveV1;
use crate::writer_participation_math::{ContributionInterval, ParticipationTotals};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const CONTRIBUTION_SEED: &[u8] = b"writer-contribution-v2";
pub const PARTICIPATION_VERSION: u8 = 3;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WriterContributionV2 {
    pub initialized: bool,
    pub bump: u8,
    pub discriminator: [u8; 3],
    pub version: u8,
    pub sleeve: Pubkey,
    pub creator: Pubkey,
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
    pub nonce: u64,
    pub policy_version: u64,
    pub policy_hash: [u8; 32],
    pub principal: u64,
    pub actual_deposit_ts: u64,
    pub entry_ts: u64,
    pub expiry_ts: u64,
    pub weight_offset: u128,
    pub claimed: bool,
}
impl WriterContributionV2 {
    pub const LEN: usize = 231;
    pub fn interval(&self) -> ContributionInterval {
        ContributionInterval {
            principal: self.principal,
            entry_ts: self.entry_ts,
            expiry_ts: self.expiry_ts,
            weight_offset: self.weight_offset,
        }
    }
}
fixed_state_deserialize_flat!(WriterContributionV2, WriterContributionV2::LEN, {
    initialized: bool, bump: u8, discriminator: [u8; 3], version: u8,
    sleeve: Pubkey, creator: Pubkey, owner: Pubkey, rent_payer: Pubkey,
    nonce: u64, policy_version: u64, policy_hash: [u8; 32], principal: u64,
    actual_deposit_ts: u64, entry_ts: u64, expiry_ts: u64,
    weight_offset: u128, claimed: bool,
});

pub fn derive_contribution(
    program: &Pubkey,
    sleeve: &Pubkey,
    creator: &Pubkey,
    nonce: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            CONTRIBUTION_SEED,
            sleeve.as_ref(),
            creator.as_ref(),
            &nonce.to_le_bytes(),
        ],
        program,
    )
}

impl WriterSleeveV1 {
    /// The current schema stores participation totals and start explicitly.
    pub fn has_time_participation(&self) -> bool {
        self.account_version == PARTICIPATION_VERSION
    }
    pub fn participation_start(&self) -> u64 {
        self.participation_start_ts
    }
    pub fn participation_totals(&self) -> ParticipationTotals {
        ParticipationTotals {
            principal: self.writer_principal_atoms,
            capital_seconds: self.capital_seconds,
            maximum_duration: self.maximum_contribution_duration,
        }
    }
    pub fn set_participation_totals(&mut self, totals: ParticipationTotals) {
        self.writer_principal_atoms = totals.principal;
        self.capital_seconds = totals.capital_seconds;
        self.maximum_contribution_duration = totals.maximum_duration;
    }
    pub fn participation_layout_valid(&self) -> bool {
        let totals = self.participation_totals();
        let start = self.participation_start();
        self.has_time_participation()
            && start > 0
            && start < self.expiry_ts
            && totals.maximum_duration <= self.expiry_ts - start
            && ((totals.capital_seconds == 0
                && totals.maximum_duration == 0
                && totals.principal == 0)
                || (totals.capital_seconds > 0
                    && totals.maximum_duration > 0
                    && totals.principal > 0
                    && totals.capital_seconds >= u128::from(totals.principal)
                    && totals.capital_seconds
                        <= u128::from(totals.principal) * u128::from(totals.maximum_duration)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WriterParticipationActionV2 {
    Transfer,
    Split {
        nonce: u64,
        principal_atoms: u64,
    },
    Close,
    /// Permissionless rent refund after the receipt's complete payout.
    ClosePaid,
    /// G3 selector 6: expire a sleeve that has never activated or issued.
    ExpireUnactivatedV3,
    /// Owner receipt exit to regular compressed USDC. The cash witness is a
    /// whole WriterCash leaf; zero selects a hot-vault-only payout.
    ClaimCompressed {
        cash_amount: u64,
        cash_leaf_index: u32,
        cash_root_index: u16,
        cash_prove_by_index: bool,
        proof: Option<[u8; 128]>,
        sponsor_fee_atoms: u64,
    },
    /// Permissionless final payout to the authenticated receipt's current owner.
    /// The transaction payer sponsors SOL; no USDC fee may be deducted.
    SettleContributionCompressed {
        cash_amount: u64,
        cash_leaf_index: u32,
        cash_root_index: u16,
        cash_prove_by_index: bool,
        proof: Option<[u8; 128]>,
    },
}

// Selectors zero and four are permanently retired. Selector one (direct
// pooled Contribute) is retired too: pooled writer capital now enters a sleeve
// only through the Earn Fund (tag 12 Allocate). Existing receipts keep every
// exit below. Explicit encoding preserves all current selectors.
impl BorshDeserialize for WriterParticipationActionV2 {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(match u8::deserialize_reader(reader)? {
            2 => Self::Transfer,
            3 => Self::Split {
                nonce: u64::deserialize_reader(reader)?,
                principal_atoms: u64::deserialize_reader(reader)?,
            },
            4 => return Err(invalid_fixed_borsh()),
            5 => Self::Close,
            9 => Self::ClosePaid,
            6 => Self::ExpireUnactivatedV3,
            7 => Self::ClaimCompressed {
                cash_amount: u64::deserialize_reader(reader)?,
                cash_leaf_index: u32::deserialize_reader(reader)?,
                cash_root_index: u16::deserialize_reader(reader)?,
                cash_prove_by_index: bool::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
                sponsor_fee_atoms: u64::deserialize_reader(reader)?,
            },
            8 => Self::SettleContributionCompressed {
                cash_amount: u64::deserialize_reader(reader)?,
                cash_leaf_index: u32::deserialize_reader(reader)?,
                cash_root_index: u16::deserialize_reader(reader)?,
                cash_prove_by_index: bool::deserialize_reader(reader)?,
                proof: Option::<[u8; 128]>::deserialize_reader(reader)?,
            },
            _ => return Err(invalid_fixed_borsh()),
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

/// `deserialize_reader` above on a cursor, selector for selector; retired and unknown selectors
/// mark the cursor invalid exactly where the reader returns its error.
impl crate::fixed_codec::CursorField for WriterParticipationActionV2 {
    #[inline(never)]
    fn read(c: &mut crate::fixed_codec::CheckedCursor<'_>) -> Self {
        match c.u8() {
            2 => Self::Transfer,
            3 => Self::Split {
                nonce: c.u64(),
                principal_atoms: c.u64(),
            },
            5 => Self::Close,
            9 => Self::ClosePaid,
            6 => Self::ExpireUnactivatedV3,
            7 => Self::ClaimCompressed {
                cash_amount: c.u64(),
                cash_leaf_index: c.u32(),
                cash_root_index: c.u16(),
                cash_prove_by_index: c.boolean(),
                proof: Option::<[u8; 128]>::read(c),
                sponsor_fee_atoms: c.u64(),
            },
            8 => Self::SettleContributionCompressed {
                cash_amount: c.u64(),
                cash_leaf_index: c.u32(),
                cash_root_index: c.u16(),
                cash_prove_by_index: c.boolean(),
                proof: Option::<[u8; 128]>::read(c),
            },
            _ => {
                c.invalid = true;
                Self::Transfer
            }
        }
    }
}
impl BorshSerialize for WriterParticipationActionV2 {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Self::Transfer => 2u8.serialize(writer),
            Self::Split {
                nonce,
                principal_atoms,
            } => {
                3u8.serialize(writer)?;
                nonce.serialize(writer)?;
                principal_atoms.serialize(writer)
            }
            Self::Close => 5u8.serialize(writer),
            Self::ClosePaid => 9u8.serialize(writer),
            Self::ExpireUnactivatedV3 => 6u8.serialize(writer),
            Self::ClaimCompressed {
                cash_amount,
                cash_leaf_index,
                cash_root_index,
                cash_prove_by_index,
                proof,
                sponsor_fee_atoms,
            } => {
                7u8.serialize(writer)?;
                cash_amount.serialize(writer)?;
                cash_leaf_index.serialize(writer)?;
                cash_root_index.serialize(writer)?;
                cash_prove_by_index.serialize(writer)?;
                proof.serialize(writer)?;
                sponsor_fee_atoms.serialize(writer)
            }
            Self::SettleContributionCompressed {
                cash_amount,
                cash_leaf_index,
                cash_root_index,
                cash_prove_by_index,
                proof,
            } => {
                8u8.serialize(writer)?;
                cash_amount.serialize(writer)?;
                cash_leaf_index.serialize(writer)?;
                cash_root_index.serialize(writer)?;
                cash_prove_by_index.serialize(writer)?;
                proof.serialize(writer)
            }
        }
    }
}
