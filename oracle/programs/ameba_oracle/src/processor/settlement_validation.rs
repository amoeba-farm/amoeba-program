use super::*;

pub(super) fn current_unix_timestamp() -> Result<u64, ProgramError> {
    let timestamp = crate::compact_error::clock()?.unix_timestamp;
    if timestamp < 0 {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    u64::try_from(timestamp).map_err(|_| OracleError::InvalidSettlementRecord.into())
}

pub(super) fn current_slot_and_unix_timestamp() -> Result<(u64, u64), ProgramError> {
    let clock = crate::compact_error::clock()?;
    if clock.unix_timestamp < 0 {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    Ok((
        clock.slot,
        u64::try_from(clock.unix_timestamp)
            .map_err(|_| ProgramError::from(OracleError::InvalidSettlementRecord))?,
    ))
}

pub(super) fn settlement_finalization_timestamp(expiry_ts: u64) -> Result<u64, ProgramError> {
    expiry_ts
        .checked_add(ORACLE_SETTLEMENT_GRACE_SECONDS)
        .ok_or_else(|| OracleError::ArithmeticOverflow.into())
}

pub(super) fn ensure_settlement_finalization_ready_at(
    expiry_ts: u64,
    current_timestamp: u64,
) -> ProgramResult {
    if current_timestamp < settlement_finalization_timestamp(expiry_ts)? {
        return Err(OracleError::SettlementGracePeriodActive.into());
    }
    Ok(())
}

/// The settlement value of a month: `round(base × (10_000 + Δ) / 10_000)`, half up, at the
/// period's six decimals. A zero base or negative factor is invalid; a positive base may round to zero.
pub(super) fn index_delta_settlement_value(
    base_oracle_atomic: u64,
    index_delta_bps: i64,
) -> Result<u64, ProgramError> {
    if base_oracle_atomic == 0 {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    let factor_bps = 10_000i128
        .checked_add(i128::from(index_delta_bps))
        .ok_or(OracleError::InvalidSettlementRecord)?;
    if factor_bps < 0 {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    let rounded = i128::from(base_oracle_atomic)
        .checked_mul(factor_bps)
        .and_then(|scaled| scaled.checked_add(5_000))
        .ok_or(OracleError::InvalidSettlementRecord)?
        / 10_000;
    match u64::try_from(rounded) {
        Ok(value) => Ok(value),
        _ => Err(OracleError::InvalidSettlementRecord.into()),
    }
}

pub(super) fn validate_text_field(
    value: &str,
    max_len: usize,
    allow_empty: bool,
    error: OracleError,
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

/// Binds a leaf to the period it settles: the period's product and its immutable expiry. The
/// month is already bound to the period by its PDA and anchor.
#[inline(never)]
pub(super) fn validate_settlement_period_binding(
    anchor: &PeriodAnchor,
    settlement: &SettlementLeaf,
) -> Result<(), ProgramError> {
    ensure_settlement_timestamp_matches_expiry(anchor.expiry_ts, settlement.settlement_ts)?;
    if settlement.underlying_id != anchor.underlying_id {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn validate_settlement_leaf(
    settlement: &SettlementLeaf,
    oracle_month: &OracleMonthState,
) -> Result<(), ProgramError> {
    validate_text_field(
        &settlement.item_id,
        32,
        false,
        OracleError::InvalidSettlementRecord,
    )?;
    if !settlement
        .item_id
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(OracleError::InvalidSettlementRecord.into());
    }

    validate_text_field(
        &settlement.expiry_id,
        24,
        false,
        OracleError::InvalidSettlementRecord,
    )?;
    if !settlement
        .expiry_id
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(OracleError::InvalidSettlementRecord.into());
    }

    validate_text_field(
        &settlement.source_uri,
        256,
        false,
        OracleError::InvalidSettlementRecord,
    )?;

    if settlement.schema_version != SettlementLeaf::CURRENT_SCHEMA_VERSION
        || crate::pubkey_is_default(&settlement.oracle_month)
        || crate::bytes32_is_zero(&settlement.recipe_hash)
        || settlement.settlement_ts == 0
        || settlement.signer_set_version == 0
        || crate::bytes32_is_zero(&settlement.signer_set_hash)
    {
        return Err(OracleError::InvalidSettlementRecord.into());
    }

    if settlement.computation != SettlementComputation::OracleIndexDelta
        || settlement.trailing_window_days != 0
        || !settlement.observations.is_empty()
        || settlement.base_oracle_atomic == 0
        || oracle_month.settlement_base_oracle_atomic == 0
        || settlement.base_oracle_atomic != oracle_month.settlement_base_oracle_atomic
        || settlement.index_delta_bps != oracle_month.index_delta_bps
    {
        return Err(OracleError::InvalidSettlementRecord.into());
    }

    let computed = settlement
        .computed_index_delta_price_atomic()
        .map_err(|_| OracleError::InvalidSettlementRecord)?;
    if settlement.settlement_price_atomic != computed {
        return Err(OracleError::SettlementRecordHashMismatch.into());
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
    settlement: &SettlementLeaf,
    registry_info: &AccountInfo,
    signer_set_info: &AccountInfo,
    instructions_sysvar_info: &AccountInfo,
) -> Result<u16, ProgramError> {
    if *instructions_sysvar_info.key != instructions::id() {
        return Err(OracleError::InvalidInstructionsSysvar.into());
    }

    let registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    let signer_set =
        load_canonical_settlement_signer_set(program_id, registry_info.key, signer_set_info)?;
    if registry.current_set != *signer_set_info.key
        || registry.current_version != signer_set.version
        || settlement.signer_set_version != signer_set.version
        || settlement.signer_set_hash != signer_set.set_hash
    {
        return Err(OracleError::SettlementSignerSetVersionMismatch.into());
    }

    let current_index = instructions::load_current_index_checked(instructions_sysvar_info)
        .map_err(|_| OracleError::InvalidInstructionsSysvar)?;
    let threshold = usize::from(signer_set.threshold);
    if usize::from(current_index) < threshold {
        return Err(OracleError::MissingSettlementOracleSignatures.into());
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
                .map_err(|_| OracleError::InvalidInstructionsSysvar)?;
        let signer_index = parse_verified_settlement_oracle_instruction(
            &instruction,
            &expected_message,
            &signer_set,
        )?;
        let signer_bit = 1u16
            .checked_shl(
                u32::try_from(signer_index)
                    .map_err(|_| OracleError::InvalidSettlementOracleSigner)?,
            )
            .ok_or(OracleError::InvalidSettlementOracleSigner)?;
        if signer_bitmap & signer_bit != 0 {
            return Err(OracleError::DuplicateSettlementOracleSigner.into());
        }
        signer_bitmap |= signer_bit;
    }

    if signer_bitmap.count_ones() == u32::from(signer_set.threshold) {
        Ok(signer_bitmap)
    } else {
        Err(OracleError::MissingSettlementOracleSignatures.into())
    }
}

pub(super) fn parse_verified_settlement_oracle_instruction(
    instruction: &Instruction,
    expected_message: &[u8],
    signer_set: &SettlementSignerSet,
) -> Result<usize, ProgramError> {
    if instruction.program_id != ed25519_program::id() || !instruction.accounts.is_empty() {
        return Err(OracleError::InvalidSettlementOracleSignature.into());
    }

    let data = instruction.data.as_slice();
    if data.len() < ED25519_DATA_START || data[0] != 1 {
        return Err(OracleError::InvalidSettlementOracleSignature.into());
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
        return Err(OracleError::InvalidSettlementOracleSignature.into());
    }

    let _signature = data
        .get(signature_offset..signature_offset + ED25519_SIGNATURE_SERIALIZED_SIZE)
        .ok_or(OracleError::InvalidSettlementOracleSignature)?;
    let public_key_slice = data
        .get(public_key_offset..public_key_offset + ED25519_PUBKEY_SERIALIZED_SIZE)
        .ok_or(OracleError::InvalidSettlementOracleSignature)?;
    let message_slice = data
        .get(message_data_offset..message_data_offset + message_data_size)
        .ok_or(OracleError::InvalidSettlementOracleSignature)?;

    if message_slice != expected_message {
        return Err(OracleError::SettlementOraclePayloadMismatch.into());
    }

    resolve_settlement_oracle_signer_index(signer_set, public_key_slice)
        .ok_or(OracleError::InvalidSettlementOracleSigner.into())
}

pub(super) fn read_ed25519_offset(data: &[u8], offset: usize) -> Result<u16, ProgramError> {
    let slice = data
        .get(offset..offset + 2)
        .ok_or(OracleError::InvalidSettlementOracleSignature)?;
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
    settlement: &SettlementLeaf,
) -> Result<Vec<u8>, ProgramError> {
    const DOMAIN: &[u8] = b"ameba_settlement_attestation_v2";
    let payload = settlement
        .signed_payload_bytes()
        .map_err(|_| OracleError::InvalidSettlementRecord)?;
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
    period_key: &Pubkey,
    anchor: &PeriodAnchor,
    settlement_record_info: &AccountInfo,
) -> Result<SettlementRecordV2, ProgramError> {
    ensure_settlement_finalization_ready(anchor)?;
    let (oracle_month, _) = derive_oracle_month_pda(program_id, period_key, anchor.expiry_ts);
    let (expected, bump) = derive_settlement_v2_pda(program_id, period_key, &oracle_month);
    if *settlement_record_info.key != expected {
        return Err(OracleError::InvalidSettlementAccount.into());
    }
    let settlement_record: SettlementRecordV2 = load_exact_zero_padded_state(
        settlement_record_info,
        program_id,
        SettlementRecordV2::LEN,
        OracleError::InvalidSettlementAccount,
    )?;
    ensure_settlement_timestamp_matches_expiry(anchor.expiry_ts, settlement_record.settlement_ts)
        .map_err(|_| OracleError::InvalidSettlementAccount)?;
    if !settlement_record.is_initialized
        || settlement_record.bump != bump
        || settlement_record.account_discriminator != SettlementRecordV2::ACCOUNT_DISCRIMINATOR
        || settlement_record.account_version != SettlementRecordV2::ACCOUNT_VERSION
        || settlement_record.signer_set_version == 0
        || settlement_record.period != *period_key
        || settlement_record.oracle_month != oracle_month
        || crate::bytes32_is_zero(&settlement_record.signed_leaf_commitment)
        || crate::pubkey_is_default(&settlement_record.submitted_by)
    {
        return Err(OracleError::InvalidSettlementAccount.into());
    }
    Ok(settlement_record)
}

pub(super) fn ensure_settlement_timestamp_matches_expiry(
    expiry_ts: u64,
    supplied_settlement_ts: u64,
) -> ProgramResult {
    if supplied_settlement_ts != expiry_ts {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    Ok(())
}

pub(super) fn ensure_settlement_finalization_ready(anchor: &PeriodAnchor) -> ProgramResult {
    ensure_settlement_finalization_ready_at(anchor.expiry_ts, current_unix_timestamp()?)
}
