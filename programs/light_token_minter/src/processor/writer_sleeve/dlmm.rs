use super::*;
use crate::state::{
    derive_writer_dlmm_policy_pda, derive_writer_dlmm_position_pda, WriterDlmmPolicyV1,
    WriterDlmmPositionV1, WriterDlmmSeriesPolicyV1, WRITER_DLMM_POLICY_HASH_DOMAIN,
    WRITER_DLMM_POLICY_SEED,
};
use crate::writer_dlmm_instruction::ManageWriterDlmmV1Params;

mod liquidity;
mod swap;
pub(in crate::processor) use liquidity::{process_initialize_position, process_liquidity_action};
pub(in crate::processor) use swap::{finish_swap, load_swap_state, WriterSwapState};

pub(in crate::processor) fn risk_limits(
    snapshot: &WriterPolicySnapshotV1,
    group: &WriterSettlementGroupV1,
) -> crate::writer_dlmm_math::WriterDlmmRiskLimits {
    crate::writer_dlmm_math::WriterDlmmRiskLimits {
        operational_buffer_atoms: snapshot.operational_buffer_atoms,
        worst_drawdown_ppm: snapshot.worst_drawdown_limit,
        lower_drawdown_ppm: snapshot.lower_drawdown_limit,
        upper_drawdown_ppm: snapshot.upper_drawdown_limit,
        lower_tail_max_settlement_atomic: snapshot.lower_tail_max_settlement_atomic,
        upper_tail_min_settlement_atomic: snapshot.upper_tail_min_settlement_atomic,
        security_mode: match snapshot.security_mode {
            WriterSecurityMode::GrossExternalMaxPayout => {
                WriterMathSecurityMode::GrossExternalMaximumPayout
            }
            WriterSecurityMode::ExactExternalEnvelope => {
                WriterMathSecurityMode::ExactExternalEnvelope
            }
        },
        security_cap_atoms: group.security_cap_atoms,
    }
}

pub(in crate::processor) fn update_cash_metrics(
    sleeve: &mut WriterSleeveV1,
    group: &WriterSettlementGroupV1,
    book: &WriterSeriesBookV1,
    snapshot: &WriterPolicySnapshotV1,
    policy: &WriterDlmmPolicyV1,
    enforce: bool,
) -> ProgramResult {
    let series = writer_book_math_series(book)?;
    let limits = risk_limits(snapshot, group);
    let summary = if enforce {
        crate::writer_dlmm_math::admit_writer_dlmm_cash(
            &series,
            crate::writer_dlmm_math::WriterDlmmCash {
                assets_atoms: sleeve.accounted_asset_atoms,
                principal_atoms: sleeve.writer_principal_atoms,
                allocated_lp_quote_atoms: policy
                    .total_pool_quote_atoms
                    .checked_sub(policy.total_uncommitted_quote_atoms)
                    .ok_or(VaultError::WriterSolvencyViolation)?,
            },
            &limits,
        )
        .map_err(|_| VaultError::WriterSolvencyViolation)?
        .0
    } else {
        exact_reserve(
            &series,
            snapshot.lower_tail_max_settlement_atomic,
            snapshot.upper_tail_min_settlement_atomic,
        )
        .map_err(writer_math_error)?
    };
    sleeve.exact_reserve_atoms = summary.reserve_atoms;
    sleeve.lower_tail_reserve_atoms = summary.lower_tail_reserve_atoms;
    sleeve.upper_tail_reserve_atoms = summary.upper_tail_reserve_atoms;
    sleeve.security_exposure_atoms =
        calculate_security_exposure(limits.security_mode, &series, summary.reserve_atoms)
            .map_err(writer_math_error)?;
    Ok(())
}

