//! Wallet-scoped settlement capabilities. The delegate can only sign inside this program.
//! No off-chain signer, amount prediction, or durable nonce is an execution authority.
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use solana_program::pubkey::Pubkey;

pub const COLLECTIVE_SETTLEMENT_DELEGATE_SEED: &[u8] = b"settlement-delegate";
pub const POSITION_SETTLEMENT_AUTHORITY_SEED: &[u8] = b"position-settlement";
pub const POSITION_SETTLEMENT_AUTHORITY_LEN: usize = 105;
pub const POSITION_SETTLEMENT_AUTHORITY_DISCRIMINATOR: [u8; 8] = *b"APSAUTH1";

/// A capability for exactly one wallet and contract mint. It never owns the wallet's tokens.
pub fn derive_collective_settlement_delegate(
    program_id: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            COLLECTIVE_SETTLEMENT_DELEGATE_SEED,
            owner.as_ref(),
            mint.as_ref(),
        ],
        program_id,
    )
}

/// A create-once permission record for one exact LP position and its owner.
pub fn derive_position_settlement_authority(
    program_id: &Pubkey,
    owner: &Pubkey,
    position: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            POSITION_SETTLEMENT_AUTHORITY_SEED,
            owner.as_ref(),
            position.as_ref(),
        ],
        program_id,
    )
}
