use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    pda::UPGRADEABLE_LOADER_ID,
    release1_state::{
        BufferVerificationStatusV1, CouncilRotationStateV1, EmergencyFreezeResolutionKindV1,
        EmergencyFreezeResolutionStateV1, ProgramDataMismatchClassV1,
        ProgramDataVerificationStatusV1, ProposalStateV2, StateCheckpointPhaseV1,
        LOADER_V3_PROGRAMDATA_METADATA_LEN_V1, LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
        MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{GateStatusV1, ProposalClassV1},
};

pub const MAX_CONTROLLER_INSTRUCTION_DATA_LEN: usize = 16_384;

pub const RECORD_PROPOSAL_APPROVAL_V1_TAG: u8 = 0;
pub const RECORD_PROPOSAL_APPROVAL_V1_LEN: usize = 1 + 32 + 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordProposalApprovalV1 {
    pub expected_proposal_digest: [u8; 32],
    pub expected_council_version: u64,
}

impl RecordProposalApprovalV1 {
    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() != RECORD_PROPOSAL_APPROVAL_V1_LEN
            || data[0] != RECORD_PROPOSAL_APPROVAL_V1_TAG
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut expected_proposal_digest = [0u8; 32];
        expected_proposal_digest.copy_from_slice(&data[1..33]);
        let mut version = [0u8; 8];
        version.copy_from_slice(&data[33..41]);
        Ok(Self {
            expected_proposal_digest,
            expected_council_version: u64::from_le_bytes(version),
        })
    }

    pub fn pack(self) -> [u8; RECORD_PROPOSAL_APPROVAL_V1_LEN] {
        let mut data = [0u8; RECORD_PROPOSAL_APPROVAL_V1_LEN];
        data[0] = RECORD_PROPOSAL_APPROVAL_V1_TAG;
        data[1..33].copy_from_slice(&self.expected_proposal_digest);
        data[33..41].copy_from_slice(&self.expected_council_version.to_le_bytes());
        data
    }
}

#[allow(clippy::too_many_arguments)]
pub fn record_proposal_approval_instruction(
    controller_program: Pubkey,
    controller_config: Pubkey,
    policy: Pubkey,
    council: Pubkey,
    proposal: Pubkey,
    seat_authority: Pubkey,
    expected_proposal_digest: [u8; 32],
    expected_council_version: u64,
) -> Instruction {
    Instruction {
        program_id: controller_program,
        accounts: vec![
            AccountMeta::new_readonly(controller_config, false),
            AccountMeta::new_readonly(policy, false),
            AccountMeta::new_readonly(council, false),
            AccountMeta::new(proposal, false),
            AccountMeta::new_readonly(seat_authority, true),
        ],
        data: RecordProposalApprovalV1 {
            expected_proposal_digest,
            expected_council_version,
        }
        .pack()
        .to_vec(),
    }
}

struct FixedReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> FixedReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], ProgramError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or(ProgramError::InvalidInstructionData)?;
        let source = self
            .bytes
            .get(self.offset..end)
            .ok_or(ProgramError::InvalidInstructionData)?;
        let mut out = [0u8; N];
        out.copy_from_slice(source);
        self.offset = end;
        Ok(out)
    }

    fn finish(self) -> Result<(), ProgramError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }
}

struct FixedWriter<'a> {
    bytes: &'a mut [u8],
    offset: usize,
}

impl<'a> FixedWriter<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn put(&mut self, value: &[u8]) {
        let end = self
            .offset
            .checked_add(value.len())
            .expect("fixed instruction length overflow");
        self.bytes[self.offset..end].copy_from_slice(value);
        self.offset = end;
    }

    fn finish(self) {
        assert_eq!(self.offset, self.bytes.len());
    }
}

trait FixedWire: Sized {
    const LEN: usize;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError>;
    fn encode(&self, writer: &mut FixedWriter<'_>);
}

impl FixedWire for u8 {
    const LEN: usize = 1;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        Ok(reader.take::<1>()?[0])
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        writer.put(&[*self]);
    }
}

impl FixedWire for bool {
    const LEN: usize = 1;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        match u8::decode(reader)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        u8::from(*self).encode(writer);
    }
}

macro_rules! fixed_wire_integer {
    ($type:ty, $len:expr) => {
        impl FixedWire for $type {
            const LEN: usize = $len;

            fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
                Ok(<$type>::from_le_bytes(reader.take::<$len>()?))
            }

            fn encode(&self, writer: &mut FixedWriter<'_>) {
                writer.put(&self.to_le_bytes());
            }
        }
    };
}

fixed_wire_integer!(u16, 2);
fixed_wire_integer!(u32, 4);
fixed_wire_integer!(u64, 8);

impl FixedWire for Pubkey {
    const LEN: usize = 32;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        Ok(Self::new_from_array(reader.take::<32>()?))
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        writer.put(self.as_ref());
    }
}

impl<T: Copy + Default + FixedWire, const N: usize> FixedWire for [T; N] {
    const LEN: usize = T::LEN * N;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        let mut values = [T::default(); N];
        for value in &mut values {
            *value = T::decode(reader)?;
        }
        Ok(values)
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        for value in self {
            value.encode(writer);
        }
    }
}

macro_rules! fixed_wire_enum {
    ($type:ty { $($value:expr => $variant:path),+ $(,)? }) => {
        impl FixedWire for $type {
            const LEN: usize = 1;

            fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
                match u8::decode(reader)? {
                    $($value => Ok($variant),)+
                    _ => Err(ProgramError::InvalidInstructionData),
                }
            }

            fn encode(&self, writer: &mut FixedWriter<'_>) {
                (*self as u8).encode(writer);
            }
        }
    };
}

fixed_wire_enum!(ProposalClassV1 {
    0 => ProposalClassV1::RoutineUpgrade,
    1 => ProposalClassV1::EmergencyRollback,
    2 => ProposalClassV1::EconomicChange,
    3 => ProposalClassV1::ConstitutionalChange,
});
fixed_wire_enum!(GateStatusV1 {
    0 => GateStatusV1::Active,
    1 => GateStatusV1::FrozenForUpgrade,
    2 => GateStatusV1::EmergencyFrozen,
});
fixed_wire_enum!(ProposalStateV2 {
    0 => ProposalStateV2::Draft,
    1 => ProposalStateV2::BufferAdopted,
    2 => ProposalStateV2::BufferVerified,
    3 => ProposalStateV2::CouncilApproved,
    5 => ProposalStateV2::GovernanceSatisfied,
    6 => ProposalStateV2::Timelocked,
    7 => ProposalStateV2::Frozen,
    8 => ProposalStateV2::Extended,
    9 => ProposalStateV2::UpgradeExecuted,
    10 => ProposalStateV2::ProgramDataVerified,
    11 => ProposalStateV2::PoststateAccepted,
    12 => ProposalStateV2::UnfreezeApproved,
    13 => ProposalStateV2::Completed,
    14 => ProposalStateV2::Cancelled,
    15 => ProposalStateV2::Expired,
    16 => ProposalStateV2::SupersededByRollback,
    17 => ProposalStateV2::Retired,
});
fixed_wire_enum!(StateCheckpointPhaseV1 {
    0 => StateCheckpointPhaseV1::Prestate,
    1 => StateCheckpointPhaseV1::Poststate,
    2 => StateCheckpointPhaseV1::Emergency,
});
fixed_wire_enum!(CouncilRotationStateV1 {
    0 => CouncilRotationStateV1::Draft,
    1 => CouncilRotationStateV1::CouncilApproved,
    2 => CouncilRotationStateV1::Timelocked,
    3 => CouncilRotationStateV1::Activated,
    4 => CouncilRotationStateV1::Cancelled,
    5 => CouncilRotationStateV1::Expired,
});
fixed_wire_enum!(EmergencyFreezeResolutionStateV1 {
    0 => EmergencyFreezeResolutionStateV1::Draft,
    1 => EmergencyFreezeResolutionStateV1::CouncilApproved,
    2 => EmergencyFreezeResolutionStateV1::Timelocked,
    3 => EmergencyFreezeResolutionStateV1::Executed,
    4 => EmergencyFreezeResolutionStateV1::Cancelled,
    5 => EmergencyFreezeResolutionStateV1::Expired,
});
fixed_wire_enum!(EmergencyFreezeResolutionKindV1 {
    0 => EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
});
fixed_wire_enum!(BufferVerificationStatusV1 {
    0 => BufferVerificationStatusV1::Adopted,
    1 => BufferVerificationStatusV1::Verifying,
    2 => BufferVerificationStatusV1::ReadyToFinalize,
    3 => BufferVerificationStatusV1::Verified,
    4 => BufferVerificationStatusV1::ConsumedByUpgrade,
    5 => BufferVerificationStatusV1::ClosedAbandoned,
});
fixed_wire_enum!(ProgramDataVerificationStatusV1 {
    0 => ProgramDataVerificationStatusV1::Verifying,
    1 => ProgramDataVerificationStatusV1::ReadyToFinalize,
    2 => ProgramDataVerificationStatusV1::Verified,
});
fixed_wire_enum!(ProgramDataMismatchClassV1 {
    0 => ProgramDataMismatchClassV1::Header,
    1 => ProgramDataMismatchClassV1::Authority,
    2 => ProgramDataMismatchClassV1::Capacity,
    3 => ProgramDataMismatchClassV1::PayloadLeaf,
    4 => ProgramDataMismatchClassV1::ZeroTail,
});

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptionalInstructionPubkeyV1(Option<Pubkey>);

impl OptionalInstructionPubkeyV1 {
    pub const fn none() -> Self {
        Self(None)
    }

    pub fn some(value: Pubkey) -> Result<Self, ProgramError> {
        if value == Pubkey::default() {
            Err(ProgramError::InvalidInstructionData)
        } else {
            Ok(Self(Some(value)))
        }
    }

    pub const fn value(self) -> Option<Pubkey> {
        self.0
    }
}

impl FixedWire for OptionalInstructionPubkeyV1 {
    const LEN: usize = 33;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        let present = u8::decode(reader)?;
        let value = Pubkey::decode(reader)?;
        match (present, value == Pubkey::default()) {
            (0, true) => Ok(Self::none()),
            (1, false) => Ok(Self(Some(value))),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        match self.0 {
            None => {
                0u8.encode(writer);
                Pubkey::default().encode(writer);
            }
            Some(value) => {
                1u8.encode(writer);
                value.encode(writer);
            }
        }
    }
}

