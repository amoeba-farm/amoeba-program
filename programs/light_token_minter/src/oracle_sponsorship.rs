//! Real USDC underwriting for gasless research. Reputation never changes oracle
//! weights or substitutes for the existing listing bond.
use crate::fixed_codec::{
    fixed_state_deserialize, invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode,
    FixedStateEncode, FixedWriter,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const RESERVE_SEED: &[u8] = b"oracle-agent-v1";
pub const AGREEMENT_SEED: &[u8] = b"oracle-sponsor-v1";
pub const EXPOSURE_SEED: &[u8] = b"oracle-credit-v1";
pub const DEFAULT_RESEARCHER_BPS: u16 = 5_000;
pub const DEFAULT_RETENTION_BPS: u16 = 2_000;

pub fn reserve_address(program: &Pubkey, agent: &[u8; 32], authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            RESERVE_SEED,
            agent,
            authority.as_ref(),
        ],
        program,
    )
}
pub fn agreement_address(program: &Pubkey, source: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            AGREEMENT_SEED,
            source.as_ref(),
        ],
        program,
    )
}
pub fn exposure_address(program: &Pubkey, reserve: &Pubkey, funder: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            EXPOSURE_SEED,
            reserve.as_ref(),
            funder.as_ref(),
        ],
        program,
    )
}

/// Floors researcher and reserve shares. Sponsor receives the remainder, so
/// all funded atoms are distributed, including amounts smaller than one cent.
pub fn split_bounty(
    amount: u64,
    researcher_bps: u16,
    retention_bps: u16,
) -> Option<(u64, u64, u64)> {
    if researcher_bps > 10_000 || retention_bps > 10_000 {
        return None;
    }
    let researcher = ((u128::from(amount) * u128::from(researcher_bps)) / 10_000) as u64;
    let retained = ((u128::from(researcher) * u128::from(retention_bps)) / 10_000) as u64;
    Some((amount - researcher, researcher - retained, retained))
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, PartialEq, BorshSerialize)]
    pub struct SponsorTerms {
        pub agent_id: [u8; 32],
        pub researcher: Pubkey,
        pub candidate_hash: [u8; 32],
        /// 0: sponsor cash; 1: researcher reserve (requires researcher signature).
        pub funding: u8,
        pub researcher_bps: u16,
        pub retention_bps: u16,
        pub maximum_bond: u64,
        pub maximum_outstanding: u64,
    }
}
impl SponsorTerms {
    pub fn valid(&self) -> bool {
        self.agent_id != [0; 32] && self.candidate_hash != [0; 32]
            && self.researcher != Pubkey::default() && self.funding <= 1
            && self.researcher_bps <= 10_000 && self.retention_bps <= 10_000
            // Gasless candidates cannot countersign a sponsor's replacement split.
            && self.researcher_bps == if self.funding == 1 { 10_000 } else { DEFAULT_RESEARCHER_BPS }
            && self.retention_bps == DEFAULT_RETENTION_BPS
            && self.maximum_bond > 0 && self.maximum_outstanding >= self.maximum_bond
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentReserve {
    pub discriminator: [u8; 8],
    pub bump: u8,
    pub agent_id: [u8; 32],
    pub authority: Pubkey,
    pub accepted: u64,
    pub rejected: u64,
    pub pending: u64,
    pub earned: u64,
    pub slashed: u64,
}
impl AgentReserve {
    pub const LEN: usize = 113;
    pub const MAGIC: [u8; 8] = *b"ORAGENT1";
}
fixed_state_deserialize!(AgentReserve, 113, {
    discriminator: [u8; 8], bump: u8, agent_id: [u8; 32], authority: Pubkey,
    accepted: u64, rejected: u64, pending: u64, earned: u64, slashed: u64
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SponsorExposure {
    pub discriminator: [u8; 8],
    pub bump: u8,
    pub reserve: Pubkey,
    pub funder: Pubkey,
    pub outstanding: u64,
}
impl SponsorExposure {
    pub const LEN: usize = 81;
    pub const MAGIC: [u8; 8] = *b"OREXPOS1";
}
fixed_state_deserialize!(SponsorExposure, 81, {
    discriminator: [u8; 8], bump: u8, reserve: Pubkey, funder: Pubkey, outstanding: u64
});

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceSponsorship {
    pub discriminator: [u8; 8],
    pub bump: u8,
    pub source: Pubkey,
    pub reserve: Pubkey,
    /// Cash owner for sponsorship; reserve PDA for self-funding.
    pub funder: Pubkey,
    pub candidate_hash: [u8; 32],
    pub bond: u64,
    pub researcher_bps: u16,
    pub retention_bps: u16,
    pub funding: u8,
    /// 0 pending, 1 accepted, 2 rejected, 3 neutral timeout/merge.
    pub outcome: u8,
    pub reward_settled: bool,
    pub reward_paid: u64,
    pub month: Pubkey,
    pub bucket_id: [u8; 32],
    pub source_type_hash: [u8; 32],
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
}
impl SourceSponsorship {
    pub const LEN: usize = 320;
    pub const MAGIC: [u8; 8] = *b"ORSPONS1";
}
fixed_state_deserialize!(SourceSponsorship, 320, {
    discriminator: [u8; 8], bump: u8, source: Pubkey, reserve: Pubkey, funder: Pubkey,
    candidate_hash: [u8; 32], bond: u64, researcher_bps: u16, retention_bps: u16,
    funding: u8, outcome: u8, reward_settled: bool, reward_paid: u64,
    month: Pubkey, bucket_id: [u8; 32], source_type_hash: [u8; 32],
    canonical_locator_hash: [u8; 32], source_definition_hash: [u8; 32]
});
