use std::io::{Error, ErrorKind, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

use crate::{GovernanceError, GovernanceResult};

pub const CONTROLLER_CONFIG_DISCRIMINATOR: [u8; 8] = *b"AGVCFG01";
pub const GOVERNANCE_POLICY_DISCRIMINATOR: [u8; 8] = *b"AGVPOL01";
pub const GOVERNANCE_COUNCIL_DISCRIMINATOR: [u8; 8] = *b"AGVCNS01";
pub const PROTOCOL_GATE_DISCRIMINATOR: [u8; 8] = *b"AGVGAT01";
pub const UPGRADE_PROPOSAL_DISCRIMINATOR: [u8; 8] = *b"AGVPRP01";
pub const ACCOUNT_VERSION_V1: u8 = 1;

pub const CONTROLLER_CONFIG_RESERVED_LEN: usize = 28;
pub const GOVERNANCE_POLICY_RESERVED_LEN: usize = 21;
pub const COUNCIL_SEAT_RESERVED_LEN: usize = 47;
pub const GOVERNANCE_COUNCIL_RESERVED_LEN: usize = 26;
pub const PROTOCOL_GATE_RESERVED_LEN: usize = 2;
pub const UPGRADE_PROPOSAL_RESERVED_LEN: usize = 142;

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
pub enum GovernanceModeV1 {
    BootstrapCouncilOnly = 0,
}
fixed_u8_enum_borsh!(GovernanceModeV1 {
    BootstrapCouncilOnly = 0,
});

impl Default for GovernanceModeV1 {
    fn default() -> Self {
        Self::BootstrapCouncilOnly
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GateStatusV1 {
    Active = 0,
    FrozenForUpgrade = 1,
    EmergencyFrozen = 2,
}
fixed_u8_enum_borsh!(GateStatusV1 {
    Active = 0,
    FrozenForUpgrade = 1,
    EmergencyFrozen = 2,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CheckpointPhaseV1 {
    Prestate = 0,
    Poststate = 1,
}
fixed_u8_enum_borsh!(CheckpointPhaseV1 {
    Prestate = 0,
    Poststate = 1,
});

impl Default for CheckpointPhaseV1 {
    fn default() -> Self {
        Self::Prestate
    }
}

impl Default for GateStatusV1 {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProposalClassV1 {
    RoutineUpgrade = 0,
    EmergencyRollback = 1,
    EconomicChange = 2,
    ConstitutionalChange = 3,
    CouncilSetRotation = 4,
    TargetImmutability = 5,
}
fixed_u8_enum_borsh!(ProposalClassV1 {
    RoutineUpgrade = 0,
    EmergencyRollback = 1,
    EconomicChange = 2,
    ConstitutionalChange = 3,
    CouncilSetRotation = 4,
    TargetImmutability = 5,
});

impl Default for ProposalClassV1 {
    fn default() -> Self {
        Self::RoutineUpgrade
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProposalStateV1 {
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
}
fixed_u8_enum_borsh!(ProposalStateV1 {
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
});

impl Default for ProposalStateV1 {
    fn default() -> Self {
        Self::Draft
    }
}

impl ProposalStateV1 {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Expired)
    }

    pub const fn is_frozen_or_later(self) -> bool {
        matches!(
            self,
            Self::Frozen
                | Self::Extended
                | Self::UpgradeExecuted
                | Self::ProgramDataVerified
                | Self::PoststateAccepted
                | Self::UnfreezeApproved
                | Self::Completed
        )
    }
}

/// Proposal-side requirement. The future vote program's result mode intentionally
/// remains a separate enum with no `None` variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum VoteRequirementV1 {
    None = 0,
    Veto = 1,
    Affirmative = 2,
}
fixed_u8_enum_borsh!(VoteRequirementV1 {
    None = 0,
    Veto = 1,
    Affirmative = 2,
});

impl Default for VoteRequirementV1 {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct OptionalPubkeyV1 {
    pub present: bool,
    pub value: Pubkey,
}

impl OptionalPubkeyV1 {
    pub const LEN: usize = 33;

    pub fn none() -> Self {
        Self::default()
    }

    pub fn some(value: Pubkey) -> GovernanceResult<Self> {
        if value == Pubkey::default() {
            return Err(GovernanceError::DefaultPubkey);
        }
        Ok(Self {
            present: true,
            value,
        })
    }

    pub fn validate(&self) -> GovernanceResult<()> {
        match (self.present, self.value == Pubkey::default()) {
            (false, true) | (true, false) => Ok(()),
            _ => Err(GovernanceError::InvalidOptionalPubkey),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ControllerConfigV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub cluster_domain: [u8; 32],
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub authority_pda: Pubkey,
    pub gate_pda: Pubkey,
    pub canonical_spill_treasury: Pubkey,
    pub current_council_version: u64,
    pub current_policy_version: u64,
    pub next_proposal_id: u64,
    pub target_nonce: u64,
    pub guardian: Pubkey,
    pub vote_program: Pubkey,
    pub vote_programdata: Pubkey,
    pub vote_config: Pubkey,
    pub vote_mint: Pubkey,
    pub token_governance_enabled: bool,
    pub routine_delay_slots: u64,
    pub major_delay_slots: u64,
    pub rollback_delay_slots: u64,
    pub terminal_delay_slots: u64,
    /// Historical frozen ABI name. Release 1 interprets this exclusively as
    /// the council review window; token governance remains disabled.
    pub vote_review_slots: u64,
    pub proposal_expiry_slots: u64,
    pub policy_flags: u64,
    pub reserved: [u8; CONTROLLER_CONFIG_RESERVED_LEN],
}

// These are protocol-level non-degeneracy floors, not a production wall-clock
// schedule.  Production planning must apply a separately reviewed timing
// profile for the observed cluster.  Keeping the floor in the account
// validator prevents a one-slot review or timelock from becoming immutable.
pub const RELEASE1_MIN_ROLLBACK_DELAY_SLOTS: u64 = 5;
pub const RELEASE1_MIN_ROUTINE_DELAY_SLOTS: u64 = 10;
pub const RELEASE1_MIN_MAJOR_DELAY_SLOTS: u64 = 20;
pub const RELEASE1_MIN_TERMINAL_DELAY_SLOTS: u64 = 30;
pub const RELEASE1_MIN_COUNCIL_REVIEW_SLOTS: u64 = 7;
pub const RELEASE1_MIN_PROPOSAL_EXPIRY_SLOTS: u64 = 50;

impl ControllerConfigV1 {
    pub const LEN: usize = 512;

    pub const fn council_review_slots(&self) -> u64 {
        self.vote_review_slots
    }

    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &CONTROLLER_CONFIG_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        // The longest ordinary lifecycle begins one slot after creation,
        // spends a complete council-review window before the major timelock,
        // and must still retain another complete checkpoint-review window plus
        // two distinct Loader slots for checked extension and upgrade. Equality
        // is insufficient because execution at expiry is forbidden.
        let minimum_expiry_slots = 1u64
            .checked_add(self.vote_review_slots)
            .and_then(|slots| slots.checked_add(self.major_delay_slots))
            .and_then(|slots| slots.checked_add(self.vote_review_slots))
            .and_then(|slots| slots.checked_add(2))
            .ok_or(GovernanceError::InvalidControllerConfig)?;
        if self.cluster_domain == [0; 32]
            || [
                self.target_program,
                self.target_programdata,
                self.upgradeable_loader,
                self.authority_pda,
                self.gate_pda,
                self.canonical_spill_treasury,
                self.guardian,
            ]
            .contains(&Pubkey::default())
            || self.current_council_version == 0
            || self.current_policy_version == 0
            || self.next_proposal_id == 0
            || self.target_nonce == 0
            || self.routine_delay_slots < RELEASE1_MIN_ROUTINE_DELAY_SLOTS
            || self.major_delay_slots < RELEASE1_MIN_MAJOR_DELAY_SLOTS
            || self.rollback_delay_slots < RELEASE1_MIN_ROLLBACK_DELAY_SLOTS
            || self.terminal_delay_slots < RELEASE1_MIN_TERMINAL_DELAY_SLOTS
            || self.vote_review_slots < RELEASE1_MIN_COUNCIL_REVIEW_SLOTS
            || self.proposal_expiry_slots < RELEASE1_MIN_PROPOSAL_EXPIRY_SLOTS
            || self.rollback_delay_slots > self.routine_delay_slots
            || self.routine_delay_slots > self.major_delay_slots
            || self.major_delay_slots > self.terminal_delay_slots
            || self.major_delay_slots >= self.proposal_expiry_slots
            || self.vote_review_slots >= self.proposal_expiry_slots
            || minimum_expiry_slots >= self.proposal_expiry_slots
            || self.policy_flags != 0
        {
            return Err(GovernanceError::InvalidControllerConfig);
        }
        let vote_keys = [
            self.vote_program,
            self.vote_programdata,
            self.vote_config,
            self.vote_mint,
        ];
        if self.token_governance_enabled || vote_keys.iter().any(|key| *key != Pubkey::default()) {
            return Err(GovernanceError::InvalidControllerConfig);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GovernancePolicyV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_config: Pubkey,
    pub version: u64,
    pub target_program: Pubkey,
    pub activation_slot: u64,
    pub council_size: u8,
    pub routine_threshold: u8,
    pub terminal_threshold: u8,
    pub governance_mode: GovernanceModeV1,
    pub policy_flags: u8,
    pub veto_quorum_bps: u16,
    pub affirmative_quorum_bps: u16,
    pub affirmative_approval_bps: u16,
    pub routine_requires_vote: bool,
    pub economic_requires_vote: bool,
    pub constitutional_requires_vote: bool,
    pub rotation_requires_vote: bool,
    pub immutability_requires_vote: bool,
    pub policy_hash: [u8; 32],
    pub reserved: [u8; GOVERNANCE_POLICY_RESERVED_LEN],
}

impl GovernancePolicyV1 {
    pub const LEN: usize = 160;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CouncilSeatV1 {
    pub seat_authority: Pubkey,
    pub term_start_slot: u64,
    pub term_end_slot: u64,
    pub active: bool,
    pub reserved: [u8; COUNCIL_SEAT_RESERVED_LEN],
}

impl CouncilSeatV1 {
    pub const LEN: usize = 96;

    pub const fn term_covers(&self, slot: u64) -> bool {
        self.active && self.term_start_slot <= slot && slot < self.term_end_slot
    }
}

impl Default for CouncilSeatV1 {
    fn default() -> Self {
        Self {
            seat_authority: Pubkey::default(),
            term_start_slot: 0,
            term_end_slot: 0,
            active: false,
            reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct GovernanceCouncilSetV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller_config: Pubkey,
    pub version: u64,
    pub target_program: Pubkey,
    pub activation_slot: u64,
    pub deactivation_slot: u64,
    pub seats: [CouncilSeatV1; 5],
    pub routine_threshold: u8,
    pub terminal_threshold: u8,
    pub policy_flags: u8,
    pub set_hash: [u8; 32],
    pub reserved: [u8; GOVERNANCE_COUNCIL_RESERVED_LEN],
}

impl GovernanceCouncilSetV1 {
    pub const LEN: usize = 640;

    pub const fn active_at(&self, slot: u64) -> bool {
        self.activation_slot <= slot
            && (self.deactivation_slot == 0 || slot < self.deactivation_slot)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ProtocolGateV1 {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub status: GateStatusV1,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub epoch: u64,
    pub active_proposal: Pubkey,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub last_completed_proposal: Pubkey,
    pub reserved: [u8; PROTOCOL_GATE_RESERVED_LEN],
}

impl ProtocolGateV1 {
    pub const LEN: usize = 192;

    pub fn validate_static(&self) -> GovernanceResult<()> {
        validate_header(
            &self.discriminator,
            &PROTOCOL_GATE_DISCRIMINATOR,
            self.version,
            self.initialized,
            &self.reserved,
        )?;
        if [
            self.controller_config,
            self.target_program,
            self.target_programdata,
        ]
        .contains(&Pubkey::default())
            || self.epoch == 0
        {
            return Err(GovernanceError::InvalidGateState);
        }
        let active_is_canonical = self.active_proposal == Pubkey::default()
            && self.freeze_slot == 0
            && self.freeze_reason_code == 0;
        let upgrade_frozen_is_canonical = self.active_proposal != Pubkey::default()
            && self.freeze_slot != 0
            && self.freeze_reason_code != 0;
        let emergency_frozen_is_canonical = self.active_proposal == Pubkey::default()
            && self.freeze_slot != 0
            && self.freeze_reason_code != 0;
        if match self.status {
            GateStatusV1::Active => !active_is_canonical,
            GateStatusV1::FrozenForUpgrade => !upgrade_frozen_is_canonical,
            GateStatusV1::EmergencyFrozen => !emergency_frozen_is_canonical,
        } {
            return Err(GovernanceError::InvalidGateState);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct UpgradeProposalV1 {
    pub discriminator: [u8; 8],
    pub account_version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub proposal_id: u64,
    pub target_nonce: u64,
    pub proposal_class: ProposalClassV1,
    pub state: ProposalStateV1,
    pub cluster_domain: [u8; 32],
    pub controller_program: Pubkey,
    pub controller_config: Pubkey,
    pub protocol_gate: Pubkey,
    pub policy_version: u64,
    pub policy_hash: [u8; 32],
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub creation_gate_epoch: u64,
    pub freeze_gate_epoch: u64,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub authority_pda: Pubkey,
    pub canonical_spill_treasury: Pubkey,
    pub buffer_pubkey: Pubkey,
    pub buffer_loader_owner: Pubkey,
    pub buffer_authority: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub source_commit_hash: [u8; 32],
    pub source_tree_hash: [u8; 32],
    pub build_input_inventory_hash: [u8; 32],
    pub reproducible_build_receipt_hash: [u8; 32],
    pub package_receipt_hash: [u8; 32],
    pub release_intent_hash: [u8; 32],
    pub current_deployed_payload_hash: [u8; 32],
    pub current_raw_programdata_hash: [u8; 32],
    pub deployed_slot: u64,
    pub current_capacity: u64,
    pub extension_delta: u64,
    pub expected_post_capacity: u64,
    pub prestate_checkpoint: Pubkey,
    pub required_poststate_checkpoint: Pubkey,
    pub rollback_proposal: OptionalPubkeyV1,
    pub rollback_buffer: OptionalPubkeyV1,
    pub rollback_artifact_hash: [u8; 32],
    pub vote_requirement: VoteRequirementV1,
    pub vote_program: Pubkey,
    pub vote_result_pda: Pubkey,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub council_approval_bitset: u8,
    pub council_approval_count: u8,
    pub poststate_approval_bitset: u8,
    pub poststate_approval_count: u8,
    pub unfreeze_approval_bitset: u8,
    pub unfreeze_approval_count: u8,
    pub proposal_digest: [u8; 32],
    pub cancellation_reason_code: u16,
    pub terminal_reason_code: u16,
    pub reserved: [u8; UPGRADE_PROPOSAL_RESERVED_LEN],
}

impl UpgradeProposalV1 {
    pub const LEN: usize = 1280;

    pub const fn is_code_upgrade(&self) -> bool {
        self.artifact_length != 0
    }

    pub const fn vote_required(&self) -> bool {
        !matches!(self.vote_requirement, VoteRequirementV1::None)
    }

    pub const fn extension_required(&self) -> bool {
        self.extension_delta != 0
    }
}

pub(crate) fn validate_header<const N: usize>(
    actual_discriminator: &[u8; 8],
    expected_discriminator: &[u8; 8],
    version: u8,
    initialized: bool,
    reserved: &[u8; N],
) -> GovernanceResult<()> {
    if actual_discriminator != expected_discriminator {
        return Err(GovernanceError::InvalidDiscriminator);
    }
    if version != ACCOUNT_VERSION_V1 {
        return Err(GovernanceError::UnsupportedVersion);
    }
    if !initialized {
        return Err(GovernanceError::Uninitialized);
    }
    validate_reserved(reserved)
}

pub(crate) fn validate_reserved<const N: usize>(reserved: &[u8; N]) -> GovernanceResult<()> {
    if reserved.iter().any(|byte| *byte != 0) {
        Err(GovernanceError::NonzeroReserved)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use borsh::{BorshDeserialize, BorshSerialize};

    use super::*;

    fn serialize_len<T: BorshSerialize>(value: &T) -> usize {
        value.try_to_vec().expect("serialize").len()
    }

    #[test]
    fn discriminators_are_unique_and_exact() {
        let values = [
            CONTROLLER_CONFIG_DISCRIMINATOR,
            GOVERNANCE_POLICY_DISCRIMINATOR,
            GOVERNANCE_COUNCIL_DISCRIMINATOR,
            PROTOCOL_GATE_DISCRIMINATOR,
            UPGRADE_PROPOSAL_DISCRIMINATOR,
        ];
        assert_eq!(values.iter().collect::<BTreeSet<_>>().len(), values.len());
        assert!(values.iter().all(|value| value.len() == 8));
    }

    #[test]
    fn optional_pubkey_is_fixed_width_and_canonical() {
        assert_eq!(
            serialize_len(&OptionalPubkeyV1::none()),
            OptionalPubkeyV1::LEN
        );
        let some = OptionalPubkeyV1::some(Pubkey::new_from_array([7; 32])).unwrap();
        assert_eq!(serialize_len(&some), OptionalPubkeyV1::LEN);
        assert_eq!(some.validate(), Ok(()));
        assert_eq!(
            OptionalPubkeyV1 {
                present: false,
                value: Pubkey::new_from_array([1; 32]),
            }
            .validate(),
            Err(GovernanceError::InvalidOptionalPubkey)
        );
    }

    #[test]
    fn enum_bytes_are_stable_and_unknown_values_fail() {
        assert_eq!(
            ProposalClassV1::TargetImmutability.try_to_vec().unwrap(),
            [5]
        );
        assert_eq!(ProposalStateV1::Expired.try_to_vec().unwrap(), [15]);
        assert_eq!(VoteRequirementV1::Affirmative.try_to_vec().unwrap(), [2]);
        assert_eq!(CheckpointPhaseV1::Poststate.try_to_vec().unwrap(), [1]);
        assert!(ProposalClassV1::try_from_slice(&[6]).is_err());
        assert!(ProposalStateV1::try_from_slice(&[16]).is_err());
        assert!(VoteRequirementV1::try_from_slice(&[3]).is_err());
        assert!(CheckpointPhaseV1::try_from_slice(&[2]).is_err());
    }
}
