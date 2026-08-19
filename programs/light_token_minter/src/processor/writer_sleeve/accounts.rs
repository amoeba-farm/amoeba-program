use super::*;

pub(super) fn load_writer_sleeve_without_group_meta(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
) -> Result<Box<WriterSleeveV1>, ProgramError> {
    load_writer_sleeve_inner(program_id, sleeve_info, None)
}

pub(super) fn writer_sleeve_signer_seeds<'a>(
    group: &'a Pubkey,
    bump: &'a [u8; 1],
) -> [&'a [u8]; 4] {
    [
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::WRITER_SLEEVE_PDA_SEED,
        group.as_ref(),
        bump,
    ]
}

#[inline(never)]
pub(super) fn validate_writer_program_accounts(
    light_program_info: &AccountInfo,
    cpi_authority_info: &AccountInfo,
    token_program_info: &AccountInfo,
    system_program_info: &AccountInfo,
) -> ProgramResult {
    if *light_program_info.key != light_token_program_id()
        || *cpi_authority_info.key != cpi_authority()
        || *token_program_info.key != spl_token_program_id()
        || *system_program_info.key != system_program::id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn validate_writer_compression_accounts(
    light_program_info: &AccountInfo,
    cpi_authority_info: &AccountInfo,
    token_program_info: &AccountInfo,
    system_program_info: &AccountInfo,
    compressible_config_info: &AccountInfo,
    rent_sponsor_info: &AccountInfo,
) -> ProgramResult {
    validate_writer_program_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
    )?;
    if *compressible_config_info.key != light_token_instruction::compressible_config()
        || *rent_sponsor_info.key != light_token_instruction::rent_sponsor()
        || !rent_sponsor_info.is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    Ok(())
}

pub(super) struct WriterPolicyContext {
    pub group: Box<WriterSettlementGroupV1>,
    pub sleeve: Box<WriterSleeveV1>,
    pub book: Box<WriterSeriesBookV1>,
    pub snapshot: Box<WriterPolicySnapshotV1>,
}

pub(super) struct WriterBookContext {
    pub group: Box<WriterSettlementGroupV1>,
    pub sleeve: Box<WriterSleeveV1>,
    pub book: Box<WriterSeriesBookV1>,
}

#[inline(never)]
pub(super) fn load_writer_book_context(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    group_info: &AccountInfo,
    book_info: &AccountInfo,
) -> Result<WriterBookContext, ProgramError> {
    let group = load_writer_settlement_group(program_id, group_info)?;
    let sleeve = load_writer_sleeve(program_id, sleeve_info, group_info.key)?;
    let book = load_writer_series_book(program_id, book_info, sleeve_info.key, group_info.key)?;
    if sleeve.underlying_id != group.underlying_id
        || sleeve.expiry_ts != group.expiry_ts
        || sleeve.settlement_mint != group.settlement_mint
        || sleeve.series_count != group.series_count
        || sleeve.series_count != book.series_count
        || book.settlement_group != sleeve.settlement_group
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(WriterBookContext {
        group,
        sleeve,
        book,
    })
}

#[inline(never)]
pub(super) fn load_writer_policy_context(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    group_info: &AccountInfo,
    book_info: &AccountInfo,
    snapshot_info: &AccountInfo,
    expected_registry: Option<&Pubkey>,
) -> Result<WriterPolicyContext, ProgramError> {
    let WriterBookContext {
        group,
        sleeve,
        book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let snapshot = load_writer_policy_snapshot(
        program_id,
        snapshot_info,
        sleeve_info.key,
        expected_registry.unwrap_or(&sleeve.policy_registry),
        sleeve.policy_version,
    )?;
    Ok(WriterPolicyContext {
        group,
        sleeve,
        book,
        snapshot,
    })
}

pub(super) fn load_writer_policy_registry(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_vault_config: &Pubkey,
) -> Result<Box<WriterPolicyRegistryV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterPolicyRegistryV1>(
        info,
        program_id,
        WriterPolicyRegistryV1::LEN,
        VaultError::InvalidWriterPolicyRegistry,
    )?);
    let (expected, bump) = derive_writer_policy_registry_pda(program_id);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.vault_config != *expected_vault_config
        || crate::pubkey_is_default(&value.policy_authority)
        || crate::pubkey_is_default(&value.protocol_fee_vault)
        || (crate::pubkey_is_default(&value.pending_policy_authority)
            != (value.pending_activation_slot == 0))
    {
        return Err(VaultError::InvalidWriterPolicyRegistry.into());
    }
    Ok(value)
}