pub(in crate::processor) fn initial_policy_hash(
    policy: &WriterDlmmPolicyV1,
    snapshot: &WriterPolicySnapshotV1,
) -> [u8; 32] {
    hashv(&[
        WRITER_DLMM_POLICY_HASH_DOMAIN,
        &[0],
        policy.sleeve.as_ref(),
        policy.policy_snapshot.as_ref(),
        &snapshot.policy_hash,
        &snapshot.series_family_hash,
        policy.management_authority.as_ref(),
        policy.committing_policy_authority.as_ref(),
        &policy.monthly_buyback_cap_atoms.to_le_bytes(),
        &policy.transaction_buyback_cap_atoms.to_le_bytes(),
        &policy.reserve_release_spend_ratio_ppm.to_le_bytes(),
        &policy.price_separation_ticks.to_le_bytes(),
        &[policy.series_count],
    ])
    .to_bytes()
}

pub(in crate::processor) fn append_policy_hash(
    previous: &[u8; 32],
    index: u8,
    record: &WriterSeriesRecordV1,
    terms: &WriterDlmmSeriesPolicyV1,
) -> [u8; 32] {
    hashv(&[
        WRITER_DLMM_POLICY_HASH_DOMAIN,
        &[1],
        previous,
        &[index],
        record.market.as_ref(),
        &record.payoff_digest,
        &terms.conservative_claim_value_atoms.to_le_bytes(),
        &terms.seller_floor_quote_atoms.to_le_bytes(),
        &terms.monthly_buyback_cap_atoms.to_le_bytes(),
        &terms.transaction_buyback_cap_atoms.to_le_bytes(),
    ])
    .to_bytes()
}

pub(in crate::processor) fn load_policy(
    program_id: &Pubkey,
    info: &AccountInfo,
    sleeve_info: &AccountInfo,
    snapshot: &WriterPolicySnapshotV1,
    book: &WriterSeriesBookV1,
    require_sealed: bool,
) -> Result<Box<WriterDlmmPolicyV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterDlmmPolicyV1>(
        info,
        program_id,
        WriterDlmmPolicyV1::LEN,
        VaultError::InvalidWriterPolicySnapshot,
    )?);
    let (address, bump) = derive_writer_dlmm_policy_pda(program_id, sleeve_info.key);
    if info.executable
        || *info.key != address
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.sleeve != *sleeve_info.key
        || value.policy_snapshot
            != derive_writer_policy_snapshot_pda(
                program_id,
                sleeve_info.key,
                snapshot.policy_version,
            )
            .0
        || snapshot.sleeve != *sleeve_info.key
        || value.series_count != book.series_count
        || snapshot.series_family_hash != writer_series_family_hash(book)
        || book.sleeve != *sleeve_info.key
        || !book.frozen
        || (require_sealed && !value.sealed)
        || value.reserve_release_spend_ratio_ppm > crate::constants::WRITER_RATIO_SCALE_PPM
        || value.price_separation_ticks == 0
        || crate::pubkey_is_default(&value.management_authority)
        || crate::pubkey_is_default(&value.committing_policy_authority)
        || crate::bytes32_is_zero(&value.expected_policy_hash)
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let mut rolling = initial_policy_hash(&value, snapshot);
    for index in 0..usize::from(value.appended_series_count) {
        validate_series_terms(&book.records[index], &value.series[index])?;
        rolling = append_policy_hash(
            &rolling,
            index as u8,
            &book.records[index],
            &value.series[index],
        );
    }
    if rolling != value.rolling_policy_hash {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    Ok(value)
}

/// Funding reads the already-sealed create-once record without expanding the
/// deposit's account list to the complete immutable series book.
pub(in crate::processor) fn load_funding_policy(
    program_id: &Pubkey,
    info: &AccountInfo,
    sleeve_info: &AccountInfo,
    sleeve: &WriterSleeveV1,
    snapshot: &WriterPolicySnapshotV1,
) -> Result<Box<WriterDlmmPolicyV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterDlmmPolicyV1>(
        info,
        program_id,
        WriterDlmmPolicyV1::LEN,
        VaultError::InvalidWriterPolicySnapshot,
    )?);
    let (address, bump) = derive_writer_dlmm_policy_pda(program_id, sleeve_info.key);
    if info.executable
        || info.is_signer
        || info.is_writable
        || *info.key != address
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || !value.sealed
        || value.sleeve != *sleeve_info.key
        || value.policy_snapshot != sleeve.policy_snapshot
        || snapshot.sleeve != *sleeve_info.key
        || snapshot.policy_hash != sleeve.policy_hash
        || value.series_count != sleeve.series_count
        || value.total_pool_quote_atoms != 0
        || value
            .series_pool_inventory_atoms
            .iter()
            .any(|amount| *amount != 0)
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    Ok(value)
}

