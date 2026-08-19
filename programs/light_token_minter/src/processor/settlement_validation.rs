use super::*;

pub(super) fn validate_text_field(
    value: &str,
    max_len: usize,
    allow_empty: bool,
    error: VaultError,
) -> Result<(), ProgramError> {
    let trimmed = value.trim();
    if !allow_empty && trimmed.is_empty() {
        return Err(error.into());
    }
    if trimmed.len() > max_len {
        return Err(error.into());
    }
    Ok(())
}

pub(super) fn validate_market_page(page: &CompressedMarketPageLeaf) -> Result<(), ProgramError> {
    validate_text_field(&page.item_id, 32, false, VaultError::InvalidMarketPage)?;
    if !page
        .item_id
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(VaultError::InvalidMarketPage.into());
    }
    validate_text_field(&page.name, 64, false, VaultError::InvalidMarketPage)?;
    validate_text_field(&page.symbol, 32, false, VaultError::InvalidMarketPage)?;
    validate_text_field(&page.title, 128, false, VaultError::InvalidMarketPage)?;
    validate_text_field(&page.subtitle, 160, false, VaultError::InvalidMarketPage)?;
    validate_text_field(&page.info_href, 160, true, VaultError::InvalidMarketPage)?;
    validate_text_field(&page.page_title, 160, false, VaultError::InvalidMarketPage)?;
    validate_text_field(
        &page.meta_description,
        280,
        false,
        VaultError::InvalidMarketPage,
    )?;

    if page.expiries.is_empty() || page.expiries.len() > 24 {
        return Err(VaultError::InvalidMarketPage.into());
    }

    for expiry in &page.expiries {
        validate_text_field(&expiry.id, 24, false, VaultError::InvalidMarketPage)?;
        validate_text_field(&expiry.label, 64, false, VaultError::InvalidMarketPage)?;
        if expiry.settlement_ts == 0 {
            return Err(VaultError::InvalidMarketPage.into());
        }
    }

    Ok(())
}

