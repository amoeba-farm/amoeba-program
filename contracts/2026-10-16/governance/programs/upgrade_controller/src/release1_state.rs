//! Fixed-width Release 1 governance account schemas.
//!
//! These types are data contracts only.  No processor in this module creates,
//! mutates, or executes them.  `UpgradeProposalV1` and every other existing V1
//! byte remain defined in `state.rs` without reinterpretation.

use std::io::{Error, ErrorKind, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::{
    artifact_merkle::{
        artifact_chunk_count, is_release1_chunk_size, ARTIFACT_MERKLE_SCHEME_ID,
        MAX_ARTIFACT_BYTES_V1, MAX_ARTIFACT_CHUNKS_V1,
    },
    council::VALID_APPROVAL_MASK,
    pda::UPGRADEABLE_LOADER_ID,
    state::{GateStatusV1, OptionalPubkeyV1, ProposalClassV1, VoteRequirementV1},
    GovernanceError, GovernanceResult,
};

pub const UPGRADE_PROPOSAL_V2_DISCRIMINATOR: [u8; 8] = *b"AGVPRP02";
pub const BUFFER_VERIFICATION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVBFV01";
pub const PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVPDV01";
pub const STATE_CHECKPOINT_V1_DISCRIMINATOR: [u8; 8] = *b"AGVCKP01";
pub const COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR: [u8; 8] = *b"AGVROT01";
pub const EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVEFR01";
pub const EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVEFO01";
pub const PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVPDF01";
pub const CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR: [u8; 8] = *b"AGVATT01";

pub const ACCOUNT_VERSION_V2: u8 = 2;
pub const RELEASE1_ACCOUNT_VERSION_V1: u8 = 1;
pub const RELEASE1_APPROVAL_THRESHOLD: u8 = 3;
pub const BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1: u16 = 1;
pub const PROPOSAL_EXPIRED_TERMINAL_REASON_V1: u16 = 1;
pub const PROPOSAL_COMPLETED_TERMINAL_REASON_V1: u16 = 2;
pub const PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1: u16 = 3;
pub const PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1: u16 = 4;
pub const COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1: u16 = 1;
pub const COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1: u16 = 2;
pub const EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1: u16 = 1;
pub const EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1: u16 = 2;

pub const UPGRADE_PROPOSAL_V2_RESERVED_LEN: usize = 146;
pub const BUFFER_VERIFICATION_V1_RESERVED_LEN: usize = 72;
pub const PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN: usize = 119;
pub const STATE_CHECKPOINT_V1_RESERVED_LEN: usize = 37;
pub const COUNCIL_ROTATION_PROPOSAL_V1_RESERVED_LEN: usize = 84;
pub const EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN: usize = 100;
pub const EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN: usize = 19;
pub const PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN: usize = 24;
pub const CHECKPOINT_ATTESTATION_V1_RESERVED_LEN: usize = 27;
pub const VERIFICATION_BITMAP_BYTES_V1: usize = MAX_ARTIFACT_CHUNKS_V1 / 8;
pub const NO_FAILING_CHUNK_INDEX_V1: u32 = u32::MAX;
pub const LOADER_V3_PROGRAM_ACCOUNT_LEN_V1: u64 = 36;
pub const LOADER_V3_PROGRAMDATA_METADATA_LEN_V1: u64 = 45;
/// Largest exact Loader-v3 ProgramData account that the composed guardian and
/// failure-observation paths hash with a conservative actual-SBF margin.
pub const MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1: u64 = 1_572_909;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProposalStateV2 {
    Draft = 0,
    BufferAdopted = 1,
    BufferVerified = 2,
    CouncilApproved = 3,
    TokenReviewOpen = 4,
    GovernanceSatisfied = 5,
    Timelocked = 6,
    Frozen = 7,
    Extended = 8,
    UpgradeExecuted = 9,
    ProgramDataVerified = 10,
    PoststateAccepted = 11,
    UnfreezeApproved = 12,
    Completed = 13,
    Cancelled = 14,
    Expired = 15,
    SupersededByRollback = 16,
    Retired = 17,
}
fixed_u8_enum_borsh!(ProposalStateV2 {
    Draft = 0,
    BufferAdopted = 1,
    BufferVerified = 2,
    CouncilApproved = 3,
    TokenReviewOpen = 4,
    GovernanceSatisfied = 5,
    Timelocked = 6,
    Frozen = 7,
    Extended = 8,
    UpgradeExecuted = 9,
    ProgramDataVerified = 10,
    PoststateAccepted = 11,
    UnfreezeApproved = 12,
    Completed = 13,
    Cancelled = 14,
    Expired = 15,
    SupersededByRollback = 16,
    Retired = 17,
});

impl Default for ProposalStateV2 {
    fn default() -> Self {
        Self::Draft
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BufferVerificationStatusV1 {
    Adopted = 0,
    Verifying = 1,
    ReadyToFinalize = 2,
    Verified = 3,
    ConsumedByUpgrade = 4,
    ClosedAbandoned = 5,
}
fixed_u8_enum_borsh!(BufferVerificationStatusV1 {
    Adopted = 0,
    Verifying = 1,
    ReadyToFinalize = 2,
    Verified = 3,
    ConsumedByUpgrade = 4,
    ClosedAbandoned = 5,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgramDataVerificationStatusV1 {
    Verifying = 0,
    ReadyToFinalize = 1,
    Verified = 2,
}
fixed_u8_enum_borsh!(ProgramDataVerificationStatusV1 {
    Verifying = 0,
    ReadyToFinalize = 1,
    Verified = 2,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StateCheckpointPhaseV1 {
    Prestate = 0,
    Poststate = 1,
    Emergency = 2,
}
fixed_u8_enum_borsh!(StateCheckpointPhaseV1 {
    Prestate = 0,
    Poststate = 1,
    Emergency = 2,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CouncilRotationStateV1 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Activated = 3,
    Cancelled = 4,
    Expired = 5,
}
fixed_u8_enum_borsh!(CouncilRotationStateV1 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Activated = 3,
    Cancelled = 4,
    Expired = 5,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EmergencyFreezeResolutionStateV1 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Executed = 3,
    Cancelled = 4,
    Expired = 5,
}
fixed_u8_enum_borsh!(EmergencyFreezeResolutionStateV1 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Executed = 3,
    Cancelled = 4,
    Expired = 5,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EmergencyFreezeResolutionKindV1 {
    ResumeWithoutUpgrade = 0,
}
fixed_u8_enum_borsh!(EmergencyFreezeResolutionKindV1 {
    ResumeWithoutUpgrade = 0,
});

/// The first mechanically observed mismatch that prevents ProgramData
/// verification from finalizing.  This is evidence, not a recovery action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgramDataMismatchClassV1 {
    Header = 0,
    Authority = 1,
    Capacity = 2,
    PayloadLeaf = 3,
    ZeroTail = 4,
}
fixed_u8_enum_borsh!(ProgramDataMismatchClassV1 {
    Header = 0,
    Authority = 1,
    Capacity = 2,
    PayloadLeaf = 3,
    ZeroTail = 4,
});

/// Release 1 code-upgrade proposal.  The field order is the wire order.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
pub struct UpgradeProposalV2 {
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
    pub buffer_pubkey: Pubkey,
    pub buffer_loader_owner: Pubkey,
    pub buffer_uploader_authority: Pubkey,
    pub buffer_final_authority: Pubkey,
    pub buffer_verification: Pubkey,
    pub programdata_verification: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_chunk_merkle_root: [u8; 32],
    pub chunk_hash_domain: [u8; 32],
    pub chunk_size: u32,
    pub chunk_count: u32,
    pub source_commit_hash: [u8; 32],
    pub source_tree_hash: [u8; 32],
    pub build_input_inventory_hash: [u8; 32],
    pub reproducible_build_receipt_hash: [u8; 32],
    pub package_receipt_hash: [u8; 32],
    pub release_intent_hash: [u8; 32],
    pub expected_execution_pre_payload_hash: [u8; 32],
    pub expected_execution_pre_chunk_root: [u8; 32],
    pub current_raw_programdata_hash: [u8; 32],
    pub deployed_slot: u64,
    pub current_capacity: u64,
    pub extension_delta: u64,
    pub expected_post_capacity: u64,
    pub prestate_checkpoint: Pubkey,
    pub required_poststate_checkpoint: Pubkey,
    pub checkpoint_schema_id: [u8; 32],
    pub checkpoint_policy_hash: [u8; 32],
    pub primary_proposal: OptionalPubkeyV1,
    pub rollback_proposal: OptionalPubkeyV1,
    pub rollback_buffer: OptionalPubkeyV1,
    pub rollback_artifact_sha256: [u8; 32],
    pub rollback_artifact_chunk_root: [u8; 32],
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
    pub proposal_digest: [u8; 32],
    pub cancellation_reason_code: u16,
    pub terminal_reason_code: u16,
    pub reserved: [u8; UPGRADE_PROPOSAL_V2_RESERVED_LEN],
}

impl Default for UpgradeProposalV2 {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            account_version: 0,
            bump: 0,
            initialized: false,
            proposal_class: ProposalClassV1::default(),
            state: ProposalStateV2::default(),
            creation_gate_status: GateStatusV1::default(),
            zero_tail_required: false,
            proposal_flags: 0,
            proposal_id: 0,
            target_nonce: 0,
            creation_slot: 0,
            cluster_domain: [0; 32],
            controller_program: Pubkey::default(),
            controller_config: Pubkey::default(),
            protocol_gate: Pubkey::default(),
            policy_version: 0,
            policy_hash: [0; 32],
            creation_council_version: 0,
            creation_council_hash: [0; 32],
            creation_gate_epoch: 0,
            freeze_gate_epoch: 0,
            target_program: Pubkey::default(),
            target_programdata: Pubkey::default(),
            upgradeable_loader: Pubkey::default(),
            authority_pda: Pubkey::default(),
            canonical_spill_treasury: Pubkey::default(),
            buffer_pubkey: Pubkey::default(),
            buffer_loader_owner: Pubkey::default(),
            buffer_uploader_authority: Pubkey::default(),
            buffer_final_authority: Pubkey::default(),
            buffer_verification: Pubkey::default(),
            programdata_verification: Pubkey::default(),
            artifact_length: 0,
            artifact_sha256: [0; 32],
            artifact_chunk_merkle_root: [0; 32],
            chunk_hash_domain: [0; 32],
            chunk_size: 0,
            chunk_count: 0,
            source_commit_hash: [0; 32],
            source_tree_hash: [0; 32],
            build_input_inventory_hash: [0; 32],
            reproducible_build_receipt_hash: [0; 32],
            package_receipt_hash: [0; 32],
            release_intent_hash: [0; 32],
            expected_execution_pre_payload_hash: [0; 32],
            expected_execution_pre_chunk_root: [0; 32],
            current_raw_programdata_hash: [0; 32],
            deployed_slot: 0,
            current_capacity: 0,
            extension_delta: 0,
            expected_post_capacity: 0,
            prestate_checkpoint: Pubkey::default(),
            required_poststate_checkpoint: Pubkey::default(),
            checkpoint_schema_id: [0; 32],
            checkpoint_policy_hash: [0; 32],
            primary_proposal: OptionalPubkeyV1::default(),
            rollback_proposal: OptionalPubkeyV1::default(),
            rollback_buffer: OptionalPubkeyV1::default(),
            rollback_artifact_sha256: [0; 32],
            rollback_artifact_chunk_root: [0; 32],
            vote_requirement: VoteRequirementV1::default(),
            vote_program: Pubkey::default(),
            vote_result_pda: Pubkey::default(),
            review_start_slot: 0,
            review_end_slot: 0,
            not_before_slot: 0,
            expiry_slot: 0,
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
            proposal_digest: [0; 32],
            cancellation_reason_code: 0,
            terminal_reason_code: 0,
            reserved: [0; UPGRADE_PROPOSAL_V2_RESERVED_LEN],
        }
    }
}

impl BorshDeserialize for UpgradeProposalV2 {
    #[inline(never)]
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // Borsh's derived struct literal keeps every decoded field temporary
        // live until the 1,792-byte proposal is assembled. Under SBPF-v0 that
        // produced a 4,160-byte frame. Populate one proposal allocation in
        // wire order instead; this is byte-identical Borsh and retains strict
        // enum/bool/truncation handling from each field decoder.
        let mut value = Self::default();
        macro_rules! read_field {
            ($field:ident) => {
                value.$field = BorshDeserialize::deserialize_reader(reader)?;
            };
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
        read_field!(buffer_pubkey);
        read_field!(buffer_loader_owner);
        read_field!(buffer_uploader_authority);
        read_field!(buffer_final_authority);
        read_field!(buffer_verification);
        read_field!(programdata_verification);
        read_field!(artifact_length);
        read_field!(artifact_sha256);
        read_field!(artifact_chunk_merkle_root);
        read_field!(chunk_hash_domain);
        read_field!(chunk_size);
        read_field!(chunk_count);
        read_field!(source_commit_hash);
        read_field!(source_tree_hash);
        read_field!(build_input_inventory_hash);
        read_field!(reproducible_build_receipt_hash);
        read_field!(package_receipt_hash);
        read_field!(release_intent_hash);
        read_field!(expected_execution_pre_payload_hash);
        read_field!(expected_execution_pre_chunk_root);
        read_field!(current_raw_programdata_hash);
        read_field!(deployed_slot);
        read_field!(current_capacity);
        read_field!(extension_delta);
        read_field!(expected_post_capacity);
        read_field!(prestate_checkpoint);
        read_field!(required_poststate_checkpoint);
        read_field!(checkpoint_schema_id);
        read_field!(checkpoint_policy_hash);
        read_field!(primary_proposal);
        read_field!(rollback_proposal);
        read_field!(rollback_buffer);
        read_field!(rollback_artifact_sha256);
        read_field!(rollback_artifact_chunk_root);
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
        read_field!(proposal_digest);
        read_field!(cancellation_reason_code);
        read_field!(terminal_reason_code);
        read_field!(reserved);
        Ok(value)
    }
}

impl UpgradeProposalV2 {
    pub const LEN: usize = 1_792;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &UPGRADE_PROPOSAL_V2_DISCRIMINATOR,
            self.account_version,
            ACCOUNT_VERSION_V2,
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
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.protocol_gate,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.authority_pda,
            self.canonical_spill_treasury,
            self.buffer_pubkey,
            self.buffer_loader_owner,
            self.buffer_uploader_authority,
            self.buffer_final_authority,
            self.buffer_verification,
            self.programdata_verification,
            self.prestate_checkpoint,
            self.required_poststate_checkpoint,
        ])?;
        if self.cluster_domain == [0; 32]
            || self.policy_version == 0
            || self.policy_hash == [0; 32]
            || self.creation_council_version == 0
            || self.creation_council_hash == [0; 32]
            || self.creation_gate_epoch == 0
            || self.proposal_id == 0
            || self.target_nonce == 0
            || self.creation_slot == 0
            || self.artifact_length == 0
            || self.artifact_length > MAX_ARTIFACT_BYTES_V1
            || self.artifact_sha256 == [0; 32]
            || self.artifact_chunk_merkle_root == [0; 32]
            || self.chunk_hash_domain != ARTIFACT_MERKLE_SCHEME_ID
            || !is_release1_chunk_size(self.chunk_size)
            || self.chunk_count != artifact_chunk_count(self.artifact_length, self.chunk_size)?
            || self.buffer_loader_owner != self.upgradeable_loader
            || self.buffer_final_authority != self.authority_pda
            || self.checkpoint_schema_id == [0; 32]
            || self.checkpoint_policy_hash == [0; 32]
            || self.proposal_digest == [0; 32]
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        let has_frozen_epoch = self.freeze_gate_epoch != 0;
        let requires_frozen_epoch = matches!(
            self.state,
            ProposalStateV2::Frozen
                | ProposalStateV2::Extended
                | ProposalStateV2::UpgradeExecuted
                | ProposalStateV2::ProgramDataVerified
                | ProposalStateV2::PoststateAccepted
                | ProposalStateV2::UnfreezeApproved
                | ProposalStateV2::Completed
                | ProposalStateV2::SupersededByRollback
        );
        if has_frozen_epoch != requires_frozen_epoch {
            return Err(GovernanceError::InvalidProposalEpoch);
        }
        for commitment in [
            self.source_commit_hash,
            self.source_tree_hash,
            self.build_input_inventory_hash,
            self.reproducible_build_receipt_hash,
            self.package_receipt_hash,
            self.release_intent_hash,
            self.expected_execution_pre_payload_hash,
            self.expected_execution_pre_chunk_root,
        ] {
            if commitment == [0; 32] {
                return Err(GovernanceError::InvalidProposalCommitment);
            }
        }
        if self
            .current_capacity
            .checked_add(self.extension_delta)
            .ok_or(GovernanceError::ArithmeticOverflow)?
            != self.expected_post_capacity
            || self.current_capacity == 0
            || self.artifact_length > self.expected_post_capacity
            || self.expected_post_capacity > MAX_ARTIFACT_BYTES_V1
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
        let rollback_present = self.rollback_proposal.present;
        if rollback_present != self.rollback_buffer.present
            || rollback_present != (self.rollback_artifact_sha256 != [0; 32])
            || rollback_present != (self.rollback_artifact_chunk_root != [0; 32])
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        match self.proposal_class {
            ProposalClassV1::EmergencyRollback => {
                if !self.primary_proposal.present
                    || rollback_present
                    || self.deployed_slot != 0
                    || self.current_raw_programdata_hash != [0; 32]
                {
                    return Err(GovernanceError::InvalidProposalCommitment);
                }
            }
            ProposalClassV1::RoutineUpgrade
            | ProposalClassV1::EconomicChange
            | ProposalClassV1::ConstitutionalChange => {
                if self.primary_proposal.present
                    || !rollback_present
                    || self.deployed_slot == 0
                    || self.current_raw_programdata_hash == [0; 32]
                {
                    return Err(GovernanceError::InvalidProposalCommitment);
                }
            }
            ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
                return Err(GovernanceError::UnsupportedProposalClass);
            }
        }
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
        self.validate_lifecycle_shape()
    }

    fn validate_lifecycle_shape(&self) -> GovernanceResult<()> {
        if matches!(self.state, ProposalStateV2::Retired)
            && !matches!(self.proposal_class, ProposalClassV1::EmergencyRollback)
            || matches!(self.state, ProposalStateV2::SupersededByRollback)
                && matches!(self.proposal_class, ProposalClassV1::EmergencyRollback)
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if self.council_approval_count > RELEASE1_APPROVAL_THRESHOLD
            || self.cancellation_approval_count > RELEASE1_APPROVAL_THRESHOLD
            || self.unfreeze_approval_count > RELEASE1_APPROVAL_THRESHOLD
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let cancellation_started = self.cancellation_approval_count != 0;
        if cancellation_started != (self.cancellation_reason_code != 0) {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if matches!(self.state, ProposalStateV2::Cancelled) {
            if self.cancellation_approval_count != RELEASE1_APPROVAL_THRESHOLD
                || self.terminal_reason_code != self.cancellation_reason_code
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        } else if self.cancellation_approval_count == RELEASE1_APPROVAL_THRESHOLD {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let first_approval_present = self.first_approval_slot != 0;
        let council_approved_present = self.council_approved_slot != 0;
        if first_approval_present != (self.council_approval_count != 0)
            || council_approved_present
                != (self.council_approval_count == RELEASE1_APPROVAL_THRESHOLD)
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if first_approval_present
            && (self.first_approval_slot < self.review_start_slot
                || self.first_approval_slot > self.review_end_slot
                || self.first_approval_slot >= self.expiry_slot)
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        if council_approved_present
            && (self.council_approved_slot < self.first_approval_slot
                || self.council_approved_slot > self.review_end_slot
                || self.council_approved_slot >= self.expiry_slot)
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }

        let governance_satisfied = self.governance_satisfied_slot != 0;
        let queued = self.queued_slot != 0;
        if governance_satisfied
            && (!council_approved_present || self.governance_satisfied_slot >= self.expiry_slot)
            || queued && (!governance_satisfied || self.queued_slot >= self.expiry_slot)
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let prefreeze_shape = match self.state {
            ProposalStateV2::Draft | ProposalStateV2::BufferAdopted => {
                self.council_approval_count == 0 && !governance_satisfied && !queued
            }
            ProposalStateV2::BufferVerified => {
                self.council_approval_count < RELEASE1_APPROVAL_THRESHOLD
                    && !governance_satisfied
                    && !queued
            }
            ProposalStateV2::CouncilApproved => {
                self.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                    && !governance_satisfied
                    && !queued
            }
            ProposalStateV2::GovernanceSatisfied => {
                self.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                    && governance_satisfied
                    && !queued
            }
            ProposalStateV2::Timelocked | ProposalStateV2::Retired => {
                self.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                    && governance_satisfied
                    && queued
            }
            ProposalStateV2::Cancelled | ProposalStateV2::Expired => true,
            ProposalStateV2::TokenReviewOpen => false,
            ProposalStateV2::Frozen
            | ProposalStateV2::Extended
            | ProposalStateV2::UpgradeExecuted
            | ProposalStateV2::ProgramDataVerified
            | ProposalStateV2::PoststateAccepted
            | ProposalStateV2::UnfreezeApproved
            | ProposalStateV2::Completed
            | ProposalStateV2::SupersededByRollback => {
                self.council_approval_count == RELEASE1_APPROVAL_THRESHOLD
                    && governance_satisfied
                    && queued
            }
        };
        if !prefreeze_shape {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let frozen_required = matches!(
            self.state,
            ProposalStateV2::Frozen
                | ProposalStateV2::Extended
                | ProposalStateV2::UpgradeExecuted
                | ProposalStateV2::ProgramDataVerified
                | ProposalStateV2::PoststateAccepted
                | ProposalStateV2::UnfreezeApproved
                | ProposalStateV2::Completed
                | ProposalStateV2::SupersededByRollback
        );
        if (self.frozen_slot != 0) != frozen_required
            || frozen_required
                && (self.frozen_slot < self.not_before_slot || self.frozen_slot >= self.expiry_slot)
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }

        let extension_required = self.extension_delta != 0;
        let extension_slot_required = extension_required
            && matches!(
                self.state,
                ProposalStateV2::Extended
                    | ProposalStateV2::UpgradeExecuted
                    | ProposalStateV2::ProgramDataVerified
                    | ProposalStateV2::PoststateAccepted
                    | ProposalStateV2::UnfreezeApproved
                    | ProposalStateV2::Completed
                    | ProposalStateV2::SupersededByRollback
            );
        if matches!(self.state, ProposalStateV2::Extended) && !extension_required
            || (self.extension_executed_slot != 0) != extension_slot_required
            || self.extension_executed_slot >= self.expiry_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let upgrade_executed = matches!(
            self.state,
            ProposalStateV2::UpgradeExecuted
                | ProposalStateV2::ProgramDataVerified
                | ProposalStateV2::PoststateAccepted
                | ProposalStateV2::UnfreezeApproved
                | ProposalStateV2::Completed
                | ProposalStateV2::SupersededByRollback
        );
        if (self.upgrade_executed_slot != 0) != upgrade_executed
            || self.upgrade_executed_slot >= self.expiry_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if extension_slot_required
            && upgrade_executed
            && self.extension_executed_slot >= self.upgrade_executed_slot
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }

        let programdata_verified_required = matches!(
            self.state,
            ProposalStateV2::ProgramDataVerified
                | ProposalStateV2::PoststateAccepted
                | ProposalStateV2::UnfreezeApproved
                | ProposalStateV2::Completed
        );
        let programdata_verified_allowed = programdata_verified_required
            || matches!(self.state, ProposalStateV2::SupersededByRollback);
        if programdata_verified_required && self.programdata_verified_slot == 0
            || !programdata_verified_allowed && self.programdata_verified_slot != 0
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let poststate_accepted = matches!(
            self.state,
            ProposalStateV2::PoststateAccepted
                | ProposalStateV2::UnfreezeApproved
                | ProposalStateV2::Completed
        );
        if (self.poststate_accepted_slot != 0) != poststate_accepted {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let unfreeze_approved = matches!(
            self.state,
            ProposalStateV2::UnfreezeApproved | ProposalStateV2::Completed
        );
        let unfreeze_shape = match self.state {
            ProposalStateV2::PoststateAccepted => {
                self.unfreeze_approval_count < RELEASE1_APPROVAL_THRESHOLD
            }
            ProposalStateV2::UnfreezeApproved | ProposalStateV2::Completed => {
                self.unfreeze_approval_count == RELEASE1_APPROVAL_THRESHOLD
            }
            _ => self.unfreeze_approval_count == 0,
        };
        if !unfreeze_shape || (self.unfreeze_approved_slot != 0) != unfreeze_approved {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let expected_terminal_reason = match self.state {
            ProposalStateV2::Completed => PROPOSAL_COMPLETED_TERMINAL_REASON_V1,
            ProposalStateV2::Cancelled => self.cancellation_reason_code,
            ProposalStateV2::Expired => PROPOSAL_EXPIRED_TERMINAL_REASON_V1,
            ProposalStateV2::SupersededByRollback => {
                PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1
            }
            ProposalStateV2::Retired => PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1,
            _ => 0,
        };
        let terminal = expected_terminal_reason != 0;
        if (self.terminal_slot != 0) != terminal
            || self.terminal_reason_code != expected_terminal_reason
            || matches!(self.state, ProposalStateV2::Cancelled)
                && self.terminal_slot >= self.expiry_slot
            || matches!(self.state, ProposalStateV2::Expired)
                && self.terminal_slot < self.expiry_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        validate_non_decreasing_nonzero_slots(&[
            self.creation_slot,
            self.first_approval_slot,
            self.council_approved_slot,
            self.governance_satisfied_slot,
            self.queued_slot,
            self.frozen_slot,
            self.extension_executed_slot,
            self.upgrade_executed_slot,
            self.programdata_verified_slot,
            self.poststate_accepted_slot,
            self.unfreeze_approved_slot,
            self.terminal_slot,
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct BufferVerificationV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub status: BufferVerificationStatusV1,
    pub controller_config: Pubkey,
    pub proposal: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub buffer: Pubkey,
    pub expected_uploader_authority: Pubkey,
    pub controller_authority: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_chunk_merkle_root: [u8; 32],
    pub chunk_hash_domain: [u8; 32],
    pub chunk_size: u32,
    pub chunk_count: u32,
    pub verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    pub verified_chunk_count: u32,
    pub adopted_slot: u64,
    pub finalized_slot: u64,
    pub sealed_buffer_header_hash: [u8; 32],
    pub terminal_slot: u64,
    pub reserved: [u8; BUFFER_VERIFICATION_V1_RESERVED_LEN],
}

impl BufferVerificationV1 {
    pub const LEN: usize = 512;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &BUFFER_VERIFICATION_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.proposal,
            self.upgradeable_loader,
            self.buffer,
            self.expected_uploader_authority,
            self.controller_authority,
        ])?;
        validate_artifact_commitment(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_chunk_merkle_root,
            &self.chunk_hash_domain,
            self.chunk_size,
            self.chunk_count,
        )?;
        validate_bitmap(
            &self.verified_chunk_bitmap,
            self.chunk_count,
            self.verified_chunk_count,
        )?;
        if self.adopted_slot == 0 || self.sealed_buffer_header_hash == [0; 32] {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let status_is_canonical = match self.status {
            BufferVerificationStatusV1::Adopted => {
                self.verified_chunk_count == 0
                    && self.finalized_slot == 0
                    && self.terminal_slot == 0
            }
            BufferVerificationStatusV1::Verifying => {
                self.verified_chunk_count > 0
                    && self.verified_chunk_count < self.chunk_count
                    && self.finalized_slot == 0
                    && self.terminal_slot == 0
            }
            BufferVerificationStatusV1::ReadyToFinalize => {
                self.verified_chunk_count == self.chunk_count
                    && self.finalized_slot == 0
                    && self.terminal_slot == 0
            }
            BufferVerificationStatusV1::Verified => {
                self.verified_chunk_count == self.chunk_count
                    && self.finalized_slot != 0
                    && self.terminal_slot == 0
            }
            BufferVerificationStatusV1::ConsumedByUpgrade => {
                self.verified_chunk_count == self.chunk_count
                    && self.finalized_slot != 0
                    && self.terminal_slot != 0
            }
            BufferVerificationStatusV1::ClosedAbandoned => {
                self.terminal_slot != 0
                    && ((self.finalized_slot == 0 && self.verified_chunk_count <= self.chunk_count)
                        || (self.finalized_slot != 0
                            && self.verified_chunk_count == self.chunk_count))
            }
        };
        if !status_is_canonical {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if self.finalized_slot != 0 && self.finalized_slot < self.adopted_slot
            || self.terminal_slot != 0 && self.terminal_slot < self.adopted_slot
            || self.finalized_slot != 0
                && self.terminal_slot != 0
                && self.terminal_slot < self.finalized_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataVerificationV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub status: ProgramDataVerificationStatusV1,
    pub controller_config: Pubkey,
    pub proposal: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_authority: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub artifact_chunk_merkle_root: [u8; 32],
    pub chunk_hash_domain: [u8; 32],
    pub chunk_size: u32,
    pub payload_chunk_count: u32,
    pub verified_payload_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    pub verified_payload_chunk_count: u32,
    pub deployed_slot: u64,
    pub capacity: u64,
    pub tail_length: u64,
    pub tail_chunk_count: u32,
    pub verified_tail_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    pub verified_tail_chunk_count: u32,
    pub raw_programdata_hash: [u8; 32],
    pub zero_tail_verified: bool,
    pub finalized_slot: u64,
    pub reserved: [u8; PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN],
}

impl ProgramDataVerificationV1 {
    pub const LEN: usize = 640;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.proposal,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.controller_authority,
        ])?;
        validate_artifact_commitment(
            self.artifact_length,
            &self.artifact_sha256,
            &self.artifact_chunk_merkle_root,
            &self.chunk_hash_domain,
            self.chunk_size,
            self.payload_chunk_count,
        )?;
        if self.deployed_slot == 0
            || self.capacity < self.artifact_length
            || self.capacity > MAX_ARTIFACT_BYTES_V1
            || self.tail_length != self.capacity - self.artifact_length
            || self.tail_chunk_count
                != artifact_chunk_count_allow_empty(self.tail_length, self.chunk_size)?
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_bitmap(
            &self.verified_payload_chunk_bitmap,
            self.payload_chunk_count,
            self.verified_payload_chunk_count,
        )?;
        validate_bitmap(
            &self.verified_tail_chunk_bitmap,
            self.tail_chunk_count,
            self.verified_tail_chunk_count,
        )?;
        let complete = self.verified_payload_chunk_count == self.payload_chunk_count
            && self.verified_tail_chunk_count == self.tail_chunk_count;
        match self.status {
            ProgramDataVerificationStatusV1::Verifying => {
                if complete
                    || self.zero_tail_verified
                    || self.raw_programdata_hash != [0; 32]
                    || self.finalized_slot != 0
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
            ProgramDataVerificationStatusV1::ReadyToFinalize => {
                if !complete
                    || self.zero_tail_verified
                    || self.raw_programdata_hash != [0; 32]
                    || self.finalized_slot != 0
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
            ProgramDataVerificationStatusV1::Verified => {
                if !complete
                    || !self.zero_tail_verified
                    || self.raw_programdata_hash == [0; 32]
                    || self.finalized_slot == 0
                {
                    return Err(GovernanceError::InvalidRelease1Account);
                }
            }
        }
        if self.finalized_slot != 0 && self.finalized_slot < self.deployed_slot {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StateCheckpointV1 {
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
    pub finalized_observation_slot: u64,
    pub gate_epoch: u64,
    pub target_programdata_slot: u64,
    pub target_payload_commitment: [u8; 32],
    pub target_raw_programdata_commitment: [u8; 32],
    pub target_capacity: u64,
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
    pub checkpoint_digest: [u8; 32],
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub accepted: bool,
    pub finalized_slot: u64,
    pub reserved: [u8; STATE_CHECKPOINT_V1_RESERVED_LEN],
}

impl StateCheckpointV1 {
    pub const LEN: usize = 704;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &STATE_CHECKPOINT_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.target_program,
            self.target_programdata,
        ])?;
        let subject_shape = match self.phase {
            StateCheckpointPhaseV1::Prestate | StateCheckpointPhaseV1::Poststate => {
                self.proposal != Pubkey::default() && self.emergency_resolution == Pubkey::default()
            }
            StateCheckpointPhaseV1::Emergency => {
                self.proposal == Pubkey::default() && self.emergency_resolution != Pubkey::default()
            }
        };
        if !subject_shape
            || self.subject_digest == [0; 32]
            || self.finalized_observation_slot == 0
            || self.gate_epoch == 0
            || self.target_programdata_slot == 0
            || self.target_payload_commitment == [0; 32]
            || self.target_raw_programdata_commitment == [0; 32]
            || self.target_capacity == 0
            || self.program_owned_state_root == [0; 32]
            || self.logical_compressed_state_root == [0; 32]
            || self.semantic_custody_accounting_root == [0; 32]
            || self.schema_identifier == [0; 32]
            || self.hard_combined_root == [0; 32]
            || self.external_metadata_observation_root == [0; 32]
            || self.external_raw_balance_observation_root == [0; 32]
            || self.checkpoint_digest == [0; 32]
            || (self.admitted_positive_donation_count == 0)
                != (self.admitted_positive_donation_root == [0; 32])
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_versioned_approval(
            self.approval_council_version,
            &self.approval_council_hash,
            self.approval_bitset,
            self.approval_count,
        )?;
        if self.approval_count != RELEASE1_APPROVAL_THRESHOLD
            || self.finalized_slot == 0
            || self.finalized_observation_slot > self.finalized_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if self.accepted != (self.forbidden_drift_count == 0) {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        crate::release1_digest::validate_state_checkpoint_hard_combined_root_v1(self)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CouncilRotationProposalV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: CouncilRotationStateV1,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub current_council: Pubkey,
    pub current_council_version: u64,
    pub current_council_hash: [u8; 32],
    pub candidate_council: Pubkey,
    pub candidate_council_version: u64,
    pub candidate_council_hash: [u8; 32],
    pub creation_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub target_nonce: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub cancellation_approval_bitset: u8,
    pub cancellation_approval_count: u8,
    pub rotation_digest: [u8; 32],
    pub activated_slot: u64,
    pub cancellation_reason_code: u16,
    pub terminal_reason_code: u16,
    pub reserved: [u8; COUNCIL_ROTATION_PROPOSAL_V1_RESERVED_LEN],
}

impl CouncilRotationProposalV1 {
    pub const LEN: usize = 384;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &COUNCIL_ROTATION_PROPOSAL_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.target_program,
            self.current_council,
            self.candidate_council,
        ])?;
        if self.current_council_version == 0
            || self.candidate_council_version <= self.current_council_version
            || self.candidate_council_version == u64::MAX
            || self.current_council_hash == [0; 32]
            || self.candidate_council_hash == [0; 32]
            || self.target_nonce == 0
            || self.creation_slot >= self.not_before_slot
            || self.not_before_slot >= self.expiry_slot
            || self.rotation_digest == [0; 32]
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_approval_pair(self.approval_bitset, self.approval_count)?;
        validate_approval_pair(
            self.cancellation_approval_bitset,
            self.cancellation_approval_count,
        )?;
        if self.approval_count > RELEASE1_APPROVAL_THRESHOLD
            || self.cancellation_approval_count > RELEASE1_APPROVAL_THRESHOLD
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let cancellation_started = self.cancellation_approval_count != 0;
        if cancellation_started != (self.cancellation_reason_code != 0) {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        if matches!(self.state, CouncilRotationStateV1::Cancelled) {
            if self.cancellation_approval_count != RELEASE1_APPROVAL_THRESHOLD
                || self.terminal_reason_code != self.cancellation_reason_code
            {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        } else if self.cancellation_approval_count == RELEASE1_APPROVAL_THRESHOLD {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let approval_shape = match self.state {
            CouncilRotationStateV1::Draft => self.approval_count < RELEASE1_APPROVAL_THRESHOLD,
            CouncilRotationStateV1::CouncilApproved
            | CouncilRotationStateV1::Timelocked
            | CouncilRotationStateV1::Activated => {
                self.approval_count == RELEASE1_APPROVAL_THRESHOLD
            }
            CouncilRotationStateV1::Cancelled | CouncilRotationStateV1::Expired => true,
        };
        if !approval_shape {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let activated = matches!(self.state, CouncilRotationStateV1::Activated);
        let expected_terminal_reason = match self.state {
            CouncilRotationStateV1::Activated => COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1,
            CouncilRotationStateV1::Cancelled => self.cancellation_reason_code,
            CouncilRotationStateV1::Expired => COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1,
            CouncilRotationStateV1::Draft
            | CouncilRotationStateV1::CouncilApproved
            | CouncilRotationStateV1::Timelocked => 0,
        };
        if activated != (self.activated_slot != 0)
            || activated
                && (self.activated_slot < self.not_before_slot
                    || self.activated_slot >= self.expiry_slot)
            || self.terminal_reason_code != expected_terminal_reason
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct EmergencyFreezeResolutionV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: EmergencyFreezeResolutionStateV1,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub emergency_freeze_observation: Pubkey,
    pub frozen_epoch: u64,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub resolution_kind: EmergencyFreezeResolutionKindV1,
    pub creation_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub target_nonce: u64,
    pub observed_program_owner: Pubkey,
    pub observed_program_executable: bool,
    pub observed_program_data_length: u64,
    pub observed_program_header_present: bool,
    pub observed_linked_programdata: OptionalPubkeyV1,
    pub observed_programdata_owner: Pubkey,
    pub observed_programdata_executable: bool,
    pub observed_programdata_data_length: u64,
    pub observed_programdata_header_present: bool,
    pub observed_programdata_slot: u64,
    pub observed_raw_hash_complete: bool,
    pub observed_raw_programdata_hash: [u8; 32],
    pub observed_capacity: u64,
    pub observed_authority: OptionalPubkeyV1,
    pub emergency_checkpoint: Pubkey,
    pub approval_council_version: u64,
    pub approval_council_hash: [u8; 32],
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub resolution_digest: [u8; 32],
    pub executed_slot: u64,
    pub cancellation_reason_code: u16,
    pub terminal_reason_code: u16,
    pub reserved: [u8; EMERGENCY_FREEZE_RESOLUTION_V1_RESERVED_LEN],
}

impl EmergencyFreezeResolutionV1 {
    pub const LEN: usize = 640;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &EMERGENCY_FREEZE_RESOLUTION_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.protocol_gate,
            self.target_program,
            self.target_programdata,
            self.emergency_freeze_observation,
            self.emergency_checkpoint,
        ])?;
        self.observed_linked_programdata.validate()?;
        self.observed_authority.validate()?;
        if self.frozen_epoch == 0
            || self.freeze_slot == 0
            || self.freeze_reason_code == 0
            || self.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.creation_slot < self.freeze_slot
            || self.creation_slot >= self.expiry_slot
            || self.freeze_slot >= self.not_before_slot
            || self.not_before_slot >= self.expiry_slot
            || self.target_nonce == 0
            || self.resolution_digest == [0; 32]
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_program_observation_shape(
            self.observed_program_header_present,
            self.observed_program_data_length,
            &self.observed_linked_programdata,
        )?;
        validate_raw_programdata_hash_shape(
            self.observed_raw_hash_complete,
            &self.observed_raw_programdata_hash,
            self.observed_programdata_data_length,
        )?;
        validate_programdata_observation_shape(
            self.observed_programdata_header_present,
            self.observed_programdata_data_length,
            self.observed_programdata_slot,
            self.observed_capacity,
            &self.observed_authority,
        )?;
        validate_versioned_approval(
            self.approval_council_version,
            &self.approval_council_hash,
            self.approval_bitset,
            self.approval_count,
        )?;
        if self.approval_count > RELEASE1_APPROVAL_THRESHOLD
            || matches!(self.state, EmergencyFreezeResolutionStateV1::Cancelled)
            || self.cancellation_reason_code != 0
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let approval_shape = match self.state {
            EmergencyFreezeResolutionStateV1::Draft => {
                self.approval_count < RELEASE1_APPROVAL_THRESHOLD
            }
            EmergencyFreezeResolutionStateV1::CouncilApproved
            | EmergencyFreezeResolutionStateV1::Timelocked
            | EmergencyFreezeResolutionStateV1::Executed => {
                self.approval_count == RELEASE1_APPROVAL_THRESHOLD
            }
            EmergencyFreezeResolutionStateV1::Expired => true,
            EmergencyFreezeResolutionStateV1::Cancelled => false,
        };
        if !approval_shape {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        let executed = matches!(self.state, EmergencyFreezeResolutionStateV1::Executed);
        let expected_terminal_reason = match self.state {
            EmergencyFreezeResolutionStateV1::Executed => {
                EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1
            }
            EmergencyFreezeResolutionStateV1::Expired => {
                EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1
            }
            EmergencyFreezeResolutionStateV1::Draft
            | EmergencyFreezeResolutionStateV1::CouncilApproved
            | EmergencyFreezeResolutionStateV1::Timelocked
            | EmergencyFreezeResolutionStateV1::Cancelled => 0,
        };
        if executed != (self.executed_slot != 0)
            || executed
                && (self.executed_slot < self.not_before_slot
                    || self.executed_slot < self.creation_slot
                    || self.executed_slot >= self.expiry_slot)
            || executed
                && (!self.observed_raw_hash_complete
                    || self.observed_program_owner != UPGRADEABLE_LOADER_ID
                    || !self.observed_program_executable
                    || !self.observed_program_header_present
                    || !self.observed_linked_programdata.present
                    || self.observed_linked_programdata.value != self.target_programdata
                    || self.observed_programdata_owner != UPGRADEABLE_LOADER_ID
                    || self.observed_programdata_executable
                    || !self.observed_programdata_header_present
                    || self.observed_programdata_slot == 0
                    || self.observed_capacity == 0
                    || !self.observed_authority.present)
            || self.terminal_reason_code != expected_terminal_reason
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}

/// Immutable, atomically finalized ProgramData observation made by a guardian
/// freeze.  The raw hash covers the exact Loader-v3 ProgramData account bytes;
/// no second full-payload digest is necessary or accepted as a substitute.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct EmergencyFreezeObservationV1 {
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
    pub frozen_epoch: u64,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub actual_program_owner: Pubkey,
    pub actual_program_executable: bool,
    pub actual_program_data_length: u64,
    pub program_header_present: bool,
    pub actual_linked_programdata: OptionalPubkeyV1,
    pub actual_programdata_owner: Pubkey,
    pub actual_programdata_executable: bool,
    pub actual_programdata_data_length: u64,
    pub programdata_header_present: bool,
    pub deployed_programdata_slot: u64,
    pub raw_hash_complete: bool,
    pub raw_programdata_sha256: [u8; 32],
    pub capacity: u64,
    pub observed_authority: OptionalPubkeyV1,
    pub observation_digest: [u8; 32],
    pub finalized_slot: u64,
    pub reserved: [u8; EMERGENCY_FREEZE_OBSERVATION_V1_RESERVED_LEN],
}

impl EmergencyFreezeObservationV1 {
    pub const LEN: usize = 512;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &EMERGENCY_FREEZE_OBSERVATION_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
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
        ])?;
        self.actual_linked_programdata.validate()?;
        self.observed_authority.validate()?;
        if !self.finalized
            || self.frozen_epoch == 0
            || self.freeze_slot == 0
            || self.freeze_reason_code == 0
            || self.freeze_reason_code == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.observation_digest == [0; 32]
            || self.finalized_slot != self.freeze_slot
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        validate_program_observation_shape(
            self.program_header_present,
            self.actual_program_data_length,
            &self.actual_linked_programdata,
        )?;
        validate_raw_programdata_hash_shape(
            self.raw_hash_complete,
            &self.raw_programdata_sha256,
            self.actual_programdata_data_length,
        )?;
        validate_programdata_observation_shape(
            self.programdata_header_present,
            self.actual_programdata_data_length,
            self.deployed_programdata_slot,
            self.capacity,
            &self.observed_authority,
        )?;
        Ok(())
    }
}

/// Immutable failure evidence for one frozen primary proposal.  The optional
/// actual authority and `programdata_header_present` flag encode exact absence
/// without inventing a sentinel public key.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProgramDataFailureObservationV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub finalized: bool,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub primary_proposal: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub frozen_epoch: u64,
    pub actual_program_owner: Pubkey,
    pub actual_program_executable: bool,
    pub actual_program_data_length: u64,
    pub program_header_present: bool,
    pub actual_linked_programdata: OptionalPubkeyV1,
    pub raw_hash_complete: bool,
    pub actual_raw_programdata_sha256: [u8; 32],
    pub actual_owner: Pubkey,
    pub actual_executable: bool,
    pub actual_data_length: u64,
    pub programdata_header_present: bool,
    pub actual_programdata_slot: u64,
    pub actual_capacity: u64,
    pub actual_authority: OptionalPubkeyV1,
    pub mismatch_class: ProgramDataMismatchClassV1,
    pub failing_chunk_index: u32,
    pub expected_leaf_hash: [u8; 32],
    pub actual_leaf_hash: [u8; 32],
    pub finalized_slot: u64,
    pub observation_digest: [u8; 32],
    pub reserved: [u8; PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN],
}

impl ProgramDataFailureObservationV1 {
    pub const LEN: usize = 512;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.protocol_gate,
            self.primary_proposal,
            self.target_program,
            self.target_programdata,
        ])?;
        self.actual_linked_programdata.validate()?;
        self.actual_authority.validate()?;
        if !self.finalized
            || self.frozen_epoch == 0
            || self.finalized_slot == 0
            || self.observation_digest == [0; 32]
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        validate_program_observation_shape(
            self.program_header_present,
            self.actual_program_data_length,
            &self.actual_linked_programdata,
        )?;
        validate_raw_programdata_hash_shape(
            self.raw_hash_complete,
            &self.actual_raw_programdata_sha256,
            self.actual_data_length,
        )?;

        validate_programdata_observation_shape(
            self.programdata_header_present,
            self.actual_data_length,
            self.actual_programdata_slot,
            self.actual_capacity,
            &self.actual_authority,
        )?;

        if !self.programdata_header_present
            && self.mismatch_class != ProgramDataMismatchClassV1::Header
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }

        let leaf_failure = matches!(
            self.mismatch_class,
            ProgramDataMismatchClassV1::PayloadLeaf | ProgramDataMismatchClassV1::ZeroTail
        );
        if leaf_failure && !self.raw_hash_complete {
            return Err(GovernanceError::InvalidRelease1Account);
        }
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

        if self.programdata_header_present {
            let header_shape = match self.mismatch_class {
                ProgramDataMismatchClassV1::Header => true,
                ProgramDataMismatchClassV1::Authority => {
                    self.actual_programdata_slot != 0 && self.actual_capacity != 0
                }
                ProgramDataMismatchClassV1::Capacity => {
                    self.actual_programdata_slot != 0 && self.actual_authority.present
                }
                ProgramDataMismatchClassV1::PayloadLeaf | ProgramDataMismatchClassV1::ZeroTail => {
                    self.actual_programdata_slot != 0
                        && self.actual_capacity != 0
                        && self.actual_authority.present
                }
            };
            if !header_shape {
                return Err(GovernanceError::InvalidRelease1Account);
            }
        }
        Ok(())
    }
}

