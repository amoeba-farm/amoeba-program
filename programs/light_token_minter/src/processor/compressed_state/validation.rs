use super::*;

pub(in crate::processor) fn validate_typed_state_data(
    program_id: &Pubkey,
    canonical_pda: &Pubkey,
    domain: CompressedStateDomain,
    data: &[u8],
    compact_out: Option<&mut Vec<u8>>,
) -> ProgramResult {
    let (account_len, encoded_len, error) = match domain {
        CompressedStateDomain::OracleCarryJournal
        | CompressedStateDomain::OracleCarryCheckpoint => {
            super::super::oracle_carry::compressed::validate_carry_payload(
                program_id,
                canonical_pda,
                domain,
                data,
            )?;
            if let Some(output) = compact_out {
                *output = data.to_vec();
            }
            return Ok(());
        }
        CompressedStateDomain::OracleSkuCoverageRecord => (
            OracleSkuCoverageRecord::LEN,
            OracleSkuCoverageRecord::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleSkuCoverageRecord,
        ),
        CompressedStateDomain::OracleUsdcSkuPool => (
            OracleUsdcSkuPool::LEN,
            OracleUsdcSkuPool::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcSkuPool,
        ),
        CompressedStateDomain::OracleUsdcSourceReward => (
            OracleUsdcSourceReward::LEN,
            OracleUsdcSourceReward::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcSourceReward,
        ),
        CompressedStateDomain::OracleSupportPosition => (
            OracleSupportPosition::LEN,
            OracleSupportPosition::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleState,
        ),
        CompressedStateDomain::OracleUsdcRewardRegistration => (
            OracleUsdcRewardRegistration::LEN,
            OracleUsdcRewardRegistration::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcRewardRegistration,
        ),
        CompressedStateDomain::OracleUsdcRewardReceipt => (
            OracleUsdcRewardReceipt::LEN,
            OracleUsdcRewardReceipt::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcRewardReceipt,
        ),

        CompressedStateDomain::OracleSourceState
        | CompressedStateDomain::OracleSourceDescriptor => (
            OracleSourceState::LEN,
            OracleSourceState::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleSourceAccount,
        ),
        CompressedStateDomain::OracleSourceObservations => (
            OracleSourceObservations::LEN,
            OracleSourceObservations::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleObservation,
        ),
    };
    if data.len() != account_len || data[encoded_len..].iter().any(|byte| *byte != 0) {
        return Err(error.into());
    }

    let invalid_encoding = match domain {
        CompressedStateDomain::OracleCarryJournal
        | CompressedStateDomain::OracleCarryCheckpoint => unreachable!(),
        CompressedStateDomain::OracleSkuCoverageRecord
        | CompressedStateDomain::OracleUsdcSkuPool => data[0] > 1,
        CompressedStateDomain::OracleUsdcSourceReward => {
            data[0] > 1
                || data[202] > 1
                || data[203] > OracleSourceStatus::TimedOut as u8
                || data[277] > 1
        }
        CompressedStateDomain::OracleSupportPosition => {
            data[0] > 1
                || data[138] > 1
                || data[139] > OracleEscrowDisposition::Transferred as u8
                || data[140] > 1
        }
        CompressedStateDomain::OracleUsdcRewardRegistration => {
            data[0] > 1 || data[102] > OracleUsdcRewardKind::Update as u8
        }
        CompressedStateDomain::OracleUsdcRewardReceipt => {
            data[0] > 1 || data[102] > OracleUsdcRewardKind::Update as u8
        }

        CompressedStateDomain::OracleSourceState
        | CompressedStateDomain::OracleSourceDescriptor => {
            data[0] > 1 || data[260] > OracleSourceStatus::TimedOut as u8 || data[261] > 1
        }
        CompressedStateDomain::OracleSourceObservations => {
            data[0] > 1
                || (data[5] != OracleSourceObservations::ACCOUNT_VERSION
                    && data[5] != OracleSourceObservations::INHERITED_ANCHOR_VERSION)
        }
    };
    if invalid_encoding {
        return Err(error.into());
    }

    let invalid = match domain {
        CompressedStateDomain::OracleCarryJournal
        | CompressedStateDomain::OracleCarryCheckpoint => unreachable!(),
        CompressedStateDomain::OracleSkuCoverageRecord => {
            let month = fixed_pubkey(data, 6);
            let sku_id = fixed_bytes32(data, 38);
            let (expected, bump) =
                derive_oracle_sku_coverage_record_pda(program_id, &month, &sku_id);
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || data[2..5] != OracleSkuCoverageRecord::ACCOUNT_DISCRIMINATOR
                || data[5] != OracleSkuCoverageRecord::ACCOUNT_VERSION
        }
        CompressedStateDomain::OracleUsdcSkuPool => {
            let schedule = fixed_pubkey(data, 6);
            let bucket_id = fixed_bytes32(data, 70);
            let (expected, bump) =
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &bucket_id);
            let reward_budget = fixed_u64(data, 102)
                .checked_add(fixed_u64(data, 118))
                .and_then(|value| value.checked_add(fixed_u64(data, 134)));
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || data[2..5] != OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR
                || data[5] != OracleUsdcSkuPool::ACCOUNT_VERSION
                || fixed_bytes32_is_zero(data, 6)
                || fixed_bytes32_is_zero(data, 38)
                || fixed_bytes32_is_zero(data, 70)
                || reward_budget.is_none()
        }
        CompressedStateDomain::OracleUsdcSourceReward => {
            let schedule = fixed_pubkey(data, 38);
            let source = fixed_pubkey(data, 102);
            let (expected, bump) =
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, &source);
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || data[2..5] != OracleUsdcSourceReward::ACCOUNT_DISCRIMINATOR
                || data[5] != OracleUsdcSourceReward::ACCOUNT_VERSION
                || fixed_bytes32_is_zero(data, 6)
                || fixed_bytes32_is_zero(data, 38)
                || fixed_bytes32_is_zero(data, 102)
                || fixed_bytes32_is_zero(data, 134)
                || fixed_bytes32_is_zero(data, 166)
        }
        CompressedStateDomain::OracleSupportPosition => {
            let month = fixed_pubkey(data, 2);
            let supporter = fixed_pubkey(data, 34);
            let source = fixed_pubkey(data, 66);
            let (expected, bump) =
                super::super::derive_oracle_support_pda(program_id, &month, &source, &supporter);
            let support_stake = fixed_u64(data, 130);
            let released = data[138];
            let disposition = data[139];
            let failed_schedule_escrow_counted = data[140];
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || fixed_bytes32_is_zero(data, 2)
                || fixed_bytes32_is_zero(data, 34)
                || fixed_bytes32_is_zero(data, 66)
                || fixed_bytes32_is_zero(data, 98)
                || support_stake == 0
                || (released == 1 && disposition != OracleEscrowDisposition::Refunded as u8)
                || (released == 0 && disposition != OracleEscrowDisposition::Unsettled as u8)
                || (failed_schedule_escrow_counted == 1 && released == 0)
        }
        CompressedStateDomain::OracleUsdcRewardRegistration => {
            let schedule = fixed_pubkey(data, 38);
            let subject = fixed_pubkey(data, 103);
            let (expected, bump) = derive_oracle_usdc_reward_registration_pda(
                program_id,
                &schedule,
                OracleUsdcRewardKind::Update,
                &subject,
            );
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || data[2..5] != OracleUsdcRewardRegistration::ACCOUNT_DISCRIMINATOR
                || data[5] != OracleUsdcRewardRegistration::ACCOUNT_VERSION
                || fixed_bytes32_is_zero(data, 6)
                || fixed_bytes32_is_zero(data, 38)
                || fixed_bytes32_is_zero(data, 70)
                || data[102] != OracleUsdcRewardKind::Update as u8
                || fixed_bytes32_is_zero(data, 103)
                || fixed_bytes32_is_zero(data, 135)
        }
        CompressedStateDomain::OracleUsdcRewardReceipt => {
            let schedule = fixed_pubkey(data, 38);
            let recipient = fixed_pubkey(data, 70);
            let kind = data[102];
            let subject = fixed_pubkey(data, 103);
            let kind = match kind {
                0 => OracleUsdcRewardKind::SourceProposer,
                1 => OracleUsdcRewardKind::SourceSupport,
                2 => OracleUsdcRewardKind::Opening,
                _ => OracleUsdcRewardKind::Update,
            };
            let (expected, bump) = derive_oracle_usdc_reward_receipt_pda(
                program_id, &schedule, kind, &subject, &recipient,
            );
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || data[2..5] != OracleUsdcRewardReceipt::ACCOUNT_DISCRIMINATOR
                || data[5] != OracleUsdcRewardReceipt::ACCOUNT_VERSION
                || fixed_bytes32_is_zero(data, 6)
                || fixed_bytes32_is_zero(data, 38)
                || fixed_bytes32_is_zero(data, 70)
                || fixed_bytes32_is_zero(data, 103)
                || fixed_u64(data, 135) == 0
        }

        CompressedStateDomain::OracleSourceState
        | CompressedStateDomain::OracleSourceDescriptor => {
            let month = fixed_pubkey(data, 2);
            let source_id = fixed_bytes32(data, 34);
            let (expected, bump) =
                super::super::derive_oracle_source_pda(program_id, &month, &source_id);
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || fixed_bytes32_is_zero(data, 2)
                || fixed_bytes32_is_zero(data, 34)
                || fixed_bytes32_is_zero(data, 66)
                || fixed_bytes32_is_zero(data, 194)
                || (domain == CompressedStateDomain::OracleSourceDescriptor
                    && (fixed_bytes32_is_zero(data, 98)
                        || fixed_bytes32_is_zero(data, 130)
                        || fixed_bytes32_is_zero(data, 162)))
        }
        CompressedStateDomain::OracleSourceObservations => {
            let source = fixed_pubkey(data, 38);
            let (expected, bump) =
                super::super::derive_oracle_source_observations_pda(program_id, &source);
            *canonical_pda != expected
                || data[0] != 1
                || data[1] != bump
                || data[2..5] != OracleSourceObservations::ACCOUNT_DISCRIMINATOR
                || !matches!(
                    data[5],
                    OracleSourceObservations::ACCOUNT_VERSION
                        | OracleSourceObservations::INHERITED_ANCHOR_VERSION
                )
                || fixed_bytes32_is_zero(data, 6)
                || fixed_bytes32_is_zero(data, 38)
                || !observation_projection_is_well_formed(
                    &data[..OracleSourceObservations::REQUIRED_DATA_LEN],
                )
        }
    };
    if invalid {
        return Err(error.into());
    }
    if let Some(output) = compact_out {
        *output = capture_compact_state_data(domain, data);
    }
    Ok(())
}

