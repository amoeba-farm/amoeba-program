//! Fail-closed fixed-account I/O shared by Release 1 processors.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program::invoke_signed,
    program_error::ProgramError, pubkey::Pubkey, rent::Rent,
};
use solana_sdk_ids::system_program;
use solana_system_interface::instruction as system_instruction;

use crate::{release1_v3_state::UpgradeProposalV3, GovernanceError};

pub fn validate_exact_privileges(
    account: &AccountInfo<'_>,
    writable: bool,
    signer: bool,
    executable: bool,
) -> ProgramResult {
    if account.is_writable != writable
        || account.is_signer != signer
        || account.executable != executable
    {
        return Err(GovernanceError::InvalidAccountPrivileges.into());
    }
    Ok(())
}

pub fn require_distinct_accounts(accounts: &[&AccountInfo<'_>]) -> ProgramResult {
    for (index, account) in accounts.iter().enumerate() {
        if accounts[..index]
            .iter()
            .any(|prior| prior.key == account.key)
        {
            return Err(GovernanceError::CrossAccountMismatch.into());
        }
    }
    Ok(())
}

pub fn load_fixed_controller_account<T: BorshDeserialize>(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    expected_len: usize,
) -> Result<Box<T>, ProgramError> {
    if account.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    if account.data_len() != expected_len {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    // Keep the deserialize result in the allocator's return place. Binding a
    // large fixed account to a local first materializes the complete value on
    // the 4 KiB SBPF stack before moving it into the `Box`.
    Ok(Box::new(
        T::try_from_slice(&account.try_borrow_data()?)
            .map_err(|_| ProgramError::InvalidAccountData)?,
    ))
}

pub fn load_upgrade_proposal_v3(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
) -> Result<Box<UpgradeProposalV3>, ProgramError> {
    if account.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    if account.data_len() != UpgradeProposalV3::LEN {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    let data = account.try_borrow_data()?;
    UpgradeProposalV3::from_bytes_boxed_strict(&data).map_err(ProgramError::from)
}

pub fn encode_fixed_account<T: BorshSerialize>(
    value: &T,
    expected_len: usize,
) -> Result<Vec<u8>, ProgramError> {
    let encoded = value
        .try_to_vec()
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if encoded.len() != expected_len {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    Ok(encoded)
}

pub fn store_fixed_controller_account<T: BorshSerialize>(
    program_id: &Pubkey,
    account: &AccountInfo<'_>,
    value: &T,
    expected_len: usize,
) -> ProgramResult {
    if account.owner != program_id {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let encoded = encode_fixed_account(value, expected_len)?;
    let mut data = account.try_borrow_mut_data()?;
    if data.len() != expected_len {
        return Err(GovernanceError::InvalidAccountSize.into());
    }
    data.copy_from_slice(&encoded);
    Ok(())
}

/// Creates a canonical fixed-size PDA, including a system-owned, zero-data,
/// prefunded address.  Foreign ownership or any existing data fails before CPI.
#[allow(clippy::too_many_arguments)]
pub fn create_fixed_pda_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    pda: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    rent: &Rent,
    space: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(pda, true, false, false)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    if *system_program_info.key != system_program::ID {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if pda.owner != &system_program::ID || pda.data_len() != 0 {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }
    let required_lamports = rent.minimum_balance(space);
    let existing_lamports = pda.lamports();
    let missing_lamports = required_lamports.saturating_sub(existing_lamports);

    if existing_lamports == 0 {
        let instruction = system_instruction::create_account(
            payer.key,
            pda.key,
            required_lamports,
            space as u64,
            program_id,
        );
        invoke_signed(
            &instruction,
            &[payer.clone(), pda.clone(), system_program_info.clone()],
            &[signer_seeds],
        )?;
    } else {
        if missing_lamports != 0 {
            let transfer = system_instruction::transfer(payer.key, pda.key, missing_lamports);
            invoke_signed(
                &transfer,
                &[payer.clone(), pda.clone(), system_program_info.clone()],
                &[],
            )?;
        }
        let allocate = system_instruction::allocate(pda.key, space as u64);
        invoke_signed(
            &allocate,
            &[pda.clone(), system_program_info.clone()],
            &[signer_seeds],
        )?;
        let assign = system_instruction::assign(pda.key, program_id);
        invoke_signed(
            &assign,
            &[pda.clone(), system_program_info.clone()],
            &[signer_seeds],
        )?;
    }

    if pda.owner != program_id || pda.data_len() != space || pda.lamports() < required_lamports {
        return Err(GovernanceError::InvalidRelease1Account.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release1_state::{BufferVerificationStatusV1, BufferVerificationV1};
    use crate::release1_state::{
        BUFFER_VERIFICATION_V1_DISCRIMINATOR, BUFFER_VERIFICATION_V1_RESERVED_LEN,
        RELEASE1_ACCOUNT_VERSION_V1,
    };

    fn key(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn minimal_buffer_verification() -> BufferVerificationV1 {
        BufferVerificationV1 {
            discriminator: BUFFER_VERIFICATION_V1_DISCRIMINATOR,
            account_version: RELEASE1_ACCOUNT_VERSION_V1,
            bump: 1,
            initialized: true,
            status: BufferVerificationStatusV1::Adopted,
            controller_config: key(1),
            proposal: key(2),
            upgradeable_loader: key(3),
            buffer: key(4),
            expected_uploader_authority: key(5),
            controller_authority: key(6),
            artifact_length: 1,
            artifact_sha256: [1; 32],
            artifact_chunk_merkle_root: [2; 32],
            chunk_hash_domain: [3; 32],
            chunk_size: 16_384,
            chunk_count: 1,
            verified_chunk_bitmap: [0; 64],
            verified_chunk_count: 0,
            adopted_slot: 1,
            finalized_slot: 0,
            sealed_buffer_header_hash: [4; 32],
            terminal_slot: 0,
            reserved: [0; BUFFER_VERIFICATION_V1_RESERVED_LEN],
        }
    }

    #[test]
    fn fixed_encoding_is_exact_and_rejects_the_wrong_declared_size() {
        let value = minimal_buffer_verification();
        assert_eq!(
            encode_fixed_account(&value, BufferVerificationV1::LEN)
                .unwrap()
                .len(),
            BufferVerificationV1::LEN
        );
        assert_eq!(
            encode_fixed_account(&value, BufferVerificationV1::LEN - 1),
            Err(ProgramError::Custom(
                GovernanceError::InvalidAccountSize as u32
            ))
        );
    }

    #[test]
    fn privilege_and_alias_checks_are_exact() {
        let owner = key(20);
        let first_key = key(21);
        let second_key = key(22);
        let mut first_lamports = 1;
        let mut second_lamports = 1;
        let mut first_data = [];
        let mut second_data = [];
        let first = AccountInfo::new(
            &first_key,
            true,
            true,
            &mut first_lamports,
            &mut first_data,
            &owner,
            false,
            0,
        );
        let second = AccountInfo::new(
            &second_key,
            false,
            false,
            &mut second_lamports,
            &mut second_data,
            &owner,
            false,
            0,
        );
        assert_eq!(validate_exact_privileges(&first, true, true, false), Ok(()));
        assert_eq!(
            validate_exact_privileges(&first, false, true, false),
            Err(ProgramError::Custom(
                GovernanceError::InvalidAccountPrivileges as u32
            ))
        );
        assert_eq!(require_distinct_accounts(&[&first, &second]), Ok(()));
        assert_eq!(
            require_distinct_accounts(&[&first, &second, &first]),
            Err(ProgramError::Custom(
                GovernanceError::CrossAccountMismatch as u32
            ))
        );
    }
}