/// One seat's independently replaceable attestation for a not-yet-created
/// canonical checkpoint.  The checkpoint account is populated only after a
/// finalizer observes three matching canonical seat attestations.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CheckpointAttestationV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub checkpoint: Pubkey,
    pub subject: Pubkey,
    pub subject_digest: [u8; 32],
    pub phase: StateCheckpointPhaseV1,
    pub checkpoint_digest: [u8; 32],
    pub council: Pubkey,
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub gate_epoch: u64,
    pub seat_index: u8,
    pub seat_authority: Pubkey,
    pub attested_slot: u64,
    pub attestation_digest: [u8; 32],
    pub reserved: [u8; CHECKPOINT_ATTESTATION_V1_RESERVED_LEN],
}

impl CheckpointAttestationV1 {
    pub const LEN: usize = 384;

    pub fn validate_schema(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &CHECKPOINT_ATTESTATION_V1_DISCRIMINATOR,
            self.account_version,
            RELEASE1_ACCOUNT_VERSION_V1,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.checkpoint,
            self.subject,
            self.council,
            self.seat_authority,
        ])?;
        if self.subject_digest == [0; 32]
            || self.checkpoint_digest == [0; 32]
            || self.council_version == 0
            || self.council_hash == [0; 32]
            || self.gate_epoch == 0
            || self.seat_index >= 5
            || self.attested_slot == 0
            || self.attestation_digest == [0; 32]
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }
}