#[inline(never)]
pub(in crate::processor) fn fixed_bytes32_is_zero(data: &[u8], offset: usize) -> bool {
    data[offset..offset + 32].iter().all(|byte| *byte == 0)
}

#[inline(never)]
pub(in crate::processor) fn fixed_u64(data: &[u8], offset: usize) -> u64 {
    let mut input = FixedCursor::new(&data[offset..]);
    input.u64()
}

#[inline(never)]
pub(in crate::processor) fn fixed_bytes32(data: &[u8], offset: usize) -> [u8; 32] {
    let mut input = FixedCursor::new(&data[offset..]);
    input.bytes()
}

#[inline(never)]
pub(in crate::processor) fn fixed_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(fixed_bytes32(data, offset))
}

pub(in crate::processor) fn validate_compact_state_data(
    domain: CompressedStateDomain,
    data: &[u8],
) -> ProgramResult {
    if domain == CompressedStateDomain::OracleSourceObservations {
        if data.len() != OracleSourceObservations::REQUIRED_DATA_LEN
            || !observation_projection_is_well_formed(data)
        {
            return Err(VaultError::InvalidOracleObservation.into());
        }
        return Ok(());
    }
    let (required_len, error) = match domain {
        CompressedStateDomain::OracleCarryJournal
        | CompressedStateDomain::OracleCarryCheckpoint => {
            return super::super::oracle_carry::compressed::validate_carry_body(domain, data)
        }
        CompressedStateDomain::OracleSkuCoverageRecord => (
            CompactOracleSkuCoverageRecord::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleSkuCoverageRecord,
        ),
        CompressedStateDomain::OracleUsdcSkuPool => (
            CompactOracleUsdcSkuPool::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcSkuPool,
        ),
        CompressedStateDomain::OracleUsdcSourceReward => (
            CompactOracleUsdcSourceReward::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcSourceReward,
        ),
        CompressedStateDomain::OracleSupportPosition => (
            CompactOracleSupportPosition::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleState,
        ),
        CompressedStateDomain::OracleUsdcRewardRegistration => (
            CompactOracleUsdcRewardRegistration::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcRewardRegistration,
        ),
        CompressedStateDomain::OracleUsdcRewardReceipt => (
            CompactOracleUsdcRewardReceipt::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleUsdcRewardReceipt,
        ),

        CompressedStateDomain::OracleSourceState
        | CompressedStateDomain::OracleSourceDescriptor => (
            if domain == CompressedStateDomain::OracleSourceState {
                CompactOracleSourceState::REQUIRED_DATA_LEN
            } else {
                CompactOracleSourceDescriptor::REQUIRED_DATA_LEN
            },
            VaultError::InvalidOracleSourceAccount,
        ),
        CompressedStateDomain::OracleSourceObservations => (
            CompactOracleSourceObservations::REQUIRED_DATA_LEN,
            VaultError::InvalidOracleObservation,
        ),
    };
    if data.len() < required_len || data[required_len..].iter().any(|byte| *byte != 0) {
        return Err(error.into());
    }

    let invalid = match domain {
        CompressedStateDomain::OracleCarryJournal
        | CompressedStateDomain::OracleCarryCheckpoint => unreachable!(),
        CompressedStateDomain::OracleSkuCoverageRecord
        | CompressedStateDomain::OracleUsdcSkuPool => false,
        CompressedStateDomain::OracleUsdcSourceReward => {
            data[36] > 1 || data[37] > OracleSourceStatus::TimedOut as u8 || data[111] > 1
        }
        CompressedStateDomain::OracleSupportPosition => {
            let support_stake = fixed_u64(data, 64);
            let released = data[72];
            let disposition = data[73];
            let failed_schedule_escrow_counted = data[74];
            fixed_bytes32_is_zero(data, 0)
                || fixed_bytes32_is_zero(data, 32)
                || released > 1
                || disposition > OracleEscrowDisposition::Transferred as u8
                || failed_schedule_escrow_counted > 1
                || support_stake == 0
                || (released == 1 && disposition != OracleEscrowDisposition::Refunded as u8)
                || (released == 0 && disposition != OracleEscrowDisposition::Unsettled as u8)
                || (failed_schedule_escrow_counted == 1 && released == 0)
        }
        CompressedStateDomain::OracleUsdcRewardRegistration => {
            fixed_bytes32_is_zero(data, 0)
                || fixed_bytes32_is_zero(data, 32)
                || fixed_bytes32_is_zero(data, 64)
                || data[96] == 0
        }
        CompressedStateDomain::OracleUsdcRewardReceipt => {
            data[0] > OracleUsdcRewardKind::Update as u8
                || fixed_bytes32_is_zero(data, 1)
                || fixed_bytes32_is_zero(data, 33)
                || fixed_u64(data, 65) == 0
        }

        CompressedStateDomain::OracleSourceState => {
            fixed_bytes32_is_zero(data, 0)
                || fixed_bytes32_is_zero(data, 32)
                || fixed_bytes32_is_zero(data, 64)
                || data[130] > OracleSourceStatus::TimedOut as u8
                || data[131] > 1
        }
        CompressedStateDomain::OracleSourceDescriptor => {
            fixed_bytes32_is_zero(data, 0)
                || fixed_bytes32_is_zero(data, 32)
                || fixed_bytes32_is_zero(data, 64)
        }
        CompressedStateDomain::OracleSourceObservations => {
            !observation_projection_is_well_formed(data)
        }
    };
    if invalid {
        Err(error.into())
    } else {
        Ok(())
    }
}

