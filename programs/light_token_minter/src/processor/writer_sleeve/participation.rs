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
    if let ClaimCompressed {
        cash_amount,
        cash_leaf_index,
        cash_root_index,
        cash_prove_by_index,
        proof,
        sponsor_fee_atoms,
    } = action
    {
        return claim_compressed(
            program,
            accounts,
            cash_amount,
            cash_leaf_index,
            cash_root_index,
            cash_prove_by_index,
            proof,
            sponsor_fee_atoms,
        );
    }
    let expected = match action {
        Contribute { .. } => 13,
        Transfer => 4,
        Split { .. } => 6,
        Close => 4,
        ExpireUnactivatedV3 => 8,
        ClaimCompressed { .. } => unreachable!(),
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
                Close => matches!(index, 2 | 3),
                ExpireUnactivatedV3 => matches!(index, 2..=4),
                ClaimCompressed { .. } => unreachable!(),
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
            .any(|previous| crate::pubkey_eq(previous.key, info.key))
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
        Transfer | Split { .. } | Close => position_action(program, accounts, action),
        ExpireUnactivatedV3 => expire_unactivated(program, accounts),
        ClaimCompressed { .. } => unreachable!(),
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
    let sleeve = load_writer_sleeve_without_group_meta(program, &a[1])?;
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
        _ => return Err(VaultError::InvalidInstructionData.into()),
    }
    store_state(&a[2], &lot)
}

