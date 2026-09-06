use super::*;

pub(super) fn load_dual_settlement_governance(
    program_id: &Pubkey,
    admin_info: &AccountInfo,
    oracle_authority_info: &AccountInfo,
    config_info: &AccountInfo,
) -> Result<VaultConfig, ProgramError> {
    if !admin_info.is_signer || !oracle_authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if !config.has_current_layout() {
        return Err(VaultError::InvalidConfigAccount.into());
    }
    if *admin_info.key != config.admin
        || *oracle_authority_info.key != config.oracle_authority
        || admin_info.key == oracle_authority_info.key
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    Ok(config)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_settlement_signer_set(
    registry: Pubkey,
    bump: u8,
    version: u64,
    threshold: u8,
    signer_count: u8,
    signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    rotation_delay_slots: u64,
    proposed_slot: u64,
    activate_after_slot: u64,
    emergency: bool,
    proposer: Pubkey,
) -> Result<SettlementSignerSet, ProgramError> {
    let mut set = SettlementSignerSet {
        is_initialized: true,
        bump,
        account_discriminator: SettlementSignerSet::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerSet::ACCOUNT_VERSION,
        registry,
        version,
        threshold,
        signer_count,
        signers,
        set_hash: [0; 32],
        rotation_delay_slots,
        proposed_slot,
        activate_after_slot,
        emergency,
        proposer,
    };
    set.set_hash = set.compute_set_hash();
    validate_settlement_signer_configuration(&set)?;
    Ok(set)
}

pub(super) fn settlement_signer_set_excludes_governance(
    set: &SettlementSignerSet,
    governance_keys: &[Pubkey],
) -> ProgramResult {
    if set
        .signers
        .iter()
        .take(usize::from(set.signer_count))
        .any(|signer| governance_keys.contains(signer))
    {
        return Err(VaultError::InvalidSettlementSignerConfiguration.into());
    }
    Ok(())
}

pub(super) fn process_initialize_settlement_signer_registry(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeSettlementSignerRegistryParams,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let oracle_authority_info = &accounts[1];
    let recovery_authority_info = &accounts[2];
    let config_info = &accounts[3];
    let registry_info = &accounts[4];
    let signer_set_info = &accounts[5];
    let system_program_info = &accounts[6];
    if !recovery_authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_dual_settlement_governance(
        program_id,
        admin_info,
        oracle_authority_info,
        config_info,
    )?;
    if !config.paused {
        return Err(VaultError::SettlementSignerRecoveryRequiresPause.into());
    }
    if params.signer_set_version != 1
        || params.recovery_authority != *recovery_authority_info.key
        || crate::pubkey_is_default(&params.recovery_authority)
        || recovery_authority_info.key == admin_info.key
        || recovery_authority_info.key == oracle_authority_info.key
        || *system_program_info.key != system_program::id()
    {
        return Err(VaultError::InvalidSettlementSignerConfiguration.into());
    }

    let (expected_registry, registry_bump) = derive_settlement_signer_registry_pda(program_id);
    let (expected_set, set_bump) =
        derive_settlement_signer_set_pda(program_id, params.signer_set_version);
    if *registry_info.key != expected_registry || *signer_set_info.key != expected_set {
        return Err(VaultError::InvalidPda.into());
    }
    validate_settlement_signer_registry_initialization_targets(
        program_id,
        registry_info,
        signer_set_info,
    )?;
    let slot = Clock::get()?.slot;
    let set = build_settlement_signer_set(
        expected_registry,
        set_bump,
        params.signer_set_version,
        params.threshold,
        params.signer_count,
        params.signers,
        params.rotation_delay_slots,
        slot,
        slot,
        false,
        *admin_info.key,
    )?;
    settlement_signer_set_excludes_governance(
        &set,
        &[
            *admin_info.key,
            *oracle_authority_info.key,
            *recovery_authority_info.key,
        ],
    )?;

    create_program_account(
        admin_info,
        signer_set_info,
        system_program_info,
        program_id,
        SettlementSignerSet::LEN,
        &[
            SETTLEMENT_SIGNER_SET_PDA_SEED,
            &params.signer_set_version.to_le_bytes(),
            &[set_bump],
        ],
    )?;
    create_program_account(
        admin_info,
        registry_info,
        system_program_info,
        program_id,
        SettlementSignerRegistry::LEN,
        &[SETTLEMENT_SIGNER_REGISTRY_PDA_SEED, &[registry_bump]],
    )?;
    let registry = SettlementSignerRegistry {
        is_initialized: true,
        bump: registry_bump,
        account_discriminator: SettlementSignerRegistry::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementSignerRegistry::ACCOUNT_VERSION,
        current_set: expected_set,
        current_version: params.signer_set_version,
        pending_set: Pubkey::default(),
        pending_version: 0,
        recovery_authority: params.recovery_authority,
        proposal_nonce: 0,
    };
    store_state(signer_set_info, &set)?;
    store_state(registry_info, &registry)
}

pub(super) fn settlement_signer_rotation_digest(
    program_id: &Pubkey,
    registry_key: &Pubkey,
    current_set_key: &Pubkey,
    current_set: &SettlementSignerSet,
    proposal_nonce: u64,
    pending_set_key: &Pubkey,
    pending_set: &SettlementSignerSet,
) -> Vec<u8> {
    const DOMAIN: &[u8] = b"ameba_settlement_signer_rotation_v1";
    hashv(&[
        DOMAIN,
        program_id.as_ref(),
        registry_key.as_ref(),
        current_set_key.as_ref(),
        &current_set.version.to_le_bytes(),
        &current_set.set_hash,
        &proposal_nonce.to_le_bytes(),
        pending_set_key.as_ref(),
        &pending_set.version.to_le_bytes(),
        &pending_set.set_hash,
        &pending_set.activate_after_slot.to_le_bytes(),
        &pending_set.rotation_delay_slots.to_le_bytes(),
        &[u8::from(pending_set.emergency)],
    ])
    .to_bytes()
    .to_vec()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn verify_settlement_signer_rotation_attestations(
    program_id: &Pubkey,
    registry_key: &Pubkey,
    current_set_key: &Pubkey,
    current_set: &SettlementSignerSet,
    proposal_nonce: u64,
    pending_set_key: &Pubkey,
    pending_set: &SettlementSignerSet,
    instructions_sysvar_info: &AccountInfo,
) -> ProgramResult {
    if *instructions_sysvar_info.key != instructions::id() {
        return Err(VaultError::InvalidInstructionsSysvar.into());
    }
    let current_index = instructions::load_current_index_checked(instructions_sysvar_info)
        .map_err(|_| VaultError::InvalidInstructionsSysvar)?;
    let threshold = usize::from(current_set.threshold);
    if usize::from(current_index) < threshold {
        return Err(VaultError::MissingSettlementOracleSignatures.into());
    }
    let digest = settlement_signer_rotation_digest(
        program_id,
        registry_key,
        current_set_key,
        current_set,
        proposal_nonce,
        pending_set_key,
        pending_set,
    );
    let first = usize::from(current_index) - threshold;
    let mut bitmap = 0u16;
    for instruction_index in first..usize::from(current_index) {
        let instruction =
            instructions::load_instruction_at_checked(instruction_index, instructions_sysvar_info)
                .map_err(|_| VaultError::InvalidInstructionsSysvar)?;
        let signer_index =
            parse_verified_settlement_oracle_instruction(&instruction, &digest, current_set)?;
        let bit = 1u16 << signer_index;
        if bitmap & bit != 0 {
            return Err(VaultError::DuplicateSettlementOracleSigner.into());
        }
        bitmap |= bit;
    }
    if bitmap.count_ones() != u32::from(current_set.threshold) {
        return Err(VaultError::MissingSettlementOracleSignatures.into());
    }
    Ok(())
}

pub(super) fn process_propose_settlement_signer_rotation(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ProposeSettlementSignerRotationParams,
    emergency: bool,
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let oracle_authority_info = &accounts[1];
    let recovery_authority_info = if emergency { Some(&accounts[2]) } else { None };
    let base = 2 + usize::from(emergency);
    let config_info = &accounts[base];
    let registry_info = &accounts[base + 1];
    let current_set_info = &accounts[base + 2];
    let pending_set_info = &accounts[base + 3];
    let instructions_sysvar_info = if emergency { None } else { Some(&accounts[6]) };
    let system_program_info = &accounts[7];
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_dual_settlement_governance(
        program_id,
        admin_info,
        oracle_authority_info,
        config_info,
    )?;
    let mut registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    if !crate::pubkey_is_default(&registry.pending_set) {
        return Err(VaultError::SettlementSignerRotationPending.into());
    }
    let current_set =
        load_canonical_settlement_signer_set(program_id, registry_info.key, current_set_info)?;
    if registry.current_set != *current_set_info.key
        || registry.current_version != current_set.version
        || params.target_version
            != current_set
                .version
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?
    {
        return Err(VaultError::SettlementSignerSetVersionMismatch.into());
    }
    if registry.recovery_authority == config.admin
        || registry.recovery_authority == config.oracle_authority
    {
        return Err(VaultError::SettlementSignerGovernanceRequired.into());
    }
    if !emergency {
        settlement_signer_set_excludes_governance(
            &current_set,
            &[
                config.admin,
                config.oracle_authority,
                registry.recovery_authority,
            ],
        )?;
    }
    if emergency {
        let recovery = recovery_authority_info.ok_or(VaultError::InvalidAccountList)?;
        if !config.paused {
            return Err(VaultError::SettlementSignerRecoveryRequiresPause.into());
        }
        if !recovery.is_signer || *recovery.key != registry.recovery_authority {
            return Err(VaultError::SettlementSignerGovernanceRequired.into());
        }
    }

    let (expected_pending, pending_bump) =
        derive_settlement_signer_set_pda(program_id, params.target_version);
    if *pending_set_info.key != expected_pending {
        return Err(VaultError::InvalidPda.into());
    }
    let slot = Clock::get()?.slot;
    let proposal_nonce = registry
        .proposal_nonce
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let delay = if emergency {
        current_set
            .rotation_delay_slots
            .checked_mul(EMERGENCY_SETTLEMENT_SIGNER_DELAY_MULTIPLIER)
            .ok_or(VaultError::ArithmeticOverflow)?
    } else {
        current_set.rotation_delay_slots
    };
    let minimum_activate_after_slot = slot
        .checked_add(delay)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if params.activate_after_slot < minimum_activate_after_slot {
        return Err(VaultError::SettlementSignerTimelockTooShort.into());
    }
    let pending_set = build_settlement_signer_set(
        *registry_info.key,
        pending_bump,
        params.target_version,
        params.threshold,
        params.signer_count,
        params.signers,
        params.rotation_delay_slots,
        slot,
        params.activate_after_slot,
        emergency,
        *admin_info.key,
    )?;
    settlement_signer_set_excludes_governance(
        &pending_set,
        &[
            *admin_info.key,
            *oracle_authority_info.key,
            registry.recovery_authority,
        ],
    )?;
    if let Some(sysvar_info) = instructions_sysvar_info {
        verify_settlement_signer_rotation_attestations(
            program_id,
            registry_info.key,
            current_set_info.key,
            &current_set,
            proposal_nonce,
            pending_set_info.key,
            &pending_set,
            sysvar_info,
        )?;
    }

    if pending_set_info.owner == program_id {
        // A dual-governance cancellation clears the registry pointer but deliberately leaves the
        // never-activated PDA allocated. A later fully authorized proposal for the same next
        // version may replace only that orphaned pending set; active versions are never mutable.
        let orphaned =
            load_canonical_settlement_signer_set(program_id, registry_info.key, pending_set_info)?;
        if orphaned.version != params.target_version
            || registry.current_set == *pending_set_info.key
            || !crate::pubkey_is_default(&registry.pending_set)
        {
            return Err(VaultError::InvalidSettlementSignerSet.into());
        }
    } else {
        create_program_account(
            admin_info,
            pending_set_info,
            system_program_info,
            program_id,
            SettlementSignerSet::LEN,
            &[
                SETTLEMENT_SIGNER_SET_PDA_SEED,
                &params.target_version.to_le_bytes(),
                &[pending_bump],
            ],
        )?;
    }
    registry.pending_set = expected_pending;
    registry.pending_version = params.target_version;
    registry.proposal_nonce = proposal_nonce;
    store_state(pending_set_info, &pending_set)?;
    store_state(registry_info, &registry)
}

pub(super) fn process_activate_settlement_signer_rotation(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let registry_info = &accounts[0];
    let pending_set_info = &accounts[1];
    let config_info = &accounts[2];
    let mut registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    let pending_set =
        load_canonical_settlement_signer_set(program_id, registry_info.key, pending_set_info)?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    let slot = Clock::get()?.slot;
    validate_settlement_signer_activation(
        &registry,
        pending_set_info.key,
        &pending_set,
        &config,
        slot,
    )?;
    registry.current_set = *pending_set_info.key;
    registry.current_version = pending_set.version;
    registry.pending_set = Pubkey::default();
    registry.pending_version = 0;
    store_state(registry_info, &registry)
}

pub(super) fn validate_settlement_signer_activation(
    registry: &SettlementSignerRegistry,
    pending_set_key: &Pubkey,
    pending_set: &SettlementSignerSet,
    config: &VaultConfig,
    slot: u64,
) -> ProgramResult {
    if registry.pending_set != *pending_set_key
        || registry.pending_version != pending_set.version
        || pending_set.version != registry.current_version.saturating_add(1)
    {
        return Err(VaultError::SettlementSignerSetVersionMismatch.into());
    }
    if !config.has_current_layout()
        || (pending_set.emergency && !config.paused)
        || config.admin == config.oracle_authority
        || registry.recovery_authority == config.admin
        || registry.recovery_authority == config.oracle_authority
    {
        return Err(VaultError::SettlementSignerRecoveryRequiresPause.into());
    }
    settlement_signer_set_excludes_governance(
        pending_set,
        &[
            config.admin,
            config.oracle_authority,
            registry.recovery_authority,
        ],
    )?;
    if slot < pending_set.activate_after_slot {
        return Err(VaultError::SettlementSignerRotationNotReady.into());
    }
    Ok(())
}

pub(super) fn process_cancel_settlement_signer_rotation(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let oracle_authority_info = &accounts[1];
    let config_info = &accounts[2];
    let registry_info = &accounts[3];
    let _ = load_dual_settlement_governance(
        program_id,
        admin_info,
        oracle_authority_info,
        config_info,
    )?;
    let mut registry = load_canonical_settlement_signer_registry(program_id, registry_info)?;
    if crate::pubkey_is_default(&registry.pending_set) {
        return Err(VaultError::InvalidSettlementSignerRegistry.into());
    }
    registry.pending_set = Pubkey::default();
    registry.pending_version = 0;
    store_state(registry_info, &registry)
}
