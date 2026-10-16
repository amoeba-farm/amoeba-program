//! Fixed-width Release 1 ceremony-closure account schemas.
//!
//! This module only defines byte contracts and their intrinsic validation. It
//! deliberately does not compute account digests, derive PDAs, or perform any
//! lifecycle transition. Published Release 1 V1/V2 bytes remain unchanged.

use std::io::{Error, ErrorKind, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{pubkey, pubkey::Pubkey};

use crate::{
    artifact_merkle::{
        artifact_chunk_count, ARTIFACT_MERKLE_SCHEME_ID, MAX_ARTIFACT_BYTES_V1,
        RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    council::VALID_APPROVAL_MASK,
    pda::UPGRADEABLE_LOADER_ID,
    programdata_observation_merkle::{
        is_programdata_observation_chunk_size_candidate, programdata_observation_chunk_count,
        MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1, MAX_PROGRAMDATA_OBSERVATION_CHUNKS_V1,
        MAX_PROGRAMDATA_OBSERVATION_TREE_DEPTH_V1, MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB, PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1,
        PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
    },
    release1_loader_accounts::{
        parse_upgradeable_program, parse_upgradeable_programdata, LOADER_PROGRAMDATA_METADATA_LEN,
        LOADER_PROGRAM_ACCOUNT_LEN,
    },
    release1_state::{BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1, RELEASE1_APPROVAL_THRESHOLD},
    state::{GateStatusV1, OptionalPubkeyV1},
    GovernanceError, GovernanceResult,
};

pub const CEREMONY_ACCOUNT_VERSION_V1: u8 = 1;

pub const PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR: [u8; 8] = *b"AGVCAP01";
pub const CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR: [u8; 8] = *b"AGVREL01";
pub const PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVOBS01";
pub const CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR: [u8; 8] = *b"AGVDEP01";
pub const CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR: [u8; 8] = *b"AGVIMR01";
pub const TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR: [u8; 8] = *b"AGVTHP01";
pub const TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR: [u8; 8] = *b"AGVTHR01";
pub const BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR: [u8; 8] = *b"AGVBAP01";
pub const BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR: [u8; 8] = *b"AGVBAR01";

pub const MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1: u64 =
    MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 - LOADER_PROGRAMDATA_METADATA_LEN as u64;
pub const PROGRAMDATA_PAYLOAD_OFFSET_V1: u32 = LOADER_PROGRAMDATA_METADATA_LEN as u32;
pub const ARTIFACT_BINDING_CHUNK_SIZE_V1: u32 = RELEASE1_ARTIFACT_CHUNK_SIZE_V1;
pub const EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1: Pubkey =
    pubkey!("2oMRZEDWT2tqtYMofhmmfQ8SsjqUFzT6sYXppQDavxwz");
pub const SET_AUTHORITY_CHECKED_FEATURE_ID_V1: Pubkey =
    pubkey!("5x3825XS7M2A3Ekbn5VGGkvFoAg5qrRWkTrY4bARP1GL");

pub const PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1: usize =
    PROGRAMDATA_OBSERVATION_FRONTIER_SLOTS_V1;
pub const PROGRAMDATA_OBSERVATION_MAX_TREE_DEPTH_V1: u8 =
    MAX_PROGRAMDATA_OBSERVATION_TREE_DEPTH_V1 as u8;
pub const PROGRAMDATA_OBSERVATION_MAX_REAL_CHUNKS_V1: u32 = MAX_PROGRAMDATA_OBSERVATION_CHUNKS_V1;
pub const PROGRAMDATA_OBSERVATION_MAX_PADDED_LEAVES_V1: u32 =
    MAX_PADDED_PROGRAMDATA_OBSERVATION_CHUNKS_V1;

pub const CEREMONY_PROPOSAL_COMPLETED_REASON_V1: u16 = 1;
pub const CEREMONY_PROPOSAL_EXPIRED_REASON_V1: u16 = 2;

pub const PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN: usize = 122;
pub const CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN: usize = 59;
pub const PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN: usize = 15;
pub const CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN: usize = 187;
pub const CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN: usize = 218;
pub const TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN: usize = 212;
pub const TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN: usize = 112;
pub const BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN: usize = 180;
pub const BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN: usize = 56;

macro_rules! fixed_u8_enum_borsh {
    ($name:ident { $($variant:ident = $value:expr),+ $(,)? }) => {
        impl BorshSerialize for $name {
            fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
                writer.write_all(&[*self as u8])
            }
        }

        impl BorshDeserialize for $name {
            fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
                let mut byte = [0u8; 1];
                reader.read_exact(&mut byte)?;
                match byte[0] {
                    $($value => Ok(Self::$variant),)+
                    value => Err(Error::new(
                        ErrorKind::InvalidData,
                        format!("unknown {} discriminant {value}", stringify!($name)),
                    )),
                }
            }
        }
    };
}

macro_rules! impl_strict_account {
    ($name:ident, $len:expr) => {
        impl $name {
            pub const LEN: usize = $len;

            #[cfg(not(target_os = "solana"))]
            pub fn from_bytes_strict(data: &[u8]) -> GovernanceResult<Self> {
                if data.len() != Self::LEN {
                    return Err(GovernanceError::InvalidAccountSize);
                }
                let value = Self::try_from_slice(data)
                    .map_err(|_| GovernanceError::InvalidRelease1Account)?;
                value.validate_static()?;
                Ok(value)
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ProgramDataObservationPurposeV1 {
    ControllerImmutability = 0,
    TargetHandoffBridge = 1,
    ProposalPrestate = 2,
    PostUpgrade = 3,
    Rollback = 4,
    EmergencyResolution = 5,
    BootstrapActivation = 6,
}
fixed_u8_enum_borsh!(ProgramDataObservationPurposeV1 {
    ControllerImmutability = 0,
    TargetHandoffBridge = 1,
    ProposalPrestate = 2,
    PostUpgrade = 3,
    Rollback = 4,
    EmergencyResolution = 5,
    BootstrapActivation = 6,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgramDataObservationStatusV1 {
    Accumulating = 0,
    ReadyToFinalize = 1,
    Finalized = 2,
    Stale = 3,
}
fixed_u8_enum_borsh!(ProgramDataObservationStatusV1 {
    Accumulating = 0,
    ReadyToFinalize = 1,
    Finalized = 2,
    Stale = 3,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CeremonyProposalStateV1 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Completed = 3,
    Expired = 4,
}
fixed_u8_enum_borsh!(CeremonyProposalStateV1 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Completed = 3,
    Expired = 4,
});

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataCapacityPolicyV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub loader_programdata_metadata_len: u64,
    pub maximum_raw_programdata_length: u64,
    pub maximum_payload_capacity: u64,
    pub maximum_artifact_length: u64,
    pub observation_scheme_id: [u8; 32],
    pub observation_chunk_size: u32,
    pub observation_max_chunk_count: u32,
    pub observation_padded_leaf_count: u32,
    pub observation_tree_depth: u8,
    pub observation_frontier_hash_count: u8,
    pub artifact_scheme_id: [u8; 32],
    pub artifact_chunk_size: u32,
    pub zero_tail_required: bool,
    pub extend_program_checked_feature: Pubkey,
    pub set_authority_checked_feature: Pubkey,
    pub policy_digest: [u8; 32],
    pub creation_slot: u64,
    pub reserved: [u8; PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN],
}

impl ProgramDataCapacityPolicyV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.extend_program_checked_feature,
            self.set_authority_checked_feature,
        ])?;
        let observation_chunk_size_is_admitted = self.observation_chunk_size
            == PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB
            || cfg!(feature = "programdata-observation-chunk-matrix")
                && is_programdata_observation_chunk_size_candidate(self.observation_chunk_size);
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.extend_program_checked_feature != EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1
            || self.set_authority_checked_feature != SET_AUTHORITY_CHECKED_FEATURE_ID_V1
            || self.loader_programdata_metadata_len != LOADER_PROGRAMDATA_METADATA_LEN as u64
            || self.maximum_raw_programdata_length != MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
            || self.maximum_payload_capacity != MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.maximum_artifact_length != MAX_ARTIFACT_BYTES_V1
            || self.observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
            || !observation_chunk_size_is_admitted
            || self.artifact_scheme_id != ARTIFACT_MERKLE_SCHEME_ID
            || self.artifact_chunk_size != ARTIFACT_BINDING_CHUNK_SIZE_V1
            || !self.zero_tail_required
            || self.policy_digest == [0; 32]
            || self.creation_slot == 0
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let (count, padded, depth) = observation_geometry(
            self.maximum_raw_programdata_length,
            self.observation_chunk_size,
        )?;
        if self.observation_max_chunk_count != count
            || self.observation_padded_leaf_count != padded
            || self.observation_tree_depth != depth
            || self.observation_frontier_hash_count
                != PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1 as u8
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(ProgramDataCapacityPolicyV1, 512);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ControllerReleaseCommitmentV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub source_commitment: [u8; 32],
    pub source_tree_commitment: [u8; 32],
    pub build_inputs_commitment: [u8; 32],
    pub toolchain_commitment: [u8; 32],
    pub package_commitment: [u8; 32],
    pub release_manifest_commitment: [u8; 32],
    pub abi_commitment: [u8; 32],
    pub pre_immutability_authority: OptionalPubkeyV1,
    pub minimum_programdata_capacity: u64,
    pub release_digest: [u8; 32],
    pub creation_slot: u64,
    pub finalized: bool,
    pub reserved: [u8; CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN],
}

impl ControllerReleaseCommitmentV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_programdata,
            self.upgradeable_loader,
            self.capacity_policy,
        ])?;
        self.pre_immutability_authority.validate()?;
        validate_artifact_identity(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        require_nonzero_hashes(&[
            self.capacity_policy_digest,
            self.source_commitment,
            self.source_tree_commitment,
            self.build_inputs_commitment,
            self.toolchain_commitment,
            self.package_commitment,
            self.release_manifest_commitment,
            self.abi_commitment,
            self.release_digest,
        ])?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || !self.pre_immutability_authority.present
            || self.minimum_programdata_capacity < self.artifact_length
            || self.minimum_programdata_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.creation_slot == 0
            || !self.finalized
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(ControllerReleaseCommitmentV1, 640);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataObservationV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub purpose: ProgramDataObservationPurposeV1,
    pub subject: Pubkey,
    pub subject_digest: [u8; 32],
    pub generation: u64,
    pub protocol_gate: Pubkey,
    pub gate_status: GateStatusV1,
    pub gate_epoch: u64,
    pub gate_active_proposal: Pubkey,
    pub gate_freeze_slot: u64,
    pub gate_freeze_reason_code: u16,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub program_owner: Pubkey,
    pub program_executable: bool,
    pub program_data_length: u64,
    pub program_header_present: bool,
    pub program_header_snapshot: [u8; LOADER_PROGRAM_ACCOUNT_LEN],
    pub linked_programdata: Pubkey,
    pub programdata_owner: Pubkey,
    pub programdata_executable: bool,
    pub programdata_header_present: bool,
    pub programdata_header_snapshot: [u8; LOADER_PROGRAMDATA_METADATA_LEN],
    pub deployed_slot: u64,
    pub upgrade_authority: OptionalPubkeyV1,
    pub raw_data_length: u64,
    pub payload_offset: u32,
    pub actual_capacity: u64,
    pub expected_artifact_length: u64,
    pub expected_artifact_sha256: [u8; 32],
    pub expected_artifact_merkle_root: [u8; 32],
    pub expected_artifact_scheme_id: [u8; 32],
    pub artifact_chunk_size: u32,
    pub artifact_chunk_count: u32,
    pub minimum_required_capacity: u64,
    pub raw_observation_scheme_id: [u8; 32],
    pub raw_chunk_size: u32,
    pub raw_chunk_count: u32,
    pub raw_padded_leaf_count: u32,
    pub raw_tree_depth: u8,
    pub next_raw_chunk_index: u32,
    pub raw_frontier: [[u8; 32]; PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1],
    pub raw_frontier_mask: u16,
    pub next_artifact_chunk_index: u32,
    pub tail_bytes_verified: u64,
    pub start_slot: u64,
    pub last_observed_slot: u64,
    pub finalized_slot: u64,
    pub final_raw_merkle_root: [u8; 32],
    pub observation_digest: [u8; 32],
    pub status: ProgramDataObservationStatusV1,
    pub reserved: [u8; PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN],
}