fn validate_instruction_program_observation_shape(
    header_present: bool,
    data_length: u64,
    linked_programdata: OptionalInstructionPubkeyV1,
) -> Result<(), ProgramError> {
    if header_present {
        if data_length != LOADER_V3_PROGRAM_ACCOUNT_LEN_V1 || linked_programdata.value().is_none() {
            return Err(ProgramError::InvalidInstructionData);
        }
    } else if linked_programdata.value().is_some() {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

fn validate_instruction_programdata_observation_shape(
    header_present: bool,
    data_length: u64,
    deployed_slot: u64,
    capacity: u64,
    authority: OptionalInstructionPubkeyV1,
) -> Result<(), ProgramError> {
    if header_present {
        if data_length
            != capacity
                .checked_add(LOADER_V3_PROGRAMDATA_METADATA_LEN_V1)
                .ok_or(ProgramError::InvalidInstructionData)?
        {
            return Err(ProgramError::InvalidInstructionData);
        }
    } else if deployed_slot != 0 || capacity != 0 || authority.value().is_some() {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

fn validate_instruction_raw_programdata_hash_shape(
    hash_complete: bool,
    raw_hash: &[u8; 32],
    data_length: u64,
) -> Result<(), ProgramError> {
    let within_atomic_ceiling = data_length <= MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1;
    if hash_complete != within_atomic_ceiling
        || (hash_complete && *raw_hash == [0; 32])
        || (!hash_complete && *raw_hash != [0; 32])
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

macro_rules! fixed_wire_struct {
    ($(#[$meta:meta])* pub struct $name:ident { $($field:ident: $type:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name {
            $(pub $field: $type,)*
        }

        impl FixedWire for $name {
            const LEN: usize = 0 $(+ <$type as FixedWire>::LEN)*;

            fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
                Ok(Self {
                    $($field: <$type as FixedWire>::decode(reader)?,)*
                })
            }

            fn encode(&self, writer: &mut FixedWriter<'_>) {
                $(self.$field.encode(writer);)*
            }
        }
    };
}

fixed_wire_struct! {
    /// Immutable term commitments for one seat authority supplied as a read-only account.
    #[derive(Default)]
    pub struct CouncilSeatTermV1 {
        term_start_slot: u64,
        term_end_slot: u64,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CheckpointSubjectStateV1 {
    ProposalFrozen = 0,
    ProposalProgramDataVerified = 1,
    EmergencyResolutionTimelocked = 2,
}

fixed_wire_enum!(CheckpointSubjectStateV1 {
    0 => CheckpointSubjectStateV1::ProposalFrozen,
    1 => CheckpointSubjectStateV1::ProposalProgramDataVerified,
    2 => CheckpointSubjectStateV1::EmergencyResolutionTimelocked,
});

impl Default for CheckpointSubjectStateV1 {
    fn default() -> Self {
        Self::ProposalFrozen
    }
}

fixed_wire_struct! {
    /// Signed observation guard shared by proposal lifecycle transitions.
    pub struct ProposalExpectationV2 {
        expected_proposal_digest: [u8; 32],
        expected_policy_version: u64,
        expected_policy_hash: [u8; 32],
        expected_council_version: u64,
        expected_council_hash: [u8; 32],
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_state: ProposalStateV2,
        expected_review_start_slot: u64,
        expected_review_end_slot: u64,
        expected_not_before_slot: u64,
        expected_expiry_slot: u64,
    }
}

fixed_wire_struct! {
    /// Signed observation guard shared by emergency-resolution transitions.
    pub struct EmergencyResolutionExpectationV1 {
        expected_resolution_digest: [u8; 32],
        expected_policy_version: u64,
        expected_policy_hash: [u8; 32],
        expected_council_version: u64,
        expected_council_hash: [u8; 32],
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        expected_freeze_slot: u64,
        expected_freeze_reason_code: u16,
        expected_target_nonce: u64,
        expected_state: EmergencyFreezeResolutionStateV1,
        expected_not_before_slot: u64,
        expected_expiry_slot: u64,
    }
}

fixed_wire_struct! {
    /// Exact candidate bytes independently signed by seats before the canonical
    /// checkpoint account exists. Controller/config/target identities and the
    /// finalized approval fields are derived from canonical accounts.
    pub struct CheckpointCandidateV1 {
        phase: StateCheckpointPhaseV1,
        expected_subject_state: CheckpointSubjectStateV1,
        expected_subject_digest: [u8; 32],
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        finalized_observation_slot: u64,
        target_programdata_slot: u64,
        target_payload_commitment: [u8; 32],
        target_raw_programdata_commitment: [u8; 32],
        target_capacity: u64,
        program_owned_state_root: [u8; 32],
        program_owned_state_count: u64,
        logical_compressed_state_root: [u8; 32],
        logical_compressed_state_count: u64,
        semantic_custody_accounting_root: [u8; 32],
        hard_combined_root: [u8; 32],
        external_metadata_observation_root: [u8; 32],
        external_raw_balance_observation_root: [u8; 32],
        schema_identifier: [u8; 32],
        admitted_positive_donation_root: [u8; 32],
        admitted_positive_donation_count: u64,
        forbidden_drift_count: u32,
        expected_checkpoint_digest: [u8; 32],
    }
}

fixed_wire_struct! {
    /// Signed observation guard shared by council-rotation transitions.
    pub struct CouncilRotationExpectationV1 {
        expected_rotation_digest: [u8; 32],
        expected_current_council_version: u64,
        expected_current_council_hash: [u8; 32],
        expected_candidate_council_version: u64,
        expected_candidate_council_hash: [u8; 32],
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_state: CouncilRotationStateV1,
        expected_not_before_slot: u64,
        expected_expiry_slot: u64,
    }
}

pub const MAX_FIXED_MERKLE_PROOF_NODES_V1: usize = 7;

/// A bounded binary-Merkle proof. Nodes after `proof_len` are canonical zero
/// padding; decode rejects any alternate representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedMerkleProofV1 {
    pub proof_len: u8,
    pub nodes: [[u8; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1],
}

impl FixedMerkleProofV1 {
    pub const LEN: usize = 1 + 32 * MAX_FIXED_MERKLE_PROOF_NODES_V1;

    pub fn validate(&self) -> Result<(), ProgramError> {
        let used = usize::from(self.proof_len);
        if used > MAX_FIXED_MERKLE_PROOF_NODES_V1
            || self.nodes[used..].iter().any(|node| *node != [0; 32])
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    pub fn empty() -> Self {
        Self {
            proof_len: 0,
            nodes: [[0; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1],
        }
    }
}

impl FixedWire for FixedMerkleProofV1 {
    const LEN: usize = Self::LEN;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        let value = Self {
            proof_len: u8::decode(reader)?,
            nodes: <[[u8; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1]>::decode(reader)?,
        };
        value.validate()?;
        Ok(value)
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        self.proof_len.encode(writer);
        self.nodes.encode(writer);
    }
}

pub const MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1: u32 = 1_400_000;
pub const MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1: u64 = 10_000_000;

/// Exact admitted top-level transaction envelope. Optional nonce fields are
/// both absent or both present, and their optional-key codec rejects default
/// public keys. These bounds are shared by the Rust and TypeScript codecs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeExpectationV1 {
    pub compute_unit_limit: u32,
    pub compute_unit_price_micro_lamports: u64,
    pub durable_nonce_account: OptionalInstructionPubkeyV1,
    pub durable_nonce_authority: OptionalInstructionPubkeyV1,
}

impl EnvelopeExpectationV1 {
    pub fn validate(&self) -> Result<(), ProgramError> {
        if self.compute_unit_limit == 0
            || self.compute_unit_limit > MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1
            || self.compute_unit_price_micro_lamports
                > MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1
            || self.durable_nonce_account.value().is_some()
                != self.durable_nonce_authority.value().is_some()
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl FixedWire for EnvelopeExpectationV1 {
    const LEN: usize = 78;

    fn decode(reader: &mut FixedReader<'_>) -> Result<Self, ProgramError> {
        let value = Self {
            compute_unit_limit: u32::decode(reader)?,
            compute_unit_price_micro_lamports: u64::decode(reader)?,
            durable_nonce_account: OptionalInstructionPubkeyV1::decode(reader)?,
            durable_nonce_authority: OptionalInstructionPubkeyV1::decode(reader)?,
        };
        value.validate()?;
        Ok(value)
    }

    fn encode(&self, writer: &mut FixedWriter<'_>) {
        self.compute_unit_limit.encode(writer);
        self.compute_unit_price_micro_lamports.encode(writer);
        self.durable_nonce_account.encode(writer);
        self.durable_nonce_authority.encode(writer);
    }
}

fixed_wire_struct! {
    /// Re-read guard for unfreeze approval and execution. This deliberately
    /// uses the current council while retaining the immutable proposal digest.
    pub struct UnfreezeExpectationV1 {
        expected_proposal_digest: [u8; 32],
        expected_policy_version: u64,
        expected_policy_hash: [u8; 32],
        expected_current_council_version: u64,
        expected_current_council_hash: [u8; 32],
        expected_frozen_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_proposal_state: ProposalStateV2,
        expected_poststate_checkpoint_digest: [u8; 32],
        expected_programdata_authority: Pubkey,
        expected_programdata_deployed_slot: u64,
        expected_programdata_capacity: u64,
        expected_raw_programdata_hash: [u8; 32],
        expected_unfreeze_approval_bitset: u8,
        expected_unfreeze_approval_count: u8,
        expected_programdata_verification_finalized_slot: u64,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProgramDataChunkPhaseV1 {
    Payload = 0,
    ZeroTail = 1,
}

fixed_wire_enum!(ProgramDataChunkPhaseV1 {
    0 => ProgramDataChunkPhaseV1::Payload,
    1 => ProgramDataChunkPhaseV1::ZeroTail,
});

macro_rules! fixed_instruction {
    (
        $name:ident,
        tag $tag_const:ident = $tag:expr,
        len $len_const:ident,
        { $($field:ident: $type:ty),* $(,)? }
    ) => {
        pub const $tag_const: u8 = $tag;
        pub const $len_const: usize = 1 $(+ <$type as FixedWire>::LEN)*;

        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            $(pub $field: $type,)*
        }

        impl $name {
            pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
                if data.len() != $len_const || data.first().copied() != Some($tag_const) {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let mut reader = FixedReader::new(&data[1..]);
                let value = Self {
                    $($field: <$type as FixedWire>::decode(&mut reader)?,)*
                };
                reader.finish()?;
                Ok(value)
            }

            pub fn pack(&self) -> [u8; $len_const] {
                let mut data = [0u8; $len_const];
                data[0] = $tag_const;
                let mut writer = FixedWriter::new(&mut data[1..]);
                $(self.$field.encode(&mut writer);)*
                writer.finish();
                data
            }
        }
    };
}

macro_rules! fixed_instruction_validated {
    (
        $name:ident,
        tag $tag_const:ident = $tag:expr,
        len $len_const:ident,
        validate $validator:ident,
        { $($field:ident: $type:ty),* $(,)? }
    ) => {
        pub const $tag_const: u8 = $tag;
        pub const $len_const: usize = 1 $(+ <$type as FixedWire>::LEN)*;

        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            $(pub $field: $type,)*
        }

        impl $name {
            pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
                if data.len() != $len_const || data.first().copied() != Some($tag_const) {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let mut reader = FixedReader::new(&data[1..]);
                let value = Self {
                    $($field: <$type as FixedWire>::decode(&mut reader)?,)*
                };
                reader.finish()?;
                value.$validator()?;
                Ok(value)
            }

            pub fn pack(&self) -> [u8; $len_const] {
                let mut data = [0u8; $len_const];
                data[0] = $tag_const;
                let mut writer = FixedWriter::new(&mut data[1..]);
                $(self.$field.encode(&mut writer);)*
                writer.finish();
                data
            }
        }
    };
}

fixed_instruction!(
    InitializeControllerV1,
    tag INITIALIZE_CONTROLLER_V1_TAG = 1,
    len INITIALIZE_CONTROLLER_V1_LEN,
    {
        cluster_domain: [u8; 32],
        initial_policy_version: u64,
        initial_council_version: u64,
        next_proposal_id: u64,
        target_nonce: u64,
        initial_gate_epoch: u64,
        policy_activation_slot: u64,
        routine_delay_slots: u64,
        major_delay_slots: u64,
        rollback_delay_slots: u64,
        terminal_delay_slots: u64,
        vote_review_slots: u64,
        proposal_expiry_slots: u64,
        expected_policy_hash: [u8; 32],
        expected_council_hash: [u8; 32],
        seat_terms: [CouncilSeatTermV1; 5],
    }
);

fixed_instruction!(
    CreateProposalV2,
    tag CREATE_PROPOSAL_V2_TAG = 2,
    len CREATE_PROPOSAL_V2_LEN,
    {
        proposal_class: ProposalClassV1,
        creation_gate_status: GateStatusV1,
        expected_proposal_id: u64,
        expected_target_nonce: u64,
        creation_slot: u64,
        expected_policy_version: u64,
        expected_policy_hash: [u8; 32],
        expected_creation_council_version: u64,
        expected_creation_council_hash: [u8; 32],
        expected_creation_gate_epoch: u64,
        expected_freeze_gate_epoch: u64,
        artifact_length: u64,
        artifact_sha256: [u8; 32],
        artifact_chunk_merkle_root: [u8; 32],
        source_commit_hash: [u8; 32],
        source_tree_hash: [u8; 32],
        build_input_inventory_hash: [u8; 32],
        reproducible_build_receipt_hash: [u8; 32],
        package_receipt_hash: [u8; 32],
        release_intent_hash: [u8; 32],
        expected_execution_pre_payload_hash: [u8; 32],
        expected_execution_pre_chunk_root: [u8; 32],
        current_raw_programdata_hash: [u8; 32],
        deployed_slot: u64,
        current_capacity: u64,
        extension_delta: u64,
        expected_post_capacity: u64,
        checkpoint_schema_id: [u8; 32],
        checkpoint_policy_hash: [u8; 32],
        primary_proposal: OptionalInstructionPubkeyV1,
        rollback_proposal: OptionalInstructionPubkeyV1,
        rollback_buffer: OptionalInstructionPubkeyV1,
        rollback_artifact_sha256: [u8; 32],
        rollback_artifact_chunk_root: [u8; 32],
        review_start_slot: u64,
        review_end_slot: u64,
        not_before_slot: u64,
        expiry_slot: u64,
        expected_proposal_digest: [u8; 32],
    }
);

fixed_instruction!(
    ApproveProposalV2,
    tag APPROVE_PROPOSAL_V2_TAG = 3,
    len APPROVE_PROPOSAL_V2_LEN,
    {
        expected: ProposalExpectationV2,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction!(
    FinalizeGovernanceV2,
    tag FINALIZE_GOVERNANCE_V2_TAG = 4,
    len FINALIZE_GOVERNANCE_V2_LEN,
    {
        expected: ProposalExpectationV2,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction!(
    QueueProposalV2,
    tag QUEUE_PROPOSAL_V2_TAG = 5,
    len QUEUE_PROPOSAL_V2_LEN,
    {
        expected: ProposalExpectationV2,
    }
);

fixed_instruction!(
    FreezeProposalV2,
    tag FREEZE_PROPOSAL_V2_TAG = 6,
    len FREEZE_PROPOSAL_V2_LEN,
    {
        expected: ProposalExpectationV2,
        expected_next_gate_epoch: u64,
    }
);

fixed_instruction!(
    CancelProposalV2,
    tag CANCEL_PROPOSAL_V2_TAG = 7,
    len CANCEL_PROPOSAL_V2_LEN,
    {
        expected: ProposalExpectationV2,
        expected_cancellation_approval_bitset: u8,
        expected_cancellation_approval_count: u8,
        cancellation_reason_code: u16,
    }
);

fixed_instruction!(
    ExpireProposalV2,
    tag EXPIRE_PROPOSAL_V2_TAG = 8,
    len EXPIRE_PROPOSAL_V2_LEN,
    {
        expected: ProposalExpectationV2,
    }
);

fixed_instruction_validated!(
    GuardianFreezeV1,
    tag GUARDIAN_FREEZE_V1_TAG = 9,
    len GUARDIAN_FREEZE_V1_LEN,
    validate validate_observation_shape,
    {
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        expected_next_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_program_owner: Pubkey,
        expected_program_executable: bool,
        expected_program_data_length: u64,
        expected_program_header_present: bool,
        expected_linked_programdata: OptionalInstructionPubkeyV1,
        expected_programdata_owner: Pubkey,
        expected_programdata_executable: bool,
        expected_programdata_data_length: u64,
        expected_programdata_header_present: bool,
        expected_programdata_slot: u64,
        expected_raw_hash_complete: bool,
        expected_raw_programdata_hash: [u8; 32],
        expected_capacity: u64,
        expected_programdata_authority: OptionalInstructionPubkeyV1,
        freeze_reason_code: u16,
        expected_observation_digest: [u8; 32],
    }
);

impl GuardianFreezeV1 {
    pub fn validate_observation_shape(&self) -> Result<(), ProgramError> {
        validate_instruction_program_observation_shape(
            self.expected_program_header_present,
            self.expected_program_data_length,
            self.expected_linked_programdata,
        )?;
        validate_instruction_programdata_observation_shape(
            self.expected_programdata_header_present,
            self.expected_programdata_data_length,
            self.expected_programdata_slot,
            self.expected_capacity,
            self.expected_programdata_authority,
        )?;
        validate_instruction_raw_programdata_hash_shape(
            self.expected_raw_hash_complete,
            &self.expected_raw_programdata_hash,
            self.expected_programdata_data_length,
        )
    }
}

fixed_instruction_validated!(
    CreateEmergencyResolutionV1,
    tag CREATE_EMERGENCY_RESOLUTION_V1_TAG = 10,
    len CREATE_EMERGENCY_RESOLUTION_V1_LEN,
    validate validate_observation_shape,
    {
        resolution_kind: EmergencyFreezeResolutionKindV1,
        creation_slot: u64,
        not_before_slot: u64,
        expiry_slot: u64,
        expected_policy_version: u64,
        expected_policy_hash: [u8; 32],
        expected_council_version: u64,
        expected_council_hash: [u8; 32],
        expected_gate_epoch: u64,
        expected_freeze_slot: u64,
        expected_freeze_reason_code: u16,
        expected_target_nonce: u64,
        expected_freeze_observation_digest: [u8; 32],
        observed_program_owner: Pubkey,
        observed_program_executable: bool,
        observed_program_data_length: u64,
        observed_program_header_present: bool,
        observed_linked_programdata: OptionalInstructionPubkeyV1,
        observed_programdata_owner: Pubkey,
        observed_programdata_executable: bool,
        observed_programdata_data_length: u64,
        observed_programdata_header_present: bool,
        observed_programdata_slot: u64,
        observed_raw_hash_complete: bool,
        observed_raw_programdata_hash: [u8; 32],
        observed_capacity: u64,
        observed_programdata_authority: OptionalInstructionPubkeyV1,
        expected_resolution_digest: [u8; 32],
    }
);

impl CreateEmergencyResolutionV1 {
    pub fn validate_observation_shape(&self) -> Result<(), ProgramError> {
        validate_instruction_program_observation_shape(
            self.observed_program_header_present,
            self.observed_program_data_length,
            self.observed_linked_programdata,
        )?;
        validate_instruction_programdata_observation_shape(
            self.observed_programdata_header_present,
            self.observed_programdata_data_length,
            self.observed_programdata_slot,
            self.observed_capacity,
            self.observed_programdata_authority,
        )?;
        validate_instruction_raw_programdata_hash_shape(
            self.observed_raw_hash_complete,
            &self.observed_raw_programdata_hash,
            self.observed_programdata_data_length,
        )
    }
}

fixed_instruction!(
    ApproveEmergencyResolutionV1,
    tag APPROVE_EMERGENCY_RESOLUTION_V1_TAG = 11,
    len APPROVE_EMERGENCY_RESOLUTION_V1_LEN,
    {
        expected: EmergencyResolutionExpectationV1,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction!(
    QueueEmergencyResolutionV1,
    tag QUEUE_EMERGENCY_RESOLUTION_V1_TAG = 12,
    len QUEUE_EMERGENCY_RESOLUTION_V1_LEN,
    {
        expected: EmergencyResolutionExpectationV1,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction_validated!(
    ExecuteEmergencyResolutionV1,
    tag EXECUTE_EMERGENCY_RESOLUTION_V1_TAG = 13,
    len EXECUTE_EMERGENCY_RESOLUTION_V1_LEN,
    validate validate_observation_shape,
    {
        expected: EmergencyResolutionExpectationV1,
        expected_freeze_observation_digest: [u8; 32],
        expected_checkpoint_digest: [u8; 32],
        expected_program_owner: Pubkey,
        expected_program_executable: bool,
        expected_program_data_length: u64,
        expected_program_header_present: bool,
        expected_linked_programdata: OptionalInstructionPubkeyV1,
        expected_programdata_owner: Pubkey,
        expected_programdata_executable: bool,
        expected_programdata_data_length: u64,
        expected_programdata_header_present: bool,
        expected_programdata_slot: u64,
        expected_raw_hash_complete: bool,
        expected_raw_programdata_hash: [u8; 32],
        expected_capacity: u64,
        expected_programdata_authority: OptionalInstructionPubkeyV1,
    }
);

impl ExecuteEmergencyResolutionV1 {
    pub fn validate_observation_shape(&self) -> Result<(), ProgramError> {
        validate_instruction_program_observation_shape(
            self.expected_program_header_present,
            self.expected_program_data_length,
            self.expected_linked_programdata,
        )?;
        validate_instruction_programdata_observation_shape(
            self.expected_programdata_header_present,
            self.expected_programdata_data_length,
            self.expected_programdata_slot,
            self.expected_capacity,
            self.expected_programdata_authority,
        )?;
        validate_instruction_raw_programdata_hash_shape(
            self.expected_raw_hash_complete,
            &self.expected_raw_programdata_hash,
            self.expected_programdata_data_length,
        )?;
        if !self.expected_raw_hash_complete {
            return Err(ProgramError::InvalidInstructionData);
        }
        if self.expected_program_owner != UPGRADEABLE_LOADER_ID
            || !self.expected_program_executable
            || !self.expected_program_header_present
            || self.expected_linked_programdata.value().is_none()
            || self.expected_programdata_owner != UPGRADEABLE_LOADER_ID
            || self.expected_programdata_executable
            || !self.expected_programdata_header_present
            || self.expected_programdata_slot == 0
            || self.expected_capacity == 0
            || self.expected_programdata_authority.value().is_none()
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

fixed_instruction!(
    ConvertEmergencyFreezeV2,
    tag CONVERT_EMERGENCY_FREEZE_V2_TAG = 14,
    len CONVERT_EMERGENCY_FREEZE_V2_LEN,
    {
        expected: ProposalExpectationV2,
        expected_next_gate_epoch: u64,
        expected_freeze_observation_digest: [u8; 32],
    }
);

fixed_instruction!(
    CreateCheckpointAttestationV1,
    tag CREATE_CHECKPOINT_ATTESTATION_V1_TAG = 15,
    len CREATE_CHECKPOINT_ATTESTATION_V1_LEN,
    {
        candidate: CheckpointCandidateV1,
        expected_council_version: u64,
        expected_council_hash: [u8; 32],
        seat_index: u8,
    }
);

fixed_instruction!(
    RecastCheckpointAttestationV1,
    tag RECAST_CHECKPOINT_ATTESTATION_V1_TAG = 16,
    len RECAST_CHECKPOINT_ATTESTATION_V1_LEN,
    {
        candidate: CheckpointCandidateV1,
        expected_council_version: u64,
        expected_council_hash: [u8; 32],
        seat_index: u8,
        expected_previous_attestation_digest: [u8; 32],
    }
);

fixed_instruction!(
    FinalizeCheckpointV1,
    tag FINALIZE_CHECKPOINT_V1_TAG = 17,
    len FINALIZE_CHECKPOINT_V1_LEN,
    {
        candidate: CheckpointCandidateV1,
        expected_council_version: u64,
        expected_council_hash: [u8; 32],
    }
);

fixed_instruction!(
    CreateCandidateCouncilSetV1,
    tag CREATE_CANDIDATE_COUNCIL_SET_V1_TAG = 18,
    len CREATE_CANDIDATE_COUNCIL_SET_V1_LEN,
    {
        expected_current_council_version: u64,
        expected_current_council_hash: [u8; 32],
        candidate_council_version: u64,
        activation_slot: u64,
        expected_target_nonce: u64,
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        expected_candidate_council_hash: [u8; 32],
        seat_terms: [CouncilSeatTermV1; 5],
    }
);

fixed_instruction!(
    CreateCouncilRotationV1,
    tag CREATE_COUNCIL_ROTATION_V1_TAG = 19,
    len CREATE_COUNCIL_ROTATION_V1_LEN,
    {
        creation_slot: u64,
        not_before_slot: u64,
        expiry_slot: u64,
        expected_current_council_version: u64,
        expected_current_council_hash: [u8; 32],
        expected_candidate_council_version: u64,
        expected_candidate_council_hash: [u8; 32],
        expected_gate_status: GateStatusV1,
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_rotation_digest: [u8; 32],
    }
);

fixed_instruction!(
    ApproveCouncilRotationV1,
    tag APPROVE_COUNCIL_ROTATION_V1_TAG = 20,
    len APPROVE_COUNCIL_ROTATION_V1_LEN,
    {
        expected: CouncilRotationExpectationV1,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction!(
    ActivateCouncilRotationV1,
    tag ACTIVATE_COUNCIL_ROTATION_V1_TAG = 21,
    len ACTIVATE_COUNCIL_ROTATION_V1_LEN,
    {
        expected: CouncilRotationExpectationV1,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction!(
    QueueCouncilRotationV1,
    tag QUEUE_COUNCIL_ROTATION_V1_TAG = 22,
    len QUEUE_COUNCIL_ROTATION_V1_LEN,
    {
        expected: CouncilRotationExpectationV1,
        expected_approval_bitset: u8,
        expected_approval_count: u8,
    }
);

fixed_instruction!(
    ExpireEmergencyResolutionV1,
    tag EXPIRE_EMERGENCY_RESOLUTION_V1_TAG = 23,
    len EXPIRE_EMERGENCY_RESOLUTION_V1_LEN,
    {
        expected: EmergencyResolutionExpectationV1,
    }
);

fixed_instruction!(
    CancelCouncilRotationV1,
    tag CANCEL_COUNCIL_ROTATION_V1_TAG = 24,
    len CANCEL_COUNCIL_ROTATION_V1_LEN,
    {
        expected: CouncilRotationExpectationV1,
        expected_cancellation_approval_bitset: u8,
        expected_cancellation_approval_count: u8,
        cancellation_reason_code: u16,
    }
);

fixed_instruction!(
    ExpireCouncilRotationV1,
    tag EXPIRE_COUNCIL_ROTATION_V1_TAG = 25,
    len EXPIRE_COUNCIL_ROTATION_V1_LEN,
    {
        expected: CouncilRotationExpectationV1,
    }
);

// Tag 26 remains intentionally closed: EmergencyFreezeResolutionV1 has no
// separate cancellation accumulator, so a safe council-cancellation codec
// requires V2. Tags 27-38 form the closed typed Loader-v3, verification,
// rollback-activation, and unfreeze surface. They never carry caller-selected
// CPI bytes, program IDs, or account vectors.
pub const CANCEL_EMERGENCY_RESOLUTION_V2_RESERVED_TAG: u8 = 26;

fixed_instruction!(
    AdoptBufferV1,
    tag ADOPT_BUFFER_V1_TAG = 27,
    len ADOPT_BUFFER_V1_LEN,
    {
        expected: ProposalExpectationV2,
    }
);

fixed_instruction!(
    VerifyBufferChunkV1,
    tag VERIFY_BUFFER_CHUNK_V1_TAG = 28,
    len VERIFY_BUFFER_CHUNK_V1_LEN,
    {
        expected: ProposalExpectationV2,
        chunk_index: u32,
        proof: FixedMerkleProofV1,
        expected_verification_status: BufferVerificationStatusV1,
        expected_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_chunk_count: u32,
    }
);

fixed_instruction!(
    FinalizeBufferVerificationV1,
    tag FINALIZE_BUFFER_VERIFICATION_V1_TAG = 29,
    len FINALIZE_BUFFER_VERIFICATION_V1_LEN,
    {
        expected: ProposalExpectationV2,
        expected_verification_status: BufferVerificationStatusV1,
        expected_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_chunk_count: u32,
    }
);

fixed_instruction!(
    ExtendTargetV1,
    tag EXTEND_TARGET_V1_TAG = 30,
    len EXTEND_TARGET_V1_LEN,
    {
        expected: ProposalExpectationV2,
        expected_prestate_checkpoint_digest: [u8; 32],
        expected_current_capacity: u64,
        expected_extension_delta: u64,
        expected_post_capacity: u64,
        envelope: EnvelopeExpectationV1,
    }
);

fixed_instruction!(
    ExecuteUpgradeV1,
    tag EXECUTE_UPGRADE_V1_TAG = 31,
    len EXECUTE_UPGRADE_V1_LEN,
    {
        expected: ProposalExpectationV2,
        expected_prestate_checkpoint_digest: [u8; 32],
        expected_current_raw_programdata_hash: [u8; 32],
        expected_sealed_buffer_header_hash: [u8; 32],
        expected_counterpart_proposal_digest: [u8; 32],
        expected_programdata_slot: u64,
        expected_capacity: u64,
        expected_verified_chunk_count: u32,
        expected_buffer_verification_status: BufferVerificationStatusV1,
        expected_counterpart_buffer_verification_status: BufferVerificationStatusV1,
        envelope: EnvelopeExpectationV1,
    }
);

fixed_instruction_validated!(
    VerifyProgramDataChunkV1,
    tag VERIFY_PROGRAMDATA_CHUNK_V1_TAG = 32,
    len VERIFY_PROGRAMDATA_CHUNK_V1_LEN,
    validate validate_phase_proof,
    {
        expected: ProposalExpectationV2,
        phase: ProgramDataChunkPhaseV1,
        chunk_index: u32,
        proof: FixedMerkleProofV1,
        expected_verification_status: ProgramDataVerificationStatusV1,
        expected_verified_payload_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_payload_chunk_count: u32,
        expected_verified_tail_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_tail_chunk_count: u32,
    }
);

impl VerifyProgramDataChunkV1 {
    pub fn validate_phase_proof(&self) -> Result<(), ProgramError> {
        self.proof.validate()?;
        if self.phase == ProgramDataChunkPhaseV1::ZeroTail && self.proof.proof_len != 0 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

fixed_instruction!(
    FinalizeProgramDataVerificationV1,
    tag FINALIZE_PROGRAMDATA_VERIFICATION_V1_TAG = 33,
    len FINALIZE_PROGRAMDATA_VERIFICATION_V1_LEN,
    {
        expected: ProposalExpectationV2,
        expected_verification_status: ProgramDataVerificationStatusV1,
        expected_verified_payload_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_payload_chunk_count: u32,
        expected_verified_tail_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_tail_chunk_count: u32,
        expected_deployed_slot: u64,
        expected_capacity: u64,
    }
);

fixed_instruction!(
    ApproveUnfreezeV1,
    tag APPROVE_UNFREEZE_V1_TAG = 34,
    len APPROVE_UNFREEZE_V1_LEN,
    {
        expected: UnfreezeExpectationV1,
    }
);

fixed_instruction_validated!(
    ExecuteUnfreezeV1,
    tag EXECUTE_UNFREEZE_V1_TAG = 35,
    len EXECUTE_UNFREEZE_V1_LEN,
    validate validate_linked_proposal,
    {
        expected: UnfreezeExpectationV1,
        linked_proposal: Pubkey,
        envelope: EnvelopeExpectationV1,
    }
);

impl ExecuteUnfreezeV1 {
    pub fn validate_linked_proposal(&self) -> Result<(), ProgramError> {
        if self.linked_proposal == Pubkey::default() {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

fixed_instruction!(
    CloseAbandonedBufferV1,
    tag CLOSE_ABANDONED_BUFFER_V1_TAG = 36,
    len CLOSE_ABANDONED_BUFFER_V1_LEN,
    {
        expected: ProposalExpectationV2,
        expected_verification_status: BufferVerificationStatusV1,
        expected_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_verified_chunk_count: u32,
        expected_buffer_verification_finalized_slot: u64,
    }
);

fixed_instruction!(
    ActivateRollbackV1,
    tag ACTIVATE_ROLLBACK_V1_TAG = 37,
    len ACTIVATE_ROLLBACK_V1_LEN,
    {
        expected_primary: ProposalExpectationV2,
        expected_rollback: ProposalExpectationV2,
        expected_failure_evidence_digest: [u8; 32],
        expected_primary_programdata_verification_status: ProgramDataVerificationStatusV1,
        expected_primary_programdata_verification_finalized_slot: u64,
        expected_rollback_buffer_verification_status: BufferVerificationStatusV1,
        expected_rollback_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
        expected_rollback_verified_chunk_count: u32,
    }
);

fixed_instruction_validated!(
    ObserveProgramDataFailureV1,
    tag OBSERVE_PROGRAMDATA_FAILURE_V1_TAG = 38,
    len OBSERVE_PROGRAMDATA_FAILURE_V1_LEN,
    validate validate_failure_shape,
    {
        expected: ProposalExpectationV2,
        expected_program_owner: Pubkey,
        expected_program_executable: bool,
        expected_program_data_length: u64,
        expected_program_header_present: bool,
        expected_linked_programdata: OptionalInstructionPubkeyV1,
        expected_programdata_owner: Pubkey,
        expected_programdata_executable: bool,
        expected_programdata_data_length: u64,
        expected_programdata_header_present: bool,
        expected_programdata_slot: u64,
        expected_raw_hash_complete: bool,
        expected_raw_programdata_hash: [u8; 32],
        expected_capacity: u64,
        expected_programdata_authority: OptionalInstructionPubkeyV1,
        mismatch_class: ProgramDataMismatchClassV1,
        failing_chunk_index: u32,
        expected_leaf_hash: [u8; 32],
        proof: FixedMerkleProofV1,
    }
);

impl ObserveProgramDataFailureV1 {
    pub fn validate_failure_shape(&self) -> Result<(), ProgramError> {
        validate_instruction_program_observation_shape(
            self.expected_program_header_present,
            self.expected_program_data_length,
            self.expected_linked_programdata,
        )?;
        validate_instruction_programdata_observation_shape(
            self.expected_programdata_header_present,
            self.expected_programdata_data_length,
            self.expected_programdata_slot,
            self.expected_capacity,
            self.expected_programdata_authority,
        )?;
        validate_instruction_raw_programdata_hash_shape(
            self.expected_raw_hash_complete,
            &self.expected_raw_programdata_hash,
            self.expected_programdata_data_length,
        )?;
        self.proof.validate()?;
        let leaf_failure = matches!(
            self.mismatch_class,
            ProgramDataMismatchClassV1::PayloadLeaf | ProgramDataMismatchClassV1::ZeroTail
        );
        if leaf_failure {
            if !self.expected_raw_hash_complete
                || self.failing_chunk_index == u32::MAX
                || self.expected_leaf_hash == [0; 32]
            {
                return Err(ProgramError::InvalidInstructionData);
            }
            if self.mismatch_class == ProgramDataMismatchClassV1::ZeroTail
                && self.proof.proof_len != 0
            {
                return Err(ProgramError::InvalidInstructionData);
            }
            // `expected_leaf_hash` is only a stale-plan guard for ZeroTail.
            // The processor must derive the canonical zero-chunk hash from the
            // actual byte range; the caller supplies no proof or trusted hash.
        } else if self.failing_chunk_index != u32::MAX
            || self.expected_leaf_hash != [0; 32]
            || self.proof.proof_len != 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpgradeControllerInstruction {
    RecordProposalApprovalV1(RecordProposalApprovalV1),
    InitializeControllerV1(InitializeControllerV1),
    CreateProposalV2(Box<CreateProposalV2>),
    ApproveProposalV2(ApproveProposalV2),
    FinalizeGovernanceV2(FinalizeGovernanceV2),
    QueueProposalV2(QueueProposalV2),
    FreezeProposalV2(FreezeProposalV2),
    CancelProposalV2(CancelProposalV2),
    ExpireProposalV2(ExpireProposalV2),
    GuardianFreezeV1(GuardianFreezeV1),
    CreateEmergencyResolutionV1(CreateEmergencyResolutionV1),
    ApproveEmergencyResolutionV1(ApproveEmergencyResolutionV1),
    QueueEmergencyResolutionV1(QueueEmergencyResolutionV1),
    ExecuteEmergencyResolutionV1(ExecuteEmergencyResolutionV1),
    ConvertEmergencyFreezeV2(ConvertEmergencyFreezeV2),
    CreateCheckpointAttestationV1(CreateCheckpointAttestationV1),
    RecastCheckpointAttestationV1(RecastCheckpointAttestationV1),
    FinalizeCheckpointV1(FinalizeCheckpointV1),
    CreateCandidateCouncilSetV1(CreateCandidateCouncilSetV1),
    CreateCouncilRotationV1(CreateCouncilRotationV1),
    ApproveCouncilRotationV1(ApproveCouncilRotationV1),
    ActivateCouncilRotationV1(ActivateCouncilRotationV1),
    QueueCouncilRotationV1(QueueCouncilRotationV1),
    ExpireEmergencyResolutionV1(ExpireEmergencyResolutionV1),
    CancelCouncilRotationV1(CancelCouncilRotationV1),
    ExpireCouncilRotationV1(ExpireCouncilRotationV1),
    AdoptBufferV1(AdoptBufferV1),
    VerifyBufferChunkV1(Box<VerifyBufferChunkV1>),
    FinalizeBufferVerificationV1(FinalizeBufferVerificationV1),
    ExtendTargetV1(ExtendTargetV1),
    ExecuteUpgradeV1(Box<ExecuteUpgradeV1>),
    VerifyProgramDataChunkV1(Box<VerifyProgramDataChunkV1>),
    FinalizeProgramDataVerificationV1(Box<FinalizeProgramDataVerificationV1>),
    ApproveUnfreezeV1(ApproveUnfreezeV1),
    ExecuteUnfreezeV1(Box<ExecuteUnfreezeV1>),
    CloseAbandonedBufferV1(CloseAbandonedBufferV1),
    ActivateRollbackV1(Box<ActivateRollbackV1>),
    ObserveProgramDataFailureV1(Box<ObserveProgramDataFailureV1>),
}

impl UpgradeControllerInstruction {
    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.is_empty() || data.len() > MAX_CONTROLLER_INSTRUCTION_DATA_LEN {
            return Err(ProgramError::InvalidInstructionData);
        }
        match data[0] {
            RECORD_PROPOSAL_APPROVAL_V1_TAG => {
                RecordProposalApprovalV1::unpack(data).map(Self::RecordProposalApprovalV1)
            }
            INITIALIZE_CONTROLLER_V1_TAG => {
                InitializeControllerV1::unpack(data).map(Self::InitializeControllerV1)
            }
            CREATE_PROPOSAL_V2_TAG => {
                CreateProposalV2::unpack(data).map(|value| Self::CreateProposalV2(Box::new(value)))
            }
            APPROVE_PROPOSAL_V2_TAG => ApproveProposalV2::unpack(data).map(Self::ApproveProposalV2),
            FINALIZE_GOVERNANCE_V2_TAG => {
                FinalizeGovernanceV2::unpack(data).map(Self::FinalizeGovernanceV2)
            }
            QUEUE_PROPOSAL_V2_TAG => QueueProposalV2::unpack(data).map(Self::QueueProposalV2),
            FREEZE_PROPOSAL_V2_TAG => FreezeProposalV2::unpack(data).map(Self::FreezeProposalV2),
            CANCEL_PROPOSAL_V2_TAG => CancelProposalV2::unpack(data).map(Self::CancelProposalV2),
            EXPIRE_PROPOSAL_V2_TAG => ExpireProposalV2::unpack(data).map(Self::ExpireProposalV2),
            GUARDIAN_FREEZE_V1_TAG => GuardianFreezeV1::unpack(data).map(Self::GuardianFreezeV1),
            CREATE_EMERGENCY_RESOLUTION_V1_TAG => {
                CreateEmergencyResolutionV1::unpack(data).map(Self::CreateEmergencyResolutionV1)
            }
            APPROVE_EMERGENCY_RESOLUTION_V1_TAG => {
                ApproveEmergencyResolutionV1::unpack(data).map(Self::ApproveEmergencyResolutionV1)
            }
            QUEUE_EMERGENCY_RESOLUTION_V1_TAG => {
                QueueEmergencyResolutionV1::unpack(data).map(Self::QueueEmergencyResolutionV1)
            }
            EXECUTE_EMERGENCY_RESOLUTION_V1_TAG => {
                ExecuteEmergencyResolutionV1::unpack(data).map(Self::ExecuteEmergencyResolutionV1)
            }
            CONVERT_EMERGENCY_FREEZE_V2_TAG => {
                ConvertEmergencyFreezeV2::unpack(data).map(Self::ConvertEmergencyFreezeV2)
            }
            CREATE_CHECKPOINT_ATTESTATION_V1_TAG => {
                CreateCheckpointAttestationV1::unpack(data).map(Self::CreateCheckpointAttestationV1)
            }
            RECAST_CHECKPOINT_ATTESTATION_V1_TAG => {
                RecastCheckpointAttestationV1::unpack(data).map(Self::RecastCheckpointAttestationV1)
            }
            FINALIZE_CHECKPOINT_V1_TAG => {
                FinalizeCheckpointV1::unpack(data).map(Self::FinalizeCheckpointV1)
            }
            CREATE_CANDIDATE_COUNCIL_SET_V1_TAG => {
                CreateCandidateCouncilSetV1::unpack(data).map(Self::CreateCandidateCouncilSetV1)
            }
            CREATE_COUNCIL_ROTATION_V1_TAG => {
                CreateCouncilRotationV1::unpack(data).map(Self::CreateCouncilRotationV1)
            }
            APPROVE_COUNCIL_ROTATION_V1_TAG => {
                ApproveCouncilRotationV1::unpack(data).map(Self::ApproveCouncilRotationV1)
            }
            ACTIVATE_COUNCIL_ROTATION_V1_TAG => {
                ActivateCouncilRotationV1::unpack(data).map(Self::ActivateCouncilRotationV1)
            }
            QUEUE_COUNCIL_ROTATION_V1_TAG => {
                QueueCouncilRotationV1::unpack(data).map(Self::QueueCouncilRotationV1)
            }
            EXPIRE_EMERGENCY_RESOLUTION_V1_TAG => {
                ExpireEmergencyResolutionV1::unpack(data).map(Self::ExpireEmergencyResolutionV1)
            }
            CANCEL_COUNCIL_ROTATION_V1_TAG => {
                CancelCouncilRotationV1::unpack(data).map(Self::CancelCouncilRotationV1)
            }
            EXPIRE_COUNCIL_ROTATION_V1_TAG => {
                ExpireCouncilRotationV1::unpack(data).map(Self::ExpireCouncilRotationV1)
            }
            ADOPT_BUFFER_V1_TAG => AdoptBufferV1::unpack(data).map(Self::AdoptBufferV1),
            VERIFY_BUFFER_CHUNK_V1_TAG => VerifyBufferChunkV1::unpack(data)
                .map(|value| Self::VerifyBufferChunkV1(Box::new(value))),
            FINALIZE_BUFFER_VERIFICATION_V1_TAG => {
                FinalizeBufferVerificationV1::unpack(data).map(Self::FinalizeBufferVerificationV1)
            }
            EXTEND_TARGET_V1_TAG => ExtendTargetV1::unpack(data).map(Self::ExtendTargetV1),
            EXECUTE_UPGRADE_V1_TAG => {
                ExecuteUpgradeV1::unpack(data).map(|value| Self::ExecuteUpgradeV1(Box::new(value)))
            }
            VERIFY_PROGRAMDATA_CHUNK_V1_TAG => {
                let value = VerifyProgramDataChunkV1::unpack(data)?;
                value.validate_phase_proof()?;
                Ok(Self::VerifyProgramDataChunkV1(Box::new(value)))
            }
            FINALIZE_PROGRAMDATA_VERIFICATION_V1_TAG => {
                FinalizeProgramDataVerificationV1::unpack(data)
                    .map(|value| Self::FinalizeProgramDataVerificationV1(Box::new(value)))
            }
            APPROVE_UNFREEZE_V1_TAG => ApproveUnfreezeV1::unpack(data).map(Self::ApproveUnfreezeV1),
            EXECUTE_UNFREEZE_V1_TAG => ExecuteUnfreezeV1::unpack(data)
                .map(|value| Self::ExecuteUnfreezeV1(Box::new(value))),
            CLOSE_ABANDONED_BUFFER_V1_TAG => {
                CloseAbandonedBufferV1::unpack(data).map(Self::CloseAbandonedBufferV1)
            }
            ACTIVATE_ROLLBACK_V1_TAG => ActivateRollbackV1::unpack(data)
                .map(|value| Self::ActivateRollbackV1(Box::new(value))),
            OBSERVE_PROGRAMDATA_FAILURE_V1_TAG => {
                let value = ObserveProgramDataFailureV1::unpack(data)?;
                value.validate_failure_shape()?;
                Ok(Self::ObserveProgramDataFailureV1(Box::new(value)))
            }
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

macro_rules! account_meta {
    ($key:expr, readonly) => {
        AccountMeta::new_readonly($key, false)
    };
    ($key:expr, writable) => {
        AccountMeta::new($key, false)
    };
    ($key:expr, signer_readonly) => {
        AccountMeta::new_readonly($key, true)
    };
    ($key:expr, signer_writable) => {
        AccountMeta::new($key, true)
    };
}

macro_rules! fixed_builder {
    (
        $accounts:ident { $($field:ident: $mode:ident),* $(,)? },
        $function:ident,
        $instruction:ty
    ) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $accounts {
            $(pub $field: Pubkey,)*
        }

        pub fn $function(
            controller_program: Pubkey,
            accounts: $accounts,
            instruction: $instruction,
        ) -> Instruction {
            Instruction {
                program_id: controller_program,
                accounts: vec![
                    $(account_meta!(accounts.$field, $mode),)*
                ],
                data: instruction.pack().to_vec(),
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitializeControllerV1Accounts {
    pub payer: Pubkey,
    pub initializer: Pubkey,
    pub controller_program: Pubkey,
    pub controller_programdata: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub upgradeable_loader: Pubkey,
    pub controller_config: Pubkey,
    pub authority_pda: Pubkey,
    pub protocol_gate: Pubkey,
    pub policy: Pubkey,
    pub council: Pubkey,
    pub canonical_spill_treasury: Pubkey,
    pub guardian: Pubkey,
    pub seat_authorities: [Pubkey; 5],
    pub system_program: Pubkey,
}

/// Accounts, in order: payer (writable signer), initializer (signer), controller
/// Program, controller ProgramData, target Program, target ProgramData, Loader,
/// config (writable), authority PDA, gate/policy/council (writable), spill,
/// guardian, five read-only seat authorities, and System Program.
pub fn initialize_controller_v1_instruction(
    controller_program_id: Pubkey,
    accounts: InitializeControllerV1Accounts,
    instruction: InitializeControllerV1,
) -> Instruction {
    let mut metas = vec![
        account_meta!(accounts.payer, signer_writable),
        account_meta!(accounts.initializer, signer_readonly),
        account_meta!(accounts.controller_program, readonly),
        account_meta!(accounts.controller_programdata, readonly),
        account_meta!(accounts.target_program, readonly),
        account_meta!(accounts.target_programdata, readonly),
        account_meta!(accounts.upgradeable_loader, readonly),
        account_meta!(accounts.controller_config, writable),
        account_meta!(accounts.authority_pda, readonly),
        account_meta!(accounts.protocol_gate, writable),
        account_meta!(accounts.policy, writable),
        account_meta!(accounts.council, writable),
        account_meta!(accounts.canonical_spill_treasury, readonly),
        account_meta!(accounts.guardian, readonly),
    ];
    metas.extend(
        accounts
            .seat_authorities
            .into_iter()
            .map(|authority| account_meta!(authority, readonly)),
    );
    metas.push(account_meta!(accounts.system_program, readonly));
    Instruction {
        program_id: controller_program_id,
        accounts: metas,
        data: instruction.pack().to_vec(),
    }
}

fixed_builder!(
    CreateProposalV2Accounts {
        payer: signer_writable,
        creator_seat_authority: signer_readonly,
        controller_config: writable,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        canonical_spill_treasury: readonly,
        buffer: readonly,
        buffer_uploader_authority: readonly,
        proposal: writable,
        system_program: readonly,
    },
    create_proposal_v2_instruction,
    CreateProposalV2
);

fixed_builder!(
    ApproveProposalV2Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        proposal: writable,
        seat_authority: signer_readonly,
    },
    approve_proposal_v2_instruction,
    ApproveProposalV2
);

fixed_builder!(
    FinalizeGovernanceV2Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        proposal: writable,
    },
    finalize_governance_v2_instruction,
    FinalizeGovernanceV2
);

fixed_builder!(
    QueueProposalV2Accounts {
        controller_config: readonly,
        policy: readonly,
        protocol_gate: readonly,
        proposal: writable,
    },
    queue_proposal_v2_instruction,
    QueueProposalV2
);

fixed_builder!(
    FreezeProposalV2Accounts {
        controller_config: writable,
        policy: readonly,
        council: readonly,
        protocol_gate: writable,
        proposal: writable,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        rollback_proposal: readonly,
        rollback_buffer_verification: readonly,
        rollback_buffer: readonly,
    },
    freeze_proposal_v2_instruction,
    FreezeProposalV2
);

fixed_builder!(
    CancelProposalV2Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        proposal: writable,
        seat_authority: signer_readonly,
    },
    cancel_proposal_v2_instruction,
    CancelProposalV2
);

fixed_builder!(
    ExpireProposalV2Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
    },
    expire_proposal_v2_instruction,
    ExpireProposalV2
);

fixed_builder!(
    GuardianFreezeV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: writable,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        guardian: signer_readonly,
        emergency_freeze_observation: writable,
        system_program: readonly,
    },
    guardian_freeze_v1_instruction,
    GuardianFreezeV1
);

fixed_builder!(
    CreateEmergencyResolutionV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        emergency_freeze_observation: readonly,
        emergency_resolution: writable,
        system_program: readonly,
    },
    create_emergency_resolution_v1_instruction,
    CreateEmergencyResolutionV1
);

fixed_builder!(
    ApproveEmergencyResolutionV1Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        emergency_resolution: writable,
        seat_authority: signer_readonly,
    },
    approve_emergency_resolution_v1_instruction,
    ApproveEmergencyResolutionV1
);

fixed_builder!(
    QueueEmergencyResolutionV1Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        emergency_resolution: writable,
    },
    queue_emergency_resolution_v1_instruction,
    QueueEmergencyResolutionV1
);

fixed_builder!(
    ExecuteEmergencyResolutionV1Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: writable,
        emergency_resolution: writable,
        emergency_freeze_observation: readonly,
        emergency_checkpoint: readonly,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        instructions_sysvar: readonly,
    },
    execute_emergency_resolution_v1_instruction,
    ExecuteEmergencyResolutionV1
);

fixed_builder!(
    ConvertEmergencyFreezeV2Accounts {
        controller_config: writable,
        policy: readonly,
        council: readonly,
        protocol_gate: writable,
        proposal: writable,
        emergency_freeze_observation: readonly,
        target_program: readonly,
        target_programdata: readonly,
        upgradeable_loader: readonly,
        authority_pda: readonly,
        rollback_proposal: readonly,
        rollback_buffer_verification: readonly,
        rollback_buffer: readonly,
    },
    convert_emergency_freeze_v2_instruction,
    ConvertEmergencyFreezeV2
);

fixed_builder!(
    CreateCheckpointAttestationV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        subject: readonly,
        checkpoint: readonly,
        checkpoint_attestation: writable,
        seat_authority: signer_readonly,
        system_program: readonly,
    },
    create_checkpoint_attestation_v1_instruction,
    CreateCheckpointAttestationV1
);

fixed_builder!(
    RecastCheckpointAttestationV1Accounts {
        controller_config: readonly,
        policy: readonly,
        council: readonly,
        protocol_gate: readonly,
        subject: readonly,
        checkpoint: readonly,
        checkpoint_attestation: writable,
        seat_authority: signer_readonly,
    },
    recast_checkpoint_attestation_v1_instruction,
    RecastCheckpointAttestationV1
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalizeCheckpointV1Accounts {
    pub payer: Pubkey,
    pub controller_config: Pubkey,
    pub policy: Pubkey,
    pub council: Pubkey,
    pub protocol_gate: Pubkey,
    pub subject: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub phase_evidence: Pubkey,
    /// Required for Poststate and for an EmergencyRollback Prestate. It is the
    /// canonical accepted primary Prestate checkpoint in both cases. Ordinary
    /// Prestate and Emergency checkpoints omit it.
    pub baseline_checkpoint: Option<Pubkey>,
    pub checkpoint: Pubkey,
    pub checkpoint_attestations: [Pubkey; 3],
    pub system_program: Pubkey,
}

/// The proposal subject is writable only for a Poststate checkpoint, whose
/// finalization performs the separate `ProgramDataVerified -> PoststateAccepted`
/// transition. Prestate and emergency subjects remain read-only.
pub fn finalize_checkpoint_v1_instruction(
    controller_program: Pubkey,
    accounts: FinalizeCheckpointV1Accounts,
    instruction: FinalizeCheckpointV1,
) -> Instruction {
    let poststate = instruction.candidate.phase == StateCheckpointPhaseV1::Poststate;
    let subject = if poststate {
        account_meta!(accounts.subject, writable)
    } else {
        account_meta!(accounts.subject, readonly)
    };
    let prestate = instruction.candidate.phase == StateCheckpointPhaseV1::Prestate;
    let emergency = instruction.candidate.phase == StateCheckpointPhaseV1::Emergency;
    assert!(!emergency || accounts.baseline_checkpoint.is_none());
    assert!(!poststate || accounts.baseline_checkpoint.is_some());
    assert!(poststate || prestate || accounts.baseline_checkpoint.is_none());
    let mut metas = vec![
        account_meta!(accounts.payer, signer_writable),
        account_meta!(accounts.controller_config, readonly),
        account_meta!(accounts.policy, readonly),
        account_meta!(accounts.council, readonly),
        account_meta!(accounts.protocol_gate, readonly),
        subject,
        account_meta!(accounts.target_program, readonly),
        account_meta!(accounts.target_programdata, readonly),
        account_meta!(accounts.phase_evidence, readonly),
    ];
    if let Some(baseline_checkpoint) = accounts.baseline_checkpoint {
        metas.push(account_meta!(baseline_checkpoint, readonly));
    }
    metas.extend([
        account_meta!(accounts.checkpoint, writable),
        account_meta!(accounts.checkpoint_attestations[0], readonly),
        account_meta!(accounts.checkpoint_attestations[1], readonly),
        account_meta!(accounts.checkpoint_attestations[2], readonly),
        account_meta!(accounts.system_program, readonly),
    ]);
    Instruction {
        program_id: controller_program,
        accounts: metas,
        data: instruction.pack().to_vec(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateCandidateCouncilSetV1Accounts {
    pub payer: Pubkey,
    pub creator_seat_authority: Pubkey,
    pub controller_config: Pubkey,
    pub policy: Pubkey,
    pub current_council: Pubkey,
    pub protocol_gate: Pubkey,
    pub candidate_council: Pubkey,
    pub candidate_seat_authorities: [Pubkey; 5],
    pub system_program: Pubkey,
}

/// The payer and active creator seat are intentionally distinct roles. Candidate
/// seat authorities are read-only and need not sign, preserving PDA/multisig seats.
pub fn create_candidate_council_set_v1_instruction(
    controller_program: Pubkey,
    accounts: CreateCandidateCouncilSetV1Accounts,
    instruction: CreateCandidateCouncilSetV1,
) -> Instruction {
    let mut metas = vec![
        account_meta!(accounts.payer, signer_writable),
        account_meta!(accounts.creator_seat_authority, signer_readonly),
        account_meta!(accounts.controller_config, readonly),
        account_meta!(accounts.policy, readonly),
        account_meta!(accounts.current_council, readonly),
        account_meta!(accounts.protocol_gate, readonly),
        account_meta!(accounts.candidate_council, writable),
    ];
    metas.extend(
        accounts
            .candidate_seat_authorities
            .into_iter()
            .map(|authority| account_meta!(authority, readonly)),
    );
    metas.push(account_meta!(accounts.system_program, readonly));
    Instruction {
        program_id: controller_program,
        accounts: metas,
        data: instruction.pack().to_vec(),
    }
}

fixed_builder!(
    CreateCouncilRotationV1Accounts {
        payer: signer_writable,
        creator_seat_authority: signer_readonly,
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        candidate_council: readonly,
        protocol_gate: readonly,
        rotation: writable,
        system_program: readonly,
    },
    create_council_rotation_v1_instruction,
    CreateCouncilRotationV1
);

fixed_builder!(
    ApproveCouncilRotationV1Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        candidate_council: readonly,
        protocol_gate: readonly,
        rotation: writable,
        seat_authority: signer_readonly,
    },
    approve_council_rotation_v1_instruction,
    ApproveCouncilRotationV1
);

fixed_builder!(
    ActivateCouncilRotationV1Accounts {
        controller_config: writable,
        policy: readonly,
        current_council: readonly,
        candidate_council: readonly,
        protocol_gate: readonly,
        rotation: writable,
    },
    activate_council_rotation_v1_instruction,
    ActivateCouncilRotationV1
);

fixed_builder!(
    QueueCouncilRotationV1Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        candidate_council: readonly,
        protocol_gate: readonly,
        rotation: writable,
    },
    queue_council_rotation_v1_instruction,
    QueueCouncilRotationV1
);

fixed_builder!(
    ExpireEmergencyResolutionV1Accounts {
        controller_config: readonly,
        policy: readonly,
        protocol_gate: readonly,
        emergency_resolution: writable,
    },
    expire_emergency_resolution_v1_instruction,
    ExpireEmergencyResolutionV1
);

fixed_builder!(
    CancelCouncilRotationV1Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        candidate_council: readonly,
        protocol_gate: readonly,
        rotation: writable,
        seat_authority: signer_readonly,
    },
    cancel_council_rotation_v1_instruction,
    CancelCouncilRotationV1
);

fixed_builder!(
    ExpireCouncilRotationV1Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        rotation: writable,
    },
    expire_council_rotation_v1_instruction,
    ExpireCouncilRotationV1
);

fixed_builder!(
    AdoptBufferV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        buffer: writable,
        uploader_authority: signer_readonly,
        authority_pda: readonly,
        buffer_verification: writable,
        upgradeable_loader: readonly,
        system_program: readonly,
    },
    adopt_buffer_v1_instruction,
    AdoptBufferV1
);

fixed_builder!(
    VerifyBufferChunkV1Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: readonly,
        buffer: readonly,
        buffer_verification: writable,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    verify_buffer_chunk_v1_instruction,
    VerifyBufferChunkV1
);

fixed_builder!(
    FinalizeBufferVerificationV1Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        buffer: readonly,
        buffer_verification: writable,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    finalize_buffer_verification_v1_instruction,
    FinalizeBufferVerificationV1
);

fixed_builder!(
    ExtendTargetV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        prestate_checkpoint: readonly,
        target_programdata: writable,
        target_program: writable,
        authority_pda: writable,
        upgradeable_loader: readonly,
        system_program: readonly,
        rent_sysvar: readonly,
        instructions_sysvar: readonly,
    },
    extend_target_v1_instruction,
    ExtendTargetV1
);

fixed_builder!(
    ExecuteUpgradeV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        policy: readonly,
        protocol_gate: readonly,
        proposal: writable,
        counterpart_proposal: readonly,
        counterpart_buffer_verification: readonly,
        prestate_checkpoint: readonly,
        buffer_verification: writable,
        programdata_verification: writable,
        target_programdata: writable,
        target_program: writable,
        buffer: writable,
        canonical_spill_treasury: writable,
        rent_sysvar: readonly,
        clock_sysvar: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        system_program: readonly,
        instructions_sysvar: readonly,
    },
    execute_upgrade_v1_instruction,
    ExecuteUpgradeV1
);

fixed_builder!(
    VerifyProgramDataChunkV1Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        programdata_verification: writable,
    },
    verify_programdata_chunk_v1_instruction,
    VerifyProgramDataChunkV1
);

fixed_builder!(
    FinalizeProgramDataVerificationV1Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        programdata_verification: writable,
    },
    finalize_programdata_verification_v1_instruction,
    FinalizeProgramDataVerificationV1
);

fixed_builder!(
    ApproveUnfreezeV1Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: readonly,
        proposal: writable,
        poststate_checkpoint: readonly,
        programdata_verification: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        seat_authority: signer_readonly,
    },
    approve_unfreeze_v1_instruction,
    ApproveUnfreezeV1
);

fixed_builder!(
    ExecuteUnfreezeV1Accounts {
        controller_config: readonly,
        policy: readonly,
        current_council: readonly,
        protocol_gate: writable,
        proposal: writable,
        linked_proposal: writable,
        poststate_checkpoint: readonly,
        programdata_verification: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        instructions_sysvar: readonly,
    },
    execute_unfreeze_v1_instruction,
    ExecuteUnfreezeV1
);

fixed_builder!(
    CloseAbandonedBufferV1Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: readonly,
        buffer_verification: writable,
        buffer: writable,
        canonical_spill_treasury: writable,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    close_abandoned_buffer_v1_instruction,
    CloseAbandonedBufferV1
);

fixed_builder!(
    ActivateRollbackV1Accounts {
        controller_config: readonly,
        policy: readonly,
        protocol_gate: writable,
        primary_proposal: readonly,
        rollback_proposal: writable,
        rollback_buffer_verification: readonly,
        primary_programdata_verification: readonly,
        failure_evidence: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    activate_rollback_v1_instruction,
    ActivateRollbackV1
);

fixed_builder!(
    ObserveProgramDataFailureV1Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        primary_proposal: readonly,
        programdata_verification: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        failure_observation: writable,
        system_program: readonly,
    },
    observe_programdata_failure_v1_instruction,
    ObserveProgramDataFailureV1
);

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::hash::hash;

    fn key(value: u8) -> Pubkey {
        Pubkey::new_from_array([value; 32])
    }

    const fn bytes(value: u8) -> [u8; 32] {
        [value; 32]
    }

    fn seat_terms() -> [CouncilSeatTermV1; 5] {
        [
            CouncilSeatTermV1 {
                term_start_slot: 10,
                term_end_slot: 110,
            },
            CouncilSeatTermV1 {
                term_start_slot: 11,
                term_end_slot: 111,
            },
            CouncilSeatTermV1 {
                term_start_slot: 12,
                term_end_slot: 112,
            },
            CouncilSeatTermV1 {
                term_start_slot: 13,
                term_end_slot: 113,
            },
            CouncilSeatTermV1 {
                term_start_slot: 14,
                term_end_slot: 114,
            },
        ]
    }

    fn initialize() -> InitializeControllerV1 {
        InitializeControllerV1 {
            cluster_domain: bytes(1),
            initial_policy_version: 2,
            initial_council_version: 3,
            next_proposal_id: 4,
            target_nonce: 5,
            initial_gate_epoch: 6,
            policy_activation_slot: 7,
            routine_delay_slots: 8,
            major_delay_slots: 9,
            rollback_delay_slots: 10,
            terminal_delay_slots: 11,
            vote_review_slots: 12,
            proposal_expiry_slots: 13,
            expected_policy_hash: bytes(2),
            expected_council_hash: bytes(3),
            seat_terms: seat_terms(),
        }
    }

    fn create_proposal() -> CreateProposalV2 {
        CreateProposalV2 {
            proposal_class: ProposalClassV1::RoutineUpgrade,
            creation_gate_status: GateStatusV1::Active,
            expected_proposal_id: 1,
            expected_target_nonce: 2,
            creation_slot: 3,
            expected_policy_version: 4,
            expected_policy_hash: bytes(4),
            expected_creation_council_version: 5,
            expected_creation_council_hash: bytes(5),
            expected_creation_gate_epoch: 6,
            expected_freeze_gate_epoch: 7,
            artifact_length: 8,
            artifact_sha256: bytes(8),
            artifact_chunk_merkle_root: bytes(9),
            source_commit_hash: bytes(10),
            source_tree_hash: bytes(11),
            build_input_inventory_hash: bytes(12),
            reproducible_build_receipt_hash: bytes(13),
            package_receipt_hash: bytes(14),
            release_intent_hash: bytes(15),
            expected_execution_pre_payload_hash: bytes(16),
            expected_execution_pre_chunk_root: bytes(17),
            current_raw_programdata_hash: bytes(18),
            deployed_slot: 19,
            current_capacity: 20,
            extension_delta: 21,
            expected_post_capacity: 41,
            checkpoint_schema_id: bytes(22),
            checkpoint_policy_hash: bytes(23),
            primary_proposal: OptionalInstructionPubkeyV1::none(),
            rollback_proposal: OptionalInstructionPubkeyV1::some(key(24)).unwrap(),
            rollback_buffer: OptionalInstructionPubkeyV1::some(key(25)).unwrap(),
            rollback_artifact_sha256: bytes(26),
            rollback_artifact_chunk_root: bytes(27),
            review_start_slot: 28,
            review_end_slot: 29,
            not_before_slot: 30,
            expiry_slot: 31,
            expected_proposal_digest: bytes(32),
        }
    }

    fn proposal_expectation() -> ProposalExpectationV2 {
        ProposalExpectationV2 {
            expected_proposal_digest: bytes(33),
            expected_policy_version: 34,
            expected_policy_hash: bytes(35),
            expected_council_version: 36,
            expected_council_hash: bytes(37),
            expected_gate_status: GateStatusV1::FrozenForUpgrade,
            expected_gate_epoch: 38,
            expected_target_nonce: 39,
            expected_state: ProposalStateV2::Timelocked,
            expected_review_start_slot: 40,
            expected_review_end_slot: 41,
            expected_not_before_slot: 42,
            expected_expiry_slot: 43,
        }
    }

    fn guardian_freeze() -> GuardianFreezeV1 {
        GuardianFreezeV1 {
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: 44,
            expected_next_gate_epoch: 45,
            expected_target_nonce: 46,
            expected_program_owner: key(43),
            expected_program_executable: true,
            expected_program_data_length: 36,
            expected_program_header_present: true,
            expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(44)).unwrap(),
            expected_programdata_owner: key(47),
            expected_programdata_executable: false,
            expected_programdata_data_length: 4_141,
            expected_programdata_header_present: true,
            expected_programdata_slot: 47,
            expected_raw_hash_complete: true,
            expected_raw_programdata_hash: bytes(48),
            expected_capacity: 4_096,
            expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(50)).unwrap(),
            freeze_reason_code: 51,
            expected_observation_digest: bytes(52),
        }
    }

    fn create_emergency_resolution() -> CreateEmergencyResolutionV1 {
        CreateEmergencyResolutionV1 {
            resolution_kind: EmergencyFreezeResolutionKindV1::ResumeWithoutUpgrade,
            creation_slot: 51,
            not_before_slot: 52,
            expiry_slot: 53,
            expected_policy_version: 54,
            expected_policy_hash: bytes(55),
            expected_council_version: 56,
            expected_council_hash: bytes(57),
            expected_gate_epoch: 58,
            expected_freeze_slot: 59,
            expected_freeze_reason_code: 60,
            expected_target_nonce: 61,
            expected_freeze_observation_digest: bytes(62),
            observed_program_owner: key(58),
            observed_program_executable: true,
            observed_program_data_length: 36,
            observed_program_header_present: true,
            observed_linked_programdata: OptionalInstructionPubkeyV1::some(key(59)).unwrap(),
            observed_programdata_owner: key(63),
            observed_programdata_executable: false,
            observed_programdata_data_length: 4_141,
            observed_programdata_header_present: true,
            observed_programdata_slot: 63,
            observed_raw_hash_complete: true,
            observed_raw_programdata_hash: bytes(64),
            observed_capacity: 4_096,
            observed_programdata_authority: OptionalInstructionPubkeyV1::some(key(66)).unwrap(),
            expected_resolution_digest: bytes(67),
        }
    }

    fn emergency_expectation() -> EmergencyResolutionExpectationV1 {
        EmergencyResolutionExpectationV1 {
            expected_resolution_digest: bytes(67),
            expected_policy_version: 68,
            expected_policy_hash: bytes(69),
            expected_council_version: 70,
            expected_council_hash: bytes(71),
            expected_gate_status: GateStatusV1::EmergencyFrozen,
            expected_gate_epoch: 72,
            expected_freeze_slot: 73,
            expected_freeze_reason_code: 74,
            expected_target_nonce: 75,
            expected_state: EmergencyFreezeResolutionStateV1::Timelocked,
            expected_not_before_slot: 76,
            expected_expiry_slot: 77,
        }
    }

    fn execute_emergency_resolution() -> ExecuteEmergencyResolutionV1 {
        ExecuteEmergencyResolutionV1 {
            expected: emergency_expectation(),
            expected_freeze_observation_digest: bytes(127),
            expected_checkpoint_digest: bytes(128),
            expected_program_owner: UPGRADEABLE_LOADER_ID,
            expected_program_executable: true,
            expected_program_data_length: LOADER_V3_PROGRAM_ACCOUNT_LEN_V1,
            expected_program_header_present: true,
            expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(126)).unwrap(),
            expected_programdata_owner: UPGRADEABLE_LOADER_ID,
            expected_programdata_executable: false,
            expected_programdata_data_length: 4_141,
            expected_programdata_header_present: true,
            expected_programdata_slot: 129,
            expected_raw_hash_complete: true,
            expected_raw_programdata_hash: bytes(130),
            expected_capacity: 4_096,
            expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(132)).unwrap(),
        }
    }

    fn checkpoint_candidate(phase: StateCheckpointPhaseV1) -> CheckpointCandidateV1 {
        CheckpointCandidateV1 {
            phase,
            expected_subject_state: match phase {
                StateCheckpointPhaseV1::Prestate => CheckpointSubjectStateV1::ProposalFrozen,
                StateCheckpointPhaseV1::Poststate => {
                    CheckpointSubjectStateV1::ProposalProgramDataVerified
                }
                StateCheckpointPhaseV1::Emergency => {
                    CheckpointSubjectStateV1::EmergencyResolutionTimelocked
                }
            },
            expected_subject_digest: bytes(78),
            expected_gate_status: if phase == StateCheckpointPhaseV1::Emergency {
                GateStatusV1::EmergencyFrozen
            } else {
                GateStatusV1::FrozenForUpgrade
            },
            expected_gate_epoch: 79,
            finalized_observation_slot: 80,
            target_programdata_slot: 81,
            target_payload_commitment: bytes(82),
            target_raw_programdata_commitment: bytes(83),
            target_capacity: 84,
            program_owned_state_root: bytes(85),
            program_owned_state_count: 86,
            logical_compressed_state_root: bytes(87),
            logical_compressed_state_count: 88,
            semantic_custody_accounting_root: bytes(89),
            hard_combined_root: bytes(90),
            external_metadata_observation_root: bytes(91),
            external_raw_balance_observation_root: bytes(92),
            schema_identifier: bytes(93),
            admitted_positive_donation_root: bytes(94),
            admitted_positive_donation_count: 95,
            forbidden_drift_count: 0,
            expected_checkpoint_digest: bytes(96),
        }
    }

    fn create_candidate_council() -> CreateCandidateCouncilSetV1 {
        CreateCandidateCouncilSetV1 {
            expected_current_council_version: 102,
            expected_current_council_hash: bytes(103),
            candidate_council_version: 103,
            activation_slot: 104,
            expected_target_nonce: 105,
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: 106,
            expected_candidate_council_hash: bytes(107),
            seat_terms: seat_terms(),
        }
    }

    fn create_council_rotation() -> CreateCouncilRotationV1 {
        CreateCouncilRotationV1 {
            creation_slot: 108,
            not_before_slot: 109,
            expiry_slot: 110,
            expected_current_council_version: 111,
            expected_current_council_hash: bytes(112),
            expected_candidate_council_version: 112,
            expected_candidate_council_hash: bytes(113),
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: 114,
            expected_target_nonce: 115,
            expected_rotation_digest: bytes(116),
        }
    }

    fn rotation_expectation() -> CouncilRotationExpectationV1 {
        CouncilRotationExpectationV1 {
            expected_rotation_digest: bytes(117),
            expected_current_council_version: 118,
            expected_current_council_hash: bytes(119),
            expected_candidate_council_version: 119,
            expected_candidate_council_hash: bytes(120),
            expected_gate_status: GateStatusV1::Active,
            expected_gate_epoch: 121,
            expected_target_nonce: 122,
            expected_state: CouncilRotationStateV1::Timelocked,
            expected_not_before_slot: 123,
            expected_expiry_slot: 124,
        }
    }

    fn merkle_proof() -> FixedMerkleProofV1 {
        let mut nodes = [[0; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1];
        nodes[0] = bytes(121);
        nodes[1] = bytes(122);
        FixedMerkleProofV1 {
            proof_len: 2,
            nodes,
        }
    }

    fn envelope() -> EnvelopeExpectationV1 {
        EnvelopeExpectationV1 {
            compute_unit_limit: 1_200_000,
            compute_unit_price_micro_lamports: 17,
            durable_nonce_account: OptionalInstructionPubkeyV1::some(key(123)).unwrap(),
            durable_nonce_authority: OptionalInstructionPubkeyV1::some(key(124)).unwrap(),
        }
    }

    fn unfreeze_expectation() -> UnfreezeExpectationV1 {
        UnfreezeExpectationV1 {
            expected_proposal_digest: bytes(125),
            expected_policy_version: 126,
            expected_policy_hash: bytes(127),
            expected_current_council_version: 128,
            expected_current_council_hash: bytes(129),
            expected_frozen_gate_epoch: 130,
            expected_target_nonce: 131,
            expected_proposal_state: ProposalStateV2::PoststateAccepted,
            expected_poststate_checkpoint_digest: bytes(132),
            expected_programdata_authority: key(133),
            expected_programdata_deployed_slot: 134,
            expected_programdata_capacity: 135,
            expected_raw_programdata_hash: bytes(136),
            expected_unfreeze_approval_bitset: 0b0000_0011,
            expected_unfreeze_approval_count: 2,
            expected_programdata_verification_finalized_slot: 137,
        }
    }

    fn bitmap(value: u8) -> [u8; VERIFICATION_BITMAP_BYTES_V1] {
        [value; VERIFICATION_BITMAP_BYTES_V1]
    }

    fn instruction_cases() -> Vec<(u8, usize, Vec<u8>)> {
        vec![
            (
                RECORD_PROPOSAL_APPROVAL_V1_TAG,
                RECORD_PROPOSAL_APPROVAL_V1_LEN,
                RecordProposalApprovalV1 {
                    expected_proposal_digest: bytes(7),
                    expected_council_version: 0x0102_0304_0506_0708,
                }
                .pack()
                .to_vec(),
            ),
            (
                INITIALIZE_CONTROLLER_V1_TAG,
                INITIALIZE_CONTROLLER_V1_LEN,
                initialize().pack().to_vec(),
            ),
            (
                CREATE_PROPOSAL_V2_TAG,
                CREATE_PROPOSAL_V2_LEN,
                create_proposal().pack().to_vec(),
            ),
            (
                APPROVE_PROPOSAL_V2_TAG,
                APPROVE_PROPOSAL_V2_LEN,
                ApproveProposalV2 {
                    expected: proposal_expectation(),
                    expected_approval_bitset: 1,
                    expected_approval_count: 1,
                }
                .pack()
                .to_vec(),
            ),
            (
                FINALIZE_GOVERNANCE_V2_TAG,
                FINALIZE_GOVERNANCE_V2_LEN,
                FinalizeGovernanceV2 {
                    expected: proposal_expectation(),
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                }
                .pack()
                .to_vec(),
            ),
            (
                QUEUE_PROPOSAL_V2_TAG,
                QUEUE_PROPOSAL_V2_LEN,
                QueueProposalV2 {
                    expected: proposal_expectation(),
                }
                .pack()
                .to_vec(),
            ),
            (
                FREEZE_PROPOSAL_V2_TAG,
                FREEZE_PROPOSAL_V2_LEN,
                FreezeProposalV2 {
                    expected: proposal_expectation(),
                    expected_next_gate_epoch: 125,
                }
                .pack()
                .to_vec(),
            ),
            (
                CANCEL_PROPOSAL_V2_TAG,
                CANCEL_PROPOSAL_V2_LEN,
                CancelProposalV2 {
                    expected: proposal_expectation(),
                    expected_cancellation_approval_bitset: 3,
                    expected_cancellation_approval_count: 2,
                    cancellation_reason_code: 126,
                }
                .pack()
                .to_vec(),
            ),
            (
                EXPIRE_PROPOSAL_V2_TAG,
                EXPIRE_PROPOSAL_V2_LEN,
                ExpireProposalV2 {
                    expected: proposal_expectation(),
                }
                .pack()
                .to_vec(),
            ),
            (
                GUARDIAN_FREEZE_V1_TAG,
                GUARDIAN_FREEZE_V1_LEN,
                guardian_freeze().pack().to_vec(),
            ),
            (
                CREATE_EMERGENCY_RESOLUTION_V1_TAG,
                CREATE_EMERGENCY_RESOLUTION_V1_LEN,
                create_emergency_resolution().pack().to_vec(),
            ),
            (
                APPROVE_EMERGENCY_RESOLUTION_V1_TAG,
                APPROVE_EMERGENCY_RESOLUTION_V1_LEN,
                ApproveEmergencyResolutionV1 {
                    expected: emergency_expectation(),
                    expected_approval_bitset: 1,
                    expected_approval_count: 1,
                }
                .pack()
                .to_vec(),
            ),
            (
                QUEUE_EMERGENCY_RESOLUTION_V1_TAG,
                QUEUE_EMERGENCY_RESOLUTION_V1_LEN,
                QueueEmergencyResolutionV1 {
                    expected: emergency_expectation(),
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                }
                .pack()
                .to_vec(),
            ),
            (
                EXECUTE_EMERGENCY_RESOLUTION_V1_TAG,
                EXECUTE_EMERGENCY_RESOLUTION_V1_LEN,
                execute_emergency_resolution().pack().to_vec(),
            ),
            (
                CONVERT_EMERGENCY_FREEZE_V2_TAG,
                CONVERT_EMERGENCY_FREEZE_V2_LEN,
                ConvertEmergencyFreezeV2 {
                    expected: proposal_expectation(),
                    expected_next_gate_epoch: 133,
                    expected_freeze_observation_digest: bytes(134),
                }
                .pack()
                .to_vec(),
            ),
            (
                CREATE_CHECKPOINT_ATTESTATION_V1_TAG,
                CREATE_CHECKPOINT_ATTESTATION_V1_LEN,
                CreateCheckpointAttestationV1 {
                    candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
                    expected_council_version: 99,
                    expected_council_hash: bytes(100),
                    seat_index: 2,
                }
                .pack()
                .to_vec(),
            ),
            (
                RECAST_CHECKPOINT_ATTESTATION_V1_TAG,
                RECAST_CHECKPOINT_ATTESTATION_V1_LEN,
                RecastCheckpointAttestationV1 {
                    candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
                    expected_council_version: 99,
                    expected_council_hash: bytes(100),
                    seat_index: 2,
                    expected_previous_attestation_digest: bytes(101),
                }
                .pack()
                .to_vec(),
            ),
            (
                FINALIZE_CHECKPOINT_V1_TAG,
                FINALIZE_CHECKPOINT_V1_LEN,
                FinalizeCheckpointV1 {
                    candidate: checkpoint_candidate(StateCheckpointPhaseV1::Poststate),
                    expected_council_version: 99,
                    expected_council_hash: bytes(100),
                }
                .pack()
                .to_vec(),
            ),
            (
                CREATE_CANDIDATE_COUNCIL_SET_V1_TAG,
                CREATE_CANDIDATE_COUNCIL_SET_V1_LEN,
                create_candidate_council().pack().to_vec(),
            ),
            (
                CREATE_COUNCIL_ROTATION_V1_TAG,
                CREATE_COUNCIL_ROTATION_V1_LEN,
                create_council_rotation().pack().to_vec(),
            ),
            (
                APPROVE_COUNCIL_ROTATION_V1_TAG,
                APPROVE_COUNCIL_ROTATION_V1_LEN,
                ApproveCouncilRotationV1 {
                    expected: rotation_expectation(),
                    expected_approval_bitset: 1,
                    expected_approval_count: 1,
                }
                .pack()
                .to_vec(),
            ),
            (
                ACTIVATE_COUNCIL_ROTATION_V1_TAG,
                ACTIVATE_COUNCIL_ROTATION_V1_LEN,
                ActivateCouncilRotationV1 {
                    expected: rotation_expectation(),
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                }
                .pack()
                .to_vec(),
            ),
            (
                QUEUE_COUNCIL_ROTATION_V1_TAG,
                QUEUE_COUNCIL_ROTATION_V1_LEN,
                QueueCouncilRotationV1 {
                    expected: rotation_expectation(),
                    expected_approval_bitset: 7,
                    expected_approval_count: 3,
                }
                .pack()
                .to_vec(),
            ),
            (
                EXPIRE_EMERGENCY_RESOLUTION_V1_TAG,
                EXPIRE_EMERGENCY_RESOLUTION_V1_LEN,
                ExpireEmergencyResolutionV1 {
                    expected: emergency_expectation(),
                }
                .pack()
                .to_vec(),
            ),
            (
                CANCEL_COUNCIL_ROTATION_V1_TAG,
                CANCEL_COUNCIL_ROTATION_V1_LEN,
                CancelCouncilRotationV1 {
                    expected: rotation_expectation(),
                    expected_cancellation_approval_bitset: 3,
                    expected_cancellation_approval_count: 2,
                    cancellation_reason_code: 125,
                }
                .pack()
                .to_vec(),
            ),
            (
                EXPIRE_COUNCIL_ROTATION_V1_TAG,
                EXPIRE_COUNCIL_ROTATION_V1_LEN,
                ExpireCouncilRotationV1 {
                    expected: rotation_expectation(),
                }
                .pack()
                .to_vec(),
            ),
            (
                ADOPT_BUFFER_V1_TAG,
                ADOPT_BUFFER_V1_LEN,
                AdoptBufferV1 {
                    expected: proposal_expectation(),
                }
                .pack()
                .to_vec(),
            ),
            (
                VERIFY_BUFFER_CHUNK_V1_TAG,
                VERIFY_BUFFER_CHUNK_V1_LEN,
                VerifyBufferChunkV1 {
                    expected: proposal_expectation(),
                    chunk_index: 1,
                    proof: merkle_proof(),
                    expected_verification_status: BufferVerificationStatusV1::Verifying,
                    expected_verified_chunk_bitmap: bitmap(1),
                    expected_verified_chunk_count: 2,
                }
                .pack()
                .to_vec(),
            ),
            (
                FINALIZE_BUFFER_VERIFICATION_V1_TAG,
                FINALIZE_BUFFER_VERIFICATION_V1_LEN,
                FinalizeBufferVerificationV1 {
                    expected: proposal_expectation(),
                    expected_verification_status: BufferVerificationStatusV1::ReadyToFinalize,
                    expected_verified_chunk_bitmap: bitmap(0xff),
                    expected_verified_chunk_count: 128,
                }
                .pack()
                .to_vec(),
            ),
            (
                EXTEND_TARGET_V1_TAG,
                EXTEND_TARGET_V1_LEN,
                ExtendTargetV1 {
                    expected: proposal_expectation(),
                    expected_prestate_checkpoint_digest: bytes(138),
                    expected_current_capacity: 1_000,
                    expected_extension_delta: 200,
                    expected_post_capacity: 1_200,
                    envelope: envelope(),
                }
                .pack()
                .to_vec(),
            ),
            (
                EXECUTE_UPGRADE_V1_TAG,
                EXECUTE_UPGRADE_V1_LEN,
                ExecuteUpgradeV1 {
                    expected: proposal_expectation(),
                    expected_prestate_checkpoint_digest: bytes(139),
                    expected_current_raw_programdata_hash: bytes(140),
                    expected_sealed_buffer_header_hash: bytes(141),
                    expected_counterpart_proposal_digest: bytes(142),
                    expected_programdata_slot: 143,
                    expected_capacity: 144,
                    expected_verified_chunk_count: 128,
                    expected_buffer_verification_status: BufferVerificationStatusV1::Verified,
                    expected_counterpart_buffer_verification_status:
                        BufferVerificationStatusV1::Verified,
                    envelope: envelope(),
                }
                .pack()
                .to_vec(),
            ),
            (
                VERIFY_PROGRAMDATA_CHUNK_V1_TAG,
                VERIFY_PROGRAMDATA_CHUNK_V1_LEN,
                VerifyProgramDataChunkV1 {
                    expected: proposal_expectation(),
                    phase: ProgramDataChunkPhaseV1::Payload,
                    chunk_index: 2,
                    proof: merkle_proof(),
                    expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
                    expected_verified_payload_chunk_bitmap: bitmap(2),
                    expected_verified_payload_chunk_count: 3,
                    expected_verified_tail_chunk_bitmap: bitmap(0),
                    expected_verified_tail_chunk_count: 0,
                }
                .pack()
                .to_vec(),
            ),
            (
                FINALIZE_PROGRAMDATA_VERIFICATION_V1_TAG,
                FINALIZE_PROGRAMDATA_VERIFICATION_V1_LEN,
                FinalizeProgramDataVerificationV1 {
                    expected: proposal_expectation(),
                    expected_verification_status: ProgramDataVerificationStatusV1::ReadyToFinalize,
                    expected_verified_payload_chunk_bitmap: bitmap(0xff),
                    expected_verified_payload_chunk_count: 128,
                    expected_verified_tail_chunk_bitmap: bitmap(0),
                    expected_verified_tail_chunk_count: 0,
                    expected_deployed_slot: 145,
                    expected_capacity: 146,
                }
                .pack()
                .to_vec(),
            ),
            (
                APPROVE_UNFREEZE_V1_TAG,
                APPROVE_UNFREEZE_V1_LEN,
                ApproveUnfreezeV1 {
                    expected: unfreeze_expectation(),
                }
                .pack()
                .to_vec(),
            ),
            (
                EXECUTE_UNFREEZE_V1_TAG,
                EXECUTE_UNFREEZE_V1_LEN,
                ExecuteUnfreezeV1 {
                    expected: unfreeze_expectation(),
                    linked_proposal: key(147),
                    envelope: envelope(),
                }
                .pack()
                .to_vec(),
            ),
            (
                CLOSE_ABANDONED_BUFFER_V1_TAG,
                CLOSE_ABANDONED_BUFFER_V1_LEN,
                CloseAbandonedBufferV1 {
                    expected: proposal_expectation(),
                    expected_verification_status: BufferVerificationStatusV1::Verified,
                    expected_verified_chunk_bitmap: bitmap(0xff),
                    expected_verified_chunk_count: 128,
                    expected_buffer_verification_finalized_slot: 148,
                }
                .pack()
                .to_vec(),
            ),
            (
                ACTIVATE_ROLLBACK_V1_TAG,
                ACTIVATE_ROLLBACK_V1_LEN,
                ActivateRollbackV1 {
                    expected_primary: proposal_expectation(),
                    expected_rollback: proposal_expectation(),
                    expected_failure_evidence_digest: bytes(149),
                    expected_primary_programdata_verification_status:
                        ProgramDataVerificationStatusV1::Verifying,
                    expected_primary_programdata_verification_finalized_slot: 0,
                    expected_rollback_buffer_verification_status:
                        BufferVerificationStatusV1::Verified,
                    expected_rollback_verified_chunk_bitmap: bitmap(0xff),
                    expected_rollback_verified_chunk_count: 128,
                }
                .pack()
                .to_vec(),
            ),
            (
                OBSERVE_PROGRAMDATA_FAILURE_V1_TAG,
                OBSERVE_PROGRAMDATA_FAILURE_V1_LEN,
                ObserveProgramDataFailureV1 {
                    expected: proposal_expectation(),
                    expected_program_owner: key(151),
                    expected_program_executable: true,
                    expected_program_data_length: 36,
                    expected_program_header_present: true,
                    expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(152))
                        .unwrap(),
                    expected_programdata_owner: key(153),
                    expected_programdata_executable: false,
                    expected_programdata_data_length: 4_141,
                    expected_programdata_header_present: true,
                    expected_programdata_slot: 153,
                    expected_raw_hash_complete: true,
                    expected_raw_programdata_hash: bytes(154),
                    expected_capacity: 4_096,
                    expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(155))
                        .unwrap(),
                    mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
                    failing_chunk_index: 3,
                    expected_leaf_hash: bytes(150),
                    proof: merkle_proof(),
                }
                .pack()
                .to_vec(),
            ),
        ]
    }

    #[test]
    fn tag_zero_remains_byte_for_byte_frozen() {
        let value = RecordProposalApprovalV1 {
            expected_proposal_digest: [7; 32],
            expected_council_version: 0x0102_0304_0506_0708,
        };
        let bytes = value.pack();
        assert_eq!(bytes.len(), RECORD_PROPOSAL_APPROVAL_V1_LEN);
        assert_eq!(RecordProposalApprovalV1::unpack(&bytes), Ok(value));
        assert_eq!(
            bytes,
            [
                0, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
                7, 7, 7, 7, 7, 8, 7, 6, 5, 4, 3, 2, 1,
            ]
        );
        assert_eq!(&bytes[33..], &value.expected_council_version.to_le_bytes());
    }

    #[test]
    fn all_instruction_codecs_are_exact_bounded_and_reject_drift() {
        let cases = instruction_cases();
        assert_eq!(cases.len(), 38);
        for (expected_tag, expected_len, packed) in cases {
            assert_eq!(packed[0], expected_tag);
            assert_eq!(packed.len(), expected_len);
            assert!(packed.len() <= MAX_CONTROLLER_INSTRUCTION_DATA_LEN);
            assert!(
                UpgradeControllerInstruction::unpack(&packed).is_ok(),
                "tag {expected_tag} failed canonical decode"
            );

            let truncated = &packed[..packed.len() - 1];
            assert_eq!(
                UpgradeControllerInstruction::unpack(truncated),
                Err(ProgramError::InvalidInstructionData),
                "tag {expected_tag} accepted truncation"
            );
            let mut trailing = packed.clone();
            trailing.push(0);
            assert_eq!(
                UpgradeControllerInstruction::unpack(&trailing),
                Err(ProgramError::InvalidInstructionData),
                "tag {expected_tag} accepted trailing data"
            );
            let mut unknown = packed;
            unknown[0] = u8::MAX;
            assert_eq!(
                UpgradeControllerInstruction::unpack(&unknown),
                Err(ProgramError::InvalidInstructionData)
            );
        }
        assert_eq!(
            UpgradeControllerInstruction::unpack(&[]),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            UpgradeControllerInstruction::unpack(&vec![0; MAX_CONTROLLER_INSTRUCTION_DATA_LEN + 1]),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            UpgradeControllerInstruction::unpack(&[CANCEL_EMERGENCY_RESOLUTION_V2_RESERVED_TAG]),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn fixed_instruction_lengths_are_fully_accounted_for() {
        assert_eq!(FixedMerkleProofV1::LEN, 225);
        assert_eq!(<EnvelopeExpectationV1 as FixedWire>::LEN, 78);
        assert_eq!(<ProposalExpectationV2 as FixedWire>::LEN, 162);
        assert_eq!(<EmergencyResolutionExpectationV1 as FixedWire>::LEN, 156);
        assert_eq!(<UnfreezeExpectationV1 as FixedWire>::LEN, 251);
        assert_eq!(GUARDIAN_FREEZE_V1_LEN, 259);
        assert_eq!(CREATE_EMERGENCY_RESOLUTION_V1_LEN, 395);
        assert_eq!(EXECUTE_EMERGENCY_RESOLUTION_V1_LEN, 420);
        assert_eq!(ADOPT_BUFFER_V1_LEN, 163);
        assert_eq!(VERIFY_BUFFER_CHUNK_V1_LEN, 461);
        assert_eq!(FINALIZE_BUFFER_VERIFICATION_V1_LEN, 232);
        assert_eq!(EXTEND_TARGET_V1_LEN, 297);
        assert_eq!(EXECUTE_UPGRADE_V1_LEN, 391);
        assert_eq!(VERIFY_PROGRAMDATA_CHUNK_V1_LEN, 530);
        assert_eq!(FINALIZE_PROGRAMDATA_VERIFICATION_V1_LEN, 316);
        assert_eq!(APPROVE_UNFREEZE_V1_LEN, 252);
        assert_eq!(EXECUTE_UNFREEZE_V1_LEN, 362);
        assert_eq!(CLOSE_ABANDONED_BUFFER_V1_LEN, 240);
        assert_eq!(ACTIVATE_ROLLBACK_V1_LEN, 435);
        assert_eq!(OBSERVE_PROGRAMDATA_FAILURE_V1_LEN, 624);
    }

    #[test]
    fn fixed_merkle_proofs_reject_noncanonical_padding_and_phase_misuse() {
        let mut valid = VerifyBufferChunkV1 {
            expected: proposal_expectation(),
            chunk_index: 1,
            proof: merkle_proof(),
            expected_verification_status: BufferVerificationStatusV1::Verifying,
            expected_verified_chunk_bitmap: bitmap(1),
            expected_verified_chunk_count: 2,
        }
        .pack();
        assert!(UpgradeControllerInstruction::unpack(&valid).is_ok());

        // tag + ProposalExpectationV2 + chunk index, then proof_len and nodes.
        let proof_len_offset = 1 + <ProposalExpectationV2 as FixedWire>::LEN + 4;
        valid[proof_len_offset] = (MAX_FIXED_MERKLE_PROOF_NODES_V1 + 1) as u8;
        assert_eq!(
            UpgradeControllerInstruction::unpack(&valid),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut padded = VerifyBufferChunkV1 {
            expected: proposal_expectation(),
            chunk_index: 1,
            proof: merkle_proof(),
            expected_verification_status: BufferVerificationStatusV1::Verifying,
            expected_verified_chunk_bitmap: bitmap(1),
            expected_verified_chunk_count: 2,
        }
        .pack();
        let first_unused_node = proof_len_offset + 1 + 2 * 32;
        padded[first_unused_node] = 1;
        assert_eq!(
            UpgradeControllerInstruction::unpack(&padded),
            Err(ProgramError::InvalidInstructionData)
        );

        let invalid_tail = VerifyProgramDataChunkV1 {
            expected: proposal_expectation(),
            phase: ProgramDataChunkPhaseV1::ZeroTail,
            chunk_index: 0,
            proof: merkle_proof(),
            expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
            expected_verified_payload_chunk_bitmap: bitmap(0),
            expected_verified_payload_chunk_count: 0,
            expected_verified_tail_chunk_bitmap: bitmap(0),
            expected_verified_tail_chunk_count: 0,
        }
        .pack();
        assert_eq!(
            VerifyProgramDataChunkV1::unpack(&invalid_tail),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            UpgradeControllerInstruction::unpack(&invalid_tail),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn failure_observation_codec_rejects_cross_class_leaf_shapes() {
        let invalid_header = ObserveProgramDataFailureV1 {
            expected: proposal_expectation(),
            expected_program_owner: key(151),
            expected_program_executable: true,
            expected_program_data_length: 36,
            expected_program_header_present: true,
            expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(152)).unwrap(),
            expected_programdata_owner: key(153),
            expected_programdata_executable: false,
            expected_programdata_data_length: 4_141,
            expected_programdata_header_present: true,
            expected_programdata_slot: 153,
            expected_raw_hash_complete: true,
            expected_raw_programdata_hash: bytes(154),
            expected_capacity: 4_096,
            expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(155)).unwrap(),
            mismatch_class: ProgramDataMismatchClassV1::Header,
            failing_chunk_index: 0,
            expected_leaf_hash: bytes(1),
            proof: merkle_proof(),
        }
        .pack();
        assert_eq!(
            ObserveProgramDataFailureV1::unpack(&invalid_header),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            UpgradeControllerInstruction::unpack(&invalid_header),
            Err(ProgramError::InvalidInstructionData)
        );

        let valid_header = ObserveProgramDataFailureV1 {
            expected: proposal_expectation(),
            expected_program_owner: key(151),
            expected_program_executable: true,
            expected_program_data_length: 36,
            expected_program_header_present: true,
            expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(152)).unwrap(),
            expected_programdata_owner: key(153),
            expected_programdata_executable: false,
            expected_programdata_data_length: 4_141,
            expected_programdata_header_present: true,
            expected_programdata_slot: 153,
            expected_raw_hash_complete: true,
            expected_raw_programdata_hash: bytes(154),
            expected_capacity: 4_096,
            expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(155)).unwrap(),
            mismatch_class: ProgramDataMismatchClassV1::Header,
            failing_chunk_index: u32::MAX,
            expected_leaf_hash: [0; 32],
            proof: FixedMerkleProofV1::empty(),
        }
        .pack();
        assert!(UpgradeControllerInstruction::unpack(&valid_header).is_ok());
    }

    #[test]
    fn envelope_decode_enforces_bounds_and_nonce_pairing() {
        let instruction = |envelope| {
            ExtendTargetV1 {
                expected: proposal_expectation(),
                expected_prestate_checkpoint_digest: bytes(1),
                expected_current_capacity: 1,
                expected_extension_delta: 1,
                expected_post_capacity: 2,
                envelope,
            }
            .pack()
        };

        for envelope in [
            EnvelopeExpectationV1 {
                compute_unit_limit: 0,
                ..envelope()
            },
            EnvelopeExpectationV1 {
                compute_unit_limit: MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1 + 1,
                ..envelope()
            },
            EnvelopeExpectationV1 {
                compute_unit_price_micro_lamports: MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1
                    + 1,
                ..envelope()
            },
            EnvelopeExpectationV1 {
                durable_nonce_account: OptionalInstructionPubkeyV1::none(),
                ..envelope()
            },
            EnvelopeExpectationV1 {
                durable_nonce_authority: OptionalInstructionPubkeyV1::none(),
                ..envelope()
            },
        ] {
            let packed = instruction(envelope);
            assert_eq!(
                ExtendTargetV1::unpack(&packed),
                Err(ProgramError::InvalidInstructionData)
            );
        }

        let no_nonce = instruction(EnvelopeExpectationV1 {
            compute_unit_limit: 1,
            compute_unit_price_micro_lamports: 0,
            durable_nonce_account: OptionalInstructionPubkeyV1::none(),
            durable_nonce_authority: OptionalInstructionPubkeyV1::none(),
        });
        assert!(ExtendTargetV1::unpack(&no_nonce).is_ok());

        let max_values = instruction(EnvelopeExpectationV1 {
            compute_unit_limit: MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1,
            compute_unit_price_micro_lamports: MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
            durable_nonce_account: OptionalInstructionPubkeyV1::some(key(1)).unwrap(),
            durable_nonce_authority: OptionalInstructionPubkeyV1::some(key(2)).unwrap(),
        });
        assert!(ExtendTargetV1::unpack(&max_values).is_ok());

        let default_link = ExecuteUnfreezeV1 {
            expected: unfreeze_expectation(),
            linked_proposal: Pubkey::default(),
            envelope: envelope(),
        }
        .pack();
        assert_eq!(
            ExecuteUnfreezeV1::unpack(&default_link),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn every_instruction_enum_is_decoded_explicitly() {
        let mut proposal = create_proposal().pack();
        proposal[1] = u8::MAX;
        assert_eq!(
            CreateProposalV2::unpack(&proposal),
            Err(ProgramError::InvalidInstructionData)
        );
        for unsupported_class in [
            ProposalClassV1::CouncilSetRotation as u8,
            ProposalClassV1::TargetImmutability as u8,
        ] {
            let mut proposal = create_proposal().pack();
            proposal[1] = unsupported_class;
            assert_eq!(
                CreateProposalV2::unpack(&proposal),
                Err(ProgramError::InvalidInstructionData)
            );
        }
        let mut proposal = create_proposal().pack();
        proposal[2] = u8::MAX;
        assert_eq!(
            CreateProposalV2::unpack(&proposal),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut approval = ApproveProposalV2 {
            expected: proposal_expectation(),
            expected_approval_bitset: 1,
            expected_approval_count: 1,
        }
        .pack();
        approval[130] = u8::MAX;
        assert_eq!(
            ApproveProposalV2::unpack(&approval),
            Err(ProgramError::InvalidInstructionData)
        );
        let mut approval = ApproveProposalV2 {
            expected: proposal_expectation(),
            expected_approval_bitset: 1,
            expected_approval_count: 1,
        }
        .pack();
        approval[130] = ProposalStateV2::TokenReviewOpen as u8;
        assert_eq!(
            ApproveProposalV2::unpack(&approval),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut emergency = create_emergency_resolution().pack();
        emergency[1] = u8::MAX;
        assert_eq!(
            CreateEmergencyResolutionV1::unpack(&emergency),
            Err(ProgramError::InvalidInstructionData)
        );
        let mut emergency = ApproveEmergencyResolutionV1 {
            expected: emergency_expectation(),
            expected_approval_bitset: 1,
            expected_approval_count: 1,
        }
        .pack();
        emergency[140] = u8::MAX;
        assert_eq!(
            ApproveEmergencyResolutionV1::unpack(&emergency),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut checkpoint = CreateCheckpointAttestationV1 {
            candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
            expected_council_version: 99,
            expected_council_hash: bytes(100),
            seat_index: 2,
        }
        .pack();
        checkpoint[1] = u8::MAX;
        assert_eq!(
            CreateCheckpointAttestationV1::unpack(&checkpoint),
            Err(ProgramError::InvalidInstructionData)
        );
        let mut checkpoint = CreateCheckpointAttestationV1 {
            candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
            expected_council_version: 99,
            expected_council_hash: bytes(100),
            seat_index: 2,
        }
        .pack();
        checkpoint[2] = u8::MAX;
        assert_eq!(
            CreateCheckpointAttestationV1::unpack(&checkpoint),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut rotation = ApproveCouncilRotationV1 {
            expected: rotation_expectation(),
            expected_approval_bitset: 1,
            expected_approval_count: 1,
        }
        .pack();
        rotation[130] = u8::MAX;
        assert_eq!(
            ApproveCouncilRotationV1::unpack(&rotation),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn optional_pubkeys_are_fixed_width_and_canonical() {
        let mut absent_nonzero = [0u8; 33];
        absent_nonzero[1] = 1;
        assert_eq!(
            OptionalInstructionPubkeyV1::decode(&mut FixedReader::new(&absent_nonzero)),
            Err(ProgramError::InvalidInstructionData)
        );
        let present_default = {
            let mut value = [0u8; 33];
            value[0] = 1;
            value
        };
        assert_eq!(
            OptionalInstructionPubkeyV1::decode(&mut FixedReader::new(&present_default)),
            Err(ProgramError::InvalidInstructionData)
        );
        let mut unknown_presence = [0u8; 33];
        unknown_presence[0] = 2;
        assert_eq!(
            OptionalInstructionPubkeyV1::decode(&mut FixedReader::new(&unknown_presence)),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut guardian_none = guardian_freeze();
        guardian_none.expected_programdata_header_present = false;
        guardian_none.expected_programdata_slot = 0;
        guardian_none.expected_capacity = 0;
        guardian_none.expected_programdata_authority = OptionalInstructionPubkeyV1::none();
        assert_eq!(
            GuardianFreezeV1::unpack(&guardian_none.pack()),
            Ok(guardian_none)
        );

        let mut create_none = create_emergency_resolution();
        create_none.observed_programdata_header_present = false;
        create_none.observed_programdata_slot = 0;
        create_none.observed_capacity = 0;
        create_none.observed_programdata_authority = OptionalInstructionPubkeyV1::none();
        assert_eq!(
            CreateEmergencyResolutionV1::unpack(&create_none.pack()),
            Ok(create_none)
        );

        let mut invalid_bool = guardian_freeze().pack();
        // tag/status/epochs/nonce/owner occupy bytes 0..58.
        invalid_bool[58] = 2;
        assert_eq!(
            GuardianFreezeV1::unpack(&invalid_bool),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn observation_instruction_codecs_fail_closed_on_malformed_graphs_and_hash_shapes() {
        let mut missing_program_link = guardian_freeze();
        missing_program_link.expected_linked_programdata = OptionalInstructionPubkeyV1::none();
        assert_eq!(
            GuardianFreezeV1::unpack(&missing_program_link.pack()),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut link_without_header = guardian_freeze();
        link_without_header.expected_program_header_present = false;
        assert_eq!(
            GuardianFreezeV1::unpack(&link_without_header.pack()),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut wrong_program_length = guardian_freeze();
        wrong_program_length.expected_program_data_length = LOADER_V3_PROGRAM_ACCOUNT_LEN_V1 - 1;
        assert_eq!(
            GuardianFreezeV1::unpack(&wrong_program_length.pack()),
            Err(ProgramError::InvalidInstructionData)
        );

        let mut oversized_guardian = guardian_freeze();
        oversized_guardian.expected_programdata_data_length =
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        oversized_guardian.expected_capacity =
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1 - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
        oversized_guardian.expected_raw_hash_complete = false;
        oversized_guardian.expected_raw_programdata_hash = [0; 32];
        assert_eq!(
            GuardianFreezeV1::unpack(&oversized_guardian.pack()),
            Ok(oversized_guardian)
        );

        let mut oversized_resolution = create_emergency_resolution();
        oversized_resolution.observed_programdata_data_length =
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        oversized_resolution.observed_capacity =
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1 - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
        oversized_resolution.observed_raw_hash_complete = false;
        oversized_resolution.observed_raw_programdata_hash = [0; 32];
        assert_eq!(
            CreateEmergencyResolutionV1::unpack(&oversized_resolution.pack()),
            Ok(oversized_resolution)
        );

        let mut incomplete_execute = execute_emergency_resolution();
        incomplete_execute.expected_programdata_data_length =
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1;
        incomplete_execute.expected_capacity =
            MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1 + 1 - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;
        incomplete_execute.expected_raw_hash_complete = false;
        incomplete_execute.expected_raw_programdata_hash = [0; 32];
        assert_eq!(
            ExecuteEmergencyResolutionV1::unpack(&incomplete_execute.pack()),
            Err(ProgramError::InvalidInstructionData)
        );

        let canonical_execute = execute_emergency_resolution();
        for malformed in [
            ExecuteEmergencyResolutionV1 {
                expected_program_owner: key(1),
                ..canonical_execute.clone()
            },
            ExecuteEmergencyResolutionV1 {
                expected_program_executable: false,
                ..canonical_execute.clone()
            },
            ExecuteEmergencyResolutionV1 {
                expected_programdata_owner: key(2),
                ..canonical_execute.clone()
            },
            ExecuteEmergencyResolutionV1 {
                expected_programdata_executable: true,
                ..canonical_execute.clone()
            },
        ] {
            assert_eq!(
                ExecuteEmergencyResolutionV1::unpack(&malformed.pack()),
                Err(ProgramError::InvalidInstructionData)
            );
        }
    }

    fn assert_account_contract(
        instruction: Instruction,
        expected_data: Vec<u8>,
        expected_accounts: &[(u8, bool, bool)],
    ) {
        assert_eq!(instruction.program_id, key(250));
        assert_eq!(instruction.data, expected_data);
        assert_eq!(instruction.accounts.len(), expected_accounts.len());
        for (index, (actual, (key_byte, is_signer, is_writable))) in instruction
            .accounts
            .iter()
            .zip(expected_accounts.iter().copied())
            .enumerate()
        {
            assert_eq!(actual.pubkey, key(key_byte), "account {index} key");
            assert_eq!(actual.is_signer, is_signer, "account {index} signer");
            assert_eq!(actual.is_writable, is_writable, "account {index} writable");
        }
    }

    macro_rules! assert_fixed_builder_contract {
        (
            $builder:ident,
            $accounts_type:ident {
                $($field:ident: $key_byte:expr => ($is_signer:expr, $is_writable:expr)),+ $(,)?
            },
            $instruction:expr $(,)?
        ) => {{
            let instruction = $instruction;
            let expected_data = instruction.pack().to_vec();
            let built = $builder(
                key(250),
                $accounts_type {
                    $($field: key($key_byte),)+
                },
                instruction,
            );
            assert_account_contract(
                built,
                expected_data,
                &[$(($key_byte, $is_signer, $is_writable)),+],
            );
        }};
    }

    #[test]
    fn legacy_and_initialization_builders_freeze_account_order_and_privileges() {
        let legacy = RecordProposalApprovalV1 {
            expected_proposal_digest: bytes(1),
            expected_council_version: 2,
        };
        assert_account_contract(
            record_proposal_approval_instruction(
                key(250),
                key(1),
                key(2),
                key(3),
                key(4),
                key(5),
                legacy.expected_proposal_digest,
                legacy.expected_council_version,
            ),
            legacy.pack().to_vec(),
            &[
                (1, false, false),
                (2, false, false),
                (3, false, false),
                (4, false, true),
                (5, true, false),
            ],
        );

        let initialization = initialize();
        assert_account_contract(
            initialize_controller_v1_instruction(
                key(250),
                InitializeControllerV1Accounts {
                    payer: key(1),
                    initializer: key(2),
                    controller_program: key(3),
                    controller_programdata: key(4),
                    target_program: key(5),
                    target_programdata: key(6),
                    upgradeable_loader: key(7),
                    controller_config: key(8),
                    authority_pda: key(9),
                    protocol_gate: key(10),
                    policy: key(11),
                    council: key(12),
                    canonical_spill_treasury: key(13),
                    guardian: key(14),
                    seat_authorities: [key(15), key(16), key(17), key(18), key(19)],
                    system_program: key(20),
                },
                initialization.clone(),
            ),
            initialization.pack().to_vec(),
            &[
                (1, true, true),
                (2, true, false),
                (3, false, false),
                (4, false, false),
                (5, false, false),
                (6, false, false),
                (7, false, false),
                (8, false, true),
                (9, false, false),
                (10, false, true),
                (11, false, true),
                (12, false, true),
                (13, false, false),
                (14, false, false),
                (15, false, false),
                (16, false, false),
                (17, false, false),
                (18, false, false),
                (19, false, false),
                (20, false, false),
            ],
        );
    }

    #[test]
    fn proposal_builders_freeze_account_order_and_privileges() {
        assert_fixed_builder_contract!(
            create_proposal_v2_instruction,
            CreateProposalV2Accounts {
                payer: 1 => (true, true),
                creator_seat_authority: 2 => (true, false),
                controller_config: 3 => (false, true),
                policy: 4 => (false, false),
                council: 5 => (false, false),
                protocol_gate: 6 => (false, false),
                target_program: 7 => (false, false),
                target_programdata: 8 => (false, false),
                upgradeable_loader: 9 => (false, false),
                authority_pda: 10 => (false, false),
                canonical_spill_treasury: 11 => (false, false),
                buffer: 12 => (false, false),
                buffer_uploader_authority: 13 => (false, false),
                proposal: 14 => (false, true),
                system_program: 15 => (false, false),
            },
            create_proposal(),
        );
        assert_fixed_builder_contract!(
            approve_proposal_v2_instruction,
            ApproveProposalV2Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                proposal: 5 => (false, true),
                seat_authority: 6 => (true, false),
            },
            ApproveProposalV2 {
                expected: proposal_expectation(),
                expected_approval_bitset: 1,
                expected_approval_count: 1,
            },
        );
        assert_fixed_builder_contract!(
            finalize_governance_v2_instruction,
            FinalizeGovernanceV2Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                proposal: 5 => (false, true),
            },
            FinalizeGovernanceV2 {
                expected: proposal_expectation(),
                expected_approval_bitset: 7,
                expected_approval_count: 3,
            },
        );
        assert_fixed_builder_contract!(
            queue_proposal_v2_instruction,
            QueueProposalV2Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                protocol_gate: 3 => (false, false),
                proposal: 4 => (false, true),
            },
            QueueProposalV2 {
                expected: proposal_expectation(),
            },
        );
        assert_fixed_builder_contract!(
            freeze_proposal_v2_instruction,
            FreezeProposalV2Accounts {
                controller_config: 1 => (false, true),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, true),
                proposal: 5 => (false, true),
                target_program: 6 => (false, false),
                target_programdata: 7 => (false, false),
                upgradeable_loader: 8 => (false, false),
                authority_pda: 9 => (false, false),
                rollback_proposal: 10 => (false, false),
                rollback_buffer_verification: 11 => (false, false),
                rollback_buffer: 12 => (false, false),
            },
            FreezeProposalV2 {
                expected: proposal_expectation(),
                expected_next_gate_epoch: 125,
            },
        );
        assert_fixed_builder_contract!(
            cancel_proposal_v2_instruction,
            CancelProposalV2Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                proposal: 5 => (false, true),
                seat_authority: 6 => (true, false),
            },
            CancelProposalV2 {
                expected: proposal_expectation(),
                expected_cancellation_approval_bitset: 3,
                expected_cancellation_approval_count: 2,
                cancellation_reason_code: 126,
            },
        );
        assert_fixed_builder_contract!(
            expire_proposal_v2_instruction,
            ExpireProposalV2Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                proposal: 3 => (false, true),
            },
            ExpireProposalV2 {
                expected: proposal_expectation(),
            },
        );
        assert_fixed_builder_contract!(
            convert_emergency_freeze_v2_instruction,
            ConvertEmergencyFreezeV2Accounts {
                controller_config: 1 => (false, true),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, true),
                proposal: 5 => (false, true),
                emergency_freeze_observation: 6 => (false, false),
                target_program: 7 => (false, false),
                target_programdata: 8 => (false, false),
                upgradeable_loader: 9 => (false, false),
                authority_pda: 10 => (false, false),
                rollback_proposal: 11 => (false, false),
                rollback_buffer_verification: 12 => (false, false),
                rollback_buffer: 13 => (false, false),
            },
            ConvertEmergencyFreezeV2 {
                expected: proposal_expectation(),
                expected_next_gate_epoch: 133,
                expected_freeze_observation_digest: bytes(134),
            },
        );
    }

    #[test]
    fn guardian_and_emergency_builders_freeze_account_order_and_privileges() {
        assert_fixed_builder_contract!(
            guardian_freeze_v1_instruction,
            GuardianFreezeV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                protocol_gate: 3 => (false, true),
                target_program: 4 => (false, false),
                target_programdata: 5 => (false, false),
                upgradeable_loader: 6 => (false, false),
                authority_pda: 7 => (false, false),
                guardian: 8 => (true, false),
                emergency_freeze_observation: 9 => (false, true),
                system_program: 10 => (false, false),
            },
            guardian_freeze(),
        );
        assert_fixed_builder_contract!(
            create_emergency_resolution_v1_instruction,
            CreateEmergencyResolutionV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                policy: 3 => (false, false),
                council: 4 => (false, false),
                protocol_gate: 5 => (false, false),
                emergency_freeze_observation: 6 => (false, false),
                emergency_resolution: 7 => (false, true),
                system_program: 8 => (false, false),
            },
            create_emergency_resolution(),
        );
        assert_fixed_builder_contract!(
            approve_emergency_resolution_v1_instruction,
            ApproveEmergencyResolutionV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                emergency_resolution: 5 => (false, true),
                seat_authority: 6 => (true, false),
            },
            ApproveEmergencyResolutionV1 {
                expected: emergency_expectation(),
                expected_approval_bitset: 1,
                expected_approval_count: 1,
            },
        );
        assert_fixed_builder_contract!(
            queue_emergency_resolution_v1_instruction,
            QueueEmergencyResolutionV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                emergency_resolution: 5 => (false, true),
            },
            QueueEmergencyResolutionV1 {
                expected: emergency_expectation(),
                expected_approval_bitset: 7,
                expected_approval_count: 3,
            },
        );
        assert_fixed_builder_contract!(
            execute_emergency_resolution_v1_instruction,
            ExecuteEmergencyResolutionV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, true),
                emergency_resolution: 5 => (false, true),
                emergency_freeze_observation: 6 => (false, false),
                emergency_checkpoint: 7 => (false, false),
                target_program: 8 => (false, false),
                target_programdata: 9 => (false, false),
                upgradeable_loader: 10 => (false, false),
                authority_pda: 11 => (false, false),
                instructions_sysvar: 12 => (false, false),
            },
            execute_emergency_resolution(),
        );
        assert_fixed_builder_contract!(
            expire_emergency_resolution_v1_instruction,
            ExpireEmergencyResolutionV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                protocol_gate: 3 => (false, false),
                emergency_resolution: 4 => (false, true),
            },
            ExpireEmergencyResolutionV1 {
                expected: emergency_expectation(),
            },
        );
    }

    #[test]
    fn checkpoint_builders_freeze_account_order_and_phase_privileges() {
        assert_fixed_builder_contract!(
            create_checkpoint_attestation_v1_instruction,
            CreateCheckpointAttestationV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                policy: 3 => (false, false),
                council: 4 => (false, false),
                protocol_gate: 5 => (false, false),
                subject: 6 => (false, false),
                checkpoint: 7 => (false, false),
                checkpoint_attestation: 8 => (false, true),
                seat_authority: 9 => (true, false),
                system_program: 10 => (false, false),
            },
            CreateCheckpointAttestationV1 {
                candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
                expected_council_version: 99,
                expected_council_hash: bytes(100),
                seat_index: 2,
            },
        );
        assert_fixed_builder_contract!(
            recast_checkpoint_attestation_v1_instruction,
            RecastCheckpointAttestationV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                subject: 5 => (false, false),
                checkpoint: 6 => (false, false),
                checkpoint_attestation: 7 => (false, true),
                seat_authority: 8 => (true, false),
            },
            RecastCheckpointAttestationV1 {
                candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
                expected_council_version: 99,
                expected_council_hash: bytes(100),
                seat_index: 2,
                expected_previous_attestation_digest: bytes(101),
            },
        );

        for (phase, subject_writable) in [
            (StateCheckpointPhaseV1::Prestate, false),
            (StateCheckpointPhaseV1::Poststate, true),
            (StateCheckpointPhaseV1::Emergency, false),
        ] {
            let instruction = FinalizeCheckpointV1 {
                candidate: checkpoint_candidate(phase),
                expected_council_version: 99,
                expected_council_hash: bytes(100),
            };
            let mut expected_accounts = vec![
                (1, true, true),
                (2, false, false),
                (3, false, false),
                (4, false, false),
                (5, false, false),
                (6, false, subject_writable),
                (7, false, false),
                (8, false, false),
                (9, false, false),
            ];
            if phase == StateCheckpointPhaseV1::Poststate {
                expected_accounts.push((10, false, false));
            }
            expected_accounts.extend([
                (11, false, true),
                (12, false, false),
                (13, false, false),
                (14, false, false),
                (15, false, false),
            ]);
            assert_account_contract(
                finalize_checkpoint_v1_instruction(
                    key(250),
                    FinalizeCheckpointV1Accounts {
                        payer: key(1),
                        controller_config: key(2),
                        policy: key(3),
                        council: key(4),
                        protocol_gate: key(5),
                        subject: key(6),
                        target_program: key(7),
                        target_programdata: key(8),
                        phase_evidence: key(9),
                        baseline_checkpoint: if phase == StateCheckpointPhaseV1::Poststate {
                            Some(key(10))
                        } else {
                            None
                        },
                        checkpoint: key(11),
                        checkpoint_attestations: [key(12), key(13), key(14)],
                        system_program: key(15),
                    },
                    instruction.clone(),
                ),
                instruction.pack().to_vec(),
                &expected_accounts,
            );
        }

        // EmergencyRollback Prestate uses the same wire instruction but adds
        // the primary proposal's accepted Prestate as an immutable baseline.
        let rollback_prestate = FinalizeCheckpointV1 {
            candidate: checkpoint_candidate(StateCheckpointPhaseV1::Prestate),
            expected_council_version: 99,
            expected_council_hash: bytes(100),
        };
        assert_account_contract(
            finalize_checkpoint_v1_instruction(
                key(250),
                FinalizeCheckpointV1Accounts {
                    payer: key(1),
                    controller_config: key(2),
                    policy: key(3),
                    council: key(4),
                    protocol_gate: key(5),
                    subject: key(6),
                    target_program: key(7),
                    target_programdata: key(8),
                    phase_evidence: key(9),
                    baseline_checkpoint: Some(key(10)),
                    checkpoint: key(11),
                    checkpoint_attestations: [key(12), key(13), key(14)],
                    system_program: key(15),
                },
                rollback_prestate.clone(),
            ),
            rollback_prestate.pack().to_vec(),
            &[
                (1, true, true),
                (2, false, false),
                (3, false, false),
                (4, false, false),
                (5, false, false),
                (6, false, false),
                (7, false, false),
                (8, false, false),
                (9, false, false),
                (10, false, false),
                (11, false, true),
                (12, false, false),
                (13, false, false),
                (14, false, false),
                (15, false, false),
            ],
        );
    }

    #[test]
    fn council_rotation_builders_freeze_account_order_and_privileges() {
        let candidate = create_candidate_council();
        assert_account_contract(
            create_candidate_council_set_v1_instruction(
                key(250),
                CreateCandidateCouncilSetV1Accounts {
                    payer: key(1),
                    creator_seat_authority: key(2),
                    controller_config: key(3),
                    policy: key(4),
                    current_council: key(5),
                    protocol_gate: key(6),
                    candidate_council: key(7),
                    candidate_seat_authorities: [key(8), key(9), key(10), key(11), key(12)],
                    system_program: key(13),
                },
                candidate.clone(),
            ),
            candidate.pack().to_vec(),
            &[
                (1, true, true),
                (2, true, false),
                (3, false, false),
                (4, false, false),
                (5, false, false),
                (6, false, false),
                (7, false, true),
                (8, false, false),
                (9, false, false),
                (10, false, false),
                (11, false, false),
                (12, false, false),
                (13, false, false),
            ],
        );
        assert_fixed_builder_contract!(
            create_council_rotation_v1_instruction,
            CreateCouncilRotationV1Accounts {
                payer: 1 => (true, true),
                creator_seat_authority: 2 => (true, false),
                controller_config: 3 => (false, false),
                policy: 4 => (false, false),
                current_council: 5 => (false, false),
                candidate_council: 6 => (false, false),
                protocol_gate: 7 => (false, false),
                rotation: 8 => (false, true),
                system_program: 9 => (false, false),
            },
            create_council_rotation(),
        );
        assert_fixed_builder_contract!(
            approve_council_rotation_v1_instruction,
            ApproveCouncilRotationV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                current_council: 3 => (false, false),
                candidate_council: 4 => (false, false),
                protocol_gate: 5 => (false, false),
                rotation: 6 => (false, true),
                seat_authority: 7 => (true, false),
            },
            ApproveCouncilRotationV1 {
                expected: rotation_expectation(),
                expected_approval_bitset: 1,
                expected_approval_count: 1,
            },
        );
        assert_fixed_builder_contract!(
            queue_council_rotation_v1_instruction,
            QueueCouncilRotationV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                current_council: 3 => (false, false),
                candidate_council: 4 => (false, false),
                protocol_gate: 5 => (false, false),
                rotation: 6 => (false, true),
            },
            QueueCouncilRotationV1 {
                expected: rotation_expectation(),
                expected_approval_bitset: 7,
                expected_approval_count: 3,
            },
        );
        assert_fixed_builder_contract!(
            activate_council_rotation_v1_instruction,
            ActivateCouncilRotationV1Accounts {
                controller_config: 1 => (false, true),
                policy: 2 => (false, false),
                current_council: 3 => (false, false),
                candidate_council: 4 => (false, false),
                protocol_gate: 5 => (false, false),
                rotation: 6 => (false, true),
            },
            ActivateCouncilRotationV1 {
                expected: rotation_expectation(),
                expected_approval_bitset: 7,
                expected_approval_count: 3,
            },
        );
        assert_fixed_builder_contract!(
            cancel_council_rotation_v1_instruction,
            CancelCouncilRotationV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                current_council: 3 => (false, false),
                candidate_council: 4 => (false, false),
                protocol_gate: 5 => (false, false),
                rotation: 6 => (false, true),
                seat_authority: 7 => (true, false),
            },
            CancelCouncilRotationV1 {
                expected: rotation_expectation(),
                expected_cancellation_approval_bitset: 3,
                expected_cancellation_approval_count: 2,
                cancellation_reason_code: 125,
            },
        );
        assert_fixed_builder_contract!(
            expire_council_rotation_v1_instruction,
            ExpireCouncilRotationV1Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                rotation: 3 => (false, true),
            },
            ExpireCouncilRotationV1 {
                expected: rotation_expectation(),
            },
        );
    }

    #[test]
    fn loader_and_unfreeze_builders_freeze_account_order_and_privileges() {
        assert_fixed_builder_contract!(
            adopt_buffer_v1_instruction,
            AdoptBufferV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                protocol_gate: 3 => (false, false),
                proposal: 4 => (false, true),
                buffer: 5 => (false, true),
                uploader_authority: 6 => (true, false),
                authority_pda: 7 => (false, false),
                buffer_verification: 8 => (false, true),
                upgradeable_loader: 9 => (false, false),
                system_program: 10 => (false, false),
            },
            AdoptBufferV1 {
                expected: proposal_expectation(),
            },
        );
        assert_fixed_builder_contract!(
            verify_buffer_chunk_v1_instruction,
            VerifyBufferChunkV1Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                proposal: 3 => (false, false),
                buffer: 4 => (false, false),
                buffer_verification: 5 => (false, true),
                authority_pda: 6 => (false, false),
                upgradeable_loader: 7 => (false, false),
            },
            VerifyBufferChunkV1 {
                expected: proposal_expectation(),
                chunk_index: 1,
                proof: merkle_proof(),
                expected_verification_status: BufferVerificationStatusV1::Verifying,
                expected_verified_chunk_bitmap: bitmap(1),
                expected_verified_chunk_count: 2,
            },
        );
        assert_fixed_builder_contract!(
            finalize_buffer_verification_v1_instruction,
            FinalizeBufferVerificationV1Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                proposal: 3 => (false, true),
                buffer: 4 => (false, false),
                buffer_verification: 5 => (false, true),
                authority_pda: 6 => (false, false),
                upgradeable_loader: 7 => (false, false),
            },
            FinalizeBufferVerificationV1 {
                expected: proposal_expectation(),
                expected_verification_status: BufferVerificationStatusV1::ReadyToFinalize,
                expected_verified_chunk_bitmap: bitmap(0xff),
                expected_verified_chunk_count: 128,
            },
        );
        assert_fixed_builder_contract!(
            extend_target_v1_instruction,
            ExtendTargetV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                protocol_gate: 3 => (false, false),
                proposal: 4 => (false, true),
                prestate_checkpoint: 5 => (false, false),
                target_programdata: 6 => (false, true),
                target_program: 7 => (false, true),
                authority_pda: 8 => (false, true),
                upgradeable_loader: 9 => (false, false),
                system_program: 10 => (false, false),
                rent_sysvar: 11 => (false, false),
                instructions_sysvar: 12 => (false, false),
            },
            ExtendTargetV1 {
                expected: proposal_expectation(),
                expected_prestate_checkpoint_digest: bytes(1),
                expected_current_capacity: 2,
                expected_extension_delta: 3,
                expected_post_capacity: 5,
                envelope: envelope(),
            },
        );
        assert_fixed_builder_contract!(
            execute_upgrade_v1_instruction,
            ExecuteUpgradeV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                policy: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                proposal: 5 => (false, true),
                counterpart_proposal: 6 => (false, false),
                counterpart_buffer_verification: 7 => (false, false),
                prestate_checkpoint: 8 => (false, false),
                buffer_verification: 9 => (false, true),
                programdata_verification: 10 => (false, true),
                target_programdata: 11 => (false, true),
                target_program: 12 => (false, true),
                buffer: 13 => (false, true),
                canonical_spill_treasury: 14 => (false, true),
                rent_sysvar: 15 => (false, false),
                clock_sysvar: 16 => (false, false),
                authority_pda: 17 => (false, false),
                upgradeable_loader: 18 => (false, false),
                system_program: 19 => (false, false),
                instructions_sysvar: 20 => (false, false),
            },
            ExecuteUpgradeV1 {
                expected: proposal_expectation(),
                expected_prestate_checkpoint_digest: bytes(1),
                expected_current_raw_programdata_hash: bytes(2),
                expected_sealed_buffer_header_hash: bytes(3),
                expected_counterpart_proposal_digest: bytes(4),
                expected_programdata_slot: 5,
                expected_capacity: 6,
                expected_verified_chunk_count: 7,
                expected_buffer_verification_status: BufferVerificationStatusV1::Verified,
                expected_counterpart_buffer_verification_status:
                    BufferVerificationStatusV1::Verified,
                envelope: envelope(),
            },
        );
        assert_fixed_builder_contract!(
            verify_programdata_chunk_v1_instruction,
            VerifyProgramDataChunkV1Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                proposal: 3 => (false, false),
                target_program: 4 => (false, false),
                target_programdata: 5 => (false, false),
                authority_pda: 6 => (false, false),
                upgradeable_loader: 7 => (false, false),
                programdata_verification: 8 => (false, true),
            },
            VerifyProgramDataChunkV1 {
                expected: proposal_expectation(),
                phase: ProgramDataChunkPhaseV1::Payload,
                chunk_index: 1,
                proof: merkle_proof(),
                expected_verification_status: ProgramDataVerificationStatusV1::Verifying,
                expected_verified_payload_chunk_bitmap: bitmap(1),
                expected_verified_payload_chunk_count: 2,
                expected_verified_tail_chunk_bitmap: bitmap(0),
                expected_verified_tail_chunk_count: 0,
            },
        );
        assert_fixed_builder_contract!(
            finalize_programdata_verification_v1_instruction,
            FinalizeProgramDataVerificationV1Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                proposal: 3 => (false, true),
                target_program: 4 => (false, false),
                target_programdata: 5 => (false, false),
                authority_pda: 6 => (false, false),
                upgradeable_loader: 7 => (false, false),
                programdata_verification: 8 => (false, true),
            },
            FinalizeProgramDataVerificationV1 {
                expected: proposal_expectation(),
                expected_verification_status: ProgramDataVerificationStatusV1::ReadyToFinalize,
                expected_verified_payload_chunk_bitmap: bitmap(0xff),
                expected_verified_payload_chunk_count: 128,
                expected_verified_tail_chunk_bitmap: bitmap(0),
                expected_verified_tail_chunk_count: 0,
                expected_deployed_slot: 1,
                expected_capacity: 2,
            },
        );
        assert_fixed_builder_contract!(
            approve_unfreeze_v1_instruction,
            ApproveUnfreezeV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                current_council: 3 => (false, false),
                protocol_gate: 4 => (false, false),
                proposal: 5 => (false, true),
                poststate_checkpoint: 6 => (false, false),
                programdata_verification: 7 => (false, false),
                target_program: 8 => (false, false),
                target_programdata: 9 => (false, false),
                authority_pda: 10 => (false, false),
                upgradeable_loader: 11 => (false, false),
                seat_authority: 12 => (true, false),
            },
            ApproveUnfreezeV1 {
                expected: unfreeze_expectation(),
            },
        );
        assert_fixed_builder_contract!(
            execute_unfreeze_v1_instruction,
            ExecuteUnfreezeV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                current_council: 3 => (false, false),
                protocol_gate: 4 => (false, true),
                proposal: 5 => (false, true),
                linked_proposal: 6 => (false, true),
                poststate_checkpoint: 7 => (false, false),
                programdata_verification: 8 => (false, false),
                target_program: 9 => (false, false),
                target_programdata: 10 => (false, false),
                authority_pda: 11 => (false, false),
                upgradeable_loader: 12 => (false, false),
                instructions_sysvar: 13 => (false, false),
            },
            ExecuteUnfreezeV1 {
                expected: unfreeze_expectation(),
                linked_proposal: key(1),
                envelope: envelope(),
            },
        );
        assert_fixed_builder_contract!(
            close_abandoned_buffer_v1_instruction,
            CloseAbandonedBufferV1Accounts {
                controller_config: 1 => (false, false),
                protocol_gate: 2 => (false, false),
                proposal: 3 => (false, false),
                buffer_verification: 4 => (false, true),
                buffer: 5 => (false, true),
                canonical_spill_treasury: 6 => (false, true),
                authority_pda: 7 => (false, false),
                upgradeable_loader: 8 => (false, false),
            },
            CloseAbandonedBufferV1 {
                expected: proposal_expectation(),
                expected_verification_status: BufferVerificationStatusV1::Verified,
                expected_verified_chunk_bitmap: bitmap(0xff),
                expected_verified_chunk_count: 128,
                expected_buffer_verification_finalized_slot: 1,
            },
        );
        assert_fixed_builder_contract!(
            activate_rollback_v1_instruction,
            ActivateRollbackV1Accounts {
                controller_config: 1 => (false, false),
                policy: 2 => (false, false),
                protocol_gate: 3 => (false, true),
                primary_proposal: 4 => (false, false),
                rollback_proposal: 5 => (false, true),
                rollback_buffer_verification: 6 => (false, false),
                primary_programdata_verification: 7 => (false, false),
                failure_evidence: 8 => (false, false),
                target_program: 9 => (false, false),
                target_programdata: 10 => (false, false),
                authority_pda: 11 => (false, false),
                upgradeable_loader: 12 => (false, false),
            },
            ActivateRollbackV1 {
                expected_primary: proposal_expectation(),
                expected_rollback: proposal_expectation(),
                expected_failure_evidence_digest: bytes(1),
                expected_primary_programdata_verification_status:
                    ProgramDataVerificationStatusV1::Verifying,
                expected_primary_programdata_verification_finalized_slot: 0,
                expected_rollback_buffer_verification_status:
                    BufferVerificationStatusV1::Verified,
                expected_rollback_verified_chunk_bitmap: bitmap(0xff),
                expected_rollback_verified_chunk_count: 128,
            },
        );
        assert_fixed_builder_contract!(
            observe_programdata_failure_v1_instruction,
            ObserveProgramDataFailureV1Accounts {
                payer: 1 => (true, true),
                controller_config: 2 => (false, false),
                protocol_gate: 3 => (false, false),
                primary_proposal: 4 => (false, false),
                programdata_verification: 5 => (false, false),
                target_program: 6 => (false, false),
                target_programdata: 7 => (false, false),
                authority_pda: 8 => (false, false),
                upgradeable_loader: 9 => (false, false),
                failure_observation: 10 => (false, true),
                system_program: 11 => (false, false),
            },
            ObserveProgramDataFailureV1 {
                expected: proposal_expectation(),
                expected_program_owner: key(151),
                expected_program_executable: true,
                expected_program_data_length: 36,
                expected_program_header_present: true,
                expected_linked_programdata: OptionalInstructionPubkeyV1::some(key(152)).unwrap(),
                expected_programdata_owner: key(153),
                expected_programdata_executable: false,
                expected_programdata_data_length: 4_141,
                expected_programdata_header_present: true,
                expected_programdata_slot: 153,
                expected_raw_hash_complete: true,
                expected_raw_programdata_hash: bytes(154),
                expected_capacity: 4_096,
                expected_programdata_authority: OptionalInstructionPubkeyV1::some(key(155))
                    .unwrap(),
                mismatch_class: ProgramDataMismatchClassV1::PayloadLeaf,
                failing_chunk_index: 1,
                expected_leaf_hash: bytes(1),
                proof: merkle_proof(),
            },
        );
    }

    const INSTRUCTION_VECTOR_LENGTHS: [usize; 38] = [
        41, 273, 806, 165, 165, 163, 171, 167, 163, 259, 395, 159, 159, 420, 203, 489, 521, 488,
        186, 154, 149, 149, 149, 157, 151, 147, 163, 461, 232, 297, 391, 530, 316, 252, 362, 240,
        435, 624,
    ];

    const INSTRUCTION_VECTOR_SHA256: [[u8; 32]; 38] = [
        [
            67, 229, 137, 7, 234, 18, 208, 235, 249, 34, 186, 123, 252, 252, 248, 128, 37, 11, 148,
            213, 209, 252, 248, 179, 110, 22, 193, 10, 118, 234, 77, 95,
        ],
        [
            253, 21, 119, 192, 9, 208, 145, 200, 225, 102, 57, 149, 217, 210, 242, 217, 21, 34, 2,
            207, 228, 120, 227, 40, 67, 178, 80, 56, 13, 10, 243, 51,
        ],
        [
            141, 55, 179, 228, 116, 252, 4, 237, 45, 207, 240, 179, 178, 182, 44, 6, 207, 2, 219,
            3, 20, 195, 242, 99, 119, 39, 23, 64, 167, 42, 170, 169,
        ],
        [
            98, 4, 155, 139, 104, 243, 148, 42, 139, 199, 201, 92, 206, 177, 40, 145, 164, 20, 103,
            253, 243, 58, 249, 189, 106, 35, 11, 241, 127, 91, 68, 19,
        ],
        [
            148, 87, 0, 14, 80, 9, 250, 160, 25, 76, 65, 76, 23, 30, 142, 4, 222, 36, 226, 26, 204,
            93, 196, 253, 26, 74, 114, 86, 145, 4, 227, 107,
        ],
        [
            33, 152, 141, 118, 92, 75, 156, 233, 145, 78, 206, 180, 87, 162, 194, 25, 89, 65, 163,
            144, 58, 240, 254, 250, 173, 3, 230, 29, 12, 3, 201, 34,
        ],
        [
            59, 98, 10, 124, 67, 41, 247, 113, 112, 143, 4, 233, 197, 166, 118, 73, 54, 22, 121,
            253, 147, 140, 6, 120, 98, 24, 158, 21, 195, 254, 30, 86,
        ],
        [
            158, 128, 52, 131, 198, 209, 132, 30, 35, 18, 2, 179, 138, 147, 64, 180, 218, 129, 88,
            244, 26, 119, 251, 240, 50, 180, 40, 145, 207, 217, 206, 250,
        ],
        [
            31, 86, 222, 247, 52, 201, 10, 110, 200, 215, 197, 118, 86, 160, 175, 219, 237, 234,
            159, 15, 2, 211, 13, 191, 86, 242, 200, 237, 124, 250, 204, 142,
        ],
        [
            133, 125, 110, 44, 35, 102, 30, 235, 74, 232, 243, 67, 161, 76, 74, 202, 46, 199, 171,
            46, 233, 12, 41, 129, 102, 199, 180, 158, 209, 40, 240, 200,
        ],
        [
            31, 94, 31, 33, 219, 95, 196, 194, 179, 1, 67, 82, 177, 107, 10, 200, 127, 203, 94,
            196, 191, 6, 253, 123, 155, 81, 106, 239, 105, 154, 174, 66,
        ],
        [
            208, 117, 139, 23, 163, 71, 97, 201, 1, 162, 23, 156, 62, 252, 166, 65, 236, 144, 213,
            235, 153, 173, 246, 18, 125, 183, 110, 94, 56, 184, 143, 63,
        ],
        [
            178, 168, 135, 211, 78, 197, 38, 194, 26, 204, 103, 209, 191, 245, 10, 138, 70, 220,
            117, 18, 19, 77, 162, 37, 203, 164, 115, 188, 115, 245, 43, 221,
        ],
        [
            200, 255, 37, 40, 60, 223, 5, 211, 250, 36, 159, 140, 187, 153, 248, 3, 170, 216, 243,
            5, 248, 77, 183, 95, 204, 237, 50, 136, 95, 170, 208, 239,
        ],
        [
            90, 77, 64, 227, 37, 27, 22, 236, 67, 51, 92, 59, 230, 104, 216, 150, 13, 229, 139,
            248, 138, 77, 63, 217, 240, 178, 241, 131, 135, 127, 78, 107,
        ],
        [
            19, 146, 98, 3, 181, 117, 67, 62, 108, 35, 152, 66, 81, 151, 175, 210, 201, 91, 55, 9,
            8, 15, 47, 139, 232, 117, 114, 9, 1, 145, 232, 95,
        ],
        [
            125, 235, 218, 125, 102, 119, 228, 194, 170, 166, 143, 138, 152, 210, 116, 72, 233,
            215, 232, 149, 134, 107, 242, 23, 51, 221, 39, 179, 208, 225, 71, 224,
        ],
        [
            241, 149, 99, 114, 17, 123, 62, 93, 46, 201, 112, 62, 253, 104, 156, 9, 231, 40, 152,
            91, 220, 222, 235, 5, 12, 70, 99, 198, 89, 30, 14, 39,
        ],
        [
            93, 239, 109, 241, 57, 13, 41, 207, 195, 7, 6, 11, 199, 31, 195, 131, 45, 73, 68, 171,
            214, 195, 219, 174, 14, 210, 25, 90, 36, 68, 149, 216,
        ],
        [
            120, 233, 225, 182, 244, 135, 44, 235, 45, 170, 226, 51, 66, 231, 188, 103, 219, 234,
            217, 156, 215, 117, 160, 65, 191, 190, 56, 84, 228, 59, 246, 65,
        ],
        [
            97, 173, 43, 66, 102, 232, 103, 134, 206, 117, 155, 81, 133, 254, 133, 73, 143, 83, 11,
            42, 206, 52, 136, 227, 38, 187, 220, 189, 47, 191, 8, 185,
        ],
        [
            122, 188, 17, 156, 146, 177, 35, 166, 80, 149, 134, 34, 220, 98, 174, 14, 165, 5, 214,
            24, 29, 89, 193, 112, 146, 160, 98, 3, 130, 55, 161, 229,
        ],
        [
            153, 15, 37, 31, 127, 158, 3, 233, 162, 186, 116, 190, 228, 102, 221, 48, 205, 251,
            121, 139, 121, 231, 53, 26, 16, 223, 14, 154, 103, 32, 233, 150,
        ],
        [
            215, 20, 150, 72, 194, 146, 200, 195, 23, 208, 98, 84, 238, 221, 93, 212, 85, 91, 104,
            205, 114, 93, 212, 141, 245, 130, 236, 77, 18, 215, 232, 137,
        ],
        [
            181, 252, 157, 154, 85, 224, 64, 70, 129, 28, 2, 31, 106, 180, 87, 187, 71, 44, 47,
            253, 233, 229, 112, 41, 246, 170, 23, 211, 204, 196, 254, 180,
        ],
        [
            106, 61, 56, 70, 8, 5, 191, 11, 208, 154, 223, 235, 70, 145, 92, 252, 226, 219, 119,
            153, 242, 24, 191, 249, 211, 187, 215, 63, 55, 140, 201, 217,
        ],
        [
            201, 67, 33, 61, 151, 39, 175, 100, 132, 9, 254, 186, 85, 66, 206, 137, 123, 200, 22,
            11, 162, 237, 159, 117, 232, 140, 172, 0, 16, 246, 131, 134,
        ],
        [
            63, 176, 206, 129, 166, 153, 21, 84, 209, 157, 228, 132, 156, 190, 213, 255, 200, 185,
            184, 251, 202, 63, 129, 157, 227, 194, 90, 83, 197, 137, 7, 235,
        ],
        [
            163, 207, 103, 2, 84, 150, 209, 8, 222, 175, 62, 210, 248, 3, 186, 3, 69, 52, 56, 251,
            228, 36, 132, 140, 90, 158, 105, 25, 117, 105, 90, 234,
        ],
        [
            36, 86, 85, 253, 112, 25, 1, 213, 77, 109, 205, 43, 34, 111, 40, 21, 24, 44, 233, 164,
            160, 61, 152, 112, 211, 149, 120, 8, 50, 44, 216, 26,
        ],
        [
            39, 7, 204, 118, 103, 94, 189, 13, 66, 114, 146, 141, 51, 105, 189, 96, 20, 69, 82,
            242, 59, 66, 195, 220, 230, 8, 36, 171, 114, 11, 238, 209,
        ],
        [
            195, 54, 156, 60, 158, 202, 113, 209, 101, 29, 248, 182, 160, 80, 23, 211, 223, 232,
            251, 66, 39, 12, 140, 170, 85, 242, 85, 36, 171, 105, 232, 82,
        ],
        [
            28, 169, 0, 119, 58, 162, 231, 27, 26, 205, 141, 248, 223, 248, 31, 5, 166, 105, 191,
            22, 106, 208, 202, 180, 24, 225, 23, 131, 191, 231, 142, 36,
        ],
        [
            177, 145, 17, 140, 72, 232, 214, 92, 31, 57, 109, 122, 7, 120, 143, 37, 12, 69, 131,
            223, 157, 105, 167, 80, 129, 134, 226, 37, 156, 123, 171, 94,
        ],
        [
            53, 229, 107, 36, 121, 250, 75, 35, 87, 246, 172, 177, 145, 63, 107, 71, 184, 212, 60,
            229, 2, 127, 191, 207, 247, 106, 194, 218, 122, 167, 83, 11,
        ],
        [
            178, 68, 147, 41, 194, 99, 11, 174, 96, 61, 69, 129, 215, 68, 193, 207, 136, 31, 93,
            42, 123, 214, 194, 175, 236, 192, 1, 202, 196, 227, 8, 118,
        ],
        [
            167, 158, 201, 63, 24, 115, 26, 25, 235, 176, 141, 182, 248, 170, 83, 208, 63, 219, 28,
            191, 38, 254, 252, 237, 187, 88, 113, 63, 114, 57, 198, 188,
        ],
        [
            162, 119, 131, 90, 32, 241, 154, 127, 123, 179, 140, 231, 58, 178, 122, 1, 52, 35, 211,
            193, 2, 14, 194, 183, 79, 6, 227, 219, 244, 192, 241, 80,
        ],
    ];

    #[test]
    fn instruction_wire_vectors_are_frozen() {
        let cases = instruction_cases();
        assert_eq!(cases.len(), INSTRUCTION_VECTOR_LENGTHS.len());
        for (index, (tag, len, packed)) in cases.into_iter().enumerate() {
            let expected_tag = if index < 26 { index } else { index + 1 };
            assert_eq!(usize::from(tag), expected_tag);
            assert_eq!(len, INSTRUCTION_VECTOR_LENGTHS[index]);
            assert_eq!(packed.len(), INSTRUCTION_VECTOR_LENGTHS[index]);
            assert_eq!(hash(&packed).to_bytes(), INSTRUCTION_VECTOR_SHA256[index]);
        }
    }
}
