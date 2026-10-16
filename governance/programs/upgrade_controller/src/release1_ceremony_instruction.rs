//! Closed fixed-wire instructions for scalable ProgramData observation.
//!
//! These codecs intentionally contain no caller-supplied ProgramData bytes or
//! raw leaf hashes. The processor reads every committed byte directly from the
//! canonical Loader-v3 ProgramData account.

use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    artifact_merkle::{
        ARTIFACT_MERKLE_SCHEME_ID, MAX_ARTIFACT_BYTES_V1, MAX_ARTIFACT_PROOF_DEPTH_V1,
    },
    release1_ceremony_state::{
        ProgramDataObservationPurposeV1, ProgramDataObservationStatusV1,
        MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1,
    },
    state::GateStatusV1,
};

pub const BEGIN_PROGRAMDATA_OBSERVATION_V1_TAG: u8 = 39;
pub const APPEND_PROGRAMDATA_OBSERVATION_CHUNK_V1_TAG: u8 = 40;
pub const VERIFY_OBSERVED_ARTIFACT_CHUNK_V1_TAG: u8 = 41;
pub const FINALIZE_PROGRAMDATA_OBSERVATION_V1_TAG: u8 = 42;

pub const BEGIN_PROGRAMDATA_OBSERVATION_V1_ACCOUNT_COUNT: usize = 10;
pub const PROGRAMDATA_OBSERVATION_STEP_V1_ACCOUNT_COUNT: usize = 8;
pub const MAX_OBSERVATION_INSTRUCTION_DATA_LEN_V1: usize = 16_384;
pub const MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1: usize = MAX_ARTIFACT_PROOF_DEPTH_V1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationAuthorityV1 {
    pub present: bool,
    pub value: Pubkey,
}

impl ObservationAuthorityV1 {
    pub const LEN: usize = 33;

    pub const fn none() -> Self {
        Self {
            present: false,
            value: Pubkey::new_from_array([0; 32]),
        }
    }

    pub fn some(value: Pubkey) -> Result<Self, ProgramError> {
        if value == Pubkey::default() {
            Err(ProgramError::InvalidInstructionData)
        } else {
            Ok(Self {
                present: true,
                value,
            })
        }
    }