impl ProgramDataObservationV1 {
    pub fn purpose_binding(&self) -> (ProgramDataObservationPurposeV1, Pubkey, [u8; 32], u64) {
        (
            self.purpose,
            self.subject,
            self.subject_digest,
            self.generation,
        )
    }

    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.capacity_policy,
            self.subject,
            self.protocol_gate,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.program_owner,
            self.linked_programdata,
            self.programdata_owner,
        ])?;
        require_nonzero_hashes(&[
            self.capacity_policy_digest,
            self.subject_digest,
            self.expected_artifact_sha256,
            self.expected_artifact_merkle_root,
            self.expected_artifact_scheme_id,
            self.raw_observation_scheme_id,
        ])?;
        self.upgrade_authority.validate()?;
        if self.generation == 0
            || self.gate_epoch == 0
            || self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.program_owner != self.upgradeable_loader
            || self.programdata_owner != self.upgradeable_loader
            || !self.program_executable
            || self.programdata_executable
            || !self.program_header_present
            || !self.programdata_header_present
            || self.program_data_length != LOADER_PROGRAM_ACCOUNT_LEN as u64
            || self.linked_programdata != self.target_programdata
            || self.payload_offset != PROGRAMDATA_PAYLOAD_OFFSET_V1
            || self.raw_data_length > MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
            || self.actual_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.raw_data_length
                != self
                    .actual_capacity
                    .checked_add(u64::from(self.payload_offset))
                    .ok_or(GovernanceError::ArithmeticOverflow)?
            || self.minimum_required_capacity < self.expected_artifact_length
            || self.minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.actual_capacity < self.minimum_required_capacity
            || self.deployed_slot == 0
            || self.start_slot == 0
            || self.last_observed_slot < self.start_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let active_gate = self.gate_active_proposal == Pubkey::default()
            && self.gate_freeze_slot == 0
            && self.gate_freeze_reason_code == 0;
        let upgrade_frozen_gate = self.gate_active_proposal == self.subject
            && self.gate_freeze_slot != 0
            && self.gate_freeze_reason_code != 0
            && self.gate_freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1;
        let emergency_frozen_gate = self.gate_active_proposal == Pubkey::default()
            && self.gate_freeze_slot != 0
            && self.gate_freeze_reason_code != 0;
        let gate_matches_purpose = match self.purpose {
            ProgramDataObservationPurposeV1::ControllerImmutability
            | ProgramDataObservationPurposeV1::TargetHandoffBridge
            | ProgramDataObservationPurposeV1::BootstrapActivation => {
                self.gate_status == GateStatusV1::EmergencyFrozen
                    && emergency_frozen_gate
                    && self.gate_freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            }
            ProgramDataObservationPurposeV1::ProposalPrestate
            | ProgramDataObservationPurposeV1::PostUpgrade
            | ProgramDataObservationPurposeV1::Rollback => {
                self.gate_status == GateStatusV1::FrozenForUpgrade && upgrade_frozen_gate
            }
            ProgramDataObservationPurposeV1::EmergencyResolution => {
                self.gate_status == GateStatusV1::EmergencyFrozen
                    && emergency_frozen_gate
                    && self.gate_freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            }
        };
        if active_gate || !gate_matches_purpose {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let program_header = parse_upgradeable_program(&self.program_header_snapshot)?;
        let programdata_header = parse_upgradeable_programdata(&self.programdata_header_snapshot)?;
        if program_header.programdata_address != self.target_programdata
            || programdata_header.deployed_slot != self.deployed_slot
            || !optional_pubkey_matches_option(
                &self.upgrade_authority,
                programdata_header.upgrade_authority,
            )
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_artifact_identity(
            self.expected_artifact_length,
            &self.expected_artifact_sha256,
            &self.expected_artifact_merkle_root,
            &self.expected_artifact_scheme_id,
        )?;
        if self.artifact_chunk_size != ARTIFACT_BINDING_CHUNK_SIZE_V1
            || self.artifact_chunk_count
                != artifact_chunk_count(self.expected_artifact_length, self.artifact_chunk_size)?
            || self.raw_observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let (raw_count, padded_count, depth) =
            observation_geometry(self.raw_data_length, self.raw_chunk_size)?;
        if self.raw_chunk_count != raw_count
            || self.raw_padded_leaf_count != padded_count
            || self.raw_tree_depth != depth
            || self.next_raw_chunk_index > self.raw_chunk_count
            || self.next_artifact_chunk_index > self.artifact_chunk_count
            || self.raw_frontier_mask & !valid_frontier_mask() != 0
            || self.raw_frontier_mask != self.next_raw_chunk_index as u16
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_frontier(&self.raw_frontier, self.raw_frontier_mask)?;

        let expected_tail = self
            .actual_capacity
            .checked_sub(self.expected_artifact_length)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        if self.tail_bytes_verified > expected_tail {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let traversal_complete = self.next_raw_chunk_index == self.raw_chunk_count
            && self.next_artifact_chunk_index == self.artifact_chunk_count
            && self.tail_bytes_verified == expected_tail;
        match self.status {
            ProgramDataObservationStatusV1::Accumulating => {
                if traversal_complete
                    || self.finalized_slot != 0
                    || self.final_raw_merkle_root != [0; 32]
                    || self.observation_digest != [0; 32]
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
            ProgramDataObservationStatusV1::ReadyToFinalize => {
                if !traversal_complete
                    || self.finalized_slot != 0
                    || self.final_raw_merkle_root != [0; 32]
                    || self.observation_digest != [0; 32]
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
            ProgramDataObservationStatusV1::Finalized => {
                if !traversal_complete
                    || self.finalized_slot < self.last_observed_slot
                    || self.final_raw_merkle_root == [0; 32]
                    || self.observation_digest == [0; 32]
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
            ProgramDataObservationStatusV1::Stale => {
                if self.finalized_slot != 0
                    || self.final_raw_merkle_root != [0; 32]
                    || self.observation_digest != [0; 32]
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
        }
        Ok(())
    }
}
impl_strict_account!(ProgramDataObservationV1, 1280);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CurrentDeploymentStateV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub actual_programdata_capacity: u64,
    pub programdata_observation: Pubkey,
    pub observation_generation: u64,
    pub observation_root: [u8; 32],
    pub observation_digest: [u8; 32],
    pub deployed_slot: u64,
    pub installed_authority: Pubkey,
    pub source_commitment: [u8; 32],
    pub build_inputs_commitment: [u8; 32],
    pub package_commitment: [u8; 32],
    pub release_manifest_commitment: [u8; 32],
    pub release_commitment: Pubkey,
    pub release_commitment_digest: [u8; 32],
    pub activation_receipt: OptionalPubkeyV1,
    pub completed_proposal: OptionalPubkeyV1,
    pub gate_epoch_at_activation: u64,
    pub deployment_generation: u64,
    pub deployment_digest: [u8; 32],
    pub last_updated_slot: u64,
    pub reserved: [u8; CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN],
}

impl CurrentDeploymentStateV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.capacity_policy,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.controller_authority,
            self.programdata_observation,
            self.installed_authority,
            self.release_commitment,
        ])?;
        self.activation_receipt.validate()?;
        self.completed_proposal.validate()?;
        validate_artifact_identity(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        require_nonzero_hashes(&[
            self.capacity_policy_digest,
            self.observation_root,
            self.observation_digest,
            self.source_commitment,
            self.build_inputs_commitment,
            self.package_commitment,
            self.release_manifest_commitment,
            self.release_commitment_digest,
            self.deployment_digest,
        ])?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.installed_authority != self.controller_authority
            || self.actual_programdata_capacity < self.artifact_length
            || self.actual_programdata_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.observation_generation == 0
            || self.deployed_slot == 0
            || self.gate_epoch_at_activation == 0
            || self.deployment_generation == 0
            || self.last_updated_slot == 0
            || self.activation_receipt.present == self.completed_proposal.present
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(CurrentDeploymentStateV1, 1024);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ControllerImmutabilityReceiptV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub release_commitment: Pubkey,
    pub release_commitment_digest: [u8; 32],
    pub pre_observation: Pubkey,
    pub pre_observation_generation: u64,
    pub pre_observation_root: [u8; 32],
    pub pre_observation_digest: [u8; 32],
    pub pre_upgrade_authority: OptionalPubkeyV1,
    pub post_observation: Pubkey,
    pub post_observation_generation: u64,
    pub post_observation_root: [u8; 32],
    pub post_observation_digest: [u8; 32],
    pub post_upgrade_authority: OptionalPubkeyV1,
    pub deployed_slot: u64,
    pub raw_programdata_length: u64,
    pub programdata_capacity: u64,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub source_commitment: [u8; 32],
    pub build_inputs_commitment: [u8; 32],
    pub package_commitment: [u8; 32],
    pub release_manifest_commitment: [u8; 32],
    pub finalized_slot: u64,
    pub receipt_digest: [u8; 32],
    pub finalized: bool,
    pub reserved: [u8; CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN],
}

impl ControllerImmutabilityReceiptV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_programdata,
            self.upgradeable_loader,
            self.capacity_policy,
            self.release_commitment,
            self.pre_observation,
            self.post_observation,
        ])?;
        self.pre_upgrade_authority.validate()?;
        self.post_upgrade_authority.validate()?;
        validate_artifact_identity(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        require_nonzero_hashes(&[
            self.capacity_policy_digest,
            self.release_commitment_digest,
            self.pre_observation_root,
            self.pre_observation_digest,
            self.post_observation_root,
            self.post_observation_digest,
            self.source_commitment,
            self.build_inputs_commitment,
            self.package_commitment,
            self.release_manifest_commitment,
            self.receipt_digest,
        ])?;
        validate_authority_change_observations(
            self.pre_observation,
            self.pre_observation_generation,
            &self.pre_observation_root,
            &self.pre_observation_digest,
            self.post_observation,
            self.post_observation_generation,
            &self.post_observation_root,
            &self.post_observation_digest,
        )?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || !self.pre_upgrade_authority.present
            || self.post_upgrade_authority.present
            || self.raw_programdata_length
                != self
                    .programdata_capacity
                    .checked_add(PROGRAMDATA_PAYLOAD_OFFSET_V1.into())
                    .ok_or(GovernanceError::ArithmeticOverflow)?
            || self.programdata_capacity < self.artifact_length
            || self.programdata_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.deployed_slot == 0
            || self.finalized_slot == 0
            || !self.finalized
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(ControllerImmutabilityReceiptV1, 1024);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TargetAuthorityHandoffProposalV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: CeremonyProposalStateV1,
    pub cluster_domain: [u8; 32],
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub controller_immutability_receipt: Pubkey,
    pub controller_immutability_digest: [u8; 32],
    pub controller_config: Pubkey,
    pub governance_policy: Pubkey,
    pub governance_policy_hash: [u8; 32],
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub gate: Pubkey,
    pub controller_authority: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub legacy_target_authority: Pubkey,
    pub bridge_artifact_length: u64,
    pub bridge_artifact_sha256: [u8; 32],
    pub bridge_artifact_merkle_root: [u8; 32],
    pub bridge_artifact_scheme_id: [u8; 32],
    pub bridge_source_commitment: [u8; 32],
    pub bridge_build_inputs_commitment: [u8; 32],
    pub bridge_package_commitment: [u8; 32],
    pub bridge_release_manifest_commitment: [u8; 32],
    pub bridge_observation: Pubkey,
    pub bridge_observation_generation: u64,
    pub bridge_observation_root: [u8; 32],
    pub bridge_observation_digest: [u8; 32],
    pub minimum_target_deployed_slot: u64,
    pub minimum_target_capacity: u64,
    pub minimum_target_raw_length: u64,
    pub bootstrap_gate_status: GateStatusV1,
    pub bootstrap_gate_epoch: u64,
    pub bootstrap_freeze_reason_code: u16,
    pub bootstrap_freeze_slot: u64,
    pub target_nonce: u64,
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub proposal_digest: [u8; 32],
    pub creation_slot: u64,
    pub reserved: [u8; TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN],
}

