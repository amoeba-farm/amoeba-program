//! Tag 245 (localnet profiles only): clear a dev month so it can be made again on another bucket
//! set, for the same period and expiry.
//!
//! On 2026-10-20 the user moved RAMX and NANDX from module and SSD part numbers to chip buckets
//! for every month expiring after 2026-11-01, and chose to rebuild the dev chain's months in
//! place ("rebuild in place"): the futures markets keep their periods, positions and history,
//! while the months under them are made again on the chip buckets. Nothing in the program could
//! do that, because months, coverage and a product's SKU list are create-once. Two steps here:
//!
//! - **CloseAccounts** closes oracle accounts into the payer: a month and everything under it
//!   (coverage, sources, observations, links, claims, recipe and bucket indices, medians, the
//!   reward schedule and SKU pools), or a product's SKU manifest so tag 190 can register the chip
//!   list. A funded reward schedule releases its unspent reservation in the reward vault first,
//!   as an aborted month's does. It never closes the program config, the economics config, the
//!   settlement signer registries, a maturity ladder, the reward vault, a period, a CFM launch
//!   receipt, the dev council config or a participant's collateral.
//! - **ReformPeriod** writes a Live or Forming period back to Forming with no effective listing,
//!   once its month is gone, so tag 240 can bootstrap it again (or 181 form it). The expiry never
//!   changes. The futures program only reads the period's status when an order opens exposure,
//!   so its markets simply don't trade between the two steps.
//!
//! The oracle authority signs both; there is no production path to any of this.

use super::usdc::{load_oracle_usdc_reward_schedule, load_oracle_usdc_reward_vault};
use super::*;
use crate::instruction::{find_dev_council_config_address, DevResetMonthParams};
use crate::state::{
    derive_oracle_usdc_reward_vault_pda, derive_user_collateral_pda, OracleEconomicsConfig,
    OracleMaturityLadderRegistry, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
    OracleUsdcRewardVault, SettlementSignerRegistry, SettlementSignerSet, UserCollateral,
};

#[inline(never)]
pub(super) fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DevResetMonthParams,
) -> ProgramResult {
    match params {
        DevResetMonthParams::CloseAccounts => close_accounts(program_id, accounts),
        DevResetMonthParams::ReformPeriod => reform_period(program_id, accounts),
    }
}

/// Accounts: 0 [signer] oracle authority; 1 [writable] recipient of the rent; 2 config;
/// 3 [writable] reward vault; 4.. [writable] the accounts to close.
fn close_accounts(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [authority_info, recipient_info, config_info, vault_info, targets @ ..] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    if targets.is_empty() || !recipient_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    if *vault_info.key != derive_oracle_usdc_reward_vault_pda(program_id).0 {
        return Err(OracleError::InvalidOracleUsdcRewardVault.into());
    }
    let mut vault = if vault_info.owner == program_id {
        Some(load_oracle_usdc_reward_vault(program_id, vault_info)?)
    } else {
        None
    };
    let protected_keys = [
        *config_info.key,
        *vault_info.key,
        *authority_info.key,
        *recipient_info.key,
        find_dev_council_config_address(program_id).0,
    ];
    for target in targets {
        if protected_keys.contains(target.key) || target.owner != program_id {
            return Err(OracleError::InvalidAccountList.into());
        }
        if is_protected(program_id, target)? {
            return Err(OracleError::InvalidAccountList.into());
        }
        if let Some(schedule) = funded_reward_schedule(program_id, target)? {
            let vault = vault
                .as_mut()
                .ok_or(OracleError::InvalidOracleUsdcRewardVault)?;
            let mut schedule = schedule;
            super::rewards::release_aborted_reward_reservation(vault, &mut schedule)?;
        }
        close_program_account(program_id, target, recipient_info)?;
    }
    if let Some(vault) = vault.as_mut() {
        if !vault_info.is_writable {
            return Err(OracleError::InvalidOracleUsdcRewardVault.into());
        }
        vault.last_updated_slot = crate::compact_error::slot()?;
        store_state(vault_info, vault)?;
    }
    Ok(())
}

/// The kinds of account a month reset never closes.
fn is_protected(program_id: &Pubkey, target: &AccountInfo) -> Result<bool, ProgramError> {
    let data = target.try_borrow_data()?;
    if data.starts_with(&ameba_oracle_interface::Period::DISCRIMINATOR)
        || data.starts_with(&crate::cfm::RECEIPT_DISCRIMINATOR)
    {
        return Ok(true);
    }
    let discriminator = data.get(2..5);
    let protected_kind = [
        OracleConfig::ACCOUNT_DISCRIMINATOR,
        OracleEconomicsConfig::ACCOUNT_DISCRIMINATOR,
        SettlementSignerRegistry::ACCOUNT_DISCRIMINATOR,
        SettlementSignerSet::ACCOUNT_DISCRIMINATOR,
        OracleMaturityLadderRegistry::ACCOUNT_DISCRIMINATOR,
        OracleUsdcRewardVault::ACCOUNT_DISCRIMINATOR,
    ]
    .iter()
    .any(|kind| discriminator == Some(kind.as_slice()));
    if protected_kind {
        return Ok(true);
    }
    // A participant's collateral ledger has no discriminator: it is the 58-byte PDA of its owner.
    if data.len() == UserCollateral::LEN {
        let owner = Pubkey::try_from(&data[2..34]).map_err(|_| OracleError::InvalidAccountList)?;
        if derive_user_collateral_pda(program_id, &owner).0 == *target.key {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The target as a funded reward schedule with an unspent reservation, if it is one.
fn funded_reward_schedule(
    program_id: &Pubkey,
    target: &AccountInfo,
) -> Result<Option<OracleUsdcRewardSchedule>, ProgramError> {
    let is_schedule = {
        let data = target.try_borrow_data()?;
        data.len() == OracleUsdcRewardSchedule::LEN
            && data.get(2..5) == Some(OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR.as_slice())
    };
    if !is_schedule {
        return Ok(None);
    }
    let schedule: OracleUsdcRewardSchedule = load_exact_zero_padded_state(
        target,
        program_id,
        OracleUsdcRewardSchedule::LEN,
        OracleError::InvalidOracleUsdcRewardSchedule,
    )?;
    let schedule = load_oracle_usdc_reward_schedule(program_id, &schedule.month, target)?;
    Ok((schedule.phase == OracleUsdcRewardSchedulePhase::Funded
        && schedule.remaining_reward_budget > 0)
        .then_some(schedule))
}

/// Accounts: 0 [signer] oracle authority; 1 config; 2 [writable] period; 3 its month PDA, closed.
fn reform_period(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [authority_info, config_info, period_info, month_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    if !matches!(
        anchor.period.status,
        PeriodStatus::Forming | PeriodStatus::Live
    ) {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let (expected_month, _) =
        derive_oracle_month_pda(program_id, period_info.key, anchor.expiry_ts);
    if *month_info.key != expected_month {
        return Err(OracleError::InvalidOracleMonthAccount.into());
    }
    if month_info.owner == program_id {
        return Err(OracleError::AlreadyInitialized.into());
    }
    if !period_info.is_writable || period_info.owner != program_id {
        return Err(OracleError::InvalidPeriodAccount.into());
    }
    let forming = Period {
        status: PeriodStatus::Forming,
        effective_listing_ts: 0,
        ..anchor.period.clone()
    };
    forming
        .validate()
        .map_err(|_| OracleError::InvalidPeriodTransition)?;
    period_info
        .try_borrow_mut_data()?
        .copy_from_slice(&forming.encode());
    Ok(())
}
