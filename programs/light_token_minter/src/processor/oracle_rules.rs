use super::*;

pub(super) fn process_finalize_oracle_opening_phase(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let active_manifest_info = &accounts[3];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_game_transition_window(&market, &month)?;
    let active_manifest =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_manifest_info)?;
    ensure_finalized_oracle_active_weight_manifest(&month, &active_manifest)?;
    month.phase = OraclePhase::Game;
    month.last_updated_slot = Clock::get()?.slot;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn process_expire_oracle_opening_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() < 4 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let cranker_info = &accounts[0];
    let market_info = &accounts[1];
    let month_info = &accounts[2];
    let source_info = &accounts[3];
    if !cranker_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let (market, mut month) =
        load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    ensure_oracle_game_transition_window(&market, &month)?;
    let mut source = load_valid_oracle_source(program_id, month_info.key, source_info)?;
    if source.status != OracleSourceStatus::Frozen || source.opening_submitted {
        return Err(VaultError::InvalidOracleState.into());
    }
    let resolved_source_count = month
        .opening_resolved_source_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if resolved_source_count > month.frozen_source_count {
        return Err(VaultError::InvalidOracleState.into());
    }
    source.status = OracleSourceStatus::Inactive;
    month.opening_resolved_source_count = resolved_source_count;
    month.last_updated_slot = Clock::get()?.slot;
    store_state(source_info, &source)?;
    store_oracle_month_state(month_info, &month)
}

pub(super) fn oracle_opening_start_ts(month: &OracleMonthState) -> Result<u64, ProgramError> {
    Ok(rulebook_schedule_boundaries(month)?.2)
}

pub fn oracle_update_claim_v2_commitment_hash(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    source_id: &[u8; 32],
    claimant: &Pubkey,
    claim_id: &[u8; 32],
    prior_state: u64,
    new_state: u64,
    source_time: u64,
    evidence_hash: &[u8; 32],
    archive_url_hash: &[u8; 32],
    secret_salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        ORACLE_UPDATE_COMMITMENT_V2_DOMAIN,
        &[OracleUpdateClaimV2::ACCOUNT_VERSION],
        program_id.as_ref(),
        month.as_ref(),
        source.as_ref(),
        source_id,
        claimant.as_ref(),
        claim_id,
        &prior_state.to_le_bytes(),
        &new_state.to_le_bytes(),
        &source_time.to_le_bytes(),
        evidence_hash,
        archive_url_hash,
        secret_salt,
    ])
    .to_bytes()
}

pub fn validate_oracle_update_claim_v2_reveal(
    program_id: &Pubkey,
    month: &Pubkey,
    source_key: &Pubkey,
    claimant: &Pubkey,
    claim_key: &Pubkey,
    source: &OracleSourceState,
    claim: &OracleUpdateClaimV2,
    params: &RevealOracleUpdateClaimV3Params,
    current_slot: u64,
) -> ProgramResult {
    let (expected_claim, expected_bump) = derive_oracle_update_claim_v2_pda(
        program_id,
        month,
        source_key,
        claimant,
        &params.claim_id,
    );
    if *claim_key != expected_claim
        || claim.claim.bump != expected_bump
        || claim.claim.month != *month
        || claim.claim.source != *source_key
        || claim.claim.source_id != source.source_id
        || claim.claim.claimant != *claimant
        || claim.claim.claim_id != params.claim_id
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    if claim.claim.status != OracleClaimStatus::Committed
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || claim.claim.prior_state != 0
        || claim.claim.new_state != 0
        || !crate::bytes32_is_zero(&claim.claim.evidence_hash)
        || claim.revealed_slot != 0
        || current_slot < claim.earliest_reveal_slot
        || current_slot > claim.reveal_deadline_slot
        || source.status != OracleSourceStatus::Active
        || params.prior_state != source.current_state
        || params.new_state == params.prior_state
    {
        return Err(VaultError::InvalidOracleUpdateRevealTiming.into());
    }
    let expected_hash = oracle_update_claim_v2_commitment_hash(
        program_id,
        month,
        source_key,
        &source.source_id,
        claimant,
        &params.claim_id,
        params.prior_state,
        params.new_state,
        params.source_time,
        &params.evidence_hash,
        &derive_oracle_opening_archive_url_hash(&params.archive_url),
        &params.secret_salt,
    );
    if claim.commit_hash != expected_hash {
        return Err(VaultError::OracleUpdateCommitmentMismatch.into());
    }
    Ok(())
}

