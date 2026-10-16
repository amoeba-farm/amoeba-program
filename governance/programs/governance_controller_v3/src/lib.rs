//! Fresh council-upgradeable controller. No V1/V2 dispatch or live-state migration.
pub mod artifact;
pub mod instruction;
pub mod processor;
pub mod state;
pub mod verification;

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &solana_program::pubkey::Pubkey,
    accounts: &[solana_program::account_info::AccountInfo<'_>],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    ameba_governance_interface::emit_profile_marker();
    #[cfg(feature = "localnet-governance")]
    if program_id != &ameba_governance_interface::LOCALNET_CONTROLLER_PROGRAM_ID {
        return Err(solana_program::program_error::ProgramError::IncorrectProgramId);
    }
    processor::process(program_id, accounts, data)
}

#[cfg(all(test, feature = "localnet-governance"))]
mod localnet_profile_tests {
    #[test]
    fn entrypoint_rejects_other_controller_identities() {
        assert_eq!(
            super::process_instruction(
                &ameba_governance_interface::PRODUCTION_CONTROLLER_PROGRAM_ID,
                &[],
                &[],
            ),
            Err(solana_program::program_error::ProgramError::IncorrectProgramId)
        );
    }
}