impl TargetAuthorityHandoffProposalV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        validate_handoff_common(self)?;
        validate_ceremony_proposal_lifecycle(
            self.state,
            self.creation_slot,
            self.review_start_slot,
            self.review_end_slot,
            self.not_before_slot,
            self.expiry_slot,
            self.approval_bitset,
            self.approval_count,
            self.approval_threshold,
            self.first_approval_slot,
            self.council_approved_slot,
            self.queued_slot,
            self.executed_slot,
            self.terminal_slot,
            self.terminal_reason_code,
        )
    }
}
impl_strict_account!(TargetAuthorityHandoffProposalV1, 1280);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TargetAuthorityHandoffReceiptV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub proposal: Pubkey,
    pub proposal_digest: [u8; 32],
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub controller_authority: Pubkey,
    pub controller_immutability_receipt: Pubkey,
    pub controller_immutability_digest: [u8; 32],
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub pre_observation: Pubkey,
    pub pre_observation_generation: u64,
    pub pre_observation_root: [u8; 32],
    pub pre_observation_digest: [u8; 32],
    pub pre_upgrade_authority: OptionalPubkeyV1,
    pub pre_programdata_header_snapshot: [u8; LOADER_PROGRAMDATA_METADATA_LEN],
    pub post_programdata_header_snapshot: [u8; LOADER_PROGRAMDATA_METADATA_LEN],
    pub post_upgrade_authority: OptionalPubkeyV1,
    pub deployed_slot: u64,
    pub raw_programdata_length: u64,
    pub programdata_capacity: u64,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub bridge_source_commitment: [u8; 32],
    pub bridge_build_inputs_commitment: [u8; 32],
    pub bridge_package_commitment: [u8; 32],
    pub bridge_release_manifest_commitment: [u8; 32],
    pub bootstrap_gate_epoch: u64,
    pub target_nonce: u64,
    pub council_version: u64,
    pub accepted_slot: u64,
    pub receipt_digest: [u8; 32],
    pub finalized: bool,
    pub reserved: [u8; TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN],
}

impl TargetAuthorityHandoffReceiptV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.proposal,
            self.controller_program,
            self.controller_config,
            self.controller_authority,
            self.controller_immutability_receipt,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.pre_observation,
        ])?;
        self.pre_upgrade_authority.validate()?;
        self.post_upgrade_authority.validate()?;
        validate_artifact_identity(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        require_nonzero_hashes(&[
            self.proposal_digest,
            self.controller_immutability_digest,
            self.pre_observation_root,
            self.pre_observation_digest,
            self.bridge_source_commitment,
            self.bridge_build_inputs_commitment,
            self.bridge_package_commitment,
            self.bridge_release_manifest_commitment,
            self.receipt_digest,
        ])?;
        let pre_header = parse_upgradeable_programdata(&self.pre_programdata_header_snapshot)?;
        let post_header = parse_upgradeable_programdata(&self.post_programdata_header_snapshot)?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || !self.pre_upgrade_authority.present
            || !self.post_upgrade_authority.present
            || self.pre_upgrade_authority.value == self.controller_authority
            || self.post_upgrade_authority.value != self.controller_authority
            || self.pre_observation_generation == 0
            || pre_header.deployed_slot != self.deployed_slot
            || post_header.deployed_slot != self.deployed_slot
            || !optional_pubkey_matches_option(
                &self.pre_upgrade_authority,
                pre_header.upgrade_authority,
            )
            || !optional_pubkey_matches_option(
                &self.post_upgrade_authority,
                post_header.upgrade_authority,
            )
            || self.raw_programdata_length
                != self
                    .programdata_capacity
                    .checked_add(PROGRAMDATA_PAYLOAD_OFFSET_V1.into())
                    .ok_or(GovernanceError::ArithmeticOverflow)?
            || self.programdata_capacity < self.artifact_length
            || self.programdata_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.deployed_slot == 0
            || self.bootstrap_gate_epoch == 0
            || self.target_nonce == 0
            || self.council_version == 0
            || self.accepted_slot == 0
            || !self.finalized
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(TargetAuthorityHandoffReceiptV1, 1024);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct BootstrapActivationProposalV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: CeremonyProposalStateV1,
    pub cluster_domain: [u8; 32],
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub controller_config: Pubkey,
    pub governance_policy: Pubkey,
    pub governance_policy_hash: [u8; 32],
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub controller_immutability_receipt: Pubkey,
    pub controller_immutability_digest: [u8; 32],
    pub target_handoff_receipt: Pubkey,
    pub target_handoff_digest: [u8; 32],
    pub gate: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub bridge_artifact_length: u64,
    pub bridge_artifact_sha256: [u8; 32],
    pub bridge_artifact_merkle_root: [u8; 32],
    pub bridge_artifact_scheme_id: [u8; 32],
    pub bridge_source_commitment: [u8; 32],
    pub bridge_build_inputs_commitment: [u8; 32],
    pub bridge_package_commitment: [u8; 32],
    pub bridge_release_manifest_commitment: [u8; 32],
    pub bridge_observation: Pubkey,
    pub bridge_observation_generation: u64,
    pub bridge_observation_root: [u8; 32],
    pub bridge_observation_digest: [u8; 32],
    pub minimum_target_deployed_slot: u64,
    pub minimum_target_capacity: u64,
    pub minimum_target_raw_length: u64,
    pub bootstrap_gate_status: GateStatusV1,
    pub bootstrap_gate_epoch: u64,
    pub bootstrap_freeze_reason_code: u16,
    pub bootstrap_freeze_slot: u64,
    pub target_nonce: u64,
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub proposal_digest: [u8; 32],
    pub creation_slot: u64,
    pub reserved: [u8; BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN],
}

impl BootstrapActivationProposalV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        validate_activation_common(self)?;
        validate_ceremony_proposal_lifecycle(
            self.state,
            self.creation_slot,
            self.review_start_slot,
            self.review_end_slot,
            self.not_before_slot,
            self.expiry_slot,
            self.approval_bitset,
            self.approval_count,
            self.approval_threshold,
            self.first_approval_slot,
            self.council_approved_slot,
            self.queued_slot,
            self.executed_slot,
            self.terminal_slot,
            self.terminal_reason_code,
        )
    }
}
impl_strict_account!(BootstrapActivationProposalV1, 1280);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct BootstrapActivationReceiptV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub proposal: Pubkey,
    pub proposal_digest: [u8; 32],
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub governance_policy: Pubkey,
    pub governance_policy_hash: [u8; 32],
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub controller_immutability_receipt: Pubkey,
    pub controller_immutability_digest: [u8; 32],
    pub target_handoff_receipt: Pubkey,
    pub target_handoff_digest: [u8; 32],
    pub gate: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub bridge_observation: Pubkey,
    pub bridge_observation_generation: u64,
    pub bridge_observation_root: [u8; 32],
    pub bridge_observation_digest: [u8; 32],
    pub bridge_artifact_length: u64,
    pub bridge_artifact_sha256: [u8; 32],
    pub bridge_artifact_merkle_root: [u8; 32],
    pub bridge_artifact_scheme_id: [u8; 32],
    pub actual_target_capacity: u64,
    pub target_deployed_slot: u64,
    pub previous_gate_status: GateStatusV1,
    pub previous_gate_epoch: u64,
    pub previous_freeze_reason_code: u16,
    pub previous_freeze_slot: u64,
    pub activated_gate_status: GateStatusV1,
    pub activated_gate_epoch: u64,
    pub target_nonce: u64,
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub deployment_generation: u64,
    pub finalized_slot: u64,
    pub receipt_digest: [u8; 32],
    pub finalized: bool,
    pub reserved: [u8; BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN],
}

