use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

solana_program::declare_id!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Derive the canonical ATA for an arbitrary token program.
pub fn get_associated_token_address_with_program_id(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program_id: &Pubkey,
) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program_id.as_ref(), mint.as_ref()],
        &id(),
    )
    .0
}

/// Build the canonical one-byte `CreateIdempotent` ATA instruction without
/// linking a second generation of the SPL Token-2022 dependency graph.
pub fn create_associated_token_account_idempotent(
    funder: &Pubkey,
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program_id: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::new(*funder, true),
            AccountMeta::new(
                get_associated_token_address_with_program_id(wallet, mint, token_program_id),
                false,
            ),
            AccountMeta::new_readonly(*wallet, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(*token_program_id, false),
        ],
        data: vec![1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_ata_helpers_match_the_canonical_spl_client() {
        let funder = Pubkey::new_from_array([1; 32]);
        let wallet = Pubkey::new_from_array([2; 32]);
        let mint = Pubkey::new_from_array([3; 32]);
        let token_program = Pubkey::new_from_array([4; 32]);

        assert_eq!(id(), spl_associated_token_account::id());
        assert_eq!(
            get_associated_token_address_with_program_id(&wallet, &mint, &token_program),
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &wallet,
                &mint,
                &token_program,
            ),
        );
        assert_eq!(
            create_associated_token_account_idempotent(&funder, &wallet, &mint, &token_program,),
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &funder,
                &wallet,
                &mint,
                &token_program,
            ),
        );
    }
}
