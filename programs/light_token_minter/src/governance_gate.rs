//! Fixed Phase 3 bridge ABI for the controller-owned protocol gate.
//!
//! This module deliberately duplicates the small wire contract instead of linking the controller
//! crate. The Phase 3 identity remains explicitly synthetic and local-test-only. A future
//! production identity is accepted only through generated source bound to a reviewed manifest and
//! exact config/gate PDA derivations.
//!
//! The routing capability is intentionally outside the public crate API:
//! ```compile_fail
//! use light_token_minter::governance_gate::GateValidated;
//! ```

#[cfg(feature = "governance-gate-v1")]
use solana_program::account_info::AccountInfo;
use solana_program::{program_error::ProgramError, pubkey::Pubkey};
use solana_sdk_ids::bpf_loader_upgradeable;

use crate::error::VaultError;

pub const GOVERNANCE_TAIL_MAGIC: [u8; 4] = *b"AGV1";
pub const GOVERNANCE_TAIL_VERSION_V1: u8 = 1;
pub const GOVERNANCE_TAIL_LEN: usize = 16;
pub const PROTOCOL_GATE_DISCRIMINATOR: [u8; 8] = *b"AGVGAT01";
pub const PROTOCOL_GATE_VERSION_V1: u8 = 1;
pub const PROTOCOL_GATE_LEN: usize = 192;

#[cfg(not(feature = "devnet-v3-governance-controller"))]
const UPGRADE_SEED_DOMAIN_V1: &[u8] = b"ameba-upgrade-v1";
#[cfg(feature = "devnet-v3-governance-controller")]
const UPGRADE_SEED_DOMAIN_V1: &[u8] = b"ameba-governance-v3";
#[cfg(not(feature = "devnet-v3-governance-controller"))]
const TARGET_SEED: &[u8] = b"target";
const GATE_SEED: &[u8] = b"gate";

#[cfg(feature = "devnet-v3-governance-controller")]
mod devnet_v3_identity;
#[cfg(feature = "devnet-v3-governance-controller")]
pub use devnet_v3_identity::{
    PINNED_CONTROLLER_CONFIG_PDA, PINNED_CONTROLLER_PROGRAM_ID, PINNED_PROTOCOL_GATE_PDA,
};
#[cfg(feature = "devnet-v3-governance-controller")]
#[inline(never)]
fn emit_selected_controller_release_marker() {
    solana_program::msg!("AMEBA_SPREAD_DEVNET_V3:2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw:8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx");
}

#[cfg(feature = "phase3-synthetic-governance-controller")]
pub const PINNED_CONTROLLER_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi");

#[cfg(feature = "reviewed-governance-controller")]
mod reviewed_controller_identity {
    include!(env!("AMEBA_GOVERNANCE_BRIDGE_IDENTITY_RS"));
}

#[cfg(feature = "reviewed-governance-controller")]
pub use reviewed_controller_identity::{
    PINNED_CONTROLLER_CONFIG_PDA, PINNED_CONTROLLER_PROGRAM_ID, PINNED_PROTOCOL_GATE_PDA,
    REVIEWED_IDENTITY_GENERATION, REVIEWED_IDENTITY_LOCAL_TEST_ONLY,
    REVIEWED_IDENTITY_MANIFEST_SHA256, REVIEWED_IDENTITY_RELEASE_MARKER,
    REVIEWED_IDENTITY_REVIEW_SHA256,
};

/// Deliberately reachable in every synthetic bridge admission so release tooling can prove that
/// an artifact was not built with the Phase 3-only controller hidden through Cargo rustflags or
/// configuration. The warning is intentionally absent from every production-feature build.
#[cfg(feature = "phase3-synthetic-governance-controller")]
#[inline(never)]
fn emit_selected_controller_release_marker() {
    solana_program::msg!("AMEBA_PHASE3_SYNTHETIC_CONTROLLER_DO_NOT_RELEASE");
}

