//! Public access to the native Light-state hydration wire codec. Consumers use
//! the exact strict parser and serializer executed by instruction 219.
pub use crate::processor::ameba_dlmm_light::lifecycle::{
    decode_decompress_params, AmoebaDlmmStateKind, DecodedAmoebaDlmmState,
    DecompressIdempotentParams, PackedCompressedAccountData,
};

use borsh::BorshSerialize;

/// Business bytes including tag 219. The caller appends the existing governed
/// generation/epoch envelope. Invalid state bodies cannot be encoded as valid
/// hydration instructions through a separate consumer codec.
pub fn instruction_data(
    params: &DecompressIdempotentParams,
) -> Result<Vec<u8>, crate::ProgramError> {
    let payload = params
        .try_to_vec()
        .map_err(|_| crate::ProgramError::InvalidInstructionData)?;
    let decoded = decode_decompress_params(&payload)?;
    if decoded.try_to_vec().ok().as_deref() != Some(payload.as_slice()) {
        return Err(crate::ProgramError::InvalidInstructionData);
    }
    let mut data =
        vec![crate::ameba_dlmm_instruction::AmoebaDlmmInstructionTag::DecompressLightState as u8];
    data.extend(payload);
    Ok(data)
}
