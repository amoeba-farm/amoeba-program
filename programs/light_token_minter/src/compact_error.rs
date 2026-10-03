//! Eight-byte program error used throughout this crate.
//!
//! `solana_program::program_error::ProgramError` is 24 bytes because its `BorshIoError` variant
//! owns a `String`, and every `?` in the program copies it. That error plumbing was the largest
//! single code-size cost in the SBF artifact. The runtime only ever receives the `u64` that a
//! Solana `ProgramError` converts into at the entrypoint, so this type stores exactly that code.
//! Every conversion is lossless for everything observable on chain: the returned error code,
//! and therefore the transaction outcome, is identical.
//!
//! The associated items mirror the Solana enum's names so handler code reads unchanged:
//! `ProgramError::MissingRequiredSignature`, `ProgramError::Custom(code)`, `.into()` and `?`.

use core::{fmt, num::NonZeroU64};
use solana_program::program_error::ProgramError as SolanaProgramError;

/// The exact Solana error code, encoded so that every constructor is one immediate store.
///
/// Builtin codes are `index << 32` (see `solana_instruction::error`); they are stored as the
/// negated index. Custom codes are stored unchanged (`Custom(0)` is the builtin `CUSTOM_ZERO`,
/// exactly as Solana encodes it). No encoding is zero, so `Result<(), ProgramError>` is eight
/// bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ProgramError(NonZeroU64);

/// Crate-wide result alias, mirroring `solana_program::entrypoint::ProgramResult`.
pub type ProgramResult = Result<(), ProgramError>;

const BUILTIN_SHIFT: u32 = 32;

#[allow(non_upper_case_globals, non_snake_case)]
impl ProgramError {
    pub const InvalidArgument: Self = Self::builtin(2);
    pub const InvalidInstructionData: Self = Self::builtin(3);
    pub const InvalidAccountData: Self = Self::builtin(4);
    pub const AccountDataTooSmall: Self = Self::builtin(5);
    pub const InsufficientFunds: Self = Self::builtin(6);
    pub const IncorrectProgramId: Self = Self::builtin(7);
    pub const MissingRequiredSignature: Self = Self::builtin(8);
    pub const AccountAlreadyInitialized: Self = Self::builtin(9);
    pub const UninitializedAccount: Self = Self::builtin(10);
    pub const NotEnoughAccountKeys: Self = Self::builtin(11);
    pub const AccountBorrowFailed: Self = Self::builtin(12);
    pub const MaxSeedLengthExceeded: Self = Self::builtin(13);
    pub const InvalidSeeds: Self = Self::builtin(14);
    /// Code of every `BorshIoError(_)`; the message never reaches the runtime.
    pub const BorshIoError: Self = Self::builtin(15);
    pub const AccountNotRentExempt: Self = Self::builtin(16);
    pub const UnsupportedSysvar: Self = Self::builtin(17);
    pub const IllegalOwner: Self = Self::builtin(18);
    pub const MaxAccountsDataAllocationsExceeded: Self = Self::builtin(19);
    pub const InvalidRealloc: Self = Self::builtin(20);
    pub const MaxInstructionTraceLengthExceeded: Self = Self::builtin(21);
    pub const BuiltinProgramsMustConsumeComputeUnits: Self = Self::builtin(22);
    pub const InvalidAccountOwner: Self = Self::builtin(23);
    pub const ArithmeticOverflow: Self = Self::builtin(24);
    pub const Immutable: Self = Self::builtin(25);
    pub const IncorrectAuthority: Self = Self::builtin(26);

    /// `ProgramError::Custom(code)`, including Solana's `Custom(0)` => `CUSTOM_ZERO` rule.
    #[inline(always)]
    pub const fn Custom(code: u32) -> Self {
        if code == 0 {
            Self::builtin(1)
        } else {
            // SAFETY: `code` is nonzero and below 2^32, so it is a valid nonzero encoding that
            // cannot collide with a negated builtin index.
            Self(unsafe { NonZeroU64::new_unchecked(code as u64) })
        }
    }

