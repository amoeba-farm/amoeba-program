use super::*;

#[inline(never)]
pub(in crate::processor) fn derive_decision_v3(
    dispute: &OracleEmergencyDisputeV3,
) -> Result<EmergencyDecisionV3, ProgramError> {
    validate_oracle_emergency_choice(dispute.kind, dispute.fallback_choice)?;
    if dispute.snapshot_total_major_tokens == 0
        || dispute.committed_power > dispute.snapshot_total_major_tokens
        || dispute.revealed_power > dispute.committed_power
        || dispute.revealed_vote_count > dispute.committed_vote_count
        || dispute.supermajority_bps == 0
        || dispute.supermajority_bps > 10_000
        || dispute.fallback_choice >= dispute.choice_count
        || !match dispute.kind {
            OracleEmergencyDisputeKind::Source => matches!(dispute.choice_count, 2 | 3),
            OracleEmergencyDisputeKind::Update | OracleEmergencyDisputeKind::Opening => {
                dispute.choice_count == 3
            }
            OracleEmergencyDisputeKind::BucketMedian => dispute.choice_count == 2,
        }
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let choice_count = dispute.choice_count;
    let mut canonical_power = 0u64;
    let mut canonical_count = 0u32;
    for index in 0..dispute.choice_power.len() {
        if index < usize::from(choice_count) {
            canonical_power = canonical_power
                .checked_add(dispute.choice_power[index])
                .ok_or(VaultError::ArithmeticOverflow)?;
            canonical_count = canonical_count
                .checked_add(dispute.choice_vote_count[index])
                .ok_or(VaultError::ArithmeticOverflow)?;
        } else if dispute.choice_power[index] != 0 || dispute.choice_vote_count[index] != 0 {
            return Err(VaultError::InvalidOracleEmergencyDispute.into());
        }
    }
    if canonical_power != dispute.revealed_power || canonical_count != dispute.revealed_vote_count {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }

    let mut winning_choice = 0u8;
    let mut winning_power = 0u64;
    let mut maximum_count = 0u8;
    for choice in 0..choice_count {
        let power = dispute.choice_power[usize::from(choice)];
        if power > winning_power {
            winning_choice = choice;
            winning_power = power;
            maximum_count = 1;
        } else if power == winning_power {
            maximum_count = maximum_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
    }
    let unique_positive_winner = winning_power > 0 && maximum_count == 1;
    let supermajority_met = dispute.revealed_power > 0
        && u128::from(winning_power)
            .checked_mul(10_000)
            .ok_or(VaultError::ArithmeticOverflow)?
            >= u128::from(dispute.revealed_power)
                .checked_mul(u128::from(dispute.supermajority_bps))
                .ok_or(VaultError::ArithmeticOverflow)?;
    let redistribute = unique_positive_winner && supermajority_met;
    let resolved_choice = if redistribute {
        winning_choice
    } else {
        dispute.fallback_choice
    };
    let winning_vote_count = if redistribute {
        dispute.choice_vote_count[usize::from(winning_choice)]
    } else {
        0
    };
    if redistribute && winning_vote_count == 0 {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(EmergencyDecisionV3 {
        winning_choice,
        winning_power,
        winning_vote_count,
        resolved_choice,
        redistribute,
    })
}

pub(in crate::processor) fn commitment_hash_v3(
    program_id: &Pubkey,
    dispute_key: &Pubkey,
    dispute_id: &[u8; 32],
    voter: &Pubkey,
    amount: u64,
    samba_mint: &Pubkey,
    choice: u8,
    salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        ORACLE_SAMBA_VOTE_COMMITMENT_V3_DOMAIN,
        &[OracleEmergencyVoteRecordV3::ACCOUNT_VERSION],
        program_id.as_ref(),
        dispute_key.as_ref(),
        dispute_id,
        voter.as_ref(),
        &amount.to_le_bytes(),
        samba_mint.as_ref(),
        &[choice],
        salt,
    ])
    .to_bytes()
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn build_open_dispute_v3(
    month: &Pubkey,
    dispute_bump: u8,
    case_hash: [u8; 32],
    kind: OracleEmergencyDisputeKind,
    target_id: [u8; 32],
    target_account: &Pubkey,
    opened_by: &Pubkey,
    packet: &DerivedEmergencyPacket,
    economics: &OracleEconomicParams,
    pot: &Pubkey,
    commit_deadline_slot: u64,
    reveal_deadline_slot: u64,
) -> Result<OracleEmergencyDisputeV3, ProgramError> {
    let minimum_vote_amount =
        oracle_emergency_v3_minimum_vote_amount(packet.snapshot_total_major_tokens)
            .ok_or(VaultError::InvalidOracleEmergencyDispute)?;
    if packet.fallback_choice >= packet.choice_count
        || !match kind {
            OracleEmergencyDisputeKind::Source => matches!(packet.choice_count, 2 | 3),
            OracleEmergencyDisputeKind::Update | OracleEmergencyDisputeKind::Opening => {
                packet.choice_count == 3
            }
            OracleEmergencyDisputeKind::BucketMedian => packet.choice_count == 2,
        }
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(OracleEmergencyDisputeV3 {
        is_initialized: true,
        bump: dispute_bump,
        account_discriminator: OracleEmergencyDisputeV3::ACCOUNT_DISCRIMINATOR,
        account_version: OracleEmergencyDisputeV3::ACCOUNT_VERSION,
        month: *month,
        dispute_id: case_hash,
        kind,
        target_id,
        target_account: *target_account,
        opened_by: *opened_by,
        status: OracleChallengeStatus::Open,
        fallback_choice: packet.fallback_choice,
        winning_choice: packet.fallback_choice,
        resolved_choice: packet.fallback_choice,
        snapshot_slot: packet.snapshot_slot,
        snapshot_total_major_tokens: packet.snapshot_total_major_tokens,
        supermajority_bps: economics.emergency_supermajority_bps,
        pot: *pot,
        commit_deadline_slot,
        reveal_deadline_slot,
        minimum_vote_amount,
        choice_count: packet.choice_count,
        ..OracleEmergencyDisputeV3::default()
    })
}

pub(in crate::processor) fn next_v3_vote_totals(
    dispute: &OracleEmergencyDisputeV3,
    amount: u64,
) -> Result<(u64, u32), ProgramError> {
    if amount < dispute.minimum_vote_amount || dispute.committed_vote_count >= ORACLE_MAX_V3_VOTERS
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    let next_committed = dispute
        .committed_power
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let next_vote_count = dispute
        .committed_vote_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if next_committed > dispute.snapshot_total_major_tokens
        || next_vote_count > ORACLE_MAX_V3_VOTERS
    {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok((next_committed, next_vote_count))
}
