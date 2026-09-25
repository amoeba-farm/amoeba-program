use super::*;

/// A deliberate managed-pool amendment, independent of immutable receipt identity.
/// Only the common config admin, current policy authority and liquidity manager
/// may enable it. No principal, custody, snapshot, receipt or payout field changes.
#[inline(never)]
pub(in crate::processor) fn process_enable_full_collateral_capacity(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    expected_policy_hash: [u8; 32],
) -> ProgramResult {
    process_capacity_amendment(program_id, accounts, expected_policy_hash, false)
}

pub(in crate::processor) fn process_enable_shared_reserve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    expected_policy_hash: [u8; 32],
) -> ProgramResult {
    process_capacity_amendment(program_id, accounts, expected_policy_hash, true)
}

#[inline(never)]
fn process_capacity_amendment(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    expected_policy_hash: [u8; 32],
    shared_reserve: bool,
) -> ProgramResult {
    // actor, config, registry, sleeve, group, book, frozen snapshot, DLMM policy.
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (index, account) in accounts.iter().enumerate() {
        if account.executable
            || account.is_signer != (index == 0)
            || account.is_writable != matches!(index, 0 | 4)
            || accounts[..index]
                .iter()
                .any(|prior| prior.key == account.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let config = load_canonical_vault_config(program_id, &accounts[1])?;
    let registry = load_writer_policy_registry(program_id, &accounts[2], accounts[1].key)?;
    let mut context = load_writer_policy_context(
        program_id,
        &accounts[3],
        &accounts[4],
        &accounts[5],
        &accounts[6],
        Some(accounts[2].key),
    )?;
    let policy = load_policy(
        program_id,
        &accounts[7],
        &accounts[3],
        &context.snapshot,
        &context.book,
        true,
    )?;
    if config.admin != *accounts[0].key
        || registry.policy_authority != *accounts[0].key
        || policy.management_authority != *accounts[0].key
    {
        return Err(VaultError::Unauthorized.into());
    }
    if context.sleeve.vault_config != *accounts[1].key
        || context.sleeve.policy_registry != *accounts[2].key
        || context.sleeve.policy_snapshot != *accounts[6].key
        || context.group.sleeve != *accounts[3].key
        || context.snapshot.policy_hash != expected_policy_hash
        || context.sleeve.policy_hash != expected_policy_hash
        || context.sleeve.scenario_set_hash != context.snapshot.scenario_set_hash
        || context.sleeve.risk_limit_hash != context.snapshot.risk_limit_hash
        || context.sleeve.security_mode != context.snapshot.security_mode
        || context.sleeve.operational_buffer_atoms != context.snapshot.operational_buffer_atoms
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let clock = Clock::get()?;
    let now =
        u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::InvalidWriterLifecycle)?;
    if config.paused
        || context.sleeve.status != WriterSleeveStatus::Active
        || context.group.status != WriterSettlementGroupStatus::Active
        || now >= context.sleeve.expiry_ts
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    if shared_reserve {
        if context.group.shared_reserve {
            return Ok(());
        }
        let mut limits = risk_limits(&context.snapshot, &context.group);
        limits.shared_reserve = true;
        let book = writer_book_math_series(&context.book)?;
        let pooled_quote_atoms = policy.total_pool_quote_atoms;
        let allocated_lp_quote_atoms = pooled_quote_atoms
            .checked_sub(policy.total_uncommitted_quote_atoms)
            .ok_or(VaultError::WriterSolvencyViolation)?;
        crate::writer_dlmm_math::admit_writer_dlmm_cash(
            &book,
            crate::writer_dlmm_math::WriterDlmmCash {
                assets_atoms: context.sleeve.accounted_asset_atoms,
                principal_atoms: context.sleeve.writer_principal_atoms,
                allocated_lp_quote_atoms,
                pooled_quote_atoms,
            },
            &limits,
        )
        .map_err(|_| VaultError::WriterSolvencyViolation)?;
        context.group.shared_reserve = true;
    } else {
        if context.group.full_collateral_capacity {
            return Ok(());
        }
        context.group.full_collateral_capacity = true;
    }
    context.group.last_updated_slot = clock.slot;
    store_state(&accounts[4], context.group.as_ref())
}
