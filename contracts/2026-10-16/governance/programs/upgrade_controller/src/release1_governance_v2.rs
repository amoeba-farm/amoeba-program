//! Fast-lane versioned governance timing and ceremony proposal foundation.
//!
//! Existing V1 accounts remain byte-for-byte unchanged. These new accounts
//! make proposal identity monotonic, bind immutable class-selected timing, and
//! use the actual on-chain `Clock` slot supplied by processors at creation.

use std::io::{Error, ErrorKind, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, program_error::ProgramError, pubkey::Pubkey};

use crate::{
    release1_authority_instruction::CeremonyEnvelopeV1, state::GateStatusV1, GovernanceError,
    GovernanceResult,
};

pub const GOVERNANCE_LIFECYCLE_REGISTRY_V2_DISCRIMINATOR: [u8; 8] = *b"AGVREG02";
pub const GOVERNANCE_TIMING_PROFILE_V1_DISCRIMINATOR: [u8; 8] = *b"AGVTPF01";
pub const TIMING_POLICY_CHANGE_PROPOSAL_V1_DISCRIMINATOR: [u8; 8] = *b"AGVTPP01";
pub const COUNCIL_ROTATION_PROPOSAL_V2_DISCRIMINATOR: [u8; 8] = *b"AGVROT02";
pub const TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_DISCRIMINATOR: [u8; 8] = *b"AGVTHP02";
pub const BOOTSTRAP_ACTIVATION_PROPOSAL_V2_DISCRIMINATOR: [u8; 8] = *b"AGVBAP02";

pub const GOVERNANCE_TIMING_PROFILE_HASH_DOMAIN_V1: &[u8] = b"AMOEBA_GOV_TIMING_PROFILE_V1";
pub const TIMING_POLICY_CHANGE_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_GOV_TIMING_POLICY_CHANGE_V1";
pub const COUNCIL_ROTATION_DIGEST_DOMAIN_V2: &[u8] = b"AMOEBA_GOV_COUNCIL_ROTATION_V2";
pub const TARGET_AUTHORITY_HANDOFF_DIGEST_DOMAIN_V2: &[u8] = b"AMOEBA_GOV_TARGET_HANDOFF_V2";
pub const BOOTSTRAP_ACTIVATION_DIGEST_DOMAIN_V2: &[u8] = b"AMOEBA_GOV_BOOTSTRAP_ACTIVATION_V2";

pub const GOVERNANCE_V2_ACCOUNT_VERSION: u8 = 2;
pub const GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION: u8 = 1;
pub const EQUAL_SEAT_THRESHOLD_V2: u8 = 3;
pub const VALID_FIVE_SEAT_MASK_V2: u8 = 0b1_1111;

pub const NOMINAL_SLOTS_PER_DAY_V1: u64 = 216_000;
pub const NOMINAL_SLOTS_PER_WEEK_V1: u64 = 1_512_000;
pub const MINIMUM_REVIEW_SLOTS_V1: u64 = 4_500;
pub const MINIMUM_DELAY_SLOTS_V1: u64 = 4_500;
pub const MINIMUM_EXECUTION_MARGIN_SLOTS_V1: u64 = 9_000;

pub const GOVERNANCE_V2_SEED_PREFIX: &[u8] = b"ameba-gov-v2";
pub const GOVERNANCE_LIFECYCLE_REGISTRY_V2_SEED: &[u8] = b"lifecycle";
pub const GOVERNANCE_TIMING_PROFILE_V1_SEED: &[u8] = b"timing-profile";
pub const GOVERNANCE_ACTION_PROPOSAL_V2_SEED: &[u8] = b"proposal";

pub const INITIALIZE_GOVERNANCE_LIFECYCLE_REGISTRY_V2_TAG: u8 = 82;
pub const CREATE_GOVERNANCE_TIMING_PROFILE_V1_TAG: u8 = 83;
pub const CREATE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG: u8 = 84;
pub const APPROVE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG: u8 = 85;
pub const CANCEL_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG: u8 = 86;
pub const EXPIRE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG: u8 = 87;
pub const QUEUE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG: u8 = 88;
pub const EXECUTE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG: u8 = 89;
pub const CREATE_COUNCIL_ROTATION_PROPOSAL_V2_TAG: u8 = 90;
pub const APPROVE_COUNCIL_ROTATION_PROPOSAL_V2_TAG: u8 = 91;
pub const CANCEL_COUNCIL_ROTATION_PROPOSAL_V2_TAG: u8 = 92;
pub const EXPIRE_COUNCIL_ROTATION_PROPOSAL_V2_TAG: u8 = 93;
pub const QUEUE_COUNCIL_ROTATION_PROPOSAL_V2_TAG: u8 = 94;
pub const EXECUTE_COUNCIL_ROTATION_PROPOSAL_V2_TAG: u8 = 95;
pub const CREATE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG: u8 = 96;
pub const APPROVE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG: u8 = 97;
pub const CANCEL_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG: u8 = 98;
pub const EXPIRE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG: u8 = 99;
pub const QUEUE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG: u8 = 100;
pub const EXECUTE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG: u8 = 101;
pub const CREATE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG: u8 = 102;
pub const APPROVE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG: u8 = 103;
pub const CANCEL_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG: u8 = 104;
pub const EXPIRE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG: u8 = 105;
pub const QUEUE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG: u8 = 106;
pub const EXECUTE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG: u8 = 107;

