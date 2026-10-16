use super::*;

/// Extends the canonical target through one checked Loader-v3 CPI. The
/// transaction envelope is closed and the resulting Loader-written slot and
/// capacity are re-read before the proposal is advanced to `Extended`.
pub fn process_extend_target_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExtendTargetV1,
) -> ProgramResult {
    let [payer, config_info, gate_info, proposal_info, prestate_info, target_programdata, target_program, authority_info, loader_info, system_program_info, rent_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };

    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(config_info, false, false, false)?;
    validate_exact_privileges(gate_info, false, false, false)?;
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(prestate_info, false, false, false)?;
    validate_exact_privileges(target_programdata, true, false, false)?;
    validate_exact_privileges(target_program, true, false, true)?;
    // Loader-v3 interface 5.0.0 marks the checked authority writable.
    validate_exact_privileges(authority_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    validate_exact_privileges(rent_info, false, false, false)?;
    validate_exact_privileges(instructions_info, false, false, false)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        gate_info,
        proposal_info,
        prestate_info,
        target_programdata,
        target_program,
        authority_info,
        loader_info,
        system_program_info,
        rent_info,
        instructions_info,
    ])?;
    validate_program_and_sysvar_ids(
        loader_info,
        system_program_info,
        Some(rent_info),
        None,
        instructions_info,
    )?;
    let rent = Rent::from_account_info(rent_info)?;

    let config = load_config(program_id, config_info)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_frozen_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if proposal.state != ProposalStateV2::Frozen
        || proposal.extension_delta == 0
        || proposal.proposal_class == ProposalClassV1::EmergencyRollback
        || instruction.expected_current_capacity != proposal.current_capacity
        || instruction.expected_extension_delta != proposal.extension_delta
        || instruction.expected_post_capacity != proposal.expected_post_capacity
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_target_account_keys(&config, target_program, target_programdata, authority_info)?;

    let slot = current_frozen_slot(&proposal)?;
    require_extension_execution_runway(&proposal, slot)?;
    let prestate = load_accepted_prestate(
        program_id,
        prestate_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
        &gate,
        &instruction.expected_prestate_checkpoint_digest,
        slot,
    )?;
    let expected_raw = expected_unextended_raw_programdata_hash(&proposal, &prestate)?;
    let before = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if before.deployed_slot != prestate.target_programdata_slot
        || before.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(before.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != proposal.current_capacity
        || slot <= before.deployed_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    require_raw_programdata_hash(target_programdata, &expected_raw)?;

    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack(),
        &instruction.envelope,
    )?;
    let delta = u32::try_from(proposal.extension_delta)
        .map_err(|_| GovernanceError::InvalidCapacityPlan)?;
    let extend = extend_program_checked(
        target_program.key,
        authority_info.key,
        Some(payer.key),
        delta,
    );
    validate_extend_cpi_shape(
        &extend,
        target_programdata.key,
        target_program.key,
        authority_info.key,
        system_program_info.key,
        payer.key,
    )?;

    let authority_bump = derive_authority_pda(program_id, &config.target_program).1;
    let bump_seed = [authority_bump];
    let signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &bump_seed,
    ];
    invoke_signed(
        &extend,
        &[
            target_programdata.clone(),
            target_program.clone(),
            authority_info.clone(),
            system_program_info.clone(),
            payer.clone(),
            loader_info.clone(),
        ],
        &[signer_seeds],
    )?;

    let after = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if after.deployed_slot != slot
        || after.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(after.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != proposal.expected_post_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_zero_appended_extension(
        target_programdata,
        after.payload_offset,
        proposal.current_capacity,
        proposal.extension_delta,
    )?;
    // The checked extension must preserve the exact prefix and append only a
    // zero-filled delta; rent is supplied and decoded before CPI so malformed
    // rent state cannot influence payment behavior.
    let _ = rent;

    proposal.state = ProposalStateV2::Extended;
    proposal.extension_executed_slot = slot;
    validate_proposal_digest_v2(&proposal)?;
    let bytes = encode_fixed_account(&*proposal, UpgradeProposalV2::LEN)?;
    commit_one_fixed_account(program_id, proposal_info, &bytes, UpgradeProposalV2::LEN)
}

