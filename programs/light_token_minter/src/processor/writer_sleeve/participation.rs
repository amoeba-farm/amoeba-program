use super::*;
use crate::writer_participation_math::final_payout;
use crate::writer_participation_state::{
    derive_contribution, WriterContributionV2, WriterParticipationActionV2, CONTRIBUTION_SEED,
    PARTICIPATION_VERSION,
};

pub(in crate::processor) fn admit_time_participation(
    sleeve: &WriterSleeveV1,
    assets: u64,
    reserve: u64,
) -> ProgramResult {
    if sleeve.has_time_participation()
        && !matches!(
            sleeve.status,
            WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::Closed
        )
    {
        if sleeve.writer_principal_atoms == 0 {
            return if reserve == 0 {
                Ok(())
            } else {
                Err(VaultError::WriterSolvencyViolation.into())
            };
        }
        sleeve
            .participation_totals()
            .admit(assets, reserve)
            .map_err(|_| VaultError::WriterSolvencyViolation)?;
    }
    Ok(())
}

fn load_lot(
    program: &Pubkey,
    info: &AccountInfo,
    sleeve_info: &AccountInfo,
    sleeve: &WriterSleeveV1,
) -> Result<WriterContributionV2, ProgramError> {
    let value = load_exact_zero_padded_state::<WriterContributionV2>(
        info,
        program,
        WriterContributionV2::LEN,
        VaultError::InvalidWriterSleeve,
    )?;
    let (key, bump) = derive_contribution(program, sleeve_info.key, &value.creator, value.nonce);
    let weight = value
        .interval()
        .weight()
        .map_err(|_| VaultError::InvalidWriterSleeve)?;
    if !sleeve.has_time_participation()
        || !value.initialized
        || value.bump != bump
        || *info.key != key
        || info.executable
        || info.is_signer
        || !info.is_writable
        || value.discriminator != *b"WCP"
        || value.version != PARTICIPATION_VERSION
        || value.sleeve != *sleeve_info.key
        || value.policy_version != sleeve.policy_version
        || value.policy_hash != sleeve.policy_hash
        || value.expiry_ts != sleeve.expiry_ts
        || value.entry_ts < sleeve.participation_start()
        || value.entry_ts != value.actual_deposit_ts.max(sleeve.participation_start())
        || crate::pubkey_is_default(&value.owner)
        || crate::pubkey_is_default(&value.rent_payer)
        || value
            .weight_offset
            .checked_add(weight)
            .is_none_or(|end| end > sleeve.participation_totals().capital_seconds)
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(value)
}

fn create_lot<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    sleeve: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    lot: &mut WriterContributionV2,
) -> ProgramResult {
    let (key, bump) = derive_contribution(program, sleeve.key, &lot.creator, lot.nonce);
    if *info.key != key
        || *system.key != system_program::id()
        || !payer.is_writable
        || !info.is_writable
        || info.is_signer
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    validate_create_only_program_account_target(program, info)?;
    create_program_account(
        payer,
        info,
        system,
        program,
        WriterContributionV2::LEN,
        &[
            CONTRIBUTION_SEED,
            sleeve.key.as_ref(),
            lot.creator.as_ref(),
            &lot.nonce.to_le_bytes(),
            &[bump],
        ],
    )?;
    lot.initialized = true;
    lot.bump = bump;
    lot.discriminator = *b"WCP";
    lot.version = PARTICIPATION_VERSION;
    store_state(info, lot)
}

