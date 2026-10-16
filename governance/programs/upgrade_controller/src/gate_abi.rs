//! Execution-free codecs for the Phase 3 Spread gate bridge ABI.
//!
//! These helpers describe bytes only. They do not mutate a gate, dispatch a
//! target instruction, or add an executable controller instruction.

use solana_program::pubkey::Pubkey;
use thiserror::Error;

use crate::state::{
    GateStatusV1, ProtocolGateV1, ACCOUNT_VERSION_V1, PROTOCOL_GATE_DISCRIMINATOR,
    PROTOCOL_GATE_RESERVED_LEN,
};

pub const GOVERNANCE_TAIL_MAGIC: [u8; 4] = *b"AGV1";
pub const GOVERNANCE_TAIL_VERSION_V1: u8 = 1;
pub const GOVERNANCE_TAIL_LEN: usize = 16;

pub mod protocol_gate_offset {
    pub const DISCRIMINATOR: usize = 0;
    pub const VERSION: usize = 8;
    pub const BUMP: usize = 9;
    pub const INITIALIZED: usize = 10;
    pub const STATUS: usize = 11;
    pub const CONTROLLER_CONFIG: usize = 12;
    pub const TARGET_PROGRAM: usize = 44;
    pub const TARGET_PROGRAMDATA: usize = 76;
    pub const EPOCH: usize = 108;
    pub const ACTIVE_PROPOSAL: usize = 116;
    pub const FREEZE_SLOT: usize = 148;
    pub const FREEZE_REASON_CODE: usize = 156;
    pub const LAST_COMPLETED_PROPOSAL: usize = 158;
    pub const RESERVED: usize = 190;
}

pub mod governance_tail_offset {
    pub const MAGIC: usize = 0;
    pub const VERSION: usize = 4;
    pub const RESERVED: usize = 5;
    pub const EXPECTED_EPOCH: usize = 8;
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GateAbiError {
    #[error("{field} must be exactly {expected} bytes, got {actual}")]
    InvalidLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("invalid ProtocolGateV1 discriminator")]
    InvalidGateDiscriminator,
    #[error("unsupported ProtocolGateV1 version")]
    UnsupportedGateVersion,
    #[error("ProtocolGateV1 initialized byte is not canonical")]
    NoncanonicalInitialized,
    #[error("ProtocolGateV1 is not initialized")]
    UninitializedGate,
    #[error("unknown GateStatusV1 value {0}")]
    UnknownGateStatus(u8),
    #[error("ProtocolGateV1 reserved bytes must be zero")]
    NonzeroGateReserved,
    #[error("ProtocolGateV1 fields are not a canonical V1 state")]
    InvalidGateState,
    #[error("invalid governance tail magic")]
    InvalidTailMagic,
    #[error("unsupported governance tail version")]
    UnsupportedTailVersion,
    #[error("governance tail reserved bytes must be zero")]
    NonzeroTailReserved,
    #[error("instruction length overflow")]
    InstructionLengthOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GovernanceInstructionTailV1 {
    pub magic: [u8; 4],
    pub version: u8,
    pub reserved: [u8; 3],
    pub expected_epoch: u64,
}

impl GovernanceInstructionTailV1 {
    pub const fn new(expected_epoch: u64) -> Self {
        Self {
            magic: GOVERNANCE_TAIL_MAGIC,
            version: GOVERNANCE_TAIL_VERSION_V1,
            reserved: [0; 3],
            expected_epoch,
        }
    }