pub const GOVERNANCE_LIFECYCLE_REGISTRY_V2_RESERVED_LEN: usize = 13;
pub const GOVERNANCE_TIMING_PROFILE_V1_RESERVED_LEN: usize = 68;
pub const TIMING_POLICY_CHANGE_PROPOSAL_V1_RESERVED_LEN: usize = 36;
pub const COUNCIL_ROTATION_PROPOSAL_V2_RESERVED_LEN: usize = 28;
pub const TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_RESERVED_LEN: usize = 73;
pub const BOOTSTRAP_ACTIVATION_PROPOSAL_V2_RESERVED_LEN: usize = 41;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GovernanceTimingClassV1 {
    EmergencyRollback = 0,
    Routine = 1,
    Major = 2,
    Constitutional = 3,
}
fixed_u8_enum_borsh!(GovernanceTimingClassV1 {
    EmergencyRollback = 0,
    Routine = 1,
    Major = 2,
    Constitutional = 3,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GovernanceActionKindV2 {
    TimingPolicyChange = 0,
    CouncilRotation = 1,
    TargetAuthorityHandoff = 2,
    BootstrapActivation = 3,
}
fixed_u8_enum_borsh!(GovernanceActionKindV2 {
    TimingPolicyChange = 0,
    CouncilRotation = 1,
    TargetAuthorityHandoff = 2,
    BootstrapActivation = 3,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GovernanceLifecycleStateV2 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Completed = 3,
    Cancelled = 4,
    Expired = 5,
}
fixed_u8_enum_borsh!(GovernanceLifecycleStateV2 {
    Draft = 0,
    CouncilApproved = 1,
    Timelocked = 2,
    Completed = 3,
    Cancelled = 4,
    Expired = 5,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GovernanceClassTimingV1 {
    pub review_slots: u64,
    pub delay_slots: u64,
    /// Total lifetime measured from the actual creation slot.
    pub expiry_slots: u64,
}

impl GovernanceClassTimingV1 {
    pub const LEN: usize = 24;

    pub fn validate(&self) -> GovernanceResult<()> {
        if self.review_slots < MINIMUM_REVIEW_SLOTS_V1 || self.delay_slots < MINIMUM_DELAY_SLOTS_V1
        {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        let minimum_expiry = self
            .review_slots
            .max(self.delay_slots)
            .checked_add(MINIMUM_EXECUTION_MARGIN_SLOTS_V1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        if self.expiry_slots <= minimum_expiry {
            return Err(GovernanceError::InvalidProposalTiming);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DerivedProposalTimingV2 {
    pub review_duration_slots: u64,
    pub delay_duration_slots: u64,
    pub expiry_duration_slots: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GovernanceLifecycleRegistryV2 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub current_timing_profile: Pubkey,
    pub current_timing_profile_version: u64,
    pub current_timing_profile_hash: [u8; 32],
    pub next_proposal_id: u64,
    pub next_timing_profile_version: u64,
    pub rotation_nonce: u64,
    pub last_policy_change_proposal: Pubkey,
    pub creation_slot: u64,
    pub reserved: [u8; GOVERNANCE_LIFECYCLE_REGISTRY_V2_RESERVED_LEN],
}

impl GovernanceLifecycleRegistryV2 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &GOVERNANCE_LIFECYCLE_REGISTRY_V2_DISCRIMINATOR,
            self.version,
            GOVERNANCE_V2_ACCOUNT_VERSION,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_program,
            self.controller_config,
            self.target_program,
            self.current_timing_profile,
        ])?;
        if self.current_timing_profile_version == 0
            || self.current_timing_profile_hash == [0; 32]
            || self.next_proposal_id == 0
            || self.next_timing_profile_version <= self.current_timing_profile_version
            || self.rotation_nonce == 0
            || self.creation_slot == 0
        {
            return Err(GovernanceError::InvalidRelease1Account);
        }
        Ok(())
    }

    pub fn consume_proposal_id(&mut self, expected: u64) -> GovernanceResult<u64> {
        if expected == 0 || expected != self.next_proposal_id {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        self.next_proposal_id = self
            .next_proposal_id
            .checked_add(1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        Ok(expected)
    }

    /// Consumed when an immutable candidate timing profile is created. A
    /// rejected candidate therefore cannot squat the next usable version.
    pub fn consume_timing_profile_version(&mut self, expected: u64) -> GovernanceResult<u64> {
        if expected == 0 || expected == u64::MAX || expected != self.next_timing_profile_version {
            return Err(GovernanceError::InvalidPolicy);
        }
        self.next_timing_profile_version = self
            .next_timing_profile_version
            .checked_add(1)
            .ok_or(GovernanceError::ArithmeticOverflow)?;
        Ok(expected)
    }
}
impl_strict_account!(GovernanceLifecycleRegistryV2, 256);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GovernanceTimingProfileV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub profile_version: u64,
    pub predecessor_profile: Pubkey,
    pub predecessor_profile_hash: [u8; 32],
    pub creation_council_version: u64,
    pub emergency_rollback: GovernanceClassTimingV1,
    pub routine: GovernanceClassTimingV1,
    pub major: GovernanceClassTimingV1,
    pub constitutional: GovernanceClassTimingV1,
    pub hard_minimum_review_slots: u64,
    pub hard_minimum_delay_slots: u64,
    pub hard_minimum_execution_margin_slots: u64,
    pub profile_hash: [u8; 32],
    pub creation_slot: u64,
    pub finalized: bool,
    pub reserved: [u8; GOVERNANCE_TIMING_PROFILE_V1_RESERVED_LEN],
}

impl GovernanceTimingProfileV1 {
    pub fn class(&self, class: GovernanceTimingClassV1) -> GovernanceClassTimingV1 {
        match class {
            GovernanceTimingClassV1::EmergencyRollback => self.emergency_rollback,
            GovernanceTimingClassV1::Routine => self.routine,
            GovernanceTimingClassV1::Major => self.major,
            GovernanceTimingClassV1::Constitutional => self.constitutional,
        }
    }

    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &GOVERNANCE_TIMING_PROFILE_V1_DISCRIMINATOR,
            self.version,
            GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[self.controller_config, self.target_program])?;
        if self.profile_version == 0
            || self.creation_council_version == 0
            || self.creation_slot == 0
            || !self.finalized
            || self.hard_minimum_review_slots != MINIMUM_REVIEW_SLOTS_V1
            || self.hard_minimum_delay_slots != MINIMUM_DELAY_SLOTS_V1
            || self.hard_minimum_execution_margin_slots != MINIMUM_EXECUTION_MARGIN_SLOTS_V1
        {
            return Err(GovernanceError::InvalidPolicy);
        }
        if self.profile_version == 1 {
            if self.predecessor_profile != Pubkey::default()
                || self.predecessor_profile_hash != [0; 32]
            {
                return Err(GovernanceError::InvalidPolicy);
            }
        } else if self.predecessor_profile == Pubkey::default()
            || self.predecessor_profile_hash == [0; 32]
        {
            return Err(GovernanceError::InvalidPolicy);
        }
        for timing in [
            self.emergency_rollback,
            self.routine,
            self.major,
            self.constitutional,
        ] {
            timing.validate()?;
        }
        if compute_governance_timing_profile_hash_v1(self)? != self.profile_hash {
            return Err(GovernanceError::PolicyHashMismatch);
        }
        Ok(())
    }
}
impl_strict_account!(GovernanceTimingProfileV1, 384);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TimingPolicyChangeProposalV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: GovernanceLifecycleStateV2,
    pub proposal_id: u64,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub lifecycle_registry: Pubkey,
    pub creation_council: Pubkey,
    pub creation_council_version: u64,
    pub creation_council_hash: [u8; 32],
    pub governing_timing_profile: Pubkey,
    pub governing_timing_profile_version: u64,
    pub governing_timing_profile_hash: [u8; 32],
    pub candidate_timing_profile: Pubkey,
    pub candidate_timing_profile_version: u64,
    pub candidate_timing_profile_hash: [u8; 32],
    pub review_duration_slots: u64,
    pub delay_duration_slots: u64,
    pub expiry_duration_slots: u64,
    pub creation_slot: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub cancellation_bitset: u8,
    pub cancellation_count: u8,
    pub cancellation_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub proposal_digest: [u8; 32],
    pub reserved: [u8; TIMING_POLICY_CHANGE_PROPOSAL_V1_RESERVED_LEN],
}

impl TimingPolicyChangeProposalV1 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &TIMING_POLICY_CHANGE_PROPOSAL_V1_DISCRIMINATOR,
            self.version,
            GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.target_program,
            self.lifecycle_registry,
            self.creation_council,
            self.governing_timing_profile,
            self.candidate_timing_profile,
        ])?;
        if self.proposal_id == 0
            || self.creation_council_version == 0
            || self.creation_council_hash == [0; 32]
            || self.governing_timing_profile_version == 0
            || self.governing_timing_profile_hash == [0; 32]
            || self.candidate_timing_profile_version <= self.governing_timing_profile_version
            || self.candidate_timing_profile_version == u64::MAX
            || self.candidate_timing_profile_hash == [0; 32]
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        validate_bound_timing(
            self.creation_slot,
            self.review_duration_slots,
            self.delay_duration_slots,
            self.expiry_duration_slots,
            self.review_start_slot,
            self.review_end_slot,
            self.not_before_slot,
            self.expiry_slot,
        )?;
        validate_lifecycle_fields(&GovernanceLifecycleFieldsV2::from(self))?;
        if compute_timing_policy_change_digest_v1(self)? != self.proposal_digest {
            return Err(GovernanceError::ProposalDigestMismatch);
        }
        Ok(())
    }
}
impl_strict_account!(TimingPolicyChangeProposalV1, 512);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CouncilRotationProposalV2 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: GovernanceLifecycleStateV2,
    pub proposal_id: u64,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub lifecycle_registry: Pubkey,
    pub governing_timing_profile: Pubkey,
    pub governing_timing_profile_version: u64,
    pub governing_timing_profile_hash: [u8; 32],
    pub current_council: Pubkey,
    pub current_council_version: u64,
    pub current_council_hash: [u8; 32],
    pub candidate_council: Pubkey,
    pub candidate_council_version: u64,
    pub candidate_council_hash: [u8; 32],
    pub rotation_nonce: u64,
    pub review_duration_slots: u64,
    pub delay_duration_slots: u64,
    pub expiry_duration_slots: u64,
    pub creation_slot: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub cancellation_bitset: u8,
    pub cancellation_count: u8,
    pub cancellation_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub proposal_digest: [u8; 32],
    pub reserved: [u8; COUNCIL_ROTATION_PROPOSAL_V2_RESERVED_LEN],
}

impl CouncilRotationProposalV2 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &COUNCIL_ROTATION_PROPOSAL_V2_DISCRIMINATOR,
            self.version,
            GOVERNANCE_V2_ACCOUNT_VERSION,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.controller_config,
            self.target_program,
            self.lifecycle_registry,
            self.governing_timing_profile,
            self.current_council,
            self.candidate_council,
        ])?;
        if self.proposal_id == 0
            || self.governing_timing_profile_version == 0
            || self.governing_timing_profile_hash == [0; 32]
            || self.current_council_version == 0
            || self.current_council_hash == [0; 32]
            || self.candidate_council_version <= self.current_council_version
            || self.candidate_council_version == u64::MAX
            || self.candidate_council_hash == [0; 32]
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        validate_bound_timing(
            self.creation_slot,
            self.review_duration_slots,
            self.delay_duration_slots,
            self.expiry_duration_slots,
            self.review_start_slot,
            self.review_end_slot,
            self.not_before_slot,
            self.expiry_slot,
        )?;
        validate_lifecycle_fields(&GovernanceLifecycleFieldsV2::from(self))?;
        if compute_council_rotation_digest_v2(self)? != self.proposal_digest {
            return Err(GovernanceError::ProposalDigestMismatch);
        }
        Ok(())
    }
}
impl_strict_account!(CouncilRotationProposalV2, 512);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TargetAuthorityHandoffProposalV2 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: GovernanceLifecycleStateV2,
    pub proposal_id: u64,
    pub lifecycle_registry: Pubkey,
    pub governing_timing_profile: Pubkey,
    pub governing_timing_profile_version: u64,
    pub governing_timing_profile_hash: [u8; 32],
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
    pub review_duration_slots: u64,
    pub delay_duration_slots: u64,
    pub expiry_duration_slots: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub cancellation_bitset: u8,
    pub cancellation_count: u8,
    pub cancellation_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub proposal_digest: [u8; 32],
    pub creation_slot: u64,
    pub reserved: [u8; TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_RESERVED_LEN],
}