pub(in crate::processor) fn load_position(
    program_id: &Pubkey,
    info: &AccountInfo,
    pool: &Pubkey,
    sleeve: &Pubkey,
    policy: &Pubkey,
    market: &Pubkey,
    series_index: u8,
) -> Result<Box<WriterDlmmPositionV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterDlmmPositionV1>(
        info,
        program_id,
        WriterDlmmPositionV1::LEN,
        VaultError::InvalidWriterSleeve,
    )?);
    let (address, bump) = derive_writer_dlmm_position_pda(program_id, pool, sleeve);
    if info.executable
        || *info.key != address
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.pool != *pool
        || value.sleeve != *sleeve
        || value.policy != *policy
        || value.market != *market
        || value.series_index != series_index
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(value)
}

/// Historical sleeves prove canonical absence; initialized records are immutable
/// program-owned policy state. This read never creates or retrofits a policy.
pub(in crate::processor) fn load_optional_policy(
    program_id: &Pubkey,
    info: &AccountInfo,
    sleeve_info: &AccountInfo,
    sleeve: &WriterSleeveV1,
) -> Result<Option<Box<WriterDlmmPolicyV1>>, ProgramError> {
    let (address, bump) = derive_writer_dlmm_policy_pda(program_id, sleeve_info.key);
    if info.is_writable || info.is_signer || info.executable || *info.key != address {
        return Err(VaultError::InvalidAccountList.into());
    }
    if info.owner != program_id {
        validate_canonical_system_zero_pda_proof(&address, info)?;
        return Ok(None);
    }
    let policy = Box::new(load_exact_zero_padded_state::<WriterDlmmPolicyV1>(
        info,
        program_id,
        WriterDlmmPolicyV1::LEN,
        VaultError::InvalidWriterPolicySnapshot,
    )?);
    if !policy.is_initialized
        || policy.bump != bump
        || !policy.has_current_layout()
        || !policy.sealed
        || policy.sleeve != *sleeve_info.key
        || policy.policy_snapshot != sleeve.policy_snapshot
        || policy.series_count != sleeve.series_count
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    Ok(Some(policy))
}