pub(super) fn load_writer_policy_snapshot(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_sleeve: &Pubkey,
    expected_registry: &Pubkey,
    expected_version: u64,
) -> Result<Box<WriterPolicySnapshotV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterPolicySnapshotV1>(
        info,
        program_id,
        WriterPolicySnapshotV1::LEN,
        VaultError::InvalidWriterPolicySnapshot,
    )?);
    let (expected, bump) =
        derive_writer_policy_snapshot_pda(program_id, expected_sleeve, expected_version);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.sleeve != *expected_sleeve
        || value.registry != *expected_registry
        || value.policy_version != expected_version
        || value.max_series != crate::constants::WRITER_MAX_LIVE_SERIES as u8
        || value.drawdown_scale != crate::constants::WRITER_RATIO_SCALE_PPM
        || value.lower_tail_max_settlement_atomic >= value.upper_tail_min_settlement_atomic
        || crate::bytes32_is_zero(&value.policy_hash)
        || crate::bytes32_is_zero(&value.scenario_set_hash)
        || crate::bytes32_is_zero(&value.risk_limit_hash)
        || crate::bytes32_is_zero(&value.series_family_hash)
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    Ok(value)
}

pub(super) fn load_writer_settlement_group(
    program_id: &Pubkey,
    info: &AccountInfo,
) -> Result<Box<WriterSettlementGroupV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterSettlementGroupV1>(
        info,
        program_id,
        WriterSettlementGroupV1::LEN,
        VaultError::InvalidWriterSettlementGroup,
    )?);
    let (expected, bump) = derive_writer_settlement_group_pda(
        program_id,
        &value.underlying_id,
        value.expiry_ts,
        &value.settlement_mint,
    );
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || crate::bytes32_is_zero(&value.underlying_id)
        || value.expiry_ts == 0
        || crate::pubkey_is_default(&value.settlement_mint)
        || crate::pubkey_is_default(&value.anchor_market)
        || crate::pubkey_is_default(&value.anchor_oracle_month)
        || crate::pubkey_is_default(&value.signer_registry)
        || crate::pubkey_is_default(&value.sleeve)
        || value.settlement_ts != value.expiry_ts
        || usize::from(value.series_count) > crate::constants::WRITER_MAX_LIVE_SERIES
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(value)
}

pub(super) fn load_writer_sleeve(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_group: &Pubkey,
) -> Result<Box<WriterSleeveV1>, ProgramError> {
    load_writer_sleeve_inner(program_id, info, Some(expected_group))
}

#[inline(never)]
fn load_writer_sleeve_inner(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_group: Option<&Pubkey>,
) -> Result<Box<WriterSleeveV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterSleeveV1>(
        info,
        program_id,
        WriterSleeveV1::LEN,
        VaultError::InvalidWriterSleeve,
    )?);
    let expected_group = expected_group.unwrap_or(&value.settlement_group);
    let (expected, bump) = derive_writer_sleeve_pda(program_id, expected_group);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.settlement_group != *expected_group
        || value.series_book != derive_writer_series_book_pda(program_id, info.key).0
        || value.usdc_vault != derive_writer_sleeve_usdc_vault_pda(program_id, info.key).0
        || value.flat_mint != derive_writer_flat_mint_pda(program_id, info.key).0
        || value.flat_staging != derive_writer_flat_staging_pda(program_id, info.key).0
        || value.flat_burn_custody != derive_writer_flat_burn_custody_pda(program_id, info.key).0
        || usize::from(value.series_count) > crate::constants::WRITER_MAX_LIVE_SERIES
        || value.exact_reserve_atoms > value.accounted_asset_atoms
        || value.long_liability_remaining_atoms > value.long_liability_initial_atoms
        || value.flat_residual_remaining_atoms > value.flat_residual_initial_atoms
        || value.flat_claim_supply_remaining_atoms > value.flat_supply_snapshot_atoms
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(value)
}

pub(super) fn load_writer_series_book(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_sleeve: &Pubkey,
    expected_group: &Pubkey,
) -> Result<Box<WriterSeriesBookV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterSeriesBookV1>(
        info,
        program_id,
        WriterSeriesBookV1::LEN,
        VaultError::InvalidWriterSeriesBook,
    )?);
    let (expected, bump) = derive_writer_series_book_pda(program_id, expected_sleeve);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.sleeve != *expected_sleeve
        || value.settlement_group != *expected_group
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    for (index, record) in value
        .records
        .iter()
        .take(usize::from(value.series_count))
        .enumerate()
    {
        if !record.active
            || record.reserved != [0; 4]
            || record.contract_size_atoms != MarketMintAccounting::CANONICAL_ATOMIC_SCALE
            || record.total_physical_supply_atoms
                != record
                    .issuer_controlled_atoms
                    .checked_add(record.external_open_interest_atoms)
                    .ok_or(VaultError::ArithmeticOverflow)?
            || (index != 0 && value.records[index - 1].series_id >= record.series_id)
        {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
    }
    Ok(value)
}

