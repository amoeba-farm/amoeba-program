use super::*;

pub(super) fn load_active_oracle_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.paused {
        return Err(VaultError::ContractPaused.into());
    }
    Ok(config)
}

pub(super) fn load_current_canonical_vault_config(
    program_id: &Pubkey,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    if config_info.data_len() != VaultConfig::LEN {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout() {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    Ok(config)
}

pub(super) fn mul_bps(amount: u64, bps: u16) -> Result<u64, ProgramError> {
    let value = (amount as u128)
        .checked_mul(bps as u128)
        .ok_or(VaultError::ArithmeticOverflow)?
        / 10_000;
    u64::try_from(value).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(crate) fn source_delta_bps(
    opening_state: u64,
    current_state: u64,
) -> Result<i64, ProgramError> {
    if opening_state == 0 || current_state == 0 {
        return Err(VaultError::InvalidOracleState.into());
    }
    let difference = current_state as i128 - opening_state as i128;
    let delta = (difference * 10_000) / opening_state as i128;
    i64::try_from(delta).map_err(|_| VaultError::ArithmeticOverflow.into())
}

pub(super) fn oracle_settlement_status(
    c_raw_bps: u16,
    g_camo_bps: u16,
    g_thin_bps: u16,
) -> (u16, OracleSettlementStatus) {
    let guard = g_camo_bps.max(g_thin_bps).min(10_000) as u32;
    let c_raw = c_raw_bps.min(10_000) as u32;
    let c_settle = ((c_raw * (10_000 - guard)) / 10_000) as u16;
    let status = if c_settle >= 5_500 {
        OracleSettlementStatus::Final
    } else if c_settle >= 3_500 {
        OracleSettlementStatus::Provisional
    } else {
        OracleSettlementStatus::FrozenPendingEvidence
    };
    (c_settle, status)
}
