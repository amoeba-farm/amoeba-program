use super::*;

#[inline(never)]
pub(super) fn derive_vault_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    crate::state::find_config_address(program_id)
}

#[inline(never)]
pub(super) fn derive_oracle_economics_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_ECONOMICS_CONFIG_PDA_SEED,
        ],
        program_id,
    )
}

/// One month per period: `[namespace, "g3-oracle_month", period, expiry_le]`.
pub(super) fn derive_oracle_month_pda(
    program_id: &Pubkey,
    period: &Pubkey,
    expiry_ts: u64,
) -> (Pubkey, u8) {
    let expiry_seed = expiry_ts.to_le_bytes();
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_MONTH_PDA_SEED,
            period.as_ref(),
            &expiry_seed,
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_source_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    source_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_SOURCE_PDA_SEED,
            month.as_ref(),
            source_id,
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_source_observations_pda(
    program_id: &Pubkey,
    source: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
            source.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_support_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    supporter: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_SUPPORT_POSITION_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
            supporter.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_source_challenge_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    challenge_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_SOURCE_CHALLENGE_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
            challenge_id,
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_opening_claim_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_OPENING_CLAIM_PDA_SEED,
            month.as_ref(),
            source.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_opening_claim_challenge_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
    challenge_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_OPENING_CLAIM_CHALLENGE_PDA_SEED,
            month.as_ref(),
            claim.as_ref(),
            challenge_id,
        ],
        program_id,
    )
}

/// Council targets bind opening challenges through this.
pub(super) fn validate_oracle_opening_claim_challenge_pda(
    program_id: &Pubkey,
    challenge_key: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
    challenge_id: &[u8; 32],
) -> ProgramResult {
    let (expected_challenge, _) =
        derive_oracle_opening_claim_challenge_pda(program_id, month, claim, challenge_id);
    if expected_challenge != *challenge_key {
        return Err(OracleError::InvalidOracleChallengeAccount.into());
    }
    Ok(())
}

pub(super) fn derive_oracle_update_challenge_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
    challenge_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_UPDATE_CHALLENGE_PDA_SEED,
            month.as_ref(),
            claim.as_ref(),
            challenge_id,
        ],
        program_id,
    )
}

pub(super) fn load_canonical_user_collateral(
    program_id: &Pubkey,
    collateral_info: &AccountInfo,
    expected_owner: &Pubkey,
) -> Result<UserCollateral, ProgramError> {
    let collateral: UserCollateral = load_exact_zero_padded_state(
        collateral_info,
        program_id,
        UserCollateral::LEN,
        OracleError::InvalidUserCollateralAccount,
    )?;
    let (expected, bump) = derive_user_collateral_pda(program_id, expected_owner);
    if !collateral.is_initialized
        || *collateral_info.key != expected
        || collateral.bump != bump
        || collateral.owner != *expected_owner
    {
        return Err(OracleError::InvalidUserCollateralAccount.into());
    }
    Ok(collateral)
}

