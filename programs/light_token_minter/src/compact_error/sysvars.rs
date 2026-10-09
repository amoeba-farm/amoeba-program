//! Shared sysvar reads keep the full SDK error conversion at one boundary.
//! Each call still performs a fresh sysvar read; values are never cached.
use super::ProgramError;
use solana_program::{clock::Clock, rent::Rent, sysvar::Sysvar};

#[inline(never)]
pub(crate) fn clock() -> Result<Clock, ProgramError> {
    Clock::get().map_err(ProgramError::from)
}

#[inline(never)]
pub(crate) fn slot() -> Result<u64, ProgramError> {
    Ok(clock()?.slot)
}

#[inline(never)]
pub(crate) fn rent() -> Result<Rent, ProgramError> {
    Rent::get().map_err(ProgramError::from)
}