/// Every reviewed bridge admission retains the exact manifest/review commitment in the artifact.
/// Local ceremony generation uses a distinct `DO_NOT_RELEASE` marker and is rejected by the
/// release wrapper even though it exercises this same production-shaped compile path.
#[cfg(feature = "reviewed-governance-controller")]
#[inline(never)]
fn emit_selected_controller_release_marker() {
    solana_program::log::sol_log(REVIEWED_IDENTITY_RELEASE_MARKER);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GovernanceInstructionTailV1 {
    pub magic: [u8; 4],
    pub version: u8,
    pub reserved: [u8; 3],
    pub expected_epoch: u64,
}

impl GovernanceInstructionTailV1 {
    pub fn decode_exact(bytes: &[u8]) -> Result<Self, ProgramError> {
        if bytes.len() != GOVERNANCE_TAIL_LEN {
            return Err(VaultError::InvalidGovernanceTail.into());
        }
        let tail = Self {
            magic: bytes[0..4]
                .try_into()
                .map_err(|_| VaultError::InvalidGovernanceTail)?,
            version: bytes[4],
            reserved: bytes[5..8]
                .try_into()
                .map_err(|_| VaultError::InvalidGovernanceTail)?,
            expected_epoch: u64::from_le_bytes(
                bytes[8..16]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceTail)?,
            ),
        };
        if tail.magic != GOVERNANCE_TAIL_MAGIC
            || tail.version != GOVERNANCE_TAIL_VERSION_V1
            || tail.reserved != [0; 3]
        {
            return Err(VaultError::InvalidGovernanceTail.into());
        }
        Ok(tail)
    }

    pub fn encode(self) -> [u8; GOVERNANCE_TAIL_LEN] {
        let mut bytes = [0u8; GOVERNANCE_TAIL_LEN];
        bytes[0..4].copy_from_slice(&self.magic);
        bytes[4] = self.version;
        bytes[5..8].copy_from_slice(&self.reserved);
        bytes[8..16].copy_from_slice(&self.expected_epoch.to_le_bytes());
        bytes
    }

    pub fn for_epoch(expected_epoch: u64) -> Self {
        Self {
            magic: GOVERNANCE_TAIL_MAGIC,
            version: GOVERNANCE_TAIL_VERSION_V1,
            reserved: [0; 3],
            expected_epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GateStatusV1 {
    Active = 0,
    FrozenForUpgrade = 1,
    EmergencyFrozen = 2,
}

impl GateStatusV1 {
    fn decode(value: u8) -> Result<Self, ProgramError> {
        match value {
            0 => Ok(Self::Active),
            1 => Ok(Self::FrozenForUpgrade),
            2 => Ok(Self::EmergencyFrozen),
            _ => Err(VaultError::InvalidGovernanceGateData.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolGateV1 {
    pub bump: u8,
    pub status: GateStatusV1,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub epoch: u64,
    pub active_proposal: Pubkey,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub last_completed_proposal: Pubkey,
}

impl ProtocolGateV1 {
    pub fn decode_exact(bytes: &[u8]) -> Result<Self, ProgramError> {
        if bytes.len() != PROTOCOL_GATE_LEN
            || bytes[0..8] != PROTOCOL_GATE_DISCRIMINATOR
            || bytes[8] != PROTOCOL_GATE_VERSION_V1
            || bytes[10] != 1
            || bytes[190..192] != [0, 0]
        {
            return Err(VaultError::InvalidGovernanceGateData.into());
        }
        let status = GateStatusV1::decode(bytes[11])?;
        let gate = Self {
            bump: bytes[9],
            status,
            controller_config: Pubkey::new_from_array(
                bytes[12..44]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            target_program: Pubkey::new_from_array(
                bytes[44..76]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            target_programdata: Pubkey::new_from_array(
                bytes[76..108]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            epoch: u64::from_le_bytes(
                bytes[108..116]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            active_proposal: Pubkey::new_from_array(
                bytes[116..148]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            freeze_slot: u64::from_le_bytes(
                bytes[148..156]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            freeze_reason_code: u16::from_le_bytes(
                bytes[156..158]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
            last_completed_proposal: Pubkey::new_from_array(
                bytes[158..190]
                    .try_into()
                    .map_err(|_| VaultError::InvalidGovernanceGateData)?,
            ),
        };
        gate.validate_canonical_status()?;
        Ok(gate)
    }

    fn validate_canonical_status(&self) -> Result<(), ProgramError> {
        let proposal_is_default = self.active_proposal == Pubkey::default();
        let freeze_is_clear = self.freeze_slot == 0 && self.freeze_reason_code == 0;
        let freeze_is_set = self.freeze_slot != 0 && self.freeze_reason_code != 0;
        let canonical = match self.status {
            GateStatusV1::Active => proposal_is_default && freeze_is_clear,
            GateStatusV1::FrozenForUpgrade => !proposal_is_default && freeze_is_set,
            GateStatusV1::EmergencyFrozen => proposal_is_default && freeze_is_set,
        };
        if !canonical {
            return Err(VaultError::InvalidGovernanceGateData.into());
        }
        Ok(())
    }
}

/// Authorization capability carried from the top-level envelope into every business router.
/// Its fields and constructors stay private to this module.
pub(crate) struct GateValidated {
    expected_epoch: u64,
    _private: (),
}

impl GateValidated {
    pub(crate) fn expected_epoch(&self) -> u64 {
        self.expected_epoch
    }
}

fn validated_capability(expected_epoch: u64) -> GateValidated {
    GateValidated {
        expected_epoch,
        _private: (),
    }
}

#[cfg(not(feature = "governance-gate-v1"))]
pub(crate) fn disabled_build_capability() -> GateValidated {
    validated_capability(0)
}

pub fn derive_controller_config_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    #[cfg(feature = "devnet-v3-governance-controller")]
    {
        let _ = target_program;
        Pubkey::find_program_address(&[UPGRADE_SEED_DOMAIN_V1, b"council"], controller_program)
    }
    #[cfg(not(feature = "devnet-v3-governance-controller"))]
    Pubkey::find_program_address(
        &[UPGRADE_SEED_DOMAIN_V1, TARGET_SEED, target_program.as_ref()],
        controller_program,
    )
}

pub fn derive_protocol_gate_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[UPGRADE_SEED_DOMAIN_V1, GATE_SEED, target_program.as_ref()],
        controller_program,
    )
}

pub fn derive_target_programdata_pda(target_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[target_program.as_ref()], &bpf_loader_upgradeable::id()).0
}

#[cfg(feature = "governance-gate-v1")]
pub(crate) fn validate_top_level_envelope<'accounts, 'info, 'data>(
    program_id: &Pubkey,
    accounts: &'accounts [AccountInfo<'info>],
    instruction_data: &'data [u8],
) -> Result<(&'accounts [AccountInfo<'info>], &'data [u8], GateValidated), ProgramError> {
    emit_selected_controller_release_marker();
    if instruction_data.len() <= GOVERNANCE_TAIL_LEN {
        return Err(VaultError::MissingGovernanceTail.into());
    }
    let legacy_data_len = instruction_data.len() - GOVERNANCE_TAIL_LEN;
    let tail = GovernanceInstructionTailV1::decode_exact(&instruction_data[legacy_data_len..])?;

    let gate_info = accounts.last().ok_or(VaultError::MissingGovernanceGate)?;
    if program_id != &crate::id() {
        return Err(VaultError::InvalidGovernanceGateData.into());
    }
    let (expected_gate, expected_bump) =
        derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, program_id);
    #[cfg(any(
        feature = "reviewed-governance-controller",
        feature = "devnet-v3-governance-controller"
    ))]
    if expected_gate != PINNED_PROTOCOL_GATE_PDA {
        return Err(VaultError::InvalidGovernanceGatePda.into());
    }
    if gate_info.key != &expected_gate {
        return Err(VaultError::InvalidGovernanceGatePda.into());
    }
    if accounts[..accounts.len() - 1]
        .iter()
        .any(|account| account.key == &expected_gate)
    {
        return Err(VaultError::DuplicateGovernanceGate.into());
    }
    if gate_info.is_signer || gate_info.is_writable || gate_info.executable {
        return Err(VaultError::InvalidGovernanceGatePrivileges.into());
    }
    if gate_info.owner != &PINNED_CONTROLLER_PROGRAM_ID {
        return Err(VaultError::InvalidGovernanceGateOwner.into());
    }

    let gate = {
        let data = gate_info
            .try_borrow_data()
            .map_err(|_| VaultError::InvalidGovernanceGateData)?;
        ProtocolGateV1::decode_exact(&data)?
    };
    let expected_controller_config =
        derive_controller_config_pda(&PINNED_CONTROLLER_PROGRAM_ID, program_id).0;
    #[cfg(any(
        feature = "reviewed-governance-controller",
        feature = "devnet-v3-governance-controller"
    ))]
    if expected_controller_config != PINNED_CONTROLLER_CONFIG_PDA {
        return Err(VaultError::InvalidGovernanceGatePda.into());
    }
    if gate.bump != expected_bump {
        return Err(VaultError::InvalidGovernanceGatePda.into());
    }
    if gate.controller_config != expected_controller_config
        || gate.target_program != *program_id
        || gate.target_programdata != derive_target_programdata_pda(program_id)
    {
        return Err(VaultError::InvalidGovernanceGateData.into());
    }
    if gate.status != GateStatusV1::Active {
        return Err(VaultError::GovernanceGateFrozen.into());
    }
    if tail.expected_epoch != gate.epoch {
        return Err(VaultError::GovernanceGateEpochMismatch.into());
    }

    Ok((
        &accounts[..accounts.len() - 1],
        &instruction_data[..legacy_data_len],
        validated_capability(tail.expected_epoch),
    ))
}

pub(crate) fn ends_with_valid_governance_tail(instruction_data: &[u8]) -> bool {
    instruction_data.len() > GOVERNANCE_TAIL_LEN
        && GovernanceInstructionTailV1::decode_exact(
            &instruction_data[instruction_data.len() - GOVERNANCE_TAIL_LEN..],
        )
        .is_ok()
}
