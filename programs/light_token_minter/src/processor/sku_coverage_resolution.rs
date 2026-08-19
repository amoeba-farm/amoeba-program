use super::*;

#[inline(never)]
pub(super) fn process_resolve_oracle_source_challenge(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ResolveOracleSourceChallengeParams,
) -> ProgramResult {
    if accounts.len() < 5 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    let challenge_info = &accounts[4];
    validate_oracle_authority(program_id, authority_info, config_info)?;
    let trailing = &accounts[5..];
    let mut month = load_oracle_month_state(month_info, program_id)?;
    ensure_oracle_resolution_freeze_window(&month)?;
    if month.weight_scheme_version == 255 {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    let mut challenge =
        load_valid_oracle_source_challenge(program_id, month_info.key, challenge_info)?;
    if challenge.source != *source_info.key || challenge.status != OracleChallengeStatus::RuleReview
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let outcome_account_count = match params.outcome {
        OracleSourceChallengeOutcome::RuleReviewUnresolved => 2,
        // comparison source, exact SKU pool, challenged-source reward, comparison-source reward
        OracleSourceChallengeOutcome::MergeSource => 4,
        _ => 0,
    };
    let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
        && !crate::bytes32_is_zero(&challenge.comparison_source_id);
    let guard_count = if has_comparison { 2 } else { 1 };
    if trailing.len() != outcome_account_count + guard_count + 2 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let outcome_account_info = if outcome_account_count > 0 {
        Some(&trailing[0])
    } else {
        None
    };
    let samba_mint_info = if params.outcome == OracleSourceChallengeOutcome::RuleReviewUnresolved {
        Some(&trailing[1])
    } else {
        None
    };
    let merge_reward_context = if params.outcome == OracleSourceChallengeOutcome::MergeSource {
        Some((&trailing[1], &trailing[2], &trailing[3]))
    } else {
        None
    };
    let source_guard_info = &trailing[outcome_account_count];
    if !source_guard_info.is_writable
        || source_guard_info.key == source_info.key
        || source_guard_info.key == challenge_info.key
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let source_guard = load_canonical_source_challenge_guard(
        program_id,
        month_info.key,
        source_info.key,
        &source.source_id,
        source_guard_info,
    )?;
    if source_guard.active_challenge != *challenge_info.key
        || source_guard.active_challenge_id != challenge.challenge_id
        || !crate::pubkey_is_default(&source_guard.active_dispute)
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    let mut comparison_guard = None;
    if has_comparison {
        let comparison_guard_info = &trailing[outcome_account_count + 1];
        if !comparison_guard_info.is_writable
            || comparison_guard_info.key == source_guard_info.key
            || comparison_guard_info.key == challenge_info.key
            || comparison_guard_info.key == source_info.key
        {
            return Err(VaultError::InvalidOracleChallengeAccount.into());
        }
        let guard = load_canonical_source_challenge_guard(
            program_id,
            month_info.key,
            &challenge.comparison_source,
            &challenge.comparison_source_id,
            comparison_guard_info,
        )?;
        if guard.active_challenge != *challenge_info.key
            || guard.active_challenge_id != challenge.challenge_id
            || !crate::pubkey_is_default(&guard.active_dispute)
        {
            return Err(VaultError::InvalidOracleChallengeAccount.into());
        }
        comparison_guard = Some((guard, comparison_guard_info));
    }
    let coverage_info = &trailing[outcome_account_count + guard_count];
    let coverage_record_info = &trailing[outcome_account_count + guard_count + 1];
    let mut coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let source_contributes_to_coverage =
        source.status == OracleSourceStatus::Candidate && source.support_stake_total > 0;
    let mut coverage_record = load_valid_oracle_sku_coverage_record(
        program_id,
        month_info.key,
        &source.bucket_id,
        coverage_record_info,
    )?;
    if source_contributes_to_coverage && coverage_record.active_supported_source_count == 0 {
        return Err(VaultError::InvalidOracleSkuCoverageRecord.into());
    }
    let mut terminalized_moot_challenge = source.status != OracleSourceStatus::Candidate;
    if terminalized_moot_challenge {
        challenge.status = OracleChallengeStatus::Rejected;
    } else {
        match params.outcome {
            OracleSourceChallengeOutcome::KeepSource => {
                challenge.status = OracleChallengeStatus::Rejected;
            }
            OracleSourceChallengeOutcome::RejectSource => {
                if source.support_stake_total > 0 {
                    decrement_oracle_supported_candidate_count(&mut month)?;
                    remove_supported_source_from_sku_coverage(&mut coverage, &mut coverage_record)?;
                }
                source.status = OracleSourceStatus::Rejected;
                challenge.status = OracleChallengeStatus::Accepted;
            }
            OracleSourceChallengeOutcome::RuleReviewUnresolved => {
                let staking_pool_info =
                    outcome_account_info.ok_or(VaultError::InvalidAccountList)?;
                let mut staking_pool = load_canonical_oracle_staking_pool(
                    program_id,
                    staking_pool_info,
                    &derive_oracle_major_token_config_pda(program_id).0,
                )?;
                let samba_mint = validate_oracle_samba_mint(
                    &staking_pool,
                    samba_mint_info.ok_or(VaultError::InvalidAccountList)?,
                    config_info.key,
                )?;
                let slot = Clock::get()?.slot;
                challenge.status = OracleChallengeStatus::RuleReviewUnresolved;
                challenge.rule_review_slot = slot;
                challenge.emergency_snapshot_total_major_tokens =
                    prepare_oracle_samba_voting_snapshot(
                        &mut staking_pool,
                        samba_mint.supply,
                        slot,
                    )?;
                challenge.emergency_snapshot_version = 2;
                store_state(staking_pool_info, &staking_pool)?;
            }
            OracleSourceChallengeOutcome::MergeSource => {
                if challenge.reason != ORACLE_SOURCE_REASON_NON_INDEPENDENT
                    || crate::pubkey_is_default(&challenge.comparison_source)
                {
                    return Err(VaultError::InvalidOracleChallengeAccount.into());
                }
                let comparison_source_info =
                    outcome_account_info.ok_or(VaultError::InvalidAccountList)?;
                if *comparison_source_info.key != challenge.comparison_source {
                    return Err(VaultError::InvalidOracleSourceAccount.into());
                }
                let mut comparison_source =
                    load_valid_oracle_source(program_id, month_info.key, comparison_source_info)?;
                if comparison_source.source_id != challenge.comparison_source_id {
                    return Err(VaultError::InvalidOracleSourceAccount.into());
                }
                if comparison_source.status != OracleSourceStatus::Candidate {
                    challenge.status = OracleChallengeStatus::Rejected;
                    terminalized_moot_challenge = true;
                } else {
                    let both_sources_supported =
                        source.support_stake_total > 0 && comparison_source.support_stake_total > 0;
                    let (sku_info, source_reward_info, comparison_reward_info) =
                        merge_reward_context.ok_or(VaultError::InvalidAccountList)?;
                    oracle_usdc::merge_oracle_usdc_source_rewards(
                        program_id,
                        month_info.key,
                        sku_info,
                        source_info,
                        &source,
                        source_reward_info,
                        comparison_source_info,
                        &comparison_source,
                        comparison_reward_info,
                    )?;
                    apply_oracle_source_merge(&mut source, &mut comparison_source)?;
                    if both_sources_supported {
                        decrement_oracle_supported_candidate_count(&mut month)?;
                        remove_supported_source_from_sku_coverage(
                            &mut coverage,
                            &mut coverage_record,
                        )?;
                    }
                    challenge.status = OracleChallengeStatus::Accepted;
                    store_state(comparison_source_info, &comparison_source)?;
                }
            }
        }
    }
    if terminalized_moot_challenge {
        decrement_pending_oracle_resolution(&mut month)?;
    } else {
        decrement_pending_oracle_source_challenge_if_terminal(&mut month, params.outcome)?;
    }
    let slot = Clock::get()?.slot;
    month.last_updated_slot = slot;
    coverage.last_updated_slot = slot;
    coverage_record.last_updated_slot = slot;
    if terminalized_moot_challenge
        || params.outcome != OracleSourceChallengeOutcome::RuleReviewUnresolved
    {
        let mut source_guard = source_guard;
        release_source_challenge_guard(
            &mut source_guard,
            challenge_info.key,
            &challenge.challenge_id,
            &Pubkey::default(),
            slot,
        )?;
        store_state(source_guard_info, &source_guard)?;
        if let Some((mut guard, guard_info)) = comparison_guard {
            release_source_challenge_guard(
                &mut guard,
                challenge_info.key,
                &challenge.challenge_id,
                &Pubkey::default(),
                slot,
            )?;
            store_state(guard_info, &guard)?;
        }
    }
    store_state(source_info, &source)?;
    store_state(challenge_info, &challenge)?;
    store_state(coverage_info, &coverage)?;
    store_state(coverage_record_info, &coverage_record)?;
    store_oracle_month_state(month_info, &month)
}
