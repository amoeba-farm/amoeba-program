//! Versioned, owner-controlled receipts. No fungible Flat is minted for a lot.
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use crate::state::WriterSleeveV1;
use crate::writer_participation_math::{ContributionInterval, ParticipationTotals};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const CONTRIBUTION_SEED: &[u8] = b"writer-contribution-v2";
pub const PARTICIPATION_VERSION: u8 = 2;

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
fixed_state_deserialize!(WriterContributionV2, WriterContributionV2::LEN, {
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
    /// Version 2 retains the exact fixed sleeve layout. Its formerly reserved
    /// bytes hold U:u128, h_max:u64, and S:u64. Version 1 bytes remain unchanged.
    pub fn has_time_participation(&self) -> bool {
        self.account_version == PARTICIPATION_VERSION
    }
    pub fn participation_start(&self) -> u64 {
        u64::from_le_bytes(self.reserved[24..32].try_into().unwrap())
    }
    pub fn participation_totals(&self) -> ParticipationTotals {
        ParticipationTotals {
            principal: self.writer_principal_atoms,
            capital_seconds: u128::from_le_bytes(self.reserved[..16].try_into().unwrap()),
            maximum_duration: u64::from_le_bytes(self.reserved[16..24].try_into().unwrap()),
        }
    }
    pub fn set_participation_totals(&mut self, totals: ParticipationTotals) {
        self.writer_principal_atoms = totals.principal;
        self.reserved[..16].copy_from_slice(&totals.capital_seconds.to_le_bytes());
        self.reserved[16..24].copy_from_slice(&totals.maximum_duration.to_le_bytes());
    }
    pub fn participation_layout_valid(&self) -> bool {
        let totals = self.participation_totals();
        let start = self.participation_start();
        self.has_time_participation()
            && start > 0
            && start < self.expiry_ts
            && self.flat_par_supply_atoms == 0
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

#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum WriterParticipationActionV2 {
    Enable { participation_start_ts: u64 },
    Contribute { nonce: u64, amount_atoms: u64 },
    Transfer,
    Split { nonce: u64, principal_atoms: u64 },
    Claim,
    Close,
}