pub mod upgrade_proposal_v2_offset {
    pub const DISCRIMINATOR: usize = 0;
    pub const ACCOUNT_VERSION: usize = 8;
    pub const INITIALIZED: usize = 10;
    pub const PROPOSAL_CLASS: usize = 11;
    pub const STATE: usize = 12;
    pub const PROPOSAL_ID: usize = 16;
    pub const CLUSTER_DOMAIN: usize = 40;
    pub const BUFFER: usize = 424;
    pub const ARTIFACT_LENGTH: usize = 616;
    pub const CHUNK_SIZE: usize = 720;
    pub const FIRST_APPROVAL_SLOT: usize = 1436;
    pub const PROPOSAL_DIGEST: usize = 1610;
    pub const RESERVED: usize = 1646;
}

pub mod buffer_verification_v1_offset {
    pub const STATUS: usize = 11;
    pub const ARTIFACT_LENGTH: usize = 204;
    pub const BITMAP: usize = 316;
    pub const VERIFIED_COUNT: usize = 380;
    pub const TERMINAL_SLOT: usize = 432;
    pub const RESERVED: usize = 440;
}

pub mod programdata_verification_v1_offset {
    pub const STATUS: usize = 11;
    pub const PAYLOAD_BITMAP: usize = 316;
    pub const TAIL_LENGTH: usize = 400;
    pub const TAIL_BITMAP: usize = 412;
    pub const RAW_HASH: usize = 480;
    pub const ZERO_TAIL_VERIFIED: usize = 512;
    pub const RESERVED: usize = 521;
}

