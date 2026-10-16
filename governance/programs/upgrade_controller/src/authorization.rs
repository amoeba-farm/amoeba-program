use solana_program::account_info::AccountInfo;

use crate::{GovernanceError, GovernanceResult};

/// Enforces the complete Bootstrap V1 seat-authority runtime contract. The
/// owner is deliberately unrestricted so direct keys and signer PDAs from
/// arbitrary programs are treated identically.
pub fn validate_seat_authority(account: &AccountInfo<'_>) -> GovernanceResult<()> {
    if !account.is_signer {
        return Err(GovernanceError::MissingSeatAuthoritySignature);
    }
    if account.is_writable {
        return Err(GovernanceError::WritableSeatAuthority);
    }
    if account.executable {
        return Err(GovernanceError::ExecutableSeatAuthority);
    }
    Ok(())
}
