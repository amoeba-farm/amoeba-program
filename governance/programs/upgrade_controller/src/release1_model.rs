//! Pure Release 1 governance reference state machine.
//!
//! This module deliberately has no account, clock, instruction, CPI, or Loader
//! access.  It is an executable specification used to compare future processor
//! transitions against the authorized Release 1 lifecycle.  [`Release1Model::apply`]
//! evaluates every action on a clone and commits the clone only after success,
//! making failure atomicity an invariant of the model itself.

use std::collections::{BTreeMap, BTreeSet};

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, pubkey::Pubkey};
use thiserror::Error;

use crate::{
    release1_state::{
        CouncilRotationStateV1, EmergencyFreezeResolutionStateV1, ProposalStateV2,
        StateCheckpointPhaseV1, BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
        COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1, COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1,
        EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1,
        EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1, LOADER_V3_PROGRAMDATA_METADATA_LEN_V1,
        LOADER_V3_PROGRAM_ACCOUNT_LEN_V1, MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
    },
    state::{GateStatusV1, ProposalClassV1},
};

pub const MODEL_COUNCIL_SIZE: usize = 5;
pub const MODEL_ROUTINE_THRESHOLD: u8 = 3;
pub const MODEL_TERMINAL_THRESHOLD: u8 = 4;
pub const MODEL_VALID_APPROVAL_MASK: u8 = 0b0001_1111;
pub const MODEL_GOVERNED_FREEZE_REASON: u16 = 2;
pub const MODEL_EXPIRED_TERMINAL_REASON: u16 = 1;
pub const MODEL_COMPLETED_TERMINAL_REASON: u16 = 2;
pub const MODEL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON: u16 = 3;
pub const MODEL_RETIRED_ROLLBACK_TERMINAL_REASON: u16 = 4;

const COUNCIL_HASH_DOMAIN: &[u8] = b"AMOEBA_RELEASE1_MODEL_COUNCIL_V1";
const HARD_COMBINED_ROOT_DOMAIN: &[u8] = b"AMOEBA_RELEASE1_MODEL_HARD_COMBINED_V1";

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum Release1ModelError {
    #[error("the model is already initialized")]
    AlreadyInitialized,
    #[error("the model is not initialized")]
    NotInitialized,
    #[error("initialization evidence or identity graph is invalid")]
    InvalidInitialization,
    #[error("configured timing is invalid")]
    InvalidTiming,
    #[error("council shape or identity is invalid")]
    InvalidCouncil,
    #[error("proposal class is unsupported in Release 1")]
    UnsupportedProposalClass,
    #[error("token governance is disabled in Release 1")]
    TokenGovernanceDisabled,
    #[error("the requested state transition is invalid")]
    InvalidStateTransition,
    #[error("proposal or auxiliary governance object was not found")]
    NotFound,
    #[error("proposal id, target nonce, council, or gate epoch is stale")]
    StaleBinding,
    #[error("approval seat or encoding is invalid")]
    InvalidApproval,
    #[error("the selected seat has already approved")]
    DuplicateApproval,
    #[error("the selected council seat is outside its active term")]
    InactiveSeat,
    #[error("the proposal creator is not an active signed council seat with a separate payer")]
    InvalidProposer,
    #[error("three-of-five quorum is not satisfied")]
    QuorumNotSatisfied,
    #[error("the proposal is outside its review, timelock, or expiry window")]
    TimingViolation,
    #[error("the gate is not in the required canonical state")]
    InvalidGate,
    #[error("a required checkpoint is absent or not accepted")]
    CheckpointNotAccepted,
    #[error("the rollback pair is absent or not reciprocally linked")]
    InvalidRollbackLink,
    #[error("the observed ProgramData identity or bytes changed")]
    ProgramDataChanged,
    #[error("typed ProgramData or hard-poststate failure evidence is absent or stale")]
    ProgramDataFailureEvidenceRequired,
    #[error("the bound poststate hard commitments differ from accepted prestate")]
    HardPoststateMismatch,
    #[error("checked arithmetic overflowed")]
    ArithmeticOverflow,
}

pub type Release1ModelResult<T> = Result<T, Release1ModelError>;

#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelIdentityGraph {
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub authority_pda: Pubkey,
    pub gate_pda: Pubkey,
    pub policy_pda: Pubkey,
    pub council_pda: Pubkey,
    pub canonical_spill_treasury: Pubkey,
    pub guardian: Pubkey,
}