/// Performs exactly one canonical Loader-v3 Upgrade CPI and creates the
/// separate deployed-byte verification account. Success never unfreezes.
pub fn process_execute_upgrade_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: ExecuteUpgradeV1,
) -> ProgramResult {
    let [payer, config_info, policy_info, gate_info, proposal_info, counterpart_info, counterpart_verification_info, prestate_info, buffer_verification_info, programdata_verification_info, target_programdata, target_program, buffer_info, spill_info, rent_info, clock_info, authority_info, loader_info, system_program_info, instructions_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };

    validate_exact_privileges(payer, true, true, false)?;
    for account in [
        config_info,
        policy_info,
        gate_info,
        counterpart_info,
        counterpart_verification_info,
        prestate_info,
        rent_info,
        clock_info,
        authority_info,
        instructions_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(proposal_info, true, false, false)?;
    validate_exact_privileges(buffer_verification_info, true, false, false)?;
    validate_exact_privileges(programdata_verification_info, true, false, false)?;
    validate_exact_privileges(target_programdata, true, false, false)?;
    validate_exact_privileges(target_program, true, false, true)?;
    validate_exact_privileges(buffer_info, true, false, false)?;
    validate_exact_privileges(spill_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        config_info,
        policy_info,
        gate_info,
        proposal_info,
        counterpart_info,
        counterpart_verification_info,
        prestate_info,
        buffer_verification_info,
        programdata_verification_info,
        target_programdata,
        target_program,
        buffer_info,
        spill_info,
        rent_info,
        clock_info,
        authority_info,
        loader_info,
        system_program_info,
        instructions_info,
    ])?;
    validate_program_and_sysvar_ids(
        loader_info,
        system_program_info,
        Some(rent_info),
        Some(clock_info),
        instructions_info,
    )?;
    let rent = Rent::from_account_info(rent_info)?;
    let clock = Clock::from_account_info(clock_info)?;

    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info, &config)?;
    let gate = load_gate(program_id, gate_info, config_info, &config)?;
    let mut proposal = load_proposal(program_id, proposal_info, config_info, &config)?;
    validate_frozen_expectation(
        &instruction.expected,
        &proposal,
        &config,
        &gate,
        proposal_info.key,
    )?;
    if instruction.expected.expected_policy_version != policy.version
        || instruction.expected.expected_policy_hash != policy.policy_hash
        || clock.slot == 0
        || clock.slot >= proposal.expiry_slot
    {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let expected_state = if proposal.extension_delta == 0 {
        ProposalStateV2::Frozen
    } else {
        ProposalStateV2::Extended
    };
    if proposal.state != expected_state
        || (proposal.extension_delta != 0
            && (proposal.extension_executed_slot == 0
                || proposal.extension_executed_slot >= clock.slot))
        || instruction.expected_capacity != proposal.expected_post_capacity
        || instruction.expected_verified_chunk_count != proposal.chunk_count
        || instruction.expected_buffer_verification_status != BufferVerificationStatusV1::Verified
    {
        return Err(GovernanceError::InvalidStateTransition.into());
    }
    validate_target_account_keys(&config, target_program, target_programdata, authority_info)?;
    if *spill_info.key != config.canonical_spill_treasury
        || *spill_info.key != proposal.canonical_spill_treasury
        || *buffer_info.key != proposal.buffer_pubkey
        || *buffer_verification_info.key != proposal.buffer_verification
        || *programdata_verification_info.key != proposal.programdata_verification
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    if programdata_verification_info.owner != &system_program::ID
        || programdata_verification_info.data_len() != 0
    {
        return Err(GovernanceError::IncorrectAccountOwner.into());
    }

    let prestate = load_accepted_prestate(
        program_id,
        prestate_info,
        proposal_info,
        config_info,
        &config,
        &proposal,
        &gate,
        &instruction.expected_prestate_checkpoint_digest,
        clock.slot,
    )?;
    if proposal.extension_delta != 0 && prestate.finalized_slot > proposal.extension_executed_slot {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let before = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    let expected_current_slot = if proposal.extension_delta == 0 {
        prestate.target_programdata_slot
    } else {
        proposal.extension_executed_slot
    };
    if before.deployed_slot != expected_current_slot
        || before.deployed_slot != instruction.expected_programdata_slot
        || before.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(before.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != proposal.expected_post_capacity
        || clock.slot <= before.deployed_slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let expected_raw = if proposal.extension_delta == 0 {
        let committed = expected_unextended_raw_programdata_hash(&proposal, &prestate)?;
        if instruction.expected_current_raw_programdata_hash != committed {
            return Err(GovernanceError::Release1DigestMismatch.into());
        }
        committed
    } else {
        // Checked extension is the only controller capability that can have
        // changed ProgramData since the accepted prestate. Its exact earlier
        // slot, resulting capacity, and authority were validated above. The
        // plan binds the resulting raw account bytes immediately before this
        // separate-slot upgrade transaction.
        if proposal.proposal_class == ProposalClassV1::EmergencyRollback
            || instruction.expected_current_raw_programdata_hash == [0; 32]
        {
            return Err(GovernanceError::InvalidCapacityPlan.into());
        }
        instruction.expected_current_raw_programdata_hash
    };
    require_raw_programdata_hash(target_programdata, &expected_raw)?;

    let mut buffer_verification = load_buffer_verification(
        program_id,
        buffer_verification_info,
        proposal_info,
        config_info,
        buffer_info,
        &config,
        &proposal,
    )?;
    if buffer_verification.status != instruction.expected_buffer_verification_status
        || buffer_verification.verified_chunk_count != instruction.expected_verified_chunk_count
        || buffer_verification.sealed_buffer_header_hash
            != instruction.expected_sealed_buffer_header_hash
        || buffer_verification.finalized_slot == 0
        || buffer_verification.finalized_slot > clock.slot
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_live_sealed_buffer(buffer_info, &config, &proposal, &buffer_verification)?;
    validate_reciprocal_counterpart(
        program_id,
        counterpart_info,
        counterpart_verification_info,
        config_info,
        &config,
        &proposal,
        proposal_info.key,
        &instruction,
        clock.slot,
    )?;

    validate_canonical_envelope(
        program_id,
        accounts,
        instructions_info,
        &instruction.pack(),
        &instruction.envelope,
    )?;

    let tail_length = proposal
        .expected_post_capacity
        .checked_sub(proposal.artifact_length)
        .ok_or(GovernanceError::InvalidCapacityPlan)?;
    let tail_chunk_count = chunk_count_allow_empty(tail_length, proposal.chunk_size)?;
    let (_, verification_bump) = derive_programdata_check_pda(program_id, proposal_info.key);
    let verification = ProgramDataVerificationV1 {
        discriminator: PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR,
        account_version: RELEASE1_ACCOUNT_VERSION_V1,
        bump: verification_bump,
        initialized: true,
        status: ProgramDataVerificationStatusV1::Verifying,
        controller_config: *config_info.key,
        proposal: *proposal_info.key,
        target_program: *target_program.key,
        target_programdata: *target_programdata.key,
        upgradeable_loader: *loader_info.key,
        controller_authority: *authority_info.key,
        artifact_length: proposal.artifact_length,
        artifact_sha256: proposal.artifact_sha256,
        artifact_chunk_merkle_root: proposal.artifact_chunk_merkle_root,
        chunk_hash_domain: proposal.chunk_hash_domain,
        chunk_size: proposal.chunk_size,
        payload_chunk_count: proposal.chunk_count,
        verified_payload_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        verified_payload_chunk_count: 0,
        deployed_slot: clock.slot,
        capacity: proposal.expected_post_capacity,
        tail_length,
        tail_chunk_count,
        verified_tail_chunk_bitmap: [0; VERIFICATION_BITMAP_BYTES_V1],
        verified_tail_chunk_count: 0,
        raw_programdata_hash: [0; 32],
        zero_tail_verified: false,
        finalized_slot: 0,
        reserved: [0; PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN],
    };
    verification.validate_schema()?;
    // Both decoded accounts are detached heap allocations. Update those only
    // after every pre-CPI check and encode all final bytes before invoking the
    // loader. This preserves transaction atomicity without retaining duplicate
    // 1,792-byte proposal / verification values on the SBPF stack.
    proposal.state = ProposalStateV2::UpgradeExecuted;
    proposal.upgrade_executed_slot = clock.slot;
    validate_proposal_digest_v2(&proposal)?;
    buffer_verification.status = BufferVerificationStatusV1::ConsumedByUpgrade;
    buffer_verification.terminal_slot = clock.slot;
    buffer_verification.validate_schema()?;
    let proposal_bytes = encode_fixed_account(&*proposal, UpgradeProposalV2::LEN)?;
    let buffer_verification_bytes =
        encode_fixed_account(&*buffer_verification, BufferVerificationV1::LEN)?;
    let programdata_verification_bytes =
        encode_fixed_account(&verification, ProgramDataVerificationV1::LEN)?;

    let upgrade_instruction = upgrade(
        target_program.key,
        buffer_info.key,
        authority_info.key,
        spill_info.key,
    );
    validate_upgrade_cpi_shape(
        &upgrade_instruction,
        target_programdata.key,
        target_program.key,
        buffer_info.key,
        spill_info.key,
        rent_info.key,
        clock_info.key,
        authority_info.key,
    )?;
    let authority_bump = derive_authority_pda(program_id, &config.target_program).1;
    let authority_bump_seed = [authority_bump];
    let authority_signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        AUTHORITY_SEED,
        config.target_program.as_ref(),
        &authority_bump_seed,
    ];
    invoke_signed(
        &upgrade_instruction,
        &[
            target_programdata.clone(),
            target_program.clone(),
            buffer_info.clone(),
            spill_info.clone(),
            rent_info.clone(),
            clock_info.clone(),
            authority_info.clone(),
            loader_info.clone(),
        ],
        &[authority_signer_seeds],
    )?;

    let after = validate_program_programdata_linkage(
        target_program,
        target_programdata,
        &config.upgradeable_loader,
    )?;
    if after.deployed_slot != clock.slot
        || after.upgrade_authority != Some(config.authority_pda)
        || u64::try_from(after.capacity).map_err(|_| GovernanceError::ArithmeticOverflow)?
            != proposal.expected_post_capacity
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    let consumed_buffer_data = buffer_info.try_borrow_data()?;
    if buffer_info.lamports() != 0
        || consumed_buffer_data.len() != LOADER_BUFFER_METADATA_LEN
        || parse_upgradeable_buffer(&consumed_buffer_data)?.authority != Some(config.authority_pda)
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    drop(consumed_buffer_data);

    let verification_bump_seed = [verification_bump];
    let verification_signer_seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        PROGRAMDATA_CHECK_SEED,
        proposal_info.key.as_ref(),
        &verification_bump_seed,
    ];
    create_fixed_pda_account(
        program_id,
        payer,
        programdata_verification_info,
        system_program_info,
        &rent,
        ProgramDataVerificationV1::LEN,
        verification_signer_seeds,
    )?;
    commit_three_fixed_accounts(
        program_id,
        proposal_info,
        &proposal_bytes,
        UpgradeProposalV2::LEN,
        buffer_verification_info,
        &buffer_verification_bytes,
        BufferVerificationV1::LEN,
        programdata_verification_info,
        &programdata_verification_bytes,
        ProgramDataVerificationV1::LEN,
    )
}
