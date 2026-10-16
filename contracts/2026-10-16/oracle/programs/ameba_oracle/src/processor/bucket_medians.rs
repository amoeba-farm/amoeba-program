use super::*;

pub(super) const RECOMPUTE_SOURCE_ACCOUNT_INDEX: u8 = 4;

/// The eligible sources a bucket needs to settle from its own median. The localnet profiles
/// relax the floor only for the dev months, whose sources tag 241 made (one per bucket); every
/// other month, a CFM launch month (tag 130) included, keeps the ordinary floor there too, so its
/// short buckets go to the council as they would in production.
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
fn recompute_floor(key: &Pubkey, source: &OracleSourceState, frozen_source_count: u16) -> u16 {
    if super::dev_bootstrap::is_dev_bootstrapped_source(key, source) {
        bucket_settlement_floor(frozen_source_count)
    } else {
        minimum_oracle_bucket_eligible_sources(frozen_source_count)
    }
}

#[cfg(not(any(feature = "localnet-dev-authority", feature = "localnet-governance")))]
fn recompute_floor(_key: &Pubkey, _source: &OracleSourceState, frozen_source_count: u16) -> u16 {
    bucket_settlement_floor(frozen_source_count)
}

pub(super) fn recompute_core_account_count(mode: u8) -> Option<usize> {
    match mode {
        0 => Some(10),
        1 => Some(10),
        _ => None,
    }
}