pub fn settle_expired_oracle_update_commitment_bond(
    claim: &mut OracleUpdateClaimV2,
    ledger: &mut OraclePlayerLedger,
    month: &mut OracleMonthState,
    current_slot: u64,
) -> ProgramResult {
    if claim.claim.status != OracleClaimStatus::Committed
        || claim.claim.stake == 0
        || claim.claim.prior_state != 0
        || claim.claim.new_state != 0
        || !crate::bytes32_is_zero(&claim.claim.evidence_hash)
        || claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || current_slot <= claim.reveal_deadline_slot
        || ledger.owner != claim.claim.claimant
        || month.pending_resolution_count == 0
    {
        return Err(VaultError::OracleEscrowNotSettleable.into());
    }
    release_oracle_major_lock_at_slot(ledger, claim.claim.stake, current_slot)?;
    claim.claim.escrow_disposition = OracleEscrowDisposition::Refunded;
    claim.claim.status = OracleClaimStatus::TimedOut;
    decrement_pending_oracle_resolution(month)?;
    month.last_updated_slot = current_slot;
    Ok(())
}

pub(super) fn validate_oracle_emergency_choice(
    kind: OracleEmergencyDisputeKind,
    choice: u8,
) -> ProgramResult {
    if choice >= oracle_emergency_choice_count(kind) {
        return Err(VaultError::InvalidOracleEmergencyDispute.into());
    }
    Ok(())
}

pub(super) struct DerivedEmergencyPacket {
    pub(super) fallback_choice: u8,
    pub(super) choice_count: u8,
    pub(super) snapshot_slot: u64,
    pub(super) snapshot_total_major_tokens: u64,
}

pub(super) fn oracle_emergency_choice_count(kind: OracleEmergencyDisputeKind) -> u8 {
    match kind {
        OracleEmergencyDisputeKind::Source => 3,
        OracleEmergencyDisputeKind::Update => 3,
        OracleEmergencyDisputeKind::Opening => 3,
        OracleEmergencyDisputeKind::BucketMedian => 2,
    }
}

pub(super) fn oracle_emergency_fallback_choice(kind: OracleEmergencyDisputeKind) -> u8 {
    match kind {
        OracleEmergencyDisputeKind::Source => 0,
        OracleEmergencyDisputeKind::Update => 2,
        OracleEmergencyDisputeKind::Opening => 2,
        OracleEmergencyDisputeKind::BucketMedian => 0,
    }
}

pub(super) fn oracle_emergency_kind_byte(kind: OracleEmergencyDisputeKind) -> u8 {
    match kind {
        OracleEmergencyDisputeKind::Source => 0,
        OracleEmergencyDisputeKind::Update => 1,
        OracleEmergencyDisputeKind::Opening => 2,
        OracleEmergencyDisputeKind::BucketMedian => 3,
    }
}

pub(super) fn oracle_emergency_case_hash(
    month_key: &Pubkey,
    kind: OracleEmergencyDisputeKind,
    target_id: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        b"amoeba-oracle-emergency-case",
        month_key.as_ref(),
        &[oracle_emergency_kind_byte(kind)],
        target_id,
    ])
    .to_bytes()
}

