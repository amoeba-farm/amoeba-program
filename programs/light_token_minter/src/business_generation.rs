//! Business-message domain. Controller-owned governance envelopes stay unchanged.
use crate::error::VaultError;
use crate::ProgramError;

pub const BUSINESS_GENERATION: u8 = 3;
pub const BUSINESS_MESSAGE_SUFFIX: &[u8; 4] = b"AMG3";

pub fn requires_generation(tag: u8) -> bool {
    // The byte the mainnet-v3 artifact does not dispatch (220) is not a business message there.
    !(cfg!(feature = "mainnet-v3")
        && crate::governance_manifest::is_mainnet_v3_retired_instruction_tag(tag))
        && matches!(tag, 12 | 30 | 32 | 59 | 75 | 80 | 81 | 84 | 86 | 87 | 89 | 96 | 113 | 116
        | 121 | 122 | 124 | 125 | 126 | 156..=188 | 191..=205 | 207..=215 | 220..=255)
}

pub fn strip_current_message(data: &[u8]) -> Result<&[u8], ProgramError> {
    let tag = *data.first().ok_or(VaultError::InvalidInstructionData)?;
    if !requires_generation(tag) {
        return Ok(data);
    }
    let end = data
        .len()
        .checked_sub(BUSINESS_MESSAGE_SUFFIX.len())
        .ok_or(VaultError::InvalidInstructionData)?;
    if end == 0 || &data[end..] != BUSINESS_MESSAGE_SUFFIX {
        return Err(VaultError::InvalidInstructionData.into());
    }
    Ok(&data[..end])
}
