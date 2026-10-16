use super::*;

/// One owner's USDC ledger inside the bond vault. Bonds, stakes and challenge pots are debited
/// from `available_balance` while they are escrowed in oracle state and credited back (or to the
/// winner) when they settle; the tokens never leave the bond vault in between.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserCollateral {
    pub is_initialized: bool,
    pub bump: u8,
    pub owner: Pubkey,
    pub available_balance: u64,
    pub position_locked_balance: u64,
    pub last_action_slot: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for UserCollateral {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            owner: Pubkey::default(),
            available_balance: 0,
            position_locked_balance: 0,
            last_action_slot: 0,
        }
    }
}

impl UserCollateral {
    pub const LEN: usize = 1 + 1 + 32 + 8 + 8 + 8;
}

pub fn derive_user_collateral_pda(program_id: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::USER_COLLATERAL_PDA_SEED,
            owner.as_ref(),
        ],
        program_id,
    )
}
