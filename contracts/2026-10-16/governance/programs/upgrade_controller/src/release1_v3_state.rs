//! Capacity-safe Release 1 lifecycle account schemas.
//!
//! These are fixed-width byte contracts only. They deliberately preserve every
//! published V1/V2 meaning and introduce fresh discriminators and digest domains
//! for the ceremony-safe lifecycle. Mutable Loader-v3 observations are bound by
//! explicit generations; the immutable proposal commits only the artifact and
//! minimum capacity policy, so a mechanically proven zero-only extension cannot
//! invalidate council approval forever.

use std::io::{Error, ErrorKind, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::{
    artifact_merkle::{
        artifact_chunk_count, is_release1_chunk_size, ARTIFACT_MERKLE_SCHEME_ID,
        MAX_ARTIFACT_BYTES_V1,
    },
    council::VALID_APPROVAL_MASK,
    pda::UPGRADEABLE_LOADER_ID,
    programdata_observation_merkle::{
        MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
    },
    release1_ceremony_state::ProgramDataObservationPurposeV1,
    release1_state::{
        EmergencyFreezeResolutionKindV1, EmergencyFreezeResolutionStateV1, ProposalStateV2,
        StateCheckpointPhaseV1, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        LOADER_V3_PROGRAMDATA_METADATA_LEN_V1, LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
        NO_FAILING_CHUNK_INDEX_V1, PROPOSAL_COMPLETED_TERMINAL_REASON_V1,
        PROPOSAL_EXPIRED_TERMINAL_REASON_V1, PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1,
        PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1, RELEASE1_APPROVAL_THRESHOLD,
    },
    state::{GateStatusV1, OptionalPubkeyV1, ProposalClassV1, VoteRequirementV1},
    GovernanceError, GovernanceResult,
};

pub const UPGRADE_PROPOSAL_V3_DISCRIMINATOR: [u8; 8] = *b"AGVPRP03";
pub const PROGRAMDATA_VERIFICATION_V2_DISCRIMINATOR: [u8; 8] = *b"AGVPDV02";
pub const STATE_CHECKPOINT_V2_DISCRIMINATOR: [u8; 8] = *b"AGVCKP02";
pub const EMERGENCY_FREEZE_OBSERVATION_V2_DISCRIMINATOR: [u8; 8] = *b"AGVEFO02";
pub const EMERGENCY_FREEZE_RESOLUTION_V2_DISCRIMINATOR: [u8; 8] = *b"AGVEFR02";
pub const PROGRAMDATA_FAILURE_OBSERVATION_V2_DISCRIMINATOR: [u8; 8] = *b"AGVPDF02";

pub const CAPACITY_SAFE_ACCOUNT_VERSION_V2: u8 = 2;
pub const CAPACITY_SAFE_ACCOUNT_VERSION_V3: u8 = 3;
pub const MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1: u64 =
    MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;

pub const UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN: &[u8] = b"AMOEBA_UPGRADE_PROPOSAL_V3";
pub const PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN: &[u8] = b"AMOEBA_PROGRAMDATA_VERIFICATION_V2";
pub const STATE_CHECKPOINT_V2_DIGEST_DOMAIN: &[u8] = b"AMOEBA_STATE_CHECKPOINT_V2";
pub const EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN: &[u8] =
    b"AMOEBA_EMERGENCY_FREEZE_OBSERVATION_V2";
pub const EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN: &[u8] =
    b"AMOEBA_EMERGENCY_FREEZE_RESOLUTION_V2";
pub const PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN: &[u8] =
    b"AMOEBA_PROGRAMDATA_FAILURE_OBSERVATION_V2";

pub const UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID: [u8; 32] = [
    0x42, 0x92, 0x17, 0xe7, 0x04, 0xbe, 0x92, 0xc9, 0x8b, 0x1e, 0xca, 0xc1, 0x09, 0x7b, 0x3d, 0xcd,
    0xe5, 0x43, 0xd6, 0x2f, 0x6b, 0xb1, 0x56, 0xf2, 0x59, 0x8e, 0xe6, 0x8b, 0x9a, 0x39, 0x83, 0x71,
];
pub const PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN_ID: [u8; 32] = [
    0xb0, 0xda, 0x6e, 0x44, 0x00, 0xf0, 0xe5, 0x90, 0xba, 0x6e, 0x2d, 0x8b, 0x49, 0xec, 0xf2, 0x12,
    0xc9, 0xf5, 0xd1, 0x11, 0x1d, 0x3c, 0x4a, 0x95, 0xae, 0x88, 0x66, 0x13, 0xce, 0x9d, 0x58, 0xa6,
];
pub const STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID: [u8; 32] = [
    0x67, 0x92, 0x27, 0xa2, 0x0e, 0x6c, 0xd2, 0xa1, 0x01, 0x31, 0xae, 0xb5, 0x51, 0x80, 0x92, 0x77,
    0x93, 0x1b, 0x2d, 0x23, 0xe2, 0x0e, 0x5b, 0x64, 0x7d, 0x76, 0xb0, 0xbd, 0x60, 0xa9, 0x39, 0x16,
];
pub const EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN_ID: [u8; 32] = [
    0x8d, 0x1f, 0x57, 0x72, 0x31, 0x87, 0x8d, 0x92, 0xea, 0x85, 0x7c, 0xcd, 0xae, 0x9f, 0xf0, 0xda,
    0x65, 0x0d, 0x36, 0xe6, 0x17, 0x11, 0x30, 0x53, 0x0f, 0x75, 0x40, 0xf2, 0x91, 0xbb, 0x26, 0x8c,
];
pub const EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID: [u8; 32] = [
    0x41, 0xee, 0x8b, 0x0e, 0x1d, 0xce, 0x34, 0xee, 0xaa, 0xcd, 0x26, 0x5c, 0xe3, 0x97, 0xfa, 0xaf,
    0x49, 0x2d, 0x59, 0x83, 0x58, 0x5e, 0x12, 0xe3, 0x1d, 0xf7, 0x1c, 0x09, 0x97, 0x4f, 0x88, 0xbd,
];
pub const PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN_ID: [u8; 32] = [
    0xbd, 0xd3, 0xfc, 0x71, 0x6a, 0xb1, 0xdc, 0x57, 0x33, 0x90, 0x4f, 0xcb, 0xf4, 0x85, 0x8b, 0xf2,
    0x24, 0x9c, 0xcd, 0x9d, 0xd5, 0x30, 0x53, 0x0b, 0x1d, 0x9e, 0xb8, 0xe4, 0x5b, 0x28, 0xca, 0xd0,
];

pub const UPGRADE_PROPOSAL_V3_RESERVED_LEN: usize = 274;
pub const PROGRAMDATA_VERIFICATION_V2_RESERVED_LEN: usize = 129;
pub const STATE_CHECKPOINT_V2_RESERVED_LEN: usize = 147;
pub const EMERGENCY_FREEZE_OBSERVATION_V2_RESERVED_LEN: usize = 21;
pub const EMERGENCY_FREEZE_RESOLUTION_V2_RESERVED_LEN: usize = 122;
pub const PROGRAMDATA_FAILURE_OBSERVATION_V2_RESERVED_LEN: usize = 47;

pub const EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V2: u16 = 1;
pub const EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V2: u16 = 2;

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
                value.validate_schema()?;
                Ok(value)
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgramDataVerificationStatusV2 {
    ObservationBound = 0,
    Verified = 1,
}
fixed_u8_enum_borsh!(ProgramDataVerificationStatusV2 {
    ObservationBound = 0,
    Verified = 1,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgramDataMismatchClassV2 {
    ProgramLinkage = 0,
    ProgramOwner = 1,
    ProgramExecutable = 2,
    ProgramDataOwner = 3,
    ProgramDataExecutable = 4,
    Header = 5,
    Authority = 6,
    CapacityDecrease = 7,
    CapacityAboveRuntimeMaximum = 8,
    ArtifactLength = 9,
    ArtifactPayload = 10,
    ZeroTail = 11,
    ObservationStale = 12,
    ObservationScheme = 13,
}
fixed_u8_enum_borsh!(ProgramDataMismatchClassV2 {
    ProgramLinkage = 0,
    ProgramOwner = 1,
    ProgramExecutable = 2,
    ProgramDataOwner = 3,
    ProgramDataExecutable = 4,
    Header = 5,
    Authority = 6,
    CapacityDecrease = 7,
    CapacityAboveRuntimeMaximum = 8,
    ArtifactLength = 9,
    ArtifactPayload = 10,
    ZeroTail = 11,
    ObservationStale = 12,
    ObservationScheme = 13,
});

/// Capacity-safe proposal. Actual Loader slot/capacity/root values are not part
/// of this immutable proposal commitment; they are supplied by generation-bound
/// observations, checkpoints, and verification accounts at each transition.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct UpgradeProposalV3 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub proposal_class: ProposalClassV1,
    pub state: ProposalStateV2,
    pub creation_gate_status: GateStatusV1,
    pub zero_tail_required: bool,
    pub proposal_flags: u8,
    pub proposal_id: u64,
    pub target_nonce: u64,
    pub creation_slot: u64,
    pub cluster_domain: [u8; 32],
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub policy_version: u64,
    pub policy_hash: [u8; 32],
    pub creation_council_version: u64,
    pub creation_council_hash: [u8; 32],
    pub creation_gate_epoch: u64,
    pub freeze_gate_epoch: u64,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub authority_pda: Pubkey,
    pub canonical_spill_treasury: Pubkey,
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub current_deployment_generation: u64,
    pub buffer_pubkey: Pubkey,
    pub buffer_loader_owner: Pubkey,
    pub buffer_uploader_authority: Pubkey,
    pub buffer_final_authority: Pubkey,
    pub buffer_verification: Pubkey,
    pub programdata_verification: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_chunk_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub artifact_chunk_size: u32,
    pub artifact_chunk_count: u32,
    pub source_commit_hash: [u8; 32],
    pub source_tree_hash: [u8; 32],
    pub build_input_inventory_hash: [u8; 32],
    pub reproducible_build_receipt_hash: [u8; 32],
    pub package_receipt_hash: [u8; 32],
    pub release_intent_hash: [u8; 32],
    pub minimum_required_capacity: u64,
    pub maximum_supported_raw_programdata_length: u64,
    pub programdata_observation_scheme_id: [u8; 32],
    pub prestate_checkpoint: Pubkey,
    pub required_poststate_checkpoint: Pubkey,
    pub checkpoint_schema_id: [u8; 32],
    pub checkpoint_policy_hash: [u8; 32],
    pub primary_proposal: OptionalPubkeyV1,
    pub rollback_proposal: OptionalPubkeyV1,
    pub rollback_buffer: OptionalPubkeyV1,
    pub rollback_artifact_length: u64,
    pub rollback_artifact_sha256: [u8; 32],
    pub rollback_artifact_chunk_root: [u8; 32],
    pub rollback_artifact_scheme_id: [u8; 32],
    pub vote_requirement: VoteRequirementV1,
    pub vote_program: Pubkey,
    pub vote_result_pda: Pubkey,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub governance_satisfied_slot: u64,
    pub queued_slot: u64,
    pub frozen_slot: u64,
    pub extension_executed_slot: u64,
    pub upgrade_executed_slot: u64,
    pub programdata_verified_slot: u64,
    pub poststate_accepted_slot: u64,
    pub unfreeze_approved_slot: u64,
    pub terminal_slot: u64,
    pub council_approval_bitset: u8,
    pub council_approval_count: u8,
    pub cancellation_council_version: u64,
    pub cancellation_council_hash: [u8; 32],
    pub cancellation_approval_bitset: u8,
    pub cancellation_approval_count: u8,
    pub unfreeze_council_version: u64,
    pub unfreeze_council_hash: [u8; 32],
    pub unfreeze_approval_bitset: u8,
    pub unfreeze_approval_count: u8,
    pub proposal_digest_domain_id: [u8; 32],
    pub proposal_digest: [u8; 32],
    pub cancellation_reason_code: u16,
    pub terminal_reason_code: u16,
    pub reserved: [u8; UPGRADE_PROPOSAL_V3_RESERVED_LEN],
}

impl UpgradeProposalV3 {
    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &UPGRADE_PROPOSAL_V3_DISCRIMINATOR,
            self.account_version,
            CAPACITY_SAFE_ACCOUNT_VERSION_V3,
            self.initialized,
            &self.reserved,
        )?;
        if self.proposal_flags != 0
            || !self.zero_tail_required
            || matches!(self.state, ProposalStateV2::TokenReviewOpen)
            || !matches!(
                self.creation_gate_status,
                GateStatusV1::Active | GateStatusV1::EmergencyFrozen
            )
            || self.proposal_digest_domain_id != UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.protocol_gate,
            self.capacity_policy,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.authority_pda,
            self.canonical_spill_treasury,
            self.current_deployment_state,
            self.buffer_pubkey,
            self.buffer_loader_owner,
            self.buffer_uploader_authority,
            self.buffer_final_authority,
            self.buffer_verification,
            self.programdata_verification,
            self.prestate_checkpoint,
            self.required_poststate_checkpoint,
        ])?;
        require_nonzero_hashes(&[
            self.cluster_domain,
            self.capacity_policy_digest,
            self.policy_hash,
            self.creation_council_hash,
            self.current_deployment_digest,
            self.source_commit_hash,
            self.source_tree_hash,
            self.build_input_inventory_hash,
            self.reproducible_build_receipt_hash,
            self.package_receipt_hash,
            self.release_intent_hash,
            self.checkpoint_schema_id,
            self.checkpoint_policy_hash,
            self.proposal_digest,
        ])?;
        if self.proposal_id == 0
            || self.target_nonce == 0
            || self.creation_slot == 0
            || self.policy_version == 0
            || self.creation_council_version == 0
            || self.creation_gate_epoch == 0
            || self.current_deployment_generation == 0
            || self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.buffer_loader_owner != self.upgradeable_loader
            || self.buffer_final_authority != self.authority_pda
            || self.maximum_supported_raw_programdata_length != MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
            || self.programdata_observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        validate_artifact_identity(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_chunk_merkle_root,
            &self.artifact_scheme_id,
            self.artifact_chunk_size,
            self.artifact_chunk_count,
        )?;
        if self.minimum_required_capacity < self.artifact_length
            || self.minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        {
            return Err(GovernanceError::InvalidCapacityPlan);
        }
        if self.creation_slot > self.review_start_slot
            || self.review_start_slot >= self.review_end_slot
            || self.review_end_slot > self.not_before_slot
            || self.not_before_slot >= self.expiry_slot
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        self.primary_proposal.validate()?;
        self.rollback_proposal.validate()?;
        self.rollback_buffer.validate()?;
        validate_rollback_shape(self)?;
        if self.vote_requirement != VoteRequirementV1::None
            || self.vote_program != Pubkey::default()
            || self.vote_result_pda != Pubkey::default()
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        validate_approval_pair(self.council_approval_bitset, self.council_approval_count)?;
        validate_versioned_approval(
            self.cancellation_council_version,
            &self.cancellation_council_hash,
            self.cancellation_approval_bitset,
            self.cancellation_approval_count,
        )?;
        validate_versioned_approval(
            self.unfreeze_council_version,
            &self.unfreeze_council_hash,
            self.unfreeze_approval_bitset,
            self.unfreeze_approval_count,
        )?;
        validate_upgrade_proposal_v3_lifecycle(self)
    }
}
impl_strict_account!(UpgradeProposalV3, 2_048);

