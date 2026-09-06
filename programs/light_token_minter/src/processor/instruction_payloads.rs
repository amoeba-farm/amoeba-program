use super::*;

#[derive(BorshDeserialize)]
pub(super) struct UpdateConfigArgs {
    pub(super) new_admin: Option<Pubkey>,
    pub(super) new_oracle_authority: Option<Pubkey>,
    pub(super) new_usdc_mint: Option<Pubkey>,
    pub(super) new_vault_token_account: Option<Pubkey>,
    pub(super) paused: Option<bool>,
}

#[derive(BorshDeserialize)]
pub(super) struct UpsertMarketPageArgs {
    pub(super) params: UpsertMarketPageParams,
    pub(super) proof: Box<light_sdk::proof::borsh_compat::ValidityProof>,
    pub(super) new_page_output: Option<Box<CompressionOutput>>,
    pub(super) existing_page: Option<Box<CompressedMarketPageWitness>>,
}

#[derive(BorshDeserialize)]
pub(super) struct UpsertSettlementArgs {
    pub(super) params: Box<UpsertSettlementParams>,
    pub(super) proof: Box<light_sdk::proof::borsh_compat::ValidityProof>,
    pub(super) new_settlement_output: Option<Box<CompressionOutput>>,
    pub(super) existing_settlement: Option<Box<CompressedSettlementWitness>>,
}

pub(super) fn deserialize_optional_compression_output(
    data: &mut &[u8],
) -> std::io::Result<Option<Box<CompressionOutput>>> {
    match deserialize_slice_bytes::<1>(data)?[0] {
        0 => Ok(None),
        1 => Ok(Some(Box::new(deserialize_compression_output(data)?))),
        _ => Err(std::io::ErrorKind::InvalidData.into()),
    }
}

pub(super) fn deserialize_upsert_market_page_args(
    mut data: &[u8],
) -> std::io::Result<UpsertMarketPageArgs> {
    let page = CompressedMarketPageLeaf::deserialize_slice(&mut data)?;
    let proof = Box::new(deserialize_validity_proof(&mut data)?);
    let new_page_output = deserialize_optional_compression_output(&mut data)?;
    let existing_page = match deserialize_slice_bytes::<1>(&mut data)?[0] {
        0 => None,
        1 => Some(Box::new(CompressedMarketPageWitness {
            meta: deserialize_compressed_account_meta(&mut data)?,
            expected_commitment: deserialize_slice_bytes(&mut data)?,
            page: CompressedMarketPageLeaf::deserialize_slice(&mut data)?,
        })),
        _ => return Err(std::io::ErrorKind::InvalidData.into()),
    };
    if !data.is_empty() {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(UpsertMarketPageArgs {
        params: UpsertMarketPageParams { page },
        proof,
        new_page_output,
        existing_page,
    })
}

pub(super) fn deserialize_upsert_settlement_args(
    mut data: &[u8],
) -> std::io::Result<UpsertSettlementArgs> {
    let settlement = CompressedSettlementLeaf::deserialize_slice(&mut data)?;
    let proof = Box::new(deserialize_validity_proof(&mut data)?);
    let new_settlement_output = deserialize_optional_compression_output(&mut data)?;
    let existing_settlement = match deserialize_slice_bytes::<1>(&mut data)?[0] {
        0 => None,
        1 => Some(Box::new(CompressedSettlementWitness {
            meta: deserialize_compressed_account_meta(&mut data)?,
            expected_commitment: deserialize_slice_bytes(&mut data)?,
            settlement: CompressedSettlementLeaf::deserialize_slice(&mut data)?,
        })),
        _ => return Err(std::io::ErrorKind::InvalidData.into()),
    };
    if !data.is_empty() {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(UpsertSettlementArgs {
        params: Box::new(UpsertSettlementParams { settlement }),
        proof,
        new_settlement_output,
        existing_settlement,
    })
}

#[inline(always)]
pub(super) fn decode_instruction_payload<T: BorshDeserialize>(
    payload: &[u8],
) -> Result<T, ProgramError> {
    let mut remaining = payload;
    let value = T::deserialize(&mut remaining)
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    if remaining.is_empty() {
        Ok(value)
    } else {
        Err(VaultError::InvalidInstructionData.into())
    }
}

#[inline(never)]
pub(super) fn decode_u64_payload(payload: &[u8]) -> Result<u64, ProgramError> {
    let bytes: [u8; 8] = payload
        .try_into()
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    Ok(u64::from_le_bytes(bytes))
}

#[inline(never)]
pub(super) fn decode_u8_payload(payload: &[u8]) -> Result<u8, ProgramError> {
    match payload {
        [value] => Ok(*value),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(always)]
pub(super) fn decode_u8_u64_payload(payload: &[u8]) -> Result<(u8, u64), ProgramError> {
    let bytes: [u8; 9] = payload
        .try_into()
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    Ok((
        bytes[0],
        u64::from_le_bytes([
            bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        ]),
    ))
}

#[inline(never)]
pub(super) fn decode_u16_pair_payload(payload: &[u8]) -> Result<(u16, u16), ProgramError> {
    let bytes: [u8; 4] = payload
        .try_into()
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    Ok((
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[2], bytes[3]]),
    ))
}

#[inline(always)]
pub(super) fn decode_u64_pair_payload(payload: &[u8]) -> Result<(u64, u64), ProgramError> {
    let bytes: [u8; 16] = payload
        .try_into()
        .map_err(|_| ProgramError::from(VaultError::InvalidInstructionData))?;
    Ok((
        u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]),
        u64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]),
    ))
}

