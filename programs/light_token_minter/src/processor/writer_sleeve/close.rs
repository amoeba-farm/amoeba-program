use super::*;
use crate::instruction::{
    BeginWriterCloseV1Params, ProcessWriterCloseCancellationV1Params, WriterSeriesIndexV1Params,
};

const BEGIN_WRITER_CLOSE_ACCOUNT_COUNT: usize = 15;
const DEPOSIT_WRITER_CLOSE_CLAIM_ACCOUNT_COUNT: usize = 13;
const FINALIZE_WRITER_CLOSE_FIXED_ACCOUNT_COUNT: usize = 13;
const FINALIZE_WRITER_CLOSE_ACCOUNTS_PER_SERIES: usize = 3;
const CANCEL_WRITER_CLOSE_SERIES_ACCOUNT_COUNT: usize = 13;
const CANCEL_WRITER_CLOSE_FLAT_ACCOUNT_COUNT: usize = 11;
const WRITER_CLOSE_FLAT_SENTINEL: u8 = u8::MAX;
const WRITER_CLOSE_BURN_ACCOUNT_META_COUNT: usize = 3;
const WRITER_CLOSE_BURN_DATA_LEN: usize = 10;
const SPL_TOKEN_BURN_CHECKED_TAG: u8 = 15;

#[cfg(target_os = "solana")]
type ReusableWriterCloseBurnInstruction =
    solana_program::stable_layout::stable_instruction::StableInstruction;

#[cfg(not(target_os = "solana"))]
type ReusableWriterCloseBurnInstruction = Instruction;

fn reusable_writer_close_burn_instruction(
    token_program: &Pubkey,
) -> Result<ReusableWriterCloseBurnInstruction, ProgramError> {
    let instruction = token_instruction::burn_checked(
        token_program,
        &Pubkey::default(),
        &Pubkey::default(),
        &Pubkey::default(),
        &[],
        0,
        MarketMintAccounting::CANONICAL_DECIMALS,
    )?;
    #[cfg(target_os = "solana")]
    {
        Ok(solana_program::stable_layout::stable_instruction::StableInstruction::from(instruction))
    }
    #[cfg(not(target_os = "solana"))]
    {
        Ok(instruction)
    }
}

#[inline(always)]
fn configure_writer_close_burn_instruction(
    instruction: &mut ReusableWriterCloseBurnInstruction,
    source: &Pubkey,
    mint: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> ProgramResult {
    if instruction.program_id != spl_token_program_id()
        || instruction.accounts[..].len() != WRITER_CLOSE_BURN_ACCOUNT_META_COUNT
        || instruction.data[..].len() != WRITER_CLOSE_BURN_DATA_LEN
        || instruction.accounts[0].is_signer
        || !instruction.accounts[0].is_writable
        || instruction.accounts[1].is_signer
        || !instruction.accounts[1].is_writable
        || !instruction.accounts[2].is_signer
        || instruction.accounts[2].is_writable
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    instruction.accounts[0].pubkey = *source;
    instruction.accounts[1].pubkey = *mint;
    instruction.accounts[2].pubkey = *authority;
    let mut data = [0; WRITER_CLOSE_BURN_DATA_LEN];
    data[0] = SPL_TOKEN_BURN_CHECKED_TAG;
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data[9] = MarketMintAccounting::CANONICAL_DECIMALS;
    instruction.data[..].copy_from_slice(&data);
    Ok(())
}

#[cfg(target_os = "solana")]
#[allow(deprecated)]
#[inline(never)]
fn invoke_reusable_writer_close_burn<'a>(
    instruction: &ReusableWriterCloseBurnInstruction,
    token_program_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    // Match solana_cpi::invoke_signed's RefCell preflight exactly. The StableInstruction owns
    // the only account/data vectors and is allocated once before the series loop; this stack
    // account array and the signer slices contain no per-burn heap ownership.
    let account_infos = [
        source_info.clone(),
        mint_info.clone(),
        authority_info.clone(),
        token_program_info.clone(),
    ];
    for account_meta in instruction.accounts.iter() {
        for account_info in account_infos.iter() {
            if account_meta.pubkey == *account_info.key {
                if account_meta.is_writable {
                    let _ = account_info.try_borrow_mut_lamports()?;
                    let _ = account_info.try_borrow_mut_data()?;
                } else {
                    let _ = account_info.try_borrow_lamports()?;
                    let _ = account_info.try_borrow_data()?;
                }
                break;
            }
        }
    }
    let result = unsafe {
        solana_program::syscalls::sol_invoke_signed_rust(
            instruction as *const _ as *const u8,
            account_infos.as_ptr() as *const u8,
            account_infos.len() as u64,
            signer_seeds.as_ptr() as *const u8,
            signer_seeds.len() as u64,
        )
    };
    if result == solana_program::entrypoint::SUCCESS {
        Ok(())
    } else {
        Err(result.into())
    }
}

#[cfg(not(target_os = "solana"))]
#[inline(never)]
fn invoke_reusable_writer_close_burn<'a>(
    instruction: &ReusableWriterCloseBurnInstruction,
    token_program_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    invoke_signed(
        instruction,
        &[
            source_info.clone(),
            mint_info.clone(),
            authority_info.clone(),
            token_program_info.clone(),
        ],
        signer_seeds,
    )
}