impl UpgradeProposalV3 {
    /// Strictly decodes the largest Release 1 account directly into heap
    /// storage. The derived Borsh decoder first constructs the full value in
    /// its caller's return place, which exceeds the SBPF-v0 frame limit when
    /// used through the generic fixed-account loader.
    pub fn from_bytes_boxed_strict(data: &[u8]) -> GovernanceResult<Box<Self>> {
        if data.len() != Self::LEN {
            return Err(GovernanceError::InvalidAccountSize);
        }
        let mut input = data;
        let mut value = Box::<Self>::new_uninit();
        let raw = value.as_mut_ptr();
        macro_rules! read_field {
            ($field:ident) => {{
                let decoded = BorshDeserialize::deserialize(&mut input)
                    .map_err(|_| GovernanceError::InvalidRelease1Account)?;
                // SAFETY: every field is written exactly once in declaration
                // order. The allocation remains `MaybeUninit` until all
                // writes and the strict trailing-byte check have succeeded.
                unsafe { core::ptr::addr_of_mut!((*raw).$field).write(decoded) };
            }};
        }

        read_field!(discriminator);
        read_field!(account_version);
        read_field!(bump);
        read_field!(initialized);
        read_field!(proposal_class);
        read_field!(state);
        read_field!(creation_gate_status);
        read_field!(zero_tail_required);
        read_field!(proposal_flags);
        read_field!(proposal_id);
        read_field!(target_nonce);
        read_field!(creation_slot);
        read_field!(cluster_domain);
        read_field!(controller_program);
        read_field!(controller_config);
        read_field!(protocol_gate);
        read_field!(capacity_policy);
        read_field!(capacity_policy_digest);
        read_field!(policy_version);
        read_field!(policy_hash);
        read_field!(creation_council_version);
        read_field!(creation_council_hash);
        read_field!(creation_gate_epoch);
        read_field!(freeze_gate_epoch);
        read_field!(target_program);
        read_field!(target_programdata);
        read_field!(upgradeable_loader);
        read_field!(authority_pda);
        read_field!(canonical_spill_treasury);
        read_field!(current_deployment_state);
        read_field!(current_deployment_digest);
        read_field!(current_deployment_generation);
        read_field!(buffer_pubkey);
        read_field!(buffer_loader_owner);
        read_field!(buffer_uploader_authority);
        read_field!(buffer_final_authority);
        read_field!(buffer_verification);
        read_field!(programdata_verification);
        read_field!(artifact_length);
        read_field!(artifact_sha256);
        read_field!(artifact_chunk_merkle_root);
        read_field!(artifact_scheme_id);
        read_field!(artifact_chunk_size);
        read_field!(artifact_chunk_count);
        read_field!(source_commit_hash);
        read_field!(source_tree_hash);
        read_field!(build_input_inventory_hash);
        read_field!(reproducible_build_receipt_hash);
        read_field!(package_receipt_hash);
        read_field!(release_intent_hash);
        read_field!(minimum_required_capacity);
        read_field!(maximum_supported_raw_programdata_length);
        read_field!(programdata_observation_scheme_id);
        read_field!(prestate_checkpoint);
        read_field!(required_poststate_checkpoint);
        read_field!(checkpoint_schema_id);
        read_field!(checkpoint_policy_hash);
        read_field!(primary_proposal);
        read_field!(rollback_proposal);
        read_field!(rollback_buffer);
        read_field!(rollback_artifact_length);
        read_field!(rollback_artifact_sha256);
        read_field!(rollback_artifact_chunk_root);
        read_field!(rollback_artifact_scheme_id);
        read_field!(vote_requirement);
        read_field!(vote_program);
        read_field!(vote_result_pda);
        read_field!(review_start_slot);
        read_field!(review_end_slot);
        read_field!(not_before_slot);
        read_field!(expiry_slot);
        read_field!(first_approval_slot);
        read_field!(council_approved_slot);
        read_field!(governance_satisfied_slot);
        read_field!(queued_slot);
        read_field!(frozen_slot);
        read_field!(extension_executed_slot);
        read_field!(upgrade_executed_slot);
        read_field!(programdata_verified_slot);
        read_field!(poststate_accepted_slot);
        read_field!(unfreeze_approved_slot);
        read_field!(terminal_slot);
        read_field!(council_approval_bitset);
        read_field!(council_approval_count);
        read_field!(cancellation_council_version);
        read_field!(cancellation_council_hash);
        read_field!(cancellation_approval_bitset);
        read_field!(cancellation_approval_count);
        read_field!(unfreeze_council_version);
        read_field!(unfreeze_council_hash);
        read_field!(unfreeze_approval_bitset);
        read_field!(unfreeze_approval_count);
        read_field!(proposal_digest_domain_id);
        read_field!(proposal_digest);
        read_field!(cancellation_reason_code);
        read_field!(terminal_reason_code);
        read_field!(reserved);

        if !input.is_empty() {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        // SAFETY: all fields above were initialized exactly once.
        let value = unsafe { value.assume_init() };
        value.validate_schema()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataVerificationV2 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub status: ProgramDataVerificationStatusV2,
    pub controller_config: Pubkey,
    pub proposal: Pubkey,
    pub proposal_digest: [u8; 32],
    pub protocol_gate: Pubkey,
    pub freeze_gate_epoch: u64,
    pub target_nonce: u64,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub current_deployment_generation: u64,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub minimum_required_capacity: u64,
    pub maximum_supported_raw_programdata_length: u64,
    pub observation_scheme_id: [u8; 32],
    pub programdata_observation: Pubkey,
    pub observation_purpose: ProgramDataObservationPurposeV1,
    pub observation_generation: u64,
    pub observation_subject_digest: [u8; 32],
    pub observation_root: [u8; 32],
    pub observation_digest: [u8; 32],
    pub observation_finalized_slot: u64,
    pub observed_deployed_slot: u64,
    pub observed_raw_data_length: u64,
    pub actual_capacity: u64,
    pub observed_authority: OptionalPubkeyV1,
    pub zero_tail_verified: bool,
    pub verification_generation: u64,
    pub previous_verification_digest: [u8; 32],
    pub verification_digest_domain_id: [u8; 32],
    pub verification_digest: [u8; 32],
    pub bound_slot: u64,
    pub finalized_slot: u64,
    pub reserved: [u8; PROGRAMDATA_VERIFICATION_V2_RESERVED_LEN],
}

impl ProgramDataVerificationV2 {
    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROGRAMDATA_VERIFICATION_V2_DISCRIMINATOR,
            self.account_version,
            CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.proposal,
            self.protocol_gate,
            self.capacity_policy,
            self.current_deployment_state,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.controller_authority,
            self.programdata_observation,
        ])?;
        require_nonzero_hashes(&[
            self.proposal_digest,
            self.capacity_policy_digest,
            self.current_deployment_digest,
            self.observation_subject_digest,
            self.observation_root,
            self.observation_digest,
            self.verification_digest,
        ])?;
        self.observed_authority.validate()?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.freeze_gate_epoch == 0
            || self.target_nonce == 0
            || self.current_deployment_generation == 0
            || self.maximum_supported_raw_programdata_length != MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
            || self.observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
            || !matches!(
                self.observation_purpose,
                ProgramDataObservationPurposeV1::PostUpgrade
                    | ProgramDataObservationPurposeV1::Rollback
            )
            || self.observation_generation == 0
            || self.observation_finalized_slot == 0
            || self.observed_deployed_slot == 0
            || !self.observed_authority.present
            || self.observed_authority.value != self.controller_authority
            || self.verification_generation == 0
            || self.verification_digest_domain_id != PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN_ID
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_artifact_identity_without_chunks(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        validate_capacity_observation(
            self.artifact_length,
            self.minimum_required_capacity,
            self.observed_raw_data_length,
            self.actual_capacity,
        )?;
        validate_generation_chain(
            self.verification_generation,
            &self.previous_verification_digest,
        )?;
        if self.bound_slot < self.observation_finalized_slot {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        match self.status {
            ProgramDataVerificationStatusV2::ObservationBound => {
                if self.zero_tail_verified || self.finalized_slot != 0 {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
            ProgramDataVerificationStatusV2::Verified => {
                if !self.zero_tail_verified || self.finalized_slot < self.bound_slot {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
        }
        Ok(())
    }
}
impl_strict_account!(ProgramDataVerificationV2, 1_024);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StateCheckpointV2 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub phase: StateCheckpointPhaseV1,
    pub controller_config: Pubkey,
    pub proposal: Pubkey,
    pub emergency_resolution: Pubkey,
    pub subject_digest: [u8; 32],
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub current_deployment_generation: u64,
    pub checkpoint_generation: u64,
    pub previous_checkpoint_digest: [u8; 32],
    pub observation_scheme_id: [u8; 32],
    pub programdata_observation: Pubkey,
    pub observation_purpose: ProgramDataObservationPurposeV1,
    pub observation_generation: u64,
    pub observation_subject_digest: [u8; 32],
    pub observation_root: [u8; 32],
    pub observation_digest: [u8; 32],
    pub observation_finalized_slot: u64,
    pub gate_epoch: u64,
    pub target_programdata_slot: u64,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub minimum_required_capacity: u64,
    pub observed_raw_data_length: u64,
    pub actual_capacity: u64,
    pub observed_authority: OptionalPubkeyV1,
    pub program_owned_state_root: [u8; 32],
    pub program_owned_state_count: u64,
    pub logical_compressed_state_root: [u8; 32],
    pub logical_compressed_state_count: u64,
    pub semantic_custody_accounting_root: [u8; 32],
    pub hard_combined_root: [u8; 32],
    pub external_metadata_observation_root: [u8; 32],
    pub external_raw_balance_observation_root: [u8; 32],
    pub schema_identifier: [u8; 32],
    pub admitted_positive_donation_root: [u8; 32],
    pub admitted_positive_donation_count: u64,
    pub forbidden_drift_count: u32,
    pub approval_council_version: u64,
    pub approval_council_hash: [u8; 32],
    pub checkpoint_digest_domain_id: [u8; 32],
    pub checkpoint_digest: [u8; 32],
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub accepted: bool,
    pub finalized_slot: u64,
    pub reserved: [u8; STATE_CHECKPOINT_V2_RESERVED_LEN],
}

impl StateCheckpointV2 {
    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &STATE_CHECKPOINT_V2_DISCRIMINATOR,
            self.account_version,
            CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.target_program,
            self.target_programdata,
            self.capacity_policy,
            self.current_deployment_state,
            self.programdata_observation,
        ])?;
        let subject_shape = match self.phase {
            StateCheckpointPhaseV1::Prestate | StateCheckpointPhaseV1::Poststate => {
                self.proposal != Pubkey::default() && self.emergency_resolution == Pubkey::default()
            }
            StateCheckpointPhaseV1::Emergency => {
                self.proposal == Pubkey::default() && self.emergency_resolution != Pubkey::default()
            }
        };
        let purpose_shape = match self.phase {
            StateCheckpointPhaseV1::Prestate => {
                self.observation_purpose == ProgramDataObservationPurposeV1::ProposalPrestate
            }
            StateCheckpointPhaseV1::Poststate => matches!(
                self.observation_purpose,
                ProgramDataObservationPurposeV1::PostUpgrade
                    | ProgramDataObservationPurposeV1::Rollback
            ),
            StateCheckpointPhaseV1::Emergency => {
                self.observation_purpose == ProgramDataObservationPurposeV1::EmergencyResolution
            }
        };
        if !subject_shape
            || !purpose_shape
            || self.current_deployment_generation == 0
            || self.checkpoint_generation == 0
            || self.observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
            || self.observation_generation == 0
            || self.observation_finalized_slot == 0
            || self.gate_epoch == 0
            || self.target_programdata_slot == 0
            || self.checkpoint_digest_domain_id != STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID
            || self.finalized_slot < self.observation_finalized_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        require_nonzero_hashes(&[
            self.subject_digest,
            self.capacity_policy_digest,
            self.current_deployment_digest,
            self.observation_subject_digest,
            self.observation_root,
            self.observation_digest,
            self.program_owned_state_root,
            self.logical_compressed_state_root,
            self.semantic_custody_accounting_root,
            self.hard_combined_root,
            self.external_metadata_observation_root,
            self.external_raw_balance_observation_root,
            self.schema_identifier,
            self.checkpoint_digest,
        ])?;
        self.observed_authority.validate()?;
        if !self.observed_authority.present {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_artifact_identity_without_chunks(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        validate_capacity_observation(
            self.artifact_length,
            self.minimum_required_capacity,
            self.observed_raw_data_length,
            self.actual_capacity,
        )?;
        validate_generation_chain(self.checkpoint_generation, &self.previous_checkpoint_digest)?;
        if (self.admitted_positive_donation_count == 0)
            != (self.admitted_positive_donation_root == [0; 32])
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_pinned_approval(
            self.approval_council_version,
            &self.approval_council_hash,
            self.approval_bitset,
            self.approval_count,
        )?;
        if self.approval_count > RELEASE1_APPROVAL_THRESHOLD
            || self.accepted
                != (self.forbidden_drift_count == 0
                    && self.approval_count == RELEASE1_APPROVAL_THRESHOLD)
            || (self.forbidden_drift_count != 0 && self.approval_count != 0)
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(StateCheckpointV2, 1_280);

/// O(1) guardian-freeze evidence. It binds only the canonical header graph and
/// trusted deployment identity; raw byte traversal happens after the gate is
/// frozen in a separate `ProgramDataObservationV1` generation.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct EmergencyFreezeObservationV2 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub finalized: bool,
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub current_deployment_generation: u64,
    pub trusted_artifact_length: u64,
    pub trusted_artifact_sha256: [u8; 32],
    pub trusted_artifact_merkle_root: [u8; 32],
    pub trusted_artifact_scheme_id: [u8; 32],
    pub minimum_required_capacity: u64,
    pub frozen_epoch: u64,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub target_nonce: u64,
    pub actual_program_owner: Pubkey,
    pub actual_program_executable: bool,
    pub actual_program_data_length: u64,
    pub program_header_present: bool,
    pub actual_linked_programdata: Pubkey,
    pub actual_programdata_owner: Pubkey,
    pub actual_programdata_executable: bool,
    pub actual_programdata_data_length: u64,
    pub programdata_header_present: bool,
    pub deployed_programdata_slot: u64,
    pub actual_capacity: u64,
    pub observed_authority: OptionalPubkeyV1,
    pub observation_digest_domain_id: [u8; 32],
    pub observation_digest: [u8; 32],
    pub finalized_slot: u64,
    pub reserved: [u8; EMERGENCY_FREEZE_OBSERVATION_V2_RESERVED_LEN],
}

impl EmergencyFreezeObservationV2 {
    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &EMERGENCY_FREEZE_OBSERVATION_V2_DISCRIMINATOR,
            self.account_version,
            CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.protocol_gate,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.controller_authority,
            self.capacity_policy,
            self.current_deployment_state,
            self.actual_program_owner,
            self.actual_linked_programdata,
            self.actual_programdata_owner,
        ])?;
        require_nonzero_hashes(&[
            self.capacity_policy_digest,
            self.current_deployment_digest,
            self.observation_digest,
        ])?;
        self.observed_authority.validate()?;
        validate_artifact_identity_without_chunks(
            self.trusted_artifact_length,
            &self.trusted_artifact_sha256,
            &self.trusted_artifact_merkle_root,
            &self.trusted_artifact_scheme_id,
        )?;
        if !self.finalized
            || self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.current_deployment_generation == 0
            || self.minimum_required_capacity < self.trusted_artifact_length
            || self.minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.frozen_epoch == 0
            || self.freeze_slot == 0
            || self.freeze_reason_code == 0
            || self.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.target_nonce == 0
            || self.actual_program_owner != self.upgradeable_loader
            || !self.actual_program_executable
            || self.actual_program_data_length != LOADER_V3_PROGRAM_ACCOUNT_LEN_V1
            || !self.program_header_present
            || self.actual_linked_programdata != self.target_programdata
            || self.actual_programdata_owner != self.upgradeable_loader
            || self.actual_programdata_executable
            || !self.programdata_header_present
            || self.deployed_programdata_slot == 0
            || !self.observed_authority.present
            || self.observed_authority.value != self.controller_authority
            || self.observation_digest_domain_id != EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN_ID
            || self.finalized_slot != self.freeze_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_capacity_observation(
            self.trusted_artifact_length,
            self.minimum_required_capacity,
            self.actual_programdata_data_length,
            self.actual_capacity,
        )
    }
}
impl_strict_account!(EmergencyFreezeObservationV2, 768);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct EmergencyFreezeResolutionV2 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: EmergencyFreezeResolutionStateV1,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub current_deployment_generation: u64,
    pub emergency_freeze_observation: Pubkey,
    pub emergency_freeze_observation_digest: [u8; 32],
    pub frozen_epoch: u64,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub resolution_kind: EmergencyFreezeResolutionKindV1,
    pub creation_slot: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub target_nonce: u64,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_merkle_root: [u8; 32],
    pub artifact_scheme_id: [u8; 32],
    pub minimum_required_capacity: u64,
    pub observation_scheme_id: [u8; 32],
    pub programdata_observation: Pubkey,
    pub observation_purpose: ProgramDataObservationPurposeV1,
    pub observation_generation: u64,
    pub observation_subject_digest: [u8; 32],
    pub observation_root: [u8; 32],
    pub observation_digest: [u8; 32],
    pub observation_finalized_slot: u64,
    pub observed_deployed_slot: u64,
    pub observed_raw_data_length: u64,
    pub actual_capacity: u64,
    pub observed_authority: OptionalPubkeyV1,
    pub emergency_checkpoint: Pubkey,
    pub emergency_checkpoint_digest: [u8; 32],
    pub approval_council_version: u64,
    pub approval_council_hash: [u8; 32],
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub resolution_digest_domain_id: [u8; 32],
    pub resolution_digest: [u8; 32],
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub reserved: [u8; EMERGENCY_FREEZE_RESOLUTION_V2_RESERVED_LEN],
}

