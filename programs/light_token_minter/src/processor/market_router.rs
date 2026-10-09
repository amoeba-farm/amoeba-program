//! Permissionless, value-preserving preparation of existing option liquidity.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::{
    ameba_dlmm_state::{self as ds, AmoebaDlmmLightState},
    compressed_custody as custody,
    constants::AMOEBA_DLMM_AUTHORITY_PDA_SEED,
    dlmm_order_state::{self as os, DlmmOrderBookHeader, DlmmOrderRecord},
    market_router as router, regular_compressed_transfer as transfer,
    state::{derive_writer_dlmm_position_pda, WriterDlmmPositionV1},
};
use solana_program::instruction::AccountMeta;

// Sort only compact keys and original indices. Both resident row types share
// this sorting body; their large financial payloads are moved only by the final
// permutation, never by the comparison sorter.
#[inline(never)]
fn sort_key_indices(keys: &mut [(u64, usize)]) {
    keys.sort_unstable_by_key(|row| row.0);
}

fn sort_rows_by_key<T>(rows: &mut [T], key: fn(&T) -> u64) {
    if rows.len() < 2 {
        return;
    }
    let mut keys = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        keys.push((key(row), index));
    }
    sort_key_indices(&mut keys);
    // Each original row has exactly one destination, including equal-key rows.
    // Existing source validation decides whether duplicate identities are valid.
    let mut destinations = vec![0usize; rows.len()];
    for (destination, (_, original)) in keys.into_iter().enumerate() {
        destinations[original] = destination;
    }
    for index in 0..rows.len() {
        while destinations[index] != index {
            let destination = destinations[index];
            rows.swap(index, destination);
            destinations.swap(index, destination);
        }
    }
}

fn invalid() -> ProgramError {
    VaultError::InvalidAccountList.into()
}
fn add(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_add(b)
        .ok_or(VaultError::ArithmeticOverflow.into())
}

fn light<T: AmoebaDlmmLightState>(program: &Pubkey, info: &AccountInfo) -> Result<T, ProgramError> {
    if info.owner != program
        || info.executable
        || info.data_len() != T::ACCOUNT_LEN
        || !info.is_writable
    {
        return Err(invalid());
    }
    let data = info.try_data()?;
    if data[..8] != T::LIGHT_DISCRIMINATOR {
        return Err(invalid());
    }
    unsafe { T::decode_fixed(&data[8..]) }.map_err(|_| invalid())
}
fn write_light<T: AmoebaDlmmLightState>(info: &AccountInfo, value: &T) -> ProgramResult {
    let mut data = info.try_data_mut()?;
    if !info.is_writable || data.len() != T::ACCOUNT_LEN {
        return Err(invalid());
    }
    data[..8].copy_from_slice(&T::LIGHT_DISCRIMINATOR);
    value.encode_fixed(&mut data[8..]);
    Ok(())
}
fn sidecar(
    program: &Pubkey,
    info: &AccountInfo,
    kind: custody::CustodyKind,
    parent: &Pubkey,
    option: &Pubkey,
    quote: &Pubkey,
) -> Result<Option<custody::CompressedCustodyV1>, ProgramError> {
    if *info.key != custody::derive_compressed_custody(program, kind, parent).0 {
        return Err(invalid());
    }
    if crate::is_system_program(info.owner) && info.data_is_empty() {
        return Ok(None);
    }
    custody::load(program, Some(info), kind, parent, option, quote)
}
fn hot(
    info: &AccountInfo,
    owner: &Pubkey,
    mint: &Pubkey,
    optional: bool,
) -> Result<u64, ProgramError> {
    if optional && crate::is_system_program(info.key) {
        return Ok(0);
    }
    Ok(load_canonical_light_token_account(info, owner, mint)?.amount)
}