impl TargetAuthorityHandoffProposalV2 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_DISCRIMINATOR,
            self.version,
            GOVERNANCE_V2_ACCOUNT_VERSION,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.lifecycle_registry,
            self.governing_timing_profile,
            self.controller_program,
            self.controller_programdata,
            self.controller_immutability_receipt,
            self.controller_config,
            self.governance_policy,
            self.capacity_policy,
            self.gate,
            self.controller_authority,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.legacy_target_authority,
            self.bridge_observation,
        ])?;
        require_nonzero_hashes(&[
            self.cluster_domain,
            self.governing_timing_profile_hash,
            self.controller_immutability_digest,
            self.governance_policy_hash,
            self.capacity_policy_digest,
            self.bridge_artifact_sha256,
            self.bridge_artifact_merkle_root,
            self.bridge_artifact_scheme_id,
            self.bridge_source_commitment,
            self.bridge_build_inputs_commitment,
            self.bridge_package_commitment,
            self.bridge_release_manifest_commitment,
            self.bridge_observation_root,
            self.bridge_observation_digest,
            self.council_hash,
        ])?;
        if self.proposal_id == 0
            || self.governing_timing_profile_version == 0
            || self.bridge_artifact_length == 0
            || self.bridge_observation_generation == 0
            || self.minimum_target_deployed_slot == 0
            || self.minimum_target_capacity < self.bridge_artifact_length
            || self.minimum_target_raw_length < self.minimum_target_capacity
            || self.bootstrap_gate_status != GateStatusV1::EmergencyFrozen
            || self.bootstrap_gate_epoch == 0
            || self.bootstrap_freeze_reason_code == 0
            || self.bootstrap_freeze_slot == 0
            || self.target_nonce == 0
            || self.council_version == 0
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        validate_bound_timing(
            self.creation_slot,
            self.review_duration_slots,
            self.delay_duration_slots,
            self.expiry_duration_slots,
            self.review_start_slot,
            self.review_end_slot,
            self.not_before_slot,
            self.expiry_slot,
        )?;
        validate_lifecycle_fields(&GovernanceLifecycleFieldsV2::from(self))?;
        if compute_target_authority_handoff_digest_v2(self)? != self.proposal_digest {
            return Err(GovernanceError::ProposalDigestMismatch);
        }
        Ok(())
    }
}
impl_strict_account!(TargetAuthorityHandoffProposalV2, 1280);

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct BootstrapActivationProposalV2 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub state: GovernanceLifecycleStateV2,
    pub proposal_id: u64,
    pub lifecycle_registry: Pubkey,
    pub governing_timing_profile: Pubkey,
    pub governing_timing_profile_version: u64,
    pub governing_timing_profile_hash: [u8; 32],
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
    pub review_duration_slots: u64,
    pub delay_duration_slots: u64,
    pub expiry_duration_slots: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub cancellation_bitset: u8,
    pub cancellation_count: u8,
    pub cancellation_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub proposal_digest: [u8; 32],
    pub creation_slot: u64,
    pub reserved: [u8; BOOTSTRAP_ACTIVATION_PROPOSAL_V2_RESERVED_LEN],
}

impl BootstrapActivationProposalV2 {
    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &BOOTSTRAP_ACTIVATION_PROPOSAL_V2_DISCRIMINATOR,
            self.version,
            GOVERNANCE_V2_ACCOUNT_VERSION,
            self.initialized,
            &self.reserved,
        )?;
        require_nondefault_keys(&[
            self.lifecycle_registry,
            self.governing_timing_profile,
            self.controller_program,
            self.controller_programdata,
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
        ])?;
        require_nonzero_hashes(&[
            self.cluster_domain,
            self.governing_timing_profile_hash,
            self.governance_policy_hash,
            self.capacity_policy_digest,
            self.controller_immutability_digest,
            self.target_handoff_digest,
            self.bridge_artifact_sha256,
            self.bridge_artifact_merkle_root,
            self.bridge_artifact_scheme_id,
            self.bridge_source_commitment,
            self.bridge_build_inputs_commitment,
            self.bridge_package_commitment,
            self.bridge_release_manifest_commitment,
            self.bridge_observation_root,
            self.bridge_observation_digest,
            self.council_hash,
        ])?;
        if self.proposal_id == 0
            || self.governing_timing_profile_version == 0
            || self.bridge_artifact_length == 0
            || self.bridge_observation_generation == 0
            || self.minimum_target_deployed_slot == 0
            || self.minimum_target_capacity < self.bridge_artifact_length
            || self.minimum_target_raw_length < self.minimum_target_capacity
            || self.bootstrap_gate_status != GateStatusV1::EmergencyFrozen
            || self.bootstrap_gate_epoch == 0
            || self.bootstrap_freeze_reason_code == 0
            || self.bootstrap_freeze_slot == 0
            || self.target_nonce == 0
            || self.council_version == 0
        {
            return Err(GovernanceError::InvalidProposalCommitment);
        }
        validate_bound_timing(
            self.creation_slot,
            self.review_duration_slots,
            self.delay_duration_slots,
            self.expiry_duration_slots,
            self.review_start_slot,
            self.review_end_slot,
            self.not_before_slot,
            self.expiry_slot,
        )?;
        validate_lifecycle_fields(&GovernanceLifecycleFieldsV2::from(self))?;
        if compute_bootstrap_activation_digest_v2(self)? != self.proposal_digest {
            return Err(GovernanceError::ProposalDigestMismatch);
        }
        Ok(())
    }
}
impl_strict_account!(BootstrapActivationProposalV2, 1280);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GovernanceLifecycleFieldsV2 {
    pub state: GovernanceLifecycleStateV2,
    pub approval_bitset: u8,
    pub approval_count: u8,
    pub approval_threshold: u8,
    pub cancellation_bitset: u8,
    pub cancellation_count: u8,
    pub cancellation_threshold: u8,
    pub first_approval_slot: u64,
    pub council_approved_slot: u64,
    pub queued_slot: u64,
    pub executed_slot: u64,
    pub terminal_slot: u64,
    pub terminal_reason_code: u16,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
}

macro_rules! impl_lifecycle_fields_from {
    ($type:ty) => {
        impl From<&$type> for GovernanceLifecycleFieldsV2 {
            fn from(value: &$type) -> Self {
                Self {
                    state: value.state,
                    approval_bitset: value.approval_bitset,
                    approval_count: value.approval_count,
                    approval_threshold: value.approval_threshold,
                    cancellation_bitset: value.cancellation_bitset,
                    cancellation_count: value.cancellation_count,
                    cancellation_threshold: value.cancellation_threshold,
                    first_approval_slot: value.first_approval_slot,
                    council_approved_slot: value.council_approved_slot,
                    queued_slot: value.queued_slot,
                    executed_slot: value.executed_slot,
                    terminal_slot: value.terminal_slot,
                    terminal_reason_code: value.terminal_reason_code,
                    review_start_slot: value.review_start_slot,
                    review_end_slot: value.review_end_slot,
                    not_before_slot: value.not_before_slot,
                    expiry_slot: value.expiry_slot,
                }
            }
        }
    };
}
impl_lifecycle_fields_from!(TimingPolicyChangeProposalV1);
impl_lifecycle_fields_from!(CouncilRotationProposalV2);
impl_lifecycle_fields_from!(TargetAuthorityHandoffProposalV2);
impl_lifecycle_fields_from!(BootstrapActivationProposalV2);

pub fn nominal_governance_timing_profile_v1(
    bump: u8,
    controller_config: Pubkey,
    target_program: Pubkey,
    creation_council_version: u64,
    creation_slot: u64,
) -> GovernanceResult<GovernanceTimingProfileV1> {
    let mut profile = GovernanceTimingProfileV1 {
        discriminator: GOVERNANCE_TIMING_PROFILE_V1_DISCRIMINATOR,
        version: GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION,
        bump,
        initialized: true,
        controller_config,
        target_program,
        profile_version: 1,
        predecessor_profile: Pubkey::default(),
        predecessor_profile_hash: [0; 32],
        creation_council_version,
        emergency_rollback: GovernanceClassTimingV1 {
            review_slots: 21_600,
            delay_slots: MINIMUM_DELAY_SLOTS_V1,
            expiry_slots: NOMINAL_SLOTS_PER_DAY_V1,
        },
        routine: GovernanceClassTimingV1 {
            review_slots: NOMINAL_SLOTS_PER_WEEK_V1,
            delay_slots: 4_500,
            expiry_slots: 12 * NOMINAL_SLOTS_PER_DAY_V1,
        },
        major: GovernanceClassTimingV1 {
            review_slots: 2 * NOMINAL_SLOTS_PER_WEEK_V1,
            delay_slots: 9_000,
            expiry_slots: 24 * NOMINAL_SLOTS_PER_DAY_V1,
        },
        constitutional: GovernanceClassTimingV1 {
            review_slots: 4 * NOMINAL_SLOTS_PER_WEEK_V1,
            delay_slots: 18_000,
            expiry_slots: 49 * NOMINAL_SLOTS_PER_DAY_V1,
        },
        hard_minimum_review_slots: MINIMUM_REVIEW_SLOTS_V1,
        hard_minimum_delay_slots: MINIMUM_DELAY_SLOTS_V1,
        hard_minimum_execution_margin_slots: MINIMUM_EXECUTION_MARGIN_SLOTS_V1,
        profile_hash: [0; 32],
        creation_slot,
        finalized: true,
        reserved: [0; GOVERNANCE_TIMING_PROFILE_V1_RESERVED_LEN],
    };
    profile.profile_hash = compute_governance_timing_profile_hash_v1(&profile)?;
    profile.validate_static()?;
    Ok(profile)
}