pub(super) fn ppm_amount(amount: u64, ppm: u64) -> Result<u64, ProgramError> {
    Ok(((amount as u128)
        .checked_mul(ppm as u128)
        .ok_or(VaultError::ArithmeticOverflow)?
        / 1_000_000u128) as u64)
}

pub fn initial_oracle_weight_manifest_hash(
    month: &Pubkey,
    expected_source_count: u16,
    expected_bucket_count: u16,
) -> [u8; 32] {
    let source_count = expected_source_count.to_le_bytes();
    let bucket_count = expected_bucket_count.to_le_bytes();
    hashv(&[
        ORACLE_RECIPE_WEIGHT_MANIFEST_HASH_DOMAIN,
        month.as_ref(),
        &[0],
        &source_count,
        &bucket_count,
    ])
    .to_bytes()
}

pub fn advance_oracle_weight_manifest_hash(
    previous_hash: &[u8; 32],
    bucket_id: &[u8; 32],
    source: &OracleSourceState,
    bucket_weight_bps: u16,
) -> [u8; 32] {
    let bucket_weight = bucket_weight_bps.to_le_bytes();
    hashv(&[
        ORACLE_RECIPE_WEIGHT_MANIFEST_HASH_DOMAIN,
        previous_hash,
        bucket_id,
        &source.source_id,
        &source.source_type_hash,
        &source.canonical_locator_hash,
        &source.source_definition_hash,
        &bucket_weight,
    ])
    .to_bytes()
}

pub fn canonical_recipe_digest(month: &Pubkey, canonical_manifest_hash: &[u8; 32]) -> [u8; 32] {
    hashv(&[
        ORACLE_CANONICAL_RECIPE_HASH_DOMAIN,
        month.as_ref(),
        canonical_manifest_hash,
    ])
    .to_bytes()
}

pub fn initial_oracle_settlement_source_digest(
    month: &Pubkey,
    canonical_recipe_hash: &[u8; 32],
    active_weight_manifest_hash: &[u8; 32],
    expected_source_count: u16,
    expected_bucket_count: u16,
) -> [u8; 32] {
    hashv(&[
        ORACLE_SETTLEMENT_SOURCE_DIGEST_DOMAIN,
        month.as_ref(),
        canonical_recipe_hash,
        active_weight_manifest_hash,
        &expected_source_count.to_le_bytes(),
        &expected_bucket_count.to_le_bytes(),
    ])
    .to_bytes()
}

pub fn advance_oracle_settlement_source_digest(
    previous_hash: &[u8; 32],
    bucket: &OracleBucketMedianState,
) -> [u8; 32] {
    hashv(&[
        ORACLE_SETTLEMENT_SOURCE_DIGEST_DOMAIN,
        previous_hash,
        &bucket.bucket_id,
        &bucket.bucket_weight_bps.to_le_bytes(),
        &bucket.frozen_source_count.to_le_bytes(),
        &bucket.active_source_count.to_le_bytes(),
        &bucket.eligible_source_count.to_le_bytes(),
        &bucket.bucket_delta_bps.to_le_bytes(),
        &bucket.source_snapshot_hash,
    ])
    .to_bytes()
}