/// Common25: payer, live Market, logical Pool, logical Book, old Pool/Book
/// custody, Pool option/quote vaults, Pool authority, Book option/quote vaults,
/// option/quote mints, Light Token, CPI authority, System, Light System,
/// registered program, compression authority/program, SPL Token, option/quote
/// interfaces, writer position (System unused), canonical config. Pages, records,
/// then packed Merkle accounts follow. No wallet or trading authority is required.
pub(super) fn prepare(program: &Pubkey, a: &[AccountInfo], p: router::Prepare) -> ProgramResult {
    let rows = usize::from(p.page_count)
        .checked_add(usize::from(p.record_count))
        .ok_or_else(invalid)?;
    let end = router::PREPARE_COMMON
        .checked_add(rows)
        .ok_or_else(invalid)?;
    if a.len() != end + usize::from(p.merkle_accounts)
        || !router::valid_prepare_direction(p.direction)
        || p.merkle_accounts < 2
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_writable
        || a[1].is_signer
        || !crate::light_token_instruction::is_program(a[13].key)
        || !crate::light_token_instruction::is_cpi_authority(a[14].key)
        || !crate::is_system_program(a[15].key)
        || !crate::token_instruction::check_id(a[20].key)
        || !crate::light_token_instruction::is_light_system_program(a[16].key)
        || !crate::light_token_instruction::is_registered_program(a[17].key)
        || !crate::light_token_instruction::is_compression_authority(a[18].key)
        || !crate::light_token_instruction::is_compression_program(a[19].key)
    {
        return Err(invalid());
    }
    let consolidate = p.direction == router::CONSOLIDATE;
    let prepare_staging = p.direction == router::PREPARE_STAGING;
    if prepare_staging
        && (rows != 0
            || p.include_book
            || p.include_writer
            || !p.batch.inputs.is_empty()
            || p.batch.proof.is_some()
            || usize::try_from(p.allocation_bytes).map_err(|_| invalid())? != a[1].data_len()
            || !a[23].is_writable)
    {
        return Err(invalid());
    }
    if consolidate
        && (rows != 0
            || p.include_book
            || p.include_writer
            || usize::try_from(p.allocation_bytes).map_err(|_| invalid())? != a[1].data_len()
            || p.batch.inputs.is_empty())
    {
        return Err(invalid());
    }
    let hash_roles = [1usize, 2, 3, 4, 5, 6, 7, 9, 10, 23];
    if p.expected_hashes.len() != hash_roles.len() + rows {
        return Err(invalid());
    }
    for (index, role) in hash_roles
        .into_iter()
        .chain(router::PREPARE_COMMON..end)
        .enumerate()
    {
        let data = a[role].try_data()?;
        if solana_program::hash::hash(&data).to_bytes() != p.expected_hashes[index] {
            return Err(invalid());
        }
    }
    let config = load_current_canonical_vault_config(program, &a[24])?;
    let market = load_valid_market(program, &a[1])?;
    let existing = router::load(program, &a[1])?;
    let mut state = if let Some(state) = existing {
        state
    } else {
        let pool = super::ameba_dlmm::load_pool(program, &a[2])?;
        router::ResidentRouterState {
            pool,
            book: None,
            records: vec![],
            bin_pages: vec![],
            writer_position: None,
            pool_option: 0,
            pool_quote: 0,
            book_option: 0,
            book_quote: 0,
            pool_hot_option: 0,
            pool_hot_quote: 0,
            book_hot_option: 0,
            book_hot_quote: 0,
        }
    };
    if state.pool_key(program) != *a[2].key
        || state.pool.market != *a[1].key
        || state.pool.quote_mint != config.usdc_mint
        || *a[11].key != state.pool.option_mint
        || *a[12].key != state.pool.quote_mint
        || *a[3].key != state.book_key(program)
    {
        return Err(invalid());
    }
    validate_collateral_mint_account(&a[11], a[20].key)?;
    validate_collateral_mint_account(&a[12], a[20].key)?;
    let pool_key = state.pool_key(program);
    let book_key = state.book_key(program);
    let (authority, authority_bump) = ds::derive_ameba_dlmm_authority_pda(program, &pool_key);
    if *a[8].key != authority
        || *a[6].key != state.pool.option_vault
        || *a[7].key != state.pool.quote_vault
        || *a[6].key != ds::derive_ameba_dlmm_vault_pda(program, &pool_key, a[11].key).0
        || *a[7].key != ds::derive_ameba_dlmm_vault_pda(program, &pool_key, a[12].key).0
    {
        return Err(invalid());
    }
    if prepare_staging {
        // This creates only the canonical SPL representation target. Existing
        // valid inventory remains unchanged; no supply, custody ledger, owner
        // trade, allocation or source row is written.
        return super::writer_sleeve::prepare_market_staging(
            program, &a[0], &a[1], &market, &a[23], &a[11], &a[20], &a[15],
        );
    }
    if p.include_book {
        let external = load_exact_zero_padded_state::<DlmmOrderBookHeader>(
            &a[3],
            program,
            DlmmOrderBookHeader::LEN,
            VaultError::InvalidAmoebaDlmmPool,
        )?;
        if external.version == 3 {
            if state.book.is_some() {
                return Err(invalid());
            }
            state.book = Some(external);
        } else if external.version != router::FORWARDED_BOOK_VERSION || state.book.is_none() {
            return Err(invalid());
        }
    }
    if state.book.is_some()
        || !crate::is_system_program(a[9].key)
        || !crate::is_system_program(a[10].key)
    {
        validate_light_associated_token_address(&book_key, a[11].key, &a[9])?;
        validate_light_associated_token_address(&book_key, a[12].key, &a[10])?;
    } else if !crate::is_system_program(a[9].key) || !crate::is_system_program(a[10].key) {
        return Err(invalid());
    }
    let mut old_pool = sidecar(
        program,
        &a[4],
        custody::CustodyKind::Pool,
        &pool_key,
        a[11].key,
        a[12].key,
    )?;
    let mut old_book = sidecar(
        program,
        &a[5],
        custody::CustodyKind::OrderBook,
        &book_key,
        a[11].key,
        a[12].key,
    )?;
    if (state.book_option != 0
        || state.book_quote != 0
        || state.book_hot_option != 0
        || state.book_hot_quote != 0
        || old_book
            .as_ref()
            .is_some_and(|v| v.option_atoms != 0 || v.quote_atoms != 0))
        && (crate::is_system_program(a[9].key) || crate::is_system_program(a[10].key))
    {
        return Err(invalid());
    }
    let old_amounts = [
        (state.total_option()?, state.total_quote()?),
        old_pool
            .as_ref()
            .map_or((0, 0), |s| (s.option_atoms, s.quote_atoms)),
        old_book
            .as_ref()
            .map_or((0, 0), |s| (s.option_atoms, s.quote_atoms)),
    ];
    if p.direction == 1 && (old_amounts[1] != (0, 0) || old_amounts[2] != (0, 0)) {
        return Err(invalid());
    }
    let hot_before = [
        hot(&a[6], &authority, a[11].key, false)?,
        hot(&a[7], &authority, a[12].key, false)?,
        hot(&a[9], &book_key, a[11].key, state.book.is_none())?,
        hot(&a[10], &book_key, a[12].key, state.book.is_none())?,
    ];
    if hot_before[0] < state.pool_hot_option
        || hot_before[1] < state.pool_hot_quote
        || hot_before[2] < state.book_hot_option
        || hot_before[3] < state.book_hot_quote
    {
        return Err(invalid());
    }
    state.pool_hot_option = hot_before[0];
    state.pool_hot_quote = hot_before[1];
    state.book_hot_option = hot_before[2];
    state.book_hot_quote = hot_before[3];
    state.pool_option = add(state.pool_option, old_amounts[1].0)?;
    state.pool_quote = add(state.pool_quote, old_amounts[1].1)?;
    state.book_option = add(state.book_option, old_amounts[2].0)?;
    state.book_quote = add(state.book_quote, old_amounts[2].1)?;
    for info in &a[router::PREPARE_COMMON..router::PREPARE_COMMON + usize::from(p.page_count)] {
        let page: ds::AmoebaDlmmBinPageV1 = light(program, info)?;
        let (key, bump) = ds::derive_ameba_dlmm_bin_page_pda(program, &pool_key, page.page_index);
        if *info.key != key || page.bump != bump || page.pool != pool_key {
            return Err(invalid());
        }
        if page.account_version == ds::AMOEBA_DLMM_ACCOUNT_VERSION {
            if state.page(page.page_index).is_some() {
                return Err(invalid());
            }
            state.bin_pages.push(page);
        } else if page.account_version != router::FORWARDED_PAGE_VERSION
            || state.page(page.page_index).is_none()
        {
            return Err(invalid());
        }
    }
    sort_rows_by_key(&mut state.bin_pages, |v| u64::from(v.page_index));
    for info in &a[router::PREPARE_COMMON + usize::from(p.page_count)..end] {
        let record = load_exact_zero_padded_state::<DlmmOrderRecord>(
            info,
            program,
            DlmmOrderRecord::LEN,
            VaultError::InvalidAmoebaDlmmPool,
        )?;
        let (key, bump) = os::derive_order_record(program, &book_key, record.order.sequence);
        if *info.key != key || record.bump != bump || record.book != book_key {
            return Err(invalid());
        }
        if record.version == 3 {
            if state.record(record.order.sequence).is_some() {
                return Err(invalid());
            }
            state.records.push(record);
        } else if record.version != router::FORWARDED_RECORD_VERSION
            || state.record(record.order.sequence).is_none()
        {
            return Err(invalid());
        }
    }
    sort_rows_by_key(&mut state.records, |v| v.order.sequence);
    if p.include_writer {
        let position = load_exact_zero_padded_state::<WriterDlmmPositionV1>(
            &a[23],
            program,
            WriterDlmmPositionV1::LEN,
            VaultError::InvalidAmoebaDlmmPool,
        )?;
        let (key, bump) = derive_writer_dlmm_position_pda(program, &pool_key, &position.sleeve);
        if *a[23].key != key
            || position.bump != bump
            || position.market != *a[1].key
            || position.pool != pool_key
        {
            return Err(invalid());
        }
        if position.account_version == WriterDlmmPositionV1::ACCOUNT_VERSION {
            if state.writer_position.is_some() {
                return Err(invalid());
            }
            state.writer_position = Some(position);
        } else if position.account_version != router::FORWARDED_WRITER_VERSION
            || state.writer_position.is_none()
        {
            return Err(invalid());
        }
    } else if !crate::is_system_program(a[23].key) {
        return Err(invalid());
    }
    state.validate(program, a[1].key, &market)?;
    let cold_before = [
        state.pool_option,
        state.pool_quote,
        state.book_option,
        state.book_quote,
    ];
    if p.direction == 0 {
        state.pool_option = add(state.pool_option, state.pool_hot_option)?;
        state.pool_quote = add(state.pool_quote, state.pool_hot_quote)?;
        state.book_option = add(state.book_option, state.book_hot_option)?;
        state.book_quote = add(state.book_quote, state.book_hot_quote)?;
        state.pool_hot_option = 0;
        state.pool_hot_quote = 0;
        state.book_hot_option = 0;
        state.book_hot_quote = 0;
    } else if p.direction == router::THAW {
        state.pool_hot_option = add(state.pool_hot_option, state.pool_option)?;
        state.pool_hot_quote = add(state.pool_hot_quote, state.pool_quote)?;
        state.book_hot_option = add(state.book_hot_option, state.book_option)?;
        state.book_hot_quote = add(state.book_hot_quote, state.book_quote)?;
        state.pool_option = 0;
        state.pool_quote = 0;
        state.book_option = 0;
        state.book_quote = 0;
    }
    let batch = &p.batch;
    // Importing more authenticated rows does not require spending an unchanged
    // resident token leaf. Only an actual representation change needs proofs.
    let metadata_only = p.direction == 0
        && hot_before == [0; 4]
        && old_amounts[1] == (0, 0)
        && old_amounts[2] == (0, 0)
        && batch.inputs.is_empty()
        && batch.proof.is_none();
    let merkle = &a[end..];
    if batch.output_tree == batch.output_queue
        || usize::from(batch.output_tree) >= merkle.len()
        || usize::from(batch.output_queue) >= merkle.len()
        || merkle
            .iter()
            .any(|v| !v.is_writable || v.is_signer || v.executable)
        || merkle
            .iter()
            .enumerate()
            .any(|(i, v)| merkle[..i].iter().any(|q| q.key == v.key))
    {
        return Err(invalid());
    }
    let mut totals = [[0u64; 2]; 3];
    let mut consumed = vec![];
    for input in &batch.inputs {
        let w = &input.witness;
        let party = usize::from(input.party);
        let asset = usize::from(input.asset);
        if party > 2
            || asset > 1
            || input.amount == 0
            || input.delegated
            || usize::from(w.tree_index) >= merkle.len()
            || usize::from(w.queue_index) >= merkle.len()
            || w.tree_index == w.queue_index
            || batch.proof.is_none() && !w.prove_by_index
        {
            return Err(invalid());
        }
        let identity = (w.tree_index, w.queue_index, w.leaf_index);
        if consumed.contains(&identity) {
            return Err(invalid());
        }
        consumed.push(identity);
        totals[party][asset] = add(totals[party][asset], input.amount)?;
    }
    for party in 0..3 {
        let expected = if metadata_only && party == 0 {
            (0, 0)
        } else {
            old_amounts[party]
        };
        // Consolidation changes neither ownership nor financial rights. Valid
        // donated tokens may exceed the liability ledger; keep those tokens
        // with the same source owner rather than inventing rights or rejecting
        // their representation maintenance.
        if !consolidate && (totals[party][1] < expected.0 || totals[party][0] < expected.1) {
            return Err(invalid());
        }
    }
    let mut roles = vec![
        (16usize, false),
        (0, true),
        (14, false),
        (17, false),
        (18, false),
        (19, false),
        (15, false),
    ];
    let mut packed = |role: usize, signer: bool| -> Result<u8, ProgramError> {
        if let Some(index) = roles[7..]
            .iter()
            .position(|(r, _)| a[*r].key == a[role].key)
        {
            roles[index + 7].1 |= signer;
            return u8::try_from(index).map_err(|_| invalid());
        }
        let index = roles.len() - 7;
        roles.push((role, signer));
        u8::try_from(index).map_err(|_| invalid())
    };
    // Transfer2's nested system CPI observes the canonical Merkle prefix as
    // well as packed indices. Preserve the same tree/queue-first layout used
    // by the existing funded-order and strip executors.
    let indexes = (end..a.len())
        .map(|r| packed(r, false))
        .collect::<Result<Vec<_>, _>>()?;
    let owners = [packed(1, true)?, packed(4, true)?, packed(5, true)?];
    let mints = [packed(12, false)?, packed(11, false)?];
    let auth = [packed(8, true)?, packed(3, true)?];
    let vaults = [
        packed(6, false)?,
        packed(7, false)?,
        packed(9, false)?,
        packed(10, false)?,
    ];
    let inputs = batch
        .inputs
        .iter()
        .map(|i| {
            let w = &i.witness;
            transfer::InputLeaf {
                owner: owners[usize::from(i.party)],
                amount: i.amount,
                mint: mints[usize::from(i.asset)],
                has_delegate: false,
                delegate: 0,
                tree: indexes[usize::from(w.tree_index)],
                queue: indexes[usize::from(w.queue_index)],
                leaf_index: w.leaf_index,
                root_index: w.root_index,
                prove_by_index: w.prove_by_index,
            }
        })
        .collect::<Vec<_>>();
    let mut compressions = vec![];
    let mut decompressions = vec![];
    let mut outputs = vec![];
    for i in 0..4 {
        if p.direction == 0 && hot_before[i] > 0 {
            compressions.push(transfer::HotCompression {
                amount: hot_before[i],
                mint: mints[if i % 2 == 0 { 1 } else { 0 }],
                source: vaults[i],
                authority: auth[i / 2],
                pool_account_index: 0,
                pool_index: 0,
                bump: 0,
                decimals: 0,
            });
        }
        if p.direction == 1 && cold_before[i] > 0 {
            decompressions.push(transfer::HotDecompression {
                amount: cold_before[i],
                mint: mints[if i % 2 == 0 { 1 } else { 0 }],
                recipient: vaults[i],
                pool_account_index: 0,
                pool_index: 0,
                bump: 0,
                decimals: 0,
            });
        }
    }
    let mut output_amounts = if consolidate {
        totals
            .iter()
            .enumerate()
            .flat_map(|(party, amounts)| {
                [
                    (owners[party], amounts[0], mints[0]),
                    (owners[party], amounts[1], mints[1]),
                ]
            })
            .collect::<Vec<_>>()
    } else {
        vec![
            (owners[0], state.total_option()?, mints[1]),
            (owners[0], state.total_quote()?, mints[0]),
        ]
    };
    if !consolidate {
        for party in 0..3 {
            let expected = if metadata_only && party == 0 {
                (0, 0)
            } else {
                old_amounts[party]
            };
            output_amounts.push((owners[party], totals[party][0] - expected.1, mints[0]));
            output_amounts.push((owners[party], totals[party][1] - expected.0, mints[1]));
        }
    }
    for (owner, amount, mint) in output_amounts {
        if amount > 0 {
            outputs.push(transfer::OutputLeaf {
                owner,
                amount,
                mint,
                has_delegate: false,
                delegate: 0,
            });
        }
    }
    if !inputs.is_empty() || !compressions.is_empty() || !decompressions.is_empty() {
        let metas = roles
            .iter()
            .map(|(r, signer)| AccountMeta {
                pubkey: *a[*r].key,
                is_signer: *signer,
                is_writable: a[*r].is_writable,
            })
            .collect();
        let ix = transfer::instruction_with_hot_actions(
            *a[13].key,
            metas,
            indexes[usize::from(batch.output_queue)],
            batch.proof,
            &inputs,
            &compressions,
            &decompressions,
            &outputs,
        )?;
        let mut infos = roles.iter().map(|(r, _)| a[*r].clone()).collect::<Vec<_>>();
        infos.push(a[13].clone());
        let market_bump = [market.bump];
        let authority_bump = [authority_bump];
        let book_bump = [os::derive_order_book(program, &pool_key).1];
        let pool_kind = [custody::CustodyKind::Pool as u8];
        let book_kind = [custody::CustodyKind::OrderBook as u8];
        let pool_custody_bump =
            [
                custody::derive_compressed_custody(program, custody::CustodyKind::Pool, &pool_key)
                    .1,
            ];
        let book_custody_bump = [custody::derive_compressed_custody(
            program,
            custody::CustodyKind::OrderBook,
            &book_key,
        )
        .1];
        invoke_signed(
            &ix,
            &infos,
            &[
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    MARKET_PDA_SEED,
                    &market.market_id,
                    &market_bump,
                ],
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_AUTHORITY_PDA_SEED,
                    pool_key.as_ref(),
                    &authority_bump,
                ],
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    os::ORDER_BOOK_SEED,
                    pool_key.as_ref(),
                    &book_bump,
                ],
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    custody::COMPRESSED_CUSTODY_SEED,
                    &pool_kind,
                    pool_key.as_ref(),
                    &pool_custody_bump,
                ],
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    custody::COMPRESSED_CUSTODY_SEED,
                    &book_kind,
                    book_key.as_ref(),
                    &book_custody_bump,
                ],
            ],
        )?;
    }
    // A representation-only sum has no state, ledger, hot-vault, forwarding,
    // allocation or claim effect. All source hashes were checked before CPI.
    if consolidate {
        return Ok(());
    }
    for (i, (role, owner, mint, optional)) in [
        (6, &authority, a[11].key, false),
        (7, &authority, a[12].key, false),
        (9, &book_key, a[11].key, state.book.is_none()),
        (10, &book_key, a[12].key, state.book.is_none()),
    ]
    .into_iter()
    .enumerate()
    {
        let after = hot(&a[role], owner, mint, optional)?;
        let expected = if p.direction == 0 {
            0
        } else {
            add(hot_before[i], cold_before[i])?
        };
        if after != expected {
            return Err(VaultError::AmoebaDlmmInvariantViolation.into());
        }
    }
    for (info, value) in [(&a[4], &mut old_pool), (&a[5], &mut old_book)] {
        if let Some(v) = value {
            v.option_atoms = 0;
            v.quote_atoms = 0;
            custody::store(info, v)?;
        }
    }
    let payload = borsh::to_vec(&state).map_err(|_| invalid())?;
    let required = crate::market_router_account::PAYLOAD_OFFSET
        .checked_add(payload.len())
        .ok_or_else(invalid)?;
    let allocation = usize::try_from(p.allocation_bytes).map_err(|_| invalid())?;
    if allocation < required {
        return Err(invalid());
    }
    if allocation != a[1].data_len() {
        let minimum = crate::compact_error::rent()?.minimum_balance(allocation);
        let shortfall = minimum.saturating_sub(a[1].lamports());
        if shortfall > 0 {
            invoke_system_transfer(&a[0], &a[1], &a[15], shortfall, &[])?;
        }
        a[1].resize(allocation)?;
    }
    // This is also the creation boundary for a new suffix. A resized legacy
    // Market has a zero suffix until this authenticated handler writes it;
    // ordinary resident stores correctly reject that uninitialized envelope.
    state.validate(program, a[1].key, &market)?;
    crate::market_router_account::write_payload(&mut a[1].try_data_mut()?, &payload)?;
    let mut forwarded = state.pool;
    forwarded.account_version = router::FORWARDED_POOL_VERSION;
    write_light(&a[2], &forwarded)?;
    if p.include_book {
        let mut h = state.book.ok_or_else(invalid)?;
        h.version = router::FORWARDED_BOOK_VERSION;
        store_state(&a[3], &h)?;
    }
    for info in &a[router::PREPARE_COMMON..router::PREPARE_COMMON + usize::from(p.page_count)] {
        let old: ds::AmoebaDlmmBinPageV1 = light(program, info)?;
        let mut page = *state.page(old.page_index).ok_or_else(invalid)?;
        page.account_version = router::FORWARDED_PAGE_VERSION;
        write_light(info, &page)?;
    }
    for info in &a[router::PREPARE_COMMON + usize::from(p.page_count)..end] {
        let old = load_exact_zero_padded_state::<DlmmOrderRecord>(
            info,
            program,
            DlmmOrderRecord::LEN,
            VaultError::InvalidAmoebaDlmmPool,
        )?;
        let mut record = *state.record(old.order.sequence).ok_or_else(invalid)?;
        record.version = router::FORWARDED_RECORD_VERSION;
        store_state(info, &record)?;
    }
    if p.include_writer {
        let mut v = state.writer_position.ok_or_else(invalid)?;
        v.account_version = router::FORWARDED_WRITER_VERSION;
        store_state(&a[23], &v)?;
    }
    Ok(())
}
