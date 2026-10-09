//! Reject retired CFM month/coverage accounts before ordinary business handlers.
//! The September-only policy and month creation handlers have been retired.
use super::*;
use crate::compact_error::CompactAccountInfo;

/// Legacy entrypoints cannot operate on CFM month/coverage records. This runs
/// after known-tag classification and before any business handler or CPI.
pub(super) fn reject_cfm_accounts(program: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    for info in accounts {
        if info.owner != program
            || ![OracleMonthState::LEN, OracleSkuCoverageManifest::LEN].contains(&info.data_len())
        {
            continue;
        }
        let data = info.try_data()?;
        if (data.len() == OracleMonthState::LEN
            && data[2..5] == OracleMonthState::CFM_DISCRIMINATOR)
            || (data.len() == OracleSkuCoverageManifest::LEN
                && data[2..5] == OracleSkuCoverageManifest::CFM_DISCRIMINATOR)
        {
            return Err(VaultError::InvalidOracleState.into());
        }
    }
    Ok(())
}