/// One receipt exits into wallet-owned regular compressed USDC. A whole
/// WriterCash leaf may supply part of the payout; the rest is compressed from
/// the canonical sleeve vault. Both CPIs and the receipt state are atomic.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn claim_compressed<'a>(
    program: &Pubkey,
    a: &[AccountInfo<'a>],
    cash_amount: u64,
    cash_leaf_index: u32,
    cash_root_index: u16,
    cash_prove_by_index: bool,
    proof: Option<[u8; 128]>,
    sponsor_fee_atoms: u64,
) -> ProgramResult {
    use crate::compressed_custody::{self as custody, CustodyKind};
    use crate::compressed_option_settlement::SPONSORED_REDEMPTION_FEE_ATOMS;
    use crate::regular_compressed_transfer::{self as transfer, InputLeaf, OutputLeaf};
    use solana_program::instruction::AccountMeta;

    // payer, owner, sleeve, receipt, hot cash vault, USDC mint, config,
    // WriterCash sidecar, SPL interface, Light Token, CPI authority, SPL Token,
    // system, Light System, registered, compression authority/program, output
    // queue, cash input tree/queue. Zero cash uses system sentinels at 18/19.
    if a.len() != 20
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_signer
        || !a[2].is_writable
        || !a[3].is_writable
        || !a[4].is_writable
        || !a[7].is_writable
        || !a[8].is_writable
        || !a[17].is_writable
        || *a[9].key != light_token_program_id()
        || *a[10].key != cpi_authority()
        || *a[11].key != spl_token_program_id()
        || *a[12].key != system_program::id()
        || *a[13].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[14].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *a[15].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *a[16].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
        || !matches!(sponsor_fee_atoms, 0 | SPONSORED_REDEMPTION_FEE_ATOMS)
        || (sponsor_fee_atoms == 0 && a[0].key != a[1].key)
        || (sponsor_fee_atoms != 0 && a[0].key == a[1].key)
        || (cash_amount == 0
            && (cash_leaf_index != 0
                || cash_root_index != 0
                || cash_prove_by_index
                || proof.is_some()
                || a[18].key != &system_program::id()
                || a[19].key != &system_program::id()))
        || (cash_amount != 0
            && (!a[18].is_writable
                || !a[19].is_writable
                || (cash_prove_by_index && cash_root_index != 0)
                || (!cash_prove_by_index && proof.is_none())))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[6])?;
    let mut sleeve = load_writer_sleeve_without_group_meta(program, &a[2])?;
    let mut lot = load_lot(program, &a[3], &a[2], &sleeve)?;
    if lot.owner != *a[1].key
        || lot.claimed
        || !matches!(
            sleeve.status,
            WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::FundingRefunds
        )
        || sleeve.vault_config != *a[6].key
        || sleeve.usdc_vault != *a[4].key
        || sleeve.settlement_mint != *a[5].key
        || config.usdc_mint != *a[5].key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(&a[5], a[11].key)?;
    validate_vault_token_account(&a[4], a[5].key, a[2].key)?;
    validate_spl_interface_account(a[5].key, &a[8])?;
    let (cash_key, cash_bump) =
        custody::derive_compressed_custody(program, CustodyKind::WriterCash, a[4].key);
    let create_cash_custody = a[7].owner != program;
    let mut cash_custody = if create_cash_custody {
        // Original native contribution lots can have entirely hot backing and no
        // sidecar. A signed claim creates only their exact empty canonical PDA.
        if cash_amount != 0 || a[7].is_signer {
            return Err(VaultError::InvalidAccountList.into());
        }
        validate_canonical_system_zero_pda_proof(&cash_key, &a[7])?;
        custody::CompressedCustodyV1::new(
            CustodyKind::WriterCash,
            *a[4].key,
            Pubkey::default(),
            *a[5].key,
            cash_bump,
        )
    } else {
        custody::load(
            program,
            Some(&a[7]),
            CustodyKind::WriterCash,
            a[4].key,
            &Pubkey::default(),
            a[5].key,
        )?
        .ok_or(VaultError::WriterSolvencyViolation)?
    };
    let hot_before = validate_token_account(&a[4])?.amount;
    if cash_custody.option_atoms != 0
        || !custody::backs(
            Some(&cash_custody),
            0,
            hot_before,
            0,
            sleeve.accounted_asset_atoms,
        )
        || cash_amount > cash_custody.quote_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let payout = final_payout(
        lot.interval(),
        sleeve.settlement_principal_atoms,
        sleeve.participation_totals().capital_seconds,
        sleeve.writer_residual_initial_atoms,
    )
    .map_err(|_| VaultError::WriterSolvencyViolation)?;
    if payout > sleeve.writer_residual_remaining_atoms
        || payout > sleeve.accounted_asset_atoms
        || lot.principal > sleeve.unclaimed_principal_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let owner_payout = payout
        .checked_sub(sponsor_fee_atoms)
        .ok_or(ProgramError::InsufficientFunds)?;
    let compressed_draw = payout.min(cash_amount);
    let hot_draw = payout - compressed_draw;
    if hot_before < hot_draw {
        return Err(ProgramError::InsufficientFunds);
    }
    if create_cash_custody {
        // Lifecycle, ownership, canonical lot/mint, exact payout and full hot
        // backing were checked above before allocating or transferring rent.
        create_program_account(
            &a[0],
            &a[7],
            &a[12],
            program,
            custody::CompressedCustodyV1::ACCOUNT_LEN,
            &[
                custody::COMPRESSED_CUSTODY_SEED,
                &[CustodyKind::WriterCash as u8],
                a[4].key.as_ref(),
                &[cash_bump],
            ],
        )?;
    }
    let compressed_owner = owner_payout.min(compressed_draw);
    let compressed_fee = compressed_draw - compressed_owner;
    if cash_amount != 0 {
        // Light's fixed seven metas, then output queue, input tree/queue,
        // canonical WriterCash signer, receipt owner, sponsor, and USDC mint.
        let indices = [13, 0, 10, 14, 15, 16, 12, 17, 18, 19, 7, 1, 0, 5];
        let metas = indices
            .iter()
            .enumerate()
            .map(|(i, &n)| AccountMeta {
                pubkey: *a[n].key,
                is_writable: matches!(i, 1 | 7 | 8 | 9 | 10),
                is_signer: i == 1 || i == 10,
            })
            .collect();
        let input = InputLeaf {
            owner: 3,
            amount: cash_amount,
            has_delegate: false,
            delegate: 0,
            mint: 6,
            tree: 1,
            queue: 2,
            leaf_index: cash_leaf_index,
            prove_by_index: cash_prove_by_index,
            root_index: cash_root_index,
        };
        let mut outputs = Vec::with_capacity(3);
        for (owner, amount) in [
            (4, compressed_owner),
            (5, compressed_fee),
            (3, cash_amount - compressed_draw),
        ] {
            if amount != 0 {
                outputs.push(OutputLeaf {
                    owner,
                    amount,
                    has_delegate: false,
                    delegate: 0,
                    mint: 6,
                });
            }
        }
        let ix = transfer::instruction(*a[9].key, metas, 0, proof, &[input], &outputs)?;
        let mut infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
        infos.push(a[9].clone());
        let bump = [cash_custody.bump];
        let kind = [CustodyKind::WriterCash as u8];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            custody::COMPRESSED_CUSTODY_SEED,
            &kind,
            a[4].key.as_ref(),
            &bump,
        ];
        invoke_signed(&ix, &infos, &[seeds])?;
    }
    let bump = [sleeve.bump];
    let sleeve_seeds = writer_sleeve_signer_seeds(&sleeve.settlement_group, &bump);
    for (recipient, amount) in [
        (1usize, owner_payout - compressed_owner),
        (0usize, sponsor_fee_atoms - compressed_fee),
    ] {
        if amount == 0 {
            continue;
        }
        let ix = light_token_instruction::compress_to_wallet(
            amount,
            MarketMintAccounting::CANONICAL_DECIMALS,
            a[4].key,
            a[4].owner,
            a[2].key,
            a[0].key,
            a[5].key,
            a[recipient].key,
            a[8].key,
            [a[13].key, a[14].key, a[15].key, a[16].key, a[17].key],
        )?;
        let indices = [13, 0, 10, 14, 15, 16, 12, 17, 5, 4, 2, recipient, 8, 11, 9];
        let infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
        invoke_signed(&ix, &infos, &[&sleeve_seeds])?;
    }
    if hot_before.checked_sub(validate_token_account(&a[4])?.amount) != Some(hot_draw) {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    cash_custody.quote_atoms = cash_custody
        .quote_atoms
        .checked_sub(compressed_draw)
        .ok_or(VaultError::ArithmeticOverflow)?;
    custody::store(&a[7], &cash_custody)?;
    sleeve.writer_residual_remaining_atoms -= payout;
    sleeve.unclaimed_principal_atoms -= lot.principal;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.last_updated_slot = Clock::get()?.slot;
    lot.claimed = true;
    store_state(&a[2], sleeve.as_ref())?;
    store_state(&a[3], &lot)
}
