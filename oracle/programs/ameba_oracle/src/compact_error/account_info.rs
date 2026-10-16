//! AccountInfo borrows with the crate's compact error representation.
//!
//! These use the same RefCells and guards as solana-account-info 2.3.0.
//! Borrow conflicts retain exactly AccountBorrowFailed; guard lifetimes and
//! aliasing remain enforced by RefCell.
use super::ProgramError;
use core::cell::{Ref, RefMut};
use solana_program::account_info::AccountInfo;

pub(crate) trait CompactAccountInfo<'info> {
    fn try_data(&self) -> Result<Ref<'_, &'info mut [u8]>, ProgramError>;
    fn try_data_mut(&self) -> Result<RefMut<'_, &'info mut [u8]>, ProgramError>;
    fn try_lamports_ref(&self) -> Result<Ref<'_, &'info mut u64>, ProgramError>;
    fn try_lamports_mut(&self) -> Result<RefMut<'_, &'info mut u64>, ProgramError>;
}

impl<'info> CompactAccountInfo<'info> for AccountInfo<'info> {
    #[inline(never)]
    fn try_data(&self) -> Result<Ref<'_, &'info mut [u8]>, ProgramError> {
        self.data
            .try_borrow()
            .map_err(|_| ProgramError::AccountBorrowFailed)
    }
    #[inline(never)]
    fn try_data_mut(&self) -> Result<RefMut<'_, &'info mut [u8]>, ProgramError> {
        self.data
            .try_borrow_mut()
            .map_err(|_| ProgramError::AccountBorrowFailed)
    }
    #[inline(never)]
    fn try_lamports_ref(&self) -> Result<Ref<'_, &'info mut u64>, ProgramError> {
        self.lamports
            .try_borrow()
            .map_err(|_| ProgramError::AccountBorrowFailed)
    }
    #[inline(never)]
    fn try_lamports_mut(&self) -> Result<RefMut<'_, &'info mut u64>, ProgramError> {
        self.lamports
            .try_borrow_mut()
            .map_err(|_| ProgramError::AccountBorrowFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn shared_and_exclusive_guards_match_the_sdk_even_for_readonly_accounts() {
        let key = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mut lamports = 7;
        let mut data = [1, 2, 3];
        let account = AccountInfo::new(
            &key,
            false,
            false,
            &mut lamports,
            &mut data,
            &owner,
            false,
            0,
        );
        let alias = account.clone();
        let compact = account.try_data().unwrap();
        let sdk = alias.try_borrow_data().unwrap();
        assert_eq!(&**compact, &**sdk);
        assert_eq!(
            alias.try_data_mut().unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            ProgramError::from(alias.try_borrow_mut_data().unwrap_err()),
            ProgramError::AccountBorrowFailed
        );
        drop((compact, sdk));
        let mut writer = account.try_data_mut().unwrap();
        writer[1] = 9;
        assert_eq!(
            alias.try_data().unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            ProgramError::from(alias.try_borrow_data().unwrap_err()),
            ProgramError::AccountBorrowFailed
        );
        drop(writer);
        assert_eq!(&**alias.try_borrow_data().unwrap(), &[1, 9, 3]);

        let compact = account.try_lamports_ref().unwrap();
        let sdk = alias.try_borrow_lamports().unwrap();
        assert_eq!(**compact, **sdk);
        assert_eq!(
            alias.try_lamports_mut().unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            ProgramError::from(alias.try_borrow_mut_lamports().unwrap_err()),
            ProgramError::AccountBorrowFailed
        );
        drop((compact, sdk));
        let mut writer = account.try_lamports_mut().unwrap();
        **writer = 11;
        assert_eq!(
            alias.try_lamports_ref().unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            ProgramError::from(alias.try_borrow_lamports().unwrap_err()),
            ProgramError::AccountBorrowFailed
        );
        drop(writer);
        assert_eq!(alias.lamports(), 11);
    }
}
