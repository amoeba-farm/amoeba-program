use super::*;

pub(super) const FIRST_ALLOCATION_BYTES: usize = 10_000;

fn validate_zero_owned_index(
    program_id: &Pubkey,
    index: &AccountInfo,
    size: usize,
) -> ProgramResult {
    if index.owner != program_id
        || index.executable
        || !index.is_writable
        || index.data_len() != size
        || index.try_borrow_data()?.iter().any(|byte| *byte != 0)
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    Ok(())
}

pub(super) fn validate_ready_bid_index(program_id: &Pubkey, index: &AccountInfo) -> ProgramResult {
    validate_zero_owned_index(program_id, index, WriterBidIndexV1::LEN)?;
    if index.lamports() < Rent::get()?.minimum_balance(WriterBidIndexV1::LEN) {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn process_prepare_writer_bid_index(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: PrepareWriterBidIndexV1Params,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    if params.phase > 1 {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let [authority, config_info, registry_info, sleeve_info, group_info, book_info, snapshot_info, auction_info, index, system_info] =
        accounts
    else {
        return Err(VaultError::InvalidAccountList.into());
    };
    if !authority.is_signer || !authority.is_writable {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let WriterPolicyContext {
        group,
        sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        Some(registry_info.key),
    )?;
    let expected_nonce = sleeve
        .auction_nonce
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if config.paused
        || registry.policy_authority != *authority.key
        || sleeve.vault_config != *config_info.key
        || sleeve.policy_registry != *registry_info.key
        || sleeve.settlement_mint != config.usdc_mint
        || sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || sleeve.active_auction.is_some()
        || sleeve.active_close_request.is_some()
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.policy_hash != snapshot.policy_hash
        || !book.frozen
        || snapshot.series_family_hash != writer_series_family_hash(&book)
        || params.auction_nonce != expected_nonce
        || current_unix_timestamp()? >= sleeve.expiry_ts
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let auction = derive_writer_auction_pda(program_id, sleeve_info.key, expected_nonce).0;
    let (expected_index, bump) = derive_writer_bid_index_pda(program_id, &auction);
    validate_canonical_system_zero_pda_proof(&auction, auction_info)?;
    if *index.key != expected_index || !index.is_writable {
        return Err(VaultError::InvalidPda.into());
    }
    // Validate the complete phase precondition before any lamport or data mutation.
    if params.phase == 0 {
        validate_create_only_program_account_target(program_id, index)?;
        create_program_account(
            authority,
            index,
            system_info,
            program_id,
            FIRST_ALLOCATION_BYTES,
            &[
                crate::constants::WRITER_BID_INDEX_PDA_SEED,
                auction.as_ref(),
                &[bump],
            ],
        )?;
        validate_zero_owned_index(program_id, index, FIRST_ALLOCATION_BYTES)
    } else {
        validate_zero_owned_index(program_id, index, FIRST_ALLOCATION_BYTES)?;
        let shortfall = Rent::get()?
            .minimum_balance(WriterBidIndexV1::LEN)
            .saturating_sub(index.lamports());
        if shortfall > 0 {
            invoke_system_transfer(authority, index, system_info, shortfall, &[])?;
        }
        index.resize(WriterBidIndexV1::LEN)?;
        index.try_borrow_mut_data()?.fill(0);
        validate_ready_bid_index(program_id, index)
    }
}
