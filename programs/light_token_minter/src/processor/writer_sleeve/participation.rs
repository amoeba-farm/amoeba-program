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
        || !crate::is_system_program(system.key)
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
            false,
        );
    }
    if let SettleContributionCompressed {
        cash_amount,
        cash_leaf_index,
        cash_root_index,
        cash_prove_by_index,
        proof,
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
            0,
            true,
        );
    }
    let expected = match action {
        Transfer => 4,
        Split { .. } => 6,
        Close | ClosePaid => 4,
        ExpireUnactivatedV3 => 8,
        ClaimCompressed { .. } | SettleContributionCompressed { .. } => unreachable!(),
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
                Transfer => index == 2,
                Split { .. } => matches!(index, 2 | 3),
                Close | ClosePaid => matches!(index, 2 | 3),
                ExpireUnactivatedV3 => matches!(index, 2..=4),
                ClaimCompressed { .. } | SettleContributionCompressed { .. } => unreachable!(),
            };
        if info.is_signer != signer
            || (info.is_writable != writable && !(signer && info.is_writable))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if info.executable
            && !crate::is_system_program(info.key)
            && !crate::token_instruction::check_id(info.key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if accounts
            .iter()
            .take(index)
            .any(|previous| crate::pubkey_eq(previous.key, info.key))
            && !(matches!(action, Close | ClosePaid) && index == 3 && info.key == accounts[0].key)
            && !(matches!(action, Split { .. }) && index == 4 && info.key == accounts[0].key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    match action {
        Transfer | Split { .. } | Close | ClosePaid => position_action(program, accounts, action),
        ExpireUnactivatedV3 => expire_unactivated(program, accounts),
        ClaimCompressed { .. } | SettleContributionCompressed { .. } => unreachable!(),
    }
}

/// Funding-stage emptiness of everything except the series records: the
/// sleeve never activated (Funding with an Anchored group and a frozen book),
/// was never settled, and has sold, reserved, owed and spent nothing. Shared
/// by `ExpireUnactivatedV3`; with
/// `book_has_no_exposure` it is the complete zero-exposure predicate.
pub(super) fn funding_has_no_exposure(
    sleeve: &WriterSleeveV1,
    group: &WriterSettlementGroupV1,
    book: &WriterSeriesBookV1,
    policy: &crate::state::WriterDlmmPolicyV1,
) -> bool {
    sleeve.status == WriterSleeveStatus::Funding
        && group.status == WriterSettlementGroupStatus::Anchored
        && book.frozen
        && crate::pubkey_is_default(&group.signer_set)
        && group.signer_set_version == 0
        && group.finalized_slot == 0
        && crate::bytes32_is_zero(&group.final_settlement_commitment)
        && sleeve.locked_primary_premium_atoms == 0
        && sleeve.accounted_asset_atoms == sleeve.writer_principal_atoms
        && sleeve.exact_reserve_atoms == 0
        && sleeve.upper_tail_reserve_atoms == 0
        && sleeve.lower_tail_reserve_atoms == 0
        && sleeve.security_exposure_atoms == 0
        && sleeve.long_liability_initial_atoms == 0
        && sleeve.long_liability_remaining_atoms == 0
        && sleeve.settlement_finalized_slot == 0
        && policy.monthly_spent_atoms == 0
        && policy
            .series_monthly_spent_atoms
            .iter()
            .all(|value| *value == 0)
}

/// No series record has ever issued, sold, collected premium or settled.
pub(super) fn book_has_no_exposure(book: &WriterSeriesBookV1) -> bool {
    book.records[..usize::from(book.series_count)]
        .iter()
        .all(|record| {
            record.total_physical_supply_atoms == 0
                && record.issuer_controlled_atoms == 0
                && record.external_open_interest_atoms == 0
                && record.primary_premium_collected_atoms == 0
                && record.settlement_external_oi_snapshot_atoms == 0
                && record.settlement_liability_initial_atoms == 0
                && record.settlement_liability_remaining_atoms == 0
                && record.custody_status == WriterSeriesCustodyStatus::Absent
                && record.settlement_status == WriterSeriesSettlementStatus::Open
        })
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
    let clock = crate::compact_error::clock()?;
    let now =
        u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::InvalidWriterLifecycle)?;
    let policy = dlmm::load_funding_policy(program, &a[7], &a[2], &sleeve, &snapshot)?;
    validate_vault_token_account(&a[6], &sleeve.settlement_mint, a[2].key)?;
    // Every pure condition precedes the one fallible balance read, exactly as
    // in the original single expression, so error precedence is unchanged.
    if sleeve.vault_config != *a[1].key
        || sleeve.usdc_vault != *a[6].key
        || now < sleeve.expiry_ts
        || !funding_has_no_exposure(&sleeve, &group, &book, &policy)
        || validate_token_account(&a[6])?.amount < sleeve.accounted_asset_atoms
        || !book_has_no_exposure(&book)
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

/// The accounts of one Earn Fund contribution: the fund PDA is the receipt
/// owner and creator, the allocator-approved keeper pays receipt rent.
pub(super) struct ContributeAccounts<'a, 'b> {
    pub payer: &'b AccountInfo<'a>,
    pub owner: &'b AccountInfo<'a>,
    pub config: &'b AccountInfo<'a>,
    pub sleeve: &'b AccountInfo<'a>,
    pub group: &'b AccountInfo<'a>,
    pub book: &'b AccountInfo<'a>,
    pub snapshot: &'b AccountInfo<'a>,
    pub policy: &'b AccountInfo<'a>,
    pub writer_usdc: &'b AccountInfo<'a>,
    /// The canonical WriterCash sidecar of `writer_usdc`, or the system
    /// program when the sleeve has none.
    pub cash_custody: &'b AccountInfo<'a>,
    pub source_usdc: &'b AccountInfo<'a>,
    pub mint: &'b AccountInfo<'a>,
    pub receipt: &'b AccountInfo<'a>,
    pub system: &'b AccountInfo<'a>,
    pub token: &'b AccountInfo<'a>,
}

/// The pooled contribution core, reachable only through Earn Fund Allocate
/// since the direct user selector was retired: the exact validation,
/// accounting, receipt creation and source transfer of a contribution.
/// `authority_seeds` sign the source transfer for the fund PDA. Returns the
/// created receipt.
#[inline(never)]
pub(super) fn contribute_core<'a>(
    program: &Pubkey,
    a: &ContributeAccounts<'a, '_>,
    nonce: u64,
    amount: u64,
    params: &crate::earn_fund_math::FundParams,
    authority_seeds: &[&[&[u8]]],
) -> Result<WriterContributionV2, ProgramError> {
    let config = load_canonical_vault_config(program, a.config)?;
    let mut context =
        load_writer_policy_context(program, a.sleeve, a.group, a.book, a.snapshot, None)?;
    let policy = dlmm::load_policy(
        program,
        a.policy,
        a.sleeve,
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
        || sleeve.vault_config != *a.config.key
        || sleeve.usdc_vault != *a.writer_usdc.key
        || sleeve.settlement_mint != *a.mint.key
        || config.usdc_mint != *a.mint.key
        || !crate::token_instruction::check_id(a.token.key)
        || !a.sleeve.is_writable
        || !a.writer_usdc.is_writable
        || !a.source_usdc.is_writable
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    // The allocator chooses exposure, including a first entry into an Active
    // sleeve with legacy pooled writers. Existing participation accounting
    // determines the lot's share of pooled profit and loss at settlement.
    if !params.sleeve_priceable(context.book.series_count) {
        return Err(VaultError::EarnFundInvalidAllocation.into());
    }
    validate_collateral_mint_account(a.mint, a.token.key)?;
    validate_vault_token_account(a.writer_usdc, a.mint.key, a.sleeve.key)?;
    let source = validate_token_account(a.source_usdc)?;
    if source.owner != *a.owner.key
        || source.mint != *a.mint.key
        || source.state != AccountState::Initialized
        || source.amount < amount
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let before = validate_token_account(a.writer_usdc)?.amount;
    // Writer cash is the hot vault plus the canonical WriterCash sidecar,
    // exactly the backing the writer DLMM swap lane admits; a compressed-mode
    // sale credits accounted assets while its premium sits in the sidecar.
    let sidecar = if crate::is_system_program(a.cash_custody.key) {
        None
    } else {
        crate::compressed_custody::load(
            program,
            Some(a.cash_custody),
            crate::compressed_custody::CustodyKind::WriterCash,
            a.writer_usdc.key,
            &Pubkey::default(),
            a.mint.key,
        )?
    };
    if sidecar.as_ref().is_some_and(|cash| cash.option_atoms != 0) {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let cash = before
        .checked_add(sidecar.as_ref().map_or(0, |cash| cash.quote_atoms))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let required_cash = sleeve
        .accounted_asset_atoms
        .checked_sub(policy.total_pool_quote_atoms)
        .ok_or(VaultError::WriterSolvencyViolation)?;
    if cash < required_cash {
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
        sleeve: *a.sleeve.key,
        creator: *a.owner.key,
        owner: *a.owner.key,
        rent_payer: *a.payer.key,
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
    create_lot(program, a.payer, a.sleeve, a.receipt, a.system, &mut lot)?;
    invoke_token_transfer_checked(
        a.token,
        a.source_usdc,
        a.mint,
        a.writer_usdc,
        a.owner,
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        authority_seeds,
    )?;
    if validate_token_account(a.writer_usdc)?
        .amount
        .checked_sub(before)
        != Some(amount)
        || source
            .amount
            .checked_sub(validate_token_account(a.source_usdc)?.amount)
            != Some(amount)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    sleeve.last_updated_slot = crate::compact_error::slot()?;
    store_state(a.sleeve, sleeve.as_ref())?;
    Ok(lot)
}

fn position_action(
    program: &Pubkey,
    a: &[AccountInfo],
    action: WriterParticipationActionV2,
) -> ProgramResult {
    // All position actions begin with owner, sleeve, receipt.
    let sleeve = load_writer_sleeve_without_group_meta(program, &a[1])?;
    let mut lot = load_lot(program, &a[2], &a[1], &sleeve)?;
    if !matches!(action, WriterParticipationActionV2::ClosePaid) && lot.owner != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    if matches!(
        action,
        WriterParticipationActionV2::Close | WriterParticipationActionV2::ClosePaid
    ) {
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
    permissionless: bool,
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
        || (!permissionless && !a[1].is_signer)
        || !a[2].is_writable
        || !a[3].is_writable
        || !a[4].is_writable
        || !a[7].is_writable
        || !a[8].is_writable
        || !a[17].is_writable
        || !crate::light_token_instruction::is_program(a[9].key)
        || !crate::light_token_instruction::is_cpi_authority(a[10].key)
        || !crate::token_instruction::check_id(a[11].key)
        || !crate::is_system_program(a[12].key)
        || !crate::light_token_instruction::is_light_system_program(a[13].key)
        || !crate::light_token_instruction::is_registered_program(a[14].key)
        || !crate::light_token_instruction::is_compression_authority(a[15].key)
        || !crate::light_token_instruction::is_compression_program(a[16].key)
        || !matches!(sponsor_fee_atoms, 0 | SPONSORED_REDEMPTION_FEE_ATOMS)
        || (permissionless && sponsor_fee_atoms != 0)
        || (!permissionless && sponsor_fee_atoms == 0 && a[0].key != a[1].key)
        || (sponsor_fee_atoms != 0 && a[0].key == a[1].key)
        || (cash_amount == 0
            && (cash_leaf_index != 0
                || cash_root_index != 0
                || cash_prove_by_index
                || proof.is_some()
                || !crate::is_system_program(a[18].key)
                || !crate::is_system_program(a[19].key)))
        || (cash_amount != 0
            && (!a[18].is_writable
                || !a[19].is_writable
                || (cash_prove_by_index && cash_root_index != 0)
                || (!cash_prove_by_index && proof.is_none())))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Earn Fund receipts settle only through the fund's Collect, which credits
    // the fund vault and frees the fund slot. Any wallet payout to the fund PDA
    // would strand the cash and leave the slot permanently open.
    if *a[1].key == crate::earn_fund_state::derive_earn_fund(program).0 {
        return Err(VaultError::Unauthorized.into());
    }
    let accounts = ClaimAccounts {
        payer: &a[0],
        owner: a[1].key,
        sleeve: &a[2],
        receipt: &a[3],
        hot_vault: &a[4],
        mint: &a[5],
        config: &a[6],
        cash_custody: &a[7],
        spl_interface: &a[8],
        token_program: &a[11],
        system_program: &a[12],
    };
    let mut plan = claim_prepare(program, &accounts, cash_amount)?;
    let payout = plan.payout;
    let owner_payout = payout
        .checked_sub(sponsor_fee_atoms)
        .ok_or(ProgramError::InsufficientFunds)?;
    let compressed_draw = payout.min(cash_amount);
    let hot_draw = payout - compressed_draw;
    if plan.hot_before < hot_draw {
        return Err(ProgramError::InsufficientFunds);
    }
    claim_create_custody(program, &accounts, &plan)?;
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
        let bump = [plan.cash_custody.bump];
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
    let bump = [plan.sleeve.bump];
    let sleeve_seeds = writer_sleeve_signer_seeds(&plan.sleeve.settlement_group, &bump);
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
    claim_commit(&accounts, &mut plan, compressed_draw, hot_draw)
}

/// The accounts a receipt claim reads and writes, independent of where the
/// payout goes. `owner` is the receipt owner the caller has authenticated.
pub(super) struct ClaimAccounts<'a, 'b> {
    pub payer: &'b AccountInfo<'a>,
    pub owner: &'b Pubkey,
    pub sleeve: &'b AccountInfo<'a>,
    pub receipt: &'b AccountInfo<'a>,
    pub hot_vault: &'b AccountInfo<'a>,
    pub mint: &'b AccountInfo<'a>,
    pub config: &'b AccountInfo<'a>,
    pub cash_custody: &'b AccountInfo<'a>,
    pub spl_interface: &'b AccountInfo<'a>,
    pub token_program: &'b AccountInfo<'a>,
    pub system_program: &'b AccountInfo<'a>,
}

/// A validated claim: the exact `final_payout` and the state it will update.
pub(super) struct ClaimPlan {
    pub sleeve: Box<WriterSleeveV1>,
    pub lot: WriterContributionV2,
    pub cash_custody: crate::compressed_custody::CompressedCustodyV1,
    pub create_cash_custody: bool,
    pub hot_before: u64,
    pub payout: u64,
}

/// Shared claim validation: lifecycle, ownership, canonical custody, backing
/// and the exact payout. Performs no writes.
#[inline(never)]
pub(super) fn claim_prepare(
    program: &Pubkey,
    a: &ClaimAccounts,
    cash_amount: u64,
) -> Result<ClaimPlan, ProgramError> {
    use crate::compressed_custody::{self as custody, CustodyKind};
    let config = load_canonical_vault_config(program, a.config)?;
    let sleeve = load_writer_sleeve_without_group_meta(program, a.sleeve)?;
    let lot = load_lot(program, a.receipt, a.sleeve, &sleeve)?;
    if lot.owner != *a.owner
        || lot.claimed
        || !matches!(
            sleeve.status,
            WriterSleeveStatus::SettlementFinalized | WriterSleeveStatus::FundingRefunds
        )
        || sleeve.vault_config != *a.config.key
        || sleeve.usdc_vault != *a.hot_vault.key
        || sleeve.settlement_mint != *a.mint.key
        || config.usdc_mint != *a.mint.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(a.mint, a.token_program.key)?;
    validate_vault_token_account(a.hot_vault, a.mint.key, a.sleeve.key)?;
    validate_spl_interface_account(a.mint.key, a.spl_interface)?;
    let (cash_key, cash_bump) =
        custody::derive_compressed_custody(program, CustodyKind::WriterCash, a.hot_vault.key);
    let create_cash_custody = a.cash_custody.owner != program;
    let cash_custody = if create_cash_custody {
        // Original native contribution lots can have entirely hot backing and no
        // sidecar. A signed claim creates only their exact empty canonical PDA.
        if cash_amount != 0 || a.cash_custody.is_signer {
            return Err(VaultError::InvalidAccountList.into());
        }
        validate_canonical_system_zero_pda_proof(&cash_key, a.cash_custody)?;
        custody::CompressedCustodyV1::new(
            CustodyKind::WriterCash,
            *a.hot_vault.key,
            Pubkey::default(),
            *a.mint.key,
            cash_bump,
        )
    } else {
        custody::load(
            program,
            Some(a.cash_custody),
            CustodyKind::WriterCash,
            a.hot_vault.key,
            &Pubkey::default(),
            a.mint.key,
        )?
        .ok_or(VaultError::WriterSolvencyViolation)?
    };
    let hot_before = validate_token_account(a.hot_vault)?.amount;
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
    Ok(ClaimPlan {
        sleeve,
        lot,
        cash_custody,
        create_cash_custody,
        hot_before,
        payout,
    })
}

/// Create the empty canonical WriterCash sidecar a hot-only lot may lack.
pub(super) fn claim_create_custody<'a>(
    program: &Pubkey,
    a: &ClaimAccounts<'a, '_>,
    plan: &ClaimPlan,
) -> ProgramResult {
    use crate::compressed_custody::{self as custody, CustodyKind};
    if plan.create_cash_custody {
        // Lifecycle, ownership, canonical lot/mint, exact payout and full hot
        // backing were checked above before allocating or transferring rent.
        create_program_account(
            a.payer,
            a.cash_custody,
            a.system_program,
            program,
            custody::CompressedCustodyV1::ACCOUNT_LEN,
            &[
                custody::COMPRESSED_CUSTODY_SEED,
                &[CustodyKind::WriterCash as u8],
                a.hot_vault.key.as_ref(),
                &[plan.cash_custody.bump],
            ],
        )?;
    }
    Ok(())
}

/// Shared claim commit after the payout transfers: exact hot-vault delta, then
/// identical custody, sleeve and receipt accounting.
pub(super) fn claim_commit(
    a: &ClaimAccounts,
    plan: &mut ClaimPlan,
    compressed_draw: u64,
    hot_draw: u64,
) -> ProgramResult {
    if plan
        .hot_before
        .checked_sub(validate_token_account(a.hot_vault)?.amount)
        != Some(hot_draw)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    plan.cash_custody.quote_atoms = plan
        .cash_custody
        .quote_atoms
        .checked_sub(compressed_draw)
        .ok_or(VaultError::ArithmeticOverflow)?;
    crate::compressed_custody::store(a.cash_custody, &plan.cash_custody)?;
    let sleeve = &mut plan.sleeve;
    sleeve.writer_residual_remaining_atoms -= plan.payout;
    sleeve.unclaimed_principal_atoms -= plan.lot.principal;
    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(plan.payout)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.last_updated_slot = crate::compact_error::slot()?;
    plan.lot.claimed = true;
    store_state(a.sleeve, sleeve.as_ref())?;
    store_state(a.receipt, &plan.lot)
}