pub mod state_checkpoint_v1_offset {
    pub const PHASE: usize = 11;
    pub const CONTROLLER_CONFIG: usize = 12;
    pub const SUBJECT_DIGEST: usize = 108;
    pub const HARD_COMBINED_ROOT: usize = 412;
    pub const FORBIDDEN_DRIFT_COUNT: usize = 580;
    pub const CHECKPOINT_DIGEST: usize = 624;
    pub const ACCEPTED: usize = 658;
    pub const FINALIZED_SLOT: usize = 659;
    pub const RESERVED: usize = 667;
}

pub mod council_rotation_v1_offset {
    pub const STATE: usize = 11;
    pub const CONTROLLER_CONFIG: usize = 12;
    pub const TARGET_NONCE: usize = 244;
    pub const ROTATION_DIGEST: usize = 256;
    pub const RESERVED: usize = 300;
}

pub mod emergency_resolution_v1_offset {
    pub const STATE: usize = 11;
    pub const CONTROLLER_CONFIG: usize = 12;
    pub const FROZEN_EPOCH: usize = 172;
    pub const RESOLUTION_KIND: usize = 190;
    pub const PROGRAM_OWNER: usize = 223;
    pub const PROGRAM_EXECUTABLE: usize = 255;
    pub const PROGRAM_DATA_LENGTH: usize = 256;
    pub const PROGRAM_HEADER_PRESENT: usize = 264;
    pub const LINKED_PROGRAMDATA: usize = 265;
    pub const PROGRAMDATA_OWNER: usize = 298;
    pub const PROGRAMDATA_EXECUTABLE: usize = 330;
    pub const PROGRAMDATA_DATA_LENGTH: usize = 331;
    pub const PROGRAMDATA_HEADER_PRESENT: usize = 339;
    pub const PROGRAMDATA_SLOT: usize = 340;
    pub const RAW_HASH_COMPLETE: usize = 348;
    pub const RAW_HASH: usize = 349;
    pub const RESOLUTION_DIGEST: usize = 496;
    pub const RESERVED: usize = 540;
}

