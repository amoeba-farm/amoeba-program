use super::*;

pub(super) fn derive_market_pda(program_id: &Pubkey, market_id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CURRENT_STATE_NAMESPACE_SEED, MARKET_PDA_SEED, market_id],
        program_id,
    )
}

#[inline(never)]
pub(super) fn derive_vault_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED], program_id)
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

pub(super) fn derive_contract_mint_pda(program_id: &Pubkey, market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            CONTRACT_MINT_PDA_SEED,
            market.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_contract_mint_staging_pda(
    program_id: &Pubkey,
    market: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            CONTRACT_MINT_STAGING_PDA_SEED,
            market.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_writer_policy_registry_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_POLICY_REGISTRY_PDA_SEED,
        ],
        program_id,
    )
}

pub(super) fn derive_writer_settlement_group_pda(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
    expiry_ts: u64,
    settlement_mint: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_SETTLEMENT_GROUP_PDA_SEED,
            underlying_id,
            &expiry_ts.to_le_bytes(),
            settlement_mint.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_writer_sleeve_pda(program_id: &Pubkey, group: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_SLEEVE_PDA_SEED,
            group.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_writer_series_book_pda(program_id: &Pubkey, sleeve: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
            sleeve.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_writer_policy_snapshot_pda(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    policy_version: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_POLICY_SNAPSHOT_PDA_SEED,
            sleeve.as_ref(),
            &policy_version.to_le_bytes(),
        ],
        program_id,
    )
}

pub(super) fn derive_writer_sleeve_usdc_vault_pda(
    program_id: &Pubkey,
    sleeve: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_SLEEVE_USDC_VAULT_PDA_SEED,
            sleeve.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_writer_retirement_custody_pda(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    market: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_RETIREMENT_CUSTODY_PDA_SEED,
            sleeve.as_ref(),
            market.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_oracle_month_pda(
    program_id: &Pubkey,
    market: &Pubkey,
    expiry_ts: u64,
) -> (Pubkey, u8) {
    let expiry_seed = expiry_ts.to_le_bytes();
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_MONTH_PDA_SEED,
            market.as_ref(),
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
        return Err(VaultError::InvalidOracleChallengeAccount.into());
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

pub(super) fn derive_user_collateral_pda(program_id: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            USER_COLLATERAL_PDA_SEED,
            owner.as_ref(),
        ],
        program_id,
    )
}

pub(super) fn derive_settlement_v2_pda(
    program_id: &Pubkey,
    market: &Pubkey,
    oracle_month: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            SETTLEMENT_V2_PDA_SEED,
            market.as_ref(),
            oracle_month.as_ref(),
        ],
        program_id,
    )
}

#[inline(never)]
pub(super) fn settlement_signed_leaf_commitment(
    settlement: &CompressedSettlementLeaf,
) -> Result<[u8; 32], ProgramError> {
    let payload = settlement
        .signed_payload_bytes()
        .map_err(|_| VaultError::InvalidSettlementRecord)?;
    Ok(hashv(&[b"ameba-settlement-record-v2", payload.as_slice()]).to_bytes())
}

pub(super) fn settlement_record_v2_from_leaf(
    bump: u8,
    market: Pubkey,
    oracle_month: Pubkey,
    settlement: &CompressedSettlementLeaf,
    signed_leaf_commitment: [u8; 32],
    submitted_by: Pubkey,
) -> SettlementRecordV2 {
    SettlementRecordV2 {
        is_initialized: true,
        bump,
        account_discriminator: SettlementRecordV2::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementRecordV2::ACCOUNT_VERSION,
        market,
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
    market: &Pubkey,
    oracle_month: &Pubkey,
    settlement: &CompressedSettlementLeaf,
    signed_leaf_commitment: &[u8; 32],
    submitted_by: &Pubkey,
) -> bool {
    record.is_initialized
        && record.account_discriminator == SettlementRecordV2::ACCOUNT_DISCRIMINATOR
        && record.account_version == SettlementRecordV2::ACCOUNT_VERSION
        && record.signer_set_version == settlement.signer_set_version
        && record.market == *market
        && record.oracle_month == *oracle_month
        && record.settlement_ts == settlement.settlement_ts
        && record.settlement_price_atomic == settlement.settlement_price_atomic
        && record.signed_leaf_commitment == *signed_leaf_commitment
        && record.submitted_by == *submitted_by
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
        VaultError::InvalidUserCollateralAccount,
    )?;
    let (expected, bump) = derive_user_collateral_pda(program_id, expected_owner);
    if !collateral.is_initialized
        || *collateral_info.key != expected
        || collateral.bump != bump
        || collateral.owner != *expected_owner
    {
        return Err(VaultError::InvalidUserCollateralAccount.into());
    }
    Ok(collateral)
}

