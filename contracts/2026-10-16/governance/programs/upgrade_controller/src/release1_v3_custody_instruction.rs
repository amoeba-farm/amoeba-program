//! Closed fixed-wire instructions for the capacity-safe V3 custody path.
//!
//! Tags 75 through 81 extend the V3 lifecycle without exposing caller-selected
//! CPI bytes, program IDs, account vectors, or dynamic collections.  Every
//! Loader operation is selected by its concrete processor and has one fixed
//! account contract.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::{
    instruction::MAX_FIXED_MERKLE_PROOF_NODES_V1,
    release1_authority_instruction::CeremonyEnvelopeV1,
    release1_state::{BufferVerificationStatusV1, VERIFICATION_BITMAP_BYTES_V1},
    release1_v3_instruction::{ProposalGuardV3, MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN},
};

pub const ADOPT_BUFFER_V2_TAG: u8 = 75;
pub const VERIFY_BUFFER_CHUNK_V2_TAG: u8 = 76;
pub const FINALIZE_BUFFER_VERIFICATION_V2_TAG: u8 = 77;
pub const EXTEND_TARGET_V2_TAG: u8 = 78;
pub const EXECUTE_UPGRADE_V2_TAG: u8 = 79;
pub const CLOSE_ABANDONED_BUFFER_V2_TAG: u8 = 80;
pub const ACTIVATE_ROLLBACK_V2_TAG: u8 = 81;

pub const ADOPT_BUFFER_V2_ACCOUNT_COUNT: usize = 12;
pub const VERIFY_BUFFER_CHUNK_V2_ACCOUNT_COUNT: usize = 7;
pub const FINALIZE_BUFFER_VERIFICATION_V2_ACCOUNT_COUNT: usize = 7;
pub const EXTEND_TARGET_V2_ACCOUNT_COUNT: usize = 15;
pub const EXECUTE_UPGRADE_V2_ACCOUNT_COUNT: usize = 21;
pub const CLOSE_ABANDONED_BUFFER_V2_ACCOUNT_COUNT: usize = 8;
pub const ACTIVATE_ROLLBACK_V2_ACCOUNT_COUNT: usize = 15;

/// Canonically zero-padded bounded artifact proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct ArtifactChunkProofV2 {
    pub proof_len: u8,
    pub nodes: [[u8; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1],
}

impl ArtifactChunkProofV2 {
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
}

trait WireValidate {
    fn validate_wire(&self) -> Result<(), ProgramError>;
}