pub mod emergency_freeze_observation_v1_offset {
    pub const FINALIZED: usize = 11;
    pub const CONTROLLER_PROGRAM: usize = 12;
    pub const FROZEN_EPOCH: usize = 236;
    pub const PROGRAM_OWNER: usize = 254;
    pub const PROGRAM_EXECUTABLE: usize = 286;
    pub const PROGRAM_DATA_LENGTH: usize = 287;
    pub const PROGRAM_HEADER_PRESENT: usize = 295;
    pub const LINKED_PROGRAMDATA: usize = 296;
    pub const PROGRAMDATA_OWNER: usize = 329;
    pub const PROGRAMDATA_EXECUTABLE: usize = 361;
    pub const PROGRAMDATA_DATA_LENGTH: usize = 362;
    pub const PROGRAMDATA_HEADER_PRESENT: usize = 370;
    pub const PROGRAMDATA_SLOT: usize = 371;
    pub const RAW_HASH_COMPLETE: usize = 379;
    pub const RAW_HASH: usize = 380;
    pub const OBSERVATION_DIGEST: usize = 453;
    pub const RESERVED: usize = 493;
}

pub mod programdata_failure_observation_v1_offset {
    pub const FINALIZED: usize = 11;
    pub const CONTROLLER_CONFIG: usize = 12;
    pub const FROZEN_EPOCH: usize = 172;
    pub const PROGRAM_OWNER: usize = 180;
    pub const PROGRAM_EXECUTABLE: usize = 212;
    pub const PROGRAM_DATA_LENGTH: usize = 213;
    pub const PROGRAM_HEADER_PRESENT: usize = 221;
    pub const LINKED_PROGRAMDATA: usize = 222;
    pub const RAW_HASH_COMPLETE: usize = 255;
    pub const RAW_HASH: usize = 256;
    pub const PROGRAMDATA_OWNER: usize = 288;
    pub const PROGRAMDATA_EXECUTABLE: usize = 320;
    pub const PROGRAMDATA_DATA_LENGTH: usize = 321;
    pub const PROGRAMDATA_HEADER_PRESENT: usize = 329;
    pub const PROGRAMDATA_SLOT: usize = 330;
    pub const MISMATCH_CLASS: usize = 379;
    pub const FAILING_CHUNK_INDEX: usize = 380;
    pub const OBSERVATION_DIGEST: usize = 456;
    pub const RESERVED: usize = 488;
}

