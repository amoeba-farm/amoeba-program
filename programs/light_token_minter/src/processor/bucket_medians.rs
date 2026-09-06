use super::*;

/// Recompute one bucket as an ordered, one-source-per-call walk.
///
/// Accounts are `[cranker, market, month, bucket, source, observations]`.
/// Grace mode appends `[vault_config, staking_pool, samba_mint]` so the final insufficient
/// source can freeze the emergency-court voting snapshot atomically. Keeping one compressed
/// source in each call preserves the full 32-print temporal median within Solana's packet cap.
#[inline(never)]
pub(super) fn process_recompute_oracle_bucket_median_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: RecomputeOracleBucketMedianV1Params,
) -> ProgramResult {
    if params.mode > 1 {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let grace = params.mode == 1;
    let expected_accounts = 6 + usize::from(grace) * 3;
    if accounts.len() != expected_accounts
        || !accounts[0].is_signer
        || accounts[1].is_signer
        || accounts[1].is_writable
        || accounts[2].is_signer
        || !accounts[2].is_writable
        || accounts[3].is_signer
        || !accounts[3].is_writable
        || accounts[4].is_signer
        || accounts[4].is_writable
        || accounts[5].is_signer
        || accounts[5].is_writable
        || (grace
            && (accounts[6].is_signer
                || accounts[6].is_writable
                || accounts[7].is_signer
                || !accounts[7].is_writable
                || accounts[8].is_signer
                || accounts[8].is_writable))
        || crate::bytes32_is_zero(&params.bucket_id)
    {
        return Err(VaultError::InvalidAccountList.into());
    }

    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let bucket_info = &accounts[3];
    let source_info = &accounts[4];
    let observations_info = &accounts[5];
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    if month.settlement_status == OracleSettlementStatus::Final || month.finalized_at_ts != 0 {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    if month.phase != OraclePhase::Game {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    ensure_oracle_opening_resolution_complete(&month)?;

    let mut bucket = load_valid_oracle_bucket_median(program_id, month_info.key, bucket_info)?;
    if bucket.bucket_id != params.bucket_id || bucket.active_source_count == 0 {
        return Err(VaultError::InvalidOracleMedian.into());
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
            return Err(VaultError::InvalidOracleMedian.into());
        }
        bucket.eligible_source_count = 0;
        bucket.last_recompute_source_id = [0; 32];
        bucket.source_snapshot_hash =
            initial_oracle_bucket_source_snapshot(month_info.key, &params.bucket_id, params.mode);
        bucket.opening_source_deltas_bps.fill(0);
    } else if !valid_status || bucket.recompute_processed_source_count >= bucket.active_source_count
    {
        return Err(VaultError::InvalidOracleMedian.into());
    }

    let source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    if source.status != OracleSourceStatus::Active
        || source.bucket_id != params.bucket_id
        || source.bucket_weight_bps != bucket.bucket_weight_bps
        || (!crate::bytes32_is_zero(&bucket.last_recompute_source_id)
            && source.source_id <= bucket.last_recompute_source_id)
    {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    let observations = load_valid_oracle_source_observations(
        program_id,
        month_info.key,
        source_info.key,
        observations_info,
    )?;
    let clock = Clock::get()?;
    let now = u64::try_from(clock.unix_timestamp)
        .map_err(|_| ProgramError::from(VaultError::InvalidOracleMedian))?;
    ensure_settlement_finalization_ready_at(market.instrument.expiry_ts, now)?;
    let days = crate::constants::ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS
        .checked_add(if grace {
            crate::constants::ORACLE_SETTLEMENT_FRESHNESS_GRACE_BUSINESS_DAYS
        } else {
            0
        })
        .ok_or(VaultError::ArithmeticOverflow)?;
    let window_start = oracle_settlement_window_start(market.instrument.expiry_ts, days)?;
    if let Some(state) = oracle_temporal_median_state(
        &source,
        &observations,
        window_start,
        market.instrument.expiry_ts,
    )? {
        let index = usize::from(bucket.eligible_source_count);
        if index >= crate::constants::MAX_ORACLE_BUCKET_SOURCES {
            return Err(VaultError::InvalidOracleMedian.into());
        }
        bucket.opening_source_deltas_bps[index] = source_delta_bps(source.baseline_state, state)?;
        bucket.eligible_source_count = bucket
            .eligible_source_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    bucket.source_snapshot_hash =
        advance_oracle_bucket_source_snapshot(&bucket.source_snapshot_hash, &source);
    bucket.last_recompute_source_id = source.source_id;
    bucket.recompute_processed_source_count = bucket
        .recompute_processed_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if bucket.recompute_processed_source_count < bucket.active_source_count {
        return store_state(bucket_info, &bucket);
    }
    if bucket.recompute_processed_source_count != bucket.active_source_count {
        return Err(VaultError::InvalidOracleMedian.into());
    }

    bucket.last_recomputed_ts = now;
    let minimum = minimum_oracle_bucket_eligible_sources(bucket.frozen_source_count);
    if bucket.eligible_source_count < minimum {
        if !grace {
            bucket.status = OracleBucketMedianStatus::GraceRequired;
        } else {
            let config_info = &accounts[6];
            let staking_pool_info = &accounts[7];
            let samba_mint_info = &accounts[8];
            let _config = load_current_canonical_vault_config(program_id, config_info)?;
            let mut staking_pool = load_canonical_oracle_staking_pool(
                program_id,
                staking_pool_info,
                &derive_oracle_major_token_config_pda(program_id).0,
            )?;
            let samba_mint =
                validate_oracle_samba_mint(&staking_pool, samba_mint_info, config_info.key)?;
            bucket.emergency_snapshot_total_samba = prepare_oracle_samba_voting_snapshot(
                &mut staking_pool,
                samba_mint.supply,
                clock.slot,
            )?;
            bucket.emergency_snapshot_slot = clock.slot;
            bucket.status = OracleBucketMedianStatus::EmergencyRequired;
            store_state(staking_pool_info, &staking_pool)?;
        }
    } else {
        let eligible = usize::from(bucket.eligible_source_count);
        let mut values = bucket.opening_source_deltas_bps;
        let new_delta = deterministic_bucket_median(&mut values[..eligible])?;
        update_month_bucket_contribution(&mut month, &mut bucket, new_delta)?;
        bucket.status = OracleBucketMedianStatus::SettlementReady;
        month.last_updated_slot = clock.slot;
        store_oracle_month_state(month_info, &month)?;
    }
    bucket.recompute_processed_source_count = 0;
    bucket.last_recompute_source_id = [0; 32];
    bucket.opening_source_deltas_bps.fill(0);
    store_state(bucket_info, &bucket)
}