pub(super) fn process(
    program: &Pubkey,
    accounts: &[AccountInfo],
    action: WriterParticipationActionV2,
) -> ProgramResult {
    use WriterParticipationActionV2::*;
    let expected = match action {
        Contribute { .. } => 13,
        Transfer => 4,
        Split { .. } => 6,
        Claim => 8,
        Close => 4,
        ExpireUnactivatedV3 => 8,
    };
    if accounts.len() != expected || !accounts[0].is_signer {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Every role is distinct except the owner may also receive its own rent.
    for (index, info) in accounts.iter().enumerate() {
        let owner_alias = info.key == accounts[0].key;
        let signer = owner_alias;
        let writable = owner_alias
            || match action {
                Contribute { .. } => matches!(index, 2 | 7 | 8 | 10),
                Transfer => index == 2,
                Split { .. } => matches!(index, 2 | 3),
                Claim => matches!(index, 1..=4),
                Close => matches!(index, 2 | 3),
                ExpireUnactivatedV3 => matches!(index, 2..=4),
            };
        if info.is_signer != signer
            || (info.is_writable != writable && !(signer && info.is_writable))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if info.executable
            && *info.key != system_program::id()
            && *info.key != spl_token_program_id()
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if accounts[..index]
            .iter()
            .any(|previous| previous.key == info.key)
            && !(matches!(action, Close) && index == 3 && info.key == accounts[0].key)
            && !(matches!(action, Split { .. }) && index == 4 && info.key == accounts[0].key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    match action {
        Contribute {
            nonce,
            amount_atoms,
        } => contribute(program, accounts, nonce, amount_atoms),
        Transfer | Split { .. } | Claim | Close => position_action(program, accounts, action),
        ExpireUnactivatedV3 => expire_unactivated(program, accounts),
    }
}

fn expire_unactivated(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    // cranker, config, sleeve, group, book, snapshot, USDC vault, writer policy.
    // Funding and Anchored are one-way predecessors of activation. Their joint
    // presence proves no historical activation; zero current supply alone cannot.
    let _config = load_canonical_vault_config(program, &a[1])?;
    let WriterPolicyContext {
        mut group,
        mut sleeve,
        mut book,
        snapshot,
    } = load_writer_policy_context(program, &a[2], &a[3], &a[4], &a[5], None)?;
    let clock = Clock::get()?;
    let now =
        u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::InvalidWriterLifecycle)?;
    let policy = dlmm::load_funding_policy(program, &a[7], &a[2], &sleeve, &snapshot)?;
    validate_vault_token_account(&a[6], &sleeve.settlement_mint, a[2].key)?;
    if sleeve.vault_config != *a[1].key
        || sleeve.usdc_vault != *a[6].key
        || sleeve.status != WriterSleeveStatus::Funding
        || group.status != WriterSettlementGroupStatus::Anchored
        || now < sleeve.expiry_ts
        || !book.frozen
        || !crate::pubkey_is_default(&group.signer_set)
        || group.signer_set_version != 0
        || group.finalized_slot != 0
        || !crate::bytes32_is_zero(&group.final_settlement_commitment)
        || sleeve.locked_primary_premium_atoms != 0
        || sleeve.accounted_asset_atoms != sleeve.writer_principal_atoms
        || sleeve.exact_reserve_atoms != 0
        || sleeve.upper_tail_reserve_atoms != 0
        || sleeve.lower_tail_reserve_atoms != 0
        || sleeve.security_exposure_atoms != 0
        || sleeve.long_liability_initial_atoms != 0
        || sleeve.long_liability_remaining_atoms != 0
        || sleeve.settlement_finalized_slot != 0
        || policy.monthly_spent_atoms != 0
        || policy
            .series_monthly_spent_atoms
            .iter()
            .any(|value| *value != 0)
        || validate_token_account(&a[6])?.amount < sleeve.accounted_asset_atoms
        || book.records[..usize::from(book.series_count)]
            .iter()
            .any(|record| {
                record.total_physical_supply_atoms != 0
                    || record.issuer_controlled_atoms != 0
                    || record.external_open_interest_atoms != 0
                    || record.primary_premium_collected_atoms != 0
                    || record.settlement_external_oi_snapshot_atoms != 0
                    || record.settlement_liability_initial_atoms != 0
                    || record.settlement_liability_remaining_atoms != 0
                    || record.custody_status != WriterSeriesCustodyStatus::Absent
                    || record.settlement_status != WriterSeriesSettlementStatus::Open
            })
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    // No synthetic oracle price or allocation: residual == principal makes each
    // transferred/split receipt's existing payout exactly its recorded principal.
    sleeve.settlement_principal_atoms = sleeve.writer_principal_atoms;
    sleeve.unclaimed_principal_atoms = sleeve.writer_principal_atoms;
    sleeve.writer_residual_initial_atoms = sleeve.writer_principal_atoms;
    sleeve.writer_residual_remaining_atoms = sleeve.writer_principal_atoms;
    sleeve.status = WriterSleeveStatus::FundingRefunds;
    sleeve.last_updated_slot = clock.slot;
    group.status = WriterSettlementGroupStatus::FundingExpired;
    group.last_updated_slot = clock.slot;
    for record in &mut book.records[..usize::from(book.series_count)] {
        record.settlement_status = WriterSeriesSettlementStatus::Exhausted;
    }
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = clock.slot;
    store_state(&a[2], sleeve.as_ref())?;
    store_state(&a[3], group.as_ref())?;
    store_state(&a[4], book.as_ref())
}

fn contribute(program: &Pubkey, a: &[AccountInfo], nonce: u64, amount: u64) -> ProgramResult {
    // owner, config, sleeve, group, book, snapshot, DLMM policy, writer USDC,
    // owner USDC, USDC mint, new receipt, system, classic SPL Token.
    let config = load_canonical_vault_config(program, &a[1])?;
    let mut context = load_writer_policy_context(program, &a[2], &a[3], &a[4], &a[5], None)?;
    let policy = dlmm::load_policy(
        program,
        &a[6],
        &a[2],
        &context.snapshot,
        &context.book,
        true,
    )?;
    let sleeve = &mut context.sleeve;
    let now = current_unix_timestamp()?;
    if config.paused
        || !sleeve.has_time_participation()
        || amount == 0
        || !matches!(
            sleeve.status,
            WriterSleeveStatus::Funding | WriterSleeveStatus::Active
        )
        || !matches!(
            context.group.status,
            WriterSettlementGroupStatus::Anchored | WriterSettlementGroupStatus::Active
        )
        || now >= sleeve.expiry_ts
        || sleeve.vault_config != *a[1].key
        || sleeve.usdc_vault != *a[7].key
        || sleeve.settlement_mint != *a[9].key
        || config.usdc_mint != *a[9].key
        || *a[12].key != spl_token_program_id()
        || !a[2].is_writable
        || !a[7].is_writable
        || !a[8].is_writable
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(&a[9], a[12].key)?;
    validate_vault_token_account(&a[7], a[9].key, a[2].key)?;
    let source = validate_token_account(&a[8])?;
    if source.owner != *a[0].key
        || source.mint != *a[9].key
        || source.state != AccountState::Initialized
        || source.amount < amount
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let before = validate_token_account(&a[7])?.amount;
    let required_cash = sleeve
        .accounted_asset_atoms
        .checked_sub(policy.total_pool_quote_atoms)
        .ok_or(VaultError::WriterSolvencyViolation)?;
    if before < required_cash {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let (totals, interval) = sleeve
        .participation_totals()
        .contribute(amount, now, sleeve.participation_start(), sleeve.expiry_ts)
        .map_err(|_| VaultError::WriterArithmeticAdmissionFailed)?;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.set_participation_totals(totals);
    dlmm::update_cash_metrics(
        sleeve,
        &context.group,
        &context.book,
        &context.snapshot,
        &policy,
        sleeve.status == WriterSleeveStatus::Active,
    )?;
    admit_time_participation(
        sleeve,
        sleeve.accounted_asset_atoms,
        sleeve.exact_reserve_atoms,
    )?;
    let mut lot = WriterContributionV2 {
        sleeve: *a[2].key,
        creator: *a[0].key,
        owner: *a[0].key,
        rent_payer: *a[0].key,
        nonce,
        policy_version: sleeve.policy_version,
        policy_hash: sleeve.policy_hash,
        principal: amount,
        actual_deposit_ts: now,
        entry_ts: interval.entry_ts,
        expiry_ts: interval.expiry_ts,
        weight_offset: interval.weight_offset,
        ..WriterContributionV2::default()
    };
    create_lot(program, &a[0], &a[2], &a[10], &a[11], &mut lot)?;
    invoke_token_transfer_checked(
        &a[12],
        &a[8],
        &a[9],
        &a[7],
        &a[0],
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[],
    )?;
    if validate_token_account(&a[7])?.amount.checked_sub(before) != Some(amount)
        || source
            .amount
            .checked_sub(validate_token_account(&a[8])?.amount)
            != Some(amount)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(&a[2], sleeve.as_ref())
}

fn position_action(
    program: &Pubkey,
    a: &[AccountInfo],
    action: WriterParticipationActionV2,
) -> ProgramResult {
    // All position actions begin with owner, sleeve, receipt.
    let mut sleeve = load_writer_sleeve_without_group_meta(program, &a[1])?;
    let mut lot = load_lot(program, &a[2], &a[1], &sleeve)?;
    if lot.owner != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    if let WriterParticipationActionV2::Close = action {
        if !lot.claimed || lot.rent_payer != *a[3].key {
            return Err(VaultError::InvalidWriterLifecycle.into());
        }
        return close_program_account(program, &a[2], &a[3]);
    }
    if lot.claimed {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    match action {
        WriterParticipationActionV2::Transfer => {
            if crate::pubkey_is_default(a[3].key) || a[3].executable {
                return Err(VaultError::Unauthorized.into());
            }
            lot.owner = *a[3].key;
        }
        WriterParticipationActionV2::Split {
            nonce,
            principal_atoms,
        } => {
            if crate::pubkey_is_default(a[4].key) || a[4].executable {
                return Err(VaultError::Unauthorized.into());
            }
            let (prefix, suffix) = lot
                .interval()
                .split(principal_atoms)
                .map_err(|_| VaultError::WriterArithmeticAdmissionFailed)?;
            let mut new = WriterContributionV2 {
                creator: *a[0].key,
                owner: *a[4].key,
                rent_payer: *a[0].key,
                nonce,
                principal: prefix.principal,
                weight_offset: prefix.weight_offset,
                ..lot.clone()
            };
            create_lot(program, &a[0], &a[1], &a[3], &a[5], &mut new)?;
            lot.principal = suffix.principal;
            lot.weight_offset = suffix.weight_offset;
        }
        WriterParticipationActionV2::Claim => {
            // owner, sleeve, receipt, writer USDC, owner's classic USDC ATA,
            // USDC mint, classic SPL Token, config. Claims remain possible while paused.
            let config = load_canonical_vault_config(program, &a[7])?;
            if !matches!(
                sleeve.status,
                WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::FundingRefunds
            ) || !a[1].is_writable
                || sleeve.vault_config != *a[7].key
                || sleeve.usdc_vault != *a[3].key
                || sleeve.settlement_mint != *a[5].key
                || config.usdc_mint != *a[5].key
                || *a[6].key != spl_token_program_id()
            {
                return Err(VaultError::InvalidWriterLifecycle.into());
            }
            validate_collateral_mint_account(&a[5], a[6].key)?;
            validate_vault_token_account(&a[3], a[5].key, a[1].key)?;
            let destination = validate_token_account(&a[4])?;
            let expected = crate::associated_token::get_associated_token_address_with_program_id(
                a[0].key, a[5].key, a[6].key,
            );
            if *a[4].key != expected
                || destination.owner != *a[0].key
                || destination.mint != *a[5].key
                || destination.state != AccountState::Initialized
            {
                return Err(VaultError::InvalidTokenAccount.into());
            }
            let payout = final_payout(
                lot.interval(),
                sleeve.settlement_principal_atoms,
                sleeve.participation_totals().capital_seconds,
                sleeve.writer_residual_initial_atoms,
            )
            .map_err(|_| VaultError::WriterSolvencyViolation)?;
            let before = validate_token_account(&a[3])?.amount;
            if before < sleeve.accounted_asset_atoms
                || payout > sleeve.writer_residual_remaining_atoms
                || lot.principal > sleeve.unclaimed_principal_atoms
            {
                return Err(VaultError::WriterSolvencyViolation.into());
            }
            let bump = [sleeve.bump];
            let seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
            if payout > 0 {
                invoke_token_transfer_checked(
                    &a[6],
                    &a[3],
                    &a[5],
                    &a[4],
                    &a[1],
                    payout,
                    MarketMintAccounting::CANONICAL_DECIMALS,
                    &[&seeds],
                )?;
            }
            if before.checked_sub(validate_token_account(&a[3])?.amount) != Some(payout)
                || validate_token_account(&a[4])?
                    .amount
                    .checked_sub(destination.amount)
                    != Some(payout)
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            sleeve.writer_residual_remaining_atoms -= payout;
            sleeve.unclaimed_principal_atoms -= lot.principal;
            sleeve.accounted_asset_atoms = sleeve
                .accounted_asset_atoms
                .checked_sub(payout)
                .ok_or(VaultError::ArithmeticOverflow)?;
            sleeve.last_updated_slot = Clock::get()?.slot;
            lot.claimed = true;
            store_state(&a[1], sleeve.as_ref())?;
        }
        _ => return Err(VaultError::InvalidInstructionData.into()),
    }
    store_state(&a[2], &lot)
}