    pub fn encode(&self) -> Result<[u8; GOVERNANCE_TAIL_LEN], GateAbiError> {
        if self.magic != GOVERNANCE_TAIL_MAGIC {
            return Err(GateAbiError::InvalidTailMagic);
        }
        if self.version != GOVERNANCE_TAIL_VERSION_V1 {
            return Err(GateAbiError::UnsupportedTailVersion);
        }
        if self.reserved != [0; 3] {
            return Err(GateAbiError::NonzeroTailReserved);
        }

        let mut out = [0u8; GOVERNANCE_TAIL_LEN];
        out[governance_tail_offset::MAGIC..governance_tail_offset::VERSION]
            .copy_from_slice(&self.magic);
        out[governance_tail_offset::VERSION] = self.version;
        out[governance_tail_offset::RESERVED..governance_tail_offset::EXPECTED_EPOCH]
            .copy_from_slice(&self.reserved);
        out[governance_tail_offset::EXPECTED_EPOCH..]
            .copy_from_slice(&self.expected_epoch.to_le_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, GateAbiError> {
        require_length("GovernanceInstructionTailV1", bytes, GOVERNANCE_TAIL_LEN)?;
        if bytes[governance_tail_offset::MAGIC..governance_tail_offset::VERSION]
            != GOVERNANCE_TAIL_MAGIC
        {
            return Err(GateAbiError::InvalidTailMagic);
        }
        if bytes[governance_tail_offset::VERSION] != GOVERNANCE_TAIL_VERSION_V1 {
            return Err(GateAbiError::UnsupportedTailVersion);
        }
        if bytes[governance_tail_offset::RESERVED..governance_tail_offset::EXPECTED_EPOCH] != [0; 3]
        {
            return Err(GateAbiError::NonzeroTailReserved);
        }

        let mut epoch = [0u8; 8];
        epoch.copy_from_slice(&bytes[governance_tail_offset::EXPECTED_EPOCH..]);
        Ok(Self::new(u64::from_le_bytes(epoch)))
    }
}

pub fn encode_protocol_gate_v1(
    gate: &ProtocolGateV1,
) -> Result<[u8; ProtocolGateV1::LEN], GateAbiError> {
    validate_gate_value(gate)?;

    let mut out = [0u8; ProtocolGateV1::LEN];
    out[protocol_gate_offset::DISCRIMINATOR..protocol_gate_offset::VERSION]
        .copy_from_slice(&gate.discriminator);
    out[protocol_gate_offset::VERSION] = gate.version;
    out[protocol_gate_offset::BUMP] = gate.bump;
    out[protocol_gate_offset::INITIALIZED] = u8::from(gate.initialized);
    out[protocol_gate_offset::STATUS] = gate.status as u8;
    put_pubkey(
        &mut out,
        protocol_gate_offset::CONTROLLER_CONFIG,
        &gate.controller_config,
    );
    put_pubkey(
        &mut out,
        protocol_gate_offset::TARGET_PROGRAM,
        &gate.target_program,
    );
    put_pubkey(
        &mut out,
        protocol_gate_offset::TARGET_PROGRAMDATA,
        &gate.target_programdata,
    );
    out[protocol_gate_offset::EPOCH..protocol_gate_offset::ACTIVE_PROPOSAL]
        .copy_from_slice(&gate.epoch.to_le_bytes());
    put_pubkey(
        &mut out,
        protocol_gate_offset::ACTIVE_PROPOSAL,
        &gate.active_proposal,
    );
    out[protocol_gate_offset::FREEZE_SLOT..protocol_gate_offset::FREEZE_REASON_CODE]
        .copy_from_slice(&gate.freeze_slot.to_le_bytes());
    out[protocol_gate_offset::FREEZE_REASON_CODE..protocol_gate_offset::LAST_COMPLETED_PROPOSAL]
        .copy_from_slice(&gate.freeze_reason_code.to_le_bytes());
    put_pubkey(
        &mut out,
        protocol_gate_offset::LAST_COMPLETED_PROPOSAL,
        &gate.last_completed_proposal,
    );
    out[protocol_gate_offset::RESERVED..].copy_from_slice(&gate.reserved);
    Ok(out)
}

pub fn decode_protocol_gate_v1(bytes: &[u8]) -> Result<ProtocolGateV1, GateAbiError> {
    require_length("ProtocolGateV1", bytes, ProtocolGateV1::LEN)?;
    if bytes[protocol_gate_offset::DISCRIMINATOR..protocol_gate_offset::VERSION]
        != PROTOCOL_GATE_DISCRIMINATOR
    {
        return Err(GateAbiError::InvalidGateDiscriminator);
    }
    if bytes[protocol_gate_offset::VERSION] != ACCOUNT_VERSION_V1 {
        return Err(GateAbiError::UnsupportedGateVersion);
    }
    let initialized = match bytes[protocol_gate_offset::INITIALIZED] {
        0 => false,
        1 => true,
        _ => return Err(GateAbiError::NoncanonicalInitialized),
    };
    let status = match bytes[protocol_gate_offset::STATUS] {
        0 => GateStatusV1::Active,
        1 => GateStatusV1::FrozenForUpgrade,
        2 => GateStatusV1::EmergencyFrozen,
        value => return Err(GateAbiError::UnknownGateStatus(value)),
    };
    if bytes[protocol_gate_offset::RESERVED..] != [0; PROTOCOL_GATE_RESERVED_LEN] {
        return Err(GateAbiError::NonzeroGateReserved);
    }

    let gate = ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: bytes[protocol_gate_offset::VERSION],
        bump: bytes[protocol_gate_offset::BUMP],
        initialized,
        status,
        controller_config: read_pubkey(bytes, protocol_gate_offset::CONTROLLER_CONFIG),
        target_program: read_pubkey(bytes, protocol_gate_offset::TARGET_PROGRAM),
        target_programdata: read_pubkey(bytes, protocol_gate_offset::TARGET_PROGRAMDATA),
        epoch: read_u64(bytes, protocol_gate_offset::EPOCH),
        active_proposal: read_pubkey(bytes, protocol_gate_offset::ACTIVE_PROPOSAL),
        freeze_slot: read_u64(bytes, protocol_gate_offset::FREEZE_SLOT),
        freeze_reason_code: read_u16(bytes, protocol_gate_offset::FREEZE_REASON_CODE),
        last_completed_proposal: read_pubkey(bytes, protocol_gate_offset::LAST_COMPLETED_PROPOSAL),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    };
    validate_gate_value(&gate)?;
    Ok(gate)
}

pub fn envelope_instruction_data_v1(
    legacy_instruction: &[u8],
    tail: &GovernanceInstructionTailV1,
) -> Result<Vec<u8>, GateAbiError> {
    let length = legacy_instruction
        .len()
        .checked_add(GOVERNANCE_TAIL_LEN)
        .ok_or(GateAbiError::InstructionLengthOverflow)?;
    let mut out = Vec::with_capacity(length);
    out.extend_from_slice(legacy_instruction);
    out.extend_from_slice(&tail.encode()?);
    Ok(out)
}

pub fn strip_governance_tail_v1(
    enveloped_instruction: &[u8],
) -> Result<(&[u8], GovernanceInstructionTailV1), GateAbiError> {
    if enveloped_instruction.len() < GOVERNANCE_TAIL_LEN {
        return Err(GateAbiError::InvalidLength {
            field: "enveloped instruction",
            expected: GOVERNANCE_TAIL_LEN,
            actual: enveloped_instruction.len(),
        });
    }
    let split = enveloped_instruction.len() - GOVERNANCE_TAIL_LEN;
    let (legacy, tail_bytes) = enveloped_instruction.split_at(split);
    Ok((legacy, GovernanceInstructionTailV1::decode(tail_bytes)?))
}

fn validate_gate_value(gate: &ProtocolGateV1) -> Result<(), GateAbiError> {
    if gate.discriminator != PROTOCOL_GATE_DISCRIMINATOR {
        return Err(GateAbiError::InvalidGateDiscriminator);
    }
    if gate.version != ACCOUNT_VERSION_V1 {
        return Err(GateAbiError::UnsupportedGateVersion);
    }
    if !gate.initialized {
        return Err(GateAbiError::UninitializedGate);
    }
    if gate.reserved != [0; PROTOCOL_GATE_RESERVED_LEN] {
        return Err(GateAbiError::NonzeroGateReserved);
    }
    gate.validate_static()
        .map_err(|_| GateAbiError::InvalidGateState)
}

fn require_length(field: &'static str, bytes: &[u8], expected: usize) -> Result<(), GateAbiError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(GateAbiError::InvalidLength {
            field,
            expected,
            actual: bytes.len(),
        })
    }
}

fn put_pubkey(out: &mut [u8; ProtocolGateV1::LEN], offset: usize, value: &Pubkey) {
    out[offset..offset + 32].copy_from_slice(value.as_ref());
}

fn read_pubkey(bytes: &[u8], offset: usize) -> Pubkey {
    let mut value = [0u8; 32];
    value.copy_from_slice(&bytes[offset..offset + 32]);
    Pubkey::new_from_array(value)
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    let mut value = [0u8; 2];
    value.copy_from_slice(&bytes[offset..offset + 2]);
    u16::from_le_bytes(value)
}