pub(super) fn load_or_create_settlement_record_v2<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    oracle_month_info: &AccountInfo<'a>,
    settlement_record_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<(Option<SettlementRecordV2>, u8), ProgramError> {
    let (expected, bump) =
        derive_settlement_v2_pda(program_id, market_info.key, oracle_month_info.key);
    if *settlement_record_info.key != expected {
        return Err(VaultError::InvalidSettlementAccount.into());
    }
    if settlement_record_info.owner == program_id {
        let record: SettlementRecordV2 = load_exact_zero_padded_state(
            settlement_record_info,
            program_id,
            SettlementRecordV2::LEN,
            VaultError::InvalidSettlementAccount,
        )?;
        if !record.is_initialized
            || record.bump != bump
            || record.account_discriminator != SettlementRecordV2::ACCOUNT_DISCRIMINATOR
            || record.account_version != SettlementRecordV2::ACCOUNT_VERSION
            || record.market != *market_info.key
            || record.oracle_month != *oracle_month_info.key
            || crate::bytes32_is_zero(&record.signed_leaf_commitment)
            || crate::pubkey_is_default(&record.submitted_by)
            || record.signer_set_version == 0
        {
            return Err(VaultError::InvalidSettlementAccount.into());
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
            market_info.key.as_ref(),
            oracle_month_info.key.as_ref(),
            &[bump],
        ],
    )?;
    Ok((None, bump))
}

pub(super) fn load_canonical_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    let (expected_config, expected_bump) = derive_vault_config_pda(program_id);
    if *config_info.key != expected_config {
        return Err(VaultError::InvalidPda.into());
    }
    if config_info.owner != program_id {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    if config_info.data_len() != VaultConfig::LEN {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    let config: VaultConfig = load_exact_zero_padded_state(
        config_info,
        program_id,
        VaultConfig::LEN,
        VaultError::InvalidConfigAccount,
    )?;
    if !config.has_current_layout() {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    if !config.is_initialized || config.bump != expected_bump {
        return Err(VaultError::InvalidConfigAccount.into());
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
        VaultError::InvalidSettlementSignerRegistry,
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
        return Err(VaultError::InvalidSettlementSignerRegistry.into());
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
        return Err(VaultError::InvalidSettlementSignerConfiguration.into());
    }
    let active = &set.signers[..signer_count];
    if active.iter().any(crate::pubkey_is_default)
        || active.windows(2).any(|pair| pair[0] >= pair[1])
        || set.signers[signer_count..]
            .iter()
            .any(|signer| !crate::pubkey_is_default(signer))
        || set.compute_set_hash() != set.set_hash
    {
        return Err(VaultError::InvalidSettlementSignerConfiguration.into());
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
        VaultError::InvalidSettlementSignerSet,
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
        return Err(VaultError::InvalidSettlementSignerSet.into());
    }
    validate_settlement_signer_configuration(&set)?;
    Ok(set)
}

pub(super) fn load_active_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
    mint_info: &AccountInfo,
    vault_token_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    if *mint_info.key != config.usdc_mint {
        return Err(VaultError::InvalidMint.into());
    }
    if *vault_token_info.key != config.vault_token_account {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    Ok(config)
}
