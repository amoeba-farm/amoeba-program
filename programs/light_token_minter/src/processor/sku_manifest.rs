use super::*;

pub(super) fn padded_ascii_underlying_matches(underlying_id: &[u8; 32], expected: &[u8]) -> bool {
    expected.len() <= underlying_id.len()
        && underlying_id[..expected.len()] == *expected
        && underlying_id[expected.len()..]
            .iter()
            .all(|byte| *byte == 0)
}

pub(super) fn expected_oracle_product_sku_count(underlying_id: &[u8; 32]) -> Option<u16> {
    if padded_ascii_underlying_matches(underlying_id, b"ram-standardized-baskets") {
        Some(RAMX_ORACLE_PRODUCT_SKU_COUNT)
    } else if padded_ascii_underlying_matches(underlying_id, b"nand-standardized-baskets") {
        Some(NANDX_ORACLE_PRODUCT_SKU_COUNT)
    } else {
        None
    }
}

pub(super) fn load_valid_oracle_product_sku_draft(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
    draft_nonce: u64,
    draft_info: &AccountInfo,
) -> Result<OracleProductSkuDraft, ProgramError> {
    let expected_count = expected_oracle_product_sku_count(underlying_id)
        .ok_or(VaultError::InvalidOracleProductSkuDraft)?;
    let draft: OracleProductSkuDraft = load_exact_zero_padded_state(
        draft_info,
        program_id,
        OracleProductSkuDraft::LEN,
        VaultError::InvalidOracleProductSkuDraft,
    )?;
    let (expected, bump) =
        derive_oracle_product_sku_draft_pda(program_id, underlying_id, draft_nonce);
    if *draft_info.key != expected
        || !draft.is_initialized
        || draft.bump != bump
        || !draft.has_canonical_layout()
        || draft.underlying_id != *underlying_id
        || draft.draft_nonce != draft_nonce
        || draft.expected_sku_count != expected_count
    {
        return Err(VaultError::InvalidOracleProductSkuDraft.into());
    }
    Ok(draft)
}

pub(super) fn load_valid_oracle_product_sku_manifest(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
    manifest_info: &AccountInfo,
) -> Result<OracleProductSkuManifest, ProgramError> {
    let manifest: OracleProductSkuManifest = load_exact_zero_padded_state(
        manifest_info,
        program_id,
        OracleProductSkuManifest::LEN,
        VaultError::InvalidOracleSkuCoverageManifest,
    )?;
    let (expected, bump) = derive_oracle_product_sku_manifest_pda(program_id, underlying_id);
    let expected_count = expected_oracle_product_sku_count(underlying_id)
        .ok_or(VaultError::InvalidOracleSkuCoverageManifest)?;
    if *manifest_info.key != expected
        || !manifest.is_initialized
        || manifest.bump != bump
        || !manifest.has_canonical_layout()
        || manifest.underlying_id != *underlying_id
        || crate::bytes32_is_zero(&manifest.required_sku_root)
        || manifest.required_sku_count != expected_count
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    Ok(manifest)
}

pub(super) fn load_valid_oracle_maturity_ladder_registry(
    program_id: &Pubkey,
    underlying_id: &[u8; 32],
    registry_info: &AccountInfo,
) -> Result<OracleMaturityLadderRegistry, ProgramError> {
    let registry: OracleMaturityLadderRegistry = load_exact_zero_padded_state(
        registry_info,
        program_id,
        OracleMaturityLadderRegistry::LEN,
        VaultError::InvalidOracleMaturityLadder,
    )?;
    let (expected, bump) = derive_oracle_maturity_ladder_registry_pda(program_id, underlying_id);
    if *registry_info.key != expected
        || !registry.is_initialized
        || registry.bump != bump
        || !registry.has_canonical_layout()
        || registry.underlying_id != *underlying_id
        || crate::bytes32_is_zero(&registry.underlying_id)
    {
        return Err(VaultError::InvalidOracleMaturityLadder.into());
    }
    validate_rolling_three_month_maturity(registry.planned_listing_ts, registry.planned_expiry_ts)
        .map_err(|_| ProgramError::from(VaultError::InvalidOracleMaturityLadder))?;
    Ok(registry)
}