impl BootstrapActivationReceiptV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.proposal,
            self.controller_program,
            self.controller_config,
            self.governance_policy,
            self.capacity_policy,
            self.controller_immutability_receipt,
            self.target_handoff_receipt,
            self.gate,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.controller_authority,
            self.bridge_observation,
            self.current_deployment_state,
        ])?;
        require_nonzero_hashes(&[
            self.proposal_digest,
            self.governance_policy_hash,
            self.capacity_policy_digest,
            self.controller_immutability_digest,
            self.target_handoff_digest,
            self.bridge_observation_root,
            self.bridge_observation_digest,
            self.current_deployment_digest,
            self.receipt_digest,
        ])?;
        validate_artifact_identity(
            self.bridge_artifact_length,
            &self.bridge_artifact_sha256,
            &self.bridge_artifact_merkle_root,
            &self.bridge_artifact_scheme_id,
        )?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.bridge_observation_generation == 0
            || self.actual_target_capacity < self.bridge_artifact_length
            || self.actual_target_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.target_deployed_slot == 0
            || self.previous_gate_status != GateStatusV1::EmergencyFrozen
            || self.previous_gate_epoch == 0
            || self.previous_freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.previous_freeze_slot == 0
            || self.activated_gate_status != GateStatusV1::Active
            || self.activated_gate_epoch
                != self
                    .previous_gate_epoch
                    .checked_add(1)
                    .ok_or(GovernanceError::ArithmeticOverflow)?
            || self.target_nonce == 0
            || self.council_version == 0
            || self.council_hash == [0; 32]
            || self.deployment_generation == 0
            || self.finalized_slot == 0
            || !self.finalized
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(BootstrapActivationReceiptV1, 1024);

fn validate_header(
    actual_discriminator: &[u8; 8],
    expected_discriminator: &[u8; 8],
    version: u8,
    initialized: bool,
    reserved: &[u8],
) -> GovernanceResult<()> {
    if actual_discriminator != expected_discriminator {
        return Err(GovernanceError::InvalidDiscriminator);
    }
    if version != CEREMONY_ACCOUNT_VERSION_V1 {
        return Err(GovernanceError::UnsupportedVersion);
    }
    if !initialized {
        return Err(GovernanceError::Uninitialized);
    }
    if reserved.iter().any(|byte| *byte != 0) {
        return Err(GovernanceError::NonzeroReserved);
    }
    Ok(())
}

fn require_nondefault_keys(keys: &[Pubkey]) -> GovernanceResult<()> {
    if keys.iter().any(|key| *key == Pubkey::default()) {
        Err(GovernanceError::DefaultPubkey)
    } else {
        Ok(())
    }
}

fn require_nonzero_hashes(hashes: &[[u8; 32]]) -> GovernanceResult<()> {
    if hashes.contains(&[0; 32]) {
        Err(GovernanceError::InvalidRelease1Account)
    } else {
        Ok(())
    }
}

fn observation_geometry(raw_length: u64, chunk_size: u32) -> GovernanceResult<(u32, u32, u8)> {
    let chunk_size_is_admitted = chunk_size == PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB
        || cfg!(feature = "programdata-observation-chunk-matrix")
            && is_programdata_observation_chunk_size_candidate(chunk_size);
    if raw_length == 0
        || raw_length > MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
        || !chunk_size_is_admitted
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    let count = programdata_observation_chunk_count(raw_length, chunk_size)?;
    let padded = count
        .checked_next_power_of_two()
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let depth = u8::try_from(padded.trailing_zeros())
        .map_err(|_| GovernanceError::InvalidMerkleParameters)?;
    if count == 0
        || count > PROGRAMDATA_OBSERVATION_MAX_REAL_CHUNKS_V1
        || padded > PROGRAMDATA_OBSERVATION_MAX_PADDED_LEAVES_V1
        || depth > PROGRAMDATA_OBSERVATION_MAX_TREE_DEPTH_V1
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok((count, padded, depth))
}

const fn valid_frontier_mask() -> u16 {
    (1u16 << PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1) - 1
}

fn validate_frontier(
    frontier: &[[u8; 32]; PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1],
    mask: u16,
) -> GovernanceResult<()> {
    for (level, hash) in frontier.iter().enumerate() {
        let occupied = mask & (1u16 << level) != 0;
        if occupied == (*hash == [0; 32]) {
            return Err(GovernanceError::InvalidRelease1Account);
        }
    }
    Ok(())
}

fn validate_artifact_identity(
    length: u64,
    sha256: &[u8; 32],
    merkle_root: &[u8; 32],
    scheme_id: &[u8; 32],
) -> GovernanceResult<()> {
    if length == 0
        || length > MAX_ARTIFACT_BYTES_V1
        || *sha256 == [0; 32]
        || *merkle_root == [0; 32]
        || *scheme_id != ARTIFACT_MERKLE_SCHEME_ID
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    artifact_chunk_count(length, ARTIFACT_BINDING_CHUNK_SIZE_V1)?;
    Ok(())
}