impl EmergencyFreezeResolutionV2 {
    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &EMERGENCY_FREEZE_RESOLUTION_V2_DISCRIMINATOR,
            self.account_version,
            CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.protocol_gate,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.controller_authority,
            self.capacity_policy,
            self.current_deployment_state,
            self.emergency_freeze_observation,
            self.programdata_observation,
            self.emergency_checkpoint,
        ])?;
        require_nonzero_hashes(&[
            self.capacity_policy_digest,
            self.current_deployment_digest,
            self.emergency_freeze_observation_digest,
            self.observation_subject_digest,
            self.observation_root,
            self.observation_digest,
            self.approval_council_hash,
            self.resolution_digest,
        ])?;
        self.observed_authority.validate()?;
        if self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.current_deployment_generation == 0
            || self.frozen_epoch == 0
            || self.freeze_slot == 0
            || self.freeze_reason_code == 0
            || self.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.target_nonce == 0
            || self.observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
            || self.observation_purpose != ProgramDataObservationPurposeV1::EmergencyResolution
            || self.observation_generation == 0
            || self.observation_subject_digest != self.emergency_freeze_observation_digest
            || self.observation_finalized_slot == 0
            || self.observed_deployed_slot == 0
            || !self.observed_authority.present
            || self.observed_authority.value != self.controller_authority
            || self.approval_council_version == 0
            || self.approval_threshold != RELEASE1_APPROVAL_THRESHOLD
            || self.resolution_digest_domain_id != EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_artifact_identity_without_chunks(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_merkle_root,
            &self.artifact_scheme_id,
        )?;
        validate_capacity_observation(
            self.artifact_length,
            self.minimum_required_capacity,
            self.observed_raw_data_length,
            self.actual_capacity,
        )?;
        if self.freeze_slot > self.creation_slot
            || self.creation_slot > self.review_start_slot
            || self.review_start_slot >= self.review_end_slot
            || self.review_end_slot > self.not_before_slot
            || self.not_before_slot >= self.expiry_slot
            || self.observation_finalized_slot < self.freeze_slot
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        validate_approval_pair(self.approval_bitset, self.approval_count)?;
        if self.approval_count > RELEASE1_APPROVAL_THRESHOLD {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if self.approval_count != 0 && self.emergency_checkpoint_digest == [0; 32] {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_emergency_resolution_v2_lifecycle(self)
    }
}
impl_strict_account!(EmergencyFreezeResolutionV2, 1_152);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataFailureObservationV2 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub finalized: bool,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub primary_proposal: Pubkey,
    pub proposal_digest: [u8; 32],
    pub capacity_policy: Pubkey,
    pub capacity_policy_digest: [u8; 32],
    pub current_deployment_state: Pubkey,
    pub current_deployment_digest: [u8; 32],
    pub current_deployment_generation: u64,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub frozen_epoch: u64,
    pub target_nonce: u64,
    pub expected_artifact_length: u64,
    pub expected_artifact_sha256: [u8; 32],
    pub expected_artifact_merkle_root: [u8; 32],
    pub expected_artifact_scheme_id: [u8; 32],
    pub minimum_required_capacity: u64,
    pub observation_scheme_id: [u8; 32],
    pub programdata_observation: Pubkey,
    pub observation_purpose: ProgramDataObservationPurposeV1,
    pub observation_generation: u64,
    pub observation_subject_digest: [u8; 32],
    pub observation_finalized: bool,
    pub observation_root: [u8; 32],
    pub observation_digest: [u8; 32],
    pub actual_program_owner: Pubkey,
    pub actual_program_executable: bool,
    pub actual_program_data_length: u64,
    pub program_header_present: bool,
    pub actual_linked_programdata: OptionalPubkeyV1,
    pub actual_programdata_owner: Pubkey,
    pub actual_programdata_executable: bool,
    pub actual_programdata_data_length: u64,
    pub programdata_header_present: bool,
    pub actual_programdata_slot: u64,
    pub actual_capacity: u64,
    pub actual_authority: OptionalPubkeyV1,
    pub mismatch_class: ProgramDataMismatchClassV2,
    pub failing_chunk_index: u32,
    pub expected_leaf_hash: [u8; 32],
    pub actual_leaf_hash: [u8; 32],
    pub failure_digest_domain_id: [u8; 32],
    pub failure_digest: [u8; 32],
    pub finalized_slot: u64,
    pub reserved: [u8; PROGRAMDATA_FAILURE_OBSERVATION_V2_RESERVED_LEN],
}

