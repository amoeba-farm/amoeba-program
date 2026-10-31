//! Tag 210, oracle v2 "heads on pikes" (`docs/heads.md`). Compiled only with the `heads` feature;
//! every other build classifies the byte as unknown and never reaches this.
//!
//! The selector payload decodes completely (exact length, every value domain) before any account
//! is read. The handlers live by area: `config` (the singleton, boards, items and components,
//! stake, burns), `delegation`, `claims`, `sources`, `contracts` and `rewards`; `token` holds
//! the AMBA custody plumbing they share. Every handler is `#[inline(never)]`, so `route` keeps a
//! small SBF frame and each handler has its own.

use super::*;
use crate::instruction::heads::HeadsInstruction;

mod accounts;
mod claims;
mod config;
mod contracts;
mod delegation;
#[cfg(any(feature = "heads-migrate", feature = "localnet-dev-authority"))]
mod migrate;
mod rewards;
mod samples;
mod sources;
// The handlers call these CPIs; until they are written only the validation is exercised.
#[allow(dead_code)]
pub(super) mod token;
/// State transitions shared by the handlers of both halves of the family.
#[allow(dead_code)]
pub(super) mod transitions;

#[cfg(test)]
mod tests;

/// Decodes the selector and its payload, then routes.
pub(super) fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let instruction = HeadsInstruction::unpack(payload)?;
    route(program_id, accounts, instruction)
}

#[inline(never)]
fn route(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction: HeadsInstruction,
) -> ProgramResult {
    use HeadsInstruction as I;
    match instruction {
        I::InitHeadsConfig(params) => config::init_heads_config(program_id, accounts, params),
        I::InitBoard(params) => config::init_board(program_id, accounts, params),
        I::SetBoardParams(params) => config::set_board_params(program_id, accounts, params),
        I::SetRulingDeadline { secs } => config::set_ruling_deadline(program_id, accounts, secs),
        I::SetItem(params) => config::set_item(program_id, accounts, params),
        I::SetComponent(params) => config::set_component(program_id, accounts, params),
        I::Stake { amount } => config::stake(program_id, accounts, amount),
        I::Unstake { amount } => config::unstake(program_id, accounts, amount),
        I::BurnPending => config::burn_pending(program_id, accounts),
        I::PostHead(params) => delegation::post_head(program_id, accounts, params),
        I::Delegate { amount } => delegation::delegate(program_id, accounts, amount),
        I::Undelegate { amount } => delegation::undelegate(program_id, accounts, amount),
        I::SyncDelegation => delegation::sync_delegation(program_id, accounts),
        I::ExpireHead => delegation::expire_head(program_id, accounts),
        I::CommitClaim(params) => claims::commit_claim(program_id, accounts, params),
        I::RevealClaim(params) => claims::reveal_claim(program_id, accounts, params),
        I::ChallengeClaim(params) => claims::challenge_claim(program_id, accounts, params),
        I::RuleChallenge(params) => claims::rule_challenge(program_id, accounts, params),
        I::FinalizeClaim => claims::finalize_claim(program_id, accounts),
        I::SettleChallenge => claims::settle_challenge(program_id, accounts),
        I::RefundExpiredCommit => claims::refund_expired_commit(program_id, accounts),
        I::ChallengeSource(params) => sources::challenge_source(program_id, accounts, params),
        I::FinalizeSourceChallenges => sources::finalize_source_challenges(program_id, accounts),
        I::ListContract(params) => contracts::list_contract(program_id, accounts, params),
        I::CreateQuarterPeriod(params) => {
            contracts::create_quarter_period(program_id, accounts, params)
        }
        I::SettleComponent(params) => contracts::settle_component(program_id, accounts, params),
        I::FinalizeContract => contracts::finalize_contract(program_id, accounts),
        I::FundBoard { amount } => rewards::fund_board(program_id, accounts, amount),
        I::ClaimRewards => rewards::claim_rewards(program_id, accounts),
        I::ClaimPrize => rewards::claim_prize(program_id, accounts),
        I::RollOverPrizes => rewards::roll_over_prizes(program_id, accounts),
        // The dev builds only: mainnet never had a Head of the old layout.
        #[cfg(any(feature = "heads-migrate", feature = "localnet-dev-authority"))]
        I::MigrateHead { contracts } => migrate::migrate_head(program_id, accounts, contracts),
        #[cfg(not(any(feature = "heads-migrate", feature = "localnet-dev-authority")))]
        I::MigrateHead { .. } => Err(OracleError::InvalidInstructionData.into()),
    }
}