pub(super) fn append_oracle_sku_frontier_node(
    frontier: &mut [[u8; 32]; crate::state::ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
    frontier_mask: &mut u16,
    mut node: [u8; 32],
) -> ProgramResult {
    for (level, frontier_node) in frontier.iter_mut().enumerate() {
        let bit = 1u16
            .checked_shl(u32::try_from(level).map_err(|_| VaultError::ArithmeticOverflow)?)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if *frontier_mask & bit == 0 {
            *frontier_node = node;
            *frontier_mask |= bit;
            return Ok(());
        }
        node = hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, frontier_node, &node]).to_bytes();
        *frontier_node = [0; 32];
        *frontier_mask &= !bit;
    }
    Err(VaultError::InvalidOracleProductSkuDraft.into())
}

pub(super) fn append_oracle_sku_frontier_leaf(
    frontier: &mut [[u8; 32]; crate::state::ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
    frontier_mask: &mut u16,
    index: u16,
    sku_id: &[u8; 32],
) -> ProgramResult {
    let leaf = hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index.to_le_bytes(), sku_id]).to_bytes();
    append_oracle_sku_frontier_node(frontier, frontier_mask, leaf)
}

pub(super) fn finalized_oracle_sku_frontier_root(
    draft: &OracleProductSkuDraft,
) -> Result<[u8; 32], ProgramError> {
    if !draft.has_canonical_layout() || draft.appended_sku_count != draft.expected_sku_count {
        return Err(VaultError::InvalidOracleProductSkuDraft.into());
    }
    let depth = oracle_sku_merkle_proof_depth(draft.expected_sku_count)?;
    let width = 1u16
        .checked_shl(u32::try_from(depth).map_err(|_| VaultError::ArithmeticOverflow)?)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let mut frontier = draft.merkle_frontier;
    let mut frontier_mask = draft.frontier_mask;
    for index in draft.appended_sku_count..width {
        let empty = hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &index.to_le_bytes()]).to_bytes();
        append_oracle_sku_frontier_node(&mut frontier, &mut frontier_mask, empty)?;
    }
    let root_bit = 1u16
        .checked_shl(u32::try_from(depth).map_err(|_| VaultError::ArithmeticOverflow)?)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let root = frontier
        .get(depth)
        .copied()
        .ok_or(VaultError::InvalidOracleProductSkuDraft)?;
    if frontier_mask != root_bit || crate::bytes32_is_zero(&root) {
        return Err(VaultError::InvalidOracleProductSkuDraft.into());
    }
    Ok(root)
}

pub(super) fn append_oracle_product_sku_chunk(
    draft: &mut OracleProductSkuDraft,
    expected_start_index: u16,
    sku_id_chunk: &[[u8; 32]],
) -> Result<Option<[u8; 32]>, ProgramError> {
    let expected_count = expected_oracle_product_sku_count(&draft.underlying_id)
        .ok_or(VaultError::InvalidOracleProductSkuDraft)?;
    let stored_count = draft.appended_sku_count;
    let chunk_count =
        u16::try_from(sku_id_chunk.len()).map_err(|_| VaultError::InvalidOracleProductSkuDraft)?;
    let resulting_count = stored_count
        .checked_add(chunk_count)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if !draft.is_initialized
        || !draft.has_canonical_layout()
        || draft.expected_sku_count != expected_count
        || stored_count == expected_count
        || expected_start_index != stored_count
        || sku_id_chunk.is_empty()
        || sku_id_chunk.len() > MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS
        || resulting_count > expected_count
        || sku_id_chunk.iter().any(crate::bytes32_is_zero)
        || sku_id_chunk.windows(2).any(|pair| pair[0] >= pair[1])
        || (stored_count > 0
            && draft.last_sku_id.ge(sku_id_chunk
                .first()
                .ok_or(VaultError::InvalidOracleProductSkuDraft)?))
    {
        return Err(VaultError::InvalidOracleProductSkuDraft.into());
    }

    for sku_id in sku_id_chunk {
        let index = draft.appended_sku_count;
        append_oracle_sku_frontier_leaf(
            &mut draft.merkle_frontier,
            &mut draft.frontier_mask,
            index,
            sku_id,
        )?;
        draft.appended_sku_count = draft
            .appended_sku_count
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        draft.last_sku_id = *sku_id;
    }
    if draft.appended_sku_count == expected_count {
        finalized_oracle_sku_frontier_root(draft).map(Some)
    } else {
        Ok(None)
    }
}

