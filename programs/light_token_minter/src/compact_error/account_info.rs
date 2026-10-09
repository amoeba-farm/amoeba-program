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