    pub fn validate(&self) -> Result<(), ProgramError> {
        match (self.present, self.value == Pubkey::default()) {
            (false, true) | (true, false) => Ok(()),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedArtifactMerkleProofV1 {
    pub proof_len: u8,
    pub nodes: [[u8; 32]; MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1],
}

impl ObservedArtifactMerkleProofV1 {
    pub const LEN: usize = 1 + 32 * MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1;

    pub const fn empty() -> Self {
        Self {
            proof_len: 0,
            nodes: [[0; 32]; MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1],
        }
    }

    pub fn validate(&self) -> Result<(), ProgramError> {
        let used = usize::from(self.proof_len);
        if used > MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1
            || self.nodes[used..].iter().any(|node| *node != [0; 32])
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramDataObservationGuardV1 {
    pub purpose: ProgramDataObservationPurposeV1,
    pub generation: u64,
    pub expected_subject_digest: [u8; 32],
    pub expected_gate_status: GateStatusV1,
    pub expected_gate_epoch: u64,
    pub expected_freeze_reason_code: u16,
    pub expected_freeze_slot: u64,
}

impl ProgramDataObservationGuardV1 {
    pub const LEN: usize = 1 + 8 + 32 + 1 + 8 + 2 + 8;

    pub fn validate(&self) -> Result<(), ProgramError> {
        if self.generation == 0
            || self.expected_subject_digest == [0; 32]
            || self.expected_gate_epoch == 0
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        let active = self.expected_freeze_reason_code == 0 && self.expected_freeze_slot == 0;
        let frozen = self.expected_freeze_reason_code != 0 && self.expected_freeze_slot != 0;
        if match self.expected_gate_status {
            GateStatusV1::Active => !active,
            GateStatusV1::FrozenForUpgrade | GateStatusV1::EmergencyFrozen => !frozen,
        } {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeginProgramDataObservationV1 {
    pub guard: ProgramDataObservationGuardV1,
    pub expected_capacity_policy_digest: [u8; 32],
    pub expected_artifact_length: u64,
    pub expected_artifact_sha256: [u8; 32],
    pub expected_artifact_merkle_root: [u8; 32],
    pub expected_artifact_scheme_id: [u8; 32],
    pub minimum_required_capacity: u64,
    pub expected_deployed_slot: u64,
    pub expected_actual_capacity: u64,
    pub expected_upgrade_authority: ObservationAuthorityV1,
}

impl BeginProgramDataObservationV1 {
    pub const LEN: usize = 1
        + ProgramDataObservationGuardV1::LEN
        + 32
        + 8
        + 32
        + 32
        + 32
        + 8
        + 8
        + 8
        + ObservationAuthorityV1::LEN;

    pub fn validate(&self) -> Result<(), ProgramError> {
        self.guard.validate()?;
        self.expected_upgrade_authority.validate()?;
        if self.expected_capacity_policy_digest == [0; 32]
            || self.expected_artifact_length == 0
            || self.expected_artifact_length > MAX_ARTIFACT_BYTES_V1
            || self.expected_artifact_sha256 == [0; 32]
            || self.expected_artifact_merkle_root == [0; 32]
            || self.expected_artifact_scheme_id != ARTIFACT_MERKLE_SCHEME_ID
            || self.minimum_required_capacity < self.expected_artifact_length
            || self.minimum_required_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
            || self.expected_deployed_slot == 0
            || self.expected_actual_capacity < self.minimum_required_capacity
            || self.expected_actual_capacity > MAX_PROGRAMDATA_PAYLOAD_CAPACITY_V1
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() != Self::LEN || data.first() != Some(&BEGIN_PROGRAMDATA_OBSERVATION_V1_TAG) {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut reader = FixedReader::new(&data[1..]);
        let value = Self {
            guard: decode_guard(&mut reader)?,
            expected_capacity_policy_digest: reader.take::<32>()?,
            expected_artifact_length: reader.u64()?,
            expected_artifact_sha256: reader.take::<32>()?,
            expected_artifact_merkle_root: reader.take::<32>()?,
            expected_artifact_scheme_id: reader.take::<32>()?,
            minimum_required_capacity: reader.u64()?,
            expected_deployed_slot: reader.u64()?,
            expected_actual_capacity: reader.u64()?,
            expected_upgrade_authority: reader.authority()?,
        };
        reader.finish()?;
        value.validate()?;
        Ok(value)
    }

    pub fn pack(&self) -> Result<[u8; Self::LEN], ProgramError> {
        self.validate()?;
        let mut data = [0u8; Self::LEN];
        data[0] = BEGIN_PROGRAMDATA_OBSERVATION_V1_TAG;
        let mut writer = FixedWriter::new(&mut data[1..]);
        encode_guard(&mut writer, &self.guard);
        writer.put(&self.expected_capacity_policy_digest);
        writer.u64(self.expected_artifact_length);
        writer.put(&self.expected_artifact_sha256);
        writer.put(&self.expected_artifact_merkle_root);
        writer.put(&self.expected_artifact_scheme_id);
        writer.u64(self.minimum_required_capacity);
        writer.u64(self.expected_deployed_slot);
        writer.u64(self.expected_actual_capacity);
        writer.authority(&self.expected_upgrade_authority);
        writer.finish()?;
        Ok(data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendProgramDataObservationChunkV1 {
    pub guard: ProgramDataObservationGuardV1,
    pub expected_status: ProgramDataObservationStatusV1,
    pub chunk_index: u32,
}

impl AppendProgramDataObservationChunkV1 {
    pub const LEN: usize = 1 + ProgramDataObservationGuardV1::LEN + 1 + 4;

    pub fn validate(&self) -> Result<(), ProgramError> {
        self.guard.validate()?;
        if self.expected_status != ProgramDataObservationStatusV1::Accumulating {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() != Self::LEN
            || data.first() != Some(&APPEND_PROGRAMDATA_OBSERVATION_CHUNK_V1_TAG)
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut reader = FixedReader::new(&data[1..]);
        let value = Self {
            guard: decode_guard(&mut reader)?,
            expected_status: reader.observation_status()?,
            chunk_index: reader.u32()?,
        };
        reader.finish()?;
        value.validate()?;
        Ok(value)
    }

    pub fn pack(&self) -> Result<[u8; Self::LEN], ProgramError> {
        self.validate()?;
        let mut data = [0u8; Self::LEN];
        data[0] = APPEND_PROGRAMDATA_OBSERVATION_CHUNK_V1_TAG;
        let mut writer = FixedWriter::new(&mut data[1..]);
        encode_guard(&mut writer, &self.guard);
        writer.u8(self.expected_status as u8);
        writer.u32(self.chunk_index);
        writer.finish()?;
        Ok(data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyObservedArtifactChunkV1 {
    pub guard: ProgramDataObservationGuardV1,
    pub expected_status: ProgramDataObservationStatusV1,
    pub chunk_index: u32,
    pub expected_next_artifact_chunk_index: u32,
    pub expected_tail_bytes_verified: u64,
    pub proof: ObservedArtifactMerkleProofV1,
}

impl VerifyObservedArtifactChunkV1 {
    pub const LEN: usize =
        1 + ProgramDataObservationGuardV1::LEN + 1 + 4 + 4 + 8 + ObservedArtifactMerkleProofV1::LEN;

    pub fn validate(&self) -> Result<(), ProgramError> {
        self.guard.validate()?;
        self.proof.validate()?;
        if self.expected_status != ProgramDataObservationStatusV1::Accumulating {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() != Self::LEN || data.first() != Some(&VERIFY_OBSERVED_ARTIFACT_CHUNK_V1_TAG) {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut reader = FixedReader::new(&data[1..]);
        let value = Self {
            guard: decode_guard(&mut reader)?,
            expected_status: reader.observation_status()?,
            chunk_index: reader.u32()?,
            expected_next_artifact_chunk_index: reader.u32()?,
            expected_tail_bytes_verified: reader.u64()?,
            proof: reader.proof()?,
        };
        reader.finish()?;
        value.validate()?;
        Ok(value)
    }

    pub fn pack(&self) -> Result<[u8; Self::LEN], ProgramError> {
        self.validate()?;
        let mut data = [0u8; Self::LEN];
        data[0] = VERIFY_OBSERVED_ARTIFACT_CHUNK_V1_TAG;
        let mut writer = FixedWriter::new(&mut data[1..]);
        encode_guard(&mut writer, &self.guard);
        writer.u8(self.expected_status as u8);
        writer.u32(self.chunk_index);
        writer.u32(self.expected_next_artifact_chunk_index);
        writer.u64(self.expected_tail_bytes_verified);
        writer.proof(&self.proof);
        writer.finish()?;
        Ok(data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalizeProgramDataObservationV1 {
    pub guard: ProgramDataObservationGuardV1,
    pub expected_status: ProgramDataObservationStatusV1,
    pub expected_next_raw_chunk_index: u32,
    pub expected_next_artifact_chunk_index: u32,
    pub expected_tail_bytes_verified: u64,
}

impl FinalizeProgramDataObservationV1 {
    pub const LEN: usize = 1 + ProgramDataObservationGuardV1::LEN + 1 + 4 + 4 + 8;

    pub fn validate(&self) -> Result<(), ProgramError> {
        self.guard.validate()?;
        if self.expected_status != ProgramDataObservationStatusV1::ReadyToFinalize {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() != Self::LEN || data.first() != Some(&FINALIZE_PROGRAMDATA_OBSERVATION_V1_TAG)
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        let mut reader = FixedReader::new(&data[1..]);
        let value = Self {
            guard: decode_guard(&mut reader)?,
            expected_status: reader.observation_status()?,
            expected_next_raw_chunk_index: reader.u32()?,
            expected_next_artifact_chunk_index: reader.u32()?,
            expected_tail_bytes_verified: reader.u64()?,
        };
        reader.finish()?;
        value.validate()?;
        Ok(value)
    }

    pub fn pack(&self) -> Result<[u8; Self::LEN], ProgramError> {
        self.validate()?;
        let mut data = [0u8; Self::LEN];
        data[0] = FINALIZE_PROGRAMDATA_OBSERVATION_V1_TAG;
        let mut writer = FixedWriter::new(&mut data[1..]);
        encode_guard(&mut writer, &self.guard);
        writer.u8(self.expected_status as u8);
        writer.u32(self.expected_next_raw_chunk_index);
        writer.u32(self.expected_next_artifact_chunk_index);
        writer.u64(self.expected_tail_bytes_verified);
        writer.finish()?;
        Ok(data)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn begin_programdata_observation_instruction(
    controller_program: Pubkey,
    payer: Pubkey,
    controller_config: Pubkey,
    protocol_gate: Pubkey,
    capacity_policy: Pubkey,
    subject: Pubkey,
    observed_program: Pubkey,
    observed_programdata: Pubkey,
    observation: Pubkey,
    upgradeable_loader: Pubkey,
    system_program: Pubkey,
    instruction: BeginProgramDataObservationV1,
) -> Result<Instruction, ProgramError> {
    Ok(Instruction {
        program_id: controller_program,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(controller_config, false),
            AccountMeta::new_readonly(protocol_gate, false),
            AccountMeta::new_readonly(capacity_policy, false),
            AccountMeta::new_readonly(subject, false),
            AccountMeta::new_readonly(observed_program, false),
            AccountMeta::new_readonly(observed_programdata, false),
            AccountMeta::new(observation, false),
            AccountMeta::new_readonly(upgradeable_loader, false),
            AccountMeta::new_readonly(system_program, false),
        ],
        data: instruction.pack()?.to_vec(),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn append_programdata_observation_chunk_instruction(
    controller_program: Pubkey,
    controller_config: Pubkey,
    protocol_gate: Pubkey,
    capacity_policy: Pubkey,
    subject: Pubkey,
    observed_program: Pubkey,
    observed_programdata: Pubkey,
    observation: Pubkey,
    upgradeable_loader: Pubkey,
    instruction: AppendProgramDataObservationChunkV1,
) -> Result<Instruction, ProgramError> {
    observation_step_instruction(
        controller_program,
        controller_config,
        protocol_gate,
        capacity_policy,
        subject,
        observed_program,
        observed_programdata,
        observation,
        upgradeable_loader,
        instruction.pack()?.to_vec(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn verify_observed_artifact_chunk_instruction(
    controller_program: Pubkey,
    controller_config: Pubkey,
    protocol_gate: Pubkey,
    capacity_policy: Pubkey,
    subject: Pubkey,
    observed_program: Pubkey,
    observed_programdata: Pubkey,
    observation: Pubkey,
    upgradeable_loader: Pubkey,
    instruction: VerifyObservedArtifactChunkV1,
) -> Result<Instruction, ProgramError> {
    observation_step_instruction(
        controller_program,
        controller_config,
        protocol_gate,
        capacity_policy,
        subject,
        observed_program,
        observed_programdata,
        observation,
        upgradeable_loader,
        instruction.pack()?.to_vec(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_programdata_observation_instruction(
    controller_program: Pubkey,
    controller_config: Pubkey,
    protocol_gate: Pubkey,
    capacity_policy: Pubkey,
    subject: Pubkey,
    observed_program: Pubkey,
    observed_programdata: Pubkey,
    observation: Pubkey,
    upgradeable_loader: Pubkey,
    instruction: FinalizeProgramDataObservationV1,
) -> Result<Instruction, ProgramError> {
    observation_step_instruction(
        controller_program,
        controller_config,
        protocol_gate,
        capacity_policy,
        subject,
        observed_program,
        observed_programdata,
        observation,
        upgradeable_loader,
        instruction.pack()?.to_vec(),
    )
}

#[allow(clippy::too_many_arguments)]
fn observation_step_instruction(
    controller_program: Pubkey,
    controller_config: Pubkey,
    protocol_gate: Pubkey,
    capacity_policy: Pubkey,
    subject: Pubkey,
    observed_program: Pubkey,
    observed_programdata: Pubkey,
    observation: Pubkey,
    upgradeable_loader: Pubkey,
    data: Vec<u8>,
) -> Result<Instruction, ProgramError> {
    if data.len() > MAX_OBSERVATION_INSTRUCTION_DATA_LEN_V1 {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(Instruction {
        program_id: controller_program,
        accounts: vec![
            AccountMeta::new_readonly(controller_config, false),
            AccountMeta::new_readonly(protocol_gate, false),
            AccountMeta::new_readonly(capacity_policy, false),
            AccountMeta::new_readonly(subject, false),
            AccountMeta::new_readonly(observed_program, false),
            AccountMeta::new_readonly(observed_programdata, false),
            AccountMeta::new(observation, false),
            AccountMeta::new_readonly(upgradeable_loader, false),
        ],
        data,
    })
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
        let mut out = [0; N];
        out.copy_from_slice(source);
        self.offset = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, ProgramError> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, ProgramError> {
        Ok(u16::from_le_bytes(self.take::<2>()?))
    }

    fn u32(&mut self) -> Result<u32, ProgramError> {
        Ok(u32::from_le_bytes(self.take::<4>()?))
    }

    fn u64(&mut self) -> Result<u64, ProgramError> {
        Ok(u64::from_le_bytes(self.take::<8>()?))
    }

    fn authority(&mut self) -> Result<ObservationAuthorityV1, ProgramError> {
        let present = match self.u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProgramError::InvalidInstructionData),
        };
        let value = Pubkey::new_from_array(self.take::<32>()?);
        let authority = ObservationAuthorityV1 { present, value };
        authority.validate()?;
        Ok(authority)
    }

    fn purpose(&mut self) -> Result<ProgramDataObservationPurposeV1, ProgramError> {
        match self.u8()? {
            0 => Ok(ProgramDataObservationPurposeV1::ControllerImmutability),
            1 => Ok(ProgramDataObservationPurposeV1::TargetHandoffBridge),
            2 => Ok(ProgramDataObservationPurposeV1::ProposalPrestate),
            3 => Ok(ProgramDataObservationPurposeV1::PostUpgrade),
            4 => Ok(ProgramDataObservationPurposeV1::Rollback),
            5 => Ok(ProgramDataObservationPurposeV1::EmergencyResolution),
            6 => Ok(ProgramDataObservationPurposeV1::BootstrapActivation),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }

    fn gate_status(&mut self) -> Result<GateStatusV1, ProgramError> {
        match self.u8()? {
            0 => Ok(GateStatusV1::Active),
            1 => Ok(GateStatusV1::FrozenForUpgrade),
            2 => Ok(GateStatusV1::EmergencyFrozen),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }

    fn observation_status(&mut self) -> Result<ProgramDataObservationStatusV1, ProgramError> {
        match self.u8()? {
            0 => Ok(ProgramDataObservationStatusV1::Accumulating),
            1 => Ok(ProgramDataObservationStatusV1::ReadyToFinalize),
            2 => Ok(ProgramDataObservationStatusV1::Finalized),
            3 => Ok(ProgramDataObservationStatusV1::Stale),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }

    fn proof(&mut self) -> Result<ObservedArtifactMerkleProofV1, ProgramError> {
        let proof_len = self.u8()?;
        let mut nodes = [[0; 32]; MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1];
        for node in &mut nodes {
            *node = self.take::<32>()?;
        }
        let proof = ObservedArtifactMerkleProofV1 { proof_len, nodes };
        proof.validate()?;
        Ok(proof)
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

    fn put(&mut self, bytes: &[u8]) {
        let end = self.offset + bytes.len();
        self.bytes[self.offset..end].copy_from_slice(bytes);
        self.offset = end;
    }

    fn u8(&mut self, value: u8) {
        self.put(&[value]);
    }

    fn u16(&mut self, value: u16) {
        self.put(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    fn authority(&mut self, authority: &ObservationAuthorityV1) {
        self.u8(u8::from(authority.present));
        self.put(authority.value.as_ref());
    }

    fn proof(&mut self, proof: &ObservedArtifactMerkleProofV1) {
        self.u8(proof.proof_len);
        for node in &proof.nodes {
            self.put(node);
        }
    }

    fn finish(self) -> Result<(), ProgramError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }
}

fn decode_guard(
    reader: &mut FixedReader<'_>,
) -> Result<ProgramDataObservationGuardV1, ProgramError> {
    let guard = ProgramDataObservationGuardV1 {
        purpose: reader.purpose()?,
        generation: reader.u64()?,
        expected_subject_digest: reader.take::<32>()?,
        expected_gate_status: reader.gate_status()?,
        expected_gate_epoch: reader.u64()?,
        expected_freeze_reason_code: reader.u16()?,
        expected_freeze_slot: reader.u64()?,
    };
    guard.validate()?;
    Ok(guard)
}

fn encode_guard(writer: &mut FixedWriter<'_>, guard: &ProgramDataObservationGuardV1) {
    writer.u8(guard.purpose as u8);
    writer.u64(guard.generation);
    writer.put(&guard.expected_subject_digest);
    writer.u8(guard.expected_gate_status as u8);
    writer.u64(guard.expected_gate_epoch);
    writer.u16(guard.expected_freeze_reason_code);
    writer.u64(guard.expected_freeze_slot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk_ids::system_program;

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn guard() -> ProgramDataObservationGuardV1 {
        ProgramDataObservationGuardV1 {
            purpose: ProgramDataObservationPurposeV1::TargetHandoffBridge,
            generation: 1,
            expected_subject_digest: [2; 32],
            expected_gate_status: GateStatusV1::EmergencyFrozen,
            expected_gate_epoch: 1,
            expected_freeze_reason_code: 1,
            expected_freeze_slot: 3,
        }
    }

    fn begin() -> BeginProgramDataObservationV1 {
        BeginProgramDataObservationV1 {
            guard: guard(),
            expected_capacity_policy_digest: [4; 32],
            expected_artifact_length: 16_384,
            expected_artifact_sha256: [5; 32],
            expected_artifact_merkle_root: [6; 32],
            expected_artifact_scheme_id: ARTIFACT_MERKLE_SCHEME_ID,
            minimum_required_capacity: 16_384,
            expected_deployed_slot: 7,
            expected_actual_capacity: 32_768,
            expected_upgrade_authority: ObservationAuthorityV1::some(key(8)).unwrap(),
        }
    }

    fn append() -> AppendProgramDataObservationChunkV1 {
        AppendProgramDataObservationChunkV1 {
            guard: guard(),
            expected_status: ProgramDataObservationStatusV1::Accumulating,
            chunk_index: 0,
        }
    }

    fn proof() -> ObservedArtifactMerkleProofV1 {
        let mut nodes = [[0; 32]; MAX_OBSERVED_ARTIFACT_PROOF_NODES_V1];
        nodes[0] = [9; 32];
        ObservedArtifactMerkleProofV1 {
            proof_len: 1,
            nodes,
        }
    }

    fn verify() -> VerifyObservedArtifactChunkV1 {
        VerifyObservedArtifactChunkV1 {
            guard: guard(),
            expected_status: ProgramDataObservationStatusV1::Accumulating,
            chunk_index: 0,
            expected_next_artifact_chunk_index: 0,
            expected_tail_bytes_verified: 0,
            proof: proof(),
        }
    }

    fn finalize() -> FinalizeProgramDataObservationV1 {
        FinalizeProgramDataObservationV1 {
            guard: guard(),
            expected_status: ProgramDataObservationStatusV1::ReadyToFinalize,
            expected_next_raw_chunk_index: 3,
            expected_next_artifact_chunk_index: 1,
            expected_tail_bytes_verified: 16_384,
        }
    }

    macro_rules! strict_codec {
        ($value:expr, $type:ty) => {{
            let value = $value;
            let encoded = value.pack().unwrap();
            assert_eq!(encoded.len(), <$type>::LEN);
            assert_eq!(<$type>::unpack(&encoded).unwrap(), value);
            for len in 0..encoded.len() {
                assert!(<$type>::unpack(&encoded[..len]).is_err(), "prefix {len}");
            }
            let mut trailing = encoded.to_vec();
            trailing.push(0);
            assert!(<$type>::unpack(&trailing).is_err());
        }};
    }

    #[test]
    fn all_codecs_are_exact_and_reject_every_truncation_or_trailing_byte() {
        strict_codec!(begin(), BeginProgramDataObservationV1);
        strict_codec!(append(), AppendProgramDataObservationChunkV1);
        strict_codec!(verify(), VerifyObservedArtifactChunkV1);
        strict_codec!(finalize(), FinalizeProgramDataObservationV1);
    }

    #[test]
    fn tags_are_unique_and_cross_tag_decode_fails() {
        let tags = [
            BEGIN_PROGRAMDATA_OBSERVATION_V1_TAG,
            APPEND_PROGRAMDATA_OBSERVATION_CHUNK_V1_TAG,
            VERIFY_OBSERVED_ARTIFACT_CHUNK_V1_TAG,
            FINALIZE_PROGRAMDATA_OBSERVATION_V1_TAG,
        ];
        let mut unique = tags.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), tags.len());
        let mut encoded = append().pack().unwrap();
        encoded[0] = BEGIN_PROGRAMDATA_OBSERVATION_V1_TAG;
        assert!(AppendProgramDataObservationChunkV1::unpack(&encoded).is_err());
    }

    #[test]
    fn enums_options_and_proof_padding_are_strict() {
        let mut encoded = begin().pack().unwrap();
        encoded[1] = 7;
        assert!(BeginProgramDataObservationV1::unpack(&encoded).is_err());

        let mut encoded = begin().pack().unwrap();
        let authority_tag_offset = BeginProgramDataObservationV1::LEN - ObservationAuthorityV1::LEN;
        encoded[authority_tag_offset] = 2;
        assert!(BeginProgramDataObservationV1::unpack(&encoded).is_err());

        let mut invalid = proof();
        invalid.nodes[2] = [1; 32];
        assert_eq!(
            invalid.validate(),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn builders_expose_only_the_closed_account_contracts() {
        let controller = key(1);
        let begin_ix = begin_programdata_observation_instruction(
            controller,
            key(2),
            key(3),
            key(4),
            key(5),
            key(6),
            key(7),
            key(8),
            key(9),
            key(10),
            system_program::ID,
            begin(),
        )
        .unwrap();
        assert_eq!(
            begin_ix.accounts.len(),
            BEGIN_PROGRAMDATA_OBSERVATION_V1_ACCOUNT_COUNT
        );
        assert!(begin_ix.accounts[0].is_signer && begin_ix.accounts[0].is_writable);
        assert!(begin_ix.accounts[7].is_writable);
        assert!(begin_ix.accounts[1..7]
            .iter()
            .all(|meta| !meta.is_signer && !meta.is_writable));

        let step = append_programdata_observation_chunk_instruction(
            controller,
            key(3),
            key(4),
            key(5),
            key(6),
            key(7),
            key(8),
            key(9),
            key(10),
            append(),
        )
        .unwrap();
        assert_eq!(
            step.accounts.len(),
            PROGRAMDATA_OBSERVATION_STEP_V1_ACCOUNT_COUNT
        );
        assert_eq!(
            step.accounts.iter().filter(|meta| meta.is_writable).count(),
            1
        );
        assert!(step.accounts[6].is_writable);
        assert!(step.accounts.iter().all(|meta| !meta.is_signer));
    }
}