pub(in crate::processor) fn require_unwound_policy(
    policy: Option<&WriterDlmmPolicyV1>,
) -> ProgramResult {
    if policy.is_some_and(|policy| {
        policy.total_pool_quote_atoms != 0
            || policy.total_uncommitted_quote_atoms != 0
            || policy
                .series_pool_inventory_atoms
                .iter()
                .any(|amount| *amount != 0)
    }) {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    Ok(())
}

pub(in crate::processor) fn activation_sides(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    pool: &crate::ameba_dlmm_state::AmoebaDlmmPoolV1,
) -> Result<(bool, bool), ProgramError> {
    let context = load_writer_policy_context(
        program_id,
        &accounts[6],
        &accounts[7],
        &accounts[8],
        &accounts[14],
        None,
    )?;
    let optional = load_optional_policy(program_id, &accounts[13], &accounts[6], &context.sleeve)?;
    let expected_position =
        derive_writer_dlmm_position_pda(program_id, accounts[9].key, accounts[6].key).0;
    if accounts[14].is_writable
        || accounts[14].is_signer
        || accounts[15].is_writable
        || accounts[15].is_signer
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if optional.is_none() {
        validate_canonical_system_zero_pda_proof(&expected_position, &accounts[15])?;
        return Ok((false, false));
    }
    let policy = load_policy(
        program_id,
        &accounts[13],
        &accounts[6],
        &context.snapshot,
        &context.book,
        true,
    )?;
    let index = context.book.records[..usize::from(context.book.series_count)]
        .iter()
        .position(|record| record.market == *accounts[2].key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    if accounts[15].owner != program_id {
        validate_canonical_system_zero_pda_proof(&expected_position, &accounts[15])?;
        if policy.series_pool_inventory_atoms[index] != 0 {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        return Ok((false, false));
    }
    let position = load_position(
        program_id,
        &accounts[15],
        accounts[9].key,
        accounts[6].key,
        accounts[13].key,
        accounts[2].key,
        index as u8,
    )?;
    if position.option_inventory_atoms != policy.series_pool_inventory_atoms[index]
        || position.option_inventory_atoms > pool.accounted_option_reserve
        || position
            .allocated_quote_atoms
            .checked_add(position.uncommitted_quote_atoms)
            .is_none_or(|quote| {
                quote > pool.accounted_quote_reserve || quote > policy.total_pool_quote_atoms
            })
        || position.bins[..usize::from(position.bin_count)]
            .iter()
            .any(|bin| bin.bin_id > pool.maximum_bin_id)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    Ok((
        position.allocated_quote_atoms > 0,
        position.option_inventory_atoms > 0,
    ))
}

fn validate_series_terms(
    record: &WriterSeriesRecordV1,
    terms: &WriterDlmmSeriesPolicyV1,
) -> ProgramResult {
    if !record.active
        || terms.seller_floor_quote_atoms == 0
        || terms.seller_floor_quote_atoms > record.max_payout_per_contract_atoms
        || terms.conservative_claim_value_atoms > record.max_payout_per_contract_atoms
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    Ok(())
}

fn ensure_pre_capital(context: &WriterPolicyContext) -> ProgramResult {
    let sleeve = &context.sleeve;
    if sleeve.status != WriterSleeveStatus::PolicyFrozen
        || !context.book.frozen
        || sleeve.writer_principal_atoms != 0
        || sleeve.flat_par_supply_atoms != 0
        || sleeve.accounted_asset_atoms != 0
        || sleeve.locked_primary_premium_atoms != 0
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || context.book.records[..usize::from(context.book.series_count)]
            .iter()
            .any(|record| {
                record.total_physical_supply_atoms != 0
                    || record.external_open_interest_atoms != 0
                    || record.issuer_controlled_atoms != 0
            })
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    Ok(())
}

/// One fixed 9-account core for all pre-capital policy actions, excluding governance tail.
pub(in crate::processor) fn process_policy_action(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    action: ManageWriterDlmmV1Params,
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (index, account) in accounts.iter().enumerate() {
        if account.is_signer != (index == 0)
            || account.is_writable != matches!(index, 0 | 7)
            || accounts[..index]
                .iter()
                .any(|earlier| earlier.key == account.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let actor = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let sleeve_info = &accounts[3];
    let group_info = &accounts[4];
    let book_info = &accounts[5];
    let snapshot_info = &accounts[6];
    let policy_info = &accounts[7];
    let system_info = &accounts[8];
    if *system_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let _config = load_canonical_vault_config(program_id, config_info)?;
    let registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let context = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        Some(registry_info.key),
    )?;
    ensure_pre_capital(&context)?;
    if context.sleeve.vault_config != *config_info.key
        || context.sleeve.policy_snapshot != *snapshot_info.key
        || context.sleeve.policy_hash != context.snapshot.policy_hash
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let slot = Clock::get()?.slot;
    match action {
        ManageWriterDlmmV1Params::BeginPolicy(params) => {
            if registry.policy_authority != *actor.key
                || crate::pubkey_is_default(&params.management_authority)
                || crate::bytes32_is_zero(&params.expected_policy_hash)
                || params.reserve_release_spend_ratio_ppm > crate::constants::WRITER_RATIO_SCALE_PPM
                || params.price_separation_ticks == 0
            {
                return Err(VaultError::InvalidWriterPolicySnapshot.into());
            }
            let (address, bump) = derive_writer_dlmm_policy_pda(program_id, sleeve_info.key);
            if *policy_info.key != address {
                return Err(VaultError::InvalidPda.into());
            }
            validate_create_only_program_account_target(program_id, policy_info)?;
            let mut policy = Box::new(WriterDlmmPolicyV1 {
                is_initialized: true,
                bump,
                account_discriminator: WriterDlmmPolicyV1::ACCOUNT_DISCRIMINATOR,
                account_version: WriterDlmmPolicyV1::ACCOUNT_VERSION,
                sleeve: *sleeve_info.key,
                policy_snapshot: *snapshot_info.key,
                management_authority: params.management_authority,
                committing_policy_authority: *actor.key,
                expected_policy_hash: params.expected_policy_hash,
                monthly_buyback_cap_atoms: params.monthly_buyback_cap_atoms,
                transaction_buyback_cap_atoms: params.transaction_buyback_cap_atoms,
                reserve_release_spend_ratio_ppm: params.reserve_release_spend_ratio_ppm,
                price_separation_ticks: params.price_separation_ticks,
                series_count: context.book.series_count,
                created_slot: slot,
                ..WriterDlmmPolicyV1::default()
            });
            policy.rolling_policy_hash = initial_policy_hash(&policy, &context.snapshot);
            create_program_account(
                actor,
                policy_info,
                system_info,
                program_id,
                WriterDlmmPolicyV1::LEN,
                &[WRITER_DLMM_POLICY_SEED, sleeve_info.key.as_ref(), &[bump]],
            )?;
            store_state(policy_info, policy.as_ref())
        }
        ManageWriterDlmmV1Params::AppendPolicySeries {
            start_index,
            entries,
        } => {
            let mut policy = load_policy(
                program_id,
                policy_info,
                sleeve_info,
                &context.snapshot,
                &context.book,
                false,
            )?;
            if policy.sealed
                || *actor.key != policy.committing_policy_authority
                || start_index != policy.appended_series_count
                || entries.is_empty()
                || entries.len() > crate::state::WRITER_DLMM_ACTION_ENTRIES
                || usize::from(start_index) + entries.len() > usize::from(policy.series_count)
            {
                return Err(VaultError::InvalidWriterPolicySnapshot.into());
            }
            for (offset, terms) in entries.iter().enumerate() {
                let index = usize::from(start_index) + offset;
                validate_series_terms(&context.book.records[index], terms)?;
                policy.rolling_policy_hash = append_policy_hash(
                    &policy.rolling_policy_hash,
                    index as u8,
                    &context.book.records[index],
                    terms,
                );
                policy.series[index] = *terms;
            }
            policy.appended_series_count += entries.len() as u8;
            if policy.appended_series_count == policy.series_count
                && policy.rolling_policy_hash != policy.expected_policy_hash
            {
                return Err(VaultError::InvalidWriterPolicySnapshot.into());
            }
            store_state(policy_info, policy.as_ref())
        }
        ManageWriterDlmmV1Params::SealPolicy => {
            let mut policy = load_policy(
                program_id,
                policy_info,
                sleeve_info,
                &context.snapshot,
                &context.book,
                false,
            )?;
            if policy.sealed
                || policy.appended_series_count != policy.series_count
                || policy.rolling_policy_hash != policy.expected_policy_hash
            {
                return Err(VaultError::InvalidWriterPolicySnapshot.into());
            }
            policy.sealed = true;
            policy.sealed_slot = slot;
            store_state(policy_info, policy.as_ref())
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

/// This is the only budget rollover. Invocation time is a canonical chain Clock value.
pub(in crate::processor) fn advance_spending_month(
    policy: &mut WriterDlmmPolicyV1,
    now: u64,
) -> ProgramResult {
    let (_, _, day, seconds) = utc_calendar_parts(now)?;
    let offset = u64::from(day - 1)
        .checked_mul(86_400)
        .and_then(|days| days.checked_add(seconds))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let start = now
        .checked_sub(offset)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if start < policy.spending_month_start_ts {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    if start > policy.spending_month_start_ts {
        policy.spending_month_start_ts = start;
        policy.monthly_spent_atoms = 0;
        policy.series_monthly_spent_atoms.fill(0);
    }
    Ok(())
}
