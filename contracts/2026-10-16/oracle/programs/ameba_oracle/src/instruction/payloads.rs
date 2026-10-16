//! Exact-length payload decoders used by the dispatcher. Every decoder rejects truncated and
//! trailing bytes with the generic invalid-instruction error before any account is touched.

use super::*;
use crate::error::OracleError;
use crate::{ProgramError, ProgramResult};

pub(crate) struct UpdateConfigArgs {
    pub(crate) new_admin: Option<Pubkey>,
    pub(crate) new_oracle_authority: Option<Pubkey>,
    pub(crate) new_usdc_mint: Option<Pubkey>,
    pub(crate) new_bond_vault: Option<Pubkey>,
    pub(crate) paused: Option<bool>,
}

#[inline(always)]
pub(crate) fn decode_instruction_payload<T: BorshDeserialize>(
    payload: &[u8],
) -> Result<T, ProgramError> {
    let mut remaining = payload;
    let value = T::deserialize(&mut remaining)
        .map_err(|_| ProgramError::from(OracleError::InvalidInstructionData))?;
    if remaining.is_empty() {
        Ok(value)
    } else {
        Err(OracleError::InvalidInstructionData.into())
    }
}

#[inline(never)]
pub(crate) fn decode_u64_payload(payload: &[u8]) -> Result<u64, ProgramError> {
    let bytes: [u8; 8] = payload
        .try_into()
        .map_err(|_| ProgramError::from(OracleError::InvalidInstructionData))?;
    Ok(u64::from_le_bytes(bytes))
}

#[inline(never)]
pub(crate) fn decode_u8_payload(payload: &[u8]) -> Result<u8, ProgramError> {
    match payload {
        [value] => Ok(*value),
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(crate) fn decode_u8_u64_payload(payload: &[u8]) -> Result<(u8, u64), ProgramError> {
    let bytes: [u8; 9] = payload
        .try_into()
        .map_err(|_| ProgramError::from(OracleError::InvalidInstructionData))?;
    Ok((
        bytes[0],
        u64::from_le_bytes([
            bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        ]),
    ))
}

pub(crate) fn decode_u16_pair_payload(payload: &[u8]) -> Result<(u16, u16), ProgramError> {
    let bytes: [u8; 4] = payload
        .try_into()
        .map_err(|_| ProgramError::from(OracleError::InvalidInstructionData))?;
    Ok((
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[2], bytes[3]]),
    ))
}

pub(crate) fn decode_bytes32_payload(payload: &[u8]) -> Result<[u8; 32], ProgramError> {
    payload
        .try_into()
        .map_err(|_| OracleError::InvalidInstructionData.into())
}

#[inline(never)]
pub(crate) fn decode_optional_bytes32_payload(
    payload: &[u8],
) -> Result<Option<[u8; 32]>, ProgramError> {
    match payload {
        [0] => Ok(None),
        [1, bytes @ ..] => decode_bytes32_payload(bytes).map(Some),
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
pub(crate) fn decode_optional_pubkey_field(
    payload: &mut &[u8],
) -> Result<Option<Pubkey>, ProgramError> {
    let Some((&tag, remaining)) = payload.split_first() else {
        return Err(OracleError::InvalidInstructionData.into());
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
                    .map_err(|_| OracleError::InvalidInstructionData)?,
            )))
        }
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(always)]
pub(crate) fn decode_optional_bool_field(
    payload: &mut &[u8],
) -> Result<Option<bool>, ProgramError> {
    let Some((&tag, remaining)) = payload.split_first() else {
        return Err(OracleError::InvalidInstructionData.into());
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
            _ => Err(OracleError::InvalidInstructionData.into()),
        },
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(always)]
pub(crate) fn decode_update_config_payload(
    payload: &[u8],
) -> Result<UpdateConfigArgs, ProgramError> {
    let mut remaining = payload;
    let args = UpdateConfigArgs {
        new_admin: decode_optional_pubkey_field(&mut remaining)?,
        new_oracle_authority: decode_optional_pubkey_field(&mut remaining)?,
        new_usdc_mint: decode_optional_pubkey_field(&mut remaining)?,
        new_bond_vault: decode_optional_pubkey_field(&mut remaining)?,
        paused: decode_optional_bool_field(&mut remaining)?,
    };
    if remaining.is_empty() {
        Ok(args)
    } else {
        Err(OracleError::InvalidInstructionData.into())
    }
}

pub(crate) fn expect_empty_payload(payload: &[u8]) -> ProgramResult {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(OracleError::InvalidInstructionData.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_config_payload_matches_the_borsh_model_and_rejects_noncanonical_bytes() {
        let instruction = OracleInstruction::UpdateConfig {
            new_admin: Some(Pubkey::new_unique()),
            new_oracle_authority: None,
            new_usdc_mint: None,
            new_bond_vault: Some(Pubkey::new_unique()),
            paused: Some(false),
        };
        let encoded = instruction.try_to_vec().unwrap();
        let args = decode_update_config_payload(&encoded[1..]).unwrap();
        let OracleInstruction::UpdateConfig {
            new_admin,
            new_oracle_authority,
            new_usdc_mint,
            new_bond_vault,
            paused,
        } = instruction
        else {
            unreachable!()
        };
        assert_eq!(args.new_admin, new_admin);
        assert_eq!(args.new_oracle_authority, new_oracle_authority);
        assert_eq!(args.new_usdc_mint, new_usdc_mint);
        assert_eq!(args.new_bond_vault, new_bond_vault);
        assert_eq!(args.paused, paused);

        let invalid = Err(ProgramError::from(OracleError::InvalidInstructionData));
        for end in 0..encoded.len() - 1 {
            assert_eq!(
                decode_update_config_payload(&encoded[1..1 + end]).map(|_| ()),
                invalid
            );
        }
        let mut trailing = encoded[1..].to_vec();
        trailing.push(0);
        assert_eq!(decode_update_config_payload(&trailing).map(|_| ()), invalid);
        let mut bad_bool = encoded[1..].to_vec();
        *bad_bool.last_mut().unwrap() = 2;
        assert_eq!(decode_update_config_payload(&bad_bool).map(|_| ()), invalid);
        assert_eq!(
            decode_update_config_payload(&[2, 0, 0, 0, 0]).map(|_| ()),
            invalid
        );
    }

    #[test]
    fn fixed_payload_decoders_are_exact_length() {
        let invalid = Err(ProgramError::from(OracleError::InvalidInstructionData));
        assert_eq!(decode_u64_payload(&7u64.to_le_bytes()), Ok(7));
        assert_eq!(decode_u64_payload(&[0; 7]).map(|_| ()), invalid);
        assert_eq!(decode_u64_payload(&[0; 9]).map(|_| ()), invalid);
        assert_eq!(decode_bytes32_payload(&[4; 32]), Ok([4; 32]));
        assert_eq!(decode_bytes32_payload(&[4; 31]).map(|_| ()), invalid);
        assert_eq!(decode_optional_bytes32_payload(&[0]), Ok(None));
        assert_eq!(
            decode_optional_bytes32_payload(&[&[1u8][..], &[5; 32]].concat()),
            Ok(Some([5; 32]))
        );
        for bad in [&[][..], &[0, 0][..], &[1][..], &[2][..]] {
            assert_eq!(decode_optional_bytes32_payload(bad).map(|_| ()), invalid);
        }
        assert_eq!(expect_empty_payload(&[]), Ok(()));
        assert_eq!(expect_empty_payload(&[0]), invalid);
    }
}