#[inline(never)]
pub(super) fn process_configure_oracle_product_sku_manifest(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ConfigureOracleProductSkuManifestParams,
) -> ProgramResult {
    if accounts.len() != 6
        || !accounts[0].is_signer
        || !accounts[1].is_signer
        || !accounts[0].is_writable
        || !accounts[3].is_writable
        || !accounts[4].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let oracle_authority_info = &accounts[1];
    let config_info = &accounts[2];
    let draft_info = &accounts[3];
    let manifest_info = &accounts[4];
    let system_program_info = &accounts[5];
    validate_system_program(system_program_info)?;
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if *admin_info.key != config.admin
        || *oracle_authority_info.key != config.oracle_authority
        || admin_info.key == oracle_authority_info.key
    {
        return Err(VaultError::Unauthorized.into());
    }

    let expected_count = expected_oracle_product_sku_count(&params.underlying_id)
        .ok_or(VaultError::InvalidOracleProductSkuDraft)?;
    let (expected_draft, draft_bump) =
        derive_oracle_product_sku_draft_pda(program_id, &params.underlying_id, params.draft_nonce);
    if *draft_info.key != expected_draft {
        return Err(VaultError::InvalidOracleProductSkuDraft.into());
    }
    let (expected_manifest, manifest_bump) =
        derive_oracle_product_sku_manifest_pda(program_id, &params.underlying_id);
    if *manifest_info.key != expected_manifest {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    validate_create_only_program_account_target(program_id, manifest_info)?;

    let draft_is_new = draft_info.owner != program_id;
    let mut draft = if draft_is_new {
        validate_create_only_program_account_target(program_id, draft_info)?;
        if params.expected_start_index != 0 {
            return Err(VaultError::InvalidOracleProductSkuDraft.into());
        }
        OracleProductSkuDraft {
            is_initialized: true,
            bump: draft_bump,
            account_discriminator: OracleProductSkuDraft::ACCOUNT_DISCRIMINATOR,
            account_version: OracleProductSkuDraft::ACCOUNT_VERSION,
            underlying_id: params.underlying_id,
            draft_nonce: params.draft_nonce,
            expected_sku_count: expected_count,
            appended_sku_count: 0,
            frontier_mask: 0,
            last_sku_id: [0; 32],
            merkle_frontier: [[0; 32]; crate::state::ORACLE_PRODUCT_SKU_FRONTIER_NODE_COUNT],
            last_updated_slot: 0,
        }
    } else {
        load_valid_oracle_product_sku_draft(
            program_id,
            &params.underlying_id,
            params.draft_nonce,
            draft_info,
        )?
    };

    let finalized_root = append_oracle_product_sku_chunk(
        &mut draft,
        params.expected_start_index,
        &params.sku_id_chunk,
    )?;
    let slot = Clock::get()?.slot;
    draft.last_updated_slot = slot;

    if draft_is_new {
        create_program_account(
            admin_info,
            draft_info,
            system_program_info,
            program_id,
            OracleProductSkuDraft::LEN,
            &[
                ORACLE_PRODUCT_SKU_DRAFT_PDA_SEED,
                &params.underlying_id,
                &params.draft_nonce.to_le_bytes(),
                &[draft_bump],
            ],
        )?;
    }

    if let Some(required_sku_root) = finalized_root {
        if crate::bytes32_is_zero(&required_sku_root) {
            return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
        }
        create_oracle_manifest_account(
            admin_info,
            manifest_info,
            system_program_info,
            program_id,
            &params.underlying_id,
            OracleProductSkuManifest::LEN,
            ORACLE_PRODUCT_SKU_MANIFEST_PDA_SEED,
            manifest_bump,
        )?;
        let manifest = OracleProductSkuManifest {
            is_initialized: true,
            bump: manifest_bump,
            account_discriminator: OracleProductSkuManifest::ACCOUNT_DISCRIMINATOR,
            account_version: OracleProductSkuManifest::ACCOUNT_VERSION,
            underlying_id: params.underlying_id,
            required_sku_root,
            required_sku_count: expected_count,
            reserved: [0; 6],
            last_updated_slot: slot,
        };
        store_state(manifest_info, &manifest)?;
        close_program_account(program_id, draft_info, admin_info)
    } else {
        store_state(draft_info, &draft)
    }
}

#[inline(never)]
pub(super) fn process_initialize_oracle_month_v5(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeOracleMonthV5Params,
) -> ProgramResult {
    if accounts.len() != 10 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let payer_info = &accounts[1];
    let market_info = &accounts[2];
    let month_info = &accounts[3];
    let coverage_info = &accounts[4];
    let config_info = &accounts[5];
    let economics_info = &accounts[6];
    let product_sku_manifest_info = &accounts[7];
    let maturity_ladder_info = &accounts[8];
    let system_program_info = &accounts[9];

    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    if params.settlement_base_oracle_atomic == 0
        || crate::bytes32_is_zero(&params.required_sku_root)
        || params.required_sku_count == 0
        || params.required_sku_count > MAX_ORACLE_REQUIRED_SKUS
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    let market = load_valid_market(program_id, market_info)?;
    let product_sku_manifest = load_valid_oracle_product_sku_manifest(
        program_id,
        &market.instrument.underlying_id,
        product_sku_manifest_info,
    )?;
    if params.required_sku_root != product_sku_manifest.required_sku_root
        || params.required_sku_count != product_sku_manifest.required_sku_count
    {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    validate_rolling_rulebook_schedule_params(
        &market,
        &InitializeOracleMonthV3Params {
            scramble_start_ts: params.scramble_start_ts,
            listing_ts: params.listing_ts,
            settlement_base_oracle_atomic: params.settlement_base_oracle_atomic,
        },
    )?;
    let economics_config = load_canonical_oracle_economics_config(program_id, economics_info)?;
    let (slot, current_ts) = current_slot_and_unix_timestamp()?;
    let underlying_id = market.instrument.underlying_id;
    let (expected_maturity_ladder, maturity_ladder_bump) =
        derive_oracle_maturity_ladder_registry_pda(program_id, &underlying_id);
    if *maturity_ladder_info.key != expected_maturity_ladder {
        return Err(VaultError::InvalidOracleMaturityLadder.into());
    }
    let (maturity_ladder, maturity_ladder_changed) = if maturity_ladder_info.owner == program_id {
        let mut existing = load_valid_oracle_maturity_ladder_registry(
            program_id,
            &underlying_id,
            maturity_ladder_info,
        )?;
        match validate_oracle_maturity_ladder_transition(
            &existing,
            params.listing_ts,
            market.instrument.expiry_ts,
            current_ts,
        )? {
            OracleMaturityLadderTransition::SameRung => (existing, false),
            OracleMaturityLadderTransition::Advance => {
                existing.planned_listing_ts = params.listing_ts;
                existing.planned_expiry_ts = market.instrument.expiry_ts;
                existing.last_updated_slot = slot;
                (existing, true)
            }
        }
    } else {
        validate_create_only_program_account_target(program_id, maturity_ladder_info)?;
        create_program_account(
            payer_info,
            maturity_ladder_info,
            system_program_info,
            program_id,
            OracleMaturityLadderRegistry::LEN,
            &[
                ORACLE_MATURITY_LADDER_PDA_SEED,
                &underlying_id,
                &[maturity_ladder_bump],
            ],
        )?;
        (
            OracleMaturityLadderRegistry {
                is_initialized: true,
                bump: maturity_ladder_bump,
                account_discriminator: OracleMaturityLadderRegistry::ACCOUNT_DISCRIMINATOR,
                account_version: OracleMaturityLadderRegistry::ACCOUNT_VERSION,
                underlying_id,
                planned_listing_ts: params.listing_ts,
                planned_expiry_ts: market.instrument.expiry_ts,
                last_updated_slot: slot,
            },
            true,
        )
    };
    let (expected_month, month_bump) =
        derive_oracle_month_pda(program_id, market_info.key, market.instrument.expiry_ts);
    if *month_info.key != expected_month {
        return Err(VaultError::InvalidOracleMonthAccount.into());
    }
    if month_info.owner == program_id {
        let existing = load_oracle_month_state(month_info, program_id)?;
        if existing.is_initialized {
            return Err(VaultError::AlreadyInitialized.into());
        }
    } else {
        let expiry_seed = market.instrument.expiry_ts.to_le_bytes();
        create_program_account(
            payer_info,
            month_info,
            system_program_info,
            program_id,
            OracleMonthState::LEN,
            &[
                ORACLE_MONTH_PDA_SEED,
                market_info.key.as_ref(),
                &expiry_seed,
                &[month_bump],
            ],
        )?;
    }

    let (expected_coverage, coverage_bump) =
        derive_oracle_sku_coverage_manifest_pda(program_id, month_info.key);
    if *coverage_info.key != expected_coverage {
        return Err(VaultError::InvalidOracleSkuCoverageManifest.into());
    }
    if coverage_info.owner == program_id {
        let existing: OracleSkuCoverageManifest = load_state(coverage_info, program_id)?;
        if existing.is_initialized {
            return Err(VaultError::AlreadyInitialized.into());
        }
    } else {
        create_program_account(
            payer_info,
            coverage_info,
            system_program_info,
            program_id,
            OracleSkuCoverageManifest::LEN,
            &[
                crate::constants::ORACLE_SKU_COVERAGE_MANIFEST_PDA_SEED,
                month_info.key.as_ref(),
                &[coverage_bump],
            ],
        )?;
    }

    let month = OracleMonthState {
        is_initialized: true,
        bump: month_bump,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        market: *market_info.key,
        authority: *authority_info.key,
        scramble_start_ts: params.scramble_start_ts,
        listing_ts: params.listing_ts,
        phase: OraclePhase::SourceSubmission,
        last_updated_slot: slot,
        settlement_base_oracle_atomic: params.settlement_base_oracle_atomic,
        economics: economics_config.economics,
        candidate_count_tracking_version: OracleMonthState::CANDIDATE_COUNT_TRACKING_VERSION,
        active_weight_initialization_version:
            OracleMonthState::ACTIVE_WEIGHT_INITIALIZATION_VERSION,
        schedule_version: OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION,
        work_reward_currency_version: OracleMonthState::WORK_REWARD_CURRENCY_USDC_V1,
        ..OracleMonthState::default()
    };
    let coverage = OracleSkuCoverageManifest {
        is_initialized: true,
        bump: coverage_bump,
        account_discriminator: OracleSkuCoverageManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSkuCoverageManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        required_sku_root: params.required_sku_root,
        required_sku_count: params.required_sku_count,
        covered_sku_count: 0,
        planned_scramble_start_ts: params.scramble_start_ts,
        planned_listing_ts: params.listing_ts,
        coverage_finalized: false,
        coverage_complete_ts: 0,
        last_updated_slot: slot,
    };
    if maturity_ladder_changed {
        store_state(maturity_ladder_info, &maturity_ladder)?;
    }
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}
