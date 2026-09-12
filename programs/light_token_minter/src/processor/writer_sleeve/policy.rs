use super::*;

pub(in crate::processor) fn process_initialize_policy_registry(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeWriterPolicyRegistryV1Params,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let fee_vault_info = &accounts[3];
    let settlement_mint_info = &accounts[4];
    let token_program_info = &accounts[5];
    let system_program_info = &accounts[6];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    if crate::pubkey_is_default(&params.policy_authority)
        || params.rotation_delay_slots < crate::constants::MIN_WRITER_POLICY_ROTATION_DELAY_SLOTS
        || config.usdc_mint != *settlement_mint_info.key
    {
        return Err(VaultError::InvalidWriterPolicyRegistry.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;

    let (expected_registry, registry_bump) = derive_writer_policy_registry_pda(program_id);
    let (expected_fee_vault, fee_vault_bump) =
        derive_writer_protocol_fee_vault_pda(program_id, registry_info.key);
    if *registry_info.key != expected_registry || *fee_vault_info.key != expected_fee_vault {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, registry_info)?;
    create_program_account(
        admin_info,
        registry_info,
        system_program_info,
        program_id,
        WriterPolicyRegistryV1::LEN,
        &[
            crate::constants::WRITER_POLICY_REGISTRY_PDA_SEED,
            &[registry_bump],
        ],
    )?;
    create_classic_token_pda(
        program_id,
        admin_info,
        fee_vault_info,
        settlement_mint_info,
        registry_info.key,
        token_program_info,
        system_program_info,
        &[
            crate::constants::WRITER_PROTOCOL_FEE_VAULT_PDA_SEED,
            registry_info.key.as_ref(),
            &[fee_vault_bump],
        ],
    )?;
    validate_vault_token_account(fee_vault_info, settlement_mint_info.key, registry_info.key)?;
    if validate_token_account(fee_vault_info)?.amount != 0 {
        return Err(VaultError::InvalidWriterPolicyRegistry.into());
    }
    let slot = Clock::get()?.slot;
    let registry = WriterPolicyRegistryV1 {
        is_initialized: true,
        bump: registry_bump,
        account_discriminator: WriterPolicyRegistryV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterPolicyRegistryV1::ACCOUNT_VERSION,
        vault_config: *config_info.key,
        policy_authority: params.policy_authority,
        pending_policy_authority: Pubkey::default(),
        protocol_fee_vault: *fee_vault_info.key,
        pending_activation_slot: 0,
        rotation_delay_slots: params.rotation_delay_slots,
        latest_policy_version: 0,
        last_updated_slot: slot,
        reserved: [0; 32],
    };
    store_state(registry_info, &registry)
}

pub(in crate::processor) fn process_manage_policy_authority(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ManageWriterPolicyAuthorityV1Params,
) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    if !actor_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let mut registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let slot = Clock::get()?.slot;
    match params.action {
        ManageWriterPolicyAuthorityActionV1::Propose => {
            if *actor_info.key != config.admin
                || crate::pubkey_is_default(&params.new_authority)
                || params.new_authority == registry.policy_authority
                || !crate::pubkey_is_default(&registry.pending_policy_authority)
            {
                return Err(VaultError::Unauthorized.into());
            }
            registry.pending_policy_authority = params.new_authority;
            registry.pending_activation_slot = slot
                .checked_add(registry.rotation_delay_slots)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        ManageWriterPolicyAuthorityActionV1::Activate => {
            if params.new_authority != registry.pending_policy_authority
                || *actor_info.key != registry.pending_policy_authority
                || slot < registry.pending_activation_slot
            {
                return Err(VaultError::InvalidWriterLifecycle.into());
            }
            registry.policy_authority = registry.pending_policy_authority;
            registry.pending_policy_authority = Pubkey::default();
            registry.pending_activation_slot = 0;
        }
        ManageWriterPolicyAuthorityActionV1::Cancel => {
            if *actor_info.key != config.admin
                || crate::pubkey_is_default(&registry.pending_policy_authority)
            {
                return Err(VaultError::Unauthorized.into());
            }
            registry.pending_policy_authority = Pubkey::default();
            registry.pending_activation_slot = 0;
        }
    }
    registry.last_updated_slot = slot;
    store_state(registry_info, &registry)
}

pub(in crate::processor) fn process_seal_policy(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SealWriterPolicyV1Params,
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let sleeve_info = &accounts[3];
    let group_info = &accounts[4];
    let book_info = &accounts[5];
    let snapshot_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let _config = load_canonical_vault_config(program_id, config_info)?;
    let mut registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    if registry.policy_authority != *authority_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterBookContext {
        mut group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let expected_policy_version = registry
        .latest_policy_version
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if group.sleeve != *sleeve_info.key
        || sleeve.status != WriterSleeveStatus::Draft
        || group.status != WriterSettlementGroupStatus::Anchored
        || book.frozen
        || book.series_count == 0
        || params.policy_version != expected_policy_version
        || params.regime_input_version == 0
        || params.beta_ppm == 0
        || params.beta_ppm >= crate::constants::WRITER_RATIO_SCALE_PPM as u32
        || params.drawdown_scale != crate::constants::WRITER_RATIO_SCALE_PPM
        || params.worst_drawdown_limit > params.drawdown_scale
        || params.upper_drawdown_limit > params.drawdown_scale
        || params.lower_drawdown_limit > params.drawdown_scale
        || params.lower_tail_max_settlement_atomic >= params.upper_tail_min_settlement_atomic
        || params.max_auction_issue_atoms == 0
        || params.max_close_flat_atoms == 0
        || params.primary_fee_bps > 10_000
        || params.security_mode != WriterSecurityMode::GrossExternalMaxPayout
        || params.reserve_rounding_mode != WriterReserveRoundingMode::AggregateBookCeiling
        || params.auction_priority_rule != WriterAuctionPriorityRule::PayAsBidPriceThenSeriesProRata
        || params.v2_feature_flags != 0
        || crate::bytes32_is_zero(&params.scenario_set_hash)
        || crate::bytes32_is_zero(&params.model_margin_vector_hash)
        || crate::bytes32_is_zero(&params.execution_cost_vector_hash)
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let series_family_hash = writer_series_family_hash(&book);
    if params.risk_limit_hash != writer_risk_limit_hash(&params)
        || params.policy_hash
            != writer_policy_hash(
                program_id,
                sleeve_info.key,
                group_info.key,
                &series_family_hash,
                &params,
            )
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let (expected_snapshot, bump) =
        derive_writer_policy_snapshot_pda(program_id, sleeve_info.key, params.policy_version);
    if *snapshot_info.key != expected_snapshot {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, snapshot_info)?;
    create_program_account(
        authority_info,
        snapshot_info,
        system_program_info,
        program_id,
        WriterPolicySnapshotV1::LEN,
        &[
            crate::constants::WRITER_POLICY_SNAPSHOT_PDA_SEED,
            sleeve_info.key.as_ref(),
            &params.policy_version.to_le_bytes(),
            &[bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let snapshot = WriterPolicySnapshotV1 {
        is_initialized: true,
        bump,
        account_discriminator: WriterPolicySnapshotV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterPolicySnapshotV1::ACCOUNT_VERSION,
        sleeve: *sleeve_info.key,
        registry: *registry_info.key,
        policy_version: params.policy_version,
        regime_input_version: params.regime_input_version,
        policy_hash: params.policy_hash,
        scenario_set_hash: params.scenario_set_hash,
        risk_limit_hash: params.risk_limit_hash,
        series_family_hash,
        security_mode: params.security_mode,
        reserve_rounding_mode: params.reserve_rounding_mode,
        auction_priority_rule: params.auction_priority_rule,
        v2_feature_flags: params.v2_feature_flags,
        max_series: crate::constants::WRITER_MAX_LIVE_SERIES as u8,
        reserved_0: 0,
        primary_fee_bps: params.primary_fee_bps,
        reserved_1: [0; 2],
        drawdown_scale: params.drawdown_scale,
        worst_drawdown_limit: params.worst_drawdown_limit,
        upper_drawdown_limit: params.upper_drawdown_limit,
        lower_drawdown_limit: params.lower_drawdown_limit,
        lower_tail_max_settlement_atomic: params.lower_tail_max_settlement_atomic,
        upper_tail_min_settlement_atomic: params.upper_tail_min_settlement_atomic,
        operational_buffer_atoms: params.operational_buffer_atoms,
        max_auction_issue_atoms: params.max_auction_issue_atoms,
        max_close_flat_atoms: params.max_close_flat_atoms,
        created_slot: slot,
        sealed_slot: slot,
        reserved: [0; 32],
    };
    sleeve.policy_snapshot = *snapshot_info.key;
    sleeve.policy_version = params.policy_version;
    sleeve.policy_hash = params.policy_hash;
    sleeve.scenario_set_hash = params.scenario_set_hash;
    sleeve.risk_limit_hash = params.risk_limit_hash;
    sleeve.operational_buffer_atoms = params.operational_buffer_atoms;
    sleeve.security_mode = params.security_mode;
    sleeve.v2_feature_flags = params.v2_feature_flags;
    sleeve.status = WriterSleeveStatus::PolicyFrozen;
    sleeve.last_updated_slot = slot;
    book.frozen = true;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    group.last_updated_slot = slot;
    registry.latest_policy_version = params.policy_version;
    registry.last_updated_slot = slot;
    store_state(snapshot_info, &snapshot)?;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(group_info, &group)?;
    store_state(registry_info, &registry)
}
