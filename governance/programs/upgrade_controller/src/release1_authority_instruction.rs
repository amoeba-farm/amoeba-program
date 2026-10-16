//! Fixed instruction codecs for controller immutability, checked authority
//! handoff, and one-time bootstrap activation.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::program_error::ProgramError;

use crate::state::OptionalPubkeyV1;

pub const RECORD_CONTROLLER_IMMUTABILITY_V1_TAG: u8 = 43;
pub const CREATE_TARGET_AUTHORITY_HANDOFF_V1_TAG: u8 = 44;
pub const APPROVE_TARGET_AUTHORITY_HANDOFF_V1_TAG: u8 = 45;
pub const QUEUE_TARGET_AUTHORITY_HANDOFF_V1_TAG: u8 = 46;
pub const ACCEPT_TARGET_AUTHORITY_CHECKED_V1_TAG: u8 = 47;
pub const CREATE_BOOTSTRAP_ACTIVATION_V1_TAG: u8 = 49;
pub const APPROVE_BOOTSTRAP_ACTIVATION_V1_TAG: u8 = 50;
pub const QUEUE_BOOTSTRAP_ACTIVATION_V1_TAG: u8 = 51;
pub const EXECUTE_BOOTSTRAP_ACTIVATION_V1_TAG: u8 = 52;

pub const MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1: u32 = 1_400_000;
pub const MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1: u64 = 10_000_000;

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CeremonyEnvelopeV1 {
    pub compute_unit_limit: u32,
    pub compute_unit_price_micro_lamports: u64,
    pub durable_nonce_account: OptionalPubkeyV1,
    pub durable_nonce_authority: OptionalPubkeyV1,
}

impl CeremonyEnvelopeV1 {
    pub const LEN: usize = 78;