pub(super) fn validate_writer_flat_mint(
    sleeve_info: &AccountInfo,
    sleeve: &WriterSleeveV1,
    mint_info: &AccountInfo,
) -> Result<Mint, ProgramError> {
    if *mint_info.key != sleeve.flat_mint || mint_info.owner != &spl_token_program_id() {
        return Err(VaultError::InvalidMint.into());
    }
    let mint = validate_mint_account(mint_info, &spl_token_program_id())?;
    if !mint.is_initialized
        || mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
        || mint.mint_authority != COption::Some(*sleeve_info.key)
        || mint.freeze_authority != COption::None
        || mint.supply != sleeve.flat_par_supply_atoms
    {
        return Err(VaultError::InvalidMint.into());
    }
    Ok(mint)
}

pub(super) fn load_writer_auction(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_sleeve: &Pubkey,
    expected_nonce: u64,
) -> Result<Box<WriterAuctionV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterAuctionV1>(
        info,
        program_id,
        WriterAuctionV1::LEN,
        VaultError::InvalidWriterAuction,
    )?);
    let (expected, bump) = derive_writer_auction_pda(program_id, expected_sleeve, expected_nonce);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.sleeve != *expected_sleeve
        || value.auction_nonce != expected_nonce
        || value.bid_index != derive_writer_bid_index_pda(program_id, info.key).0
        || value.escrow != derive_writer_auction_escrow_pda(program_id, info.key).0
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    Ok(value)
}

pub(super) fn load_writer_bid_index(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_auction: &Pubkey,
) -> Result<Box<WriterBidIndexV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterBidIndexV1>(
        info,
        program_id,
        WriterBidIndexV1::LEN,
        VaultError::InvalidWriterAuction,
    )?);
    let (expected, bump) = derive_writer_bid_index_pda(program_id, expected_auction);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.auction != *expected_auction
    {
        return Err(VaultError::InvalidWriterAuction.into());
    }
    Ok(value)
}

pub(super) fn load_writer_bid(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_auction: &Pubkey,
) -> Result<Box<WriterBidV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterBidV1>(
        info,
        program_id,
        WriterBidV1::LEN,
        VaultError::InvalidWriterBid,
    )?);
    let (expected, bump) =
        derive_writer_bid_pda(program_id, expected_auction, &value.bidder, value.order_id);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.auction != *expected_auction
        || crate::pubkey_is_default(&value.bidder)
        || crate::pubkey_is_default(&value.refund_token_account)
        || crate::pubkey_is_default(&value.claim_destination)
        || value.requested_contract_atoms == 0
        || value.bid_price_per_contract_atoms == 0
        || value.accepted_contract_atoms > value.requested_contract_atoms
        || value.executed_contract_atoms > value.accepted_contract_atoms
        || value
            .premium_charged_atoms
            .checked_add(value.fee_charged_atoms)
            .and_then(|charged| charged.checked_add(value.refunded_atoms))
            .is_none_or(|accounted| accounted > value.escrowed_atoms)
    {
        return Err(VaultError::InvalidWriterBid.into());
    }
    Ok(value)
}

pub(super) fn load_writer_close_request(
    program_id: &Pubkey,
    info: &AccountInfo,
    expected_sleeve: &Pubkey,
    expected_nonce: u64,
) -> Result<Box<WriterCloseRequestV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<WriterCloseRequestV1>(
        info,
        program_id,
        WriterCloseRequestV1::LEN,
        VaultError::InvalidWriterCloseRequest,
    )?);
    let (expected, bump) =
        derive_writer_close_request_pda(program_id, expected_sleeve, expected_nonce);
    if *info.key != expected
        || !value.is_initialized
        || value.bump != bump
        || !value.has_current_layout()
        || value.sleeve != *expected_sleeve
        || value.request_nonce != expected_nonce
        || value.flat_escrow != derive_writer_close_flat_escrow_pda(program_id, info.key).0
        || value.flat_mint == Pubkey::default()
        || value.owner == Pubkey::default()
        || value.flat_amount_atoms == 0
        || value.series_count == 0
    {
        return Err(VaultError::InvalidWriterCloseRequest.into());
    }
    Ok(value)
}
