use super::*;

#[inline(always)]
fn insertion_sort_bucket_values(values: &mut [i64]) {
    for index in 1..values.len() {
        let current = values[index];
        let mut position = index;
        while position > 0 {
            let previous = values[position - 1];
            if previous <= current {
                break;
            }
            values[position] = previous;
            position -= 1;
        }
        values[position] = current;
    }
}

#[inline(always)]
fn insertion_sort_u64(values: &mut [u64]) {
    for index in 1..values.len() {
        let current = values[index];
        let mut position = index;
        while position > 0 && values[position - 1] > current {
            values[position] = values[position - 1];
            position -= 1;
        }
        values[position] = current;
    }
}

pub fn deterministic_bucket_median(values: &mut [i64]) -> Result<i64, ProgramError> {
    if values.is_empty() || values.len() > crate::constants::MAX_ORACLE_BUCKET_SOURCES {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    insertion_sort_bucket_values(values);
    let upper = values.len() / 2;
    if !values.len().is_multiple_of(2) {
        return Ok(values[upper]);
    }
    let lower = values[upper - 1];
    let upper = values[upper];
    let differing_bits = lower ^ upper;
    let floor = (lower & upper) + (differing_bits >> 1);
    Ok(floor + i64::from(floor < 0 && differing_bits & 1 != 0))
}

pub fn deterministic_temporal_median(values: &mut [u64]) -> Result<u64, ProgramError> {
    if values.is_empty() || values.len() > crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    insertion_sort_u64(values);
    let upper = values.len() / 2;
    if !values.len().is_multiple_of(2) {
        return Ok(values[upper]);
    }
    let lower = values[upper - 1];
    let upper = values[upper];
    Ok((lower & upper) + ((lower ^ upper) >> 1))
}

pub fn bucket_index_contribution_bps(
    bucket_weight_bps: u16,
    bucket_delta_bps: i64,
) -> Result<i64, ProgramError> {
    if bucket_weight_bps == 0 || bucket_weight_bps > 10_000 {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    let value = i128::from(bucket_delta_bps)
        .checked_mul(i128::from(bucket_weight_bps))
        .ok_or(VaultError::ArithmeticOverflow)?
        / 10_000;
    i64::try_from(value).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(super) fn oracle_bucket_bounty_share(
    bounty: u64,
    bucket_weight_start_bps: u16,
    bucket_weight_bps: u16,
) -> Result<u64, ProgramError> {
    let end = bucket_weight_start_bps
        .checked_add(bucket_weight_bps)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if bucket_weight_bps == 0 || end > 10_000 {
        return Err(VaultError::InvalidOracleBountySweep.into());
    }
    let allocated = |weight| {
        u64::try_from(
            u128::from(bounty)
                .checked_mul(u128::from(weight))
                .ok_or(VaultError::ArithmeticOverflow)?
                / 10_000,
        )
        .map_err(|_| VaultError::ArithmeticOverflow)
    };
    allocated(end)?
        .checked_sub(allocated(bucket_weight_start_bps)?)
        .ok_or_else(|| VaultError::InvalidOracleBountySweep.into())
}

#[inline(always)]
fn is_utc_business_day(timestamp: u64) -> bool {
    // 1970-01-01 was Thursday. Map Monday..Sunday to 0..6.
    let epoch_day = timestamp / 86_400;
    ((epoch_day + 3) % 7) < 5
}

pub fn oracle_settlement_window_start(
    expiry_ts: u64,
    business_days: u8,
) -> Result<u64, ProgramError> {
    if expiry_ts == 0 || business_days == 0 {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    let mut cursor = expiry_ts;
    let mut remaining = business_days;
    while remaining > 0 {
        cursor = cursor
            .checked_sub(86_400)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if is_utc_business_day(cursor) {
            remaining -= 1;
        }
    }
    Ok(cursor)
}

pub fn minimum_oracle_bucket_eligible_sources(frozen_source_count: u16) -> u16 {
    3u16.max((frozen_source_count / 2) + (frozen_source_count % 2))
}

pub fn oracle_bucket_security_cap(
    active_source_count: u16,
    listing_bond: u64,
    support_at_risk_per_source: u64,
    kappa_bps: u16,
) -> Result<(u16, u64), ProgramError> {
    if active_source_count == 0
        || listing_bond == 0
        || support_at_risk_per_source == 0
        || kappa_bps == 0
        || kappa_bps > 5_000
    {
        return Err(VaultError::InvalidOracleSecurityBudget.into());
    }
    let threshold = active_source_count / 2 + 1;
    let per_source = listing_bond
        .checked_add(support_at_risk_per_source)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let gross = per_source
        .checked_mul(u64::from(threshold))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let cap = mul_bps(gross, kappa_bps)?;
    if cap == 0 {
        return Err(VaultError::InvalidOracleSecurityBudget.into());
    }
    Ok((threshold, cap))
}

pub(super) fn append_oracle_source_observation(
    source: &mut OracleSourceState,
    observations: &mut OracleSourceObservations,
    state: u64,
    source_time: u64,
    evidence_hash: &[u8; 32],
    archive_url_hash: &[u8; 32],
) -> ProgramResult {
    let index = usize::from(source.observation_count);
    if state == 0
        || source_time == 0
        || crate::bytes32_is_zero(evidence_hash)
        || crate::bytes32_is_zero(archive_url_hash)
        || index >= crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS
        || (index > 0 && source_time <= observations.source_times[index - 1])
    {
        return Err(VaultError::InvalidOracleObservation.into());
    }
    observations.states[index] = state;
    observations.source_times[index] = source_time;
    source.observation_count = source
        .observation_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    source.rolling_observation_hash = hashv(&[
        crate::constants::ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
        &source.rolling_observation_hash,
        source.month.as_ref(),
        &source.source_id,
        &[source.observation_count],
        &state.to_le_bytes(),
        &source_time.to_le_bytes(),
        evidence_hash,
        archive_url_hash,
    ])
    .to_bytes();
    Ok(())
}

pub(super) fn validate_oracle_observation_shape(
    source: &OracleSourceState,
    observations: &OracleSourceObservations,
) -> ProgramResult {
    let count = usize::from(source.observation_count);
    if count == 0
        || count > crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS
        || observations.states[0] != source.baseline_state
        || observations.states[count - 1] != source.current_state
        || crate::bytes32_is_zero(&source.rolling_observation_hash)
    {
        return Err(VaultError::InvalidOracleObservation.into());
    }
    for index in 0..count {
        if observations.states[index] == 0
            || observations.source_times[index] == 0
            || (index > 0
                && observations.source_times[index] <= observations.source_times[index - 1])
        {
            return Err(VaultError::InvalidOracleObservation.into());
        }
    }
    if observations.states[count..].iter().any(|value| *value != 0)
        || observations.source_times[count..]
            .iter()
            .any(|value| *value != 0)
    {
        return Err(VaultError::InvalidOracleObservation.into());
    }
    Ok(())
}

pub fn oracle_temporal_median_state(
    source: &OracleSourceState,
    observations: &OracleSourceObservations,
    window_start_ts: u64,
    window_end_ts: u64,
) -> Result<Option<u64>, ProgramError> {
    validate_oracle_observation_shape(source, observations)?;
    if window_start_ts == 0 || window_end_ts < window_start_ts {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    let mut values = [0u64; crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS];
    let mut value_count = 0usize;
    for index in 0..usize::from(source.observation_count) {
        let source_time = observations.source_times[index];
        if (window_start_ts..=window_end_ts).contains(&source_time) {
            values[value_count] = observations.states[index];
            value_count += 1;
        }
    }
    if value_count > 0 {
        return Ok(Some(deterministic_temporal_median(
            &mut values[..value_count],
        )?));
    }

    // An unchanged source remains economically present without manufacturing a paid update.
    // This fallback does not add an equal-weight sample to an already-populated window, so every
    // source that was settleable under the existing temporal-median rule keeps the same result.
    Ok((0..usize::from(source.observation_count))
        .rev()
        .find(|index| observations.source_times[*index] <= window_start_ts)
        .map(|index| observations.states[index]))
}

pub(super) fn initial_oracle_bucket_source_snapshot(
    month: &Pubkey,
    bucket_id: &[u8; 32],
    mode: u8,
) -> [u8; 32] {
    hashv(&[
        crate::constants::ORACLE_BUCKET_MEDIAN_HASH_DOMAIN,
        month.as_ref(),
        bucket_id,
        &[mode],
    ])
    .to_bytes()
}

pub(super) fn advance_oracle_bucket_source_snapshot(
    previous: &[u8; 32],
    source: &OracleSourceState,
) -> [u8; 32] {
    hashv(&[
        crate::constants::ORACLE_BUCKET_MEDIAN_HASH_DOMAIN,
        previous,
        &source.source_id,
        &source.bucket_id,
        &source.baseline_state.to_le_bytes(),
        &[source.observation_count],
        &source.rolling_observation_hash,
    ])
    .to_bytes()
}

pub(super) fn update_month_bucket_contribution(
    month: &mut OracleMonthState,
    bucket: &mut OracleBucketMedianState,
    new_delta_bps: i64,
) -> ProgramResult {
    let contribution = bucket_index_contribution_bps(bucket.bucket_weight_bps, new_delta_bps)?;
    let previous =
        bucket_index_contribution_bps(bucket.bucket_weight_bps, bucket.bucket_delta_bps)?;
    let next = i128::from(month.index_delta_bps)
        .checked_sub(i128::from(previous))
        .and_then(|value| value.checked_add(i128::from(contribution)))
        .ok_or(VaultError::ArithmeticOverflow)?;
    month.index_delta_bps = i64::try_from(next).map_err(|_| VaultError::ArithmeticOverflow)?;
    bucket.bucket_delta_bps = new_delta_bps;
    Ok(())
}
