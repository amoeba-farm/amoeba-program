//! Append-only, dispute-isolated sAMBA emergency-vote escrow.
//!
//! Challenge bonds remain in `UserCollateral` and are settled only by tag 169; these handlers
//! move only sAMBA vote principal held by one canonical ATA per dispute.

use super::*;
use crate::{
    constants::{
        ORACLE_EMERGENCY_DISPUTE_V3_PDA_SEED, ORACLE_EMERGENCY_VOTE_V3_PDA_SEED,
        ORACLE_MAX_V3_VOTERS, ORACLE_SAMBA_EMERGENCY_POT_PDA_SEED,
        ORACLE_SAMBA_VOTE_COMMITMENT_V3_DOMAIN, ORACLE_SAMBA_VOTE_SETTLEMENT_PDA_SEED,
        ORACLE_SAMBA_WINNING_VOTE_PDA_SEED,
    },
    state::{
        derive_oracle_emergency_dispute_v3_pda, derive_oracle_emergency_vote_v3_pda,
        derive_oracle_samba_emergency_pot_pda, derive_oracle_samba_vote_settlement_pda,
        derive_oracle_samba_winning_vote_pda, oracle_emergency_v3_minimum_vote_amount,
        OracleEmergencyDisputeV3, OracleEmergencyVoteRecordV3, OracleSambaEmergencyPayoutMode,
        OracleSambaEmergencyPot, OracleSambaVoteSettlementReceipt, OracleSambaWinningVote,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::processor) struct EmergencyDecisionV3 {
    winning_choice: u8,
    winning_power: u64,
    winning_vote_count: u32,
    resolved_choice: u8,
    redistribute: bool,
}

mod accounts;
mod decision;
mod payouts;
mod resolution;
mod voting;

pub(super) use accounts::*;
pub(super) use decision::*;
pub(super) use payouts::*;
pub(super) use resolution::*;
pub(super) use voting::*;