pub fn derive_proposal_timing_v2(
    profile: &GovernanceTimingProfileV1,
    class: GovernanceTimingClassV1,
    actual_creation_slot: u64,
) -> GovernanceResult<DerivedProposalTimingV2> {
    profile.validate_static()?;
    let timing = profile.class(class);
    timing.validate()?;
    if actual_creation_slot == 0 {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    let review_end_slot = actual_creation_slot
        .checked_add(timing.review_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let not_before_slot = actual_creation_slot
        .checked_add(timing.delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expiry_slot = actual_creation_slot
        .checked_add(timing.expiry_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let minimum_expiry = review_end_slot
        .max(not_before_slot)
        .checked_add(MINIMUM_EXECUTION_MARGIN_SLOTS_V1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if expiry_slot < minimum_expiry {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    Ok(DerivedProposalTimingV2 {
        review_duration_slots: timing.review_slots,
        delay_duration_slots: timing.delay_slots,
        expiry_duration_slots: timing.expiry_slots,
        review_start_slot: actual_creation_slot,
        review_end_slot,
        not_before_slot,
        expiry_slot,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn validate_proposal_timing_binding_v2(
    creation_slot: u64,
    review_duration_slots: u64,
    delay_duration_slots: u64,
    expiry_duration_slots: u64,
    review_start_slot: u64,
    review_end_slot: u64,
    not_before_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<()> {
    validate_bound_timing(
        creation_slot,
        review_duration_slots,
        delay_duration_slots,
        expiry_duration_slots,
        review_start_slot,
        review_end_slot,
        not_before_slot,
        expiry_slot,
    )
}

pub fn validate_governance_lifecycle_fields_v2(
    fields: &GovernanceLifecycleFieldsV2,
) -> GovernanceResult<()> {
    validate_lifecycle_fields(fields)
}

pub fn record_governance_approval_v2(
    bitset: &mut u8,
    count: &mut u8,
    seat_index: u8,
    actual_clock_slot: u64,
    review_start_slot: u64,
    review_end_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<bool> {
    if actual_clock_slot < review_start_slot
        || actual_clock_slot > review_end_slot
        || actual_clock_slot >= expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    record_equal_seat_vote(bitset, count, seat_index)
}

pub fn record_governance_cancellation_v2(
    state: GovernanceLifecycleStateV2,
    bitset: &mut u8,
    count: &mut u8,
    seat_index: u8,
    actual_clock_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<bool> {
    if matches!(
        state,
        GovernanceLifecycleStateV2::Completed
            | GovernanceLifecycleStateV2::Cancelled
            | GovernanceLifecycleStateV2::Expired
    ) || actual_clock_slot >= expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition);
    }
    record_equal_seat_vote(bitset, count, seat_index)
}

pub fn require_governance_queueable_v2(
    state: GovernanceLifecycleStateV2,
    actual_clock_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<()> {
    if state != GovernanceLifecycleStateV2::CouncilApproved || actual_clock_slot >= expiry_slot {
        return Err(GovernanceError::InvalidStateTransition);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release1_ceremony_state::{
        BootstrapActivationProposalV1, TargetAuthorityHandoffProposalV1,
        BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
    };

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn assert_zero_wire_len<T: BorshDeserialize + BorshSerialize>(expected: usize) {
        let bytes = vec![0u8; expected];
        let value = T::try_from_slice(&bytes).expect("zero wire must decode for length test");
        assert_eq!(value.try_to_vec().unwrap().len(), expected);
    }

    fn profile() -> GovernanceTimingProfileV1 {
        nominal_governance_timing_profile_v1(254, key(1), key(2), 1, 10).unwrap()
    }

    fn guard() -> GovernanceActionGuardV2 {
        GovernanceActionGuardV2 {
            proposal_id: 7,
            expected_proposal_digest: [3; 32],
            expected_council_version: 2,
            expected_timing_profile_version: 1,
            expected_timing_profile_hash: [4; 32],
        }
    }

    fn policy_proposal() -> TimingPolicyChangeProposalV1 {
        let timing_profile = profile();
        let timing = derive_proposal_timing_v2(
            &timing_profile,
            GovernanceTimingClassV1::Constitutional,
            100,
        )
        .unwrap();
        let mut proposal = TimingPolicyChangeProposalV1 {
            discriminator: TIMING_POLICY_CHANGE_PROPOSAL_V1_DISCRIMINATOR,
            version: GOVERNANCE_TIMING_PROFILE_ACCOUNT_VERSION,
            bump: 201,
            initialized: true,
            state: GovernanceLifecycleStateV2::Draft,
            proposal_id: 7,
            controller_config: key(1),
            target_program: key(2),
            lifecycle_registry: key(3),
            creation_council: key(4),
            creation_council_version: 2,
            creation_council_hash: [5; 32],
            governing_timing_profile: key(6),
            governing_timing_profile_version: 1,
            governing_timing_profile_hash: timing_profile.profile_hash,
            candidate_timing_profile: key(7),
            candidate_timing_profile_version: 3,
            candidate_timing_profile_hash: [8; 32],
            review_duration_slots: timing.review_duration_slots,
            delay_duration_slots: timing.delay_duration_slots,
            expiry_duration_slots: timing.expiry_duration_slots,
            creation_slot: timing.review_start_slot,
            review_start_slot: timing.review_start_slot,
            review_end_slot: timing.review_end_slot,
            not_before_slot: timing.not_before_slot,
            expiry_slot: timing.expiry_slot,
            approval_bitset: 0,
            approval_count: 0,
            approval_threshold: EQUAL_SEAT_THRESHOLD_V2,
            cancellation_bitset: 0,
            cancellation_count: 0,
            cancellation_threshold: EQUAL_SEAT_THRESHOLD_V2,
            first_approval_slot: 0,
            council_approved_slot: 0,
            queued_slot: 0,
            executed_slot: 0,
            terminal_slot: 0,
            terminal_reason_code: 0,
            proposal_digest: [0; 32],
            reserved: [0; TIMING_POLICY_CHANGE_PROPOSAL_V1_RESERVED_LEN],
        };
        proposal.proposal_digest = compute_timing_policy_change_digest_v1(&proposal).unwrap();
        proposal.validate_static().unwrap();
        proposal
    }

    #[test]
    fn account_lengths_and_discriminators_are_exact() {
        assert_zero_wire_len::<GovernanceLifecycleRegistryV2>(GovernanceLifecycleRegistryV2::LEN);
        assert_zero_wire_len::<GovernanceTimingProfileV1>(GovernanceTimingProfileV1::LEN);
        assert_zero_wire_len::<TimingPolicyChangeProposalV1>(TimingPolicyChangeProposalV1::LEN);
        assert_zero_wire_len::<CouncilRotationProposalV2>(CouncilRotationProposalV2::LEN);
        assert_zero_wire_len::<TargetAuthorityHandoffProposalV2>(
            TargetAuthorityHandoffProposalV2::LEN,
        );
        assert_zero_wire_len::<BootstrapActivationProposalV2>(BootstrapActivationProposalV2::LEN);
        assert_eq!(GOVERNANCE_LIFECYCLE_REGISTRY_V2_DISCRIMINATOR, *b"AGVREG02");
        assert_eq!(GOVERNANCE_TIMING_PROFILE_V1_DISCRIMINATOR, *b"AGVTPF01");
        assert_eq!(TIMING_POLICY_CHANGE_PROPOSAL_V1_DISCRIMINATOR, *b"AGVTPP01");
        assert_eq!(COUNCIL_ROTATION_PROPOSAL_V2_DISCRIMINATOR, *b"AGVROT02");
        assert_eq!(
            TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_DISCRIMINATOR,
            *b"AGVTHP02"
        );
        assert_eq!(BOOTSTRAP_ACTIVATION_PROPOSAL_V2_DISCRIMINATOR, *b"AGVBAP02");
    }

    #[test]
    fn nominal_profile_is_week_scale_and_hash_sensitive() {
        let profile = profile();
        assert_eq!(profile.routine.review_slots, NOMINAL_SLOTS_PER_WEEK_V1);
        assert_eq!(profile.emergency_rollback.delay_slots, 4_500);
        assert_eq!(profile.routine.delay_slots, 4_500);
        assert_eq!(profile.major.delay_slots, 9_000);
        assert_eq!(profile.constitutional.delay_slots, 18_000);
        profile.validate_static().unwrap();
        let original = profile.profile_hash;
        let mut later_creation = profile.clone();
        later_creation.creation_slot = 999_999;
        assert_eq!(
            compute_governance_timing_profile_hash_v1(&later_creation).unwrap(),
            original
        );
        later_creation.profile_hash = original;
        later_creation.validate_static().unwrap();
        let encoded_later = later_creation.try_to_vec().unwrap();
        assert_ne!(profile.try_to_vec().unwrap(), encoded_later);
        assert_eq!(
            GovernanceTimingProfileV1::try_from_slice(&encoded_later)
                .unwrap()
                .creation_slot,
            999_999
        );
        let mut changed = profile;
        changed.routine.review_slots += 1;
        assert_ne!(
            compute_governance_timing_profile_hash_v1(&changed).unwrap(),
            original
        );
        assert_eq!(
            changed.validate_static(),
            Err(GovernanceError::PolicyHashMismatch)
        );
    }

    #[test]
    fn timing_requires_strict_margin_and_uses_checked_actual_slot() {
        let exact_boundary = GovernanceClassTimingV1 {
            review_slots: MINIMUM_REVIEW_SLOTS_V1,
            delay_slots: MINIMUM_DELAY_SLOTS_V1,
            expiry_slots: MINIMUM_REVIEW_SLOTS_V1 + MINIMUM_EXECUTION_MARGIN_SLOTS_V1,
        };
        assert_eq!(
            exact_boundary.validate(),
            Err(GovernanceError::InvalidProposalTiming)
        );
        assert_eq!(
            derive_proposal_timing_v2(&profile(), GovernanceTimingClassV1::Routine, u64::MAX),
            Err(GovernanceError::ArithmeticOverflow)
        );
        let derived =
            derive_proposal_timing_v2(&profile(), GovernanceTimingClassV1::Routine, 100).unwrap();
        assert_eq!(derived.review_start_slot, 100);
        assert_eq!(derived.review_end_slot, 100 + NOMINAL_SLOTS_PER_WEEK_V1);
        assert_eq!(derived.not_before_slot, 100 + 4_500);
        assert!(derived.expiry_slot > derived.not_before_slot);
    }

    #[test]
    fn proposal_pdas_are_unique_by_kind_and_monotonic_id() {
        let controller = key(9);
        let target = key(10);
        let a = derive_governance_action_proposal_v2(
            &controller,
            &target,
            GovernanceActionKindV2::CouncilRotation,
            1,
        );
        let b = derive_governance_action_proposal_v2(
            &controller,
            &target,
            GovernanceActionKindV2::CouncilRotation,
            2,
        );
        let c = derive_governance_action_proposal_v2(
            &controller,
            &target,
            GovernanceActionKindV2::TargetAuthorityHandoff,
            1,
        );
        assert_ne!(a.0, b.0);
        assert_ne!(a.0, c.0);
        assert_ne!(
            derive_governance_lifecycle_registry_v2(&controller, &target).0,
            derive_governance_timing_profile_v1(&controller, &target, 1).0
        );
    }

    #[test]
    fn proposal_and_candidate_counters_are_monotonic_and_skippable() {
        let mut registry = GovernanceLifecycleRegistryV2 {
            discriminator: GOVERNANCE_LIFECYCLE_REGISTRY_V2_DISCRIMINATOR,
            version: GOVERNANCE_V2_ACCOUNT_VERSION,
            bump: 1,
            initialized: true,
            controller_program: key(1),
            controller_config: key(2),
            target_program: key(3),
            current_timing_profile: key(4),
            current_timing_profile_version: 1,
            current_timing_profile_hash: [5; 32],
            next_proposal_id: 1,
            next_timing_profile_version: 2,
            rotation_nonce: 1,
            last_policy_change_proposal: Pubkey::default(),
            creation_slot: 1,
            reserved: [0; GOVERNANCE_LIFECYCLE_REGISTRY_V2_RESERVED_LEN],
        };
        registry.validate_static().unwrap();
        assert_eq!(registry.consume_proposal_id(1).unwrap(), 1);
        assert_eq!(registry.next_proposal_id, 2);
        assert_eq!(registry.consume_timing_profile_version(2).unwrap(), 2);
        assert_eq!(registry.next_timing_profile_version, 3);
        assert_eq!(
            registry.consume_timing_profile_version(2),
            Err(GovernanceError::InvalidPolicy)
        );
        assert_eq!(registry.consume_timing_profile_version(3).unwrap(), 3);
    }

    #[test]
    fn approvals_cancellation_and_expiry_are_separate_three_of_five_paths() {
        let mut approvals = 0;
        let mut approval_count = 0;
        assert!(!record_governance_approval_v2(
            &mut approvals,
            &mut approval_count,
            0,
            10,
            10,
            20,
            40
        )
        .unwrap());
        assert!(!record_governance_approval_v2(
            &mut approvals,
            &mut approval_count,
            2,
            11,
            10,
            20,
            40
        )
        .unwrap());
        assert!(record_governance_approval_v2(
            &mut approvals,
            &mut approval_count,
            4,
            12,
            10,
            20,
            40
        )
        .unwrap());
        assert_eq!(approval_count, 3);
        assert_eq!(
            record_governance_approval_v2(&mut approvals, &mut approval_count, 4, 12, 10, 20, 40),
            Err(GovernanceError::DuplicateApproval)
        );

        let mut cancellations = 0;
        let mut cancellation_count = 0;
        for seat in 0..3 {
            let crossed = record_governance_cancellation_v2(
                GovernanceLifecycleStateV2::Timelocked,
                &mut cancellations,
                &mut cancellation_count,
                seat,
                25,
                40,
            )
            .unwrap();
            assert_eq!(crossed, seat == 2);
        }
        assert_eq!(cancellation_count, 3);
        require_governance_expirable_v2(GovernanceLifecycleStateV2::Draft, 40, 40).unwrap();
        assert!(
            require_governance_expirable_v2(GovernanceLifecycleStateV2::Cancelled, 40, 40).is_err()
        );
    }

    #[test]
    fn instruction_codecs_are_exact_and_cancellation_reason_is_bound() {
        let cancel = CancelTimingPolicyChangeProposalV1 {
            guard: guard(),
            cancellation_reason_code: 7,
        };
        let bytes = cancel.pack().unwrap();
        assert_eq!(bytes.len(), CancelTimingPolicyChangeProposalV1::LEN);
        assert!(matches!(
            Release1GovernanceV2Instruction::unpack(&bytes).unwrap(),
            Release1GovernanceV2Instruction::CancelTimingPolicyChange(_)
        ));
        let zero_reason = CancelTimingPolicyChangeProposalV1 {
            guard: guard(),
            cancellation_reason_code: 0,
        }
        .pack()
        .unwrap();
        assert!(Release1GovernanceV2Instruction::unpack(&zero_reason).is_err());
        for len in 0..bytes.len() {
            assert!(Release1GovernanceV2Instruction::unpack(&bytes[..len]).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Release1GovernanceV2Instruction::unpack(&trailing).is_err());
        assert_eq!((82u8..=107).count(), 26);
    }

    #[test]
    fn v1_ceremony_bytes_remain_untouched() {
        assert_eq!(TargetAuthorityHandoffProposalV1::LEN, 1280);
        assert_eq!(BootstrapActivationProposalV1::LEN, 1280);
        assert_eq!(
            TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
            *b"AGVTHP01"
        );
        assert_eq!(BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR, *b"AGVBAP01");
    }

    #[test]
    fn rust_typescript_golden_vector_is_frozen() {
        let controller = key(9);
        let target = key(10);
        let profile = profile();
        let policy = policy_proposal();
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|value| format!("{value:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            hex(&profile.profile_hash),
            "b206b4639e1867dda2895a8089393df2deb13eeb0562e7ba6d2dc0443f221182"
        );
        assert_eq!(
            hex(&policy.proposal_digest),
            "2c3c08f7f99d09297eb266142c0f8857de9ce258218995d1d41822c101346a13"
        );
        assert_eq!(
            derive_governance_lifecycle_registry_v2(&controller, &target)
                .0
                .to_string(),
            "6hX43xoXUUtFMGmBMwE9u744ECmkyiuTcvsU3e4SD19X"
        );
        assert_eq!(
            derive_governance_timing_profile_v1(&controller, &target, 1)
                .0
                .to_string(),
            "2xwLZqxM6hNJMuMcCULBHvmsr61Wmk7L51yi5qoFAviw"
        );
        assert_eq!(
            derive_governance_action_proposal_v2(
                &controller,
                &target,
                GovernanceActionKindV2::TargetAuthorityHandoff,
                7
            )
            .0
            .to_string(),
            "6rZgfEMjaYVfsQghBf7LPFrYzMzTVDJGMnQVMdb2kQMU"
        );
    }
}

pub fn require_governance_executable_v2(
    state: GovernanceLifecycleStateV2,
    actual_clock_slot: u64,
    not_before_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<()> {
    if state != GovernanceLifecycleStateV2::Timelocked
        || actual_clock_slot < not_before_slot
        || actual_clock_slot >= expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition);
    }
    Ok(())
}

pub fn require_governance_expirable_v2(
    state: GovernanceLifecycleStateV2,
    actual_clock_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<()> {
    if matches!(
        state,
        GovernanceLifecycleStateV2::Completed
            | GovernanceLifecycleStateV2::Cancelled
            | GovernanceLifecycleStateV2::Expired
    ) || actual_clock_slot < expiry_slot
    {
        return Err(GovernanceError::InvalidStateTransition);
    }
    Ok(())
}

pub fn derive_governance_lifecycle_registry_v2(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            GOVERNANCE_V2_SEED_PREFIX,
            GOVERNANCE_LIFECYCLE_REGISTRY_V2_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_governance_timing_profile_v1(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    profile_version: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            GOVERNANCE_V2_SEED_PREFIX,
            GOVERNANCE_TIMING_PROFILE_V1_SEED,
            target_program.as_ref(),
            &profile_version.to_le_bytes(),
        ],
        controller_program,
    )
}

pub fn derive_governance_action_proposal_v2(
    controller_program: &Pubkey,
    target_program: &Pubkey,
    kind: GovernanceActionKindV2,
    proposal_id: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            GOVERNANCE_V2_SEED_PREFIX,
            GOVERNANCE_ACTION_PROPOSAL_V2_SEED,
            target_program.as_ref(),
            &[kind as u8],
            &proposal_id.to_le_bytes(),
        ],
        controller_program,
    )
}

pub fn compute_governance_timing_profile_hash_v1(
    profile: &GovernanceTimingProfileV1,
) -> GovernanceResult<[u8; 32]> {
    let mut normalized = profile.clone();
    normalized.profile_hash = [0; 32];
    // The creation slot is a runtime observation recorded from Clock. It is
    // intentionally not policy identity, so plans never have to predict the
    // slot in which initialization lands.
    normalized.creation_slot = 0;
    normalized.reserved = [0; GOVERNANCE_TIMING_PROFILE_V1_RESERVED_LEN];
    hash_borsh(GOVERNANCE_TIMING_PROFILE_HASH_DOMAIN_V1, &normalized)
}

pub fn validate_governance_timing_profile_hash_v1(
    profile: &GovernanceTimingProfileV1,
) -> GovernanceResult<()> {
    if compute_governance_timing_profile_hash_v1(profile)? != profile.profile_hash {
        return Err(GovernanceError::PolicyHashMismatch);
    }
    Ok(())
}

macro_rules! normalize_proposal_lifecycle {
    ($value:ident) => {{
        $value.state = GovernanceLifecycleStateV2::Draft;
        $value.approval_bitset = 0;
        $value.approval_count = 0;
        $value.cancellation_bitset = 0;
        $value.cancellation_count = 0;
        $value.first_approval_slot = 0;
        $value.council_approved_slot = 0;
        $value.queued_slot = 0;
        $value.executed_slot = 0;
        $value.terminal_slot = 0;
        $value.terminal_reason_code = 0;
        $value.proposal_digest = [0; 32];
    }};
}

pub fn compute_timing_policy_change_digest_v1(
    proposal: &TimingPolicyChangeProposalV1,
) -> GovernanceResult<[u8; 32]> {
    let mut normalized = proposal.clone();
    normalize_proposal_lifecycle!(normalized);
    normalized.reserved = [0; TIMING_POLICY_CHANGE_PROPOSAL_V1_RESERVED_LEN];
    hash_borsh(TIMING_POLICY_CHANGE_DIGEST_DOMAIN_V1, &normalized)
}

pub fn validate_timing_policy_change_digest_v1(
    proposal: &TimingPolicyChangeProposalV1,
) -> GovernanceResult<()> {
    validate_digest(
        compute_timing_policy_change_digest_v1(proposal)?,
        proposal.proposal_digest,
    )
}

pub fn compute_council_rotation_digest_v2(
    proposal: &CouncilRotationProposalV2,
) -> GovernanceResult<[u8; 32]> {
    let mut normalized = proposal.clone();
    normalize_proposal_lifecycle!(normalized);
    normalized.reserved = [0; COUNCIL_ROTATION_PROPOSAL_V2_RESERVED_LEN];
    hash_borsh(COUNCIL_ROTATION_DIGEST_DOMAIN_V2, &normalized)
}

pub fn validate_council_rotation_digest_v2(
    proposal: &CouncilRotationProposalV2,
) -> GovernanceResult<()> {
    validate_digest(
        compute_council_rotation_digest_v2(proposal)?,
        proposal.proposal_digest,
    )
}

pub fn compute_target_authority_handoff_digest_v2(
    proposal: &TargetAuthorityHandoffProposalV2,
) -> GovernanceResult<[u8; 32]> {
    let mut normalized = proposal.clone();
    normalize_proposal_lifecycle!(normalized);
    normalized.reserved = [0; TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_RESERVED_LEN];
    hash_borsh(TARGET_AUTHORITY_HANDOFF_DIGEST_DOMAIN_V2, &normalized)
}

pub fn validate_target_authority_handoff_digest_v2(
    proposal: &TargetAuthorityHandoffProposalV2,
) -> GovernanceResult<()> {
    validate_digest(
        compute_target_authority_handoff_digest_v2(proposal)?,
        proposal.proposal_digest,
    )
}

pub fn compute_bootstrap_activation_digest_v2(
    proposal: &BootstrapActivationProposalV2,
) -> GovernanceResult<[u8; 32]> {
    let mut normalized = proposal.clone();
    normalize_proposal_lifecycle!(normalized);
    normalized.reserved = [0; BOOTSTRAP_ACTIVATION_PROPOSAL_V2_RESERVED_LEN];
    hash_borsh(BOOTSTRAP_ACTIVATION_DIGEST_DOMAIN_V2, &normalized)
}

pub fn validate_bootstrap_activation_digest_v2(
    proposal: &BootstrapActivationProposalV2,
) -> GovernanceResult<()> {
    validate_digest(
        compute_bootstrap_activation_digest_v2(proposal)?,
        proposal.proposal_digest,
    )
}

fn hash_borsh<T: BorshSerialize>(domain: &[u8], value: &T) -> GovernanceResult<[u8; 32]> {
    let bytes = value
        .try_to_vec()
        .map_err(|_| GovernanceError::InvalidRelease1Account)?;
    Ok(hashv(&[domain, &bytes]).to_bytes())
}

fn validate_digest(actual: [u8; 32], expected: [u8; 32]) -> GovernanceResult<()> {
    if actual != expected {
        return Err(GovernanceError::ProposalDigestMismatch);
    }
    Ok(())
}

fn record_equal_seat_vote(
    bitset: &mut u8,
    count: &mut u8,
    seat_index: u8,
) -> GovernanceResult<bool> {
    if seat_index >= 5 || *bitset & !VALID_FIVE_SEAT_MASK_V2 != 0 {
        return Err(GovernanceError::InvalidApprovalBitset);
    }
    let bit = 1u8
        .checked_shl(u32::from(seat_index))
        .ok_or(GovernanceError::InvalidApprovalBitset)?;
    if *bitset & bit != 0 {
        return Err(GovernanceError::DuplicateApproval);
    }
    *bitset |= bit;
    *count = bitset.count_ones() as u8;
    Ok(*count >= EQUAL_SEAT_THRESHOLD_V2)
}

#[allow(clippy::too_many_arguments)]
fn validate_bound_timing(
    creation_slot: u64,
    review_duration_slots: u64,
    delay_duration_slots: u64,
    expiry_duration_slots: u64,
    review_start_slot: u64,
    review_end_slot: u64,
    not_before_slot: u64,
    expiry_slot: u64,
) -> GovernanceResult<()> {
    let timing = GovernanceClassTimingV1 {
        review_slots: review_duration_slots,
        delay_slots: delay_duration_slots,
        expiry_slots: expiry_duration_slots,
    };
    timing.validate()?;
    if creation_slot == 0 || review_start_slot != creation_slot {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    let expected_review_end = creation_slot
        .checked_add(review_duration_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expected_not_before = creation_slot
        .checked_add(delay_duration_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    let expected_expiry = creation_slot
        .checked_add(expiry_duration_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if review_end_slot != expected_review_end
        || not_before_slot != expected_not_before
        || expiry_slot != expected_expiry
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    Ok(())
}

fn validate_lifecycle_fields(fields: &GovernanceLifecycleFieldsV2) -> GovernanceResult<()> {
    for (bitset, count, threshold) in [
        (
            fields.approval_bitset,
            fields.approval_count,
            fields.approval_threshold,
        ),
        (
            fields.cancellation_bitset,
            fields.cancellation_count,
            fields.cancellation_threshold,
        ),
    ] {
        if bitset & !VALID_FIVE_SEAT_MASK_V2 != 0
            || count != bitset.count_ones() as u8
            || threshold != EQUAL_SEAT_THRESHOLD_V2
        {
            return Err(GovernanceError::InvalidApprovalBitset);
        }
    }
    if fields.review_start_slot == 0
        || fields.review_start_slot >= fields.review_end_slot
        || fields.review_start_slot >= fields.not_before_slot
        || fields.review_end_slot >= fields.expiry_slot
        || fields.not_before_slot >= fields.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming);
    }
    if fields.approval_count == 0 {
        if fields.first_approval_slot != 0 {
            return Err(GovernanceError::InvalidStateTransition);
        }
    } else if fields.first_approval_slot < fields.review_start_slot
        || fields.first_approval_slot > fields.review_end_slot
    {
        return Err(GovernanceError::InvalidStateTransition);
    }
    match fields.state {
        GovernanceLifecycleStateV2::Draft => {
            if fields.approval_count >= EQUAL_SEAT_THRESHOLD_V2
                || fields.cancellation_count >= EQUAL_SEAT_THRESHOLD_V2
                || fields.council_approved_slot != 0
                || fields.queued_slot != 0
                || fields.executed_slot != 0
                || fields.terminal_slot != 0
                || fields.terminal_reason_code != 0
            {
                return Err(GovernanceError::InvalidStateTransition);
            }
        }
        GovernanceLifecycleStateV2::CouncilApproved => {
            if fields.approval_count < EQUAL_SEAT_THRESHOLD_V2
                || fields.cancellation_count >= EQUAL_SEAT_THRESHOLD_V2
                || fields.council_approved_slot < fields.first_approval_slot
                || fields.council_approved_slot > fields.review_end_slot
                || fields.queued_slot != 0
                || fields.executed_slot != 0
                || fields.terminal_slot != 0
                || fields.terminal_reason_code != 0
            {
                return Err(GovernanceError::InvalidStateTransition);
            }
        }
        GovernanceLifecycleStateV2::Timelocked => {
            if fields.approval_count < EQUAL_SEAT_THRESHOLD_V2
                || fields.cancellation_count >= EQUAL_SEAT_THRESHOLD_V2
                || fields.council_approved_slot < fields.first_approval_slot
                || fields.queued_slot < fields.council_approved_slot
                || fields.queued_slot >= fields.expiry_slot
                || fields.executed_slot != 0
                || fields.terminal_slot != 0
                || fields.terminal_reason_code != 0
            {
                return Err(GovernanceError::InvalidStateTransition);
            }
        }
        GovernanceLifecycleStateV2::Completed => {
            if fields.approval_count < EQUAL_SEAT_THRESHOLD_V2
                || fields.queued_slot == 0
                || fields.executed_slot < fields.not_before_slot
                || fields.executed_slot >= fields.expiry_slot
                || fields.terminal_slot != fields.executed_slot
                || fields.terminal_reason_code == 0
            {
                return Err(GovernanceError::InvalidStateTransition);
            }
        }
        GovernanceLifecycleStateV2::Cancelled => {
            if fields.cancellation_count < EQUAL_SEAT_THRESHOLD_V2
                || fields.executed_slot != 0
                || fields.terminal_slot == 0
                || fields.terminal_slot >= fields.expiry_slot
                || fields.terminal_reason_code == 0
            {
                return Err(GovernanceError::InvalidStateTransition);
            }
        }
        GovernanceLifecycleStateV2::Expired => {
            if fields.executed_slot != 0
                || fields.terminal_slot < fields.expiry_slot
                || fields.terminal_reason_code == 0
            {
                return Err(GovernanceError::InvalidStateTransition);
            }
        }
    }
    Ok(())
}

fn validate_header(
    discriminator: &[u8; 8],
    expected_discriminator: &[u8; 8],
    version: u8,
    expected_version: u8,
    initialized: bool,
    reserved: &[u8],
) -> GovernanceResult<()> {
    if discriminator != expected_discriminator {
        return Err(GovernanceError::InvalidDiscriminator);
    }
    if version != expected_version {
        return Err(GovernanceError::UnsupportedVersion);
    }
    if !initialized {
        return Err(GovernanceError::Uninitialized);
    }
    if reserved.iter().any(|value| *value != 0) {
        return Err(GovernanceError::NonzeroReserved);
    }
    Ok(())
}

fn require_nondefault_keys(keys: &[Pubkey]) -> GovernanceResult<()> {
    if keys.iter().any(|key| *key == Pubkey::default()) {
        return Err(GovernanceError::DefaultPubkey);
    }
    Ok(())
}

fn require_nonzero_hashes(hashes: &[[u8; 32]]) -> GovernanceResult<()> {
    if hashes.contains(&[0; 32]) {
        return Err(GovernanceError::InvalidProposalCommitment);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GovernanceActionGuardV2 {
    pub proposal_id: u64,
    pub expected_proposal_digest: [u8; 32],
    pub expected_council_version: u64,
    pub expected_timing_profile_version: u64,
    pub expected_timing_profile_hash: [u8; 32],
}

impl GovernanceActionGuardV2 {
    pub const LEN: usize = 88;

    pub fn validate(&self) -> Result<(), ProgramError> {
        if self.proposal_id == 0
            || self.expected_proposal_digest == [0; 32]
            || self.expected_council_version == 0
            || self.expected_timing_profile_version == 0
            || self.expected_timing_profile_hash == [0; 32]
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

macro_rules! fixed_instruction {
    ($name:ident, $tag:expr, $payload_len:expr, { $($field:ident: $type:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
        pub struct $name {
            $(pub $field: $type,)*
        }

        impl $name {
            pub const TAG: u8 = $tag;
            pub const PAYLOAD_LEN: usize = $payload_len;
            pub const LEN: usize = 1 + Self::PAYLOAD_LEN;

            pub fn pack(&self) -> Result<Vec<u8>, ProgramError> {
                let payload = self
                    .try_to_vec()
                    .map_err(|_| ProgramError::InvalidInstructionData)?;
                if payload.len() != Self::PAYLOAD_LEN {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let mut out = Vec::with_capacity(Self::LEN);
                out.push(Self::TAG);
                out.extend_from_slice(&payload);
                Ok(out)
            }

            pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
                if data.len() != Self::LEN || data.first().copied() != Some(Self::TAG) {
                    return Err(ProgramError::InvalidInstructionData);
                }
                Self::try_from_slice(&data[1..])
                    .map_err(|_| ProgramError::InvalidInstructionData)
            }
        }
    };
}

fixed_instruction!(
    InitializeGovernanceLifecycleRegistryV2,
    INITIALIZE_GOVERNANCE_LIFECYCLE_REGISTRY_V2_TAG,
    56,
    {
        expected_initial_timing_profile_version: u64,
        expected_initial_timing_profile_hash: [u8; 32],
        expected_initial_next_proposal_id: u64,
        expected_initial_rotation_nonce: u64
    }
);

fixed_instruction!(
    CreateGovernanceTimingProfileV1,
    CREATE_GOVERNANCE_TIMING_PROFILE_V1_TAG,
    136,
    {
        profile_version: u64,
        predecessor_profile_hash: [u8; 32],
        emergency_rollback: GovernanceClassTimingV1,
        routine: GovernanceClassTimingV1,
        major: GovernanceClassTimingV1,
        constitutional: GovernanceClassTimingV1
    }
);

fixed_instruction!(
    CreateTimingPolicyChangeProposalV1,
    CREATE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG,
    128,
    {
        expected_proposal_id: u64,
        expected_current_timing_profile_version: u64,
        expected_current_timing_profile_hash: [u8; 32],
        candidate_timing_profile_version: u64,
        candidate_timing_profile_hash: [u8; 32],
        expected_council_version: u64,
        expected_council_hash: [u8; 32]
    }
);

macro_rules! fixed_guard_instruction {
    ($name:ident, $tag:expr) => {
        fixed_instruction!($name, $tag, GovernanceActionGuardV2::LEN, {
            guard: GovernanceActionGuardV2
        });
    };
}

macro_rules! fixed_cancel_instruction {
    ($name:ident, $tag:expr) => {
        fixed_instruction!($name, $tag, GovernanceActionGuardV2::LEN + 2, {
            guard: GovernanceActionGuardV2,
            cancellation_reason_code: u16
        });
    };
}

fixed_guard_instruction!(
    ApproveTimingPolicyChangeProposalV1,
    APPROVE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG
);
fixed_cancel_instruction!(
    CancelTimingPolicyChangeProposalV1,
    CANCEL_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG
);
fixed_guard_instruction!(
    ExpireTimingPolicyChangeProposalV1,
    EXPIRE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG
);
fixed_guard_instruction!(
    QueueTimingPolicyChangeProposalV1,
    QUEUE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG
);
fixed_guard_instruction!(
    ExecuteTimingPolicyChangeProposalV1,
    EXECUTE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG
);

fixed_instruction!(
    CreateCouncilRotationProposalV2,
    CREATE_COUNCIL_ROTATION_PROPOSAL_V2_TAG,
    136,
    {
        expected_proposal_id: u64,
        expected_current_council_version: u64,
        expected_current_council_hash: [u8; 32],
        candidate_council_version: u64,
        candidate_council_hash: [u8; 32],
        expected_rotation_nonce: u64,
        expected_timing_profile_version: u64,
        expected_timing_profile_hash: [u8; 32]
    }
);
fixed_guard_instruction!(
    ApproveCouncilRotationProposalV2,
    APPROVE_COUNCIL_ROTATION_PROPOSAL_V2_TAG
);
fixed_cancel_instruction!(
    CancelCouncilRotationProposalV2,
    CANCEL_COUNCIL_ROTATION_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    ExpireCouncilRotationProposalV2,
    EXPIRE_COUNCIL_ROTATION_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    QueueCouncilRotationProposalV2,
    QUEUE_COUNCIL_ROTATION_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    ExecuteCouncilRotationProposalV2,
    EXECUTE_COUNCIL_ROTATION_PROPOSAL_V2_TAG
);

fixed_instruction!(
    CreateTargetAuthorityHandoffProposalV2,
    CREATE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG,
    200,
    {
        expected_proposal_id: u64,
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_council_version: u64,
        expected_timing_profile_version: u64,
        expected_timing_profile_hash: [u8; 32],
        bridge_source_commitment: [u8; 32],
        bridge_build_inputs_commitment: [u8; 32],
        bridge_package_commitment: [u8; 32],
        bridge_release_manifest_commitment: [u8; 32]
    }
);
fixed_guard_instruction!(
    ApproveTargetAuthorityHandoffProposalV2,
    APPROVE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG
);
fixed_cancel_instruction!(
    CancelTargetAuthorityHandoffProposalV2,
    CANCEL_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    ExpireTargetAuthorityHandoffProposalV2,
    EXPIRE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    QueueTargetAuthorityHandoffProposalV2,
    QUEUE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG
);
fixed_instruction!(
    ExecuteTargetAuthorityHandoffProposalV2,
    EXECUTE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG,
    214,
    {
        guard: GovernanceActionGuardV2,
        expected_bridge_observation_digest: [u8; 32],
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        envelope: CeremonyEnvelopeV1
    }
);

fixed_instruction!(
    CreateBootstrapActivationProposalV2,
    CREATE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG,
    168,
    {
        expected_proposal_id: u64,
        expected_controller_immutability_digest: [u8; 32],
        expected_handoff_receipt_digest: [u8; 32],
        expected_bridge_observation_digest: [u8; 32],
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_council_version: u64,
        expected_timing_profile_version: u64,
        expected_timing_profile_hash: [u8; 32]
    }
);
fixed_guard_instruction!(
    ApproveBootstrapActivationProposalV2,
    APPROVE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG
);
fixed_cancel_instruction!(
    CancelBootstrapActivationProposalV2,
    CANCEL_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    ExpireBootstrapActivationProposalV2,
    EXPIRE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG
);
fixed_guard_instruction!(
    QueueBootstrapActivationProposalV2,
    QUEUE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG
);
fixed_instruction!(
    ExecuteBootstrapActivationProposalV2,
    EXECUTE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG,
    278,
    {
        guard: GovernanceActionGuardV2,
        expected_bridge_observation_digest: [u8; 32],
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_deployment_plan_digest: [u8; 32],
        expected_receipt_plan_digest: [u8; 32],
        envelope: CeremonyEnvelopeV1
    }
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Release1GovernanceV2Instruction {
    InitializeRegistry(InitializeGovernanceLifecycleRegistryV2),
    CreateTimingProfile(CreateGovernanceTimingProfileV1),
    CreateTimingPolicyChange(CreateTimingPolicyChangeProposalV1),
    ApproveTimingPolicyChange(ApproveTimingPolicyChangeProposalV1),
    CancelTimingPolicyChange(CancelTimingPolicyChangeProposalV1),
    ExpireTimingPolicyChange(ExpireTimingPolicyChangeProposalV1),
    QueueTimingPolicyChange(QueueTimingPolicyChangeProposalV1),
    ExecuteTimingPolicyChange(ExecuteTimingPolicyChangeProposalV1),
    CreateCouncilRotation(CreateCouncilRotationProposalV2),
    ApproveCouncilRotation(ApproveCouncilRotationProposalV2),
    CancelCouncilRotation(CancelCouncilRotationProposalV2),
    ExpireCouncilRotation(ExpireCouncilRotationProposalV2),
    QueueCouncilRotation(QueueCouncilRotationProposalV2),
    ExecuteCouncilRotation(ExecuteCouncilRotationProposalV2),
    CreateTargetAuthorityHandoff(CreateTargetAuthorityHandoffProposalV2),
    ApproveTargetAuthorityHandoff(ApproveTargetAuthorityHandoffProposalV2),
    CancelTargetAuthorityHandoff(CancelTargetAuthorityHandoffProposalV2),
    ExpireTargetAuthorityHandoff(ExpireTargetAuthorityHandoffProposalV2),
    QueueTargetAuthorityHandoff(QueueTargetAuthorityHandoffProposalV2),
    ExecuteTargetAuthorityHandoff(ExecuteTargetAuthorityHandoffProposalV2),
    CreateBootstrapActivation(CreateBootstrapActivationProposalV2),
    ApproveBootstrapActivation(ApproveBootstrapActivationProposalV2),
    CancelBootstrapActivation(CancelBootstrapActivationProposalV2),
    ExpireBootstrapActivation(ExpireBootstrapActivationProposalV2),
    QueueBootstrapActivation(QueueBootstrapActivationProposalV2),
    ExecuteBootstrapActivation(ExecuteBootstrapActivationProposalV2),
}

impl Release1GovernanceV2Instruction {
    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        let tag = data
            .first()
            .copied()
            .ok_or(ProgramError::InvalidInstructionData)?;
        let value = match tag {
            INITIALIZE_GOVERNANCE_LIFECYCLE_REGISTRY_V2_TAG => {
                Self::InitializeRegistry(InitializeGovernanceLifecycleRegistryV2::unpack(data)?)
            }
            CREATE_GOVERNANCE_TIMING_PROFILE_V1_TAG => {
                Self::CreateTimingProfile(CreateGovernanceTimingProfileV1::unpack(data)?)
            }
            CREATE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
                Self::CreateTimingPolicyChange(CreateTimingPolicyChangeProposalV1::unpack(data)?)
            }
            APPROVE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
                Self::ApproveTimingPolicyChange(ApproveTimingPolicyChangeProposalV1::unpack(data)?)
            }
            CANCEL_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
                Self::CancelTimingPolicyChange(CancelTimingPolicyChangeProposalV1::unpack(data)?)
            }
            EXPIRE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
                Self::ExpireTimingPolicyChange(ExpireTimingPolicyChangeProposalV1::unpack(data)?)
            }
            QUEUE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
                Self::QueueTimingPolicyChange(QueueTimingPolicyChangeProposalV1::unpack(data)?)
            }
            EXECUTE_TIMING_POLICY_CHANGE_PROPOSAL_V1_TAG => {
                Self::ExecuteTimingPolicyChange(ExecuteTimingPolicyChangeProposalV1::unpack(data)?)
            }
            CREATE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
                Self::CreateCouncilRotation(CreateCouncilRotationProposalV2::unpack(data)?)
            }
            APPROVE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
                Self::ApproveCouncilRotation(ApproveCouncilRotationProposalV2::unpack(data)?)
            }
            CANCEL_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
                Self::CancelCouncilRotation(CancelCouncilRotationProposalV2::unpack(data)?)
            }
            EXPIRE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
                Self::ExpireCouncilRotation(ExpireCouncilRotationProposalV2::unpack(data)?)
            }
            QUEUE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
                Self::QueueCouncilRotation(QueueCouncilRotationProposalV2::unpack(data)?)
            }
            EXECUTE_COUNCIL_ROTATION_PROPOSAL_V2_TAG => {
                Self::ExecuteCouncilRotation(ExecuteCouncilRotationProposalV2::unpack(data)?)
            }
            CREATE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => Self::CreateTargetAuthorityHandoff(
                CreateTargetAuthorityHandoffProposalV2::unpack(data)?,
            ),
            APPROVE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
                Self::ApproveTargetAuthorityHandoff(
                    ApproveTargetAuthorityHandoffProposalV2::unpack(data)?,
                )
            }
            CANCEL_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => Self::CancelTargetAuthorityHandoff(
                CancelTargetAuthorityHandoffProposalV2::unpack(data)?,
            ),
            EXPIRE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => Self::ExpireTargetAuthorityHandoff(
                ExpireTargetAuthorityHandoffProposalV2::unpack(data)?,
            ),
            QUEUE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => Self::QueueTargetAuthorityHandoff(
                QueueTargetAuthorityHandoffProposalV2::unpack(data)?,
            ),
            EXECUTE_TARGET_AUTHORITY_HANDOFF_PROPOSAL_V2_TAG => {
                Self::ExecuteTargetAuthorityHandoff(
                    ExecuteTargetAuthorityHandoffProposalV2::unpack(data)?,
                )
            }
            CREATE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
                Self::CreateBootstrapActivation(CreateBootstrapActivationProposalV2::unpack(data)?)
            }
            APPROVE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => Self::ApproveBootstrapActivation(
                ApproveBootstrapActivationProposalV2::unpack(data)?,
            ),
            CANCEL_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
                Self::CancelBootstrapActivation(CancelBootstrapActivationProposalV2::unpack(data)?)
            }
            EXPIRE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
                Self::ExpireBootstrapActivation(ExpireBootstrapActivationProposalV2::unpack(data)?)
            }
            QUEUE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => {
                Self::QueueBootstrapActivation(QueueBootstrapActivationProposalV2::unpack(data)?)
            }
            EXECUTE_BOOTSTRAP_ACTIVATION_PROPOSAL_V2_TAG => Self::ExecuteBootstrapActivation(
                ExecuteBootstrapActivationProposalV2::unpack(data)?,
            ),
            _ => return Err(ProgramError::InvalidInstructionData),
        };
        validate_instruction_payload(&value)?;
        Ok(value)
    }
}

fn validate_instruction_payload(
    value: &Release1GovernanceV2Instruction,
) -> Result<(), ProgramError> {
    use Release1GovernanceV2Instruction::*;
    match value {
        InitializeRegistry(ix) => {
            if ix.expected_initial_timing_profile_version != 1
                || ix.expected_initial_timing_profile_hash == [0; 32]
                || ix.expected_initial_next_proposal_id != 1
                || ix.expected_initial_rotation_nonce != 1
            {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        CreateTimingProfile(ix) => {
            if ix.profile_version == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
            for timing in [
                ix.emergency_rollback,
                ix.routine,
                ix.major,
                ix.constitutional,
            ] {
                timing.validate().map_err(ProgramError::from)?;
            }
        }
        CreateTimingPolicyChange(ix) => {
            if ix.expected_proposal_id == 0
                || ix.expected_current_timing_profile_version == 0
                || ix.expected_current_timing_profile_hash == [0; 32]
                || ix.candidate_timing_profile_version <= ix.expected_current_timing_profile_version
                || ix.candidate_timing_profile_version == u64::MAX
                || ix.candidate_timing_profile_hash == [0; 32]
                || ix.expected_council_version == 0
                || ix.expected_council_hash == [0; 32]
            {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        CreateCouncilRotation(ix) => {
            if ix.expected_proposal_id == 0
                || ix.expected_current_council_version == 0
                || ix.expected_current_council_hash == [0; 32]
                || ix.candidate_council_version <= ix.expected_current_council_version
                || ix.candidate_council_version == u64::MAX
                || ix.candidate_council_hash == [0; 32]
                || ix.expected_timing_profile_version == 0
                || ix.expected_timing_profile_hash == [0; 32]
            {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        CreateTargetAuthorityHandoff(ix) => {
            if ix.expected_proposal_id == 0
                || ix.expected_gate_epoch == 0
                || ix.expected_target_nonce == 0
                || ix.expected_council_version == 0
                || ix.expected_timing_profile_version == 0
                || ix.expected_timing_profile_hash == [0; 32]
            {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        CreateBootstrapActivation(ix) => {
            if ix.expected_proposal_id == 0
                || ix.expected_gate_epoch == 0
                || ix.expected_target_nonce == 0
                || ix.expected_council_version == 0
                || ix.expected_timing_profile_version == 0
                || ix.expected_timing_profile_hash == [0; 32]
            {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        ApproveTimingPolicyChange(ix) => ix.guard.validate()?,
        CancelTimingPolicyChange(ix) => {
            ix.guard.validate()?;
            if ix.cancellation_reason_code == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        ExpireTimingPolicyChange(ix) => ix.guard.validate()?,
        QueueTimingPolicyChange(ix) => ix.guard.validate()?,
        ExecuteTimingPolicyChange(ix) => ix.guard.validate()?,
        ApproveCouncilRotation(ix) => ix.guard.validate()?,
        CancelCouncilRotation(ix) => {
            ix.guard.validate()?;
            if ix.cancellation_reason_code == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        ExpireCouncilRotation(ix) => ix.guard.validate()?,
        QueueCouncilRotation(ix) => ix.guard.validate()?,
        ExecuteCouncilRotation(ix) => ix.guard.validate()?,
        ApproveTargetAuthorityHandoff(ix) => ix.guard.validate()?,
        CancelTargetAuthorityHandoff(ix) => {
            ix.guard.validate()?;
            if ix.cancellation_reason_code == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        ExpireTargetAuthorityHandoff(ix) => ix.guard.validate()?,
        QueueTargetAuthorityHandoff(ix) => ix.guard.validate()?,
        ExecuteTargetAuthorityHandoff(ix) => {
            ix.guard.validate()?;
            ix.envelope.validate()?;
        }
        ApproveBootstrapActivation(ix) => ix.guard.validate()?,
        CancelBootstrapActivation(ix) => {
            ix.guard.validate()?;
            if ix.cancellation_reason_code == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        ExpireBootstrapActivation(ix) => ix.guard.validate()?,
        QueueBootstrapActivation(ix) => ix.guard.validate()?,
        ExecuteBootstrapActivation(ix) => {
            ix.guard.validate()?;
            ix.envelope.validate()?;
        }
    }
    Ok(())
}
