//! Group-wide writer liquidity. Cash remains in canonical sleeve custody.
use super::collective_binding::{
    load_collective_anchor_month_status, load_collective_group_binding,
    load_collective_market_binding,
};
use super::*;
use crate::capped_strip::{self as strip, Lane, Row};
use crate::compressed_custody::{self as cash_custody, CustodyKind};
use crate::constants::WRITER_SETTLEMENT_GROUP_PDA_SEED;
use borsh::BorshSerialize;

fn invalid() -> ProgramError {
    VaultError::InvalidAccountList.into()
}
fn checked(v: Option<u64>) -> Result<u64, ProgramError> {
    v.ok_or(VaultError::ArithmeticOverflow.into())
}

// Compact execution observes the mint and the authenticated pool inventory
// ledger. Unobserved hot staging or retirement inventory cannot be included.
fn compact_supply_reconciled(
    book: &WriterSeriesBookV1,
    index: usize,
    pool_inventory: u64,
    mint_supply: u64,
    outstanding: u64,
) -> bool {
    let record = &book.records[index];
    record.issuer_controlled_atoms == pool_inventory
        && record.total_physical_supply_atoms == mint_supply
        && record
            .external_open_interest_atoms
            .checked_add(pool_inventory)
            .and_then(|v| v.checked_add(book.individual.compressed_retired_atoms[index]))
            .and_then(|v| v.checked_add(book.individual.forfeited_atoms[index]))
            == Some(mint_supply)
        && record
            .external_open_interest_atoms
            .checked_add(pool_inventory)
            == Some(outstanding)
}

fn cash_state(
    program: &Pubkey,
    info: &AccountInfo,
    vault: &Pubkey,
    mint: &Pubkey,
) -> Result<cash_custody::CompressedCustodyV1, ProgramError> {
    let (key, bump) =
        cash_custody::derive_compressed_custody(program, CustodyKind::WriterCash, vault);
    if *info.key != key || !info.is_writable || info.executable {
        return Err(invalid());
    }
    if info.owner == program {
        cash_custody::load(
            program,
            Some(info),
            CustodyKind::WriterCash,
            vault,
            &Pubkey::default(),
            mint,
        )?
        .ok_or_else(invalid)
    } else {
        if info.owner != &system_program::id() || !info.data_is_empty() {
            return Err(invalid());
        }
        Ok(cash_custody::CompressedCustodyV1::new(
            CustodyKind::WriterCash,
            *vault,
            Pubkey::default(),
            *mint,
            bump,
        ))
    }
}

fn store(info: &AccountInfo, lane: &Lane) -> ProgramResult {
    if info.data_len() != strip::LEN {
        return Err(invalid());
    }
    store_state(info, lane)
}

fn load(
    program: &Pubkey,
    info: &AccountInfo,
    sleeve: &Pubkey,
    group: &Pubkey,
    book: &Pubkey,
    hash: &[u8; 32],
    count: u8,
) -> Result<Box<Lane>, ProgramError> {
    let (key, bump) = strip::derive(program, sleeve);
    if info.owner != program
        || info.executable
        || info.data_len() != strip::LEN
        || *info.key != key
        || !info.is_writable
    {
        return Err(invalid());
    }
    // SAFETY: The exact canonical lane allocation was checked above.
    let lane = Box::new(
        unsafe {
            <Lane as crate::fixed_codec::FixedStateDecode>::decode_fixed(&info.try_borrow_data()?)
        }
        .map_err(|_| invalid())?,
    );
    if lane.magic != strip::MAGIC
        || lane.version != 1
        || lane.bump != bump
        || lane.sleeve != *sleeve
        || lane.group != *group
        || lane.book != *book
        || lane.policy_hash != *hash
        || lane.series_count != count
        || usize::from(count) > strip::SERIES
        || lane.rows[usize::from(count)..]
            .iter()
            .any(|r| *r != Row::default())
    {
        return Err(invalid());
    }
    for row in &lane.rows[..usize::from(count)] {
        let n = usize::from(row.bin_count);
        if n > strip::BINS
            || row.bins[n..]
                .iter()
                .any(|b| *b != crate::state::WriterDlmmBinV1::default())
            || row.bins[..n].iter().any(|b| {
                b.bin_id == 0 || b.bin_id > 100 || (b.option_atoms == 0 && b.quote_atoms == 0)
            })
            || row.bins[..n].windows(2).any(|p| p[0].bin_id >= p[1].bin_id)
        {
            return Err(invalid());
        }
    }
    Ok(lane)
}