    #[inline(always)]
    const fn builtin(index: u64) -> Self {
        // SAFETY: builtin indexes are 1..=26, so the negated index is nonzero.
        Self(unsafe { NonZeroU64::new_unchecked(index.wrapping_neg()) })
    }

    /// The exact `u64` the runtime receives, equal to `u64::from(solana_program_error)`.
    #[inline]
    pub const fn code(self) -> u64 {
        let raw = self.0.get();
        if (raw as i64) < 0 {
            raw.wrapping_neg() << BUILTIN_SHIFT
        } else {
            raw
        }
    }

    /// Inverse of [`Self::code`] for every code a Solana `ProgramError` can produce.
    #[inline]
    pub const fn from_code(code: u64) -> Self {
        let index = code >> BUILTIN_SHIFT;
        if index != 0 {
            Self::builtin(index)
        } else {
            Self::Custom(code as u32)
        }
    }
}

impl From<SolanaProgramError> for ProgramError {
    #[inline]
    fn from(error: SolanaProgramError) -> Self {
        Self::from_code(u64::from(error))
    }
}

impl From<ProgramError> for SolanaProgramError {
    #[inline]
    fn from(error: ProgramError) -> Self {
        Self::from(error.code())
    }
}

impl From<ProgramError> for u64 {
    #[inline]
    fn from(error: ProgramError) -> Self {
        error.code()
    }
}

/// Same code as Solana's `ProgramError::from(borsh::io::Error)`, without formatting a message.
impl From<std::io::Error> for ProgramError {
    #[inline]
    fn from(_: std::io::Error) -> Self {
        Self::BorshIoError
    }
}

impl From<light_sdk::error::LightSdkError> for ProgramError {
    #[inline]
    fn from(error: light_sdk::error::LightSdkError) -> Self {
        SolanaProgramError::from(error).into()
    }
}

impl From<solana_program::pubkey::PubkeyError> for ProgramError {
    #[inline]
    fn from(error: solana_program::pubkey::PubkeyError) -> Self {
        SolanaProgramError::from(error).into()
    }
}

/// Cross-program invocation with the compact error. A failing callee aborts the whole
/// transaction in the runtime; only pre-invoke validation errors are returned here, and their
/// codes are preserved exactly.
pub(crate) mod cpi {
    use super::{ProgramError, ProgramResult};
    use solana_program::{account_info::AccountInfo, instruction::Instruction};

    #[inline(always)]
    pub(crate) fn invoke(
        instruction: &Instruction,
        account_infos: &[AccountInfo],
    ) -> ProgramResult {
        invoke_signed(instruction, account_infos, &[])
    }

    /// `solana_program::program::invoke_signed`, statement for statement: the same RefCell
    /// consistency check (first matching account, mutable or shared borrows), then the unchecked
    /// invoke. Only the key comparison differs: a word compare instead of a `memcmp` syscall for
    /// each of the metas x infos pairs.
    #[inline(never)]
    pub(crate) fn invoke_signed(
        instruction: &Instruction,
        account_infos: &[AccountInfo],
        signers_seeds: &[&[&[u8]]],
    ) -> ProgramResult {
        for account_meta in instruction.accounts.iter() {
            for account_info in account_infos.iter() {
                if crate::pubkey_eq(&account_meta.pubkey, account_info.key) {
                    if account_meta.is_writable {
                        let _ = account_info.try_borrow_mut_lamports()?;
                        let _ = account_info.try_borrow_mut_data()?;
                    } else {
                        let _ = account_info.try_borrow_lamports()?;
                        let _ = account_info.try_borrow_data()?;
                    }
                    break;
                }
            }
        }
        solana_program::program::invoke_signed_unchecked(instruction, account_infos, signers_seeds)
            .map_err(ProgramError::from)
    }
}

impl fmt::Debug for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&SolanaProgramError::from(*self), f)
    }
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&SolanaProgramError::from(*self), f)
    }
}

impl std::error::Error for ProgramError {}
