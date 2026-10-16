//! Exact read-only parsing for Upgradeable Loader v3 account bytes.
//!
//! Release 1 never accepts a caller-described loader state.  These helpers
//! bind controller checks to the canonical loader owner, fixed metadata
//! offsets, exact Program/ProgramData linkage, and authority bytes actually
//! stored in the supplied accounts.  Loader CPI construction lives elsewhere.

use solana_program::{account_info::AccountInfo, hash::hashv, pubkey::Pubkey};

use crate::{GovernanceError, GovernanceResult};

pub const LOADER_STATE_TAG_UNINITIALIZED: u32 = 0;
pub const LOADER_STATE_TAG_BUFFER: u32 = 1;
pub const LOADER_STATE_TAG_PROGRAM: u32 = 2;
pub const LOADER_STATE_TAG_PROGRAMDATA: u32 = 3;
pub const LOADER_PROGRAM_ACCOUNT_LEN: usize = 36;
pub const LOADER_BUFFER_METADATA_LEN: usize = 37;
pub const LOADER_PROGRAMDATA_METADATA_LEN: usize = 45;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpgradeableProgramHeaderV1 {
    pub programdata_address: Pubkey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpgradeableBufferHeaderV1 {
    pub authority: Option<Pubkey>,
    pub payload_offset: usize,
    pub payload_length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpgradeableProgramDataHeaderV1 {
    pub deployed_slot: u64,
    pub upgrade_authority: Option<Pubkey>,
    pub payload_offset: usize,
    pub capacity: usize,
}

pub fn parse_upgradeable_program(data: &[u8]) -> GovernanceResult<UpgradeableProgramHeaderV1> {
    if data.len() != LOADER_PROGRAM_ACCOUNT_LEN || read_u32(data, 0)? != LOADER_STATE_TAG_PROGRAM {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(UpgradeableProgramHeaderV1 {
        programdata_address: read_pubkey(data, 4)?,
    })
}

pub fn parse_upgradeable_buffer(data: &[u8]) -> GovernanceResult<UpgradeableBufferHeaderV1> {
    if data.len() < LOADER_BUFFER_METADATA_LEN || read_u32(data, 0)? != LOADER_STATE_TAG_BUFFER {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(UpgradeableBufferHeaderV1 {
        authority: read_fixed_optional_pubkey(data, 4)?,
        payload_offset: LOADER_BUFFER_METADATA_LEN,
        payload_length: data.len() - LOADER_BUFFER_METADATA_LEN,
    })
}

pub fn parse_upgradeable_programdata(
    data: &[u8],
) -> GovernanceResult<UpgradeableProgramDataHeaderV1> {
    if data.len() < LOADER_PROGRAMDATA_METADATA_LEN
        || read_u32(data, 0)? != LOADER_STATE_TAG_PROGRAMDATA
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(UpgradeableProgramDataHeaderV1 {
        deployed_slot: read_u64(data, 4)?,
        upgrade_authority: read_fixed_optional_pubkey(data, 12)?,
        payload_offset: LOADER_PROGRAMDATA_METADATA_LEN,
        capacity: data.len() - LOADER_PROGRAMDATA_METADATA_LEN,
    })
}

pub fn validate_program_programdata_linkage(
    target_program: &AccountInfo<'_>,
    target_programdata: &AccountInfo<'_>,
    upgradeable_loader: &Pubkey,
) -> GovernanceResult<UpgradeableProgramDataHeaderV1> {
    if target_program.owner != upgradeable_loader
        || target_programdata.owner != upgradeable_loader
        || !target_program.executable
        || target_programdata.executable
        || target_program.is_signer
        || target_programdata.is_signer
    {
        return Err(GovernanceError::IncorrectAccountOwner);
    }
    let program_data = target_program
        .try_borrow_data()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    let program = parse_upgradeable_program(&program_data)?;
    if program.programdata_address != *target_programdata.key {
        return Err(GovernanceError::CrossAccountMismatch);
    }
    let programdata_data = target_programdata
        .try_borrow_data()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    parse_upgradeable_programdata(&programdata_data)
}

pub fn validate_buffer_account(
    buffer: &AccountInfo<'_>,
    upgradeable_loader: &Pubkey,
) -> GovernanceResult<UpgradeableBufferHeaderV1> {
    if buffer.owner != upgradeable_loader || buffer.executable || buffer.is_signer {
        return Err(GovernanceError::IncorrectAccountOwner);
    }
    let data = buffer
        .try_borrow_data()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    parse_upgradeable_buffer(&data)
}

pub fn loader_account_data_hash(data: &[u8]) -> [u8; 32] {
    hashv(&[data]).to_bytes()
}

pub fn loader_payload_hash(data: &[u8], payload_offset: usize) -> GovernanceResult<[u8; 32]> {
    let payload = data
        .get(payload_offset..)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    Ok(hashv(&[payload]).to_bytes())
}

fn read_u32(data: &[u8], offset: usize) -> GovernanceResult<u32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> GovernanceResult<u64> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_pubkey(data: &[u8], offset: usize) -> GovernanceResult<Pubkey> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .ok_or(GovernanceError::InvalidRelease1Account)?
        .try_into()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    Ok(Pubkey::new_from_array(bytes))
}

fn read_fixed_optional_pubkey(
    data: &[u8],
    option_offset: usize,
) -> GovernanceResult<Option<Pubkey>> {
    let tag = *data
        .get(option_offset)
        .ok_or(GovernanceError::InvalidRelease1Account)?;
    let key = read_pubkey(data, option_offset + 1)?;
    // Loader v3 serializes `None` as only the one-byte option tag and does
    // not clear the remaining bytes in its fixed metadata region.  Those
    // bytes may therefore retain the previous authority after immutability;
    // they are ignored semantically but remain covered by the raw-account
    // commitment.  `Some(default)` is still rejected as malformed.
    match (tag, key == Pubkey::default()) {
        (0, _) => Ok(None),
        (1, false) => Ok(Some(key)),
        _ => Err(GovernanceError::InvalidRelease1Account),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn program_bytes(programdata: Pubkey) -> Vec<u8> {
        let mut data = vec![0; LOADER_PROGRAM_ACCOUNT_LEN];
        data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAM.to_le_bytes());
        data[4..36].copy_from_slice(programdata.as_ref());
        data
    }

    fn buffer_bytes(authority: Option<Pubkey>, payload: &[u8]) -> Vec<u8> {
        let mut data = vec![0; LOADER_BUFFER_METADATA_LEN + payload.len()];
        data[..4].copy_from_slice(&LOADER_STATE_TAG_BUFFER.to_le_bytes());
        if let Some(authority) = authority {
            data[4] = 1;
            data[5..37].copy_from_slice(authority.as_ref());
        }
        data[37..].copy_from_slice(payload);
        data
    }

    fn programdata_bytes(slot: u64, authority: Option<Pubkey>, payload: &[u8]) -> Vec<u8> {
        let mut data = vec![0; LOADER_PROGRAMDATA_METADATA_LEN + payload.len()];
        data[..4].copy_from_slice(&LOADER_STATE_TAG_PROGRAMDATA.to_le_bytes());
        data[4..12].copy_from_slice(&slot.to_le_bytes());
        if let Some(authority) = authority {
            data[12] = 1;
            data[13..45].copy_from_slice(authority.as_ref());
        }
        data[45..].copy_from_slice(payload);
        data
    }

    #[test]
    fn exact_program_buffer_and_programdata_headers_are_parsed() {
        let programdata = key(1);
        let authority = key(2);
        assert_eq!(
            parse_upgradeable_program(&program_bytes(programdata)).unwrap(),
            UpgradeableProgramHeaderV1 {
                programdata_address: programdata,
            }
        );
        assert_eq!(
            parse_upgradeable_buffer(&buffer_bytes(Some(authority), &[7; 19])).unwrap(),
            UpgradeableBufferHeaderV1 {
                authority: Some(authority),
                payload_offset: 37,
                payload_length: 19,
            }
        );
        assert_eq!(
            parse_upgradeable_programdata(&programdata_bytes(42, Some(authority), &[9; 31]))
                .unwrap(),
            UpgradeableProgramDataHeaderV1 {
                deployed_slot: 42,
                upgrade_authority: Some(authority),
                payload_offset: 45,
                capacity: 31,
            }
        );
    }

    #[test]
    fn immutable_authority_encoding_preserves_loader_v3_stale_padding_semantics() {
        let buffer = buffer_bytes(None, &[1]);
        assert_eq!(parse_upgradeable_buffer(&buffer).unwrap().authority, None);
        let programdata = programdata_bytes(1, None, &[2]);
        assert_eq!(
            parse_upgradeable_programdata(&programdata)
                .unwrap()
                .upgrade_authority,
            None
        );

        let mut nonzero_none = programdata;
        nonzero_none[13..45].copy_from_slice(key(9).as_ref());
        assert_eq!(
            parse_upgradeable_programdata(&nonzero_none)
                .unwrap()
                .upgrade_authority,
            None
        );
        let mut default_some = buffer;
        default_some[4] = 1;
        assert_eq!(
            parse_upgradeable_buffer(&default_some),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn every_tag_size_option_and_payload_boundary_fails_closed() {
        let authority = key(3);
        let mut program = program_bytes(key(4));
        program[0] = LOADER_STATE_TAG_BUFFER as u8;
        assert_eq!(
            parse_upgradeable_program(&program),
            Err(GovernanceError::InvalidRelease1Account)
        );
        assert_eq!(
            parse_upgradeable_program(&program_bytes(key(4))[..35]),
            Err(GovernanceError::InvalidRelease1Account)
        );
        let mut trailing = program_bytes(key(4));
        trailing.push(0);
        assert_eq!(
            parse_upgradeable_program(&trailing),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut buffer = buffer_bytes(Some(authority), &[]);
        buffer[4] = 2;
        assert_eq!(
            parse_upgradeable_buffer(&buffer),
            Err(GovernanceError::InvalidRelease1Account)
        );
        assert_eq!(
            parse_upgradeable_buffer(&buffer[..36]),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut programdata = programdata_bytes(9, Some(authority), &[]);
        programdata[12] = 2;
        assert_eq!(
            parse_upgradeable_programdata(&programdata),
            Err(GovernanceError::InvalidRelease1Account)
        );
        assert_eq!(
            parse_upgradeable_programdata(&programdata[..44]),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn hashes_cover_exact_raw_and_payload_bytes() {
        let data = programdata_bytes(5, Some(key(6)), &[1, 2, 3, 4]);
        let raw = loader_account_data_hash(&data);
        let payload = loader_payload_hash(&data, LOADER_PROGRAMDATA_METADATA_LEN).unwrap();
        assert_eq!(raw, hashv(&[&data]).to_bytes());
        assert_eq!(payload, hashv(&[&[1, 2, 3, 4]]).to_bytes());
        assert_ne!(raw, payload);
        assert_eq!(
            loader_payload_hash(&data, data.len() + 1),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn account_linkage_uses_actual_owner_executable_and_programdata_bytes() {
        let loader = key(10);
        let program_key = key(11);
        let programdata_key = key(12);
        let authority = key(13);
        let mut program_lamports = 1;
        let mut programdata_lamports = 1;
        let mut program_data = program_bytes(programdata_key);
        let mut programdata_data = programdata_bytes(77, Some(authority), &[4; 8]);
        let program_info = AccountInfo::new(
            &program_key,
            false,
            false,
            &mut program_lamports,
            &mut program_data,
            &loader,
            true,
            0,
        );
        let programdata_info = AccountInfo::new(
            &programdata_key,
            false,
            false,
            &mut programdata_lamports,
            &mut programdata_data,
            &loader,
            false,
            0,
        );
        assert_eq!(
            validate_program_programdata_linkage(&program_info, &programdata_info, &loader)
                .unwrap(),
            UpgradeableProgramDataHeaderV1 {
                deployed_slot: 77,
                upgrade_authority: Some(authority),
                payload_offset: 45,
                capacity: 8,
            }
        );

        let wrong_loader = key(14);
        assert_eq!(
            validate_program_programdata_linkage(&program_info, &programdata_info, &wrong_loader,),
            Err(GovernanceError::IncorrectAccountOwner)
        );
    }
}