impl ProgramDataFailureObservationV2 {
    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROGRAMDATA_FAILURE_OBSERVATION_V2_DISCRIMINATOR,
            self.account_version,
            CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.protocol_gate,
            self.primary_proposal,
            self.capacity_policy,
            self.current_deployment_state,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.programdata_observation,
        ])?;
        require_nonzero_hashes(&[
            self.proposal_digest,
            self.capacity_policy_digest,
            self.current_deployment_digest,
            self.observation_subject_digest,
            self.failure_digest,
        ])?;
        self.actual_linked_programdata.validate()?;
        self.actual_authority.validate()?;
        if !self.finalized
            || self.current_deployment_generation == 0
            || self.upgradeable_loader != UPGRADEABLE_LOADER_ID
            || self.frozen_epoch == 0
            || self.target_nonce == 0
            || self.observation_scheme_id != PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
            || !matches!(
                self.observation_purpose,
                ProgramDataObservationPurposeV1::PostUpgrade
                    | ProgramDataObservationPurposeV1::Rollback
            )
            || self.observation_generation == 0
            || self.failure_digest_domain_id != PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN_ID
            || self.finalized_slot == 0
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_artifact_identity_without_chunks(
            self.expected_artifact_length,
            &self.expected_artifact_sha256,
            &self.expected_artifact_merkle_root,
            &self.expected_artifact_scheme_id,
        )?;
        if self.minimum_required_capacity < self.expected_artifact_length
            || self.minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        {
            return Err(GovernanceError::InvalidCapacityPlan);
        }
        let finalized_observation_shape = if self.observation_finalized {
            self.observation_root != [0; 32] && self.observation_digest != [0; 32]
        } else {
            self.observation_root == [0; 32] && self.observation_digest == [0; 32]
        };
        if !finalized_observation_shape {
            return Err(GovernanceError::InvalidProgramDataObservation);
        }
        validate_failure_graph_shape(self)?;
        let leaf_failure = matches!(
            self.mismatch_class,
            ProgramDataMismatchClassV2::ArtifactPayload | ProgramDataMismatchClassV2::ZeroTail
        );
        let leaf_shape = if leaf_failure {
            self.failing_chunk_index != NO_FAILING_CHUNK_INDEX_V1
                && self.expected_leaf_hash != [0; 32]
                && self.actual_leaf_hash != [0; 32]
                && self.expected_leaf_hash != self.actual_leaf_hash
        } else {
            self.failing_chunk_index == NO_FAILING_CHUNK_INDEX_V1
                && self.expected_leaf_hash == [0; 32]
                && self.actual_leaf_hash == [0; 32]
        };
        if !leaf_shape {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}
impl_strict_account!(ProgramDataFailureObservationV2, 1_024);

pub mod upgrade_proposal_v3_offset {
    pub const ACCOUNT_VERSION: usize = 8;
    pub const INITIALIZED: usize = 10;
    pub const PROPOSAL_CLASS: usize = 11;
    pub const STATE: usize = 12;
    pub const CREATION_GATE_STATUS: usize = 13;
    pub const PROPOSAL_ID: usize = 16;
    pub const CAPACITY_POLICY: usize = 168;
    pub const CURRENT_DEPLOYMENT_STATE: usize = 488;
    pub const ARTIFACT_LENGTH: usize = 752;
    pub const MINIMUM_REQUIRED_CAPACITY: usize = 1_056;
    pub const OBSERVATION_SCHEME_ID: usize = 1_072;
    pub const PROPOSAL_DIGEST_DOMAIN_ID: usize = 1_706;
    pub const PROPOSAL_DIGEST: usize = 1_738;
    pub const RESERVED: usize = 1_774;
}

pub mod programdata_verification_v2_offset {
    pub const STATUS: usize = 11;
    pub const CAPACITY_POLICY: usize = 156;
    pub const ARTIFACT_LENGTH: usize = 420;
    pub const OBSERVATION_SCHEME_ID: usize = 540;
    pub const OBSERVATION_PURPOSE: usize = 604;
    pub const ACTUAL_CAPACITY: usize = 733;
    pub const ZERO_TAIL_VERIFIED: usize = 774;
    pub const VERIFICATION_DIGEST_DOMAIN_ID: usize = 815;
    pub const RESERVED: usize = 895;
}

pub mod state_checkpoint_v2_offset {
    pub const PHASE: usize = 11;
    pub const CAPACITY_POLICY: usize = 204;
    pub const CHECKPOINT_GENERATION: usize = 340;
    pub const OBSERVATION_SCHEME_ID: usize = 380;
    pub const OBSERVATION_PURPOSE: usize = 444;
    pub const ACTUAL_CAPACITY: usize = 693;
    pub const CHECKPOINT_DIGEST_DOMAIN_ID: usize = 1_058;
    pub const ACCEPTED: usize = 1_124;
    pub const RESERVED: usize = 1_133;
}

pub mod emergency_freeze_observation_v2_offset {
    pub const FINALIZED: usize = 11;
    pub const CAPACITY_POLICY: usize = 236;
    pub const FROZEN_EPOCH: usize = 484;
    pub const PROGRAM_OWNER: usize = 510;
    pub const PROGRAMDATA_OWNER: usize = 584;
    pub const ACTUAL_CAPACITY: usize = 634;
    pub const OBSERVATION_DIGEST_DOMAIN_ID: usize = 675;
    pub const RESERVED: usize = 747;
}

pub mod emergency_freeze_resolution_v2_offset {
    pub const STATE: usize = 11;
    pub const CAPACITY_POLICY: usize = 204;
    pub const FROZEN_EPOCH: usize = 404;
    pub const OBSERVATION_SCHEME_ID: usize = 583;
    pub const OBSERVATION_PURPOSE: usize = 647;
    pub const ACTUAL_CAPACITY: usize = 776;
    pub const RESOLUTION_DIGEST_DOMAIN_ID: usize = 924;
    pub const RESERVED: usize = 1_030;
}

pub mod programdata_failure_observation_v2_offset {
    pub const FINALIZED: usize = 11;
    pub const CAPACITY_POLICY: usize = 140;
    pub const FROZEN_EPOCH: usize = 372;
    pub const OBSERVATION_SCHEME_ID: usize = 500;
    pub const OBSERVATION_PURPOSE: usize = 564;
    pub const OBSERVATION_FINALIZED: usize = 605;
    pub const MISMATCH_CLASS: usize = 836;
    pub const FAILURE_DIGEST_DOMAIN_ID: usize = 905;
    pub const RESERVED: usize = 977;
}