pub fn validate_canonical_settlement_provenance(
    oracle_month_key: &Pubkey,
    oracle_month: &OracleMonthState,
    recipe_manifest: &OracleRecipeWeightManifest,
    active_manifest: &OracleActiveWeightManifest,
    source_manifest: &OracleSettlementSourceManifest,
    settlement: &CompressedSettlementLeaf,
) -> ProgramResult {
    if settlement.oracle_month != *oracle_month_key
        || crate::bytes32_is_zero(&settlement.recipe_hash)
        || settlement.recipe_hash != oracle_month.recipe_hash
    {
        return Err(VaultError::InvalidOracleMonthAccount.into());
    }

    let recipe_is_complete = recipe_manifest.phase == OracleRecipeWeightPhase::Finalized
        && recipe_manifest.recipe_hash == oracle_month.recipe_hash
        && recipe_manifest.rolling_manifest_hash == oracle_month.weight_manifest_hash
        && recipe_manifest.expected_source_count == oracle_month.frozen_source_count
        && recipe_manifest.processed_source_count == recipe_manifest.expected_source_count
        && recipe_manifest.processed_bucket_count == recipe_manifest.expected_bucket_count
        && recipe_manifest.declared_weight_total_bps == 10_000
        && canonical_recipe_digest(oracle_month_key, &recipe_manifest.rolling_manifest_hash)
            == oracle_month.recipe_hash;
    let source_is_complete = source_manifest.phase == OracleRecipeWeightPhase::Finalized
        && source_manifest.expected_source_count == oracle_month.frozen_source_count
        && source_manifest.processed_source_count == source_manifest.expected_source_count
        && source_manifest.processed_bucket_count == source_manifest.expected_bucket_count
        && source_manifest.declared_weight_total_bps == 10_000
        && !crate::bytes32_is_zero(&source_manifest.rolling_source_digest)
        && settlement.source_digest == source_manifest.rolling_source_digest;
    if crate::bytes32_is_zero(&oracle_month.recipe_hash)
        || oracle_month.weight_scheme_version != 1
        || crate::bytes32_is_zero(&oracle_month.weight_manifest_hash)
        || ensure_finalized_oracle_active_weight_manifest(oracle_month, active_manifest).is_err()
        || !recipe_is_complete
        || !source_is_complete
    {
        return Err(VaultError::InvalidOracleWeightManifest.into());
    }
    Ok(())
}

pub fn validate_active_group_source_identity(
    expected_group_id: &[u8; 32],
    expected_bucket_weight_bps: u16,
    prior_source_count: u16,
    last_source_id: &[u8; 32],
    source: &OracleSourceState,
) -> ProgramResult {
    if source.bucket_id != *expected_group_id
        || source.bucket_weight_bps != expected_bucket_weight_bps
        || (prior_source_count > 0 && source.source_id <= *last_source_id)
    {
        return Err(VaultError::InvalidOracleWeightOrder.into());
    }
    Ok(())
}