fn advance_required_cursor(request: &WriterCloseRequestV1, mut cursor: u8) -> u8 {
    while cursor < request.series_count && request.required_claim_atoms[usize::from(cursor)] == 0 {
        cursor = cursor.saturating_add(1);
    }
    cursor
}

fn advance_cancel_cursor(request: &WriterCloseRequestV1, mut cursor: u8) -> u8 {
    while cursor < request.series_count && request.deposited_claim_atoms[usize::from(cursor)] == 0 {
        cursor = cursor.saturating_add(1);
    }
    cursor
}

fn close_request_signer_seeds<'a>(
    sleeve: &'a Pubkey,
    nonce: &'a [u8; 8],
    bump: &'a [u8; 1],
) -> [&'a [u8]; 5] {
    [
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_CLOSE_REQUEST_PDA_SEED,
        sleeve.as_ref(),
        nonce,
        bump,
    ]
}

#[inline]
fn writer_close_begin_deadline_valid(now: u64, deadline: u64, expiry: u64) -> bool {
    now < deadline && deadline < expiry
}

#[inline]
fn writer_close_collection_window_open(now: u64, deadline: u64) -> bool {
    now <= deadline
}

#[inline]
fn writer_close_finalize_window_open(now: u64, deadline: u64, expiry: u64) -> bool {
    writer_close_collection_window_open(now, deadline) && now < expiry
}

#[inline]
fn writer_close_cancellation_requires_owner(now: u64, deadline: u64) -> bool {
    writer_close_collection_window_open(now, deadline)
}

fn validate_close_snapshot(
    sleeve: &WriterSleeveV1,
    group_info: &AccountInfo,
    group: &WriterSettlementGroupV1,
    book: &WriterSeriesBookV1,
    request: &WriterCloseRequestV1,
) -> ProgramResult {
    let count = usize::from(book.series_count);
    if request.snapshot_asset_atoms != sleeve.accounted_asset_atoms
        || request.snapshot_reserve_atoms != sleeve.exact_reserve_atoms
        || request.snapshot_writer_principal_atoms != sleeve.writer_principal_atoms
        || request.snapshot_locked_primary_premium_atoms != sleeve.locked_primary_premium_atoms
        || request.snapshot_flat_supply_atoms != sleeve.flat_par_supply_atoms
        || request.snapshot_security_exposure_atoms != sleeve.security_exposure_atoms
        || request.snapshot_policy_version != sleeve.policy_version
        || request.snapshot_group_commitment != writer_group_commitment(group_info.key, group)
        || request.snapshot_book_digest != writer_book_digest(book)
        || request.snapshot_book_digest != book.book_digest
        || request.series_count != book.series_count
        || request.snapshot_external_oi_atoms[..count]
            .iter()
            .zip(book.records[..count].iter())
            .any(|(snapshot, record)| *snapshot != record.external_open_interest_atoms)
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    Ok(())
}

