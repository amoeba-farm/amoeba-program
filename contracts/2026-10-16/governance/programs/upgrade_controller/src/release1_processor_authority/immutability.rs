use super::*;

/// Records the immutable controller release after independently finalized pre/post observations.
///
/// Accounts (exact order): payer W/S; controller Program RO/X; controller ProgramData RO;
/// config RO; capacity policy RO; controller release RO; pre observation RO; post observation RO;
/// immutability receipt W; Upgradeable Loader RO/X; System Program RO/X.
pub fn process_record_controller_immutability_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: RecordControllerImmutabilityV1,
) -> ProgramResult {
    let [payer, controller_program, controller_programdata, config_info, capacity_info, release_info, pre_info, post_info, receipt_info, loader_info, system_program_info] =
        accounts
    else {
        return Err(GovernanceError::InvalidAccountCount.into());
    };
    debug_assert_eq!(
        accounts.len(),
        RECORD_CONTROLLER_IMMUTABILITY_V1_ACCOUNT_COUNT
    );

    validate_exact_privileges(payer, true, true, false)?;
    validate_exact_privileges(controller_program, false, false, true)?;
    for account in [
        controller_programdata,
        config_info,
        capacity_info,
        release_info,
        pre_info,
        post_info,
    ] {
        validate_exact_privileges(account, false, false, false)?;
    }
    validate_exact_privileges(receipt_info, true, false, false)?;
    validate_exact_privileges(loader_info, false, false, true)?;
    validate_exact_privileges(system_program_info, false, false, true)?;
    require_distinct_accounts(&[
        payer,
        controller_program,
        controller_programdata,
        config_info,
        capacity_info,
        release_info,
        pre_info,
        post_info,
        receipt_info,
        loader_info,
        system_program_info,
    ])?;

    let config = load_config(program_id, config_info)?;
    let capacity = load_capacity_policy(program_id, capacity_info, config_info, &config)?;
    let release = load_controller_release(
        program_id,
        release_info,
        capacity_info,
        &capacity,
        controller_programdata.key,
        &config,
    )?;
    let pre = load_observation(
        program_id,
        pre_info,
        config_info,
        &config,
        capacity_info,
        &capacity,
        controller_program.key,
        ProgramDataObservationPurposeV1::ControllerImmutability,
        release_info.key,
    )?;
    let post = load_observation(
        program_id,
        post_info,
        config_info,
        &config,
        capacity_info,
        &capacity,
        controller_program.key,
        ProgramDataObservationPurposeV1::ControllerImmutability,
        release_info.key,
    )?;

    if *controller_program.key != *program_id
        || *controller_programdata.key != derive_upgradeable_programdata_address(program_id).0
        || *loader_info.key != UPGRADEABLE_LOADER_ID
        || *system_program_info.key != system_program::ID
        || instruction.expected_capacity_policy_digest != capacity.policy_digest
        || instruction.expected_release_digest != release.release_digest
        || instruction.expected_pre_observation_digest != pre.observation_digest
        || instruction.expected_post_observation_digest != post.observation_digest
    {
        return Err(GovernanceError::CrossAccountMismatch.into());
    }
    validate_controller_immutability_transition(
        controller_program,
        controller_programdata,
        &release,
        &pre,
        &post,
    )?;

    let (expected_receipt, receipt_bump) =
        derive_controller_immutability_receipt_pda(program_id, &config.target_program);
    if *receipt_info.key != expected_receipt {
        return Err(GovernanceError::InvalidPda.into());
    }
    let receipt = ControllerImmutabilityReceiptV1 {
        discriminator: CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
        version: CEREMONY_ACCOUNT_VERSION_V1,
        bump: receipt_bump,
        initialized: true,
        controller_program: *program_id,
        controller_programdata: *controller_programdata.key,
        upgradeable_loader: UPGRADEABLE_LOADER_ID,
        capacity_policy: *capacity_info.key,
        capacity_policy_digest: capacity.policy_digest,
        release_commitment: *release_info.key,
        release_commitment_digest: release.release_digest,
        pre_observation: *pre_info.key,
        pre_observation_generation: pre.generation,
        pre_observation_root: pre.final_raw_merkle_root,
        pre_observation_digest: pre.observation_digest,
        pre_upgrade_authority: pre.upgrade_authority,
        post_observation: *post_info.key,
        post_observation_generation: post.generation,
        post_observation_root: post.final_raw_merkle_root,
        post_observation_digest: post.observation_digest,
        post_upgrade_authority: post.upgrade_authority,
        deployed_slot: post.deployed_slot,
        raw_programdata_length: post.raw_data_length,
        programdata_capacity: post.actual_capacity,
        artifact_length: release.artifact_length,
        artifact_sha256: release.artifact_sha256,
        artifact_merkle_root: release.artifact_merkle_root,
        artifact_scheme_id: release.artifact_scheme_id,
        source_commitment: release.source_commitment,
        build_inputs_commitment: release.build_inputs_commitment,
        package_commitment: release.package_commitment,
        release_manifest_commitment: release.release_manifest_commitment,
        finalized_slot: post.finalized_slot,
        receipt_digest: instruction.expected_receipt_digest,
        finalized: true,
        reserved: [0; CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN],
    };
    if compute_controller_immutability_receipt_digest_v1(&receipt)?
        != instruction.expected_receipt_digest
    {
        return Err(GovernanceError::Release1DigestMismatch.into());
    }
    validate_controller_immutability_receipt_digest_v1(&receipt)?;
    let receipt_bytes = encode_fixed_account(&receipt, ControllerImmutabilityReceiptV1::LEN)?;

    let bump_seed = [receipt_bump];
    create_fixed_pda_account(
        program_id,
        payer,
        receipt_info,
        system_program_info,
        &Rent::get()?,
        ControllerImmutabilityReceiptV1::LEN,
        &[
            UPGRADE_SEED_DOMAIN_V1,
            CONTROLLER_IMMUTABILITY_SEED,
            config.target_program.as_ref(),
            &bump_seed,
        ],
    )?;
    commit_preencoded(program_id, &[(receipt_info, &receipt_bytes)])
}