fn validate_proposal_guard(guard: &ProposalGuardV3) -> Result<(), ProgramError> {
    if guard.expected_proposal_digest == [0; 32]
        || guard.expected_gate_epoch == 0
        || guard.expected_target_nonce == 0
        || guard.expected_capacity_policy_digest == [0; 32]
        || guard.expected_current_deployment_digest == [0; 32]
        || guard.expected_current_deployment_generation == 0
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

fn require_nonzero_hashes(values: &[[u8; 32]]) -> Result<(), ProgramError> {
    if values.contains(&[0; 32]) {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

macro_rules! fixed_instruction {
    ($name:ident, $tag:ident, $payload_len:expr, { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
        pub struct $name {
            $(pub $field: $ty,)*
        }

        impl $name {
            pub const TAG: u8 = $tag;
            pub const PAYLOAD_LEN: usize = $payload_len;
            pub const LEN: usize = 1 + Self::PAYLOAD_LEN;

            pub fn pack(&self) -> Result<Vec<u8>, ProgramError> {
                self.validate_wire()?;
                let payload = self
                    .try_to_vec()
                    .map_err(|_| ProgramError::InvalidInstructionData)?;
                if payload.len() != Self::PAYLOAD_LEN
                    || Self::LEN > MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let mut data = Vec::with_capacity(Self::LEN);
                data.push(Self::TAG);
                data.extend_from_slice(&payload);
                Ok(data)
            }

            pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
                if data.len() != Self::LEN
                    || data.len() > MAX_RELEASE1_V3_INSTRUCTION_DATA_LEN
                    || data.first().copied() != Some(Self::TAG)
                {
                    return Err(ProgramError::InvalidInstructionData);
                }
                let value = Self::try_from_slice(&data[1..])
                    .map_err(|_| ProgramError::InvalidInstructionData)?;
                value.validate_wire()?;
                Ok(value)
            }
        }
    };
}

fixed_instruction!(AdoptBufferV2, ADOPT_BUFFER_V2_TAG, 122, {
    expected: ProposalGuardV3
});

fixed_instruction!(VerifyBufferChunkV2, VERIFY_BUFFER_CHUNK_V2_TAG, 420, {
    expected: ProposalGuardV3,
    chunk_index: u32,
    proof: ArtifactChunkProofV2,
    expected_verification_status: BufferVerificationStatusV1,
    expected_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    expected_verified_chunk_count: u32
});

fixed_instruction!(FinalizeBufferVerificationV2, FINALIZE_BUFFER_VERIFICATION_V2_TAG, 223, {
    expected: ProposalGuardV3,
    expected_verification_status: BufferVerificationStatusV1,
    expected_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    expected_verified_chunk_count: u32,
    expected_sealed_buffer_header_hash: [u8; 32]
});

fixed_instruction!(ExtendTargetV2, EXTEND_TARGET_V2_TAG, 384, {
    expected: ProposalGuardV3,
    expected_prestate_checkpoint_digest: [u8; 32],
    expected_prestate_checkpoint_generation: u64,
    expected_observation_digest: [u8; 32],
    expected_observation_generation: u64,
    expected_observation_root: [u8; 32],
    expected_observation_finalized_slot: u64,
    expected_current_capacity: u64,
    expected_extension_delta: u64,
    expected_post_capacity: u64,
    expected_next_deployment_generation: u64,
    expected_next_deployment_digest: [u8; 32],
    envelope: CeremonyEnvelopeV1
});

fixed_instruction!(ExecuteUpgradeV2, EXECUTE_UPGRADE_V2_TAG, 398, {
    expected: ProposalGuardV3,
    expected_prestate_checkpoint_digest: [u8; 32],
    expected_prestate_checkpoint_generation: u64,
    expected_observation_digest: [u8; 32],
    expected_observation_generation: u64,
    expected_observation_root: [u8; 32],
    expected_observation_finalized_slot: u64,
    expected_actual_capacity: u64,
    expected_sealed_buffer_header_hash: [u8; 32],
    expected_verified_chunk_count: u32,
    expected_buffer_verification_status: BufferVerificationStatusV1,
    expected_counterpart_proposal_digest: [u8; 32],
    expected_counterpart_buffer_verification_status: BufferVerificationStatusV1,
    envelope: CeremonyEnvelopeV1
});

fixed_instruction!(CloseAbandonedBufferV2, CLOSE_ABANDONED_BUFFER_V2_TAG, 199, {
    expected: ProposalGuardV3,
    expected_verification_status: BufferVerificationStatusV1,
    expected_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    expected_verified_chunk_count: u32,
    expected_buffer_verification_finalized_slot: u64
});

fixed_instruction!(ActivateRollbackV2, ACTIVATE_ROLLBACK_V2_TAG, 409, {
    expected_primary: ProposalGuardV3,
    expected_rollback: ProposalGuardV3,
    expected_failure_evidence_digest: [u8; 32],
    expected_primary_verification_generation: u64,
    expected_programdata_observation_state_hash: [u8; 32],
    expected_programdata_observation_generation: u64,
    expected_rollback_buffer_verification_status: BufferVerificationStatusV1,
    expected_rollback_verified_chunk_bitmap: [u8; VERIFICATION_BITMAP_BYTES_V1],
    expected_rollback_verified_chunk_count: u32,
    expected_rollback_buffer_finalized_slot: u64,
    expected_next_gate_epoch: u64
});

impl WireValidate for AdoptBufferV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected)
    }
}