impl ModelIdentityGraph {
    fn validate(&self) -> Release1ModelResult<()> {
        let keys = [
            self.controller_program,
            self.controller_programdata,
            self.target_program,
            self.target_programdata,
            self.upgradeable_loader,
            self.authority_pda,
            self.gate_pda,
            self.policy_pda,
            self.council_pda,
            self.canonical_spill_treasury,
            self.guardian,
        ];
        if keys.contains(&Pubkey::default()) {
            return Err(Release1ModelError::InvalidInitialization);
        }
        let distinct = keys
            .iter()
            .map(|key| key.to_bytes())
            .collect::<BTreeSet<_>>();
        if distinct.len() != keys.len() {
            return Err(Release1ModelError::InvalidInitialization);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelDelays {
    pub review_slots: u64,
    pub rollback_slots: u64,
    pub routine_slots: u64,
    pub major_slots: u64,
    pub terminal_slots: u64,
    pub proposal_expiry_slots: u64,
}

impl ModelDelays {
    fn validate(&self) -> Release1ModelResult<()> {
        if self.review_slots == 0
            || self.rollback_slots == 0
            || self.routine_slots == 0
            || self.major_slots == 0
            || self.terminal_slots == 0
            || self.proposal_expiry_slots == 0
            || self.rollback_slots > self.routine_slots
            || self.routine_slots > self.major_slots
            || self.major_slots > self.terminal_slots
        {
            return Err(Release1ModelError::InvalidTiming);
        }
        let required = 1u64
            .checked_add(self.review_slots)
            .and_then(|value| value.checked_add(self.major_slots))
            .and_then(|value| value.checked_add(self.review_slots))
            // Worst-case frozen execution reserves the protected-checkpoint
            // window, checked extension, and a strictly later upgrade slot.
            .and_then(|value| value.checked_add(2))
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if self.proposal_expiry_slots <= required {
            return Err(Release1ModelError::InvalidTiming);
        }
        Ok(())
    }

    pub fn class_delay(&self, class: ProposalClassV1) -> Release1ModelResult<u64> {
        match class {
            ProposalClassV1::EmergencyRollback => Ok(self.rollback_slots),
            ProposalClassV1::RoutineUpgrade => Ok(self.routine_slots),
            ProposalClassV1::EconomicChange | ProposalClassV1::ConstitutionalChange => {
                Ok(self.major_slots)
            }
            ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => {
                Err(Release1ModelError::UnsupportedProposalClass)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelSeatTerm {
    pub start_slot: u64,
    pub end_slot: u64,
}

impl ModelSeatTerm {
    fn validate_at(&self, slot: u64) -> Release1ModelResult<()> {
        if self.start_slot >= self.end_slot || !self.covers(slot) {
            return Err(Release1ModelError::InactiveSeat);
        }
        Ok(())
    }

    pub const fn covers(&self, slot: u64) -> bool {
        self.start_slot <= slot && slot < self.end_slot
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelCouncil {
    pub version: u64,
    pub activation_slot: u64,
    pub seats: [Pubkey; MODEL_COUNCIL_SIZE],
    pub seat_terms: [ModelSeatTerm; MODEL_COUNCIL_SIZE],
    pub hash: [u8; 32],
}

impl ModelCouncil {
    pub fn new(
        version: u64,
        activation_slot: u64,
        seats: [Pubkey; MODEL_COUNCIL_SIZE],
        seat_terms: [ModelSeatTerm; MODEL_COUNCIL_SIZE],
    ) -> Release1ModelResult<Self> {
        if version == 0 || activation_slot == 0 || seats.contains(&Pubkey::default()) {
            return Err(Release1ModelError::InvalidCouncil);
        }
        let unique = seats
            .iter()
            .map(|seat| seat.to_bytes())
            .collect::<BTreeSet<_>>();
        if unique.len() != MODEL_COUNCIL_SIZE {
            return Err(Release1ModelError::InvalidCouncil);
        }
        for term in &seat_terms {
            term.validate_at(activation_slot)?;
        }
        let hash = model_council_hash(version, activation_slot, &seats, &seat_terms);
        Ok(Self {
            version,
            activation_slot,
            seats,
            seat_terms,
            hash,
        })
    }

    fn validate(&self) -> Release1ModelResult<()> {
        if Self::new(
            self.version,
            self.activation_slot,
            self.seats,
            self.seat_terms,
        )? != *self
        {
            return Err(Release1ModelError::InvalidCouncil);
        }
        Ok(())
    }

    fn require_seat_active(&self, seat: u8, slot: u64) -> Release1ModelResult<()> {
        let index = usize::from(seat);
        if index >= MODEL_COUNCIL_SIZE {
            return Err(Release1ModelError::InvalidApproval);
        }
        if slot < self.activation_slot {
            return Err(Release1ModelError::InactiveSeat);
        }
        self.seat_terms[index].validate_at(slot)
    }

    fn require_mask_active(&self, bitset: u8, slot: u64) -> Release1ModelResult<()> {
        if bitset & !MODEL_VALID_APPROVAL_MASK != 0 {
            return Err(Release1ModelError::InvalidApproval);
        }
        for seat in 0..MODEL_COUNCIL_SIZE {
            if bitset & (1u8 << seat) != 0 {
                self.require_seat_active(seat as u8, slot)?;
            }
        }
        Ok(())
    }
}

pub fn model_council_hash(
    version: u64,
    activation_slot: u64,
    seats: &[Pubkey; MODEL_COUNCIL_SIZE],
    seat_terms: &[ModelSeatTerm; MODEL_COUNCIL_SIZE],
) -> [u8; 32] {
    let version_bytes = version.to_le_bytes();
    let activation_bytes = activation_slot.to_le_bytes();
    let term_bytes = seat_terms.map(|term| {
        let mut encoded = [0u8; 16];
        encoded[..8].copy_from_slice(&term.start_slot.to_le_bytes());
        encoded[8..].copy_from_slice(&term.end_slot.to_le_bytes());
        encoded
    });
    let mut values: Vec<&[u8]> = Vec::with_capacity(MODEL_COUNCIL_SIZE * 2 + 3);
    values.push(COUNCIL_HASH_DOMAIN);
    values.push(&version_bytes);
    values.push(&activation_bytes);
    for (seat, term) in seats.iter().zip(term_bytes.iter()) {
        values.push(seat.as_ref());
        values.push(term);
    }
    hashv(&values).to_bytes()
}

#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelProgramDataObservation {
    pub slot: u64,
    pub payload_hash: [u8; 32],
    pub raw_hash: [u8; 32],
    pub capacity: u64,
    pub authority: Pubkey,
}

impl ModelProgramDataObservation {
    fn validate(&self) -> Release1ModelResult<()> {
        if self.slot == 0
            || self.payload_hash == [0; 32]
            || self.raw_hash == [0; 32]
            || self.capacity == 0
            || self.authority == Pubkey::default()
        {
            return Err(Release1ModelError::InvalidInitialization);
        }
        Ok(())
    }
}

/// Freeze-time target Program and ProgramData evidence.  Header flags mean
/// "canonical-valid and representable Loader-v3 header", not merely that a tag
/// byte could be decoded.  This lets GuardianFreeze persist malformed accounts:
/// exact owner/executable/length metadata and a bounded raw commitment remain
/// available while invalid optional keys are represented by `header=false`.
#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelGuardianProgramDataObservation {
    pub program_owner: Pubkey,
    pub program_executable: bool,
    pub program_data_length: u64,
    pub program_header_present: bool,
    pub linked_programdata: Option<Pubkey>,
    pub programdata_owner: Pubkey,
    pub programdata_executable: bool,
    pub programdata_data_length: u64,
    pub programdata_header_present: bool,
    pub slot: u64,
    pub raw_hash_complete: bool,
    pub raw_hash: [u8; 32],
    pub capacity: u64,
    pub authority: Option<Pubkey>,
}

impl ModelGuardianProgramDataObservation {
    pub fn canonical(
        graph: &ModelIdentityGraph,
        observation: &ModelProgramDataObservation,
        authority: Option<Pubkey>,
    ) -> Release1ModelResult<Self> {
        observation.validate()?;
        if authority == Some(Pubkey::default()) {
            return Err(Release1ModelError::InvalidInitialization);
        }
        let programdata_data_length = observation
            .capacity
            .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let canonical = Self {
            program_owner: graph.upgradeable_loader,
            program_executable: true,
            program_data_length: LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
            program_header_present: true,
            linked_programdata: Some(graph.target_programdata),
            programdata_owner: graph.upgradeable_loader,
            programdata_executable: false,
            programdata_data_length,
            programdata_header_present: true,
            slot: observation.slot,
            raw_hash_complete: true,
            raw_hash: observation.raw_hash,
            capacity: observation.capacity,
            authority,
        };
        canonical.validate_persistable()?;
        Ok(canonical)
    }

    fn validate_persistable(&self) -> Release1ModelResult<()> {
        if self.linked_programdata == Some(Pubkey::default())
            || self.authority == Some(Pubkey::default())
        {
            return Err(Release1ModelError::InvalidInitialization);
        }
        if self.program_header_present {
            if self.program_data_length != LOADER_V3_PROGRAM_ACCOUNT_LEN_V1
                || self.linked_programdata.is_none()
            {
                return Err(Release1ModelError::InvalidInitialization);
            }
        } else if self.linked_programdata.is_some() {
            return Err(Release1ModelError::InvalidInitialization);
        }
        if self.programdata_header_present {
            if self.programdata_data_length
                != self
                    .capacity
                    .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
                    .ok_or(Release1ModelError::ArithmeticOverflow)?
            {
                return Err(Release1ModelError::InvalidInitialization);
            }
        } else if self.slot != 0 || self.capacity != 0 || self.authority.is_some() {
            return Err(Release1ModelError::InvalidInitialization);
        }
        let within_atomic_ceiling =
            self.programdata_data_length <= MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
        if self.raw_hash_complete != within_atomic_ceiling
            || (self.raw_hash_complete && self.raw_hash == [0; 32])
            || (!self.raw_hash_complete && self.raw_hash != [0; 32])
        {
            return Err(Release1ModelError::InvalidInitialization);
        }
        Ok(())
    }

    fn is_canonical_for(
        &self,
        graph: &ModelIdentityGraph,
        observation: &ModelProgramDataObservation,
        authority: Option<Pubkey>,
    ) -> bool {
        Self::canonical(graph, observation, authority)
            .map(|canonical| canonical == *self)
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelEmergencyFreezeObservation {
    pub epoch: u64,
    pub freeze_slot: u64,
    pub freeze_reason: u16,
    pub programdata: ModelGuardianProgramDataObservation,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelHardStateObservation {
    pub schema_identifier: [u8; 32],
    pub program_owned_root: [u8; 32],
    pub program_owned_count: u64,
    pub logical_compressed_root: [u8; 32],
    pub logical_compressed_count: u64,
    pub semantic_custody_root: [u8; 32],
    pub custody_identity_root: [u8; 32],
    pub hard_combined_root: [u8; 32],
}

impl ModelHardStateObservation {
    pub fn recompute_hard_combined_root(&self) -> [u8; 32] {
        hashv(&[
            HARD_COMBINED_ROOT_DOMAIN,
            &self.schema_identifier,
            &self.program_owned_root,
            &self.program_owned_count.to_le_bytes(),
            &self.logical_compressed_root,
            &self.logical_compressed_count.to_le_bytes(),
            &self.semantic_custody_root,
            &self.custody_identity_root,
        ])
        .to_bytes()
    }

    fn validate(&self) -> Release1ModelResult<()> {
        if self.schema_identifier == [0; 32]
            || self.program_owned_root == [0; 32]
            || self.logical_compressed_root == [0; 32]
            || self.semantic_custody_root == [0; 32]
            || self.custody_identity_root == [0; 32]
            || self.hard_combined_root == [0; 32]
            || self.hard_combined_root != self.recompute_hard_combined_root()
        {
            return Err(Release1ModelError::InvalidInitialization);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelGate {
    pub status: GateStatusV1,
    pub epoch: u64,
    pub active_proposal: Option<u64>,
    pub freeze_slot: u64,
    pub freeze_reason: u16,
    pub last_completed_proposal: Option<u64>,
}

impl ModelGate {
    fn validate(&self) -> Release1ModelResult<()> {
        if self.epoch == 0 {
            return Err(Release1ModelError::InvalidGate);
        }
        let canonical = match self.status {
            GateStatusV1::Active => {
                self.active_proposal.is_none() && self.freeze_slot == 0 && self.freeze_reason == 0
            }
            GateStatusV1::FrozenForUpgrade => {
                self.active_proposal.is_some() && self.freeze_slot != 0 && self.freeze_reason != 0
            }
            GateStatusV1::EmergencyFrozen => {
                self.active_proposal.is_none() && self.freeze_slot != 0 && self.freeze_reason != 0
            }
        };
        if !canonical {
            return Err(Release1ModelError::InvalidGate);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelTiming {
    pub creation_slot: u64,
    pub review_start_slot: u64,
    pub review_end_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelApprovalAccumulator {
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub bitset: u8,
    pub count: u8,
}

impl ModelApprovalAccumulator {
    fn pinned(council: &ModelCouncil) -> Self {
        Self {
            council_version: council.version,
            council_hash: council.hash,
            bitset: 0,
            count: 0,
        }
    }

    fn validate(&self) -> Release1ModelResult<()> {
        if self.bitset & !MODEL_VALID_APPROVAL_MASK != 0
            || self.count != self.bitset.count_ones() as u8
            || ((self.council_version == 0) != (self.council_hash == [0; 32]))
            || (self.council_version == 0 && (self.bitset != 0 || self.count != 0))
        {
            return Err(Release1ModelError::InvalidApproval);
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.count >= MODEL_ROUTINE_THRESHOLD
    }

    fn is_complete_at(&self, council: &ModelCouncil, slot: u64) -> Release1ModelResult<bool> {
        self.validate()?;
        if self.council_version != council.version || self.council_hash != council.hash {
            return Err(Release1ModelError::StaleBinding);
        }
        council.require_mask_active(self.bitset, slot)?;
        Ok(self.is_complete())
    }

    fn record_pinned(
        &mut self,
        council: &ModelCouncil,
        seat: u8,
        slot: u64,
    ) -> Release1ModelResult<bool> {
        self.validate()?;
        if self.council_version != council.version || self.council_hash != council.hash {
            return Err(Release1ModelError::StaleBinding);
        }
        council.require_seat_active(seat, slot)?;
        self.record_seat(seat)
    }

    fn record_current(
        &mut self,
        council: &ModelCouncil,
        seat: u8,
        slot: u64,
    ) -> Release1ModelResult<bool> {
        self.validate()?;
        if self.council_version != council.version || self.council_hash != council.hash {
            *self = Self::pinned(council);
        }
        council.require_seat_active(seat, slot)?;
        self.record_seat(seat)
    }

    fn record_seat(&mut self, seat: u8) -> Release1ModelResult<bool> {
        if usize::from(seat) >= MODEL_COUNCIL_SIZE {
            return Err(Release1ModelError::InvalidApproval);
        }
        let bit = 1u8 << seat;
        if self.bitset & bit != 0 {
            return Err(Release1ModelError::DuplicateApproval);
        }
        let was_complete = self.is_complete();
        self.bitset |= bit;
        self.count = self.bitset.count_ones() as u8;
        Ok(!was_complete && self.is_complete())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelCheckpoint {
    pub phase: StateCheckpointPhaseV1,
    pub gate_epoch: u64,
    pub hard_state: ModelHardStateObservation,
    pub forbidden_drift_count: u32,
    pub approvals: ModelApprovalAccumulator,
    pub finalized: bool,
    pub accepted: bool,
    pub rejected_hard_mismatch: bool,
    pub accepted_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelCheckpointAttestation {
    pub subject_id: u64,
    pub phase: StateCheckpointPhaseV1,
    pub gate_epoch: u64,
    pub council_version: u64,
    pub council_hash: [u8; 32],
    pub seat: u8,
    pub seat_authority: Pubkey,
    pub hard_state: ModelHardStateObservation,
    pub forbidden_drift_count: u32,
    pub attested_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelProposal {
    pub id: u64,
    pub class: ProposalClassV1,
    pub state: ProposalStateV2,
    pub target_nonce: u64,
    pub creation_gate_status: GateStatusV1,
    pub creation_gate_epoch: u64,
    pub creation_emergency_observation: Option<ModelEmergencyFreezeObservation>,
    pub creation_programdata: ModelProgramDataObservation,
    pub freeze_gate_epoch: u64,
    pub creation_council_version: u64,
    pub creation_council_hash: [u8; 32],
    pub timing: ModelTiming,
    pub extension_required: bool,
    pub primary_proposal: Option<u64>,
    pub rollback_proposal: Option<u64>,
    pub initial_approvals: ModelApprovalAccumulator,
    pub cancellation_approvals: ModelApprovalAccumulator,
    pub unfreeze_approvals: ModelApprovalAccumulator,
    pub prestate: Option<ModelCheckpoint>,
    pub poststate: Option<ModelCheckpoint>,
    pub prestate_attestations: Vec<ModelCheckpointAttestation>,
    pub poststate_attestations: Vec<ModelCheckpointAttestation>,
    pub frozen_slot: u64,
    pub extended_slot: u64,
    pub upgraded_slot: u64,
    pub programdata_verified_slot: u64,
    pub programdata_failure: Option<ModelProgramDataFailureEvidence>,
    pub terminal_slot: u64,
    pub cancellation_reason: u16,
    pub terminal_reason: u16,
}

impl ModelProposal {
    fn is_pre_freeze(&self) -> bool {
        matches!(
            self.state,
            ProposalStateV2::Draft
                | ProposalStateV2::BufferAdopted
                | ProposalStateV2::BufferVerified
                | ProposalStateV2::CouncilApproved
                | ProposalStateV2::GovernanceSatisfied
                | ProposalStateV2::Timelocked
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelCouncilRotation {
    pub id: u64,
    pub state: CouncilRotationStateV1,
    pub current_council_version: u64,
    pub current_council_hash: [u8; 32],
    pub candidate: ModelCouncil,
    pub target_nonce: u64,
    pub creation_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub approvals: ModelApprovalAccumulator,
    pub cancellation_approvals: ModelApprovalAccumulator,
    pub cancellation_reason: u16,
    pub terminal_slot: u64,
    pub terminal_reason: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelEmergencyResolution {
    pub id: u64,
    pub state: EmergencyFreezeResolutionStateV1,
    pub frozen_epoch: u64,
    pub freeze_slot: u64,
    pub freeze_reason: u16,
    pub target_nonce: u64,
    pub creation_slot: u64,
    pub not_before_slot: u64,
    pub expiry_slot: u64,
    pub observed_programdata: ModelGuardianProgramDataObservation,
    pub approvals: ModelApprovalAccumulator,
    pub checkpoint: Option<ModelCheckpoint>,
    pub checkpoint_attestations: Vec<ModelCheckpointAttestation>,
    pub executed_slot: u64,
    pub terminal_reason: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub enum ModelProgramDataFailureKind {
    DeployedSlot,
    Authority,
    Capacity,
    PayloadChunk,
    MerkleRoot,
    NonzeroTail,
    RawCommitment,
    IncompleteVerification,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelProgramDataFailureEvidence {
    pub proposal_id: u64,
    pub gate_epoch: u64,
    pub observed_slot: u64,
    pub kind: ModelProgramDataFailureKind,
    pub observed_programdata: ModelProgramDataObservation,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ModelInitialization {
    pub slot: u64,
    pub graph: ModelIdentityGraph,
    pub seats: [Pubkey; MODEL_COUNCIL_SIZE],
    pub seat_terms: [ModelSeatTerm; MODEL_COUNCIL_SIZE],
    pub delays: ModelDelays,
    pub programdata: ModelProgramDataObservation,
    pub controller_programdata_linked: bool,
    pub initializer_is_controller_upgrade_authority: bool,
    pub target_programdata_linked: bool,
    pub canonical_pdas_verified: bool,
    pub seat_accounts_readonly: bool,
    pub seat_accounts_nonexecutable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelProposalRequest {
    pub class: ProposalClassV1,
    pub extension_required: bool,
    pub primary_proposal: Option<u64>,
    pub rollback_proposal: Option<u64>,
    pub proposer_seat: u8,
    pub proposer_signed: bool,
    pub payer_is_separate: bool,
    pub slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Release1ModelAction {
    Initialize(Box<ModelInitialization>),
    /// Test-harness-only simulation of the future bridge installation and
    /// ProgramData-authority handoff ceremony. There is deliberately no
    /// production controller instruction corresponding to this model action.
    SimulatePostHandoffActivation {
        slot: u64,
        bridge_and_authority_graph_verified: bool,
    },
    SetProgramDataObservation(ModelProgramDataObservation),
    SetProgramDataAuthorityObservation(Option<Pubkey>),
    /// Test-harness observation of the exact runtime Program/ProgramData
    /// accounts. GuardianFreeze must remain available even when this evidence
    /// is malformed or too large for one-transaction hashing.
    SetGuardianRuntimeObservation(ModelGuardianProgramDataObservation),
    CreateProposal(ModelProposalRequest),
    AdoptBuffer {
        proposal_id: u64,
    },
    VerifyBuffer {
        proposal_id: u64,
    },
    ApproveProposal {
        proposal_id: u64,
        seat: u8,
        slot: u64,
    },
    SatisfyGovernance {
        proposal_id: u64,
        slot: u64,
    },
    QueueProposal {
        proposal_id: u64,
        slot: u64,
    },
    ApproveCancellation {
        proposal_id: u64,
        seat: u8,
        slot: u64,
        reason: u16,
    },
    ExpireProposal {
        proposal_id: u64,
        slot: u64,
    },
    FreezeProposal {
        proposal_id: u64,
        slot: u64,
    },
    ConvertEmergencyFreeze {
        proposal_id: u64,
        slot: u64,
    },
    AttestCheckpoint {
        proposal_id: u64,
        phase: StateCheckpointPhaseV1,
        seat: u8,
        hard_state: ModelHardStateObservation,
        forbidden_drift_count: u32,
        slot: u64,
    },
    FinalizeCheckpoint {
        proposal_id: u64,
        phase: StateCheckpointPhaseV1,
        attesting_seats: [u8; MODEL_ROUTINE_THRESHOLD as usize],
        slot: u64,
    },
    ExtendTarget {
        proposal_id: u64,
        slot: u64,
    },
    ExecuteUpgrade {
        proposal_id: u64,
        slot: u64,
    },
    VerifyProgramData {
        proposal_id: u64,
        slot: u64,
    },
    RecordProgramDataFailure {
        proposal_id: u64,
        kind: ModelProgramDataFailureKind,
        slot: u64,
    },
    ActivateRollback {
        primary_proposal_id: u64,
        rollback_proposal_id: u64,
        slot: u64,
    },
    ApproveUnfreeze {
        proposal_id: u64,
        seat: u8,
        slot: u64,
    },
    ExecuteUnfreeze {
        proposal_id: u64,
        slot: u64,
    },
    GuardianFreeze {
        guardian: Pubkey,
        slot: u64,
        reason: u16,
    },
    CreateEmergencyResolution {
        slot: u64,
    },
    ApproveEmergencyResolution {
        resolution_id: u64,
        seat: u8,
        slot: u64,
    },
    QueueEmergencyResolution {
        resolution_id: u64,
        slot: u64,
    },
    AttestEmergencyCheckpoint {
        resolution_id: u64,
        seat: u8,
        hard_state: ModelHardStateObservation,
        forbidden_drift_count: u32,
        slot: u64,
    },
    FinalizeEmergencyCheckpoint {
        resolution_id: u64,
        attesting_seats: [u8; MODEL_ROUTINE_THRESHOLD as usize],
        slot: u64,
    },
    ExecuteEmergencyResume {
        resolution_id: u64,
        slot: u64,
    },
    ExpireEmergencyResolution {
        resolution_id: u64,
        slot: u64,
    },
    CreateCandidateCouncilSet {
        candidate_version: u64,
        candidate_seats: [Pubkey; MODEL_COUNCIL_SIZE],
        candidate_seat_terms: [ModelSeatTerm; MODEL_COUNCIL_SIZE],
        activation_slot: u64,
        slot: u64,
    },
    CreateCouncilRotation {
        candidate_version: u64,
        slot: u64,
    },
    ApproveCouncilRotation {
        rotation_id: u64,
        seat: u8,
        slot: u64,
    },
    QueueCouncilRotation {
        rotation_id: u64,
        slot: u64,
    },
    ApproveCouncilRotationCancellation {
        rotation_id: u64,
        seat: u8,
        slot: u64,
        reason: u16,
    },
    ExpireCouncilRotation {
        rotation_id: u64,
        slot: u64,
    },
    ActivateCouncilRotation {
        rotation_id: u64,
        slot: u64,
    },
    EnableTokenGovernance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Release1ModelOutcome {
    Applied,
    ProposalCreated(u64),
    EmergencyResolutionCreated(u64),
    CandidateCouncilCreated(u64),
    CouncilRotationCreated(u64),
}

#[derive(Clone, Debug, Default, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct Release1Model {
    pub initialized: bool,
    pub graph: ModelIdentityGraph,
    pub delays: ModelDelays,
    pub council: ModelCouncil,
    pub council_history: BTreeMap<u64, ModelCouncil>,
    pub programdata: ModelProgramDataObservation,
    pub current_programdata_authority: Option<Pubkey>,
    pub current_guardian_runtime_observation: ModelGuardianProgramDataObservation,
    pub token_governance_enabled: bool,
    pub target_immutability_enabled: bool,
    pub next_proposal_id: u64,
    pub target_nonce: u64,
    pub next_candidate_council_version: u64,
    pub gate: ModelGate,
    pub emergency_freeze_observations: BTreeMap<u64, ModelEmergencyFreezeObservation>,
    pub proposals: BTreeMap<u64, ModelProposal>,
    pub rotations: BTreeMap<u64, ModelCouncilRotation>,
    pub emergency_resolutions: BTreeMap<u64, ModelEmergencyResolution>,
}

impl Release1Model {
    pub fn apply(
        &mut self,
        action: Release1ModelAction,
    ) -> Release1ModelResult<Release1ModelOutcome> {
        let mut candidate = self.clone();
        let outcome = candidate.apply_in_place(action)?;
        *self = candidate;
        Ok(outcome)
    }

    fn apply_in_place(
        &mut self,
        action: Release1ModelAction,
    ) -> Release1ModelResult<Release1ModelOutcome> {
        let action = match action {
            Release1ModelAction::Initialize(initialization) => {
                self.initialize(*initialization)?;
                return Ok(Release1ModelOutcome::Applied);
            }
            action => action,
        };
        self.validate_core()?;
        if self.gate.status == GateStatusV1::Active
            && !self.current_guardian_runtime_observation.is_canonical_for(
                &self.graph,
                &self.programdata,
                Some(self.graph.authority_pda),
            )
            && !matches!(
                &action,
                Release1ModelAction::GuardianFreeze { .. }
                    | Release1ModelAction::SetProgramDataObservation(_)
                    | Release1ModelAction::SetProgramDataAuthorityObservation(_)
                    | Release1ModelAction::SetGuardianRuntimeObservation(_)
            )
        {
            return Err(Release1ModelError::ProgramDataChanged);
        }
        match action {
            Release1ModelAction::Initialize(_) => unreachable!(),
            Release1ModelAction::SimulatePostHandoffActivation {
                slot,
                bridge_and_authority_graph_verified,
            } => {
                self.simulate_post_handoff_activation(slot, bridge_and_authority_graph_verified)?
            }
            Release1ModelAction::SetProgramDataObservation(observation) => {
                observation.validate()?;
                self.current_programdata_authority = Some(observation.authority);
                self.current_guardian_runtime_observation =
                    ModelGuardianProgramDataObservation::canonical(
                        &self.graph,
                        &observation,
                        self.current_programdata_authority,
                    )?;
                self.programdata = observation;
            }
            Release1ModelAction::SetProgramDataAuthorityObservation(authority) => {
                if authority == Some(Pubkey::default()) {
                    return Err(Release1ModelError::InvalidInitialization);
                }
                self.current_programdata_authority = authority;
                self.current_guardian_runtime_observation =
                    ModelGuardianProgramDataObservation::canonical(
                        &self.graph,
                        &self.programdata,
                        authority,
                    )?;
            }
            Release1ModelAction::SetGuardianRuntimeObservation(observation) => {
                observation.validate_persistable()?;
                self.current_programdata_authority = observation.authority;
                self.current_guardian_runtime_observation = observation;
            }
            Release1ModelAction::CreateProposal(request) => {
                let id = self.create_proposal(request)?;
                return Ok(Release1ModelOutcome::ProposalCreated(id));
            }
            Release1ModelAction::AdoptBuffer { proposal_id } => {
                self.transition_current_candidate(
                    proposal_id,
                    ProposalStateV2::Draft,
                    ProposalStateV2::BufferAdopted,
                )?;
            }
            Release1ModelAction::VerifyBuffer { proposal_id } => {
                self.transition_current_candidate(
                    proposal_id,
                    ProposalStateV2::BufferAdopted,
                    ProposalStateV2::BufferVerified,
                )?;
            }
            Release1ModelAction::ApproveProposal {
                proposal_id,
                seat,
                slot,
            } => self.approve_proposal(proposal_id, seat, slot)?,
            Release1ModelAction::SatisfyGovernance { proposal_id, slot } => {
                self.satisfy_governance(proposal_id, slot)?;
            }
            Release1ModelAction::QueueProposal { proposal_id, slot } => {
                self.queue_proposal(proposal_id, slot)?;
            }
            Release1ModelAction::ApproveCancellation {
                proposal_id,
                seat,
                slot,
                reason,
            } => self.approve_cancellation(proposal_id, seat, slot, reason)?,
            Release1ModelAction::ExpireProposal { proposal_id, slot } => {
                self.expire_proposal(proposal_id, slot)?;
            }
            Release1ModelAction::FreezeProposal { proposal_id, slot } => {
                self.freeze_proposal(proposal_id, slot)?;
            }
            Release1ModelAction::ConvertEmergencyFreeze { proposal_id, slot } => {
                self.convert_emergency_freeze(proposal_id, slot)?;
            }
            Release1ModelAction::AttestCheckpoint {
                proposal_id,
                phase,
                seat,
                hard_state,
                forbidden_drift_count,
                slot,
            } => {
                self.attest_checkpoint(
                    proposal_id,
                    phase,
                    seat,
                    hard_state,
                    forbidden_drift_count,
                    slot,
                )?;
            }
            Release1ModelAction::FinalizeCheckpoint {
                proposal_id,
                phase,
                attesting_seats,
                slot,
            } => self.finalize_checkpoint(proposal_id, phase, attesting_seats, slot)?,
            Release1ModelAction::ExtendTarget { proposal_id, slot } => {
                self.extend_target(proposal_id, slot)?;
            }
            Release1ModelAction::ExecuteUpgrade { proposal_id, slot } => {
                self.execute_upgrade(proposal_id, slot)?;
            }
            Release1ModelAction::VerifyProgramData { proposal_id, slot } => {
                self.verify_programdata(proposal_id, slot)?;
            }
            Release1ModelAction::RecordProgramDataFailure {
                proposal_id,
                kind,
                slot,
            } => self.record_programdata_failure(proposal_id, kind, slot)?,
            Release1ModelAction::ActivateRollback {
                primary_proposal_id,
                rollback_proposal_id,
                slot,
            } => self.activate_rollback(primary_proposal_id, rollback_proposal_id, slot)?,
            Release1ModelAction::ApproveUnfreeze {
                proposal_id,
                seat,
                slot,
            } => self.approve_unfreeze(proposal_id, seat, slot)?,
            Release1ModelAction::ExecuteUnfreeze { proposal_id, slot } => {
                self.execute_unfreeze(proposal_id, slot)?;
            }
            Release1ModelAction::GuardianFreeze {
                guardian,
                slot,
                reason,
            } => self.guardian_freeze(guardian, slot, reason)?,
            Release1ModelAction::CreateEmergencyResolution { slot } => {
                let id = self.create_emergency_resolution(slot)?;
                return Ok(Release1ModelOutcome::EmergencyResolutionCreated(id));
            }
            Release1ModelAction::ApproveEmergencyResolution {
                resolution_id,
                seat,
                slot,
            } => self.approve_emergency_resolution(resolution_id, seat, slot)?,
            Release1ModelAction::QueueEmergencyResolution {
                resolution_id,
                slot,
            } => self.queue_emergency_resolution(resolution_id, slot)?,
            Release1ModelAction::AttestEmergencyCheckpoint {
                resolution_id,
                seat,
                hard_state,
                forbidden_drift_count,
                slot,
            } => self.attest_emergency_checkpoint(
                resolution_id,
                seat,
                hard_state,
                forbidden_drift_count,
                slot,
            )?,
            Release1ModelAction::FinalizeEmergencyCheckpoint {
                resolution_id,
                attesting_seats,
                slot,
            } => self.finalize_emergency_checkpoint(resolution_id, attesting_seats, slot)?,
            Release1ModelAction::ExecuteEmergencyResume {
                resolution_id,
                slot,
            } => self.execute_emergency_resume(resolution_id, slot)?,
            Release1ModelAction::ExpireEmergencyResolution {
                resolution_id,
                slot,
            } => self.expire_emergency_resolution(resolution_id, slot)?,
            Release1ModelAction::CreateCandidateCouncilSet {
                candidate_version,
                candidate_seats,
                candidate_seat_terms,
                activation_slot,
                slot,
            } => {
                let id = self.create_candidate_council_set(
                    candidate_version,
                    candidate_seats,
                    candidate_seat_terms,
                    activation_slot,
                    slot,
                )?;
                return Ok(Release1ModelOutcome::CandidateCouncilCreated(id));
            }
            Release1ModelAction::CreateCouncilRotation {
                candidate_version,
                slot,
            } => {
                let id = self.create_council_rotation(candidate_version, slot)?;
                return Ok(Release1ModelOutcome::CouncilRotationCreated(id));
            }
            Release1ModelAction::ApproveCouncilRotation {
                rotation_id,
                seat,
                slot,
            } => {
                self.approve_council_rotation(rotation_id, seat, slot)?;
            }
            Release1ModelAction::QueueCouncilRotation { rotation_id, slot } => {
                self.queue_council_rotation(rotation_id, slot)?;
            }
            Release1ModelAction::ApproveCouncilRotationCancellation {
                rotation_id,
                seat,
                slot,
                reason,
            } => self.approve_council_rotation_cancellation(rotation_id, seat, slot, reason)?,
            Release1ModelAction::ExpireCouncilRotation { rotation_id, slot } => {
                self.expire_council_rotation(rotation_id, slot)?;
            }
            Release1ModelAction::ActivateCouncilRotation { rotation_id, slot } => {
                self.activate_council_rotation(rotation_id, slot)?;
            }
            Release1ModelAction::EnableTokenGovernance => {
                return Err(Release1ModelError::TokenGovernanceDisabled);
            }
        }
        Ok(Release1ModelOutcome::Applied)
    }

    fn initialize(&mut self, input: ModelInitialization) -> Release1ModelResult<()> {
        if self.initialized {
            return Err(Release1ModelError::AlreadyInitialized);
        }
        if input.slot == 0
            || !input.controller_programdata_linked
            || !input.initializer_is_controller_upgrade_authority
            || !input.target_programdata_linked
            || !input.canonical_pdas_verified
            || !input.seat_accounts_readonly
            || !input.seat_accounts_nonexecutable
        {
            return Err(Release1ModelError::InvalidInitialization);
        }
        input.graph.validate()?;
        input.delays.validate()?;
        input.programdata.validate()?;
        if input.programdata.authority == input.graph.authority_pda {
            return Err(Release1ModelError::InvalidInitialization);
        }
        let current_guardian_runtime_observation = ModelGuardianProgramDataObservation::canonical(
            &input.graph,
            &input.programdata,
            Some(input.programdata.authority),
        )?;
        let council = ModelCouncil::new(1, input.slot, input.seats, input.seat_terms)?;
        let mut council_history = BTreeMap::new();
        council_history.insert(council.version, council.clone());
        *self = Self {
            initialized: true,
            graph: input.graph,
            delays: input.delays,
            council,
            council_history,
            current_programdata_authority: Some(input.programdata.authority),
            programdata: input.programdata,
            current_guardian_runtime_observation,
            token_governance_enabled: false,
            target_immutability_enabled: false,
            next_proposal_id: 1,
            target_nonce: 1,
            next_candidate_council_version: 2,
            gate: ModelGate {
                status: GateStatusV1::EmergencyFrozen,
                epoch: 1,
                active_proposal: None,
                freeze_slot: input.slot,
                freeze_reason: BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1,
                last_completed_proposal: None,
            },
            emergency_freeze_observations: BTreeMap::new(),
            proposals: BTreeMap::new(),
            rotations: BTreeMap::new(),
            emergency_resolutions: BTreeMap::new(),
        };
        self.validate_core()
    }

    fn validate_core(&self) -> Release1ModelResult<()> {
        if !self.initialized {
            return Err(Release1ModelError::NotInitialized);
        }
        self.graph.validate()?;
        self.delays.validate()?;
        self.council.validate()?;
        for (version, council) in &self.council_history {
            if *version != council.version {
                return Err(Release1ModelError::InvalidCouncil);
            }
            council.validate()?;
        }
        if self.council_history.get(&self.council.version) != Some(&self.council) {
            return Err(Release1ModelError::InvalidCouncil);
        }
        self.programdata.validate()?;
        if self.current_programdata_authority == Some(Pubkey::default()) {
            return Err(Release1ModelError::InvalidInitialization);
        }
        self.current_guardian_runtime_observation
            .validate_persistable()?;
        self.gate.validate()?;
        if self.gate.status == GateStatusV1::EmergencyFrozen
            && self.gate.freeze_reason != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        {
            let observation = self
                .emergency_freeze_observations
                .get(&self.gate.epoch)
                .ok_or(Release1ModelError::InvalidGate)?;
            if observation.epoch != self.gate.epoch
                || observation.freeze_slot != self.gate.freeze_slot
                || observation.freeze_reason != self.gate.freeze_reason
            {
                return Err(Release1ModelError::InvalidGate);
            }
            observation.programdata.validate_persistable()?;
        }
        if self.token_governance_enabled
            || self.target_immutability_enabled
            || self.next_proposal_id == 0
            || self.target_nonce == 0
            || self.next_candidate_council_version <= self.council.version
        {
            return Err(Release1ModelError::InvalidInitialization);
        }
        Ok(())
    }

    fn simulate_post_handoff_activation(
        &mut self,
        slot: u64,
        verified: bool,
    ) -> Release1ModelResult<()> {
        if !verified
            || slot == 0
            || self.gate.status != GateStatusV1::EmergencyFrozen
            || self.gate.freeze_reason != BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.gate.active_proposal.is_some()
            || self.programdata.authority != self.graph.authority_pda
            || self.current_programdata_authority != Some(self.graph.authority_pda)
            || !self.current_guardian_runtime_observation.is_canonical_for(
                &self.graph,
                &self.programdata,
                Some(self.graph.authority_pda),
            )
        {
            return Err(Release1ModelError::InvalidGate);
        }
        self.gate.epoch = checked_increment(self.gate.epoch)?;
        self.clear_gate_to_active();
        Ok(())
    }

    fn create_proposal(&mut self, request: ModelProposalRequest) -> Release1ModelResult<u64> {
        self.delays.class_delay(request.class)?;
        if request.slot == 0
            || self.gate.status == GateStatusV1::FrozenForUpgrade
            || (self.gate.status == GateStatusV1::EmergencyFrozen
                && self.gate.freeze_reason == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1)
        {
            return Err(Release1ModelError::InvalidGate);
        }
        self.require_controller_programdata_authority()?;
        if !request.proposer_signed || !request.payer_is_separate {
            return Err(Release1ModelError::InvalidProposer);
        }
        self.council
            .require_seat_active(request.proposer_seat, request.slot)
            .map_err(|_| Release1ModelError::InvalidProposer)?;
        let link_shape = match request.class {
            ProposalClassV1::EmergencyRollback => {
                request.primary_proposal.is_some() && request.rollback_proposal.is_none()
            }
            ProposalClassV1::RoutineUpgrade
            | ProposalClassV1::EconomicChange
            | ProposalClassV1::ConstitutionalChange => {
                request.primary_proposal.is_none() && request.rollback_proposal.is_some()
            }
            ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability => false,
        };
        if !link_shape {
            return Err(
                if matches!(
                    request.class,
                    ProposalClassV1::CouncilSetRotation | ProposalClassV1::TargetImmutability
                ) {
                    Release1ModelError::UnsupportedProposalClass
                } else {
                    Release1ModelError::InvalidRollbackLink
                },
            );
        }
        let creation_emergency_observation = if self.gate.status == GateStatusV1::EmergencyFrozen {
            let observation = self
                .emergency_freeze_observations
                .get(&self.gate.epoch)
                .ok_or(Release1ModelError::InvalidGate)?
                .clone();
            if observation.epoch != self.gate.epoch
                || observation.freeze_slot != self.gate.freeze_slot
                || observation.freeze_reason != self.gate.freeze_reason
            {
                return Err(Release1ModelError::InvalidGate);
            }
            Some(observation)
        } else {
            None
        };
        let id = self.next_proposal_id;
        self.next_proposal_id = checked_increment(id)?;
        let timing = self.proposal_timing(request.slot, request.class)?;
        let proposal = ModelProposal {
            id,
            class: request.class,
            state: ProposalStateV2::Draft,
            target_nonce: self.target_nonce,
            creation_gate_status: self.gate.status,
            creation_gate_epoch: self.gate.epoch,
            creation_emergency_observation,
            creation_programdata: self.programdata.clone(),
            freeze_gate_epoch: 0,
            creation_council_version: self.council.version,
            creation_council_hash: self.council.hash,
            timing,
            extension_required: request.extension_required,
            primary_proposal: request.primary_proposal,
            rollback_proposal: request.rollback_proposal,
            initial_approvals: ModelApprovalAccumulator::pinned(&self.council),
            cancellation_approvals: ModelApprovalAccumulator::default(),
            unfreeze_approvals: ModelApprovalAccumulator::default(),
            prestate: None,
            poststate: None,
            prestate_attestations: Vec::new(),
            poststate_attestations: Vec::new(),
            frozen_slot: 0,
            extended_slot: 0,
            upgraded_slot: 0,
            programdata_verified_slot: 0,
            programdata_failure: None,
            terminal_slot: 0,
            cancellation_reason: 0,
            terminal_reason: 0,
        };
        self.proposals.insert(id, proposal);
        Ok(id)
    }

    fn proposal_timing(
        &self,
        creation_slot: u64,
        class: ProposalClassV1,
    ) -> Release1ModelResult<ModelTiming> {
        let review_start_slot = checked_increment(creation_slot)?;
        let review_end_slot = review_start_slot
            .checked_add(self.delays.review_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let not_before_slot = review_end_slot
            .checked_add(self.delays.class_delay(class)?)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let expiry_slot = creation_slot
            .checked_add(self.delays.proposal_expiry_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if not_before_slot >= expiry_slot {
            return Err(Release1ModelError::InvalidTiming);
        }
        Ok(ModelTiming {
            creation_slot,
            review_start_slot,
            review_end_slot,
            not_before_slot,
            expiry_slot,
        })
    }

    fn validate_proposal_timing(&self, proposal: &ModelProposal) -> Release1ModelResult<()> {
        if self.proposal_timing(proposal.timing.creation_slot, proposal.class)? != proposal.timing {
            return Err(Release1ModelError::InvalidTiming);
        }
        Ok(())
    }

    fn transition_simple(
        &mut self,
        proposal_id: u64,
        current: ProposalStateV2,
        next: ProposalStateV2,
    ) -> Release1ModelResult<()> {
        let proposal = self
            .proposals
            .get_mut(&proposal_id)
            .ok_or(Release1ModelError::NotFound)?;
        if proposal.state != current
            || !proposal_edge_allowed(current, next, proposal.extension_required)
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        proposal.state = next;
        Ok(())
    }

    fn transition_current_candidate(
        &mut self,
        proposal_id: u64,
        current: ProposalStateV2,
        next: ProposalStateV2,
    ) -> Release1ModelResult<()> {
        let proposal = self.proposal(proposal_id)?.clone();
        self.require_current_nonce(&proposal)?;
        self.require_creation_gate(&proposal)?;
        self.transition_simple(proposal_id, current, next)
    }

    fn approve_proposal(
        &mut self,
        proposal_id: u64,
        seat: u8,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.validate_proposal_timing(&snapshot)?;
        if snapshot.state != ProposalStateV2::BufferVerified
            || slot < snapshot.timing.review_start_slot
            || slot > snapshot.timing.review_end_slot
            || slot >= snapshot.timing.expiry_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }
        self.require_current_nonce(&snapshot)?;
        self.require_creation_gate(&snapshot)?;
        if snapshot.creation_council_version != self.council.version
            || snapshot.creation_council_hash != self.council.hash
        {
            return Err(Release1ModelError::StaleBinding);
        }
        let council = self.council.clone();
        let proposal = self.proposal_mut(proposal_id)?;
        let crossed = proposal
            .initial_approvals
            .record_pinned(&council, seat, slot)?;
        if crossed {
            proposal.state = ProposalStateV2::CouncilApproved;
        }
        Ok(())
    }

    fn satisfy_governance(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let proposal = self.proposal(proposal_id)?.clone();
        self.validate_proposal_timing(&proposal)?;
        if proposal.state != ProposalStateV2::CouncilApproved || slot >= proposal.timing.expiry_slot
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        if !self.pinned_initial_quorum_complete(&proposal)? {
            return Err(Release1ModelError::QuorumNotSatisfied);
        }
        self.require_current_nonce(&proposal)?;
        self.require_creation_gate(&proposal)?;
        self.transition_simple(
            proposal_id,
            ProposalStateV2::CouncilApproved,
            ProposalStateV2::GovernanceSatisfied,
        )
    }

    fn queue_proposal(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let proposal = self.proposal(proposal_id)?.clone();
        self.validate_proposal_timing(&proposal)?;
        if proposal.state != ProposalStateV2::GovernanceSatisfied
            || slot >= proposal.timing.expiry_slot
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        if !self.pinned_initial_quorum_complete(&proposal)? {
            return Err(Release1ModelError::QuorumNotSatisfied);
        }
        self.require_current_nonce(&proposal)?;
        self.require_creation_gate(&proposal)?;
        self.transition_simple(
            proposal_id,
            ProposalStateV2::GovernanceSatisfied,
            ProposalStateV2::Timelocked,
        )
    }

    fn approve_cancellation(
        &mut self,
        proposal_id: u64,
        seat: u8,
        slot: u64,
        reason: u16,
    ) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        if !snapshot.is_pre_freeze()
            || slot >= snapshot.timing.expiry_slot
            || reason == 0
            || (snapshot.cancellation_reason != 0 && snapshot.cancellation_reason != reason)
            || snapshot.terminal_reason != 0
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let council = self.council.clone();
        let proposal = self.proposal_mut(proposal_id)?;
        if proposal.cancellation_reason == 0 {
            proposal.cancellation_reason = reason;
        }
        if proposal
            .cancellation_approvals
            .record_current(&council, seat, slot)?
        {
            proposal.state = ProposalStateV2::Cancelled;
            proposal.terminal_slot = slot;
            proposal.terminal_reason = reason;
        }
        Ok(())
    }

    fn expire_proposal(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let proposal = self.proposal_mut(proposal_id)?;
        if !proposal.is_pre_freeze() || slot < proposal.timing.expiry_slot {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        proposal.state = ProposalStateV2::Expired;
        proposal.terminal_slot = slot;
        proposal.terminal_reason = MODEL_EXPIRED_TERMINAL_REASON;
        Ok(())
    }

    fn freeze_proposal(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let proposal = self.proposal(proposal_id)?.clone();
        self.validate_proposal_timing(&proposal)?;
        if proposal.class == ProposalClassV1::EmergencyRollback {
            return Err(Release1ModelError::InvalidRollbackLink);
        }
        if proposal.state != ProposalStateV2::Timelocked
            || slot < proposal.timing.not_before_slot
            || slot >= proposal.timing.expiry_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }
        self.require_freeze_execution_runway(&proposal, slot)?;
        if !self.pinned_initial_quorum_complete(&proposal)? {
            return Err(Release1ModelError::QuorumNotSatisfied);
        }
        self.require_current_nonce(&proposal)?;
        self.require_creation_gate(&proposal)?;
        if proposal.creation_gate_status != GateStatusV1::Active
            || proposal.creation_emergency_observation.is_some()
            || self.gate.status != GateStatusV1::Active
            || proposal.creation_gate_epoch != self.gate.epoch
        {
            return Err(Release1ModelError::InvalidGate);
        }
        self.validate_prepared_rollback(&proposal, slot)?;
        self.consume_proposal_freeze(proposal_id, slot)
    }

    fn convert_emergency_freeze(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let proposal = self.proposal(proposal_id)?.clone();
        self.require_controller_programdata_authority()?;
        self.validate_proposal_timing(&proposal)?;
        if proposal.class == ProposalClassV1::EmergencyRollback {
            return Err(Release1ModelError::InvalidRollbackLink);
        }
        if proposal.state != ProposalStateV2::Timelocked
            || slot < proposal.timing.not_before_slot
            || slot >= proposal.timing.expiry_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }
        self.require_freeze_execution_runway(&proposal, slot)?;
        if !self.pinned_initial_quorum_complete(&proposal)? {
            return Err(Release1ModelError::QuorumNotSatisfied);
        }
        self.require_current_nonce(&proposal)?;
        self.require_creation_gate(&proposal)?;
        if proposal.creation_gate_status != GateStatusV1::EmergencyFrozen
            || self.gate.status != GateStatusV1::EmergencyFrozen
            || proposal.creation_gate_epoch != self.gate.epoch
            || self.gate.active_proposal.is_some()
            || self.gate.freeze_reason == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        {
            return Err(Release1ModelError::InvalidGate);
        }
        let bound_observation = proposal
            .creation_emergency_observation
            .as_ref()
            .ok_or(Release1ModelError::InvalidGate)?;
        let canonical_observation = self
            .emergency_freeze_observations
            .get(&self.gate.epoch)
            .ok_or(Release1ModelError::InvalidGate)?;
        if bound_observation != canonical_observation
            || bound_observation.epoch != self.gate.epoch
            || bound_observation.freeze_slot != self.gate.freeze_slot
            || bound_observation.freeze_reason != self.gate.freeze_reason
        {
            return Err(Release1ModelError::StaleBinding);
        }
        // Conversion deliberately preserves two different commitments: the
        // immutable guardian-freeze observation above, and the proposal's
        // separately captured creation ProgramData.  Requiring the live bytes
        // to remain equal to the guardian observation would make the governed
        // repair path unusable for the very drift that caused the emergency.
        // `require_creation_gate` already proves that the current ProgramData
        // still equals the proposal's creation commitment.
        self.validate_prepared_rollback(&proposal, slot)?;
        self.consume_proposal_freeze(proposal_id, slot)
    }

    fn consume_proposal_freeze(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        self.target_nonce = checked_increment(self.target_nonce)?;
        self.gate.epoch = checked_increment(self.gate.epoch)?;
        self.gate.status = GateStatusV1::FrozenForUpgrade;
        self.gate.active_proposal = Some(proposal_id);
        self.gate.freeze_slot = slot;
        self.gate.freeze_reason = MODEL_GOVERNED_FREEZE_REASON;
        let epoch = self.gate.epoch;
        let proposal = self.proposal_mut(proposal_id)?;
        proposal.state = ProposalStateV2::Frozen;
        proposal.freeze_gate_epoch = epoch;
        proposal.frozen_slot = slot;
        Ok(())
    }

    fn attest_checkpoint(
        &mut self,
        proposal_id: u64,
        phase: StateCheckpointPhaseV1,
        seat: u8,
        hard_state: ModelHardStateObservation,
        forbidden_drift_count: u32,
        slot: u64,
    ) -> Release1ModelResult<()> {
        hard_state.validate()?;
        if phase == StateCheckpointPhaseV1::Emergency {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_frozen_proposal(&snapshot)?;
        let state_matches_phase = match phase {
            StateCheckpointPhaseV1::Prestate => snapshot.state == ProposalStateV2::Frozen,
            StateCheckpointPhaseV1::Poststate => {
                snapshot.state == ProposalStateV2::ProgramDataVerified
            }
            StateCheckpointPhaseV1::Emergency => false,
        };
        let canonical_exists = match phase {
            StateCheckpointPhaseV1::Prestate => snapshot.prestate.is_some(),
            StateCheckpointPhaseV1::Poststate => snapshot.poststate.is_some(),
            StateCheckpointPhaseV1::Emergency => true,
        };
        if !state_matches_phase || canonical_exists || slot == 0 {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let council = self.council.clone();
        council.require_seat_active(seat, slot)?;
        let seat_authority = *council
            .seats
            .get(usize::from(seat))
            .ok_or(Release1ModelError::InvalidApproval)?;
        let attestation = ModelCheckpointAttestation {
            subject_id: proposal_id,
            phase,
            gate_epoch: self.gate.epoch,
            council_version: council.version,
            council_hash: council.hash,
            seat,
            seat_authority,
            hard_state,
            forbidden_drift_count,
            attested_slot: slot,
        };
        let proposal = self.proposal_mut(proposal_id)?;
        let attestations = match phase {
            StateCheckpointPhaseV1::Prestate => &mut proposal.prestate_attestations,
            StateCheckpointPhaseV1::Poststate => &mut proposal.poststate_attestations,
            StateCheckpointPhaseV1::Emergency => unreachable!(),
        };
        if let Some(existing) = attestations
            .iter_mut()
            .find(|existing| existing.council_version == council.version && existing.seat == seat)
        {
            *existing = attestation;
        } else {
            attestations.push(attestation);
        }
        Ok(())
    }

    fn finalize_checkpoint(
        &mut self,
        proposal_id: u64,
        phase: StateCheckpointPhaseV1,
        attesting_seats: [u8; MODEL_ROUTINE_THRESHOLD as usize],
        slot: u64,
    ) -> Release1ModelResult<()> {
        if phase == StateCheckpointPhaseV1::Emergency || slot == 0 {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_frozen_proposal(&snapshot)?;
        let state_matches_phase = match phase {
            StateCheckpointPhaseV1::Prestate => snapshot.state == ProposalStateV2::Frozen,
            StateCheckpointPhaseV1::Poststate => {
                snapshot.state == ProposalStateV2::ProgramDataVerified
            }
            StateCheckpointPhaseV1::Emergency => false,
        };
        let canonical_exists = match phase {
            StateCheckpointPhaseV1::Prestate => snapshot.prestate.is_some(),
            StateCheckpointPhaseV1::Poststate => snapshot.poststate.is_some(),
            StateCheckpointPhaseV1::Emergency => true,
        };
        if !state_matches_phase || canonical_exists {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let attestations = match phase {
            StateCheckpointPhaseV1::Prestate => &snapshot.prestate_attestations,
            StateCheckpointPhaseV1::Poststate => &snapshot.poststate_attestations,
            StateCheckpointPhaseV1::Emergency => unreachable!(),
        };
        let (selected, approvals) =
            self.select_checkpoint_quorum(proposal_id, phase, attestations, attesting_seats, slot)?;
        let mut accepted = selected.forbidden_drift_count == 0;
        let mut rejected_hard_mismatch = selected.forbidden_drift_count != 0;
        if phase == StateCheckpointPhaseV1::Poststate {
            let prestate = snapshot
                .prestate
                .as_ref()
                .filter(|checkpoint| checkpoint.accepted)
                .ok_or(Release1ModelError::CheckpointNotAccepted)?;
            if selected.hard_state != prestate.hard_state && selected.forbidden_drift_count == 0 {
                return Err(Release1ModelError::HardPoststateMismatch);
            }
            accepted = selected.forbidden_drift_count == 0;
            rejected_hard_mismatch = !accepted;
        }
        let checkpoint = ModelCheckpoint {
            phase,
            gate_epoch: self.gate.epoch,
            hard_state: selected.hard_state,
            forbidden_drift_count: selected.forbidden_drift_count,
            approvals,
            finalized: true,
            accepted,
            rejected_hard_mismatch,
            accepted_slot: if accepted { slot } else { 0 },
        };
        let proposal = self.proposal_mut(proposal_id)?;
        match phase {
            StateCheckpointPhaseV1::Prestate => proposal.prestate = Some(checkpoint),
            StateCheckpointPhaseV1::Poststate => proposal.poststate = Some(checkpoint),
            StateCheckpointPhaseV1::Emergency => unreachable!(),
        }
        if phase == StateCheckpointPhaseV1::Poststate && accepted {
            proposal.state = ProposalStateV2::PoststateAccepted;
        }
        Ok(())
    }

    fn select_checkpoint_quorum(
        &self,
        subject_id: u64,
        phase: StateCheckpointPhaseV1,
        attestations: &[ModelCheckpointAttestation],
        attesting_seats: [u8; MODEL_ROUTINE_THRESHOLD as usize],
        slot: u64,
    ) -> Release1ModelResult<(ModelCheckpointAttestation, ModelApprovalAccumulator)> {
        if slot == 0
            || attesting_seats.into_iter().collect::<BTreeSet<_>>().len()
                != MODEL_ROUTINE_THRESHOLD as usize
        {
            return Err(Release1ModelError::InvalidApproval);
        }
        let mut selected: Option<ModelCheckpointAttestation> = None;
        let mut approvals = ModelApprovalAccumulator::pinned(&self.council);
        for seat in attesting_seats {
            self.council.require_seat_active(seat, slot)?;
            let attestation = attestations
                .iter()
                .find(|attestation| {
                    attestation.council_version == self.council.version && attestation.seat == seat
                })
                .ok_or(Release1ModelError::QuorumNotSatisfied)?;
            let expected_authority = *self
                .council
                .seats
                .get(usize::from(seat))
                .ok_or(Release1ModelError::InvalidApproval)?;
            if attestation.subject_id != subject_id
                || attestation.phase != phase
                || attestation.gate_epoch != self.gate.epoch
                || attestation.council_hash != self.council.hash
                || attestation.seat_authority != expected_authority
                || attestation.attested_slot == 0
                || attestation.attested_slot > slot
            {
                return Err(Release1ModelError::StaleBinding);
            }
            if let Some(first) = selected.as_ref() {
                if attestation.hard_state != first.hard_state
                    || attestation.forbidden_drift_count != first.forbidden_drift_count
                {
                    return Err(Release1ModelError::QuorumNotSatisfied);
                }
            } else {
                selected = Some(attestation.clone());
            }
            approvals.record_current(&self.council, seat, slot)?;
        }
        if !approvals.is_complete_at(&self.council, slot)? {
            return Err(Release1ModelError::QuorumNotSatisfied);
        }
        Ok((
            selected.ok_or(Release1ModelError::QuorumNotSatisfied)?,
            approvals,
        ))
    }

    fn extend_target(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_controller_programdata_authority()?;
        self.require_frozen_proposal(&snapshot)?;
        if snapshot.state != ProposalStateV2::Frozen
            || !snapshot.extension_required
            || !checkpoint_accepted(snapshot.prestate.as_ref())
            || slot == 0
            || slot < snapshot.frozen_slot
            || slot >= snapshot.timing.expiry_slot
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let proposal = self.proposal_mut(proposal_id)?;
        proposal.state = ProposalStateV2::Extended;
        proposal.extended_slot = slot;
        Ok(())
    }

    fn execute_upgrade(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_controller_programdata_authority()?;
        self.require_frozen_proposal(&snapshot)?;
        let correct_state = if snapshot.extension_required {
            snapshot.state == ProposalStateV2::Extended
                && snapshot.extended_slot != 0
                && slot > snapshot.extended_slot
        } else {
            snapshot.state == ProposalStateV2::Frozen
        };
        if !correct_state
            || !checkpoint_accepted(snapshot.prestate.as_ref())
            || slot == 0
            || slot >= snapshot.timing.expiry_slot
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        if snapshot.class != ProposalClassV1::EmergencyRollback {
            self.validate_prepared_rollback(&snapshot, slot)?;
            self.require_primary_execution_rollback_runway(&snapshot, slot)?;
        }
        let proposal = self.proposal_mut(proposal_id)?;
        proposal.state = ProposalStateV2::UpgradeExecuted;
        proposal.upgraded_slot = slot;
        Ok(())
    }

    fn verify_programdata(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_controller_programdata_authority()?;
        self.require_frozen_proposal(&snapshot)?;
        if snapshot.state != ProposalStateV2::UpgradeExecuted
            || snapshot.upgraded_slot == 0
            || slot < snapshot.upgraded_slot
            || snapshot.programdata_failure.is_some()
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let proposal = self.proposal_mut(proposal_id)?;
        proposal.state = ProposalStateV2::ProgramDataVerified;
        proposal.programdata_verified_slot = slot;
        Ok(())
    }

    fn record_programdata_failure(
        &mut self,
        proposal_id: u64,
        kind: ModelProgramDataFailureKind,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_frozen_proposal(&snapshot)?;
        if snapshot.class == ProposalClassV1::EmergencyRollback
            || snapshot.state != ProposalStateV2::UpgradeExecuted
            || snapshot.upgraded_slot == 0
            || slot < snapshot.upgraded_slot
            || snapshot.programdata_failure.is_some()
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        self.programdata.validate()?;
        let evidence = ModelProgramDataFailureEvidence {
            proposal_id,
            gate_epoch: self.gate.epoch,
            observed_slot: slot,
            kind,
            observed_programdata: self.programdata.clone(),
        };
        self.proposal_mut(proposal_id)?.programdata_failure = Some(evidence);
        Ok(())
    }

    fn activate_rollback(
        &mut self,
        primary_id: u64,
        rollback_id: u64,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let primary = self.proposal(primary_id)?.clone();
        let rollback = self.proposal(rollback_id)?.clone();
        self.require_frozen_proposal(&primary)?;
        if rollback.state != ProposalStateV2::Timelocked
            || rollback.class != ProposalClassV1::EmergencyRollback
            || primary.rollback_proposal != Some(rollback_id)
            || rollback.primary_proposal != Some(primary_id)
            || rollback.target_nonce != primary.target_nonce
            || self.target_nonce != checked_increment(primary.target_nonce)?
            || slot >= rollback.timing.expiry_slot
        {
            return Err(Release1ModelError::InvalidRollbackLink);
        }
        self.require_freeze_execution_runway(&rollback, slot)?;
        let rollback_ready_slot = primary
            .upgraded_slot
            .checked_add(self.delays.rollback_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if primary.upgraded_slot == 0
            || slot < rollback.timing.not_before_slot
            || slot < rollback_ready_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }

        let programdata_failure = primary.programdata_failure.as_ref().filter(|failure| {
            primary.state == ProposalStateV2::UpgradeExecuted
                && failure.proposal_id == primary_id
                && failure.gate_epoch == self.gate.epoch
                && failure.observed_slot >= primary.upgraded_slot
                && failure.observed_slot <= slot
        });
        if let Some(failure) = programdata_failure {
            if failure.observed_programdata != self.programdata {
                return Err(Release1ModelError::ProgramDataChanged);
            }
        }
        let hard_poststate_failure = if primary.state == ProposalStateV2::ProgramDataVerified {
            match (primary.prestate.as_ref(), primary.poststate.as_ref()) {
                (Some(prestate), Some(poststate)) => {
                    prestate.accepted
                        && prestate.finalized
                        && !poststate.accepted
                        && poststate.finalized
                        && poststate.rejected_hard_mismatch
                        && prestate.phase == StateCheckpointPhaseV1::Prestate
                        && poststate.phase == StateCheckpointPhaseV1::Poststate
                        && prestate.gate_epoch == self.gate.epoch
                        && poststate.gate_epoch == self.gate.epoch
                        && (prestate.hard_state != poststate.hard_state
                            || poststate.forbidden_drift_count != 0)
                        && poststate.approvals.council_version == self.council.version
                        && poststate.approvals.council_hash == self.council.hash
                        && poststate.approvals.is_complete_at(&self.council, slot)?
                }
                _ => false,
            }
        } else {
            false
        };
        if programdata_failure.is_none() && !hard_poststate_failure {
            return Err(Release1ModelError::ProgramDataFailureEvidenceRequired);
        }
        self.gate.epoch = checked_increment(self.gate.epoch)?;
        self.gate.active_proposal = Some(rollback_id);
        self.gate.freeze_slot = slot;
        self.gate.freeze_reason = MODEL_GOVERNED_FREEZE_REASON;
        let epoch = self.gate.epoch;
        let rollback = self.proposal_mut(rollback_id)?;
        rollback.state = ProposalStateV2::Frozen;
        rollback.freeze_gate_epoch = epoch;
        rollback.frozen_slot = slot;
        Ok(())
    }

    fn require_freeze_execution_runway(
        &self,
        proposal: &ModelProposal,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let execution_slots = if proposal.extension_required { 2 } else { 1 };
        let horizon = slot
            .checked_add(self.delays.review_slots)
            .and_then(|value| value.checked_add(execution_slots))
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if horizon >= proposal.timing.expiry_slot {
            return Err(Release1ModelError::TimingViolation);
        }
        Ok(())
    }

    fn approve_unfreeze(
        &mut self,
        proposal_id: u64,
        seat: u8,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_frozen_proposal(&snapshot)?;
        if snapshot.state != ProposalStateV2::PoststateAccepted
            && !(snapshot.state == ProposalStateV2::UnfreezeApproved
                && (snapshot.unfreeze_approvals.council_version != self.council.version
                    || snapshot.unfreeze_approvals.council_hash != self.council.hash))
            || !checkpoint_accepted(snapshot.poststate.as_ref())
        {
            return Err(Release1ModelError::CheckpointNotAccepted);
        }
        let council = self.council.clone();
        let proposal = self.proposal_mut(proposal_id)?;
        if proposal.state == ProposalStateV2::UnfreezeApproved {
            proposal.state = ProposalStateV2::PoststateAccepted;
        }
        if proposal
            .unfreeze_approvals
            .record_current(&council, seat, slot)?
        {
            proposal.state = ProposalStateV2::UnfreezeApproved;
        }
        Ok(())
    }

    fn execute_unfreeze(&mut self, proposal_id: u64, slot: u64) -> Release1ModelResult<()> {
        let snapshot = self.proposal(proposal_id)?.clone();
        self.require_controller_programdata_authority()?;
        self.require_frozen_proposal(&snapshot)?;
        if snapshot.state != ProposalStateV2::UnfreezeApproved
            || !checkpoint_accepted(snapshot.poststate.as_ref())
            || snapshot.unfreeze_approvals.council_version != self.council.version
            || snapshot.unfreeze_approvals.council_hash != self.council.hash
            || !snapshot
                .unfreeze_approvals
                .is_complete_at(&self.council, slot)?
            || slot == 0
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        self.gate.epoch = checked_increment(self.gate.epoch)?;
        self.gate.last_completed_proposal = Some(proposal_id);
        self.clear_gate_to_active();
        {
            let proposal = self.proposal_mut(proposal_id)?;
            proposal.state = ProposalStateV2::Completed;
            proposal.terminal_slot = slot;
            proposal.terminal_reason = MODEL_COMPLETED_TERMINAL_REASON;
        }
        if snapshot.class == ProposalClassV1::EmergencyRollback {
            let primary_id = snapshot
                .primary_proposal
                .ok_or(Release1ModelError::InvalidRollbackLink)?;
            let primary = self.proposal_mut(primary_id)?;
            primary.state = ProposalStateV2::SupersededByRollback;
            primary.terminal_slot = slot;
            primary.terminal_reason = MODEL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON;
        } else {
            let rollback_id = snapshot
                .rollback_proposal
                .ok_or(Release1ModelError::InvalidRollbackLink)?;
            let rollback = self.proposal_mut(rollback_id)?;
            if rollback.state != ProposalStateV2::Timelocked {
                return Err(Release1ModelError::InvalidRollbackLink);
            }
            rollback.state = ProposalStateV2::Retired;
            rollback.terminal_slot = slot;
            rollback.terminal_reason = MODEL_RETIRED_ROLLBACK_TERMINAL_REASON;
        }
        Ok(())
    }

    fn guardian_freeze(
        &mut self,
        guardian: Pubkey,
        slot: u64,
        reason: u16,
    ) -> Release1ModelResult<()> {
        if guardian != self.graph.guardian
            || self.gate.status != GateStatusV1::Active
            || slot == 0
            || reason == 0
            || reason == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
        {
            return Err(Release1ModelError::InvalidGate);
        }
        self.gate.epoch = checked_increment(self.gate.epoch)?;
        let epoch = self.gate.epoch;
        let freeze_observation = ModelEmergencyFreezeObservation {
            epoch,
            freeze_slot: slot,
            freeze_reason: reason,
            programdata: self.current_guardian_programdata_observation(),
        };
        if self
            .emergency_freeze_observations
            .insert(epoch, freeze_observation)
            .is_some()
        {
            return Err(Release1ModelError::InvalidGate);
        }
        self.gate.status = GateStatusV1::EmergencyFrozen;
        self.gate.active_proposal = None;
        self.gate.freeze_slot = slot;
        self.gate.freeze_reason = reason;
        Ok(())
    }

    fn create_emergency_resolution(&mut self, slot: u64) -> Release1ModelResult<u64> {
        if self.gate.status != GateStatusV1::EmergencyFrozen
            || self.gate.freeze_reason == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.gate.active_proposal.is_some()
            || slot == 0
        {
            return Err(Release1ModelError::InvalidGate);
        }
        let freeze_observation = self
            .emergency_freeze_observations
            .get(&self.gate.epoch)
            .ok_or(Release1ModelError::InvalidGate)?
            .clone();
        if freeze_observation.epoch != self.gate.epoch
            || freeze_observation.freeze_slot != self.gate.freeze_slot
            || freeze_observation.freeze_reason != self.gate.freeze_reason
        {
            return Err(Release1ModelError::InvalidGate);
        }
        let id = self.gate.epoch;
        if self.emergency_resolutions.contains_key(&id) {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let not_before_slot = freeze_observation
            .freeze_slot
            .checked_add(self.delays.routine_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let expiry_slot = freeze_observation
            .freeze_slot
            .checked_add(self.delays.proposal_expiry_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if not_before_slot >= expiry_slot
            || slot < freeze_observation.freeze_slot
            || slot >= expiry_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }
        self.emergency_resolutions.insert(
            id,
            ModelEmergencyResolution {
                id,
                state: EmergencyFreezeResolutionStateV1::Draft,
                frozen_epoch: self.gate.epoch,
                freeze_slot: self.gate.freeze_slot,
                freeze_reason: self.gate.freeze_reason,
                target_nonce: self.target_nonce,
                creation_slot: slot,
                not_before_slot,
                expiry_slot,
                observed_programdata: freeze_observation.programdata,
                approvals: ModelApprovalAccumulator::default(),
                checkpoint: None,
                checkpoint_attestations: Vec::new(),
                executed_slot: 0,
                terminal_reason: 0,
            },
        );
        Ok(id)
    }

    fn approve_emergency_resolution(
        &mut self,
        resolution_id: u64,
        seat: u8,
        slot: u64,
    ) -> Release1ModelResult<()> {
        self.require_emergency_binding(resolution_id, slot)?;
        let council = self.council.clone();
        let resolution = self.resolution_mut(resolution_id)?;
        if slot < resolution.not_before_slot {
            return Err(Release1ModelError::TimingViolation);
        }
        let approvals_stale = resolution.approvals.council_version != council.version
            || resolution.approvals.council_hash != council.hash;
        if resolution.state != EmergencyFreezeResolutionStateV1::Draft
            && !(approvals_stale
                && matches!(
                    resolution.state,
                    EmergencyFreezeResolutionStateV1::CouncilApproved
                        | EmergencyFreezeResolutionStateV1::Timelocked
                ))
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        if resolution.state != EmergencyFreezeResolutionStateV1::Draft {
            resolution.state = EmergencyFreezeResolutionStateV1::Draft;
        }
        if resolution.approvals.record_current(&council, seat, slot)? {
            resolution.state = EmergencyFreezeResolutionStateV1::CouncilApproved;
        }
        Ok(())
    }

    fn queue_emergency_resolution(
        &mut self,
        resolution_id: u64,
        slot: u64,
    ) -> Release1ModelResult<()> {
        self.require_emergency_binding(resolution_id, slot)?;
        let snapshot = self.resolution(resolution_id)?.clone();
        if slot < snapshot.not_before_slot {
            return Err(Release1ModelError::TimingViolation);
        }
        if snapshot.state != EmergencyFreezeResolutionStateV1::CouncilApproved
            || !snapshot.approvals.is_complete_at(&self.council, slot)?
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let resolution = self.resolution_mut(resolution_id)?;
        resolution.state = EmergencyFreezeResolutionStateV1::Timelocked;
        Ok(())
    }

    fn attest_emergency_checkpoint(
        &mut self,
        resolution_id: u64,
        seat: u8,
        hard_state: ModelHardStateObservation,
        forbidden_drift_count: u32,
        slot: u64,
    ) -> Release1ModelResult<()> {
        hard_state.validate()?;
        self.require_emergency_binding(resolution_id, slot)?;
        let council = self.council.clone();
        council.require_seat_active(seat, slot)?;
        let seat_authority = *council
            .seats
            .get(usize::from(seat))
            .ok_or(Release1ModelError::InvalidApproval)?;
        let resolution = self.resolution_mut(resolution_id)?;
        if slot < resolution.not_before_slot {
            return Err(Release1ModelError::TimingViolation);
        }
        if resolution.state != EmergencyFreezeResolutionStateV1::Timelocked
            || resolution.checkpoint.is_some()
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let attestation = ModelCheckpointAttestation {
            subject_id: resolution_id,
            phase: StateCheckpointPhaseV1::Emergency,
            gate_epoch: resolution.frozen_epoch,
            council_version: council.version,
            council_hash: council.hash,
            seat,
            seat_authority,
            hard_state,
            forbidden_drift_count,
            attested_slot: slot,
        };
        if let Some(existing) = resolution
            .checkpoint_attestations
            .iter_mut()
            .find(|existing| existing.council_version == council.version && existing.seat == seat)
        {
            *existing = attestation;
        } else {
            resolution.checkpoint_attestations.push(attestation);
        }
        Ok(())
    }

    fn finalize_emergency_checkpoint(
        &mut self,
        resolution_id: u64,
        attesting_seats: [u8; MODEL_ROUTINE_THRESHOLD as usize],
        slot: u64,
    ) -> Release1ModelResult<()> {
        if slot == 0 {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        self.require_emergency_binding(resolution_id, slot)?;
        let snapshot = self.resolution(resolution_id)?.clone();
        if slot < snapshot.not_before_slot {
            return Err(Release1ModelError::TimingViolation);
        }
        if snapshot.state != EmergencyFreezeResolutionStateV1::Timelocked
            || snapshot.checkpoint.is_some()
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let (selected, approvals) = self.select_checkpoint_quorum(
            resolution_id,
            StateCheckpointPhaseV1::Emergency,
            &snapshot.checkpoint_attestations,
            attesting_seats,
            slot,
        )?;
        let accepted = selected.forbidden_drift_count == 0;
        let gate_epoch = self.gate.epoch;
        self.resolution_mut(resolution_id)?.checkpoint = Some(ModelCheckpoint {
            phase: StateCheckpointPhaseV1::Emergency,
            gate_epoch,
            hard_state: selected.hard_state,
            forbidden_drift_count: selected.forbidden_drift_count,
            approvals,
            finalized: true,
            accepted,
            rejected_hard_mismatch: !accepted,
            accepted_slot: if accepted { slot } else { 0 },
        });
        Ok(())
    }

    fn execute_emergency_resume(
        &mut self,
        resolution_id: u64,
        slot: u64,
    ) -> Release1ModelResult<()> {
        self.require_emergency_binding(resolution_id, slot)?;
        let snapshot = self.resolution(resolution_id)?.clone();
        let checkpoint_ready = snapshot
            .checkpoint
            .as_ref()
            .map(|checkpoint| {
                checkpoint_accepted(Some(checkpoint))
                    && checkpoint.accepted_slot != 0
                    && slot >= checkpoint.accepted_slot
            })
            .unwrap_or(false);
        let current_programdata = self.current_guardian_programdata_observation();
        let programdata_changed = snapshot.observed_programdata != current_programdata
            || !current_programdata.is_canonical_for(
                &self.graph,
                &self.programdata,
                Some(self.graph.authority_pda),
            );
        if snapshot.state != EmergencyFreezeResolutionStateV1::Timelocked
            || slot < snapshot.not_before_slot
            || slot >= snapshot.expiry_slot
            || snapshot.approvals.council_version != self.council.version
            || snapshot.approvals.council_hash != self.council.hash
            || !snapshot.approvals.is_complete_at(&self.council, slot)?
            || !checkpoint_ready
            || programdata_changed
        {
            return Err(if programdata_changed {
                Release1ModelError::ProgramDataChanged
            } else {
                Release1ModelError::InvalidStateTransition
            });
        }
        self.gate.epoch = checked_increment(self.gate.epoch)?;
        self.clear_gate_to_active();
        let resolution = self.resolution_mut(resolution_id)?;
        resolution.state = EmergencyFreezeResolutionStateV1::Executed;
        resolution.executed_slot = slot;
        resolution.terminal_reason = EMERGENCY_RESOLUTION_EXECUTED_TERMINAL_REASON_V1;
        Ok(())
    }

    fn expire_emergency_resolution(
        &mut self,
        resolution_id: u64,
        slot: u64,
    ) -> Release1ModelResult<()> {
        self.require_emergency_binding(resolution_id, 0)?;
        let snapshot = self.resolution(resolution_id)?.clone();
        if !matches!(
            snapshot.state,
            EmergencyFreezeResolutionStateV1::Draft
                | EmergencyFreezeResolutionStateV1::CouncilApproved
                | EmergencyFreezeResolutionStateV1::Timelocked
        ) {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        if slot < snapshot.expiry_slot {
            return Err(Release1ModelError::TimingViolation);
        }
        let resolution = self.resolution_mut(resolution_id)?;
        resolution.state = EmergencyFreezeResolutionStateV1::Expired;
        resolution.terminal_reason = EMERGENCY_RESOLUTION_EXPIRED_TERMINAL_REASON_V1;
        Ok(())
    }

    fn create_candidate_council_set(
        &mut self,
        candidate_version: u64,
        candidate_seats: [Pubkey; MODEL_COUNCIL_SIZE],
        candidate_seat_terms: [ModelSeatTerm; MODEL_COUNCIL_SIZE],
        activation_slot: u64,
        slot: u64,
    ) -> Release1ModelResult<u64> {
        if slot == 0 || candidate_version <= self.council.version {
            return Err(Release1ModelError::InvalidCouncil);
        }
        let minimum_activation_slot = slot
            .checked_add(self.delays.major_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if activation_slot < minimum_activation_slot {
            return Err(Release1ModelError::InvalidTiming);
        }
        let candidate = ModelCouncil::new(
            candidate_version,
            activation_slot,
            candidate_seats,
            candidate_seat_terms,
        )?;
        if self.council_history.contains_key(&candidate_version) {
            return Err(Release1ModelError::InvalidCouncil);
        }
        let next_after_candidate = candidate_version
            .checked_add(1)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        self.council_history.insert(candidate_version, candidate);
        self.next_candidate_council_version = self
            .next_candidate_council_version
            .max(next_after_candidate);
        Ok(candidate_version)
    }

    fn create_council_rotation(
        &mut self,
        candidate_version: u64,
        slot: u64,
    ) -> Release1ModelResult<u64> {
        if slot == 0 || candidate_version <= self.council.version {
            return Err(Release1ModelError::InvalidTiming);
        }
        let minimum_not_before_slot = slot
            .checked_add(self.delays.major_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let expiry_slot = slot
            .checked_add(self.delays.proposal_expiry_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let candidate = self
            .council_history
            .get(&candidate_version)
            .cloned()
            .ok_or(Release1ModelError::NotFound)?;
        let not_before_slot = candidate.activation_slot;
        if not_before_slot < minimum_not_before_slot || not_before_slot >= expiry_slot {
            return Err(Release1ModelError::InvalidTiming);
        }
        let id = candidate_version;
        if self.rotations.contains_key(&id) {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        self.rotations.insert(
            candidate_version,
            ModelCouncilRotation {
                id,
                state: CouncilRotationStateV1::Draft,
                current_council_version: self.council.version,
                current_council_hash: self.council.hash,
                candidate,
                target_nonce: self.target_nonce,
                creation_slot: slot,
                not_before_slot,
                expiry_slot,
                approvals: ModelApprovalAccumulator::pinned(&self.council),
                cancellation_approvals: ModelApprovalAccumulator::default(),
                cancellation_reason: 0,
                terminal_slot: 0,
                terminal_reason: 0,
            },
        );
        Ok(id)
    }

    fn approve_council_rotation(
        &mut self,
        rotation_id: u64,
        seat: u8,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let council = self.council.clone();
        let target_nonce = self.target_nonce;
        let rotation = self.rotation_mut(rotation_id)?;
        if rotation.state != CouncilRotationStateV1::Draft
            || rotation.current_council_version != council.version
            || rotation.current_council_hash != council.hash
            || rotation.target_nonce != target_nonce
            || slot < rotation.creation_slot
            || slot >= rotation.expiry_slot
        {
            return Err(Release1ModelError::StaleBinding);
        }
        if rotation.approvals.record_pinned(&council, seat, slot)? {
            rotation.state = CouncilRotationStateV1::CouncilApproved;
        }
        Ok(())
    }

    fn queue_council_rotation(&mut self, rotation_id: u64, slot: u64) -> Release1ModelResult<()> {
        let snapshot = self
            .rotations
            .get(&rotation_id)
            .cloned()
            .ok_or(Release1ModelError::NotFound)?;
        if snapshot.state != CouncilRotationStateV1::CouncilApproved
            || snapshot.current_council_version != self.council.version
            || snapshot.current_council_hash != self.council.hash
            || snapshot.target_nonce != self.target_nonce
            || slot < snapshot.creation_slot
            || slot >= snapshot.expiry_slot
            || !snapshot.approvals.is_complete_at(&self.council, slot)?
        {
            return Err(Release1ModelError::InvalidStateTransition);
        }
        let rotation = self.rotation_mut(rotation_id)?;
        rotation.state = CouncilRotationStateV1::Timelocked;
        Ok(())
    }

    fn approve_council_rotation_cancellation(
        &mut self,
        rotation_id: u64,
        seat: u8,
        slot: u64,
        reason: u16,
    ) -> Release1ModelResult<()> {
        let council = self.council.clone();
        let snapshot = self
            .rotations
            .get(&rotation_id)
            .cloned()
            .ok_or(Release1ModelError::NotFound)?;
        if !matches!(
            snapshot.state,
            CouncilRotationStateV1::Draft
                | CouncilRotationStateV1::CouncilApproved
                | CouncilRotationStateV1::Timelocked
        ) || snapshot.current_council_version != council.version
            || snapshot.current_council_hash != council.hash
            || snapshot.target_nonce != self.target_nonce
            || slot < snapshot.creation_slot
            || slot >= snapshot.expiry_slot
            || reason == 0
            || (snapshot.cancellation_reason != 0 && snapshot.cancellation_reason != reason)
            || snapshot.terminal_reason != 0
        {
            return Err(Release1ModelError::StaleBinding);
        }
        let rotation = self.rotation_mut(rotation_id)?;
        if rotation.cancellation_reason == 0 {
            rotation.cancellation_reason = reason;
        }
        if rotation
            .cancellation_approvals
            .record_current(&council, seat, slot)?
        {
            rotation.state = CouncilRotationStateV1::Cancelled;
            rotation.terminal_slot = slot;
            rotation.terminal_reason = reason;
        }
        Ok(())
    }

    fn expire_council_rotation(&mut self, rotation_id: u64, slot: u64) -> Release1ModelResult<()> {
        let snapshot = self
            .rotations
            .get(&rotation_id)
            .cloned()
            .ok_or(Release1ModelError::NotFound)?;
        if !matches!(
            snapshot.state,
            CouncilRotationStateV1::Draft
                | CouncilRotationStateV1::CouncilApproved
                | CouncilRotationStateV1::Timelocked
        ) || slot < snapshot.expiry_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }
        let rotation = self.rotation_mut(rotation_id)?;
        rotation.state = CouncilRotationStateV1::Expired;
        rotation.terminal_slot = slot;
        rotation.terminal_reason = COUNCIL_ROTATION_EXPIRED_TERMINAL_REASON_V1;
        Ok(())
    }

    fn activate_council_rotation(
        &mut self,
        rotation_id: u64,
        slot: u64,
    ) -> Release1ModelResult<()> {
        let rotation = self
            .rotations
            .get(&rotation_id)
            .cloned()
            .ok_or(Release1ModelError::NotFound)?;
        let canonical_candidate = self
            .council_history
            .get(&rotation.candidate.version)
            .ok_or(Release1ModelError::StaleBinding)?;
        if rotation.state != CouncilRotationStateV1::Timelocked
            || slot < rotation.not_before_slot
            || slot >= rotation.expiry_slot
            || rotation.current_council_version != self.council.version
            || rotation.current_council_hash != self.council.hash
            || rotation.target_nonce != self.target_nonce
            || rotation.id != rotation.candidate.version
            || canonical_candidate != &rotation.candidate
            || !rotation.approvals.is_complete_at(&self.council, slot)?
        {
            return Err(Release1ModelError::StaleBinding);
        }
        rotation
            .candidate
            .require_mask_active(MODEL_VALID_APPROVAL_MASK, slot)?;
        self.council = rotation.candidate;
        let rotation = self.rotation_mut(rotation_id)?;
        rotation.state = CouncilRotationStateV1::Activated;
        rotation.terminal_slot = slot;
        rotation.terminal_reason = COUNCIL_ROTATION_ACTIVATED_TERMINAL_REASON_V1;
        Ok(())
    }

    fn validate_prepared_rollback(
        &self,
        primary: &ModelProposal,
        current_slot: u64,
    ) -> Release1ModelResult<()> {
        let rollback_id = primary
            .rollback_proposal
            .ok_or(Release1ModelError::InvalidRollbackLink)?;
        let rollback = self.proposal(rollback_id)?;
        if rollback.class != ProposalClassV1::EmergencyRollback
            || rollback.primary_proposal != Some(primary.id)
            || rollback.rollback_proposal.is_some()
            || rollback.target_nonce != primary.target_nonce
            || rollback.state != ProposalStateV2::Timelocked
        {
            return Err(Release1ModelError::InvalidRollbackLink);
        }
        let executable_after_delay = current_slot
            .checked_add(self.delays.rollback_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        let primary_expiry_with_rollback_runway = primary
            .timing
            .expiry_slot
            .checked_add(self.delays.rollback_slots)
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if current_slot >= rollback.timing.expiry_slot
            || executable_after_delay >= rollback.timing.expiry_slot
            || rollback.timing.expiry_slot <= primary_expiry_with_rollback_runway
        {
            return Err(Release1ModelError::TimingViolation);
        }
        if !self.pinned_initial_quorum_complete(rollback)? {
            return Err(Release1ModelError::InvalidRollbackLink);
        }
        Ok(())
    }

    fn require_primary_execution_rollback_runway(
        &self,
        primary: &ModelProposal,
        current_slot: u64,
    ) -> Release1ModelResult<()> {
        let rollback_id = primary
            .rollback_proposal
            .ok_or(Release1ModelError::InvalidRollbackLink)?;
        let rollback = self.proposal(rollback_id)?;
        let recovery_horizon = current_slot
            .checked_add(self.delays.rollback_slots)
            .and_then(|value| value.checked_add(self.delays.review_slots))
            .and_then(|value| value.checked_add(1))
            .ok_or(Release1ModelError::ArithmeticOverflow)?;
        if current_slot < rollback.timing.not_before_slot
            || recovery_horizon >= rollback.timing.expiry_slot
        {
            return Err(Release1ModelError::TimingViolation);
        }
        Ok(())
    }

    fn require_current_nonce(&self, proposal: &ModelProposal) -> Release1ModelResult<()> {
        if proposal.target_nonce != self.target_nonce {
            return Err(Release1ModelError::StaleBinding);
        }
        Ok(())
    }

    fn pinned_initial_quorum_complete(
        &self,
        proposal: &ModelProposal,
    ) -> Release1ModelResult<bool> {
        let council = self
            .council_history
            .get(&proposal.creation_council_version)
            .ok_or(Release1ModelError::StaleBinding)?;
        proposal.initial_approvals.validate()?;
        if council.hash != proposal.creation_council_hash
            || proposal.initial_approvals.council_version != council.version
            || proposal.initial_approvals.council_hash != council.hash
        {
            return Err(Release1ModelError::StaleBinding);
        }
        Ok(proposal.initial_approvals.is_complete())
    }

    fn require_creation_gate(&self, proposal: &ModelProposal) -> Release1ModelResult<()> {
        if proposal.creation_gate_status != self.gate.status
            || proposal.creation_gate_epoch != self.gate.epoch
            || (self.gate.status == GateStatusV1::EmergencyFrozen
                && self.gate.freeze_reason == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1)
        {
            return Err(Release1ModelError::StaleBinding);
        }
        match proposal.creation_gate_status {
            GateStatusV1::Active => {
                if proposal.creation_emergency_observation.is_some() {
                    return Err(Release1ModelError::StaleBinding);
                }
            }
            GateStatusV1::EmergencyFrozen => {
                let proposal_observation = proposal
                    .creation_emergency_observation
                    .as_ref()
                    .ok_or(Release1ModelError::StaleBinding)?;
                let canonical_observation = self
                    .emergency_freeze_observations
                    .get(&self.gate.epoch)
                    .ok_or(Release1ModelError::StaleBinding)?;
                if proposal_observation != canonical_observation {
                    return Err(Release1ModelError::StaleBinding);
                }
            }
            GateStatusV1::FrozenForUpgrade => {
                return Err(Release1ModelError::StaleBinding);
            }
        }
        if proposal.creation_programdata != self.programdata {
            return Err(Release1ModelError::ProgramDataChanged);
        }
        self.require_controller_programdata_authority()?;
        Ok(())
    }

    fn require_frozen_proposal(&self, proposal: &ModelProposal) -> Release1ModelResult<()> {
        if self.gate.status != GateStatusV1::FrozenForUpgrade
            || self.gate.active_proposal != Some(proposal.id)
            || proposal.freeze_gate_epoch == 0
            || proposal.freeze_gate_epoch != self.gate.epoch
            || self.target_nonce != checked_increment(proposal.target_nonce)?
        {
            return Err(Release1ModelError::InvalidGate);
        }
        Ok(())
    }

    fn require_emergency_binding(&self, resolution_id: u64, slot: u64) -> Release1ModelResult<()> {
        let resolution = self.resolution(resolution_id)?;
        let freeze_observation = self
            .emergency_freeze_observations
            .get(&resolution.frozen_epoch)
            .ok_or(Release1ModelError::StaleBinding)?;
        if self.gate.status != GateStatusV1::EmergencyFrozen
            || self.gate.active_proposal.is_some()
            || self.gate.epoch != resolution.frozen_epoch
            || self.gate.freeze_slot != resolution.freeze_slot
            || self.gate.freeze_reason != resolution.freeze_reason
            || self.gate.freeze_reason == BOOTSTRAP_INITIALIZATION_FREEZE_REASON_V1
            || self.target_nonce != resolution.target_nonce
            || freeze_observation.epoch != resolution.frozen_epoch
            || freeze_observation.freeze_slot != resolution.freeze_slot
            || freeze_observation.freeze_reason != resolution.freeze_reason
            || freeze_observation.programdata != resolution.observed_programdata
            || (slot != 0 && slot >= resolution.expiry_slot)
        {
            return Err(Release1ModelError::StaleBinding);
        }
        Ok(())
    }

    fn clear_gate_to_active(&mut self) {
        self.gate.status = GateStatusV1::Active;
        self.gate.active_proposal = None;
        self.gate.freeze_slot = 0;
        self.gate.freeze_reason = 0;
    }

    fn current_guardian_programdata_observation(&self) -> ModelGuardianProgramDataObservation {
        self.current_guardian_runtime_observation.clone()
    }

    fn require_controller_programdata_authority(&self) -> Release1ModelResult<()> {
        if self.current_programdata_authority != Some(self.graph.authority_pda)
            || !self.current_guardian_runtime_observation.is_canonical_for(
                &self.graph,
                &self.programdata,
                Some(self.graph.authority_pda),
            )
        {
            return Err(Release1ModelError::ProgramDataChanged);
        }
        Ok(())
    }

    fn proposal(&self, id: u64) -> Release1ModelResult<&ModelProposal> {
        self.proposals.get(&id).ok_or(Release1ModelError::NotFound)
    }

    fn proposal_mut(&mut self, id: u64) -> Release1ModelResult<&mut ModelProposal> {
        self.proposals
            .get_mut(&id)
            .ok_or(Release1ModelError::NotFound)
    }

    fn rotation_mut(&mut self, id: u64) -> Release1ModelResult<&mut ModelCouncilRotation> {
        self.rotations
            .get_mut(&id)
            .ok_or(Release1ModelError::NotFound)
    }

    fn resolution(&self, id: u64) -> Release1ModelResult<&ModelEmergencyResolution> {
        self.emergency_resolutions
            .get(&id)
            .ok_or(Release1ModelError::NotFound)
    }

    fn resolution_mut(&mut self, id: u64) -> Release1ModelResult<&mut ModelEmergencyResolution> {
        self.emergency_resolutions
            .get_mut(&id)
            .ok_or(Release1ModelError::NotFound)
    }
}

pub fn routine_mask_satisfied(bitset: u8) -> Release1ModelResult<bool> {
    if bitset & !MODEL_VALID_APPROVAL_MASK != 0 {
        return Err(Release1ModelError::InvalidApproval);
    }
    Ok(bitset.count_ones() as u8 >= MODEL_ROUTINE_THRESHOLD)
}

pub fn terminal_mask_satisfied(bitset: u8) -> Release1ModelResult<bool> {
    if bitset & !MODEL_VALID_APPROVAL_MASK != 0 {
        return Err(Release1ModelError::InvalidApproval);
    }
    Ok(bitset.count_ones() as u8 >= MODEL_TERMINAL_THRESHOLD)
}

pub const fn proposal_edge_allowed(
    current: ProposalStateV2,
    next: ProposalStateV2,
    extension_required: bool,
) -> bool {
    match (current, next) {
        (ProposalStateV2::Draft, ProposalStateV2::BufferAdopted)
        | (ProposalStateV2::BufferAdopted, ProposalStateV2::BufferVerified)
        | (ProposalStateV2::BufferVerified, ProposalStateV2::CouncilApproved)
        | (ProposalStateV2::CouncilApproved, ProposalStateV2::GovernanceSatisfied)
        | (ProposalStateV2::GovernanceSatisfied, ProposalStateV2::Timelocked)
        | (ProposalStateV2::Timelocked, ProposalStateV2::Frozen)
        | (ProposalStateV2::Extended, ProposalStateV2::UpgradeExecuted)
        | (ProposalStateV2::UpgradeExecuted, ProposalStateV2::ProgramDataVerified)
        | (ProposalStateV2::ProgramDataVerified, ProposalStateV2::PoststateAccepted)
        | (ProposalStateV2::PoststateAccepted, ProposalStateV2::UnfreezeApproved)
        | (ProposalStateV2::UnfreezeApproved, ProposalStateV2::Completed) => true,
        (ProposalStateV2::Frozen, ProposalStateV2::Extended) => extension_required,
        (ProposalStateV2::Frozen, ProposalStateV2::UpgradeExecuted) => !extension_required,
        _ => false,
    }
}

fn checkpoint_accepted(checkpoint: Option<&ModelCheckpoint>) -> bool {
    checkpoint
        .is_some_and(|value| value.finalized && value.accepted && value.approvals.is_complete())
}

fn checked_increment(value: u64) -> Release1ModelResult<u64> {
    value
        .checked_add(1)
        .ok_or(Release1ModelError::ArithmeticOverflow)
}