/// Validates the canonical 582-byte observation projection. The account header and authenticated
/// month/source identities stay in the projection; only the adaptive transport omits the two
/// identities. Empty histories are valid for newly-created or frozen accounts, while populated
/// histories must be a contiguous, strictly increasing `(state, timestamp)` prefix.
#[inline(never)]
fn observation_projection_is_well_formed(data: &[u8]) -> bool {
    if data.len() != OracleSourceObservations::REQUIRED_DATA_LEN
        || data[0] != 1
        || data[2..5] != OracleSourceObservations::ACCOUNT_DISCRIMINATOR
        || !matches!(
            data[5],
            OracleSourceObservations::ACCOUNT_VERSION
                | OracleSourceObservations::INHERITED_ANCHOR_VERSION
        )
        || fixed_bytes32_is_zero(data, 6)
        || fixed_bytes32_is_zero(data, 38)
    {
        return false;
    }
    let mut previous_time = 0;
    let mut empty_tail = false;
    for index in 0..crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS {
        let state = fixed_u64(data, 70 + index * 8);
        let timestamp = fixed_u64(data, 326 + index * 8);
        if state == 0 && timestamp == 0 {
            empty_tail = true;
            continue;
        }
        if empty_tail || state == 0 || timestamp == 0 || (index > 0 && timestamp <= previous_time) {
            return false;
        }
        previous_time = timestamp;
    }
    true
}