    pub fn validate(&self) -> Result<(), ProgramError> {
        self.durable_nonce_account
            .validate()
            .map_err(ProgramError::from)?;
        self.durable_nonce_authority
            .validate()
            .map_err(ProgramError::from)?;
        if self.compute_unit_limit == 0
            || self.compute_unit_limit > MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1
            || self.compute_unit_price_micro_lamports
                > MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1
            || self.durable_nonce_account.present != self.durable_nonce_authority.present
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
    RecordControllerImmutabilityV1,
    RECORD_CONTROLLER_IMMUTABILITY_V1_TAG,
    160,
    {
        expected_capacity_policy_digest: [u8; 32],
        expected_release_digest: [u8; 32],
        expected_pre_observation_digest: [u8; 32],
        expected_post_observation_digest: [u8; 32],
        expected_receipt_digest: [u8; 32]
    }
);

fixed_instruction!(
    CreateTargetAuthorityHandoffV1,
    CREATE_TARGET_AUTHORITY_HANDOFF_V1_TAG,
    160,
    {
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_council_version: u64,
        bridge_source_commitment: [u8; 32],
        bridge_build_inputs_commitment: [u8; 32],
        bridge_package_commitment: [u8; 32],
        bridge_release_manifest_commitment: [u8; 32],
        plan_valid_until_slot: u64
    }
);

fixed_instruction!(
    ApproveTargetAuthorityHandoffV1,
    APPROVE_TARGET_AUTHORITY_HANDOFF_V1_TAG,
    56,
    {
        expected_proposal_digest: [u8; 32],
        expected_council_version: u64,
        expected_gate_epoch: u64,
        expected_target_nonce: u64
    }
);

fixed_instruction!(
    QueueTargetAuthorityHandoffV1,
    QUEUE_TARGET_AUTHORITY_HANDOFF_V1_TAG,
    56,
    {
        expected_proposal_digest: [u8; 32],
        expected_council_version: u64,
        expected_gate_epoch: u64,
        expected_target_nonce: u64
    }
);

fixed_instruction!(
    AcceptTargetAuthorityCheckedV1,
    ACCEPT_TARGET_AUTHORITY_CHECKED_V1_TAG,
    158,
    {
        expected_proposal_digest: [u8; 32],
        expected_bridge_observation_digest: [u8; 32],
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        envelope: CeremonyEnvelopeV1
    }
);

fixed_instruction!(
    CreateBootstrapActivationV1,
    CREATE_BOOTSTRAP_ACTIVATION_V1_TAG,
    128,
    {
        expected_controller_immutability_digest: [u8; 32],
        expected_handoff_receipt_digest: [u8; 32],
        expected_bridge_observation_digest: [u8; 32],
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_council_version: u64,
        plan_valid_until_slot: u64
    }
);

fixed_instruction!(
    ApproveBootstrapActivationV1,
    APPROVE_BOOTSTRAP_ACTIVATION_V1_TAG,
    56,
    {
        expected_proposal_digest: [u8; 32],
        expected_council_version: u64,
        expected_gate_epoch: u64,
        expected_target_nonce: u64
    }
);

fixed_instruction!(
    QueueBootstrapActivationV1,
    QUEUE_BOOTSTRAP_ACTIVATION_V1_TAG,
    56,
    {
        expected_proposal_digest: [u8; 32],
        expected_council_version: u64,
        expected_gate_epoch: u64,
        expected_target_nonce: u64
    }
);

fixed_instruction!(
    ExecuteBootstrapActivationV1,
    EXECUTE_BOOTSTRAP_ACTIVATION_V1_TAG,
    222,
    {
        expected_proposal_digest: [u8; 32],
        expected_bridge_observation_digest: [u8; 32],
        expected_gate_epoch: u64,
        expected_target_nonce: u64,
        expected_deployment_plan_digest: [u8; 32],
        expected_receipt_plan_digest: [u8; 32],
        envelope: CeremonyEnvelopeV1
    }
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Release1AuthorityInstruction {
    RecordControllerImmutability(RecordControllerImmutabilityV1),
    CreateTargetAuthorityHandoff(CreateTargetAuthorityHandoffV1),
    ApproveTargetAuthorityHandoff(ApproveTargetAuthorityHandoffV1),
    QueueTargetAuthorityHandoff(QueueTargetAuthorityHandoffV1),
    AcceptTargetAuthorityChecked(AcceptTargetAuthorityCheckedV1),
    CreateBootstrapActivation(CreateBootstrapActivationV1),
    ApproveBootstrapActivation(ApproveBootstrapActivationV1),
    QueueBootstrapActivation(QueueBootstrapActivationV1),
    ExecuteBootstrapActivation(ExecuteBootstrapActivationV1),
}

impl Release1AuthorityInstruction {
    pub fn unpack(data: &[u8]) -> Result<Self, ProgramError> {
        let tag = data
            .first()
            .copied()
            .ok_or(ProgramError::InvalidInstructionData)?;
        let value = match tag {
            RECORD_CONTROLLER_IMMUTABILITY_V1_TAG => {
                Self::RecordControllerImmutability(RecordControllerImmutabilityV1::unpack(data)?)
            }
            CREATE_TARGET_AUTHORITY_HANDOFF_V1_TAG => {
                Self::CreateTargetAuthorityHandoff(CreateTargetAuthorityHandoffV1::unpack(data)?)
            }
            APPROVE_TARGET_AUTHORITY_HANDOFF_V1_TAG => {
                Self::ApproveTargetAuthorityHandoff(ApproveTargetAuthorityHandoffV1::unpack(data)?)
            }
            QUEUE_TARGET_AUTHORITY_HANDOFF_V1_TAG => {
                Self::QueueTargetAuthorityHandoff(QueueTargetAuthorityHandoffV1::unpack(data)?)
            }
            ACCEPT_TARGET_AUTHORITY_CHECKED_V1_TAG => {
                Self::AcceptTargetAuthorityChecked(AcceptTargetAuthorityCheckedV1::unpack(data)?)
            }
            CREATE_BOOTSTRAP_ACTIVATION_V1_TAG => {
                Self::CreateBootstrapActivation(CreateBootstrapActivationV1::unpack(data)?)
            }
            APPROVE_BOOTSTRAP_ACTIVATION_V1_TAG => {
                Self::ApproveBootstrapActivation(ApproveBootstrapActivationV1::unpack(data)?)
            }
            QUEUE_BOOTSTRAP_ACTIVATION_V1_TAG => {
                Self::QueueBootstrapActivation(QueueBootstrapActivationV1::unpack(data)?)
            }
            EXECUTE_BOOTSTRAP_ACTIVATION_V1_TAG => {
                Self::ExecuteBootstrapActivation(ExecuteBootstrapActivationV1::unpack(data)?)
            }
            _ => return Err(ProgramError::InvalidInstructionData),
        };
        match &value {
            Self::AcceptTargetAuthorityChecked(ix) => ix.envelope.validate()?,
            Self::ExecuteBootstrapActivation(ix) => ix.envelope.validate()?,
            _ => {}
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use solana_program::pubkey::Pubkey;

    use super::*;

    fn some(byte: u8) -> OptionalPubkeyV1 {
        OptionalPubkeyV1::some(Pubkey::new_from_array([byte; 32])).unwrap()
    }

    fn envelope() -> CeremonyEnvelopeV1 {
        CeremonyEnvelopeV1 {
            compute_unit_limit: 1_000_000,
            compute_unit_price_micro_lamports: 1,
            durable_nonce_account: some(1),
            durable_nonce_authority: some(2),
        }
    }

    #[test]
    fn tags_are_unique_and_begin_after_observation_protocol() {
        let tags = [
            RECORD_CONTROLLER_IMMUTABILITY_V1_TAG,
            CREATE_TARGET_AUTHORITY_HANDOFF_V1_TAG,
            APPROVE_TARGET_AUTHORITY_HANDOFF_V1_TAG,
            QUEUE_TARGET_AUTHORITY_HANDOFF_V1_TAG,
            ACCEPT_TARGET_AUTHORITY_CHECKED_V1_TAG,
            CREATE_BOOTSTRAP_ACTIVATION_V1_TAG,
            APPROVE_BOOTSTRAP_ACTIVATION_V1_TAG,
            QUEUE_BOOTSTRAP_ACTIVATION_V1_TAG,
            EXECUTE_BOOTSTRAP_ACTIVATION_V1_TAG,
        ];
        assert_eq!(tags, [43, 44, 45, 46, 47, 49, 50, 51, 52]);
    }

    #[test]
    fn exact_codecs_reject_every_size_and_tag_drift() {
        let ix = AcceptTargetAuthorityCheckedV1 {
            expected_proposal_digest: [1; 32],
            expected_bridge_observation_digest: [2; 32],
            expected_gate_epoch: 3,
            expected_target_nonce: 4,
            envelope: envelope(),
        };
        let bytes = ix.pack().unwrap();
        assert_eq!(bytes.len(), AcceptTargetAuthorityCheckedV1::LEN);
        assert_eq!(AcceptTargetAuthorityCheckedV1::unpack(&bytes), Ok(ix));
        for length in 0..bytes.len() {
            assert_eq!(
                AcceptTargetAuthorityCheckedV1::unpack(&bytes[..length]),
                Err(ProgramError::InvalidInstructionData)
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            AcceptTargetAuthorityCheckedV1::unpack(&trailing),
            Err(ProgramError::InvalidInstructionData)
        );
        let mut wrong_tag = bytes;
        wrong_tag[0] ^= 1;
        assert_eq!(
            AcceptTargetAuthorityCheckedV1::unpack(&wrong_tag),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn envelope_is_bounded_and_nonce_pair_is_canonical() {
        assert_eq!(envelope().validate(), Ok(()));
        let mut bad = envelope();
        bad.compute_unit_limit = 0;
        assert_eq!(bad.validate(), Err(ProgramError::InvalidInstructionData));
        let mut bad = envelope();
        bad.compute_unit_limit = MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1 + 1;
        assert_eq!(bad.validate(), Err(ProgramError::InvalidInstructionData));
        let mut bad = envelope();
        bad.durable_nonce_authority = OptionalPubkeyV1::none();
        assert_eq!(bad.validate(), Err(ProgramError::InvalidInstructionData));
    }
}