fn authorize_cancellation(
    actor_info: &AccountInfo,
    request: &WriterCloseRequestV1,
) -> ProgramResult {
    if !actor_info.is_signer || !actor_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if writer_close_cancellation_requires_owner(current_unix_timestamp()?, request.deadline_ts)
        && *actor_info.key != request.owner
    {
        return Err(VaultError::Unauthorized.into());
    }
    if !matches!(
        request.status,
        WriterCloseRequestStatus::Collecting
            | WriterCloseRequestStatus::Complete
            | WriterCloseRequestStatus::Cancelling
    ) {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn close_request_token_escrow<'a>(
    request: &WriterCloseRequestV1,
    request_info: &AccountInfo<'a>,
    escrow_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    if validate_token_account(escrow_info)?.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let nonce = request.request_nonce.to_le_bytes();
    let bump = [request.bump];
    let signer = close_request_signer_seeds(&request.sleeve, &nonce, &bump);
    invoke_token_close_account(
        token_program_info,
        escrow_info,
        request_info,
        request_info,
        &[&signer],
    )
}

pub(super) fn process_begin_writer_close(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: BeginWriterCloseV1Params,
) -> ProgramResult {
    if accounts.len() != BEGIN_WRITER_CLOSE_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let request_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    let flat_source_info = &accounts[8];
    let flat_escrow_info = &accounts[9];
    let flat_interface_info = &accounts[10];
    let light_program_info = &accounts[11];
    let cpi_authority_info = &accounts[12];
    let token_program_info = &accounts[13];
    let system_program_info = &accounts[14];
    if !owner_info.is_signer || !owner_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterPolicyContext {
        group,
        mut sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    let now = current_unix_timestamp()?;
    let request_nonce = sleeve
        .close_nonce
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || group.sleeve != *sleeve_info.key
        || !book.frozen
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || params.flat_amount_atoms == 0
        || params.flat_amount_atoms >= sleeve.flat_par_supply_atoms
        || params.flat_amount_atoms > snapshot.max_close_flat_atoms
        || !writer_close_begin_deadline_valid(now, params.deadline_ts, sleeve.expiry_ts)
    {
        return Err(VaultError::InvalidWriterDeadline.into());
    }
    recompute_writer_metrics(
        &mut sleeve,
        &book,
        &snapshot,
        Some(group.security_cap_atoms),
        false,
    )?;
    // A reconciled holder burn can lower W without changing A, B, or the live
    // liabilities.  The current drawdown ratios may therefore be outside their
    // admission bounds even though a proportional close would restore them.
    // Beginning a close moves Flat only into cancelable escrow; finalization
    // below still enforces the exact post-close solvency and drawdown gates
    // before any settlement asset leaves the sleeve.
    let current_required_assets = sleeve
        .exact_reserve_atoms
        .checked_add(snapshot.operational_buffer_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if sleeve.writer_principal_atoms == 0 || sleeve.accounted_asset_atoms < current_required_assets
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let series = writer_book_math_series(&book)?;
    let preview = proportional_close_preview(
        sleeve.accounted_asset_atoms,
        sleeve.flat_par_supply_atoms,
        params.flat_amount_atoms,
        &series,
        snapshot.lower_tail_max_settlement_atomic,
        snapshot.upper_tail_min_settlement_atomic,
    )
    .map_err(writer_math_error)?;
    if preview.reserve_before_atoms != sleeve.exact_reserve_atoms
        || preview.withdrawal_atoms < params.minimum_withdrawal_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    validate_light_associated_token_account(owner_info.key, flat_mint_info.key, flat_source_info)?;
    let source_before =
        load_canonical_light_token_account(flat_source_info, owner_info.key, flat_mint_info.key)?;
    if source_before.amount < params.flat_amount_atoms
        || flat_mint.supply != sleeve.flat_par_supply_atoms
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_spl_interface_account(flat_mint_info.key, flat_interface_info)?;

    let (expected_request, request_bump) =
        derive_writer_close_request_pda(program_id, sleeve_info.key, request_nonce);
    let (expected_escrow, escrow_bump) =
        derive_writer_close_flat_escrow_pda(program_id, request_info.key);
    if *request_info.key != expected_request || *flat_escrow_info.key != expected_escrow {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, request_info)?;
    create_program_account(
        owner_info,
        request_info,
        system_program_info,
        program_id,
        WriterCloseRequestV1::LEN,
        &[
            crate::constants::WRITER_CLOSE_REQUEST_PDA_SEED,
            sleeve_info.key.as_ref(),
            &request_nonce.to_le_bytes(),
            &[request_bump],
        ],
    )?;
    create_classic_token_pda(
        program_id,
        owner_info,
        flat_escrow_info,
        flat_mint_info,
        request_info.key,
        token_program_info,
        system_program_info,
        &[
            crate::constants::WRITER_CLOSE_FLAT_ESCROW_PDA_SEED,
            request_info.key.as_ref(),
            &[escrow_bump],
        ],
    )?;
    if validate_token_account(flat_escrow_info)?.amount != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    invoke_light_token_account_transfer(
        params.flat_amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        owner_info,
        flat_source_info,
        flat_escrow_info,
        owner_info,
        flat_mint_info,
        flat_interface_info,
        token_program_info,
        system_program_info,
    )?;
    let source_after =
        load_canonical_light_token_account(flat_source_info, owner_info.key, flat_mint_info.key)?;
    if source_before.amount.checked_sub(source_after.amount) != Some(params.flat_amount_atoms)
        || validate_token_account(flat_escrow_info)?.amount != params.flat_amount_atoms
        || validate_mint_account(flat_mint_info, token_program_info.key)?.supply != flat_mint.supply
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }

    let mut required_claim_atoms = [0u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY];
    let mut snapshot_external_oi_atoms = [0u64; crate::constants::WRITER_SERIES_STORAGE_CAPACITY];
    let count = usize::from(book.series_count);
    required_claim_atoms[..count].copy_from_slice(&preview.required_claim_atoms.values[..count]);
    for (destination, record) in snapshot_external_oi_atoms[..count]
        .iter_mut()
        .zip(book.records[..count].iter())
    {
        *destination = record.external_open_interest_atoms;
    }
    let slot = Clock::get()?.slot;
    let mut request = WriterCloseRequestV1 {
        is_initialized: true,
        bump: request_bump,
        account_discriminator: WriterCloseRequestV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterCloseRequestV1::ACCOUNT_VERSION,
        sleeve: *sleeve_info.key,
        owner: *owner_info.key,
        flat_escrow: *flat_escrow_info.key,
        flat_mint: *flat_mint_info.key,
        request_nonce,
        flat_amount_atoms: params.flat_amount_atoms,
        minimum_withdrawal_atoms: params.minimum_withdrawal_atoms,
        snapshot_asset_atoms: sleeve.accounted_asset_atoms,
        snapshot_reserve_atoms: sleeve.exact_reserve_atoms,
        snapshot_writer_principal_atoms: sleeve.writer_principal_atoms,
        snapshot_locked_primary_premium_atoms: sleeve.locked_primary_premium_atoms,
        snapshot_flat_supply_atoms: sleeve.flat_par_supply_atoms,
        snapshot_security_exposure_atoms: sleeve.security_exposure_atoms,
        snapshot_policy_version: sleeve.policy_version,
        snapshot_group_commitment: writer_group_commitment(group_info.key, &group),
        snapshot_book_digest: writer_book_digest(&book),
        deadline_ts: params.deadline_ts,
        status: WriterCloseRequestStatus::Collecting,
        series_count: book.series_count,
        next_deposit_index: 0,
        next_cancel_index: 0,
        reserved: [0; 6],
        required_claim_atoms,
        deposited_claim_atoms: [0; crate::constants::WRITER_SERIES_STORAGE_CAPACITY],
        snapshot_external_oi_atoms,
        final_withdrawal_atoms: preview.withdrawal_atoms,
        finalized_slot: 0,
        last_updated_slot: slot,
    };
    request.next_deposit_index = advance_required_cursor(&request, 0);
    if request.next_deposit_index == request.series_count {
        request.status = WriterCloseRequestStatus::Complete;
    }
    sleeve.close_nonce = request_nonce;
    sleeve.active_close_request = Some(*request_info.key);
    sleeve.status = WriterSleeveStatus::CloseStaging;
    sleeve.last_updated_slot = slot;
    store_state(request_info, &request)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_deposit_writer_close_claim(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: WriterSeriesIndexV1Params,
) -> ProgramResult {
    if accounts.len() != DEPOSIT_WRITER_CLOSE_CLAIM_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let book_info = &accounts[2];
    let request_info = &accounts[3];
    let market_info = &accounts[4];
    let mint_info = &accounts[5];
    let source_info = &accounts[6];
    let retirement_info = &accounts[7];
    let interface_info = &accounts[8];
    let light_program_info = &accounts[9];
    let cpi_authority_info = &accounts[10];
    let token_program_info = &accounts[11];
    let system_program_info = &accounts[12];
    if !owner_info.is_signer || !owner_info.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let book = load_writer_series_book(
        program_id,
        book_info,
        sleeve_info.key,
        &sleeve.settlement_group,
    )?;
    let mut request = load_writer_close_request(
        program_id,
        request_info,
        sleeve_info.key,
        sleeve.close_nonce,
    )?;
    if sleeve.status != WriterSleeveStatus::CloseStaging
        || sleeve.active_close_request != Some(*request_info.key)
        || request.owner != *owner_info.key
        || request.status != WriterCloseRequestStatus::Collecting
        || !writer_close_collection_window_open(current_unix_timestamp()?, request.deadline_ts)
        || params.series_index != request.next_deposit_index
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let index = usize::from(params.series_index);
    if index >= usize::from(book.series_count) {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let amount = request.required_claim_atoms[index];
    let record = book.records[index];
    if amount == 0
        || request.deposited_claim_atoms[index] != 0
        || record.market != *market_info.key
        || record.contract_mint != *mint_info.key
        || record.retirement_custody != *retirement_info.key
        || request.snapshot_external_oi_atoms[index] != record.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    if market.market_id != record.series_id
        || market.long_contract_mint != Some(*mint_info.key)
        || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mint = validate_canonical_market_mint(market_info, &mut market, mint_info, 0)?;
    if mint.supply != record.total_physical_supply_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_light_associated_token_account(owner_info.key, mint_info.key, source_info)?;
    let source_before = super::super::scoped_settlement::load_scoped_holder_token_account(
        program_id,
        source_info,
        owner_info.key,
        mint_info.key,
    )?;
    if source_before.amount < amount {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_spl_interface_account(mint_info.key, interface_info)?;
    let retirement_before = load_or_create_writer_retirement_custody(
        program_id,
        owner_info,
        sleeve_info,
        market_info,
        retirement_info,
        mint_info,
        token_program_info,
        system_program_info,
    )?
    .amount;
    invoke_light_token_account_transfer(
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        owner_info,
        source_info,
        retirement_info,
        owner_info,
        mint_info,
        interface_info,
        token_program_info,
        system_program_info,
    )?;
    let source_after = super::super::scoped_settlement::load_scoped_holder_token_account(
        program_id,
        source_info,
        owner_info.key,
        mint_info.key,
    )?;
    let retirement_after = validate_token_account(retirement_info)?.amount;
    if source_before.amount.checked_sub(source_after.amount) != Some(amount)
        || retirement_after.checked_sub(retirement_before) != Some(amount)
        || validate_mint_account(mint_info, token_program_info.key)?.supply != mint.supply
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    request.deposited_claim_atoms[index] = amount;
    request.next_deposit_index = advance_required_cursor(
        &request,
        request
            .next_deposit_index
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    if request.next_deposit_index == request.series_count {
        request.status = WriterCloseRequestStatus::Complete;
    }
    request.last_updated_slot = Clock::get()?.slot;
    store_state(request_info, &request)
}

pub(super) fn process_finalize_writer_close(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < FINALIZE_WRITER_CLOSE_FIXED_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let owner_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let request_info = &accounts[6];
    let sleeve_vault_info = &accounts[7];
    let destination_info = &accounts[8];
    let settlement_mint_info = &accounts[9];
    let flat_mint_info = &accounts[10];
    let flat_escrow_info = &accounts[11];
    let token_program_info = &accounts[12];
    if !owner_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    // The central writer dispatcher has already required the exact effective signer/writable
    // pattern for every role. These local checks retain defense in depth. The runtime cannot
    // attribute an effective privilege to a source meta or observe a duplicated readonly alias.
    if !sleeve_info.is_writable
        || !book_info.is_writable
        || !request_info.is_writable
        || !sleeve_vault_info.is_writable
        || !destination_info.is_writable
        || !flat_mint_info.is_writable
        || !flat_escrow_info.is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let WriterBookContext {
        group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    if accounts.len()
        != FINALIZE_WRITER_CLOSE_FIXED_ACCOUNT_COUNT
            + usize::from(book.series_count) * FINALIZE_WRITER_CLOSE_ACCOUNTS_PER_SERIES
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Freeze the complete dynamic account shape before the first burn. This prevents a later
    // series' missing privilege, reorder, omission, or substitution from being discovered only
    // after an earlier series has entered Tokenkeg.
    for (index, series_accounts) in accounts[FINALIZE_WRITER_CLOSE_FIXED_ACCOUNT_COUNT..]
        .chunks_exact(FINALIZE_WRITER_CLOSE_ACCOUNTS_PER_SERIES)
        .enumerate()
    {
        let record = &book.records[index];
        let market_info = &series_accounts[0];
        let mint_info = &series_accounts[1];
        let retirement_info = &series_accounts[2];
        if !market_info.is_writable
            || !mint_info.is_writable
            || !retirement_info.is_writable
            || *market_info.key != record.market
            || *mint_info.key != record.contract_mint
            || *retirement_info.key != record.retirement_custody
            || *retirement_info.key
                != derive_writer_retirement_custody_pda(
                    program_id,
                    sleeve_info.key,
                    market_info.key,
                )
                .0
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        &sleeve.policy_registry,
        sleeve.policy_version,
    )?;
    let mut request = load_writer_close_request(
        program_id,
        request_info,
        sleeve_info.key,
        sleeve.close_nonce,
    )?;
    let now = current_unix_timestamp()?;
    if config.paused
        || sleeve.vault_config != *config_info.key
        || sleeve.status != WriterSleeveStatus::CloseStaging
        || sleeve.active_close_request != Some(*request_info.key)
        || group.status != WriterSettlementGroupStatus::Active
        || request.owner != *owner_info.key
        || request.status != WriterCloseRequestStatus::Complete
        || request.next_deposit_index != request.series_count
        || !writer_close_finalize_window_open(now, request.deadline_ts, sleeve.expiry_ts)
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || request.flat_mint != *flat_mint_info.key
        || request.flat_escrow != *flat_escrow_info.key
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    validate_close_snapshot(&sleeve, group_info, &group, &book, &request)?;
    if snapshot.policy_hash != sleeve.policy_hash
        || snapshot.policy_version != sleeve.policy_version
        || snapshot.series_family_hash != writer_series_family_hash(&book)
        || snapshot.security_mode != sleeve.security_mode
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let count = usize::from(book.series_count);
    if request.deposited_claim_atoms[..count] != request.required_claim_atoms[..count] {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let mut series = writer_book_math_series(&book)?;
    let preview = proportional_close_preview(
        sleeve.accounted_asset_atoms,
        sleeve.flat_par_supply_atoms,
        request.flat_amount_atoms,
        &series,
        snapshot.lower_tail_max_settlement_atomic,
        snapshot.upper_tail_min_settlement_atomic,
    )
    .map_err(writer_math_error)?;
    if preview.reserve_before_atoms != sleeve.exact_reserve_atoms
        || preview.withdrawal_atoms != request.final_withdrawal_atoms
        || preview.withdrawal_atoms < request.minimum_withdrawal_atoms
        || preview.required_claim_atoms.values[..count] != request.required_claim_atoms[..count]
        || request.flat_amount_atoms > sleeve.writer_principal_atoms
        || request.flat_amount_atoms > sleeve.flat_par_supply_atoms
        || preview.withdrawal_atoms > sleeve.accounted_asset_atoms
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }

    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    validate_vault_token_account(flat_escrow_info, flat_mint_info.key, request_info.key)?;
    if validate_token_account(flat_escrow_info)?.amount != request.flat_amount_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    if config.usdc_mint != *settlement_mint_info.key
        || sleeve.settlement_mint != *settlement_mint_info.key
        || sleeve.usdc_vault != *sleeve_vault_info.key
    {
        return Err(VaultError::InvalidMint.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let destination = validate_token_account(destination_info)?;
    if destination.owner != request.owner
        || destination.mint != *settlement_mint_info.key
        || destination.state != AccountState::Initialized
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    let vault_before = validate_token_account(sleeve_vault_info)?.amount;
    let destination_before = destination.amount;
    if vault_before < sleeve.accounted_asset_atoms || vault_before < preview.withdrawal_atoms {
        return Err(VaultError::WriterSolvencyViolation.into());
    }

    let sleeve_bump = [sleeve.bump];
    let settlement_group = sleeve.settlement_group;
    let sleeve_signer = writer_sleeve_signer_seeds(&settlement_group, &sleeve_bump);
    let mut burn_instruction = reusable_writer_close_burn_instruction(token_program_info.key)?;
    let mut markets = Vec::with_capacity(count);
    for (index, series_accounts) in accounts[FINALIZE_WRITER_CLOSE_FIXED_ACCOUNT_COUNT..]
        .chunks_exact(FINALIZE_WRITER_CLOSE_ACCOUNTS_PER_SERIES)
        .enumerate()
    {
        let market_info = &series_accounts[0];
        let mint_info = &series_accounts[1];
        let retirement_info = &series_accounts[2];
        let record = &mut book.records[index];
        let mut market = load_valid_market(program_id, market_info)?;
        let required = request.required_claim_atoms[index];
        if market.market_id != record.series_id
            || market.long_contract_mint != Some(record.contract_mint)
            || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
            || request.snapshot_external_oi_atoms[index] != record.external_open_interest_atoms
            || required > record.external_open_interest_atoms
        {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
        let mint_before = validate_canonical_market_mint(market_info, &mut market, mint_info, 0)?;
        if mint_before.supply != record.total_physical_supply_atoms
            || record
                .issuer_controlled_atoms
                .checked_add(record.external_open_interest_atoms)
                != Some(record.total_physical_supply_atoms)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        let custody_before = if retirement_info.owner == token_program_info.key {
            validate_vault_token_account(retirement_info, mint_info.key, sleeve_info.key)?;
            validate_token_account(retirement_info)?.amount
        } else {
            if required != 0 || record.issuer_controlled_atoms != 0 || !retirement_info.is_writable
            {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            validate_canonical_system_zero_pda_proof(retirement_info.key, retirement_info)?;
            0
        };
        if custody_before
            < record
                .issuer_controlled_atoms
                .checked_add(required)
                .ok_or(VaultError::ArithmeticOverflow)?
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        if required != 0 {
            configure_writer_close_burn_instruction(
                &mut burn_instruction,
                retirement_info.key,
                mint_info.key,
                sleeve_info.key,
                required,
            )?;
            invoke_reusable_writer_close_burn(
                &burn_instruction,
                token_program_info,
                retirement_info,
                mint_info,
                sleeve_info,
                &[&sleeve_signer],
            )?;
            consume_market_contracts(&mut market, required, true)?;
        }
        let mint_after = validate_mint_account(mint_info, token_program_info.key)?;
        let custody_after = if retirement_info.owner == token_program_info.key {
            validate_token_account(retirement_info)?.amount
        } else {
            0
        };
        if mint_before.supply.checked_sub(mint_after.supply) != Some(required)
            || custody_before.checked_sub(custody_after) != Some(required)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        record.total_physical_supply_atoms = record
            .total_physical_supply_atoms
            .checked_sub(required)
            .ok_or(VaultError::ArithmeticOverflow)?;
        record.external_open_interest_atoms = record
            .external_open_interest_atoms
            .checked_sub(required)
            .ok_or(VaultError::ArithmeticOverflow)?;
        series[index].external_oi_atoms = record.external_open_interest_atoms;
        // Custody status tracks positive physical retirement custody, including any unreconciled
        // surplus above the recorded issuer-controlled balance. Keeping that surplus Open blocks
        // settlement cleanup until the permissionless reconciliation lane classifies it, while an
        // initialized but exactly emptied retirement account is safely Absent.
        if custody_after != 0 {
            record.custody_status = WriterSeriesCustodyStatus::Open;
        } else {
            record.custody_status = WriterSeriesCustodyStatus::Absent;
        }
        if market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
            || market
                .mint_accounting
                .total_issued
                .checked_sub(market.mint_accounting.total_burned)
                != Some(record.total_physical_supply_atoms)
            || record
                .issuer_controlled_atoms
                .checked_add(record.external_open_interest_atoms)
                != Some(record.total_physical_supply_atoms)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        markets.push(market);
    }

    sleeve.accounted_asset_atoms = sleeve
        .accounted_asset_atoms
        .checked_sub(preview.withdrawal_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.writer_principal_atoms = sleeve
        .writer_principal_atoms
        .checked_sub(request.flat_amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.flat_par_supply_atoms = sleeve
        .flat_par_supply_atoms
        .checked_sub(request.flat_amount_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    // The fused close preview is the exact post-close reserve walk. Re-running
    // `exact_reserve` here would duplicate the bounded candidate pass and can
    // exceed the SBF budget at the live series cap. Admission below reuses the
    // preview's exact reserve outputs and independently recomputes only the
    // selected security metric and drawdown inequalities over the mutated book.
    let post_reserve = crate::writer_sleeve_math::WriterReserveSummary {
        reserve_atoms: preview.reserve_after_atoms,
        lower_tail_reserve_atoms: preview.lower_tail_reserve_after_atoms,
        upper_tail_reserve_atoms: preview.upper_tail_reserve_after_atoms,
        candidate_count: preview.candidate_count,
        ..crate::writer_sleeve_math::WriterReserveSummary::default()
    };
    let math_security_mode = match sleeve.security_mode {
        WriterSecurityMode::GrossExternalMaxPayout => {
            WriterMathSecurityMode::GrossExternalMaximumPayout
        }
        WriterSecurityMode::ExactExternalEnvelope => WriterMathSecurityMode::ExactExternalEnvelope,
    };
    let post_security =
        calculate_security_exposure(math_security_mode, &series, post_reserve.reserve_atoms)
            .map_err(writer_math_error)?;
    if post_security > group.security_cap_atoms {
        return Err(VaultError::WriterSecurityCapExceeded.into());
    }
    let required_assets = post_reserve
        .reserve_atoms
        .checked_add(snapshot.operational_buffer_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let post_drawdown = drawdown_checks(
        &post_reserve,
        sleeve.locked_primary_premium_atoms,
        sleeve.writer_principal_atoms,
        snapshot.worst_drawdown_limit,
        snapshot.lower_drawdown_limit,
        snapshot.upper_drawdown_limit,
    )
    .map_err(writer_math_error)?;
    if sleeve.accounted_asset_atoms < required_assets
        || sleeve.writer_principal_atoms == 0
        || !post_drawdown.all_pass()
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    sleeve.exact_reserve_atoms = post_reserve.reserve_atoms;
    sleeve.lower_tail_reserve_atoms = post_reserve.lower_tail_reserve_atoms;
    sleeve.upper_tail_reserve_atoms = post_reserve.upper_tail_reserve_atoms;
    sleeve.security_exposure_atoms = post_security;

    drop(series);
    drop(snapshot);
    drop(group);

    let request_nonce = request.request_nonce.to_le_bytes();
    let request_bump = [request.bump];
    let request_signer = close_request_signer_seeds(&request.sleeve, &request_nonce, &request_bump);
    invoke_token_burn_checked(
        token_program_info,
        flat_escrow_info,
        flat_mint_info,
        request_info,
        request.flat_amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&request_signer],
    )?;
    invoke_token_transfer_checked(
        token_program_info,
        sleeve_vault_info,
        settlement_mint_info,
        destination_info,
        sleeve_info,
        preview.withdrawal_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[&sleeve_signer],
    )?;
    if flat_mint
        .supply
        .checked_sub(validate_mint_account(flat_mint_info, token_program_info.key)?.supply)
        != Some(request.flat_amount_atoms)
        || validate_token_account(flat_escrow_info)?.amount != 0
        || vault_before.checked_sub(validate_token_account(sleeve_vault_info)?.amount)
            != Some(preview.withdrawal_atoms)
        || validate_token_account(destination_info)?
            .amount
            .checked_sub(destination_before)
            != Some(preview.withdrawal_atoms)
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    close_request_token_escrow(&request, request_info, flat_escrow_info, token_program_info)?;

    let slot = Clock::get()?.slot;
    request.status = WriterCloseRequestStatus::Finalized;
    request.finalized_slot = slot;
    request.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    sleeve.active_close_request = None;
    sleeve.status = WriterSleeveStatus::Active;
    sleeve.last_updated_slot = slot;
    for (series_accounts, market) in accounts[FINALIZE_WRITER_CLOSE_FIXED_ACCOUNT_COUNT..]
        .chunks_exact(FINALIZE_WRITER_CLOSE_ACCOUNTS_PER_SERIES)
        .zip(markets.iter())
    {
        store_state(&series_accounts[0], market)?;
    }
    store_state(book_info, &book)?;
    store_state(request_info, &request)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_writer_close_cancellation(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ProcessWriterCloseCancellationV1Params,
) -> ProgramResult {
    if params.selector == WRITER_CLOSE_FLAT_SENTINEL {
        process_flat_cancellation(program_id, accounts)
    } else {
        process_series_cancellation(program_id, accounts, params.selector)
    }
}

fn process_series_cancellation(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    selector: u8,
) -> ProgramResult {
    if accounts.len() != CANCEL_WRITER_CLOSE_SERIES_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let book_info = &accounts[2];
    let request_info = &accounts[3];
    let market_info = &accounts[4];
    let mint_info = &accounts[5];
    let retirement_info = &accounts[6];
    let destination_info = &accounts[7];
    let interface_info = &accounts[8];
    let light_program_info = &accounts[9];
    let cpi_authority_info = &accounts[10];
    let token_program_info = &accounts[11];
    let system_program_info = &accounts[12];
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let book = load_writer_series_book(
        program_id,
        book_info,
        sleeve_info.key,
        &sleeve.settlement_group,
    )?;
    let mut request = load_writer_close_request(
        program_id,
        request_info,
        sleeve_info.key,
        sleeve.close_nonce,
    )?;
    authorize_cancellation(actor_info, &request)?;
    if sleeve.status != WriterSleeveStatus::CloseStaging
        || sleeve.active_close_request != Some(*request_info.key)
        || request.series_count != book.series_count
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let expected = advance_cancel_cursor(&request, request.next_cancel_index);
    if selector != expected || selector >= request.series_count {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let index = usize::from(selector);
    let record = book.records[index];
    let amount = request.deposited_claim_atoms[index];
    if amount == 0
        || record.market != *market_info.key
        || record.contract_mint != *mint_info.key
        || record.retirement_custody != *retirement_info.key
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    if market.market_id != record.series_id
        || market.long_contract_mint != Some(*mint_info.key)
        || market_outstanding_contract_amount(&market)? != record.external_open_interest_atoms
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mint = validate_canonical_market_mint(market_info, &mut market, mint_info, 0)?;
    validate_vault_token_account(retirement_info, mint_info.key, sleeve_info.key)?;
    validate_light_associated_token_account(&request.owner, mint_info.key, destination_info)?;
    let destination_before = super::super::scoped_settlement::load_scoped_holder_token_account(
        program_id,
        destination_info,
        &request.owner,
        mint_info.key,
    )?;
    let retirement_before = validate_token_account(retirement_info)?.amount;
    if retirement_before < amount {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_spl_interface_account(mint_info.key, interface_info)?;
    let sleeve_bump = [sleeve.bump];
    let signer = writer_sleeve_signer_seeds(&sleeve.settlement_group, &sleeve_bump);
    invoke_light_token_account_transfer_with_signer_seeds(
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        actor_info,
        retirement_info,
        destination_info,
        sleeve_info,
        mint_info,
        interface_info,
        token_program_info,
        system_program_info,
        &[&signer],
    )?;
    let destination_after = super::super::scoped_settlement::load_scoped_holder_token_account(
        program_id,
        destination_info,
        &request.owner,
        mint_info.key,
    )?;
    if retirement_before.checked_sub(validate_token_account(retirement_info)?.amount)
        != Some(amount)
        || destination_after
            .amount
            .checked_sub(destination_before.amount)
            != Some(amount)
        || validate_mint_account(mint_info, token_program_info.key)?.supply != mint.supply
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    request.status = WriterCloseRequestStatus::Cancelling;
    request.deposited_claim_atoms[index] = 0;
    request.next_cancel_index = advance_cancel_cursor(
        &request,
        selector
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    request.last_updated_slot = Clock::get()?.slot;
    store_state(request_info, &request)
}

fn process_flat_cancellation(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    if accounts.len() != CANCEL_WRITER_CLOSE_FLAT_ACCOUNT_COUNT {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let sleeve_info = &accounts[1];
    let request_info = &accounts[2];
    let flat_mint_info = &accounts[3];
    let flat_escrow_info = &accounts[4];
    let destination_info = &accounts[5];
    let interface_info = &accounts[6];
    let light_program_info = &accounts[7];
    let cpi_authority_info = &accounts[8];
    let token_program_info = &accounts[9];
    let system_program_info = &accounts[10];
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    let mut sleeve = load_writer_sleeve_without_group_meta(program_id, sleeve_info)?;
    let mut request = load_writer_close_request(
        program_id,
        request_info,
        sleeve_info.key,
        sleeve.close_nonce,
    )?;
    authorize_cancellation(actor_info, &request)?;
    if sleeve.status != WriterSleeveStatus::CloseStaging
        || sleeve.active_close_request != Some(*request_info.key)
        || request.flat_mint != *flat_mint_info.key
        || request.flat_escrow != *flat_escrow_info.key
        || advance_cancel_cursor(&request, request.next_cancel_index) != request.series_count
        || request.deposited_claim_atoms[..usize::from(request.series_count)]
            .iter()
            .any(|amount| *amount != 0)
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    let flat_mint = validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    validate_vault_token_account(flat_escrow_info, flat_mint_info.key, request_info.key)?;
    let escrow_before = validate_token_account(flat_escrow_info)?.amount;
    if escrow_before != request.flat_amount_atoms {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    validate_light_associated_token_account(&request.owner, flat_mint_info.key, destination_info)?;
    let destination_before =
        load_canonical_light_token_account(destination_info, &request.owner, flat_mint_info.key)?;
    validate_spl_interface_account(flat_mint_info.key, interface_info)?;
    let nonce = request.request_nonce.to_le_bytes();
    let bump = [request.bump];
    let signer = close_request_signer_seeds(&request.sleeve, &nonce, &bump);
    invoke_light_token_account_transfer_with_signer_seeds(
        request.flat_amount_atoms,
        MarketMintAccounting::CANONICAL_DECIMALS,
        light_program_info,
        cpi_authority_info,
        actor_info,
        flat_escrow_info,
        destination_info,
        request_info,
        flat_mint_info,
        interface_info,
        token_program_info,
        system_program_info,
        &[&signer],
    )?;
    let destination_after =
        load_canonical_light_token_account(destination_info, &request.owner, flat_mint_info.key)?;
    if validate_token_account(flat_escrow_info)?.amount != 0
        || destination_after
            .amount
            .checked_sub(destination_before.amount)
            != Some(request.flat_amount_atoms)
        || validate_mint_account(flat_mint_info, token_program_info.key)?.supply != flat_mint.supply
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    close_request_token_escrow(&request, request_info, flat_escrow_info, token_program_info)?;
    let slot = Clock::get()?.slot;
    request.status = WriterCloseRequestStatus::Cancelled;
    request.next_cancel_index = request.series_count;
    request.last_updated_slot = slot;
    sleeve.active_close_request = None;
    sleeve.status = WriterSleeveStatus::Active;
    sleeve.last_updated_slot = slot;
    store_state(request_info, &request)?;
    store_state(sleeve_info, &sleeve)
}
