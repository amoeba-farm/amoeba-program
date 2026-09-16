use super::*;
use borsh::{BorshDeserialize, BorshSerialize};
pub const CASE_SEED: &[u8] = b"g3-council-case-v1";
pub const ROUND_SEED: &[u8] = b"g3-council-round-v1";
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CouncilCase {
    pub discriminator: [u8; 3],
    pub version: u8,
    pub initialized: bool,
    pub bump: u8,
    pub month: Pubkey,
    pub target: Pubkey,
    pub target_id: [u8; 32],
    pub case_hash: [u8; 32],
    pub evidence_hash: [u8; 32],
    pub kind: OracleEmergencyDisputeKind,
    pub choices: u8,
    pub fallback: u8,
    pub snapshot_slot: u64,
    pub opened_slot: u64,
    pub deadline: u64,
    pub finalized: bool,
    pub outcome: u8,
    pub reason: u8,
    pub decision_epoch: u64,
    pub seat_mask: u8,
    pub resolved_slot: u64,
}
impl CouncilCase {
    pub const LEN: usize = 213;
}
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CouncilRound {
    pub discriminator: [u8; 3],
    pub version: u8,
    pub initialized: bool,
    pub bump: u8,
    pub case: Pubkey,
    pub epoch: u64,
    pub seats_hash: [u8; 32],
    pub ballots: [u8; 5],
}
impl CouncilRound {
    pub const LEN: usize = 83;
}
#[derive(Clone, Copy)]
pub(super) struct CouncilResolutionContext {
    pub month: Pubkey,
    pub kind: OracleEmergencyDisputeKind,
    pub target_id: [u8; 32],
    pub case_key: Pubkey,
}
pub(super) fn case_address(program: &Pubkey, month: &Pubkey, target: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            CASE_SEED,
            month.as_ref(),
            target.as_ref(),
        ],
        program,
    )
}
pub(super) fn round_address(program: &Pubkey, case: &Pubkey, epoch: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ROUND_SEED,
            case.as_ref(),
            &epoch.to_le_bytes(),
        ],
        program,
    )
}
pub(super) fn load_case(program: &Pubkey, info: &AccountInfo) -> Result<CouncilCase, ProgramError> {
    let v: CouncilCase = load_council_state(
        info,
        program,
        CouncilCase::LEN,
        VaultError::InvalidOracleEmergencyDispute,
    )?;
    if !v.initialized
        || v.discriminator != *b"OCC"
        || v.version != 1
        || case_address(program, &v.month, &v.target) != (*info.key, v.bump)
        || v.opened_slot == 0
        || v.deadline <= v.opened_slot
        || v.snapshot_slot == 0
        || v.snapshot_slot > v.opened_slot
        || !(2..=3).contains(&v.choices)
        || v.fallback >= v.choices
        || v.outcome >= v.choices
        || v.reason > 2
        || v.finalized != (v.resolved_slot > 0)
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(v)
}
pub(super) fn load_round(
    program: &Pubkey,
    info: &AccountInfo,
    case: &Pubkey,
    view: &CouncilView,
) -> Result<CouncilRound, ProgramError> {
    let v: CouncilRound = load_council_state(
        info,
        program,
        CouncilRound::LEN,
        VaultError::InvalidOracleEmergencyDispute,
    )?;
    if !v.initialized
        || v.discriminator != *b"OCR"
        || v.version != 1
        || v.case != *case
        || v.epoch != view.epoch
        || v.seats_hash != view.digest
        || round_address(program, case, view.epoch) != (*info.key, v.bump)
        || v.ballots.iter().any(|v| *v != 255 && *v > 2)
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(v)
}
/// Absolute three-seat threshold; zeros/split votes derive the existing case fallback.
pub fn council_decision(
    ballots: &[u8; 5],
    choices: u8,
    fallback: u8,
) -> Result<(u8, u8, bool), ProgramError> {
    if !(2..=3).contains(&choices)
        || fallback >= choices
        || ballots.iter().any(|b| *b != 255 && *b >= choices)
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    for choice in 0..choices {
        let mask = ballots
            .iter()
            .enumerate()
            .fold(0u8, |a, (i, b)| a | if *b == choice { 1 << i } else { 0 });
        if mask.count_ones() >= 3 {
            return Ok((choice, mask, true));
        }
    }
    Ok((fallback, 0, false))
}