/// Recompute one bucket as an ordered, one-source-per-call walk.
///
/// Accounts are `[cranker, period, month, bucket, source, observations]` followed by the
/// authenticated recipe/source index and aggregation accounts in both modes. No token
/// electorate is loaded in grace mode. Walk every frozen source in
/// authenticated order, including inactive ones; only active sources contribute observations.
/// For an inactive source, observations is its absent canonical observations PDA.
/// One source per call preserves the full temporal median within the packet cap.
#[inline(never)]
pub(super) fn process_recompute_oracle_bucket_median_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RecomputeOracleBucketMedianV1Params,
) -> ProgramResult {
    let expected_accounts =
        recompute_core_account_count(params.mode).ok_or(OracleError::InvalidInstructionData)?;
    let grace = params.mode == 1;
    if accounts.len() != expected_accounts
        || !accounts[0].is_signer
        || accounts[1].is_signer
        || accounts[1].is_writable
        || accounts[2].is_signer
        || !accounts[2].is_writable
        || accounts[3].is_signer
        || !accounts[3].is_writable
        || accounts[4].is_signer
        // The source is passed writable but never written here.
        || !accounts[4].is_writable
        || accounts[5].is_signer
        || accounts[5].is_writable
        || crate::bytes32_is_zero(&params.bucket_id)
    {
        return Err(OracleError::InvalidAccountList.into());
    }

    let period_info = &accounts[1];
    let month_info = &accounts[2];
    let bucket_info = &accounts[3];
    let source_info = &accounts[usize::from(RECOMPUTE_SOURCE_ACCOUNT_INDEX)];
    let observations_info = &accounts[5];
    let (anchor, mut month) =
        load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    if month.settlement_status == OracleSettlementStatus::Final || month.finalized_at_ts != 0 {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    if month.phase != OraclePhase::Game {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    ensure_oracle_opening_resolution_complete(&month)?;

    let membership = oracle_membership::load_complete_bucket_source_index(
        program_id,
        month_info.key,
        &month,
        &params.bucket_id,
        &accounts[expected_accounts - 4],
        &accounts[expected_accounts - 3],
    )?;

    let mut bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
    if bucket.bucket_id != params.bucket_id
        || bucket.active_source_count == 0
        || bucket.active_source_count > membership.source_count
        || bucket.frozen_source_count != membership.source_count
        || bucket.group_index != membership.group_index
        || bucket.bucket_weight_bps != membership.bucket_weight_bps
    {
        return Err(OracleError::InvalidOracleMedian.into());
    }
    let valid_status = if grace {
        bucket.status == OracleBucketMedianStatus::GraceRequired
    } else {
        matches!(
            bucket.status,
            OracleBucketMedianStatus::Live | OracleBucketMedianStatus::Dirty
        )
    };
    if bucket.recompute_processed_source_count == 0 {
        if !valid_status {
            return Err(OracleError::InvalidOracleMedian.into());
        }
        bucket.eligible_source_count = 0;
        bucket.last_recompute_source_id = [0; 32];
        bucket.source_snapshot_hash =
            initial_oracle_bucket_source_snapshot(month_info.key, &params.bucket_id, params.mode);
        bucket.opening_source_deltas_bps.fill(0);
    } else if !valid_status
        || bucket.recompute_processed_source_count >= membership.source_count
        || crate::bytes32_is_zero(&bucket.last_recompute_source_id)
    {
        return Err(OracleError::InvalidOracleMedian.into());
    }

    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    if !matches!(
        source.status,
        OracleSourceStatus::Active | OracleSourceStatus::Inactive
    ) || source.bucket_id != params.bucket_id
        || source.bucket_weight_bps != bucket.bucket_weight_bps
        || (!crate::bytes32_is_zero(&bucket.last_recompute_source_id)
            && source.source_id <= bucket.last_recompute_source_id)
    {
        return Err(OracleError::InvalidOracleMedian.into());
    }
    oracle_membership::require_member(
        program_id,
        accounts[expected_accounts - 3].key,
        &membership,
        &accounts[expected_accounts - 2],
        bucket.recompute_processed_source_count,
        &source.source_id,
    )?;
    let clock = crate::compact_error::clock()?;
    let now = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(OracleError::InvalidOracleMedian))?;
    ensure_settlement_finalization_ready_at(anchor.expiry_ts, now)?;
    let days = crate::constants::ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS
        .checked_add(if grace {
            crate::constants::ORACLE_SETTLEMENT_FRESHNESS_GRACE_BUSINESS_DAYS
        } else {
            0
        })
        .ok_or(OracleError::ArithmeticOverflow)?;
    let window_start = oracle_settlement_window_start(anchor.expiry_ts, days)?;
    let temporal_state = if source.status == OracleSourceStatus::Active {
        if source.observation_count > crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS as u32 {
            carry::verified_history_median(
                program_id,
                observations_info,
                source_info.key,
                &source,
                window_start,
                anchor.expiry_ts,
                params.mode,
            )?
        } else {
            let observations = load_valid_oracle_source_observations(
                program_id,
                month_info.key,
                source_info.key,
                observations_info,
            )?;
            oracle_temporal_median_state(&source, &observations, window_start, anchor.expiry_ts)?
        }
    } else {
        if source.observation_count != 0
            || !crate::bytes32_is_zero(&source.rolling_observation_hash)
            || source.opening_submitted
            || !crate::bytes32_is_zero(&source.opening_evidence_hash)
            || source.baseline_state != 0
            || source.current_state != 0
            || *observations_info.key
                != derive_oracle_source_observations_pda(program_id, source_info.key).0
            || observations_info.executable
        {
            return Err(OracleError::InvalidOracleObservation.into());
        }
        // An inactive source never recorded an observation: its canonical observations account
        // is absent, or it is the empty account its proposal created.
        if !(crate::is_system_program(observations_info.owner) && observations_info.data_len() == 0)
        {
            let observations = load_valid_oracle_source_observations(
                program_id,
                month_info.key,
                source_info.key,
                observations_info,
            )?;
            if observations.states.iter().any(|state| *state != 0)
                || observations.source_times.iter().any(|time| *time != 0)
            {
                return Err(OracleError::InvalidOracleObservation.into());
            }
        }
        None
    };
    if let Some(state) = temporal_state {
        let index = usize::from(bucket.eligible_source_count);
        if index < crate::constants::INLINE_ORACLE_BUCKET_MEDIAN_CAPACITY {
            bucket.opening_source_deltas_bps[index] =
                source_delta_bps(source.baseline_state, state)?;
        }
        bucket.eligible_source_count = bucket
            .eligible_source_count
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?;
    }
    if source.status == OracleSourceStatus::Active {
        bucket.source_snapshot_hash =
            advance_oracle_bucket_source_snapshot(&bucket.source_snapshot_hash, &source);
    }
    bucket.last_recompute_source_id = source.source_id;
    bucket.recompute_processed_source_count = bucket
        .recompute_processed_source_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    if bucket.recompute_processed_source_count < membership.source_count {
        return store_state(bucket_info, &bucket);
    }
    if bucket.recompute_processed_source_count != membership.source_count {
        return Err(OracleError::InvalidOracleMedian.into());
    }

    bucket.last_recomputed_ts = now;
    let minimum = recompute_floor(source_info.key, &source, bucket.frozen_source_count);
    let mut measured = None;
    if bucket.eligible_source_count < minimum {
        if !grace {
            bucket.status = OracleBucketMedianStatus::GraceRequired;
        } else {
            bucket.council_authority_version = 1;
            bucket.emergency_snapshot_slot = clock.slot;
            bucket.status = OracleBucketMedianStatus::EmergencyRequired;
            measured = measured_council_median(&bucket)?;
        }
    } else {
        let eligible = usize::from(bucket.eligible_source_count);
        let mut values = bucket.opening_source_deltas_bps;
        let new_delta = if eligible <= crate::constants::INLINE_ORACLE_BUCKET_MEDIAN_CAPACITY {
            deterministic_bucket_median(&mut values[..eligible])?
        } else {
            carry::verified_bucket_rank(
                program_id,
                &accounts[expected_accounts - 1],
                accounts[expected_accounts - 3].key,
                month_info.key,
                &bucket.source_snapshot_hash,
                bucket.frozen_source_count,
                bucket.eligible_source_count,
                params.mode,
            )?
        };
        update_month_bucket_contribution(&mut month, &mut bucket, new_delta)?;
        bucket.status = OracleBucketMedianStatus::SettlementReady;
        month.last_updated_slot = clock.slot;
        store_oracle_month_state(month_info, &month)?;
    }
    bucket.recompute_processed_source_count = 0;
    bucket.last_recompute_source_id = [0; 32];
    bucket.opening_source_deltas_bps.fill(0);
    if let Some(median) = measured {
        bucket.opening_source_deltas_bps[0] = median;
    }
    store_state(bucket_info, &bucket)
}

/// The median the council may confirm for a bucket the grace walk leaves below its floor: the
/// eligible sources' temporal-median deltas, as a settling bucket's median is computed. It is
/// kept in slot 0 of `opening_source_deltas_bps` (the other slots stay zero), so the case's
/// evidence hash commits to it. A bucket with no eligible source, or more eligible sources than
/// the inline median holds, has none; its council keeps the two choices that retain the last
/// accepted value.
fn measured_council_median(bucket: &OracleBucketMedianState) -> Result<Option<i64>, ProgramError> {
    let eligible = usize::from(bucket.eligible_source_count);
    if eligible == 0 || eligible > crate::constants::INLINE_ORACLE_BUCKET_MEDIAN_CAPACITY {
        return Ok(None);
    }
    let mut values = bucket.opening_source_deltas_bps;
    Ok(Some(deterministic_bucket_median(&mut values[..eligible])?))
}

/// The measured median a bucket in EmergencyRequired holds for the council, if it has one.
pub(super) fn council_measured_median(bucket: &OracleBucketMedianState) -> Option<i64> {
    let eligible = usize::from(bucket.eligible_source_count);
    (eligible > 0 && eligible <= crate::constants::INLINE_ORACLE_BUCKET_MEDIAN_CAPACITY)
        .then_some(bucket.opening_source_deltas_bps[0])
}