pub(super) fn manage(
    program: &Pubkey,
    a: &[AccountInfo],
    action: crate::writer_dlmm_instruction::ManageWriterDlmmV1Params,
) -> ProgramResult {
    use crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::*;
    let (hash, change) = match action {
        InitializeSharedStripLane {
            expected_policy_hash,
        } => (expected_policy_hash, None),
        SetSharedStripLiquidity {
            expected_policy_hash,
            series_index,
            entries,
        } => (expected_policy_hash, Some((series_index, entries))),
        _ => return Err(invalid()),
    };
    if a.len() != 12
        || !a[0].is_signer
        || a[1..].iter().any(|i| i.is_signer)
        || !a[0].is_writable
        || !a[8].is_writable
        || *a[9].key != system_program::id()
    {
        return Err(invalid());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    let context = load_writer_policy_context(program, &a[2], &a[3], &a[4], &a[5], Some(a[7].key))?;
    let policy = dlmm::load_policy(
        program,
        &a[6],
        &a[2],
        &context.snapshot,
        &context.book,
        true,
    )?;
    load_writer_policy_registry(program, &a[7], a[1].key)?;
    if policy.management_authority != *a[0].key
        || policy.rolling_policy_hash != hash
        || context.sleeve.vault_config != *a[1].key
        || context.sleeve.status != WriterSleeveStatus::Active
        || context.group.status != WriterSettlementGroupStatus::Active
        || current_unix_timestamp()? >= context.group.expiry_ts
        || context.group.settlement_mint != config.usdc_mint
    {
        return Err(VaultError::Unauthorized.into());
    }
    validate_vault_token_account(&a[10], &config.usdc_mint, a[2].key)?;
    let hot = validate_token_account(&a[10])?.amount;
    if context.sleeve.usdc_vault != *a[10].key {
        return Err(invalid());
    }
    let cash = cash_state(program, &a[11], a[10].key, &config.usdc_mint)?.quote_atoms;
    if hot.checked_add(cash).is_none_or(|v| {
        v < context
            .sleeve
            .accounted_asset_atoms
            .saturating_sub(policy.total_pool_quote_atoms)
    }) {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let (address, bump) = strip::derive(program, a[2].key);
    if *a[8].key != address {
        return Err(invalid());
    }
    let mut lane = if let Some((index, entries)) = change {
        let mut lane = load(
            program,
            &a[8],
            a[2].key,
            a[3].key,
            a[4].key,
            &hash,
            context.book.series_count,
        )?;
        if usize::from(index) >= usize::from(lane.series_count)
            || entries.is_empty()
            || entries.len() > strip::BINS
        {
            return Err(invalid());
        }
        let mut row = Row::default();
        row.bin_count = entries.len() as u8;
        for (i, entry) in entries.into_iter().enumerate() {
            if entry.bin_id == 0
                || entry.bin_id > 100
                || (entry.option_atoms == 0 && entry.quote_atoms == 0)
                || i > 0 && row.bins[i - 1].bin_id >= entry.bin_id
            {
                return Err(invalid());
            }
            row.bins[i] = entry;
        }
        lane.rows[usize::from(index)] = row;
        lane
    } else {
        // The compact lane adds executable terms to the existing writer book.
        // Independent pool inventory and external longs retain their owners.
        if context
            .book
            .individual
            .series
            .iter()
            .any(|s| s.issued != 0 || s.outstanding != 0)
        {
            return Err(VaultError::InvalidWriterLifecycle.into());
        }
        validate_create_only_program_account_target(program, &a[8])?;
        create_program_account(
            &a[0],
            &a[8],
            &a[9],
            program,
            strip::LEN,
            &[strip::SEED, a[2].key.as_ref(), &[bump]],
        )?;
        Box::new(Lane {
            magic: strip::MAGIC,
            version: 1,
            bump,
            sleeve: *a[2].key,
            group: *a[3].key,
            book: *a[4].key,
            policy_hash: hash,
            series_count: context.book.series_count,
            rows: [Row::default(); strip::SERIES],
            last_updated_slot: 0,
        })
    };
    lane.last_updated_slot = Clock::get()?.slot;
    store(&a[8], &lane)
}

/// Move group-owned zero-liability leaves to the existing permanent retirement
/// owners. Supply, outstanding interest, cash and the retirement ledger do not change.
pub(super) fn cleanup<'a>(
    program: &Pubkey,
    a: &[AccountInfo<'a>],
    wire: strip::Cleanup,
) -> ProgramResult {
    use crate::regular_compressed_transfer::{self as transfer, InputLeaf, OutputLeaf};
    use solana_program::instruction::AccountMeta;
    let count = wire.legs.len();
    if count <= 4 && wire.tail_proof.is_some() {
        return Err(invalid());
    }
    if !(1..=8).contains(&count)
        || a.len() != 19 + 3 * count
        || !a[0].is_signer
        || !a[0].is_writable
        || a[1..].iter().any(|i| i.is_signer)
        || [4, 8, 16, 17].iter().any(|i| !a[*i].is_writable)
        || *a[9].key != system_program::id()
        || *a[10].key != light_token_program_id()
        || *a[11].key != cpi_authority()
        || *a[12].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[13].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *a[14].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *a[15].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
        || a[16].key == a[17].key
    {
        return Err(invalid());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    let context = load_writer_policy_context(program, &a[2], &a[3], &a[4], &a[5], Some(a[7].key))?;
    let policy = dlmm::load_policy(
        program,
        &a[6],
        &a[2],
        &context.snapshot,
        &context.book,
        true,
    )?;
    load_writer_policy_registry(program, &a[7], a[1].key)?;
    let _lane = load(
        program,
        &a[8],
        a[2].key,
        a[3].key,
        a[4].key,
        &wire.expected_policy_hash,
        context.book.series_count,
    )?;
    if policy.management_authority != *a[0].key
        || policy.rolling_policy_hash != wire.expected_policy_hash
        || context.sleeve.vault_config != *a[1].key
        || config.usdc_mint != *a[18].key
        || context.group.settlement_mint != *a[18].key
    {
        return Err(VaultError::Unauthorized.into());
    }
    let mut inputs = Vec::with_capacity(count);
    let mut outputs = Vec::with_capacity(count);
    let mut roles = vec![12, 0, 11, 13, 14, 15, 9, 16, 17, 3];
    for (i, leg) in wire.legs.iter().enumerate() {
        let index = usize::from(leg.series_index);
        let n = 19 + 3 * i;
        if index >= usize::from(context.book.series_count)
            || leg.input_amount == 0
            || leg.input_amount > context.book.individual.compressed_retired_atoms[index]
            || wire.legs.iter().take(i).any(|old| {
                old.series_index == leg.series_index || old.input.leaf_index == leg.input.leaf_index
            })
            || (if i < 4 { wire.proof } else { wire.tail_proof }).is_none()
                && !leg.input.prove_by_index
        {
            return Err(invalid());
        }
        let record = context.book.records[index];
        let binding = load_collective_group_binding(
            &a[2],
            &a[4],
            a[n].key,
            &context.sleeve,
            &context.group,
            &context.book,
        )?;
        let mut market = load_collective_market_binding(program, &a[n], &binding)?.market;
        let mint = validate_canonical_market_mint(&a[n], &mut market, &a[n + 1], 0)?;
        if *a[n].key != record.market
            || *a[n + 1].key != record.contract_mint
            || mint.supply != record.total_physical_supply_atoms
            || *a[n + 2].key
                != crate::compressed_option_settlement::retirement_owner(
                    program, a[2].key, a[n].key,
                )
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        let mint = (3 + 2 * i) as u8;
        inputs.push(InputLeaf {
            owner: 2,
            amount: leg.input_amount,
            has_delegate: false,
            delegate: 0,
            mint,
            tree: 0,
            queue: 1,
            leaf_index: leg.input.leaf_index,
            root_index: leg.input.root_index,
            prove_by_index: leg.input.prove_by_index,
        });
        outputs.push(OutputLeaf {
            owner: mint + 1,
            amount: leg.input_amount,
            has_delegate: false,
            delegate: 0,
            mint,
        });
        roles.extend([n + 1, n + 2]);
    }
    let expiry = context.group.expiry_ts.to_le_bytes();
    let bump = [context.group.bump];
    let seeds = &[
        CURRENT_STATE_NAMESPACE_SEED,
        WRITER_SETTLEMENT_GROUP_PDA_SEED,
        &context.group.underlying_id,
        &expiry,
        a[18].key.as_ref(),
        &bump,
    ];
    for start in (0..count).step_by(4) {
        let end = (start + 4).min(count);
        let mut batch_roles = roles[..10].to_vec();
        batch_roles.extend_from_slice(&roles[10 + 2 * start..10 + 2 * end]);
        let metas = batch_roles
            .iter()
            .enumerate()
            .map(|(i, r)| AccountMeta {
                pubkey: *a[*r].key,
                is_signer: matches!(i, 1 | 9),
                is_writable: a[*r].is_writable,
            })
            .collect();
        let batch_inputs = inputs[start..end]
            .iter()
            .map(|x| InputLeaf {
                mint: x.mint - 2 * start as u8,
                ..*x
            })
            .collect::<Vec<_>>();
        let batch_outputs = outputs[start..end]
            .iter()
            .map(|x| OutputLeaf {
                mint: x.mint - 2 * start as u8,
                owner: x.owner - 2 * start as u8,
                ..*x
            })
            .collect::<Vec<_>>();
        let ix = transfer::instruction(
            *a[10].key,
            metas,
            1,
            if start == 0 {
                wire.proof
            } else {
                wire.tail_proof
            },
            &batch_inputs,
            &batch_outputs,
        )?;
        let mut infos = batch_roles
            .iter()
            .map(|r| a[*r].clone())
            .collect::<Vec<_>>();
        infos.push(a[10].clone());
        invoke_signed(&ix, &infos, &[seeds])?;
    }
    Ok(())
}

pub(in crate::processor) fn trade<'a>(
    program: &Pubkey,
    a: &[AccountInfo<'a>],
    wire: strip::Trade,
    owner: Pubkey,
) -> ProgramResult {
    use crate::{
        regular_compressed_transfer::{self as transfer, HotCompression, InputLeaf, OutputLeaf},
        writer_dlmm_math::{WriterDlmmBuybackLimits, WriterDlmmCash, WriterDlmmSeriesLimits},
        writer_dlmm_quote::WriterDlmmSwapPolicy,
    };
    use solana_program::instruction::AccountMeta;
    let count = wire.legs.len();
    let buy = wire.direction == 0;
    if (buy || count <= 4) && wire.tail_proof.is_some() {
        return Err(invalid());
    }
    if !(2..=8).contains(&count)
        || wire.direction > 1
        || wire.option_quantity == 0
        || wire.aggregate_quote_bound == 0
        || a.len() != strip::COMMON + 4 * count
        || *a[0].key != owner
        || *a[2].key != crate::trading_session::derive(program, &owner).0
        || !a[3].is_signer
        || !a[3].is_writable
        || !a[2].is_writable
        || [5, 7, 9, 11, 14, 15, 16, 25, 26]
            .iter()
            .any(|i| !a[*i].is_writable)
        || a[strip::COMMON..]
            .iter()
            .enumerate()
            .any(|(i, info)| !info.is_writable && (buy || i % 4 < 2))
        || *a[17].key != light_token_program_id()
        || *a[18].key != cpi_authority()
        || *a[19].key != spl_token_program_id()
        || *a[20].key != system_program::id()
        || *a[21].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[22].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *a[23].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *a[24].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
        || a[25].key == a[26].key
        || a[25..27].iter().any(|i| i.is_signer || i.executable)
    {
        return Err(invalid());
    }
    let now = current_unix_timestamp()?;
    if now > wire.deadline {
        return Err(VaultError::AmoebaDlmmDeadlineElapsed.into());
    }
    let config = load_canonical_vault_config(program, &a[4])?;
    let mut context =
        load_writer_policy_context(program, &a[5], &a[6], &a[7], &a[8], Some(a[10].key))?;
    let mut policy = dlmm::load_policy(
        program,
        &a[9],
        &a[5],
        &context.snapshot,
        &context.book,
        true,
    )?;
    load_writer_policy_registry(program, &a[10], a[4].key)?;
    let mut lane = load(
        program,
        &a[11],
        a[5].key,
        a[6].key,
        a[7].key,
        &policy.rolling_policy_hash,
        context.book.series_count,
    )?;
    if context.sleeve.vault_config != *a[4].key
        || context.sleeve.usdc_vault != *a[14].key
        || config.usdc_mint != *a[13].key
        || context.sleeve.status != WriterSleeveStatus::Active
        || context.group.status != WriterSettlementGroupStatus::Active
        || now >= context.group.expiry_ts
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let mut series = writer_book_math_series(&context.book)?;
    if !strip::validate_strip(&series, &wire.legs) {
        return Err(invalid());
    }
    validate_collateral_mint_account(&a[13], a[19].key)?;
    let (_, cash_interface_bump) = validate_spl_interface_account_with_bump(a[13].key, &a[16])?;
    validate_vault_token_account(&a[14], a[13].key, a[5].key)?;
    let hot_before = validate_token_account(&a[14])?.amount;
    let mut cash = cash_state(program, &a[15], a[14].key, a[13].key)?;
    if cash.option_atoms != 0
        || checked(hot_before.checked_add(cash.quote_atoms))?
            < checked(
                context
                    .sleeve
                    .accounted_asset_atoms
                    .checked_sub(policy.total_pool_quote_atoms),
            )?
        || wire.writer_cash.is_some() != (cash.quote_atoms > 0)
        || (buy && (wire.user_cash.is_none() || wire.user_cash_amount == 0))
        || (!buy && (wire.user_cash.is_some() || wire.user_cash_amount != 0))
    {
        return Err(invalid());
    }
    let all_witnesses = wire
        .user_cash
        .iter()
        .chain(wire.writer_cash.iter())
        .chain(wire.legs.iter().filter_map(|l| l.holder_input.as_ref()))
        .collect::<Vec<_>>();
    if wire
        .user_cash
        .iter()
        .chain(wire.writer_cash.iter())
        .any(|w| wire.proof.is_none() && !w.prove_by_index)
        || wire.legs.iter().enumerate().any(|(i, l)| {
            l.holder_input.is_some_and(|w| {
                (if i < 4 { wire.proof } else { wire.tail_proof }).is_none() && !w.prove_by_index
            })
        })
        || all_witnesses.iter().enumerate().any(|(i, w)| {
            all_witnesses
                .iter()
                .take(i)
                .any(|p| p.leaf_index == w.leaf_index)
        })
    {
        return Err(invalid());
    }
    dlmm::advance_spending_month(&mut policy, now)?;
    if series.len() > policy.series.len() {
        return Err(invalid());
    }
    let terms = policy
        .series
        .iter()
        .take(series.len())
        .map(|s| WriterDlmmSeriesLimits {
            conservative_claim_value_atoms: s.conservative_claim_value_atoms,
            seller_floor_quote_atoms: s.seller_floor_quote_atoms,
            monthly_buyback_cap_atoms: s.monthly_buyback_cap_atoms,
            transaction_buyback_cap_atoms: s.transaction_buyback_cap_atoms,
        })
        .collect::<Vec<_>>();
    let risk = dlmm::risk_limits(&context.snapshot, &context.group);
    let reserve_targets = wire
        .legs
        .iter()
        .map(|leg| usize::from(leg.series_index))
        .collect::<Vec<_>>();
    let mut reserve_grid = crate::writer_sleeve_math::PreparedWriterStrip::for_targets(
        &series,
        risk.lower_tail_max_settlement_atomic,
        risk.upper_tail_min_settlement_atomic,
        &reserve_targets,
    )
    .map_err(writer_math_error)?;
    let mut last_admitted = None;
    let mut anchor_month = None;
    let mut prepared: Vec<(Box<Market>, u64, u64)> = Vec::with_capacity(count);
    let mut gross = 0u64;
    let mut staging_evidence = Vec::with_capacity(count);
    let mut option_interface_bumps = Vec::with_capacity(count);
    for (leg_number, leg) in wire.legs.iter().enumerate() {
        let n = strip::COMMON + 4 * leg_number;
        let index = usize::from(leg.series_index);
        let record = context.book.records[index];
        let binding = load_collective_group_binding(
            &a[5],
            &a[7],
            a[n].key,
            &context.sleeve,
            &context.group,
            &context.book,
        )?;
        let mut market = load_collective_market_binding(program, &a[n], &binding)?.market;
        if anchor_month.is_none() {
            anchor_month = Some(load_collective_anchor_month_status(
                program, &a[12], &binding,
            )?);
        }
        // All bindings came from this validated group; every market remains
        // individually bound and checked against its own record above.
        if binding.anchor_market != context.group.anchor_market
            || binding.anchor_oracle_month != context.group.anchor_oracle_month
            || binding.expiry_ts != context.group.expiry_ts
        {
            return Err(invalid());
        }
        let month = anchor_month.as_ref().ok_or_else(invalid)?;
        ensure_market_value_flow_unpaused(&config, &market)?;
        ensure_oracle_game_window(&market, &month)?;
        let mint = validate_canonical_market_mint(&a[n], &mut market, &a[n + 1], 0)?;
        if *a[n + 1].key != record.contract_mint
            || *a[n].key != record.market
            || !record.active
            || market.instrument.strike_price != record.strike_price_atomic
            || market.instrument.cap_price != record.cap_or_floor_price_atomic
            || market.params.tick_size != strip::TICK
            || market.instrument.max_payout_per_contract != strip::WIDTH
            || context.book.individual.series[index].issued != 0
            || context.book.individual.series[index].outstanding != 0
            || !compact_supply_reconciled(
                &context.book,
                index,
                policy.series_pool_inventory_atoms[index],
                mint.supply,
                market_outstanding_contract_amount(&market)?,
            )
            || (buy
                && (leg.holder_input.is_some()
                    || leg.holder_input_amount != 0
                    || leg.maximum_quote_input == 0))
            || (!buy
                && (leg.holder_input.is_none() || leg.holder_input_amount < wire.option_quantity))
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        if buy {
            let (_, bump) = validate_spl_interface_account_with_bump(a[n + 1].key, &a[n + 3])?;
            option_interface_bumps.push(bump);
            staging_evidence.push(Some(custody::prepare_empty_market_staging(
                program,
                &a[n],
                &a[n + 2],
                &a[n + 1],
                &a[19],
            )?));
        } else {
            if *a[n + 2].key != system_program::id() || *a[n + 3].key != system_program::id() {
                return Err(invalid());
            }
            staging_evidence.push(None);
        }
        let series_terms = terms.get(index).ok_or_else(invalid)?;
        let (_, _, round_trip_fee) = crate::writer_dlmm_math::writer_dlmm_price_bounds(
            series_terms.seller_floor_quote_atoms,
            strip::TICK,
            policy.price_separation_ticks,
        )
        .map_err(|_| invalid())?;
        let quote_policy = WriterDlmmSwapPolicy {
            eligible: true,
            participation: context
                .sleeve
                .has_time_participation()
                .then(|| context.sleeve.participation_totals()),
            book: &series,
            series_index: index,
            cash: WriterDlmmCash {
                assets_atoms: context.sleeve.accounted_asset_atoms,
                principal_atoms: context.sleeve.writer_principal_atoms,
                allocated_lp_quote_atoms: checked(
                    policy
                        .total_pool_quote_atoms
                        .checked_sub(policy.total_uncommitted_quote_atoms),
                )?,
                pooled_quote_atoms: policy.total_pool_quote_atoms,
            },
            risk,
            buyback: WriterDlmmBuybackLimits {
                monthly_buyback_cap_atoms: policy.monthly_buyback_cap_atoms,
                transaction_buyback_cap_atoms: policy
                    .transaction_buyback_cap_atoms
                    .saturating_sub(if buy { 0 } else { gross }),
                reserve_release_spend_ratio_ppm: policy.reserve_release_spend_ratio_ppm,
                tick_size_quote_atoms: strip::TICK,
                price_separation_ticks: policy.price_separation_ticks,
                round_trip_fee_quote_atoms: round_trip_fee,
            },
            series_limits: &terms,
            month_spent_atoms: policy.monthly_spent_atoms,
            series_month_spent_atoms: policy.series_monthly_spent_atoms[index],
        };
        let prepared_reserve = reserve_grid
            .prepare(&series, index)
            .map_err(writer_math_error)?;
        let route = strip::quote_leg_prepared(
            &wire,
            leg,
            &lane.rows[index],
            &quote_policy,
            &prepared_reserve,
        )
        .map_err(|_| VaultError::InvalidAmoebaDlmmRoute)?;
        last_admitted = route.admitted_cash().cloned();
        let premium = if buy {
            route.quote.amount_in
        } else {
            route.quote.amount_out
        };
        gross = checked(gross.checked_add(premium))?;
        for fill in &route.writer_fills {
            let row = &mut lane.rows[index];
            let bin = row.bins[..usize::from(row.bin_count)]
                .iter_mut()
                .find(|b| b.bin_id == fill.bin_id)
                .ok_or_else(invalid)?;
            bin.option_atoms = fill.option_reserve_after;
            bin.quote_atoms = fill.quote_reserve_after;
        }
        let row = &mut lane.rows[index];
        let mut retained = 0;
        for i in 0..usize::from(row.bin_count) {
            if row.bins[i].option_atoms != 0 || row.bins[i].quote_atoms != 0 {
                row.bins[retained] = row.bins[i];
                retained += 1;
            }
        }
        row.bins[retained..].fill(crate::state::WriterDlmmBinV1::default());
        row.bin_count = retained as u8;
        let record = &mut context.book.records[index];
        if buy {
            context.sleeve.accounted_asset_atoms =
                checked(context.sleeve.accounted_asset_atoms.checked_add(premium))?;
            context.sleeve.locked_primary_premium_atoms = checked(
                context
                    .sleeve
                    .locked_primary_premium_atoms
                    .checked_add(premium),
            )?;
            record.primary_premium_collected_atoms =
                checked(record.primary_premium_collected_atoms.checked_add(premium))?;
            record.external_open_interest_atoms = checked(
                record
                    .external_open_interest_atoms
                    .checked_add(wire.option_quantity),
            )?;
            record.total_physical_supply_atoms = checked(
                record
                    .total_physical_supply_atoms
                    .checked_add(wire.option_quantity),
            )?;
            market.mint_accounting.total_issued = checked(
                market
                    .mint_accounting
                    .total_issued
                    .checked_add(wire.option_quantity),
            )?;
        } else {
            context.sleeve.accounted_asset_atoms =
                checked(context.sleeve.accounted_asset_atoms.checked_sub(premium))?;
            record.external_open_interest_atoms = checked(
                record
                    .external_open_interest_atoms
                    .checked_sub(wire.option_quantity),
            )?;
            context.book.individual.compressed_retired_atoms[index] = checked(
                context.book.individual.compressed_retired_atoms[index]
                    .checked_add(wire.option_quantity),
            )?;
            consume_market_contracts(&mut market, wire.option_quantity, false)?;
            policy.monthly_spent_atoms = checked(policy.monthly_spent_atoms.checked_add(premium))?;
            policy.series_monthly_spent_atoms[index] =
                checked(policy.series_monthly_spent_atoms[index].checked_add(premium))?;
        }
        series[index].external_oi_atoms = record.external_open_interest_atoms;
        reserve_grid
            .update(index, record.external_open_interest_atoms)
            .map_err(writer_math_error)?;
        record.custody_status = if record.issuer_controlled_atoms == 0 {
            WriterSeriesCustodyStatus::Closed
        } else {
            WriterSeriesCustodyStatus::Open
        };
        prepared.push((market, mint.supply, premium));
    }
    let fees = checked(crate::trading_session::SPONSOR_FEE_ATOMS.checked_mul(count as u64))?;
    let aggregate = if buy {
        checked(gross.checked_add(fees))?
    } else {
        checked(gross.checked_sub(fees))?
    };
    if (buy && (aggregate > wire.aggregate_quote_bound || aggregate > wire.user_cash_amount))
        || (!buy && aggregate < wire.aggregate_quote_bound)
    {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    dlmm::update_cash_metrics_with_admission(
        &mut context.sleeve,
        &context.group,
        &context.book,
        &context.snapshot,
        &policy,
        true,
        last_admitted.as_ref(),
    )?;
    // All pricing, geometry, funding and total limits have passed before minting.
    // Preserve the exact fill before token/Light CPI logs can exhaust the runtime
    // log allowance. This is a receipt only when the whole transaction succeeds;
    // later custody checks and CPIs still roll back every state change on failure.
    let filled = strip::Filled {
        version: 1,
        owner,
        trading_pda: *a[2].key,
        book: *a[7].key,
        lane: *a[11].key,
        direction: wire.direction,
        option_quantity: wire.option_quantity,
        aggregate_quote_atoms: aggregate,
        legs: prepared
            .iter()
            .enumerate()
            .map(|(i, p)| strip::FilledLeg {
                series_index: wire.legs[i].series_index,
                market: *a[strip::COMMON + 4 * i].key,
                mint: *a[strip::COMMON + 4 * i + 1].key,
                gross_quote_atoms: p.2,
                sponsor_fee_atoms: crate::trading_session::SPONSOR_FEE_ATOMS,
            })
            .collect(),
    };
    solana_program::log::sol_log_data(&[
        strip::FILL_EVENT,
        &filled.try_to_vec().map_err(|_| invalid())?,
    ]);
    let original_cash = cash.quote_atoms;
    if a[15].owner != program {
        let kind = [CustodyKind::WriterCash as u8];
        let bump = [cash.bump];
        invoke_create_or_allocate_account(
            &a[3],
            &a[15],
            &a[20],
            program,
            cash_custody::CompressedCustodyV1::ACCOUNT_LEN,
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                cash_custody::COMPRESSED_CUSTODY_SEED,
                &kind,
                a[14].key.as_ref(),
                &bump,
            ],
        )?;
        cash_custody::store(&a[15], &cash)?;
    }
    let mut inputs = Vec::with_capacity(count + 2);
    let mut outputs = Vec::with_capacity(2 * count + 3);
    let mut compressions = Vec::with_capacity(count);
    let leaf = |w: strip::Witness, owner: u8, amount: u64, mint: u8| InputLeaf {
        owner,
        amount,
        mint,
        has_delegate: false,
        delegate: 0,
        tree: 0,
        queue: 1,
        leaf_index: w.leaf_index,
        root_index: w.root_index,
        prove_by_index: w.prove_by_index,
    };
    let out = |owner: u8, amount: u64, mint: u8| OutputLeaf {
        owner,
        amount,
        mint,
        has_delegate: false,
        delegate: 0,
    };
    if let Some(w) = wire.user_cash {
        inputs.push(leaf(w, 2, wire.user_cash_amount, 3));
    }
    if let Some(w) = wire.writer_cash {
        inputs.push(leaf(w, 4, original_cash, 3));
    }
    if buy {
        if wire.user_cash_amount > aggregate {
            outputs.push(out(2, wire.user_cash_amount - aggregate, 3));
        }
        cash.quote_atoms = checked(original_cash.checked_add(gross))?;
    } else {
        cash.quote_atoms = original_cash.saturating_sub(gross);
        let hot_draw = gross.saturating_sub(original_cash);
        if hot_draw > hot_before {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        if hot_draw > 0 {
            compressions.push(HotCompression {
                amount: hot_draw,
                mint: 3,
                source: 7,
                authority: 8,
                pool_account_index: 9,
                pool_index: 0,
                bump: cash_interface_bump,
                decimals: 6,
            });
        }
        outputs.push(out(2, aggregate, 3));
    }
    if cash.quote_atoms > 0 {
        outputs.push(out(4, cash.quote_atoms, 3));
    }
    outputs.push(out(5, fees, 3));
    for (i, (market, _, _)) in prepared.iter().enumerate() {
        let n = strip::COMMON + 4 * i;
        let mint = (11 + 4 * i) as u8;
        if buy {
            custody::create_prepared_market_staging(
                program,
                &a[3],
                &a[n],
                &a[n + 2],
                &a[n + 1],
                &a[19],
                &a[20],
                staging_evidence[i].as_ref().ok_or_else(invalid)?,
            )?;
            let bump = [market.bump];
            let seeds = custody::market_signer_seeds(market, &bump);
            invoke_token_mint_to_checked(
                &a[19],
                &a[n + 1],
                &a[n + 2],
                &a[n],
                wire.option_quantity,
                6,
                &[&seeds],
            )?;
            compressions.push(HotCompression {
                amount: wire.option_quantity,
                mint,
                source: mint + 1,
                authority: mint + 2,
                pool_account_index: mint + 3,
                pool_index: 0,
                bump: option_interface_bumps[i],
                decimals: 6,
            });
            outputs.push(out(2, wire.option_quantity, mint));
        } else {
            let leg = &wire.legs[i];
            inputs.push(leaf(
                leg.holder_input.ok_or_else(invalid)?,
                2,
                leg.holder_input_amount,
                mint,
            ));
            if leg.holder_input_amount > wire.option_quantity {
                outputs.push(out(2, leg.holder_input_amount - wire.option_quantity, mint));
            }
            outputs.push(out(6, wire.option_quantity, mint));
        }
    }
    let mut roles = vec![
        21, 3, 18, 22, 23, 24, 20, 25, 26, 2, 13, 15, 3, 6, 14, 5, 16, 19,
    ];
    for i in 0..count {
        let n = strip::COMMON + 4 * i;
        roles.extend([n + 1, n + 2, n, n + 3]);
    }
    let trade_bump = [crate::trading_session::derive(program, &owner).1];
    let trading_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::trading_session::SEED,
        owner.as_ref(),
        &trade_bump,
    ];
    let cash_kind = [CustodyKind::WriterCash as u8];
    let cash_bump = [cash.bump];
    let cash_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        cash_custody::COMPRESSED_CUSTODY_SEED,
        &cash_kind,
        a[14].key.as_ref(),
        &cash_bump,
    ];
    let sleeve_bump = [context.sleeve.bump];
    let sleeve_seeds = writer_sleeve_signer_seeds(&context.sleeve.settlement_group, &sleeve_bump);
    let bumps = prepared.iter().map(|p| [p.0.bump]).collect::<Vec<_>>();
    let market_seeds = prepared
        .iter()
        .enumerate()
        .map(|(i, p)| custody::market_signer_seeds(&p.0, &bumps[i]))
        .collect::<Vec<_>>();
    // Light's multi-mint sum check admits at most five distinct mints. Each
    // independently proven batch holds up to four options; only the first owns
    // the shared cash movement. Both CPIs remain inside this atomic instruction.
    for start in (0..count).step_by(4) {
        let end = (start + 4).min(count);
        let selected = |mint: u8| {
            if mint == 3 {
                start == 0
            } else {
                mint >= 11
                    && usize::from((mint - 11) / 4) >= start
                    && usize::from((mint - 11) / 4) < end
            }
        };
        let remap = |index: u8| {
            if index >= 11 {
                index - 4 * start as u8
            } else {
                index
            }
        };
        let batch_inputs = inputs
            .iter()
            .filter(|x| selected(x.mint))
            .map(|x| InputLeaf {
                mint: remap(x.mint),
                ..*x
            })
            .collect::<Vec<_>>();
        let batch_outputs = outputs
            .iter()
            .filter(|x| selected(x.mint))
            .map(|x| OutputLeaf {
                mint: remap(x.mint),
                ..*x
            })
            .collect::<Vec<_>>();
        let batch_compressions = compressions
            .iter()
            .filter(|x| selected(x.mint))
            .map(|x| HotCompression {
                mint: remap(x.mint),
                source: remap(x.source),
                authority: remap(x.authority),
                pool_account_index: remap(x.pool_account_index),
                ..*x
            })
            .collect::<Vec<_>>();
        let mut batch_roles = roles[..18].to_vec();
        batch_roles.extend_from_slice(&roles[18 + 4 * start..18 + 4 * end]);
        let metas = batch_roles
            .iter()
            .enumerate()
            .map(|(i, r)| AccountMeta {
                pubkey: *a[*r].key,
                is_signer: matches!(i, 1 | 9 | 11 | 12 | 15) || buy && i >= 18 && (i - 18) % 4 == 2,
                is_writable: a[*r].is_writable,
            })
            .collect();
        let ix = transfer::instruction_with_compressions(
            *a[17].key,
            metas,
            1,
            if start == 0 {
                wire.proof
            } else {
                wire.tail_proof
            },
            &batch_inputs,
            &batch_compressions,
            &batch_outputs,
        )?;
        let mut infos = batch_roles
            .iter()
            .map(|r| a[*r].clone())
            .collect::<Vec<_>>();
        infos.push(a[17].clone());
        let mut signers = vec![trading_seeds, cash_seeds, sleeve_seeds.as_slice()];
        if buy {
            signers.extend(market_seeds[start..end].iter().map(|s| s.as_slice()));
        }
        invoke_signed(&ix, &infos, &signers)?;
    }
    cash_custody::store(&a[15], &cash)?;
    let hot_after = validate_token_account(&a[14])?.amount;
    let expected = if buy {
        checked(
            hot_before
                .checked_add(original_cash)
                .and_then(|v| v.checked_add(gross)),
        )?
    } else {
        checked(
            hot_before
                .checked_add(original_cash)
                .and_then(|v| v.checked_sub(gross)),
        )?
    };
    if checked(hot_after.checked_add(cash.quote_atoms))? != expected
        || expected
            < checked(
                context
                    .sleeve
                    .accounted_asset_atoms
                    .checked_sub(policy.total_pool_quote_atoms),
            )?
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let slot = Clock::get()?.slot;
    lane.last_updated_slot = slot;
    context.sleeve.last_updated_slot = slot;
    context.book.last_updated_slot = slot;
    context.book.book_digest = writer_book_digest(&context.book);
    for (i, (market, before, _)) in prepared.iter().enumerate() {
        let n = strip::COMMON + 4 * i;
        let index = usize::from(wire.legs[i].series_index);
        let mint = validate_mint_account(&a[n + 1], a[19].key)?;
        if mint.supply != context.book.records[index].total_physical_supply_atoms
            || mint.supply
                != if buy {
                    checked(before.checked_add(wire.option_quantity))?
                } else {
                    *before
                }
            || market_outstanding_contract_amount(market)?
                != checked(
                    context.book.records[index]
                        .external_open_interest_atoms
                        .checked_add(policy.series_pool_inventory_atoms[index]),
                )?
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        if buy {
            if validate_token_account(&a[n + 2])?.amount != 0 {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            let bump = [market.bump];
            let seeds = custody::market_signer_seeds(market, &bump);
            invoke_token_close_account(&a[19], &a[n + 2], &a[3], &a[n], &[&seeds])?;
        }
        store_state(&a[n], market.as_ref())?;
    }
    store_state(&a[5], context.sleeve.as_ref())?;
    store_state(&a[7], context.book.as_ref())?;
    store_state(&a[9], policy.as_ref())?;
    store(&a[11], &lane)?;
    Ok(())
}