#[inline(always)]
pub(super) fn decode_bool_payload(payload: &[u8]) -> Result<bool, ProgramError> {
    match payload {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(super) fn decode_bytes32_payload(payload: &[u8]) -> Result<[u8; 32], ProgramError> {
    payload
        .try_into()
        .map_err(|_| VaultError::InvalidInstructionData.into())
}

#[inline(never)]
pub(super) fn decode_optional_bytes32_payload(
    payload: &[u8],
) -> Result<Option<[u8; 32]>, ProgramError> {
    match payload {
        [0] => Ok(None),
        [1, bytes @ ..] => decode_bytes32_payload(bytes).map(Some),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(super) fn decode_optional_pubkey_field(
    payload: &mut &[u8],
) -> Result<Option<Pubkey>, ProgramError> {
    let Some((&tag, remaining)) = payload.split_first() else {
        return Err(VaultError::InvalidInstructionData.into());
    };
    *payload = remaining;
    match tag {
        0 => Ok(None),
        1 if payload.len() >= 32 => {
            let (bytes, remaining) = payload.split_at(32);
            *payload = remaining;
            Ok(Some(Pubkey::new_from_array(
                bytes
                    .try_into()
                    .map_err(|_| VaultError::InvalidInstructionData)?,
            )))
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(always)]
pub(super) fn decode_optional_bool_field(
    payload: &mut &[u8],
) -> Result<Option<bool>, ProgramError> {
    let Some((&tag, remaining)) = payload.split_first() else {
        return Err(VaultError::InvalidInstructionData.into());
    };
    *payload = remaining;
    match tag {
        0 => Ok(None),
        1 => match payload.split_first() {
            Some((&0, remaining)) => {
                *payload = remaining;
                Ok(Some(false))
            }
            Some((&1, remaining)) => {
                *payload = remaining;
                Ok(Some(true))
            }
            _ => Err(VaultError::InvalidInstructionData.into()),
        },
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(always)]
pub(super) fn decode_update_config_payload(
    payload: &[u8],
) -> Result<UpdateConfigArgs, ProgramError> {
    let mut remaining = payload;
    let args = UpdateConfigArgs {
        new_admin: decode_optional_pubkey_field(&mut remaining)?,
        new_oracle_authority: decode_optional_pubkey_field(&mut remaining)?,
        new_usdc_mint: decode_optional_pubkey_field(&mut remaining)?,
        new_vault_token_account: decode_optional_pubkey_field(&mut remaining)?,
        paused: decode_optional_bool_field(&mut remaining)?,
    };
    if remaining.is_empty() {
        Ok(args)
    } else {
        Err(VaultError::InvalidInstructionData.into())
    }
}

pub(super) type EmergencyDisputePayload = (u8, [u8; 32], Option<[u8; 32]>);

#[inline(always)]
pub(super) fn decode_emergency_dispute_payload(
    payload: &[u8],
) -> Result<EmergencyDisputePayload, ProgramError> {
    if payload.len() < 34 {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let kind = payload[0];
    let target_id = decode_bytes32_payload(&payload[1..33])?;
    let expected_case_hash = decode_optional_bytes32_payload(&payload[33..])?;
    Ok((kind, target_id, expected_case_hash))
}

pub(super) fn expect_empty_payload(payload: &[u8]) -> ProgramResult {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(VaultError::InvalidInstructionData.into())
    }
}