pub(super) fn load_canonical_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<OracleConfig, ProgramError> {
    let (expected_config, expected_bump) = derive_vault_config_pda(program_id);
    if *config_info.key != expected_config {
        return Err(OracleError::InvalidPda.into());
    }
    if config_info.owner != program_id {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    if config_info.data_len() != OracleConfig::LEN {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    let config: OracleConfig = load_exact_zero_padded_state(
        config_info,
        program_id,
        OracleConfig::LEN,
        OracleError::InvalidConfigAccount,
    )?;
    if !config.has_current_layout() {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    if !config.is_initialized || config.bump != expected_bump {
        return Err(OracleError::InvalidConfigAccount.into());
    }
    Ok(config)
}

pub(super) fn load_canonical_settlement_signer_registry(
    program_id: &Pubkey,
    registry_info: &AccountInfo,
) -> Result<SettlementSignerRegistry, ProgramError> {
    let (expected, bump) = derive_settlement_signer_registry_pda(program_id);
    let registry: SettlementSignerRegistry = load_exact_zero_padded_state(
        registry_info,
        program_id,
        SettlementSignerRegistry::LEN,
        OracleError::InvalidSettlementSignerRegistry,
    )?;
    if *registry_info.key != expected
        || !registry.is_initialized
        || registry.bump != bump
        || !registry.has_current_layout()
        || crate::pubkey_is_default(&registry.current_set)
        || registry.current_version == 0
        || crate::pubkey_is_default(&registry.recovery_authority)
        || (crate::pubkey_is_default(&registry.pending_set)) != (registry.pending_version == 0)
    {
        return Err(OracleError::InvalidSettlementSignerRegistry.into());
    }
    Ok(registry)
}

pub(super) fn validate_settlement_signer_configuration(set: &SettlementSignerSet) -> ProgramResult {
    let signer_count = usize::from(set.signer_count);
    let threshold = usize::from(set.threshold);
    if set.version == 0
        || signer_count == 0
        || signer_count > MAX_SETTLEMENT_SIGNER_COUNT
        || threshold == 0
        || threshold > signer_count
        || threshold.saturating_mul(2) <= signer_count
        || set.rotation_delay_slots < MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS
    {
        return Err(OracleError::InvalidSettlementSignerConfiguration.into());
    }
    let active = &set.signers[..signer_count];
    if active.iter().any(crate::pubkey_is_default)
        || active.windows(2).any(|pair| pair[0] >= pair[1])
        || set.signers[signer_count..]
            .iter()
            .any(|signer| !crate::pubkey_is_default(signer))
        || set.compute_set_hash() != set.set_hash
    {
        return Err(OracleError::InvalidSettlementSignerConfiguration.into());
    }
    Ok(())
}

pub(super) fn load_canonical_settlement_signer_set(
    program_id: &Pubkey,
    registry: &Pubkey,
    signer_set_info: &AccountInfo,
) -> Result<SettlementSignerSet, ProgramError> {
    let set: SettlementSignerSet = load_exact_zero_padded_state(
        signer_set_info,
        program_id,
        SettlementSignerSet::LEN,
        OracleError::InvalidSettlementSignerSet,
    )?;
    let (expected, bump) = derive_settlement_signer_set_pda(program_id, set.version);
    if *signer_set_info.key != expected
        || !set.is_initialized
        || set.bump != bump
        || !set.has_current_layout()
        || set.registry != *registry
        || crate::pubkey_is_default(&set.proposer)
        || set.activate_after_slot < set.proposed_slot
    {
        return Err(OracleError::InvalidSettlementSignerSet.into());
    }
    validate_settlement_signer_configuration(&set)?;
    Ok(set)
}

pub(super) fn load_active_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
    mint_info: &AccountInfo,
    vault_token_info: &AccountInfo,
) -> Result<OracleConfig, ProgramError> {
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused {
        return Err(OracleError::ContractPaused.into());
    }
    if *mint_info.key != config.usdc_mint {
        return Err(OracleError::InvalidMint.into());
    }
    if *vault_token_info.key != config.bond_vault {
        return Err(OracleError::InvalidTokenAccount.into());
    }
    Ok(config)
}

#[inline(never)]
pub(super) fn settlement_signed_leaf_commitment(
    settlement: &SettlementLeaf,
) -> Result<[u8; 32], ProgramError> {
    let payload = settlement
        .signed_payload_bytes()
        .map_err(|_| OracleError::InvalidSettlementRecord)?;
    Ok(hashv(&[b"ameba-settlement-record-v2", payload.as_slice()]).to_bytes())
}

pub(super) fn settlement_record_v2_from_leaf(
    bump: u8,
    period: Pubkey,
    oracle_month: Pubkey,
    settlement: &SettlementLeaf,
    signed_leaf_commitment: [u8; 32],
    submitted_by: Pubkey,
) -> SettlementRecordV2 {
    SettlementRecordV2 {
        is_initialized: true,
        bump,
        account_discriminator: SettlementRecordV2::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementRecordV2::ACCOUNT_VERSION,
        period,
        oracle_month,
        settlement_ts: settlement.settlement_ts,
        settlement_price_atomic: settlement.settlement_price_atomic,
        signed_leaf_commitment,
        submitted_by,
        signer_set_version: settlement.signer_set_version,
    }
}

pub(super) fn settlement_record_v2_matches_submission(
    record: &SettlementRecordV2,
    period: &Pubkey,
    oracle_month: &Pubkey,
    settlement: &SettlementLeaf,
    signed_leaf_commitment: &[u8; 32],
    submitted_by: &Pubkey,
) -> bool {
    record.is_initialized
        && record.account_discriminator == SettlementRecordV2::ACCOUNT_DISCRIMINATOR
        && record.account_version == SettlementRecordV2::ACCOUNT_VERSION
        && record.signer_set_version == settlement.signer_set_version
        && record.period == *period
        && record.oracle_month == *oracle_month
        && record.settlement_ts == settlement.settlement_ts
        && record.settlement_price_atomic == settlement.settlement_price_atomic
        && record.signed_leaf_commitment == *signed_leaf_commitment
        && record.submitted_by == *submitted_by
}

pub(super) fn load_or_create_settlement_record_v2<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    period_info: &AccountInfo<'a>,
    oracle_month_info: &AccountInfo<'a>,
    settlement_record_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<(Option<SettlementRecordV2>, u8), ProgramError> {
    let (expected, bump) =
        derive_settlement_v2_pda(program_id, period_info.key, oracle_month_info.key);
    if *settlement_record_info.key != expected {
        return Err(OracleError::InvalidSettlementAccount.into());
    }
    if settlement_record_info.owner == program_id {
        let record: SettlementRecordV2 = load_exact_zero_padded_state(
            settlement_record_info,
            program_id,
            SettlementRecordV2::LEN,
            OracleError::InvalidSettlementAccount,
        )?;
        if !record.is_initialized
            || record.bump != bump
            || record.account_discriminator != SettlementRecordV2::ACCOUNT_DISCRIMINATOR
            || record.account_version != SettlementRecordV2::ACCOUNT_VERSION
            || record.period != *period_info.key
            || record.oracle_month != *oracle_month_info.key
            || crate::bytes32_is_zero(&record.signed_leaf_commitment)
            || crate::pubkey_is_default(&record.submitted_by)
            || record.signer_set_version == 0
        {
            return Err(OracleError::InvalidSettlementAccount.into());
        }
        return Ok((Some(record), bump));
    }

    create_program_account(
        payer_info,
        settlement_record_info,
        system_program_info,
        program_id,
        SettlementRecordV2::LEN,
        &[
            SETTLEMENT_V2_PDA_SEED,
            period_info.key.as_ref(),
            oracle_month_info.key.as_ref(),
            &[bump],
        ],
    )?;
    Ok((None, bump))
}