#[inline(never)]
pub(super) fn validate_settlement_market_binding(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    oracle_month_info: &AccountInfo,
    market: &Market,
    oracle_month: &OracleMonthState,
    settlement: &CompressedSettlementLeaf,
) -> Result<(), ProgramError> {
    ensure_settlement_timestamp_matches_expiry(
        market.instrument.expiry_ts,
        settlement.settlement_ts,
    )?;
    let (expected_month, _) =
        derive_oracle_month_pda(program_id, market_info.key, market.instrument.expiry_ts);
    if *oracle_month_info.key != expected_month
        || oracle_month.market != *market_info.key
        || settlement.underlying_id != market.instrument.underlying_id
    {
        return Err(VaultError::InvalidSettlementRecord.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn validate_settlement_leaf(
    settlement: &CompressedSettlementLeaf,
    oracle_month: &OracleMonthState,
) -> Result<(), ProgramError> {
    validate_text_field(
        &settlement.item_id,
        32,
        false,
        VaultError::InvalidSettlementRecord,
    )?;
    if !settlement
        .item_id
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(VaultError::InvalidSettlementRecord.into());
    }

    validate_text_field(
        &settlement.expiry_id,
        24,
        false,
        VaultError::InvalidSettlementRecord,
    )?;
    if !settlement
        .expiry_id
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(VaultError::InvalidSettlementRecord.into());
    }

    validate_text_field(
        &settlement.source_uri,
        256,
        false,
        VaultError::InvalidSettlementRecord,
    )?;

    if settlement.schema_version != CompressedSettlementLeaf::CURRENT_SCHEMA_VERSION
        || crate::pubkey_is_default(&settlement.oracle_month)
        || crate::bytes32_is_zero(&settlement.recipe_hash)
        || settlement.settlement_ts == 0
        || settlement.signer_set_version == 0
        || crate::bytes32_is_zero(&settlement.signer_set_hash)
    {
        return Err(VaultError::InvalidSettlementRecord.into());
    }

    if settlement.computation != SettlementComputation::SpreadOracleIndexDelta
        || settlement.trailing_window_days != 0
        || !settlement.observations.is_empty()
        || settlement.base_oracle_atomic == 0
        || oracle_month.settlement_base_oracle_atomic == 0
        || settlement.base_oracle_atomic != oracle_month.settlement_base_oracle_atomic
        || settlement.index_delta_bps != oracle_month.index_delta_bps
    {
        return Err(VaultError::InvalidSettlementRecord.into());
    }

    let computed = settlement
        .computed_spread_oracle_index_delta_price_atomic()
        .map_err(|_| VaultError::InvalidSettlementRecord)?;
    if settlement.settlement_price_atomic != computed {
        return Err(VaultError::SettlementRecordHashMismatch.into());
    }

    Ok(())
}

pub(super) const ED25519_SIGNATURE_OFFSETS_START: usize = 2;
pub(super) const ED25519_SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
pub(super) const ED25519_DATA_START: usize =
    ED25519_SIGNATURE_OFFSETS_START + ED25519_SIGNATURE_OFFSETS_SERIALIZED_SIZE;
pub(super) const ED25519_SIGNATURE_SERIALIZED_SIZE: usize = 64;
pub(super) const ED25519_PUBKEY_SERIALIZED_SIZE: usize = 32;

#[inline(never)]
pub(super) fn verify_settlement_oracle_attestations(
    program_id: &Pubkey,
    settlement: &CompressedSettlementLeaf,
    registry_info: &AccountInfo,
    signer_set_info: &AccountInfo,
    instructions_sysvar_info: &AccountInfo,
) -> Result<u16, ProgramError> {
    if *instructions_sysvar_info.key != instructions::id() {
        return Err(VaultError::InvalidInstructionsSysvar.into());
    }

    let registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    let signer_set =
        load_canonical_settlement_signer_set(program_id, registry_info.key, signer_set_info)?;
    if registry.current_set != *signer_set_info.key
        || registry.current_version != signer_set.version
        || settlement.signer_set_version != signer_set.version
        || settlement.signer_set_hash != signer_set.set_hash
    {
        return Err(VaultError::SettlementSignerSetVersionMismatch.into());
    }

    let current_index = instructions::load_current_index_checked(instructions_sysvar_info)
        .map_err(|_| VaultError::InvalidInstructionsSysvar)?;
    let threshold = usize::from(signer_set.threshold);
    if usize::from(current_index) < threshold {
        return Err(VaultError::MissingSettlementOracleSignatures.into());
    }

    let expected_message = settlement_attestation_digest(
        program_id,
        registry_info.key,
        signer_set_info.key,
        &signer_set,
        settlement,
    )?;
    let first_signature_index = usize::from(current_index) - threshold;
    let mut signer_bitmap = 0u16;

    for signature_index in first_signature_index..usize::from(current_index) {
        let instruction =
            instructions::load_instruction_at_checked(signature_index, instructions_sysvar_info)
                .map_err(|_| VaultError::InvalidInstructionsSysvar)?;
        let signer_index = parse_verified_settlement_oracle_instruction(
            &instruction,
            &expected_message,
            &signer_set,
        )?;
        let signer_bit = 1u16
            .checked_shl(
                u32::try_from(signer_index)
                    .map_err(|_| VaultError::InvalidSettlementOracleSigner)?,
            )
            .ok_or(VaultError::InvalidSettlementOracleSigner)?;
        if signer_bitmap & signer_bit != 0 {
            return Err(VaultError::DuplicateSettlementOracleSigner.into());
        }
        signer_bitmap |= signer_bit;
    }

    if signer_bitmap.count_ones() == u32::from(signer_set.threshold) {
        Ok(signer_bitmap)
    } else {
        Err(VaultError::MissingSettlementOracleSignatures.into())
    }
}

pub(super) fn parse_verified_settlement_oracle_instruction(
    instruction: &Instruction,
    expected_message: &[u8],
    signer_set: &SettlementSignerSet,
) -> Result<usize, ProgramError> {
    if instruction.program_id != ed25519_program::id() || !instruction.accounts.is_empty() {
        return Err(VaultError::InvalidSettlementOracleSignature.into());
    }

    let data = instruction.data.as_slice();
    if data.len() < ED25519_DATA_START || data[0] != 1 {
        return Err(VaultError::InvalidSettlementOracleSignature.into());
    }

    let signature_offset = usize::from(read_ed25519_offset(data, 2)?);
    let signature_instruction_index = read_ed25519_offset(data, 4)?;
    let public_key_offset = usize::from(read_ed25519_offset(data, 6)?);
    let public_key_instruction_index = read_ed25519_offset(data, 8)?;
    let message_data_offset = usize::from(read_ed25519_offset(data, 10)?);
    let message_data_size = usize::from(read_ed25519_offset(data, 12)?);
    let message_instruction_index = read_ed25519_offset(data, 14)?;

    if signature_instruction_index != u16::MAX
        || public_key_instruction_index != u16::MAX
        || message_instruction_index != u16::MAX
    {
        return Err(VaultError::InvalidSettlementOracleSignature.into());
    }

    let _signature = data
        .get(signature_offset..signature_offset + ED25519_SIGNATURE_SERIALIZED_SIZE)
        .ok_or(VaultError::InvalidSettlementOracleSignature)?;
    let public_key_slice = data
        .get(public_key_offset..public_key_offset + ED25519_PUBKEY_SERIALIZED_SIZE)
        .ok_or(VaultError::InvalidSettlementOracleSignature)?;
    let message_slice = data
        .get(message_data_offset..message_data_offset + message_data_size)
        .ok_or(VaultError::InvalidSettlementOracleSignature)?;

    if message_slice != expected_message {
        return Err(VaultError::SettlementOraclePayloadMismatch.into());
    }

    resolve_settlement_oracle_signer_index(signer_set, public_key_slice)
        .ok_or(VaultError::InvalidSettlementOracleSigner.into())
}

pub(super) fn read_ed25519_offset(data: &[u8], offset: usize) -> Result<u16, ProgramError> {
    let slice = data
        .get(offset..offset + 2)
        .ok_or(VaultError::InvalidSettlementOracleSignature)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

pub(super) fn resolve_settlement_oracle_signer_index(
    signer_set: &SettlementSignerSet,
    public_key: &[u8],
) -> Option<usize> {
    signer_set
        .signers
        .iter()
        .take(usize::from(signer_set.signer_count))
        .position(|candidate| candidate.as_ref() == public_key)
}

pub(super) fn settlement_attestation_digest(
    program_id: &Pubkey,
    registry: &Pubkey,
    signer_set_key: &Pubkey,
    signer_set: &SettlementSignerSet,
    settlement: &CompressedSettlementLeaf,
) -> Result<Vec<u8>, ProgramError> {
    const DOMAIN: &[u8] = b"ameba_settlement_attestation_v2";
    let payload = settlement
        .signed_payload_bytes()
        .map_err(|_| VaultError::InvalidSettlementRecord)?;
    let payload_hash = hashv(&[payload.as_slice()]).to_bytes();
    Ok(hashv(&[
        DOMAIN,
        program_id.as_ref(),
        registry.as_ref(),
        signer_set_key.as_ref(),
        &signer_set.version.to_le_bytes(),
        &signer_set.set_hash,
        &payload_hash,
    ])
    .to_bytes()
    .to_vec())
}

pub(super) fn load_valid_settlement_record_v2(
    program_id: &Pubkey,
    market_key: &Pubkey,
    market: &Market,
    settlement_record_info: &AccountInfo,
) -> Result<SettlementRecordV2, ProgramError> {
    ensure_settlement_finalization_ready(market)?;
    let (oracle_month, _) =
        derive_oracle_month_pda(program_id, market_key, market.instrument.expiry_ts);
    let (expected, bump) = derive_settlement_v2_pda(program_id, market_key, &oracle_month);
    if *settlement_record_info.key != expected {
        return Err(VaultError::InvalidSettlementAccount.into());
    }
    let settlement_record: SettlementRecordV2 = load_exact_zero_padded_state(
        settlement_record_info,
        program_id,
        SettlementRecordV2::LEN,
        VaultError::InvalidSettlementAccount,
    )?;
    ensure_settlement_timestamp_matches_expiry(
        market.instrument.expiry_ts,
        settlement_record.settlement_ts,
    )
    .map_err(|_| VaultError::InvalidSettlementAccount)?;
    if !settlement_record.is_initialized
        || settlement_record.bump != bump
        || settlement_record.account_discriminator != SettlementRecordV2::ACCOUNT_DISCRIMINATOR
        || settlement_record.account_version != SettlementRecordV2::ACCOUNT_VERSION
        || settlement_record.signer_set_version == 0
        || settlement_record.market != *market_key
        || settlement_record.oracle_month != oracle_month
        || crate::bytes32_is_zero(&settlement_record.signed_leaf_commitment)
        || crate::pubkey_is_default(&settlement_record.submitted_by)
    {
        return Err(VaultError::InvalidSettlementAccount.into());
    }
    Ok(settlement_record)
}

pub(super) fn scaled_contract_payout(
    spread_width_atomic: u64,
    contract_size_atomic: u64,
) -> Result<u64, ProgramError> {
    let payout = (spread_width_atomic as u128)
        .checked_mul(contract_size_atomic as u128)
        .ok_or(VaultError::ArithmeticOverflow)?
        .checked_div(MarketMintAccounting::CANONICAL_ATOMIC_SCALE as u128)
        .ok_or(VaultError::ArithmeticOverflow)?;
    u64::try_from(payout).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(super) fn validate_instrument_definition(
    instrument: &crate::state::InstrumentDefinition,
) -> Result<(), ProgramError> {
    if instrument.expiry_ts == 0
        || instrument.contract_size != MarketMintAccounting::CANONICAL_ATOMIC_SCALE
        || instrument.max_payout_per_contract == 0
    {
        return Err(VaultError::InvalidMarketConfig.into());
    }

    let spread_width = match instrument.kind {
        crate::state::OptionKind::CallSpread => {
            if instrument.cap_price <= instrument.strike_price {
                return Err(VaultError::InvalidMarketConfig.into());
            }
            instrument
                .cap_price
                .checked_sub(instrument.strike_price)
                .ok_or(VaultError::ArithmeticOverflow)?
        }
        crate::state::OptionKind::PutSpread => {
            if instrument.cap_price >= instrument.strike_price {
                return Err(VaultError::InvalidMarketConfig.into());
            }
            instrument
                .strike_price
                .checked_sub(instrument.cap_price)
                .ok_or(VaultError::ArithmeticOverflow)?
        }
    };
    let expected_max_payout = scaled_contract_payout(spread_width, instrument.contract_size)?;

    if expected_max_payout == 0 || expected_max_payout != instrument.max_payout_per_contract {
        return Err(VaultError::InvalidMarketConfig.into());
    }

    Ok(())
}

pub(super) fn validate_market_parameters(
    params: &crate::state::MarketParameters,
    max_payout_per_contract: u64,
) -> Result<(), ProgramError> {
    let tick = crate::constants::AMOEBA_DLMM_TICK_SIZE_QUOTE_ATOMIC;
    let maximum_bin_id = max_payout_per_contract
        .checked_div(tick)
        .ok_or(VaultError::InvalidMarketConfig)?;
    if params.tick_size != tick
        || max_payout_per_contract % tick != 0
        || maximum_bin_id == 0
        || maximum_bin_id > crate::constants::MAX_AMOEBA_DLMM_BIN_COUNT as u64
        || params.lot_size != 1
        || params.min_order_qty != 1
        || params.maker_fee_bps > 10_000
        || params.taker_fee_bps != 20
        || params.cancel_fee_bps > 10_000
        || params.max_fills_per_instruction != crate::constants::MAX_AMOEBA_DLMM_BINS_PER_SWAP
    {
        return Err(VaultError::InvalidMarketConfig.into());
    }
    Ok(())
}

pub(super) fn current_unix_timestamp() -> Result<u64, ProgramError> {
    let timestamp = Clock::get()?.unix_timestamp;
    if timestamp < 0 {
        return Err(VaultError::InvalidSettlementRecord.into());
    }
    u64::try_from(timestamp).map_err(|_| VaultError::InvalidSettlementRecord.into())
}

pub(super) fn current_slot_and_unix_timestamp() -> Result<(u64, u64), ProgramError> {
    let clock = Clock::get()?;
    if clock.unix_timestamp < 0 {
        return Err(VaultError::InvalidSettlementRecord.into());
    }
    Ok((
        clock.slot,
        u64::try_from(clock.unix_timestamp)
            .map_err(|_| ProgramError::from(VaultError::InvalidSettlementRecord))?,
    ))
}

pub(super) fn settlement_finalization_timestamp(expiry_ts: u64) -> Result<u64, ProgramError> {
    expiry_ts
        .checked_add(ORACLE_SETTLEMENT_GRACE_SECONDS)
        .ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

pub(super) fn ensure_settlement_timestamp_matches_expiry(
    expiry_ts: u64,
    supplied_settlement_ts: u64,
) -> ProgramResult {
    if supplied_settlement_ts != expiry_ts {
        return Err(VaultError::InvalidSettlementRecord.into());
    }
    Ok(())
}

pub(super) fn ensure_settlement_finalization_ready_at(
    expiry_ts: u64,
    current_timestamp: u64,
) -> ProgramResult {
    if current_timestamp < settlement_finalization_timestamp(expiry_ts)? {
        return Err(VaultError::SettlementGracePeriodActive.into());
    }
    Ok(())
}

pub(super) fn ensure_settlement_finalization_ready(market: &Market) -> ProgramResult {
    ensure_settlement_finalization_ready_at(market.instrument.expiry_ts, current_unix_timestamp()?)
}

pub(super) fn validate_light_token_account(account_info: &AccountInfo) -> ProgramResult {
    if account_info.owner != &light_token_program_id() {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(())
}

pub(super) fn validate_light_associated_token_account(
    owner: &Pubkey,
    mint: &Pubkey,
    account_info: &AccountInfo,
) -> ProgramResult {
    validate_light_associated_token_address(owner, mint, account_info)?;
    let _ = load_canonical_light_token_account(account_info, owner, mint)?;
    Ok(())
}

pub(super) fn validate_light_associated_token_address(
    owner: &Pubkey,
    mint: &Pubkey,
    account_info: &AccountInfo,
) -> ProgramResult {
    if *account_info.key != light_token_instruction::get_associated_token_address(owner, mint) {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_or_create_light_associated_token_account<'a>(
    payer_info: &AccountInfo<'a>,
    owner_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    account_info: &AccountInfo<'a>,
    light_token_program_info: &AccountInfo<'a>,
    compressible_config_info: &AccountInfo<'a>,
    rent_sponsor_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<TokenAccount, ProgramError> {
    validate_light_associated_token_address(owner_info.key, mint_info.key, account_info)?;
    if account_info.owner == &light_token_program_id() {
        return load_canonical_light_token_account(account_info, owner_info.key, mint_info.key);
    }
    if account_info.owner != &system_program::id()
        || account_info.executable
        || account_info.data_len() != 0
        || !account_info.is_writable
    {
        return Err(VaultError::InvalidLightTokenAccount.into());
    }
    if !payer_info.is_signer || !payer_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *light_token_program_info.key != light_token_program_id() {
        return Err(VaultError::InvalidLightTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    if *compressible_config_info.key != light_token_instruction::compressible_config()
        || *rent_sponsor_info.key != light_token_instruction::rent_sponsor()
        || !rent_sponsor_info.is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let instruction = light_token_instruction::create_associated_token_account_idempotent(
        payer_info.key,
        owner_info.key,
        mint_info.key,
        compressible_config_info.key,
        rent_sponsor_info.key,
    );
    invoke(
        &instruction,
        &[
            owner_info.clone(),
            mint_info.clone(),
            payer_info.clone(),
            account_info.clone(),
            system_program_info.clone(),
            compressible_config_info.clone(),
            rent_sponsor_info.clone(),
            light_token_program_info.clone(),
        ],
    )?;
    load_canonical_light_token_account(account_info, owner_info.key, mint_info.key)
}

pub(super) fn ensure_market_not_expired(market: &Market) -> Result<(), ProgramError> {
    if current_unix_timestamp()? >= market.instrument.expiry_ts {
        return Err(VaultError::MarketExpired.into());
    }
    Ok(())
}
