use crate::state::{Action, Result, MAX_DATA};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{program_error::ProgramError, pubkey::Pubkey};

/// Closed instruction set. Unknown tags fail before account access or payload decoding.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum Instruction {
    Initialize {
        seats: [Pubkey; 5],
        treasury: Pubkey,
    },
    Create {
        expected_id: u64,
        action: Action,
    },
    SealBuffer {
        digest: [u8; 32],
    },
    VerifyChunk {
        digest: [u8; 32],
        index: u32,
        proof_len: u8,
        proof: [[u8; 32]; 7],
    },
    Approve {
        digest: [u8; 32],
    },
    Cancel {
        digest: [u8; 32],
    },
    Expire,
    ExecutePolicy {
        digest: [u8; 32],
    },
    ExecuteControllerUpgrade {
        digest: [u8; 32],
    },
    CloseBuffer {
        digest: [u8; 32],
    },
    /// Retired tag 10, kept so later tags keep their Borsh positions.
    /// `unpack` rejects it like an unknown tag.
    ExtendController {
        digest: [u8; 32],
    },
    RegisterTarget,
    ExecuteTargetUpgrade {
        digest: [u8; 32],
    },
    /// Retired tag 13, rejected by `unpack` like tag 10.
    ExtendTarget {
        digest: [u8; 32],
    },
    ExecuteTargetGate {
        digest: [u8; 32],
    },
    ExecuteSpreadLightConfig {
        digest: [u8; 32],
    },
    PublishVerification {
        digest: [u8; 32],
    },
}
impl Instruction {
    pub fn unpack(data: &[u8]) -> Result<Self> {
        if data.len() > MAX_DATA || !matches!(data.first(), Some(0..=9 | 11..=12 | 14..=16)) {
            return Err(ProgramError::InvalidInstructionData);
        }
        Self::try_from_slice(data).map_err(|_| ProgramError::InvalidInstructionData)
    }
}