pub fn validate_active_group_collection_completion(
    manifest: &OracleActiveWeightManifest,
) -> ProgramResult {
    if manifest.current_group_source_count == 0
        || manifest.current_group_active_count == 0
        || usize::from(manifest.current_group_source_count)
            > crate::constants::MAX_ORACLE_BUCKET_SOURCES
    {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    Ok(())
}

pub fn validate_oracle_active_manifest_completion(
    month: &OracleMonthState,
    recipe: &OracleRecipeWeightManifest,
    manifest: &OracleActiveWeightManifest,
) -> ProgramResult {
    if manifest.phase != OracleRecipeWeightPhase::ReadyToFinalize
        || manifest.processed_source_count != manifest.expected_source_count
        || manifest.processed_group_count != manifest.expected_group_count
        || manifest.expected_source_count != month.frozen_source_count
        || month.opened_source_count == 0
        || month.active_weight_group_count != manifest.expected_group_count
        || manifest.processed_bucket_weight_bps != 10_000
        || manifest.max_open_interest_payout == 0
        || crate::bytes32_is_zero(&manifest.rolling_manifest_hash)
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || recipe.recipe_hash != month.recipe_hash
        || recipe.rolling_manifest_hash != month.weight_manifest_hash
        || !crate::bytes32_is_zero(&month.active_weight_manifest_hash)
    {
        return Err(VaultError::OracleWeightManifestIncomplete.into());
    }
    Ok(())
}

pub fn validate_oracle_active_manifest_begin_membership(
    month: &OracleMonthState,
    recipe: &OracleRecipeWeightManifest,
    expected_source_count: u16,
    expected_group_count: u16,
) -> ProgramResult {
    if expected_source_count == 0
        || expected_source_count != month.frozen_source_count
        || expected_group_count == 0
        || expected_group_count > expected_source_count
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || recipe.recipe_hash != month.recipe_hash
        || recipe.rolling_manifest_hash != month.weight_manifest_hash
        || recipe.expected_source_count != month.frozen_source_count
        || expected_group_count != recipe.expected_bucket_count
    {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    Ok(())
}

pub(super) fn apply_oracle_source_merge(
    challenged_source: &mut OracleSourceState,
    canonical_source: &mut OracleSourceState,
) -> ProgramResult {
    if challenged_source.month != canonical_source.month
        || challenged_source.bucket_id != canonical_source.bucket_id
        || challenged_source.source_id == canonical_source.source_id
        || challenged_source.status != OracleSourceStatus::Candidate
        || canonical_source.status != OracleSourceStatus::Candidate
    {
        return Err(VaultError::InvalidOracleSourceAccount.into());
    }
    canonical_source.support_stake_total = canonical_source
        .support_stake_total
        .checked_add(challenged_source.support_stake_total)
        .ok_or(VaultError::ArithmeticOverflow)?;
    challenged_source.support_stake_total = 0;
    challenged_source.bucket_weight_bps = 0;
    challenged_source.status = OracleSourceStatus::Merged;
    Ok(())
}

pub(super) fn local_file_floor(local_backing: u64, file_ppm: u64) -> Result<u64, ProgramError> {
    Ok(ORACLE_LOCAL_FILE_ABS_MIN.max(ppm_amount(local_backing, file_ppm)?))
}

pub(super) fn oracle_source_backing(source: &OracleSourceState) -> u64 {
    source.support_stake_total.max(source.listing_bond_locked)
}

pub(super) fn oracle_bucket_local_backing(source: &OracleSourceState) -> u64 {
    oracle_source_backing(source)
}

pub fn oracle_update_claim_required_bond(
    month: &OracleMonthState,
    source: &OracleSourceState,
) -> Result<u64, ProgramError> {
    ensure_oracle_canonical_weight_scheme(month)?;
    ensure_oracle_active_weight_scheme(month)?;
    let local_backing = oracle_bucket_local_backing(source);
    let file = local_file_floor(local_backing, ORACLE_UPDATE_FILE_PPM)?;
    let cap = (file.checked_mul(3).ok_or(VaultError::ArithmeticOverflow)?)
        .max(ppm_amount(local_backing, ORACLE_UPDATE_CAP_PPM)?);
    Ok(cap.min(file.max(ORACLE_UPDATE_BASE_BOND)))
}

pub fn validate_oracle_update_claim_v2_commit_bond(
    month: &OracleMonthState,
    source: &OracleSourceState,
    stake: u64,
) -> ProgramResult {
    let required_bond = oracle_update_claim_required_bond(month, source)?;
    if stake < required_bond {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    Ok(())
}

pub(super) fn ensure_live_revealed_oracle_update_claim(
    source_key: &Pubkey,
    source: &OracleSourceState,
    claim: &OracleUpdateClaimData,
) -> ProgramResult {
    if source.status != OracleSourceStatus::Active
        || !source.opening_submitted
        || claim.status != OracleClaimStatus::Revealed
        || claim.source != *source_key
        || claim.source_id != source.source_id
        || claim.prior_state != source.current_state
        || crate::bytes32_is_zero(&claim.evidence_hash)
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    Ok(())
}

pub(super) fn release_oracle_major_lock_at_slot(
    ledger: &mut OraclePlayerLedger,
    amount: u64,
    current_slot: u64,
) -> ProgramResult {
    if ledger.locked_major_tokens < amount {
        return Err(VaultError::InvalidOraclePlayerLedger.into());
    }
    ledger.locked_major_tokens = ledger
        .locked_major_tokens
        .checked_sub(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    ledger.major_tokens = ledger
        .major_tokens
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    ledger.last_updated_slot = current_slot;
    Ok(())
}