fn optional_pubkey_matches_option(optional: &OptionalPubkeyV1, value: Option<Pubkey>) -> bool {
    match value {
        Some(value) => optional.present && optional.value == value,
        None => !optional.present && optional.value == Pubkey::default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_authority_change_observations(
    pre_observation: Pubkey,
    pre_generation: u64,
    pre_root: &[u8; 32],
    pre_digest: &[u8; 32],
    post_observation: Pubkey,
    post_generation: u64,
    post_root: &[u8; 32],
    post_digest: &[u8; 32],
) -> GovernanceResult<()> {
    if pre_observation == post_observation
        || pre_generation == 0
        || post_generation <= pre_generation
        || pre_root == post_root
        || pre_digest == post_digest
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn validate_handoff_common(value: &TargetAuthorityHandoffProposalV1) -> GovernanceResult<()> {
    require_nondefault_keys(&[
        value.controller_program,
        value.controller_programdata,
        value.controller_immutability_receipt,
        value.controller_config,
        value.governance_policy,
        value.capacity_policy,
        value.gate,
        value.controller_authority,
        value.target_program,
        value.target_programdata,
        value.upgradeable_loader,
        value.legacy_target_authority,
        value.bridge_observation,
    ])?;
    require_nonzero_hashes(&[
        value.cluster_domain,
        value.controller_immutability_digest,
        value.governance_policy_hash,
        value.capacity_policy_digest,
        value.bridge_source_commitment,
        value.bridge_build_inputs_commitment,
        value.bridge_package_commitment,
        value.bridge_release_manifest_commitment,
        value.bridge_observation_root,
        value.bridge_observation_digest,
        value.council_hash,
        value.proposal_digest,
    ])?;
    validate_artifact_identity(
        value.bridge_artifact_length,
        &value.bridge_artifact_sha256,
        &value.bridge_artifact_merkle_root,
        &value.bridge_artifact_scheme_id,
    )?;
    if value.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || value.legacy_target_authority == value.controller_authority
        || value.bridge_observation_generation == 0
        || value.minimum_target_deployed_slot == 0
        || value.minimum_target_capacity < value.bridge_artifact_length
        || value.minimum_target_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        || value.minimum_target_raw_length
            != value
                .minimum_target_capacity
                .checked_add(PROGRAMDATA_PAYLOAD_OFFSET_V1.into())
                .ok_or(GovernanceError::ArithmeticOverflow)?
        || value.bootstrap_gate_status != GateStatusV1::EmergencyFrozen
        || value.bootstrap_gate_epoch == 0
        || value.bootstrap_freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || value.bootstrap_freeze_slot == 0
        || value.target_nonce == 0
        || value.council_version == 0
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn validate_activation_common(value: &BootstrapActivationProposalV1) -> GovernanceResult<()> {
    require_nondefault_keys(&[
        value.controller_program,
        value.controller_programdata,
        value.controller_config,
        value.governance_policy,
        value.capacity_policy,
        value.controller_immutability_receipt,
        value.target_handoff_receipt,
        value.gate,
        value.target_program,
        value.target_programdata,
        value.upgradeable_loader,
        value.controller_authority,
        value.bridge_observation,
    ])?;
    require_nonzero_hashes(&[
        value.cluster_domain,
        value.governance_policy_hash,
        value.capacity_policy_digest,
        value.controller_immutability_digest,
        value.target_handoff_digest,
        value.bridge_source_commitment,
        value.bridge_build_inputs_commitment,
        value.bridge_package_commitment,
        value.bridge_release_manifest_commitment,
        value.bridge_observation_root,
        value.bridge_observation_digest,
        value.council_hash,
        value.proposal_digest,
    ])?;
    validate_artifact_identity(
        value.bridge_artifact_length,
        &value.bridge_artifact_sha256,
        &value.bridge_artifact_merkle_root,
        &value.bridge_artifact_scheme_id,
    )?;
    if value.upgradeable_loader != UPGRADEABLE_LOADER_ID
        || value.bridge_observation_generation == 0
        || value.minimum_target_deployed_slot == 0
        || value.minimum_target_capacity < value.bridge_artifact_length
        || value.minimum_target_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        || value.minimum_target_raw_length
            != value
                .minimum_target_capacity
                .checked_add(PROGRAMDATA_PAYLOAD_OFFSET_V1.into())
                .ok_or(GovernanceError::ArithmeticOverflow)?
        || value.bootstrap_gate_status != GateStatusV1::EmergencyFrozen
        || value.bootstrap_gate_epoch == 0
        || value.bootstrap_freeze_reason_code != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        || value.bootstrap_freeze_slot == 0
        || value.target_nonce == 0
        || value.council_version == 0
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_ceremony_proposal_lifecycle(
    state: CeremonyProposalStateV1,
    creation_slot: u64,
    review_start_slot: u64,
    review_end_slot: u64,
    not_before_slot: u64,
    expiry_slot: u64,
    approval_bitset: u8,
    approval_count: u8,
    approval_threshold: u8,
    first_approval_slot: u64,
    council_approved_slot: u64,
    queued_slot: u64,
    executed_slot: u64,
    terminal_slot: u64,
    terminal_reason_code: u16,
) -> GovernanceResult<()> {
    if creation_slot == 0
        || creation_slot > review_start_slot
        || review_start_slot >= review_end_slot
        || review_end_slot > not_before_slot
        || not_before_slot >= expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    validate_approval_pair(approval_bitset, approval_count)?;
    if approval_threshold != RELEASE1_APPROVAL_THRESHOLD
        || approval_count > approval_threshold
        || (approval_count == 0) != (first_approval_slot == 0)
        || (approval_count == approval_threshold) != (council_approved_slot != 0)
        || (first_approval_slot != 0
            && (first_approval_slot < review_start_slot
                || first_approval_slot > review_end_slot
                || first_approval_slot >= expiry_slot))
        || (council_approved_slot != 0
            && (council_approved_slot < first_approval_slot
                || council_approved_slot > review_end_slot
                || council_approved_slot >= expiry_slot))
        || (queued_slot != 0
            && (approval_count != approval_threshold
                || queued_slot < council_approved_slot
                || queued_slot >= expiry_slot))
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    match state {
        CeremonyProposalStateV1::Draft => {
            if approval_count == approval_threshold
                || council_approved_slot != 0
                || queued_slot != 0
                || executed_slot != 0
                || terminal_slot != 0
                || terminal_reason_code != 0
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        CeremonyProposalStateV1::CouncilApproved => {
            if approval_count != approval_threshold
                || queued_slot != 0
                || executed_slot != 0
                || terminal_slot != 0
                || terminal_reason_code != 0
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        CeremonyProposalStateV1::Timelocked => {
            if approval_count != approval_threshold
                || queued_slot == 0
                || executed_slot != 0
                || terminal_slot != 0
                || terminal_reason_code != 0
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        CeremonyProposalStateV1::Completed => {
            if approval_count != approval_threshold
                || queued_slot == 0
                || executed_slot < not_before_slot
                || executed_slot >= expiry_slot
                || terminal_slot != executed_slot
                || terminal_reason_code != CEREMONY_PROPOSAL_COMPLETED_REASON_V1
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        CeremonyProposalStateV1::Expired => {
            if executed_slot != 0
                || terminal_slot < expiry_slot
                || terminal_reason_code != CEREMONY_PROPOSAL_EXPIRED_REASON_V1
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
    }
    Ok(())
}

fn validate_approval_pair(bitset: u8, count: u8) -> GovernanceResult<()> {
    if bitset & !VALID_APPROVAL_MASK != 0 {
        return Err(GovernanceError::InvalidApprovalBitset);
    }
    if bitset.count_ones() as u8 != count {
        return Err(GovernanceError::ApprovalCountMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::PathBuf};

    use serde_json::{json, Value};
    use solana_program::hash::hash;

    use super::*;
    use crate::{
        pda::{derive_bootstrap_activation_pda, derive_target_handoff_pda},
        programdata_observation_merkle::{
            PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
            PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1,
            PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_MATERIAL_V1,
        },
        release1_ceremony_digest::{
            compute_bootstrap_activation_deployment_plan_digest_v1,
            compute_bootstrap_activation_proposal_digest_v1,
            compute_bootstrap_activation_receipt_digest_v1,
            compute_bootstrap_activation_receipt_plan_digest_v1, compute_capacity_policy_digest_v1,
            compute_controller_immutability_receipt_digest_v1,
            compute_controller_release_digest_v1, compute_current_deployment_digest_v1,
            compute_programdata_observation_digest_v1,
            compute_programdata_observation_subject_digest_v1,
            compute_target_handoff_proposal_digest_v1, compute_target_handoff_receipt_digest_v1,
        },
    };

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn digest(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn some(byte: u8) -> OptionalPubkeyV1 {
        OptionalPubkeyV1::some(key(byte)).unwrap()
    }

    fn program_header(programdata: Pubkey) -> [u8; LOADER_PROGRAM_ACCOUNT_LEN] {
        let mut out = [0u8; LOADER_PROGRAM_ACCOUNT_LEN];
        out[..4].copy_from_slice(&2u32.to_le_bytes());
        out[4..].copy_from_slice(programdata.as_ref());
        out
    }

    fn programdata_header(
        deployed_slot: u64,
        authority: OptionalPubkeyV1,
    ) -> [u8; LOADER_PROGRAMDATA_METADATA_LEN] {
        let mut out = [0u8; LOADER_PROGRAMDATA_METADATA_LEN];
        out[..4].copy_from_slice(&3u32.to_le_bytes());
        out[4..12].copy_from_slice(&deployed_slot.to_le_bytes());
        if authority.present {
            out[12] = 1;
            out[13..45].copy_from_slice(authority.value.as_ref());
        }
        out
    }

    fn capacity_policy() -> ProgramDataCapacityPolicyV1 {
        ProgramDataCapacityPolicyV1 {
            discriminator: PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            controller_program: key(1),
            controller_config: key(2),
            target_program: key(3),
            target_programdata: key(4),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            loader_programdata_metadata_len: LOADER_PROGRAMDATA_METADATA_LEN as u64,
            maximum_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
            maximum_payload_capacity: MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
            maximum_artifact_length: MAX_ARTIFACT_BYTES_V1,
            observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            observation_chunk_size: PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
            observation_max_chunk_count: PROGRAMDATA_OBSERVATION_MAX_REAL_CHUNKS_V1,
            observation_padded_leaf_count: PROGRAMDATA_OBSERVATION_MAX_PADDED_LEAVES_V1,
            observation_tree_depth: PROGRAMDATA_OBSERVATION_MAX_TREE_DEPTH_V1,
            observation_frontier_hash_count: PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1 as u8,
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
            zero_tail_required: true,
            extend_program_checked_feature: EXTEND_PROGRAM_CHECKED_FEATURE_ID_V1,
            set_authority_checked_feature: SET_AUTHORITY_CHECKED_FEATURE_ID_V1,
            policy_digest: digest(7),
            creation_slot: 8,
            reserved: [0; PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN],
        }
    }

    fn release_commitment() -> ControllerReleaseCommitmentV1 {
        ControllerReleaseCommitmentV1 {
            discriminator: CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            controller_program: key(1),
            controller_programdata: key(2),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            capacity_policy: key(3),
            capacity_policy_digest: digest(4),
            artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            artifact_sha256: digest(5),
            artifact_merkle_root: digest(6),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            source_commitment: digest(7),
            source_tree_commitment: digest(8),
            build_inputs_commitment: digest(9),
            toolchain_commitment: digest(10),
            package_commitment: digest(11),
            release_manifest_commitment: digest(12),
            abi_commitment: digest(13),
            pre_immutability_authority: some(14),
            minimum_programdata_capacity: 32_768,
            release_digest: digest(15),
            creation_slot: 16,
            finalized: true,
            reserved: [0; CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN],
        }
    }

    fn observation_with(
        purpose: ProgramDataObservationPurposeV1,
        authority: OptionalPubkeyV1,
    ) -> ProgramDataObservationV1 {
        let target_programdata = key(11);
        let mut frontier = [[0u8; 32]; PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1];
        // Three real chunks have the canonical streaming-frontier mask 0b11.
        frontier[0] = digest(40);
        frontier[1] = digest(41);
        ProgramDataObservationV1 {
            discriminator: PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            controller_program: key(1),
            controller_config: key(2),
            capacity_policy: key(3),
            capacity_policy_digest: digest(4),
            purpose,
            subject: key(5),
            subject_digest: digest(6),
            generation: 1,
            protocol_gate: key(8),
            gate_status: GateStatusV1::FrozenForUpgrade,
            gate_epoch: 2,
            gate_active_proposal: key(5),
            gate_freeze_slot: 6,
            gate_freeze_reason_code: 7,
            target_program: key(10),
            target_programdata,
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            program_owner: UPGRADEABLE_LOADER_ID,
            program_executable: true,
            program_data_length: LOADER_PROGRAM_ACCOUNT_LEN as u64,
            program_header_present: true,
            program_header_snapshot: program_header(target_programdata),
            linked_programdata: target_programdata,
            programdata_owner: UPGRADEABLE_LOADER_ID,
            programdata_executable: false,
            programdata_header_present: true,
            programdata_header_snapshot: programdata_header(7, authority),
            deployed_slot: 7,
            upgrade_authority: authority,
            raw_data_length: 32_768 + u64::from(PROGRAMDATA_PAYLOAD_OFFSET_V1),
            payload_offset: PROGRAMDATA_PAYLOAD_OFFSET_V1,
            actual_capacity: 32_768,
            expected_artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            expected_artifact_sha256: digest(12),
            expected_artifact_merkle_root: digest(13),
            expected_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            artifact_chunk_size: ARTIFACT_BINDING_CHUNK_SIZE_V1,
            artifact_chunk_count: 1,
            minimum_required_capacity: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            raw_observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            raw_chunk_size: PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB,
            raw_chunk_count: 3,
            raw_padded_leaf_count: 4,
            raw_tree_depth: 2,
            next_raw_chunk_index: 3,
            raw_frontier: frontier,
            raw_frontier_mask: 3,
            next_artifact_chunk_index: 1,
            tail_bytes_verified: 16_384,
            start_slot: 8,
            last_observed_slot: 9,
            finalized_slot: 10,
            final_raw_merkle_root: digest(14),
            observation_digest: digest(15),
            status: ProgramDataObservationStatusV1::Finalized,
            reserved: [0; PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN],
        }
    }

    fn current_deployment() -> CurrentDeploymentStateV1 {
        CurrentDeploymentStateV1 {
            discriminator: CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            controller_program: key(1),
            controller_config: key(2),
            capacity_policy: key(3),
            capacity_policy_digest: digest(4),
            target_program: key(5),
            target_programdata: key(6),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_authority: key(7),
            artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            artifact_sha256: digest(8),
            artifact_merkle_root: digest(9),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            actual_programdata_capacity: 32_768,
            programdata_observation: key(10),
            observation_generation: 1,
            observation_root: digest(11),
            observation_digest: digest(12),
            deployed_slot: 13,
            installed_authority: key(7),
            source_commitment: digest(14),
            build_inputs_commitment: digest(15),
            package_commitment: digest(16),
            release_manifest_commitment: digest(17),
            release_commitment: key(18),
            release_commitment_digest: digest(19),
            activation_receipt: some(20),
            completed_proposal: OptionalPubkeyV1::none(),
            gate_epoch_at_activation: 2,
            deployment_generation: 1,
            deployment_digest: digest(21),
            last_updated_slot: 22,
            reserved: [0; CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN],
        }
    }

    fn immutability_receipt() -> ControllerImmutabilityReceiptV1 {
        ControllerImmutabilityReceiptV1 {
            discriminator: CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            controller_program: key(1),
            controller_programdata: key(2),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            capacity_policy: key(3),
            capacity_policy_digest: digest(4),
            release_commitment: key(5),
            release_commitment_digest: digest(6),
            pre_observation: key(7),
            pre_observation_generation: 1,
            pre_observation_root: digest(8),
            pre_observation_digest: digest(9),
            pre_upgrade_authority: some(10),
            post_observation: key(11),
            post_observation_generation: 2,
            post_observation_root: digest(12),
            post_observation_digest: digest(13),
            post_upgrade_authority: OptionalPubkeyV1::none(),
            deployed_slot: 14,
            raw_programdata_length: 32_768 + u64::from(PROGRAMDATA_PAYLOAD_OFFSET_V1),
            programdata_capacity: 32_768,
            artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            artifact_sha256: digest(15),
            artifact_merkle_root: digest(16),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            source_commitment: digest(17),
            build_inputs_commitment: digest(18),
            package_commitment: digest(19),
            release_manifest_commitment: digest(20),
            finalized_slot: 21,
            receipt_digest: digest(22),
            finalized: true,
            reserved: [0; CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN],
        }
    }

    fn handoff_proposal() -> TargetAuthorityHandoffProposalV1 {
        TargetAuthorityHandoffProposalV1 {
            discriminator: TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            state: CeremonyProposalStateV1::Draft,
            cluster_domain: digest(1),
            controller_program: key(2),
            controller_programdata: key(3),
            controller_immutability_receipt: key(4),
            controller_immutability_digest: digest(5),
            controller_config: key(6),
            governance_policy: key(7),
            governance_policy_hash: digest(8),
            capacity_policy: key(9),
            capacity_policy_digest: digest(10),
            gate: key(11),
            controller_authority: key(12),
            target_program: key(13),
            target_programdata: key(14),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            legacy_target_authority: key(15),
            bridge_artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            bridge_artifact_sha256: digest(16),
            bridge_artifact_merkle_root: digest(17),
            bridge_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            bridge_source_commitment: digest(18),
            bridge_build_inputs_commitment: digest(19),
            bridge_package_commitment: digest(20),
            bridge_release_manifest_commitment: digest(21),
            bridge_observation: key(22),
            bridge_observation_generation: 1,
            bridge_observation_root: digest(23),
            bridge_observation_digest: digest(24),
            minimum_target_deployed_slot: 25,
            minimum_target_capacity: 32_768,
            minimum_target_raw_length: 32_768 + u64::from(PROGRAMDATA_PAYLOAD_OFFSET_V1),
            bootstrap_gate_status: GateStatusV1::EmergencyFrozen,
            bootstrap_gate_epoch: 1,
            bootstrap_freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
            bootstrap_freeze_slot: 26,
            target_nonce: 1,
            council_version: 1,
            council_hash: digest(27),
            review_start_slot: 31,
            review_end_slot: 32,
            not_before_slot: 33,
            expiry_slot: 40,
            approval_bitset: 0,
            approval_count: 0,
            approval_threshold: RELEASE1_APPROVAL_THRESHOLD,
            first_approval_slot: 0,
            council_approved_slot: 0,
            queued_slot: 0,
            executed_slot: 0,
            terminal_slot: 0,
            terminal_reason_code: 0,
            proposal_digest: digest(28),
            creation_slot: 30,
            reserved: [0; TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN],
        }
    }

    fn handoff_receipt() -> TargetAuthorityHandoffReceiptV1 {
        TargetAuthorityHandoffReceiptV1 {
            discriminator: TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            proposal: key(1),
            proposal_digest: digest(2),
            controller_program: key(3),
            controller_config: key(4),
            controller_authority: key(5),
            controller_immutability_receipt: key(6),
            controller_immutability_digest: digest(7),
            target_program: key(8),
            target_programdata: key(9),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            pre_observation: key(10),
            pre_observation_generation: 1,
            pre_observation_root: digest(11),
            pre_observation_digest: digest(12),
            pre_upgrade_authority: some(13),
            pre_programdata_header_snapshot: programdata_header(17, some(13)),
            post_programdata_header_snapshot: programdata_header(17, some(5)),
            post_upgrade_authority: some(5),
            deployed_slot: 17,
            raw_programdata_length: 32_768 + u64::from(PROGRAMDATA_PAYLOAD_OFFSET_V1),
            programdata_capacity: 32_768,
            artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            artifact_sha256: digest(18),
            artifact_merkle_root: digest(19),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            bridge_source_commitment: digest(20),
            bridge_build_inputs_commitment: digest(21),
            bridge_package_commitment: digest(22),
            bridge_release_manifest_commitment: digest(23),
            bootstrap_gate_epoch: 1,
            target_nonce: 1,
            council_version: 1,
            accepted_slot: 24,
            receipt_digest: digest(25),
            finalized: true,
            reserved: [0; TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN],
        }
    }

    fn activation_proposal() -> BootstrapActivationProposalV1 {
        BootstrapActivationProposalV1 {
            discriminator: BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            state: CeremonyProposalStateV1::Draft,
            cluster_domain: digest(1),
            controller_program: key(2),
            controller_programdata: key(3),
            controller_config: key(4),
            governance_policy: key(5),
            governance_policy_hash: digest(6),
            capacity_policy: key(7),
            capacity_policy_digest: digest(8),
            controller_immutability_receipt: key(9),
            controller_immutability_digest: digest(10),
            target_handoff_receipt: key(11),
            target_handoff_digest: digest(12),
            gate: key(13),
            target_program: key(14),
            target_programdata: key(15),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_authority: key(16),
            bridge_artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            bridge_artifact_sha256: digest(17),
            bridge_artifact_merkle_root: digest(18),
            bridge_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            bridge_source_commitment: digest(19),
            bridge_build_inputs_commitment: digest(20),
            bridge_package_commitment: digest(21),
            bridge_release_manifest_commitment: digest(22),
            bridge_observation: key(23),
            bridge_observation_generation: 1,
            bridge_observation_root: digest(24),
            bridge_observation_digest: digest(25),
            minimum_target_deployed_slot: 26,
            minimum_target_capacity: 32_768,
            minimum_target_raw_length: 32_768 + u64::from(PROGRAMDATA_PAYLOAD_OFFSET_V1),
            bootstrap_gate_status: GateStatusV1::EmergencyFrozen,
            bootstrap_gate_epoch: 1,
            bootstrap_freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
            bootstrap_freeze_slot: 27,
            target_nonce: 1,
            council_version: 1,
            council_hash: digest(28),
            review_start_slot: 31,
            review_end_slot: 32,
            not_before_slot: 33,
            expiry_slot: 40,
            approval_bitset: 0,
            approval_count: 0,
            approval_threshold: RELEASE1_APPROVAL_THRESHOLD,
            first_approval_slot: 0,
            council_approved_slot: 0,
            queued_slot: 0,
            executed_slot: 0,
            terminal_slot: 0,
            terminal_reason_code: 0,
            proposal_digest: digest(29),
            creation_slot: 30,
            reserved: [0; BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN],
        }
    }

    fn activation_receipt() -> BootstrapActivationReceiptV1 {
        BootstrapActivationReceiptV1 {
            discriminator: BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
            version: CEREMONY_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            proposal: key(1),
            proposal_digest: digest(2),
            controller_program: key(3),
            controller_config: key(4),
            governance_policy: key(5),
            governance_policy_hash: digest(6),
            capacity_policy: key(7),
            capacity_policy_digest: digest(8),
            controller_immutability_receipt: key(9),
            controller_immutability_digest: digest(10),
            target_handoff_receipt: key(11),
            target_handoff_digest: digest(12),
            gate: key(13),
            target_program: key(14),
            target_programdata: key(15),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_authority: key(16),
            bridge_observation: key(17),
            bridge_observation_generation: 1,
            bridge_observation_root: digest(18),
            bridge_observation_digest: digest(19),
            bridge_artifact_length: u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1),
            bridge_artifact_sha256: digest(20),
            bridge_artifact_merkle_root: digest(21),
            bridge_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            actual_target_capacity: 32_768,
            target_deployed_slot: 22,
            previous_gate_status: GateStatusV1::EmergencyFrozen,
            previous_gate_epoch: 1,
            previous_freeze_reason_code: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
            previous_freeze_slot: 23,
            activated_gate_status: GateStatusV1::Active,
            activated_gate_epoch: 2,
            target_nonce: 1,
            council_version: 1,
            council_hash: digest(24),
            current_deployment_state: key(25),
            current_deployment_digest: digest(26),
            deployment_generation: 1,
            finalized_slot: 27,
            receipt_digest: digest(28),
            finalized: true,
            reserved: [0; BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN],
        }
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let first = chunk[0];
            let second = chunk.get(1).copied().unwrap_or(0);
            let third = chunk.get(2).copied().unwrap_or(0);
            encoded.push(ALPHABET[usize::from(first >> 2)] as char);
            encoded.push(ALPHABET[usize::from(((first & 0x03) << 4) | (second >> 4))] as char);
            encoded.push(if chunk.len() > 1 {
                ALPHABET[usize::from(((second & 0x0f) << 2) | (third >> 6))] as char
            } else {
                '='
            });
            encoded.push(if chunk.len() > 2 {
                ALPHABET[usize::from(third & 0x3f)] as char
            } else {
                '='
            });
        }
        encoded
    }

    fn encode_hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            encoded.push(DIGITS[usize::from(byte >> 4)] as char);
            encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
        }
        encoded
    }

    fn fixture_entry<T: BorshSerialize>(
        account_type: &str,
        length: usize,
        discriminator: [u8; 8],
        reserved_length: usize,
        digest: [u8; 32],
        value: &T,
    ) -> Value {
        let bytes = value.try_to_vec().expect("fixture account must encode");
        assert_eq!(bytes.len(), length, "{account_type} fixture length drifted");
        assert_eq!(&bytes[..8], &discriminator);
        assert!(bytes[length - reserved_length..]
            .iter()
            .all(|byte| *byte == 0));
        json!({
            "account_type": account_type,
            "length": length,
            "discriminator_ascii": std::str::from_utf8(&discriminator)
                .expect("fixture discriminator must be ASCII"),
            "reserved_length": reserved_length,
            "digest_hex": encode_hex(&digest),
            "data_base64": encode_base64(&bytes),
        })
    }

    /// Rust-authoritative regeneration entrypoint for the shared ceremony ABI
    /// fixture. Run the exact command documented in `fixtures/README.md`.
    #[test]
    #[ignore = "writes the tracked cross-language ceremony fixture"]
    fn regenerate_release1_ceremony_fixture() {
        let controller_program = key(1);
        let target_program = key(3);
        let council_version = 1;
        let target_handoff =
            derive_target_handoff_pda(&controller_program, &target_program, council_version);
        let bootstrap_activation =
            derive_bootstrap_activation_pda(&controller_program, &target_program, council_version);

        let mut capacity = capacity_policy();
        capacity.policy_digest =
            compute_capacity_policy_digest_v1(&capacity).expect("capacity digest must compute");

        let mut release = release_commitment();
        release.release_digest =
            compute_controller_release_digest_v1(&release).expect("release digest must compute");

        let mut observation =
            observation_with(ProgramDataObservationPurposeV1::ProposalPrestate, some(30));
        observation.subject_digest = compute_programdata_observation_subject_digest_v1(
            &observation.controller_program,
            &observation.controller_config,
            &observation.target_program,
            &observation.target_programdata,
            observation.purpose,
            &observation.subject,
            observation.generation,
            &observation.protocol_gate,
            observation.gate_status,
            observation.gate_epoch,
            &observation.gate_active_proposal,
            observation.gate_freeze_slot,
            observation.gate_freeze_reason_code,
            &observation.capacity_policy_digest,
            observation.expected_artifact_length,
            &observation.expected_artifact_sha256,
            &observation.expected_artifact_merkle_root,
            &observation.expected_artifact_scheme_id,
            observation.minimum_required_capacity,
        )
        .expect("observation subject digest must compute");
        observation.observation_digest = compute_programdata_observation_digest_v1(&observation)
            .expect("observation digest must compute");

        let mut deployment = current_deployment();
        deployment.deployment_digest = compute_current_deployment_digest_v1(&deployment)
            .expect("deployment digest must compute");

        let mut immutability = immutability_receipt();
        immutability.receipt_digest =
            compute_controller_immutability_receipt_digest_v1(&immutability)
                .expect("immutability receipt digest must compute");

        let mut handoff = handoff_proposal();
        handoff.controller_program = controller_program;
        handoff.target_program = target_program;
        handoff.bump = target_handoff.1;
        handoff.proposal_digest = compute_target_handoff_proposal_digest_v1(&handoff)
            .expect("handoff proposal digest must compute");

        let mut handoff_receipt = handoff_receipt();
        handoff_receipt.proposal = target_handoff.0;
        handoff_receipt.proposal_digest = handoff.proposal_digest;
        handoff_receipt.controller_program = controller_program;
        handoff_receipt.target_program = target_program;
        handoff_receipt.receipt_digest = compute_target_handoff_receipt_digest_v1(&handoff_receipt)
            .expect("handoff receipt digest must compute");

        let mut activation = activation_proposal();
        activation.controller_program = controller_program;
        activation.target_program = target_program;
        activation.bump = bootstrap_activation.1;
        activation.proposal_digest = compute_bootstrap_activation_proposal_digest_v1(&activation)
            .expect("activation proposal digest must compute");

        let mut activation_receipt = activation_receipt();
        activation_receipt.proposal = bootstrap_activation.0;
        activation_receipt.proposal_digest = activation.proposal_digest;
        activation_receipt.controller_program = controller_program;
        activation_receipt.target_program = target_program;
        activation_receipt.receipt_digest =
            compute_bootstrap_activation_receipt_digest_v1(&activation_receipt)
                .expect("activation receipt digest must compute");

        let entries = vec![
            fixture_entry(
                "ProgramDataCapacityPolicyV1",
                ProgramDataCapacityPolicyV1::LEN,
                PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
                PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN,
                capacity.policy_digest,
                &capacity,
            ),
            fixture_entry(
                "ControllerReleaseCommitmentV1",
                ControllerReleaseCommitmentV1::LEN,
                CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
                CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN,
                release.release_digest,
                &release,
            ),
            fixture_entry(
                "ProgramDataObservationV1",
                ProgramDataObservationV1::LEN,
                PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
                PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN,
                observation.observation_digest,
                &observation,
            ),
            fixture_entry(
                "CurrentDeploymentStateV1",
                CurrentDeploymentStateV1::LEN,
                CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
                CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN,
                deployment.deployment_digest,
                &deployment,
            ),
            fixture_entry(
                "ControllerImmutabilityReceiptV1",
                ControllerImmutabilityReceiptV1::LEN,
                CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
                CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN,
                immutability.receipt_digest,
                &immutability,
            ),
            fixture_entry(
                "TargetAuthorityHandoffProposalV1",
                TargetAuthorityHandoffProposalV1::LEN,
                TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
                TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN,
                handoff.proposal_digest,
                &handoff,
            ),
            fixture_entry(
                "TargetAuthorityHandoffReceiptV1",
                TargetAuthorityHandoffReceiptV1::LEN,
                TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
                TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN,
                handoff_receipt.receipt_digest,
                &handoff_receipt,
            ),
            fixture_entry(
                "BootstrapActivationProposalV1",
                BootstrapActivationProposalV1::LEN,
                BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
                BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN,
                activation.proposal_digest,
                &activation,
            ),
            fixture_entry(
                "BootstrapActivationReceiptV1",
                BootstrapActivationReceiptV1::LEN,
                BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
                BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN,
                activation_receipt.receipt_digest,
                &activation_receipt,
            ),
        ];
        let fixture = json!({
            "fixture_version": 1,
            "activation_plan_digests": {
                "deployment_plan_digest_hex": encode_hex(
                    &compute_bootstrap_activation_deployment_plan_digest_v1(&deployment)
                        .expect("activation deployment plan digest must compute")
                ),
                "receipt_plan_digest_hex": encode_hex(
                    &compute_bootstrap_activation_receipt_plan_digest_v1(&activation_receipt)
                        .expect("activation receipt plan digest must compute")
                ),
            },
            "proposal_pdas": {
                "controller_program": controller_program.to_string(),
                "target_program": target_program.to_string(),
                "council_version": council_version,
                "target_authority_handoff_proposal": {
                    "address": target_handoff.0.to_string(),
                    "bump": target_handoff.1,
                },
                "bootstrap_activation_proposal": {
                    "address": bootstrap_activation.0.to_string(),
                    "bump": bootstrap_activation.1,
                },
            },
            "entries": entries,
        });
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/release1_ceremony_accounts_v1.json");
        fs::write(
            fixture_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&fixture).expect("fixture JSON must encode")
            ),
        )
        .expect("shared fixture must be writable");
    }

    #[test]
    fn activation_plan_digests_ignore_only_slot_derived_fields() {
        let mut deployment = current_deployment();
        deployment.deployment_digest = compute_current_deployment_digest_v1(&deployment).unwrap();
        let deployment_plan =
            compute_bootstrap_activation_deployment_plan_digest_v1(&deployment).unwrap();
        let deployment_digest = deployment.deployment_digest;
        deployment.last_updated_slot += 17;
        deployment.deployment_digest = compute_current_deployment_digest_v1(&deployment).unwrap();
        assert_eq!(
            compute_bootstrap_activation_deployment_plan_digest_v1(&deployment).unwrap(),
            deployment_plan
        );
        assert_ne!(deployment.deployment_digest, deployment_digest);
        deployment.artifact_sha256[0] ^= 1;
        assert_ne!(
            compute_bootstrap_activation_deployment_plan_digest_v1(&deployment).unwrap(),
            deployment_plan
        );

        let mut receipt = activation_receipt();
        receipt.receipt_digest = compute_bootstrap_activation_receipt_digest_v1(&receipt).unwrap();
        let receipt_plan = compute_bootstrap_activation_receipt_plan_digest_v1(&receipt).unwrap();
        let receipt_digest = receipt.receipt_digest;
        receipt.finalized_slot += 17;
        receipt.current_deployment_digest = digest(97);
        receipt.receipt_digest = compute_bootstrap_activation_receipt_digest_v1(&receipt).unwrap();
        assert_eq!(
            compute_bootstrap_activation_receipt_plan_digest_v1(&receipt).unwrap(),
            receipt_plan
        );
        assert_ne!(receipt.receipt_digest, receipt_digest);
        receipt.bridge_artifact_sha256[0] ^= 1;
        assert_ne!(
            compute_bootstrap_activation_receipt_plan_digest_v1(&receipt).unwrap(),
            receipt_plan
        );
    }

    macro_rules! assert_strict_account {
        ($value:expr, $ty:ty) => {{
            let value = $value;
            value.validate_static().unwrap();
            let encoded = value.try_to_vec().unwrap();
            assert_eq!(encoded.len(), <$ty>::LEN);
            assert_eq!(<$ty>::from_bytes_strict(&encoded).unwrap(), value);
            assert!(<$ty>::from_bytes_strict(&encoded[..encoded.len() - 1]).is_err());
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(<$ty>::from_bytes_strict(&trailing).is_err());
            assert!(<$ty>::try_from_slice(&trailing).is_err());
        }};
    }

    #[test]
    fn exact_lengths_round_trip_and_strict_decoders_reject_size_drift() {
        assert_strict_account!(capacity_policy(), ProgramDataCapacityPolicyV1);
        assert_strict_account!(release_commitment(), ControllerReleaseCommitmentV1);
        assert_strict_account!(
            observation_with(ProgramDataObservationPurposeV1::ProposalPrestate, some(30)),
            ProgramDataObservationV1
        );
        assert_strict_account!(current_deployment(), CurrentDeploymentStateV1);
        assert_strict_account!(immutability_receipt(), ControllerImmutabilityReceiptV1);
        assert_strict_account!(handoff_proposal(), TargetAuthorityHandoffProposalV1);
        assert_strict_account!(handoff_receipt(), TargetAuthorityHandoffReceiptV1);
        assert_strict_account!(activation_proposal(), BootstrapActivationProposalV1);
        assert_strict_account!(activation_receipt(), BootstrapActivationReceiptV1);
    }

    #[test]
    fn discriminators_are_unique_and_scheme_id_matches_its_material() {
        let discriminators = [
            PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
            CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
            PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
            CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
            CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
            TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
            TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
            BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
            BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        ];
        assert_eq!(
            discriminators.iter().collect::<BTreeSet<_>>().len(),
            discriminators.len()
        );
        assert_eq!(
            hash(PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_MATERIAL_V1).to_bytes(),
            PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
        );
    }

    #[test]
    fn enums_are_one_byte_strict_and_reject_unknown_truncated_or_trailing_data() {
        for (index, purpose) in [
            ProgramDataObservationPurposeV1::ControllerImmutability,
            ProgramDataObservationPurposeV1::TargetHandoffBridge,
            ProgramDataObservationPurposeV1::ProposalPrestate,
            ProgramDataObservationPurposeV1::PostUpgrade,
            ProgramDataObservationPurposeV1::Rollback,
            ProgramDataObservationPurposeV1::EmergencyResolution,
            ProgramDataObservationPurposeV1::BootstrapActivation,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(purpose.try_to_vec().unwrap(), [index as u8]);
            assert_eq!(
                ProgramDataObservationPurposeV1::try_from_slice(&[index as u8]).unwrap(),
                purpose
            );
        }
        assert!(ProgramDataObservationPurposeV1::try_from_slice(&[]).is_err());
        assert!(ProgramDataObservationPurposeV1::try_from_slice(&[7]).is_err());
        assert!(ProgramDataObservationPurposeV1::try_from_slice(&[0, 0]).is_err());

        for (index, status) in [
            ProgramDataObservationStatusV1::Accumulating,
            ProgramDataObservationStatusV1::ReadyToFinalize,
            ProgramDataObservationStatusV1::Finalized,
            ProgramDataObservationStatusV1::Stale,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(status.try_to_vec().unwrap(), [index as u8]);
        }
        assert!(ProgramDataObservationStatusV1::try_from_slice(&[]).is_err());
        assert!(ProgramDataObservationStatusV1::try_from_slice(&[4]).is_err());
        assert!(ProgramDataObservationStatusV1::try_from_slice(&[0, 0]).is_err());

        for (index, state) in [
            CeremonyProposalStateV1::Draft,
            CeremonyProposalStateV1::CouncilApproved,
            CeremonyProposalStateV1::Timelocked,
            CeremonyProposalStateV1::Completed,
            CeremonyProposalStateV1::Expired,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(state.try_to_vec().unwrap(), [index as u8]);
        }
        assert!(CeremonyProposalStateV1::try_from_slice(&[]).is_err());
        assert!(CeremonyProposalStateV1::try_from_slice(&[5]).is_err());
        assert!(CeremonyProposalStateV1::try_from_slice(&[0, 0]).is_err());
    }

    macro_rules! assert_nonzero_reserved_rejected {
        ($value:expr) => {{
            let mut value = $value;
            value.reserved[0] = 1;
            assert_eq!(
                value.validate_static(),
                Err(GovernanceError::NonzeroReserved)
            );
        }};
    }

    #[test]
    fn every_account_rejects_nonzero_reserved_bytes() {
        assert_nonzero_reserved_rejected!(capacity_policy());
        assert_nonzero_reserved_rejected!(release_commitment());
        assert_nonzero_reserved_rejected!(observation_with(
            ProgramDataObservationPurposeV1::ProposalPrestate,
            some(30)
        ));
        assert_nonzero_reserved_rejected!(current_deployment());
        assert_nonzero_reserved_rejected!(immutability_receipt());
        assert_nonzero_reserved_rejected!(handoff_proposal());
        assert_nonzero_reserved_rejected!(handoff_receipt());
        assert_nonzero_reserved_rejected!(activation_proposal());
        assert_nonzero_reserved_rejected!(activation_receipt());
    }

    #[test]
    fn all_five_seat_masks_have_exact_equal_seat_quorum_behavior() {
        for mask in 0u8..32 {
            let count = mask.count_ones() as u8;
            let mut proposal = handoff_proposal();
            proposal.approval_bitset = mask;
            proposal.approval_count = count;
            if count != 0 {
                proposal.first_approval_slot = proposal.review_start_slot;
            }
            if count == RELEASE1_APPROVAL_THRESHOLD {
                proposal.state = CeremonyProposalStateV1::CouncilApproved;
                proposal.council_approved_slot = proposal.review_end_slot;
            }
            if count > RELEASE1_APPROVAL_THRESHOLD {
                proposal.state = CeremonyProposalStateV1::CouncilApproved;
                proposal.council_approved_slot = proposal.review_end_slot;
            }
            assert_eq!(
                proposal.validate_static().is_ok(),
                count <= RELEASE1_APPROVAL_THRESHOLD,
                "mask {mask:05b}"
            );
        }

        let mut unknown = handoff_proposal();
        unknown.approval_bitset = 0b0010_0000;
        unknown.approval_count = 1;
        unknown.first_approval_slot = unknown.review_start_slot;
        assert_eq!(
            unknown.validate_static(),
            Err(GovernanceError::InvalidApprovalBitset)
        );

        let mut mismatch = handoff_proposal();
        mismatch.approval_bitset = 0b0000_0011;
        mismatch.approval_count = 1;
        mismatch.first_approval_slot = mismatch.review_start_slot;
        assert_eq!(
            mismatch.validate_static(),
            Err(GovernanceError::ApprovalCountMismatch)
        );
    }

    #[test]
    fn authority_receipts_require_distinct_observations_and_exact_transition_shape() {
        let mut immutable = immutability_receipt();
        immutable.validate_static().unwrap();
        immutable.post_observation_root = immutable.pre_observation_root;
        assert_eq!(
            immutable.validate_static(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut immutable = immutability_receipt();
        immutable.post_upgrade_authority = some(31);
        assert_eq!(
            immutable.validate_static(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut handoff = handoff_receipt();
        handoff.validate_static().unwrap();
        handoff.pre_upgrade_authority = some(5);
        assert_eq!(
            handoff.validate_static(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut handoff = handoff_receipt();
        handoff.post_upgrade_authority = some(31);
        assert_eq!(
            handoff.validate_static(),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn maximum_runtime_capacity_is_representable_and_one_byte_more_is_rejected() {
        capacity_policy().validate_static().unwrap();

        let mut observation =
            observation_with(ProgramDataObservationPurposeV1::ProposalPrestate, some(30));
        observation.raw_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
        observation.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1;
        observation.minimum_required_capacity = u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1);
        observation.raw_chunk_count = PROGRAMDATA_OBSERVATION_MAX_REAL_CHUNKS_V1;
        observation.raw_padded_leaf_count = PROGRAMDATA_OBSERVATION_MAX_PADDED_LEAVES_V1;
        observation.raw_tree_depth = PROGRAMDATA_OBSERVATION_MAX_TREE_DEPTH_V1;
        observation.next_raw_chunk_index = PROGRAMDATA_OBSERVATION_MAX_REAL_CHUNKS_V1;
        observation.raw_frontier = [[0u8; 32]; PROGRAMDATA_OBSERVATION_FRONTIER_HASHES_V1];
        observation.raw_frontier[7] = digest(40);
        observation.raw_frontier[9] = digest(41);
        observation.raw_frontier_mask = PROGRAMDATA_OBSERVATION_MAX_REAL_CHUNKS_V1 as u16;
        observation.tail_bytes_verified =
            MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1 - u64::from(ARTIFACT_BINDING_CHUNK_SIZE_V1);
        observation.validate_static().unwrap();

        observation.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1 + 1;
        observation.raw_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        assert_eq!(
            observation.validate_static(),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn comparative_chunk_candidates_cannot_change_normal_release_admission() {
        for chunk_size in PROGRAMDATA_OBSERVATION_CHUNK_SIZE_CANDIDATES_V1 {
            let mut policy = capacity_policy();
            let count = programdata_observation_chunk_count(
                policy.maximum_raw_programdata_length,
                chunk_size,
            )
            .unwrap();
            let padded = count.next_power_of_two();
            let depth = padded.trailing_zeros() as u8;
            policy.observation_chunk_size = chunk_size;
            policy.observation_max_chunk_count = count;
            policy.observation_padded_leaf_count = padded;
            policy.observation_tree_depth = depth;

            if cfg!(feature = "programdata-observation-chunk-matrix")
                || chunk_size == PROGRAMDATA_OBSERVATION_CHUNK_SIZE_16_KIB
            {
                policy.validate_static().unwrap();
            } else {
                assert_eq!(
                    policy.validate_static(),
                    Err(GovernanceError::InvalidRelease1Account)
                );
            }
        }
    }

    #[test]
    fn purpose_is_an_explicit_part_of_every_observation_binding() {
        let purposes = [
            ProgramDataObservationPurposeV1::ControllerImmutability,
            ProgramDataObservationPurposeV1::TargetHandoffBridge,
            ProgramDataObservationPurposeV1::ProposalPrestate,
            ProgramDataObservationPurposeV1::PostUpgrade,
            ProgramDataObservationPurposeV1::Rollback,
            ProgramDataObservationPurposeV1::EmergencyResolution,
            ProgramDataObservationPurposeV1::BootstrapActivation,
        ];
        let bindings = purposes
            .into_iter()
            .map(|purpose| {
                let mut observation = observation_with(purpose, some(30));
                match purpose {
                    ProgramDataObservationPurposeV1::ControllerImmutability
                    | ProgramDataObservationPurposeV1::TargetHandoffBridge
                    | ProgramDataObservationPurposeV1::BootstrapActivation => {
                        observation.gate_status = GateStatusV1::EmergencyFrozen;
                        observation.gate_active_proposal = Pubkey::default();
                        observation.gate_freeze_reason_code =
                            BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1;
                    }
                    ProgramDataObservationPurposeV1::EmergencyResolution => {
                        observation.gate_status = GateStatusV1::EmergencyFrozen;
                        observation.gate_active_proposal = Pubkey::default();
                    }
                    ProgramDataObservationPurposeV1::ProposalPrestate
                    | ProgramDataObservationPurposeV1::PostUpgrade
                    | ProgramDataObservationPurposeV1::Rollback => {}
                }
                observation.validate_static().unwrap();
                let encoded = observation.try_to_vec().unwrap();
                assert_eq!(encoded[139], purpose as u8);
                (observation.purpose_binding(), encoded)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bindings
                .iter()
                .map(|(binding, _)| *binding)
                .collect::<BTreeSet<_>>()
                .len(),
            purposes.len()
        );
        assert_eq!(
            bindings
                .iter()
                .map(|(_, encoded)| encoded[139])
                .collect::<BTreeSet<_>>()
                .len(),
            purposes.len()
        );
    }

    #[test]
    fn strict_account_decoder_rejects_invalid_inner_enum_values() {
        let mut observation =
            observation_with(ProgramDataObservationPurposeV1::ProposalPrestate, some(30))
                .try_to_vec()
                .unwrap();
        observation[139] = 7;
        assert_eq!(
            ProgramDataObservationV1::from_bytes_strict(&observation),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut handoff = handoff_proposal().try_to_vec().unwrap();
        handoff[11] = 5;
        assert_eq!(
            TargetAuthorityHandoffProposalV1::from_bytes_strict(&handoff),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut activation = activation_proposal().try_to_vec().unwrap();
        activation[11] = 5;
        assert_eq!(
            BootstrapActivationProposalV1::from_bytes_strict(&activation),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }
}