impl WireValidate for VerifyBufferChunkV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected)?;
        self.proof.validate()
    }
}

impl WireValidate for FinalizeBufferVerificationV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected)?;
        require_nonzero_hashes(&[self.expected_sealed_buffer_header_hash])
    }
}

impl WireValidate for ExtendTargetV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected)?;
        require_nonzero_hashes(&[
            self.expected_prestate_checkpoint_digest,
            self.expected_observation_digest,
            self.expected_observation_root,
            self.expected_next_deployment_digest,
        ])?;
        self.envelope.validate()?;
        if self.expected_prestate_checkpoint_generation == 0
            || self.expected_observation_generation == 0
            || self.expected_observation_finalized_slot == 0
            || self.expected_current_capacity == 0
            || self.expected_extension_delta == 0
            || self.expected_post_capacity <= self.expected_current_capacity
            || self.expected_post_capacity
                != self
                    .expected_current_capacity
                    .checked_add(self.expected_extension_delta)
                    .ok_or(ProgramError::InvalidInstructionData)?
            || self.expected_next_deployment_generation
                != self
                    .expected
                    .expected_current_deployment_generation
                    .checked_add(1)
                    .ok_or(ProgramError::InvalidInstructionData)?
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for ExecuteUpgradeV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected)?;
        require_nonzero_hashes(&[
            self.expected_prestate_checkpoint_digest,
            self.expected_observation_digest,
            self.expected_observation_root,
            self.expected_sealed_buffer_header_hash,
            self.expected_counterpart_proposal_digest,
        ])?;
        self.envelope.validate()?;
        if self.expected_prestate_checkpoint_generation == 0
            || self.expected_observation_generation == 0
            || self.expected_observation_finalized_slot == 0
            || self.expected_actual_capacity == 0
            || self.expected_verified_chunk_count == 0
            || self.expected_buffer_verification_status != BufferVerificationStatusV1::Verified
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

impl WireValidate for CloseAbandonedBufferV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected)
    }
}