fn validate_header<const N: usize>(
    actual_discriminator: &[u8; 8],
    expected_discriminator: &[u8; 8],
    actual_version: u8,
    expected_version: u8,
    initialized: bool,
    reserved: &[u8; N],
) -> GovernanceResult<()> {
    if actual_discriminator != expected_discriminator {
        return Err(GovernanceError::InvalidDiscriminator);
    }
    if actual_version != expected_version {
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
        Err(GovernanceError::InvalidProposalCommitment)
    } else {
        Ok(())
    }
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

fn validate_versioned_approval(
    council_version: u64,
    council_hash: &[u8; 32],
    bitset: u8,
    count: u8,
) -> GovernanceResult<()> {
    validate_approval_pair(bitset, count)?;
    let empty = council_version == 0 && *council_hash == [0; 32] && bitset == 0 && count == 0;
    let pinned = council_version != 0 && *council_hash != [0; 32] && count != 0;
    if empty || pinned {
        Ok(())
    } else {
        Err(GovernanceError::InvalidRelease1Account)
    }
}

fn validate_pinned_approval(
    council_version: u64,
    council_hash: &[u8; 32],
    bitset: u8,
    count: u8,
) -> GovernanceResult<()> {
    validate_approval_pair(bitset, count)?;
    if council_version == 0 || *council_hash == [0; 32] {
        Err(GovernanceError::InvalidRelease1Account)
    } else {
        Ok(())
    }
}

fn validate_artifact_identity(
    artifact_length: u64,
    artifact_sha256: &[u8; 32],
    artifact_root: &[u8; 32],
    artifact_scheme_id: &[u8; 32],
    chunk_size: u32,
    chunk_count: u32,
) -> GovernanceResult<()> {
    validate_artifact_identity_without_chunks(
        artifact_length,
        artifact_sha256,
        artifact_root,
        artifact_scheme_id,
    )?;
    if !is_release1_chunk_size(chunk_size)
        || chunk_count != artifact_chunk_count(artifact_length, chunk_size)?
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(())
}

fn validate_artifact_identity_without_chunks(
    artifact_length: u64,
    artifact_sha256: &[u8; 32],
    artifact_root: &[u8; 32],
    artifact_scheme_id: &[u8; 32],
) -> GovernanceResult<()> {
    if artifact_length == 0
        || artifact_length > MAX_ARTIFACT_BYTES_V1
        || *artifact_sha256 == [0; 32]
        || *artifact_root == [0; 32]
        || *artifact_scheme_id != ARTIFACT_MERKLE_SCHEME_ID
    {
        return Err(GovernanceError::InvalidMerkleParameters);
    }
    Ok(())
}

fn validate_capacity_observation(
    artifact_length: u64,
    minimum_required_capacity: u64,
    raw_data_length: u64,
    actual_capacity: u64,
) -> GovernanceResult<()> {
    let expected_raw_length = actual_capacity
        .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if minimum_required_capacity < artifact_length
        || minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        || actual_capacity < minimum_required_capacity
        || actual_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        || raw_data_length != expected_raw_length
        || raw_data_length > MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
    {
        return Err(GovernanceError::InvalidCapacityPlan);
    }
    Ok(())
}

fn validate_generation_chain(generation: u64, previous_digest: &[u8; 32]) -> GovernanceResult<()> {
    if generation == 0
        || (generation == 1 && *previous_digest != [0; 32])
        || (generation > 1 && *previous_digest == [0; 32])
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn validate_rollback_shape(proposal: &UpgradeProposalV3) -> GovernanceResult<()> {
    let rollback_present = proposal.rollback_proposal.present;
    let rollback_shape = rollback_present == proposal.rollback_buffer.present
        && rollback_present == (proposal.rollback_artifact_length != 0)
        && rollback_present == (proposal.rollback_artifact_sha256 != [0; 32])
        && rollback_present == (proposal.rollback_artifact_chunk_root != [0; 32])
        && rollback_present == (proposal.rollback_artifact_scheme_id != [0; 32]);
    if !rollback_shape {
        return Err(GovernanceError::InvalidProposalCommitment);
    }
    if rollback_present {
        validate_artifact_identity_without_chunks(
            proposal.rollback_artifact_length,
            &proposal.rollback_artifact_sha256,
            &proposal.rollback_artifact_chunk_root,
            &proposal.rollback_artifact_scheme_id,
        )?;
    }
    match proposal.proposal_class {
        ProposalClassV1::EmergencyRollback => {
            if !proposal.primary_proposal.present || rollback_present {
                return Err(GovernanceError::InvalidProposalCommitment);
            }
        }
        ProposalClassV1::RoutineUpgrade
        | ProposalClassV1::EconomicChange
        | ProposalClassV1::ConstitutionalChange => {
            if proposal.primary_proposal.present || !rollback_present {
                return Err(GovernanceError::InvalidProposalCommitment);
            }
        }
        ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
            return Err(GovernanceError::UnsupportedProposalClass);
        }
    }
    Ok(())
}

fn validate_upgrade_proposal_v3_lifecycle(proposal: &UpgradeProposalV3) -> GovernanceResult<()> {
    if proposal.proposal_id == u64::MAX
        || proposal.target_nonce == u64::MAX
        || (matches!(proposal.state, ProposalStateV2::Retired)
            && proposal.proposal_class != ProposalClassV1::EmergencyRollback)
        || (matches!(proposal.state, ProposalStateV2::SupersededByRollback)
            && proposal.proposal_class == ProposalClassV1::EmergencyRollback)
        || proposal.council_approval_count > RELEASE1_APPROVAL_THRESHOLD
        || proposal.cancellation_approval_count > RELEASE1_APPROVAL_THRESHOLD
        || proposal.unfreeze_approval_count > RELEASE1_APPROVAL_THRESHOLD
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let frozen_or_later = matches!(
        proposal.state,
        ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    if (proposal.freeze_gate_epoch != 0) != frozen_or_later
        || (proposal.frozen_slot != 0) != frozen_or_later
        || (frozen_or_later
            && (proposal.freeze_gate_epoch <= proposal.creation_gate_epoch
                || proposal.frozen_slot < proposal.not_before_slot
                || proposal.frozen_slot >= proposal.expiry_slot))
    {
        return Err(GovernanceError::InvalidProposalEpoch);
    }

    let cancellation_started = proposal.cancellation_approval_count != 0;
    if cancellation_started != (proposal.cancellation_reason_code != 0) {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    if proposal.state == ProposalStateV2::Cancelled {
        if proposal.cancellation_approval_count != RELEASE1_APPROVAL_THRESHOLD
            || proposal.terminal_reason_code != proposal.cancellation_reason_code
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
    } else if proposal.cancellation_approval_count == RELEASE1_APPROVAL_THRESHOLD {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let first_approval_present = proposal.first_approval_slot != 0;
    let council_approved_present = proposal.council_approved_slot != 0;
    if first_approval_present != (proposal.council_approval_count != 0)
        || council_approved_present
            != (proposal.council_approval_count == RELEASE1_APPROVAL_THRESHOLD)
        || (first_approval_present
            && (proposal.first_approval_slot < proposal.review_start_slot
                || proposal.first_approval_slot > proposal.review_end_slot
                || proposal.first_approval_slot >= proposal.expiry_slot))
        || (council_approved_present
            && (proposal.council_approved_slot < proposal.first_approval_slot
                || proposal.council_approved_slot > proposal.review_end_slot
                || proposal.council_approved_slot >= proposal.expiry_slot))
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }

    let governance_satisfied = proposal.governance_satisfied_slot != 0;
    let queued = proposal.queued_slot != 0;
    if (governance_satisfied
        && (!council_approved_present
            || proposal.governance_satisfied_slot >= proposal.expiry_slot))
        || (queued && (!governance_satisfied || proposal.queued_slot >= proposal.expiry_slot))
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }

    let approval_shape = match proposal.state {
        ProposalStateV2::Draft | ProposalStateV2::BufferAdopted => {
            proposal.council_approval_count == 0 && !governance_satisfied && !queued
        }
        ProposalStateV2::BufferVerified => {
            proposal.council_approval_count < RELEASE1_APPROVAL_THRESHOLD
                && !governance_satisfied
                && !queued
        }
        ProposalStateV2::CouncilApproved => {
            proposal.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                && !governance_satisfied
                && !queued
        }
        ProposalStateV2::GovernanceSatisfied => {
            proposal.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                && governance_satisfied
                && !queued
        }
        ProposalStateV2::Timelocked
        | ProposalStateV2::Retired
        | ProposalStateV2::Frozen
        | ProposalStateV2::Extended
        | ProposalStateV2::UpgradeExecuted
        | ProposalStateV2::ProgramDataVerified
        | ProposalStateV2::PoststateAccepted
        | ProposalStateV2::UnfreezeApproved
        | ProposalStateV2::Completed
        | ProposalStateV2::SupersededByRollback => {
            proposal.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                && governance_satisfied
                && queued
        }
        ProposalStateV2::Cancelled | ProposalStateV2::Expired => true,
        ProposalStateV2::TokenReviewOpen => false,
    };
    if !approval_shape {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let extension_allowed = matches!(
        proposal.state,
        ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    let extension_slot = proposal.extension_executed_slot;
    let extended_without_slot = proposal.state == ProposalStateV2::Extended && extension_slot == 0;
    let extension_in_disallowed_state = !extension_allowed && extension_slot != 0;
    let extension_outside_frozen_window = extension_slot != 0
        && (extension_slot < proposal.frozen_slot || extension_slot >= proposal.expiry_slot);
    if extended_without_slot || extension_in_disallowed_state || extension_outside_frozen_window {
        return Err(GovernanceError::InvalidProposalTiming);
    }

    let upgrade_executed = matches!(
        proposal.state,
        ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback
    );
    if (proposal.upgrade_executed_slot != 0) != upgrade_executed
        || (upgrade_executed && proposal.upgrade_executed_slot >= proposal.expiry_slot)
        || (proposal.extension_executed_slot != 0
            && upgrade_executed
            && proposal.extension_executed_slot >= proposal.upgrade_executed_slot)
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }

    let programdata_verified = matches!(
        proposal.state,
        ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
    );
    let programdata_verification_allowed =
        programdata_verified || proposal.state == ProposalStateV2::SupersededByRollback;
    if (programdata_verified && proposal.programdata_verified_slot == 0)
        || (!programdata_verification_allowed && proposal.programdata_verified_slot != 0)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let poststate_accepted = matches!(
        proposal.state,
        ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
    );
    if (proposal.poststate_accepted_slot != 0) != poststate_accepted {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let unfreeze_approved = matches!(
        proposal.state,
        ProposalStateV2::UnfreezeApproved | ProposalStateV2::Completed
    );
    let unfreeze_shape = match proposal.state {
        ProposalStateV2::PoststateAccepted => {
            proposal.unfreeze_approval_count < RELEASE1_APPROVAL_THRESHOLD
        }
        ProposalStateV2::UnfreezeApproved | ProposalStateV2::Completed => {
            proposal.unfreeze_approval_count == RELEASE1_APPROVAL_THRESHOLD
        }
        _ => proposal.unfreeze_approval_count == 0,
    };
    if !unfreeze_shape
        || (proposal.unfreeze_approved_slot != 0) != unfreeze_approved
        || (unfreeze_approved && proposal.unfreeze_approved_slot < proposal.poststate_accepted_slot)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let expected_terminal_reason = match proposal.state {
        ProposalStateV2::Completed => PROPOSAL_COMPLETED_TERMINAL_REASON_V1,
        ProposalStateV2::Cancelled => proposal.cancellation_reason_code,
        ProposalStateV2::Expired => PROPOSAL_EXPIRED_TERMINAL_REASON_V1,
        ProposalStateV2::SupersededByRollback => PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1,
        ProposalStateV2::Retired => PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1,
        _ => 0,
    };
    let terminal = expected_terminal_reason != 0;
    if (proposal.terminal_slot != 0) != terminal
        || proposal.terminal_reason_code != expected_terminal_reason
        || (proposal.state == ProposalStateV2::Cancelled
            && proposal.terminal_slot >= proposal.expiry_slot)
        || (proposal.state == ProposalStateV2::Expired
            && proposal.terminal_slot < proposal.expiry_slot)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    validate_non_decreasing_nonzero_slots(&[
        proposal.creation_slot,
        proposal.first_approval_slot,
        proposal.council_approved_slot,
        proposal.governance_satisfied_slot,
        proposal.queued_slot,
        proposal.frozen_slot,
        proposal.extension_executed_slot,
        proposal.upgrade_executed_slot,
        proposal.programdata_verified_slot,
        proposal.poststate_accepted_slot,
        proposal.unfreeze_approved_slot,
        proposal.terminal_slot,
    ])
}

fn validate_emergency_resolution_v2_lifecycle(
    resolution: &EmergencyFreezeResolutionV2,
) -> GovernanceResult<()> {
    if resolution.state == EmergencyFreezeResolutionStateV1::Cancelled {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    let first_approval_present = resolution.first_approval_slot != 0;
    let council_approved_present = resolution.council_approved_slot != 0;
    if first_approval_present != (resolution.approval_count != 0)
        || council_approved_present != (resolution.approval_count == RELEASE1_APPROVAL_THRESHOLD)
        || (first_approval_present
            && (resolution.first_approval_slot < resolution.review_start_slot
                || resolution.first_approval_slot > resolution.review_end_slot
                || resolution.first_approval_slot >= resolution.expiry_slot))
        || (council_approved_present
            && (resolution.council_approved_slot < resolution.first_approval_slot
                || resolution.council_approved_slot > resolution.review_end_slot
                || resolution.council_approved_slot >= resolution.expiry_slot))
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    let queued = resolution.queued_slot != 0;
    let approval_shape = match resolution.state {
        EmergencyFreezeResolutionStateV1::Draft => {
            resolution.approval_count < RELEASE1_APPROVAL_THRESHOLD && !queued
        }
        EmergencyFreezeResolutionStateV1::CouncilApproved => {
            resolution.approval_count == RELEASE1_APPROVAL_THRESHOLD && !queued
        }
        EmergencyFreezeResolutionStateV1::Timelocked
        | EmergencyFreezeResolutionStateV1::Executed => {
            resolution.approval_count == RELEASE1_APPROVAL_THRESHOLD && queued
        }
        EmergencyFreezeResolutionStateV1::Expired => true,
        EmergencyFreezeResolutionStateV1::Cancelled => false,
    };
    if !approval_shape
        || (queued
            && (resolution.queued_slot < resolution.council_approved_slot
                || resolution.queued_slot >= resolution.expiry_slot))
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    let expected_terminal_reason = match resolution.state {
        EmergencyFreezeResolutionStateV1::Executed => {
            EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V2
        }
        EmergencyFreezeResolutionStateV1::Expired => {
            EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V2
        }
        _ => 0,
    };
    let executed = resolution.state == EmergencyFreezeResolutionStateV1::Executed;
    let terminal = expected_terminal_reason != 0;
    if (resolution.executed_slot != 0) != executed
        || (resolution.terminal_slot != 0) != terminal
        || resolution.terminal_reason_code != expected_terminal_reason
        || (executed
            && (resolution.executed_slot < resolution.not_before_slot
                || resolution.executed_slot >= resolution.expiry_slot
                || resolution.terminal_slot != resolution.executed_slot))
        || (resolution.state == EmergencyFreezeResolutionStateV1::Expired
            && resolution.terminal_slot < resolution.expiry_slot)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    validate_non_decreasing_nonzero_slots(&[
        resolution.creation_slot,
        resolution.first_approval_slot,
        resolution.council_approved_slot,
        resolution.queued_slot,
        resolution.executed_slot,
        resolution.terminal_slot,
    ])
}

fn validate_failure_graph_shape(failure: &ProgramDataFailureObservationV2) -> GovernanceResult<()> {
    let program_graph_canonical = failure.actual_program_owner == UPGRADEABLE_LOADER_ID
        && failure.actual_program_executable
        && failure.actual_program_data_length == LOADER_V3_PROGRAM_ACCOUNT_LEN_V1
        && failure.program_header_present
        && failure.actual_linked_programdata.present
        && failure.actual_linked_programdata.value == failure.target_programdata;
    let programdata_owner_canonical = failure.actual_programdata_owner == UPGRADEABLE_LOADER_ID;
    let programdata_executable_canonical = !failure.actual_programdata_executable;
    let programdata_header_shape = if failure.programdata_header_present {
        let expected_length = failure
            .actual_capacity
            .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        failure.actual_programdata_data_length == expected_length
            && failure.actual_programdata_slot != 0
    } else {
        failure.actual_programdata_slot == 0
            && failure.actual_capacity == 0
            && !failure.actual_authority.present
    };
    if !programdata_header_shape {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    let canonical_through_program = match failure.mismatch_class {
        ProgramDataMismatchClassV2::ProgramLinkage
        | ProgramDataMismatchClassV2::ProgramOwner
        | ProgramDataMismatchClassV2::ProgramExecutable => true,
        _ => program_graph_canonical,
    };
    let canonical_through_programdata_owner = match failure.mismatch_class {
        ProgramDataMismatchClassV2::ProgramLinkage
        | ProgramDataMismatchClassV2::ProgramOwner
        | ProgramDataMismatchClassV2::ProgramExecutable
        | ProgramDataMismatchClassV2::ProgramDataOwner => true,
        _ => programdata_owner_canonical,
    };
    let canonical_through_executable = match failure.mismatch_class {
        ProgramDataMismatchClassV2::ProgramLinkage
        | ProgramDataMismatchClassV2::ProgramOwner
        | ProgramDataMismatchClassV2::ProgramExecutable
        | ProgramDataMismatchClassV2::ProgramDataOwner
        | ProgramDataMismatchClassV2::ProgramDataExecutable => true,
        _ => programdata_executable_canonical,
    };
    let header_required = !matches!(
        failure.mismatch_class,
        ProgramDataMismatchClassV2::ProgramLinkage
            | ProgramDataMismatchClassV2::ProgramOwner
            | ProgramDataMismatchClassV2::ProgramExecutable
            | ProgramDataMismatchClassV2::ProgramDataOwner
            | ProgramDataMismatchClassV2::ProgramDataExecutable
            | ProgramDataMismatchClassV2::Header
    );
    if !canonical_through_program
        || !canonical_through_programdata_owner
        || !canonical_through_executable
        || (header_required && !failure.programdata_header_present)
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }

    match failure.mismatch_class {
        ProgramDataMismatchClassV2::CapacityAboveRuntimeMaximum => {
            if failure.actual_programdata_data_length <= MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        ProgramDataMismatchClassV2::CapacityDecrease => {
            if failure.actual_capacity >= failure.minimum_required_capacity {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        ProgramDataMismatchClassV2::ArtifactPayload
        | ProgramDataMismatchClassV2::ZeroTail
        | ProgramDataMismatchClassV2::ArtifactLength
        | ProgramDataMismatchClassV2::ObservationStale
        | ProgramDataMismatchClassV2::ObservationScheme => {
            if failure.actual_programdata_data_length > MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1
                || !failure.actual_authority.present
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_non_decreasing_nonzero_slots(slots: &[u64]) -> GovernanceResult<()> {
    let mut previous = 0;
    for slot in slots.iter().copied().filter(|slot| *slot != 0) {
        if slot < previous {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        previous = slot;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use borsh::{BorshDeserialize, BorshSerialize};
    use solana_program::hash::hash;

    use super::*;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn digest(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn some(byte: u8) -> OptionalPubkeyV1 {
        OptionalPubkeyV1::some(key(byte)).unwrap()
    }

    fn proposal() -> UpgradeProposalV3 {
        UpgradeProposalV3 {
            discriminator: UPGRADE_PROPOSAL_V3_DISCRIMINATOR,
            account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V3,
            bump: 1,
            initialized: true,
            proposal_class: ProposalClassV1::RoutineUpgrade,
            state: ProposalStateV2::BufferVerified,
            creation_gate_status: GateStatusV1::Active,
            zero_tail_required: true,
            proposal_flags: 0,
            proposal_id: 1,
            target_nonce: 1,
            creation_slot: 10,
            cluster_domain: digest(1),
            controller_program: key(2),
            controller_config: key(3),
            protocol_gate: key(4),
            capacity_policy: key(5),
            capacity_policy_digest: digest(6),
            policy_version: 1,
            policy_hash: digest(7),
            creation_council_version: 1,
            creation_council_hash: digest(8),
            creation_gate_epoch: 7,
            freeze_gate_epoch: 0,
            target_program: key(9),
            target_programdata: key(10),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            authority_pda: key(11),
            canonical_spill_treasury: key(12),
            current_deployment_state: key(13),
            current_deployment_digest: digest(14),
            current_deployment_generation: 1,
            buffer_pubkey: key(15),
            buffer_loader_owner: UPGRADEABLE_LOADER_ID,
            buffer_uploader_authority: key(16),
            buffer_final_authority: key(11),
            buffer_verification: key(17),
            programdata_verification: key(18),
            artifact_length: 16_384,
            artifact_sha256: digest(19),
            artifact_chunk_merkle_root: digest(20),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            artifact_chunk_size: 16_384,
            artifact_chunk_count: 1,
            source_commit_hash: digest(21),
            source_tree_hash: digest(22),
            build_input_inventory_hash: digest(23),
            reproducible_build_receipt_hash: digest(24),
            package_receipt_hash: digest(25),
            release_intent_hash: digest(26),
            minimum_required_capacity: 16_384,
            maximum_supported_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
            programdata_observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            prestate_checkpoint: key(27),
            required_poststate_checkpoint: key(28),
            checkpoint_schema_id: digest(29),
            checkpoint_policy_hash: digest(30),
            primary_proposal: OptionalPubkeyV1::none(),
            rollback_proposal: some(31),
            rollback_buffer: some(32),
            rollback_artifact_length: 16_384,
            rollback_artifact_sha256: digest(33),
            rollback_artifact_chunk_root: digest(34),
            rollback_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            vote_requirement: VoteRequirementV1::None,
            vote_program: Pubkey::default(),
            vote_result_pda: Pubkey::default(),
            review_start_slot: 11,
            review_end_slot: 20,
            not_before_slot: 30,
            expiry_slot: 100,
            first_approval_slot: 0,
            council_approved_slot: 0,
            governance_satisfied_slot: 0,
            queued_slot: 0,
            frozen_slot: 0,
            extension_executed_slot: 0,
            upgrade_executed_slot: 0,
            programdata_verified_slot: 0,
            poststate_accepted_slot: 0,
            unfreeze_approved_slot: 0,
            terminal_slot: 0,
            council_approval_bitset: 0,
            council_approval_count: 0,
            cancellation_council_version: 0,
            cancellation_council_hash: [0; 32],
            cancellation_approval_bitset: 0,
            cancellation_approval_count: 0,
            unfreeze_council_version: 0,
            unfreeze_council_hash: [0; 32],
            unfreeze_approval_bitset: 0,
            unfreeze_approval_count: 0,
            proposal_digest_domain_id: UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID,
            proposal_digest: digest(35),
            cancellation_reason_code: 0,
            terminal_reason_code: 0,
            reserved: [0; UPGRADE_PROPOSAL_V3_RESERVED_LEN],
        }
    }

    fn verification() -> ProgramDataVerificationV2 {
        ProgramDataVerificationV2 {
            discriminator: PROGRAMDATA_VERIFICATION_V2_DISCRIMINATOR,
            account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            bump: 1,
            initialized: true,
            status: ProgramDataVerificationStatusV2::Verified,
            controller_config: key(1),
            proposal: key(2),
            proposal_digest: digest(3),
            protocol_gate: key(4),
            freeze_gate_epoch: 8,
            target_nonce: 1,
            capacity_policy: key(5),
            capacity_policy_digest: digest(6),
            current_deployment_state: key(7),
            current_deployment_digest: digest(8),
            current_deployment_generation: 1,
            target_program: key(9),
            target_programdata: key(10),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_authority: key(11),
            artifact_length: 16_384,
            artifact_sha256: digest(12),
            artifact_merkle_root: digest(13),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: 16_384,
            maximum_supported_raw_programdata_length: MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
            observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            programdata_observation: key(14),
            observation_purpose: ProgramDataObservationPurposeV1::PostUpgrade,
            observation_generation: 1,
            observation_subject_digest: digest(15),
            observation_root: digest(16),
            observation_digest: digest(17),
            observation_finalized_slot: 40,
            observed_deployed_slot: 39,
            observed_raw_data_length: 32_768 + LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
            actual_capacity: 32_768,
            observed_authority: some(11),
            zero_tail_verified: true,
            verification_generation: 1,
            previous_verification_digest: [0; 32],
            verification_digest_domain_id: PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN_ID,
            verification_digest: digest(18),
            bound_slot: 41,
            finalized_slot: 42,
            reserved: [0; PROGRAMDATA_VERIFICATION_V2_RESERVED_LEN],
        }
    }

    fn checkpoint() -> StateCheckpointV2 {
        StateCheckpointV2 {
            discriminator: STATE_CHECKPOINT_V2_DISCRIMINATOR,
            account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            bump: 1,
            initialized: true,
            phase: StateCheckpointPhaseV1::Prestate,
            controller_config: key(1),
            proposal: key(2),
            emergency_resolution: Pubkey::default(),
            subject_digest: digest(3),
            target_program: key(4),
            target_programdata: key(5),
            capacity_policy: key(6),
            capacity_policy_digest: digest(7),
            current_deployment_state: key(8),
            current_deployment_digest: digest(9),
            current_deployment_generation: 1,
            checkpoint_generation: 1,
            previous_checkpoint_digest: [0; 32],
            observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            programdata_observation: key(10),
            observation_purpose: ProgramDataObservationPurposeV1::ProposalPrestate,
            observation_generation: 1,
            observation_subject_digest: digest(11),
            observation_root: digest(12),
            observation_digest: digest(13),
            observation_finalized_slot: 40,
            gate_epoch: 8,
            target_programdata_slot: 39,
            artifact_length: 16_384,
            artifact_sha256: digest(14),
            artifact_merkle_root: digest(15),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: 16_384,
            observed_raw_data_length: 32_768 + LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
            actual_capacity: 32_768,
            observed_authority: some(16),
            program_owned_state_root: digest(17),
            program_owned_state_count: 10,
            logical_compressed_state_root: digest(18),
            logical_compressed_state_count: 11,
            semantic_custody_accounting_root: digest(19),
            hard_combined_root: digest(20),
            external_metadata_observation_root: digest(21),
            external_raw_balance_observation_root: digest(22),
            schema_identifier: digest(23),
            admitted_positive_donation_root: [0; 32],
            admitted_positive_donation_count: 0,
            forbidden_drift_count: 0,
            approval_council_version: 1,
            approval_council_hash: digest(24),
            checkpoint_digest_domain_id: STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID,
            checkpoint_digest: digest(25),
            approval_bitset: 0b00111,
            approval_count: RELEASE1_APPROVAL_THRESHOLD,
            accepted: true,
            finalized_slot: 50,
            reserved: [0; STATE_CHECKPOINT_V2_RESERVED_LEN],
        }
    }

    fn emergency_freeze_observation() -> EmergencyFreezeObservationV2 {
        EmergencyFreezeObservationV2 {
            discriminator: EMERGENCY_FREEZE_OBSERVATION_V2_DISCRIMINATOR,
            account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            bump: 1,
            initialized: true,
            finalized: true,
            controller_program: key(1),
            controller_config: key(2),
            protocol_gate: key(3),
            target_program: key(4),
            target_programdata: key(5),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_authority: key(6),
            capacity_policy: key(7),
            capacity_policy_digest: digest(8),
            current_deployment_state: key(9),
            current_deployment_digest: digest(10),
            current_deployment_generation: 1,
            trusted_artifact_length: 16_384,
            trusted_artifact_sha256: digest(11),
            trusted_artifact_merkle_root: digest(12),
            trusted_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: 16_384,
            frozen_epoch: 8,
            freeze_slot: 30,
            freeze_reason_code: 2,
            target_nonce: 1,
            actual_program_owner: UPGRADEABLE_LOADER_ID,
            actual_program_executable: true,
            actual_program_data_length: LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
            program_header_present: true,
            actual_linked_programdata: key(5),
            actual_programdata_owner: UPGRADEABLE_LOADER_ID,
            actual_programdata_executable: false,
            actual_programdata_data_length: 32_768 + LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
            programdata_header_present: true,
            deployed_programdata_slot: 29,
            actual_capacity: 32_768,
            observed_authority: some(6),
            observation_digest_domain_id: EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN_ID,
            observation_digest: digest(13),
            finalized_slot: 30,
            reserved: [0; EMERGENCY_FREEZE_OBSERVATION_V2_RESERVED_LEN],
        }
    }

    fn emergency_resolution() -> EmergencyFreezeResolutionV2 {
        EmergencyFreezeResolutionV2 {
            discriminator: EMERGENCY_FREEZE_RESOLUTION_V2_DISCRIMINATOR,
            account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            bump: 1,
            initialized: true,
            state: EmergencyFreezeResolutionStateV1::Draft,
            controller_config: key(1),
            protocol_gate: key(2),
            target_program: key(3),
            target_programdata: key(4),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            controller_authority: key(5),
            capacity_policy: key(6),
            capacity_policy_digest: digest(7),
            current_deployment_state: key(8),
            current_deployment_digest: digest(9),
            current_deployment_generation: 1,
            emergency_freeze_observation: key(10),
            emergency_freeze_observation_digest: digest(11),
            frozen_epoch: 8,
            freeze_slot: 30,
            freeze_reason_code: 2,
            resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
            creation_slot: 32,
            review_start_slot: 33,
            review_end_slot: 40,
            not_before_slot: 50,
            expiry_slot: 100,
            target_nonce: 1,
            artifact_length: 16_384,
            artifact_sha256: digest(12),
            artifact_merkle_root: digest(13),
            artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: 16_384,
            observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            programdata_observation: key(14),
            observation_purpose: ProgramDataObservationPurposeV1::EmergencyResolution,
            observation_generation: 1,
            observation_subject_digest: digest(11),
            observation_root: digest(15),
            observation_digest: digest(16),
            observation_finalized_slot: 31,
            observed_deployed_slot: 29,
            observed_raw_data_length: 32_768 + LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
            actual_capacity: 32_768,
            observed_authority: some(5),
            emergency_checkpoint: key(17),
            emergency_checkpoint_digest: [0; 32],
            approval_council_version: 1,
            approval_council_hash: digest(18),
            approval_bitset: 0,
            approval_count: 0,
            approval_threshold: RELEASE1_APPROVAL_THRESHOLD,
            resolution_digest_domain_id: EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID,
            resolution_digest: digest(19),
            first_approval_slot: 0,
            council_approved_slot: 0,
            queued_slot: 0,
            executed_slot: 0,
            terminal_slot: 0,
            terminal_reason_code: 0,
            reserved: [0; EMERGENCY_FREEZE_RESOLUTION_V2_RESERVED_LEN],
        }
    }

    fn failure_observation() -> ProgramDataFailureObservationV2 {
        ProgramDataFailureObservationV2 {
            discriminator: PROGRAMDATA_FAILURE_OBSERVATION_V2_DISCRIMINATOR,
            account_version: CAPACITY_SAFE_ACCOUNT_VERSION_V2,
            bump: 1,
            initialized: true,
            finalized: true,
            controller_config: key(1),
            protocol_gate: key(2),
            primary_proposal: key(3),
            proposal_digest: digest(4),
            capacity_policy: key(5),
            capacity_policy_digest: digest(6),
            current_deployment_state: key(7),
            current_deployment_digest: digest(8),
            current_deployment_generation: 1,
            target_program: key(9),
            target_programdata: key(10),
            upgradeable_loader: UPGRADEABLE_LOADER_ID,
            frozen_epoch: 8,
            target_nonce: 1,
            expected_artifact_length: 16_384,
            expected_artifact_sha256: digest(11),
            expected_artifact_merkle_root: digest(12),
            expected_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: 16_384,
            observation_scheme_id: PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1,
            programdata_observation: key(13),
            observation_purpose: ProgramDataObservationPurposeV1::PostUpgrade,
            observation_generation: 1,
            observation_subject_digest: digest(14),
            observation_finalized: false,
            observation_root: [0; 32],
            observation_digest: [0; 32],
            actual_program_owner: UPGRADEABLE_LOADER_ID,
            actual_program_executable: true,
            actual_program_data_length: LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
            program_header_present: true,
            actual_linked_programdata: some(10),
            actual_programdata_owner: UPGRADEABLE_LOADER_ID,
            actual_programdata_executable: false,
            actual_programdata_data_length: 32_768 + LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
            programdata_header_present: true,
            actual_programdata_slot: 39,
            actual_capacity: 32_768,
            actual_authority: some(15),
            mismatch_class: ProgramDataMismatchClassV2::ZeroTail,
            failing_chunk_index: 2,
            expected_leaf_hash: digest(16),
            actual_leaf_hash: digest(17),
            failure_digest_domain_id: PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN_ID,
            failure_digest: digest(18),
            finalized_slot: 40,
            reserved: [0; PROGRAMDATA_FAILURE_OBSERVATION_V2_RESERVED_LEN],
        }
    }

    macro_rules! assert_strict_account {
        ($value:expr, $type:ty) => {{
            let value = $value;
            value.validate_schema().unwrap();
            let encoded = value.try_to_vec().unwrap();
            assert_eq!(encoded.len(), <$type>::LEN);
            assert_eq!(<$type>::from_bytes_strict(&encoded).unwrap(), value);
            assert!(<$type>::from_bytes_strict(&encoded[..encoded.len() - 1]).is_err());
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(<$type>::from_bytes_strict(&trailing).is_err());
            assert!(<$type>::try_from_slice(&trailing).is_err());
        }};
    }

    #[test]
    fn exact_lengths_round_trip_and_strict_decoders_reject_size_drift() {
        assert_strict_account!(proposal(), UpgradeProposalV3);
        assert_strict_account!(verification(), ProgramDataVerificationV2);
        assert_strict_account!(checkpoint(), StateCheckpointV2);
        assert_strict_account!(emergency_freeze_observation(), EmergencyFreezeObservationV2);
        assert_strict_account!(emergency_resolution(), EmergencyFreezeResolutionV2);
        assert_strict_account!(failure_observation(), ProgramDataFailureObservationV2);
    }

    #[test]
    fn boxed_upgrade_proposal_decoder_matches_canonical_borsh_and_rejects_size_drift() {
        let expected = proposal();
        let encoded = expected.try_to_vec().unwrap();
        assert_eq!(
            *UpgradeProposalV3::from_bytes_boxed_strict(&encoded).unwrap(),
            expected
        );
        assert!(UpgradeProposalV3::from_bytes_boxed_strict(&encoded[..encoded.len() - 1]).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(UpgradeProposalV3::from_bytes_boxed_strict(&trailing).is_err());
    }

    #[test]
    fn discriminators_domains_and_observation_scheme_are_frozen() {
        let discriminators = [
            UPGRADE_PROPOSAL_V3_DISCRIMINATOR,
            PROGRAMDATA_VERIFICATION_V2_DISCRIMINATOR,
            STATE_CHECKPOINT_V2_DISCRIMINATOR,
            EMERGENCY_FREEZE_OBSERVATION_V2_DISCRIMINATOR,
            EMERGENCY_FREEZE_RESOLUTION_V2_DISCRIMINATOR,
            PROGRAMDATA_FAILURE_OBSERVATION_V2_DISCRIMINATOR,
        ];
        assert_eq!(
            discriminators.iter().collect::<BTreeSet<_>>().len(),
            discriminators.len()
        );
        assert_eq!(
            hash(UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN).to_bytes(),
            UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            hash(PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN).to_bytes(),
            PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            hash(STATE_CHECKPOINT_V2_DIGEST_DOMAIN).to_bytes(),
            STATE_CHECKPOINT_V2_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            hash(EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN).to_bytes(),
            EMERGENCY_FREEZE_OBSERVATION_V2_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            hash(EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN).to_bytes(),
            EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            hash(PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN).to_bytes(),
            PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            proposal().programdata_observation_scheme_id,
            PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
        );
    }

    #[test]
    fn enums_are_strict_one_byte_and_unknown_values_fail() {
        assert_eq!(
            ProgramDataVerificationStatusV2::ObservationBound
                .try_to_vec()
                .unwrap(),
            [0]
        );
        assert_eq!(
            ProgramDataVerificationStatusV2::Verified
                .try_to_vec()
                .unwrap(),
            [1]
        );
        assert!(ProgramDataVerificationStatusV2::try_from_slice(&[2]).is_err());
        for value in 0u8..=13 {
            assert_eq!(
                ProgramDataMismatchClassV2::try_from_slice(&[value])
                    .unwrap()
                    .try_to_vec()
                    .unwrap(),
                [value]
            );
        }
        assert!(ProgramDataMismatchClassV2::try_from_slice(&[14]).is_err());
        assert!(ProgramDataMismatchClassV2::try_from_slice(&[]).is_err());
        assert!(ProgramDataMismatchClassV2::try_from_slice(&[0, 0]).is_err());
    }

    #[test]
    fn every_account_rejects_nonzero_reserved_bytes() {
        let mut proposal = proposal();
        proposal.reserved[0] = 1;
        assert_eq!(
            proposal.validate_schema(),
            Err(GovernanceError::NonzeroReserved)
        );

        let mut verification = verification();
        verification.reserved[0] = 1;
        assert_eq!(
            verification.validate_schema(),
            Err(GovernanceError::NonzeroReserved)
        );

        let mut checkpoint = checkpoint();
        checkpoint.reserved[0] = 1;
        assert_eq!(
            checkpoint.validate_schema(),
            Err(GovernanceError::NonzeroReserved)
        );

        let mut freeze = emergency_freeze_observation();
        freeze.reserved[0] = 1;
        assert_eq!(
            freeze.validate_schema(),
            Err(GovernanceError::NonzeroReserved)
        );

        let mut resolution = emergency_resolution();
        resolution.reserved[0] = 1;
        assert_eq!(
            resolution.validate_schema(),
            Err(GovernanceError::NonzeroReserved)
        );

        let mut failure = failure_observation();
        failure.reserved[0] = 1;
        assert_eq!(
            failure.validate_schema(),
            Err(GovernanceError::NonzeroReserved)
        );
    }

    #[test]
    fn strict_account_decoders_reject_unknown_enums_and_noncanonical_bools() {
        let mut proposal = proposal().try_to_vec().unwrap();
        proposal[upgrade_proposal_v3_offset::STATE] = 18;
        assert_eq!(
            UpgradeProposalV3::from_bytes_strict(&proposal),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut verification = verification().try_to_vec().unwrap();
        verification[programdata_verification_v2_offset::STATUS] = 2;
        assert_eq!(
            ProgramDataVerificationV2::from_bytes_strict(&verification),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut checkpoint = checkpoint().try_to_vec().unwrap();
        checkpoint[state_checkpoint_v2_offset::PHASE] = 3;
        assert_eq!(
            StateCheckpointV2::from_bytes_strict(&checkpoint),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut freeze = emergency_freeze_observation().try_to_vec().unwrap();
        freeze[emergency_freeze_observation_v2_offset::FINALIZED] = 2;
        assert_eq!(
            EmergencyFreezeObservationV2::from_bytes_strict(&freeze),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut resolution = emergency_resolution().try_to_vec().unwrap();
        resolution[emergency_freeze_resolution_v2_offset::OBSERVATION_PURPOSE] = 7;
        assert_eq!(
            EmergencyFreezeResolutionV2::from_bytes_strict(&resolution),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut failure = failure_observation().try_to_vec().unwrap();
        failure[programdata_failure_observation_v2_offset::MISMATCH_CLASS] = 14;
        assert_eq!(
            ProgramDataFailureObservationV2::from_bytes_strict(&failure),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn runtime_maximum_capacity_is_valid_and_one_byte_more_is_rejected() {
        let mut verification = verification();
        verification.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1;
        verification.observed_raw_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
        verification.validate_schema().unwrap();

        verification.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1 + 1;
        verification.observed_raw_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        assert_eq!(
            verification.validate_schema(),
            Err(GovernanceError::InvalidCapacityPlan)
        );
    }

    #[test]
    fn zero_only_capacity_growth_refreshes_evidence_without_changing_proposal_bytes() {
        let proposal = proposal();
        proposal.validate_schema().unwrap();
        let immutable_proposal_bytes = proposal.try_to_vec().unwrap();

        let first = verification();
        first.validate_schema().unwrap();
        let mut refreshed = first.clone();
        refreshed.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1;
        refreshed.observed_raw_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
        refreshed.observation_generation = 2;
        refreshed.programdata_observation = key(90);
        refreshed.observation_root = digest(91);
        refreshed.observation_digest = digest(92);
        refreshed.verification_generation = 2;
        refreshed.previous_verification_digest = first.verification_digest;
        refreshed.verification_digest = digest(93);
        refreshed.observed_deployed_slot += 1;
        refreshed.observation_finalized_slot += 1;
        refreshed.bound_slot += 1;
        refreshed.finalized_slot += 1;
        refreshed.validate_schema().unwrap();

        assert_eq!(proposal.try_to_vec().unwrap(), immutable_proposal_bytes);
        assert_ne!(first.observation_digest, refreshed.observation_digest);
        assert!(refreshed.actual_capacity > first.actual_capacity);
    }

    #[test]
    fn generation_chain_and_scheme_drift_fail_closed() {
        let mut verification = verification();
        verification.verification_generation = 2;
        assert_eq!(
            verification.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
        verification.previous_verification_digest = digest(50);
        verification.validate_schema().unwrap();

        verification.observation_scheme_id = digest(51);
        assert_eq!(
            verification.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut proposal = proposal();
        proposal.programdata_observation_scheme_id = digest(52);
        assert_eq!(
            proposal.validate_schema(),
            Err(GovernanceError::InvalidProposalCommitment)
        );
    }

    #[test]
    fn proposal_defaults_approval_counts_and_timing_are_strict() {
        let mut value = proposal();
        value.current_deployment_state = Pubkey::default();
        assert_eq!(value.validate_schema(), Err(GovernanceError::DefaultPubkey));

        let mut value = proposal();
        value.council_approval_bitset = 0b00011;
        value.council_approval_count = 1;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::ApprovalCountMismatch)
        );

        let mut value = proposal();
        value.target_nonce = u64::MAX;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut value = proposal();
        value.review_end_slot = value.review_start_slot;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidProposalTiming)
        );
    }

    #[test]
    fn verified_programdata_requires_zero_tail_and_controller_authority() {
        let mut value = verification();
        value.zero_tail_verified = false;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut value = verification();
        value.observed_authority = some(99);
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut value = verification();
        value.status = ProgramDataVerificationStatusV2::ObservationBound;
        value.zero_tail_verified = false;
        value.finalized_slot = 0;
        value.validate_schema().unwrap();
    }

    #[test]
    fn emergency_freeze_is_size_independent_and_resolution_binds_later_observation() {
        let mut freeze = emergency_freeze_observation();
        freeze.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1;
        freeze.actual_programdata_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
        freeze.validate_schema().unwrap();

        let resolution = emergency_resolution();
        resolution.validate_schema().unwrap();
        assert_eq!(
            resolution.observation_subject_digest,
            resolution.emergency_freeze_observation_digest
        );

        let mut stale = resolution;
        stale.observation_subject_digest = digest(99);
        assert_eq!(
            stale.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn checkpoint_phase_purpose_generation_and_quorum_are_not_replayable() {
        let mut value = checkpoint();
        value.observation_purpose = ProgramDataObservationPurposeV1::PostUpgrade;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut value = checkpoint();
        value.checkpoint_generation = 2;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
        value.previous_checkpoint_digest = digest(70);
        value.validate_schema().unwrap();

        let mut value = checkpoint();
        value.approval_bitset = 0b00011;
        value.approval_count = 2;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
        value.accepted = false;
        value.validate_schema().unwrap();
    }

    #[test]
    fn checkpoint_acceptance_tracks_exact_three_seat_threshold_without_rekeying_content() {
        let complete = checkpoint();
        complete.validate_schema().unwrap();

        let mut pending = complete.clone();
        pending.approval_bitset = 0;
        pending.approval_count = 0;
        pending.accepted = false;
        pending.validate_schema().unwrap();

        pending.approval_bitset = 0b00011;
        pending.approval_count = 2;
        pending.validate_schema().unwrap();

        pending.approval_bitset = 0b00111;
        pending.approval_count = 3;
        assert_eq!(
            pending.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
        pending.accepted = true;
        pending.validate_schema().unwrap();

        let mut blocked = complete;
        blocked.forbidden_drift_count = 1;
        blocked.approval_bitset = 0;
        blocked.approval_count = 0;
        blocked.accepted = false;
        blocked.validate_schema().unwrap();
        blocked.approval_bitset = 1;
        blocked.approval_count = 1;
        assert_eq!(
            blocked.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn failure_evidence_has_strict_leaf_and_capacity_shapes() {
        let mut value = failure_observation();
        value.expected_leaf_hash = value.actual_leaf_hash;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );

        let mut value = failure_observation();
        value.mismatch_class = ProgramDataMismatchClassV2::CapacityAboveRuntimeMaximum;
        value.failing_chunk_index = NO_FAILING_CHUNK_INDEX_V1;
        value.expected_leaf_hash = [0; 32];
        value.actual_leaf_hash = [0; 32];
        value.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1 + 1;
        value.actual_programdata_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        value.validate_schema().unwrap();

        value.actual_capacity = MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1;
        value.actual_programdata_data_length = MAX_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
        assert_eq!(
            value.validate_schema(),
            Err(GovernanceError::InvalidRelease1Account)
        );
    }

    #[test]
    fn offset_constants_match_frozen_wire_positions() {
        let proposal = proposal().try_to_vec().unwrap();
        assert_eq!(
            &proposal[upgrade_proposal_v3_offset::PROPOSAL_DIGEST_DOMAIN_ID
                ..upgrade_proposal_v3_offset::PROPOSAL_DIGEST_DOMAIN_ID + 32],
            UPGRADE_PROPOSAL_V3_DIGEST_DOMAIN_ID
        );
        assert_eq!(
            &proposal[upgrade_proposal_v3_offset::OBSERVATION_SCHEME_ID
                ..upgrade_proposal_v3_offset::OBSERVATION_SCHEME_ID + 32],
            PROGRAMDATA_OBSERVATION_MERKLE_SCHEME_ID_V1
        );
        let verification = verification().try_to_vec().unwrap();
        assert_eq!(
            verification[programdata_verification_v2_offset::OBSERVATION_PURPOSE],
            ProgramDataObservationPurposeV1::PostUpgrade as u8
        );
        let checkpoint = checkpoint().try_to_vec().unwrap();
        assert_eq!(checkpoint[state_checkpoint_v2_offset::ACCEPTED], 1);
        let resolution = emergency_resolution().try_to_vec().unwrap();
        assert_eq!(
            &resolution[emergency_freeze_resolution_v2_offset::RESOLUTION_DIGEST_DOMAIN_ID
                ..emergency_freeze_resolution_v2_offset::RESOLUTION_DIGEST_DOMAIN_ID + 32],
            EMERGENCY_FREEZE_RESOLUTION_V2_DIGEST_DOMAIN_ID
        );
    }
}