pub mod checkpoint_attestation_v1_offset {
    pub const CONTROLLER_PROGRAM: usize = 11;
    pub const CONTROLLER_CONFIG: usize = 43;
    pub const CHECKPOINT: usize = 75;
    pub const SUBJECT: usize = 107;
    pub const SUBJECT_DIGEST: usize = 139;
    pub const PHASE: usize = 171;
    pub const CHECKPOINT_DIGEST: usize = 172;
    pub const COUNCIL_VERSION: usize = 236;
    pub const SEAT_INDEX: usize = 284;
    pub const ATTESTATION_DIGEST: usize = 325;
    pub const RESERVED: usize = 357;
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

fn validate_artifact_commitment(
    artifact_length: u64,
    artifact_sha256: &[u8; 32],
    artifact_root: &[u8; 32],
    scheme_id: &[u8; 32],
    chunk_size: u32,
    chunk_count: u32,
) -> GovernanceResult<()> {
    if artifact_length == 0
        || artifact_length > MAX_ARTIFACT_BYTES_V1
        || *artifact_sha256 == [0; 32]
        || *artifact_root == [0; 32]
        || *scheme_id != ARTIFACT_MERKLE_SCHEME_ID
        || !is_release1_chunk_size(chunk_size)
        || chunk_count != artifact_chunk_count(artifact_length, chunk_size)?
    {
        Err(GovernanceError::InvalidMerkleParameters)
    } else {
        Ok(())
    }
}

fn validate_programdata_observation_shape(
    header_present: bool,
    data_length: u64,
    deployed_slot: u64,
    capacity: u64,
    authority: &OptionalPubkeyV1,
) -> GovernanceResult<()> {
    authority.validate()?;
    if header_present {
        let expected_data_length = capacity
            .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        if data_length != expected_data_length {
            return Err(GovernanceError::InvalidRelease1Account);
        }
    } else if deployed_slot != 0 || capacity != 0 || authority.present {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn validate_program_observation_shape(
    header_present: bool,
    data_length: u64,
    linked_programdata: &OptionalPubkeyV1,
) -> GovernanceResult<()> {
    linked_programdata.validate()?;
    if header_present {
        if data_length != LOADER_V3_PROGRAM_ACCOUNT_LEN_V1 || !linked_programdata.present {
            return Err(GovernanceError::InvalidRelease1Account);
        }
    } else if linked_programdata.present {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn validate_raw_programdata_hash_shape(
    hash_complete: bool,
    raw_hash: &[u8; 32],
    data_length: u64,
) -> GovernanceResult<()> {
    let within_atomic_ceiling = data_length <= MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
    if hash_complete != within_atomic_ceiling
        || (hash_complete && *raw_hash == [0; 32])
        || (!hash_complete && *raw_hash != [0; 32])
    {
        return Err(GovernanceError::InvalidRelease1Account);
    }
    Ok(())
}

fn artifact_chunk_count_allow_empty(length: u64, chunk_size: u32) -> GovernanceResult<u32> {
    if length == 0 {
        Ok(0)
    } else {
        artifact_chunk_count(length, chunk_size)
    }
}

fn validate_bitmap(
    bitmap: &[u8; VERIFICATION_BITMAP_BYTES_V1],
    bit_count: u32,
    stored_count: u32,
) -> GovernanceResult<()> {
    if bit_count > MAX_ARTIFACT_CHUNKS_V1 as u32 {
        return Err(GovernanceError::InvalidRelease1Bitmap);
    }
    let actual_count = bitmap.iter().map(|byte| byte.count_ones()).sum::<u32>();
    if actual_count != stored_count {
        return Err(GovernanceError::InvalidRelease1Bitmap);
    }
    for index in bit_count..MAX_ARTIFACT_CHUNKS_V1 as u32 {
        if bitmap[(index / 8) as usize] & (1 << (index % 8)) != 0 {
            return Err(GovernanceError::InvalidRelease1Bitmap);
        }
    }
    Ok(())
}