impl WireValidate for ActivateRollbackV2 {
    fn validate_wire(&self) -> Result<(), ProgramError> {
        validate_proposal_guard(&self.expected_primary)?;
        validate_proposal_guard(&self.expected_rollback)?;
        require_nonzero_hashes(&[
            self.expected_failure_evidence_digest,
            self.expected_programdata_observation_state_hash,
        ])?;
        if self.expected_programdata_observation_generation == 0
            || self.expected_rollback_buffer_verification_status
                != BufferVerificationStatusV1::Verified
            || self.expected_rollback_verified_chunk_count == 0
            || self.expected_rollback_buffer_finalized_slot == 0
            || self.expected_next_gate_epoch
                != self
                    .expected_primary
                    .expected_gate_epoch
                    .checked_add(1)
                    .ok_or(ProgramError::InvalidInstructionData)?
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CustodyInstructionV2 {
    AdoptBuffer(AdoptBufferV2),
    VerifyBufferChunk(Box<VerifyBufferChunkV2>),
    FinalizeBufferVerification(FinalizeBufferVerificationV2),
    ExtendTarget(Box<ExtendTargetV2>),
    ExecuteUpgrade(Box<ExecuteUpgradeV2>),
    CloseAbandonedBuffer(CloseAbandonedBufferV2),
    ActivateRollback(Box<ActivateRollbackV2>),
}

impl CustodyInstructionV2 {
    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        match data.first().copied() {
            Some(ADOPT_BUFFER_V2_TAG) => AdoptBufferV2::unpack(data).map(Self::AdoptBuffer),
            Some(VERIFY_BUFFER_CHUNK_V2_TAG) => VerifyBufferChunkV2::unpack(data)
                .map(|value| Self::VerifyBufferChunk(Box::new(value))),
            Some(FINALIZE_BUFFER_VERIFICATION_V2_TAG) => {
                FinalizeBufferVerificationV2::unpack(data).map(Self::FinalizeBufferVerification)
            }
            Some(EXTEND_TARGET_V2_TAG) => {
                ExtendTargetV2::unpack(data).map(|value| Self::ExtendTarget(Box::new(value)))
            }
            Some(EXECUTE_UPGRADE_V2_TAG) => {
                ExecuteUpgradeV2::unpack(data).map(|value| Self::ExecuteUpgrade(Box::new(value)))
            }
            Some(CLOSE_ABANDONED_BUFFER_V2_TAG) => {
                CloseAbandonedBufferV2::unpack(data).map(Self::CloseAbandonedBuffer)
            }
            Some(ACTIVATE_ROLLBACK_V2_TAG) => ActivateRollbackV2::unpack(data)
                .map(|value| Self::ActivateRollback(Box::new(value))),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

macro_rules! account_meta {
    ($key:expr, signer_writable) => {
        AccountMeta::new($key, true)
    };
    ($key:expr, signer_readonly) => {
        AccountMeta::new_readonly($key, true)
    };
    ($key:expr, writable) => {
        AccountMeta::new($key, false)
    };
    ($key:expr, readonly) => {
        AccountMeta::new_readonly($key, false)
    };
}

macro_rules! closed_builder {
    ($accounts:ident { $($field:ident: $privilege:ident),* $(,)? }, $function:ident, $instruction:ty) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $accounts {
            $(pub $field: Pubkey,)*
        }

        pub fn $function(
            controller_program: Pubkey,
            accounts: $accounts,
            instruction: $instruction,
        ) -> Result<Instruction, ProgramError> {
            Ok(Instruction {
                program_id: controller_program,
                accounts: vec![$(account_meta!(accounts.$field, $privilege),)*],
                data: instruction.pack()?,
            })
        }
    };
}

closed_builder!(
    AdoptBufferV2Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        capacity_policy: readonly,
        current_deployment: readonly,
        buffer: writable,
        uploader_authority: signer_readonly,
        authority_pda: readonly,
        buffer_verification: writable,
        upgradeable_loader: readonly,
        system_program: readonly,
    },
    adopt_buffer_v2_instruction,
    AdoptBufferV2
);

closed_builder!(
    VerifyBufferChunkV2Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: readonly,
        buffer: readonly,
        buffer_verification: writable,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    verify_buffer_chunk_v2_instruction,
    VerifyBufferChunkV2
);

closed_builder!(
    FinalizeBufferVerificationV2Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        buffer: readonly,
        buffer_verification: writable,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    finalize_buffer_verification_v2_instruction,
    FinalizeBufferVerificationV2
);

closed_builder!(
    ExtendTargetV2Accounts {
        payer: signer_writable,
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: writable,
        capacity_policy: readonly,
        current_deployment: writable,
        programdata_observation: readonly,
        prestate_checkpoint: readonly,
        target_programdata: writable,
        target_program: writable,
        authority_pda: writable,
        upgradeable_loader: readonly,
        system_program: readonly,
        rent_sysvar: readonly,
        instructions_sysvar: readonly,
    },
    extend_target_v2_instruction,
    ExtendTargetV2
);

closed_builder!(
    ExecuteUpgradeV2Accounts {
        controller_config: readonly,
        policy: readonly,
        protocol_gate: readonly,
        proposal: writable,
        counterpart_proposal: readonly,
        counterpart_buffer_verification: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        prestate_programdata_observation: readonly,
        prestate_checkpoint: readonly,
        current_programdata_observation: readonly,
        buffer_verification: writable,
        target_programdata: writable,
        target_program: writable,
        buffer: writable,
        canonical_spill_treasury: writable,
        rent_sysvar: readonly,
        clock_sysvar: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
        instructions_sysvar: readonly,
    },
    execute_upgrade_v2_instruction,
    ExecuteUpgradeV2
);

closed_builder!(
    CloseAbandonedBufferV2Accounts {
        controller_config: readonly,
        protocol_gate: readonly,
        proposal: readonly,
        buffer_verification: writable,
        buffer: writable,
        canonical_spill_treasury: writable,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    close_abandoned_buffer_v2_instruction,
    CloseAbandonedBufferV2
);

closed_builder!(
    ActivateRollbackV2Accounts {
        controller_config: readonly,
        policy: readonly,
        protocol_gate: writable,
        primary_proposal: readonly,
        rollback_proposal: writable,
        rollback_buffer_verification: readonly,
        primary_programdata_verification: readonly,
        failure_observation: readonly,
        capacity_policy: readonly,
        current_deployment: readonly,
        programdata_observation: readonly,
        target_program: readonly,
        target_programdata: readonly,
        authority_pda: readonly,
        upgradeable_loader: readonly,
    },
    activate_rollback_v2_instruction,
    ActivateRollbackV2
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        release1_state::ProposalStateV2,
        release1_v3_instruction::ProgramDataVerificationGuardV2,
        state::{GateStatusV1, OptionalPubkeyV1, ProposalClassV1},
    };

    fn bytes(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn key(seed: u8) -> Pubkey {
        Pubkey::new_from_array(bytes(seed))
    }

    fn guard(state: ProposalStateV2) -> ProposalGuardV3 {
        ProposalGuardV3 {
            expected_proposal_digest: bytes(1),
            expected_state: state,
            expected_gate_status: GateStatusV1::FrozenForUpgrade,
            expected_gate_epoch: 3,
            expected_target_nonce: 7,
            expected_capacity_policy_digest: bytes(2),
            expected_current_deployment_digest: bytes(3),
            expected_current_deployment_generation: 4,
        }
    }

    fn envelope() -> CeremonyEnvelopeV1 {
        CeremonyEnvelopeV1 {
            compute_unit_limit: 1_000_000,
            compute_unit_price_micro_lamports: 1,
            durable_nonce_account: OptionalPubkeyV1::none(),
            durable_nonce_authority: OptionalPubkeyV1::none(),
        }
    }

    #[test]
    fn tags_are_contiguous_unique_and_follow_v3_lifecycle() {
        let tags = [
            ADOPT_BUFFER_V2_TAG,
            VERIFY_BUFFER_CHUNK_V2_TAG,
            FINALIZE_BUFFER_VERIFICATION_V2_TAG,
            EXTEND_TARGET_V2_TAG,
            EXECUTE_UPGRADE_V2_TAG,
            CLOSE_ABANDONED_BUFFER_V2_TAG,
            ACTIVATE_ROLLBACK_V2_TAG,
        ];
        assert_eq!(tags, [75, 76, 77, 78, 79, 80, 81]);
        let mut sorted = tags.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), tags.len());
    }

    #[test]
    fn every_codec_is_exact_and_rejects_trailing_or_wrong_tag() {
        let value = ExtendTargetV2 {
            expected: guard(ProposalStateV2::Frozen),
            expected_prestate_checkpoint_digest: bytes(4),
            expected_prestate_checkpoint_generation: 1,
            expected_observation_digest: bytes(5),
            expected_observation_generation: 2,
            expected_observation_root: bytes(6),
            expected_observation_finalized_slot: 10,
            expected_current_capacity: 100,
            expected_extension_delta: 20,
            expected_post_capacity: 120,
            expected_next_deployment_generation: 5,
            expected_next_deployment_digest: bytes(7),
            envelope: envelope(),
        };
        let packed = value.pack().unwrap();
        assert_eq!(packed.len(), ExtendTargetV2::LEN);
        assert_eq!(ExtendTargetV2::unpack(&packed).unwrap(), value);
        let mut trailing = packed.clone();
        trailing.push(0);
        assert!(ExtendTargetV2::unpack(&trailing).is_err());
        let mut wrong = packed;
        wrong[0] = 77;
        assert!(ExtendTargetV2::unpack(&wrong).is_err());
    }

    #[test]
    fn payload_length_constants_match_every_fixed_codec() {
        let proof = ArtifactChunkProofV2 {
            proof_len: 0,
            nodes: [[0; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1],
        };
        let adopt = AdoptBufferV2 {
            expected: guard(ProposalStateV2::Draft),
        };
        let verify = VerifyBufferChunkV2 {
            expected: guard(ProposalStateV2::BufferAdopted),
            chunk_index: 0,
            proof,
            expected_verification_status: BufferVerificationStatusV1::Adopted,
            expected_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
            expected_verified_chunk_count: 0,
        };
        let finalize = FinalizeBufferVerificationV2 {
            expected: guard(ProposalStateV2::BufferAdopted),
            expected_verification_status: BufferVerificationStatusV1::ReadyToFinalize,
            expected_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
            expected_verified_chunk_count: 0,
            expected_sealed_buffer_header_hash: bytes(6),
        };
        let close = CloseAbandonedBufferV2 {
            expected: guard(ProposalStateV2::Cancelled),
            expected_verification_status: BufferVerificationStatusV1::Verified,
            expected_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
            expected_verified_chunk_count: 0,
            expected_buffer_verification_finalized_slot: 9,
        };
        let rollback = ActivateRollbackV2 {
            expected_primary: guard(ProposalStateV2::UpgradeExecuted),
            expected_rollback: guard(ProposalStateV2::Timelocked),
            expected_failure_evidence_digest: bytes(7),
            expected_primary_verification_generation: 0,
            expected_programdata_observation_state_hash: bytes(8),
            expected_programdata_observation_generation: 1,
            expected_rollback_buffer_verification_status: BufferVerificationStatusV1::Verified,
            expected_rollback_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
            expected_rollback_verified_chunk_count: 1,
            expected_rollback_buffer_finalized_slot: 10,
            expected_next_gate_epoch: 4,
        };

        assert_eq!(adopt.pack().unwrap().len(), AdoptBufferV2::LEN);
        assert_eq!(verify.pack().unwrap().len(), VerifyBufferChunkV2::LEN);
        assert_eq!(
            finalize.pack().unwrap().len(),
            FinalizeBufferVerificationV2::LEN
        );
        assert_eq!(close.pack().unwrap().len(), CloseAbandonedBufferV2::LEN);
        assert_eq!(rollback.pack().unwrap().len(), ActivateRollbackV2::LEN);
    }

    #[test]
    fn proofs_reject_nonzero_padding_and_extension_guards_reject_bad_arithmetic() {
        let mut proof = ArtifactChunkProofV2 {
            proof_len: 1,
            nodes: [[0; 32]; MAX_FIXED_MERKLE_PROOF_NODES_V1],
        };
        proof.nodes[0] = bytes(9);
        proof.validate().unwrap();
        proof.nodes[1] = bytes(8);
        assert!(proof.validate().is_err());

        let mut value = ExtendTargetV2 {
            expected: guard(ProposalStateV2::Frozen),
            expected_prestate_checkpoint_digest: bytes(4),
            expected_prestate_checkpoint_generation: 1,
            expected_observation_digest: bytes(5),
            expected_observation_generation: 2,
            expected_observation_root: bytes(6),
            expected_observation_finalized_slot: 10,
            expected_current_capacity: 100,
            expected_extension_delta: 20,
            expected_post_capacity: 120,
            expected_next_deployment_generation: 5,
            expected_next_deployment_digest: bytes(7),
            envelope: envelope(),
        };
        value.pack().unwrap();
        value.expected_post_capacity = 121;
        assert!(value.pack().is_err());
    }

    #[test]
    fn closed_builder_has_exact_accounts_and_no_caller_vector() {
        let instruction = close_abandoned_buffer_v2_instruction(
            key(1),
            CloseAbandonedBufferV2Accounts {
                controller_config: key(2),
                protocol_gate: key(3),
                proposal: key(4),
                buffer_verification: key(5),
                buffer: key(6),
                canonical_spill_treasury: key(7),
                authority_pda: key(8),
                upgradeable_loader: key(9),
            },
            CloseAbandonedBufferV2 {
                expected: guard(ProposalStateV2::Cancelled),
                expected_verification_status: BufferVerificationStatusV1::Verified,
                expected_verified_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
                expected_verified_chunk_count: 0,
                expected_buffer_verification_finalized_slot: 9,
            },
        )
        .unwrap();
        assert_eq!(
            instruction.accounts.len(),
            CLOSE_ABANDONED_BUFFER_V2_ACCOUNT_COUNT
        );
        assert_eq!(instruction.accounts[3], AccountMeta::new(key(5), false));
        assert_eq!(instruction.accounts[5], AccountMeta::new(key(7), false));
    }

    #[test]
    fn execute_builder_keeps_prestate_and_fresh_observations_distinct() {
        let instruction = execute_upgrade_v2_instruction(
            key(1),
            ExecuteUpgradeV2Accounts {
                controller_config: key(2),
                policy: key(3),
                protocol_gate: key(4),
                proposal: key(5),
                counterpart_proposal: key(6),
                counterpart_buffer_verification: key(7),
                capacity_policy: key(8),
                current_deployment: key(9),
                prestate_programdata_observation: key(10),
                prestate_checkpoint: key(11),
                current_programdata_observation: key(12),
                buffer_verification: key(13),
                target_programdata: key(14),
                target_program: key(15),
                buffer: key(16),
                canonical_spill_treasury: key(17),
                rent_sysvar: key(18),
                clock_sysvar: key(19),
                authority_pda: key(20),
                upgradeable_loader: key(21),
                instructions_sysvar: key(22),
            },
            ExecuteUpgradeV2 {
                expected: guard(ProposalStateV2::Frozen),
                expected_prestate_checkpoint_digest: bytes(4),
                expected_prestate_checkpoint_generation: 1,
                expected_observation_digest: bytes(5),
                expected_observation_generation: 2,
                expected_observation_root: bytes(6),
                expected_observation_finalized_slot: 10,
                expected_actual_capacity: 120,
                expected_sealed_buffer_header_hash: bytes(7),
                expected_verified_chunk_count: 1,
                expected_buffer_verification_status: BufferVerificationStatusV1::Verified,
                expected_counterpart_proposal_digest: bytes(8),
                expected_counterpart_buffer_verification_status:
                    BufferVerificationStatusV1::Verified,
                envelope: envelope(),
            },
        )
        .unwrap();
        assert_eq!(instruction.accounts.len(), EXECUTE_UPGRADE_V2_ACCOUNT_COUNT);
        assert_eq!(
            instruction.accounts[8],
            AccountMeta::new_readonly(key(10), false)
        );
        assert_eq!(
            instruction.accounts[10],
            AccountMeta::new_readonly(key(12), false)
        );
        assert_ne!(
            instruction.accounts[8].pubkey,
            instruction.accounts[10].pubkey
        );
        assert_eq!(instruction.accounts[12], AccountMeta::new(key(14), false));
        assert_eq!(
            instruction.accounts[20],
            AccountMeta::new_readonly(key(22), false)
        );
    }

    #[test]
    fn existing_verification_guard_remains_a_separate_domain() {
        let guard = ProgramDataVerificationGuardV2 {
            expected_proposal_digest: bytes(1),
            expected_verification_digest: bytes(2),
            expected_verification_generation: 1,
            expected_status: crate::release1_v3_state::ProgramDataVerificationStatusV2::Verified,
            expected_gate_epoch: 3,
            expected_target_nonce: 7,
            expected_capacity_policy_digest: bytes(4),
            expected_current_deployment_digest: bytes(5),
            expected_current_deployment_generation: 6,
            expected_observation_digest: bytes(7),
            expected_observation_generation: 2,
            expected_actual_capacity: 100,
            expected_authority: key(8),
        };
        guard.validate().unwrap();
        assert_ne!(
            ProposalClassV1::EmergencyRollback as u8,
            ProposalClassV1::RoutineUpgrade as u8
        );
    }
}
