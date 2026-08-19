use super::*;

pub(super) fn invoke_light_token_account_transfer<'a>(
    amount: u64,
    decimals: u8,
    light_token_program_info: &AccountInfo<'a>,
    compressed_token_authority_info: &AccountInfo<'a>,
    payer_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    spl_interface_pda_info: &AccountInfo<'a>,
    spl_token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> ProgramResult {
    invoke_light_token_account_transfer_with_signer_seeds(
        amount,
        decimals,
        light_token_program_info,
        compressed_token_authority_info,
        payer_info,
        source_info,
        destination_info,
        authority_info,
        mint_info,
        spl_interface_pda_info,
        spl_token_program_info,
        system_program_info,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn invoke_light_token_account_transfer_with_signer_seeds<'a>(
    amount: u64,
    decimals: u8,
    light_token_program_info: &AccountInfo<'a>,
    compressed_token_authority_info: &AccountInfo<'a>,
    payer_info: &AccountInfo<'a>,
    source_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    authority_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    spl_interface_pda_info: &AccountInfo<'a>,
    spl_token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    if *light_token_program_info.key != light_token_program_id() {
        return Err(VaultError::InvalidLightTokenProgram.into());
    }
    if *compressed_token_authority_info.key != cpi_authority() {
        return Err(VaultError::InvalidCompressedTokenAuthority.into());
    }
    let (expected_spl_interface_pda, spl_interface_bump) =
        light_token_instruction::get_spl_interface_pda_and_bump(mint_info.key);
    if *spl_interface_pda_info.key != expected_spl_interface_pda {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }

    let transfer_ix = light_token_instruction::transfer_interface(
        source_info.key,
        destination_info.key,
        amount,
        decimals,
        authority_info.key,
        payer_info.key,
        mint_info.key,
        spl_interface_pda_info.key,
        spl_interface_bump,
        spl_token_program_info.key,
        source_info.owner,
        destination_info.owner,
    )?;

    let account_infos = [
        light_token_program_info.clone(),
        compressed_token_authority_info.clone(),
        payer_info.clone(),
        mint_info.clone(),
        source_info.clone(),
        destination_info.clone(),
        authority_info.clone(),
        spl_interface_pda_info.clone(),
        spl_token_program_info.clone(),
        system_program_info.clone(),
    ];
    if signer_seeds.is_empty() {
        invoke(&transfer_ix, &account_infos)
    } else {
        invoke_signed(&transfer_ix, &account_infos, signer_seeds)
    }
}

#[inline(never)]
pub(super) fn invoke_system_create_account<'a>(
    payer_info: &AccountInfo<'a>,
    target_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    lamports: u64,
    size: usize,
    owner: &Pubkey,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    invoke_signed(
        &system_instruction::create_account(
            payer_info.key,
            target_info.key,
            lamports,
            size as u64,
            owner,
        ),
        &[
            payer_info.clone(),
            target_info.clone(),
            system_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_system_transfer<'a>(
    source_info: &AccountInfo<'a>,
    destination_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    lamports: u64,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    invoke_signed(
        &system_instruction::transfer(source_info.key, destination_info.key, lamports),
        &[
            source_info.clone(),
            destination_info.clone(),
            system_program_info.clone(),
        ],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_system_allocate<'a>(
    target_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    size: usize,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    invoke_signed(
        &system_instruction::allocate(target_info.key, size as u64),
        &[target_info.clone(), system_program_info.clone()],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_system_assign<'a>(
    target_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    owner: &Pubkey,
    signer_seeds: &[&[&[u8]]],
) -> ProgramResult {
    invoke_signed(
        &system_instruction::assign(target_info.key, owner),
        &[target_info.clone(), system_program_info.clone()],
        signer_seeds,
    )
}

#[inline(never)]
pub(super) fn invoke_create_or_allocate_account<'a>(
    payer_info: &AccountInfo<'a>,
    target_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    owner: &Pubkey,
    size: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    let rent = Rent::get()?;
    let rent_lamports = rent.minimum_balance(size);
    let existing_lamports = target_info.lamports();
    if existing_lamports == 0 {
        return invoke_system_create_account(
            payer_info,
            target_info,
            system_program_info,
            rent_lamports,
            size,
            owner,
            &[signer_seeds],
        );
    }

    let rent_shortfall = rent_lamports.saturating_sub(existing_lamports);
    if rent_shortfall > 0 {
        invoke_system_transfer(
            payer_info,
            target_info,
            system_program_info,
            rent_shortfall,
            &[],
        )?;
    }
    invoke_system_allocate(target_info, system_program_info, size, &[signer_seeds])?;
    invoke_system_assign(target_info, system_program_info, owner, &[signer_seeds])
}

pub(super) fn create_program_account<'a>(
    payer_info: &AccountInfo<'a>,
    target_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    owner: &Pubkey,
    size: usize,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    if target_info.owner == owner {
        return Ok(());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    if target_info.owner != &system_program::id()
        || target_info.executable
        || target_info.data_len() != 0
    {
        return Err(VaultError::InvalidPda.into());
    }
    let mut namespaced_signer_seeds = Vec::with_capacity(signer_seeds.len() + 1);
    namespaced_signer_seeds.push(CURRENT_STATE_NAMESPACE_SEED);
    namespaced_signer_seeds.extend_from_slice(signer_seeds);
    invoke_create_or_allocate_account(
        payer_info,
        target_info,
        system_program_info,
        owner,
        size,
        &namespaced_signer_seeds,
    )
}

pub(super) fn close_program_account(
    program_id: &Pubkey,
    target_info: &AccountInfo,
    recipient_info: &AccountInfo,
) -> ProgramResult {
    if target_info.key == recipient_info.key
        || target_info.owner != program_id
        || target_info.executable
        || !target_info.is_writable
        || !recipient_info.is_writable
    {
        return Err(VaultError::InvalidPda.into());
    }
    let target_lamports = target_info.lamports();
    let recipient_lamports = recipient_info
        .lamports()
        .checked_add(target_lamports)
        .ok_or(VaultError::ArithmeticOverflow)?;
    **recipient_info.try_borrow_mut_lamports()? = recipient_lamports;
    **target_info.try_borrow_mut_lamports()? = 0;
    target_info.resize(0)?;
    target_info.assign(&system_program::id());
    Ok(())
}

pub(super) fn validate_create_only_program_account_target(
    program_id: &Pubkey,
    target_info: &AccountInfo,
) -> ProgramResult {
    if target_info.owner == program_id {
        return Err(VaultError::AlreadyInitialized.into());
    }
    if !target_info.is_writable
        || target_info.owner != &system_program::id()
        || target_info.executable
        || target_info.data_len() != 0
    {
        return Err(VaultError::InvalidPda.into());
    }
    Ok(())
}

pub(super) fn validate_canonical_system_zero_pda_proof(
    expected: &Pubkey,
    account_info: &AccountInfo,
) -> ProgramResult {
    if account_info.key != expected
        || account_info.owner != &system_program::id()
        || account_info.executable
        || account_info.data_len() != 0
    {
        return Err(VaultError::InvalidPda.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn load_valid_oracle_support_position(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    source_id: &[u8; 32],
    support_info: &AccountInfo,
    invalid_state: VaultError,
) -> Result<OracleSupportPosition, ProgramError> {
    let support: OracleSupportPosition = load_exact_zero_padded_state(
        support_info,
        program_id,
        OracleSupportPosition::LEN,
        VaultError::InvalidOracleState,
    )?;
    let (expected, bump) = derive_oracle_support_pda(program_id, month, source, &support.supporter);
    if *support_info.key != expected
        || !support.is_initialized
        || support.bump != bump
        || support.month != *month
        || support.source != *source
        || support.source_id != *source_id
        || support.support_stake == 0
    {
        return Err(invalid_state.into());
    }
    Ok(support)
}

#[inline(never)]
pub(super) fn load_valid_oracle_source_challenge(
    program_id: &Pubkey,
    month: &Pubkey,
    challenge_info: &AccountInfo,
) -> Result<OracleSourceChallenge, ProgramError> {
    let challenge: OracleSourceChallenge = load_exact_zero_padded_state(
        challenge_info,
        program_id,
        OracleSourceChallenge::LEN,
        VaultError::InvalidOracleChallengeAccount,
    )?;
    let (expected, bump) = derive_oracle_source_challenge_pda(
        program_id,
        month,
        &challenge.source,
        &challenge.challenge_id,
    );
    let has_comparison = !crate::pubkey_is_default(&challenge.comparison_source)
        && !crate::bytes32_is_zero(&challenge.comparison_source_id);
    if !challenge.is_initialized
        || *challenge_info.key != expected
        || challenge.bump != bump
        || challenge.month != *month
        || crate::pubkey_is_default(&challenge.source)
        || crate::bytes32_is_zero(&challenge.source_id)
        || crate::bytes32_is_zero(&challenge.challenge_id)
        || crate::pubkey_is_default(&challenge.challenger)
        || crate::bytes32_is_zero(&challenge.evidence_hash)
        || challenge.bond == 0
        || challenge.bond != challenge.required_bond
        || crate::pubkey_is_default(&challenge.comparison_source)
            != (crate::bytes32_is_zero(&challenge.comparison_source_id))
        || (challenge.reason == ORACLE_SOURCE_REASON_NON_INDEPENDENT) != has_comparison
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    Ok(challenge)
}

pub(super) fn load_canonical_source_challenge_guard(
    program_id: &Pubkey,
    month: &Pubkey,
    source: &Pubkey,
    source_id: &[u8; 32],
    guard_info: &AccountInfo,
) -> Result<OracleSourceChallengeGuard, ProgramError> {
    let guard: OracleSourceChallengeGuard = load_exact_zero_padded_state(
        guard_info,
        program_id,
        OracleSourceChallengeGuard::LEN,
        VaultError::InvalidOracleChallengeAccount,
    )?;
    let (expected, bump) = derive_oracle_source_challenge_guard_pda(program_id, month, source);
    if !guard.is_initialized
        || guard.bump != bump
        || guard.account_discriminator != OracleSourceChallengeGuard::ACCOUNT_DISCRIMINATOR
        || guard.account_version != OracleSourceChallengeGuard::ACCOUNT_VERSION
        || *guard_info.key != expected
        || guard.month != *month
        || guard.source != *source
        || guard.source_id != *source_id
        || (crate::pubkey_is_default(&guard.active_challenge))
            != (crate::bytes32_is_zero(&guard.active_challenge_id))
        || (crate::pubkey_is_default(&guard.active_challenge)
            && !crate::pubkey_is_default(&guard.active_dispute))
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    Ok(guard)
}

pub(super) fn reserve_source_challenge_guard(
    guard: &mut OracleSourceChallengeGuard,
    challenge: &Pubkey,
    challenge_id: &[u8; 32],
    slot: u64,
) -> ProgramResult {
    if crate::pubkey_is_default(&*challenge)
        || crate::bytes32_is_zero(&*challenge_id)
        || !crate::pubkey_is_default(&guard.active_challenge)
        || !crate::bytes32_is_zero(&guard.active_challenge_id)
        || !crate::pubkey_is_default(&guard.active_dispute)
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    guard.active_challenge = *challenge;
    guard.active_challenge_id = *challenge_id;
    guard.active_dispute = Pubkey::default();
    guard.last_updated_slot = slot;
    Ok(())
}

pub(super) fn bind_source_challenge_guard_dispute(
    guard: &mut OracleSourceChallengeGuard,
    challenge: &Pubkey,
    challenge_id: &[u8; 32],
    expected_dispute: &Pubkey,
    dispute: &Pubkey,
    slot: u64,
) -> ProgramResult {
    if crate::pubkey_is_default(&*dispute)
        || guard.active_challenge != *challenge
        || guard.active_challenge_id != *challenge_id
        || guard.active_dispute != *expected_dispute
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    guard.active_dispute = *dispute;
    guard.last_updated_slot = slot;
    Ok(())
}

pub(super) fn release_source_challenge_guard(
    guard: &mut OracleSourceChallengeGuard,
    challenge: &Pubkey,
    challenge_id: &[u8; 32],
    expected_dispute: &Pubkey,
    slot: u64,
) -> ProgramResult {
    if guard.active_challenge != *challenge
        || guard.active_challenge_id != *challenge_id
        || guard.active_dispute != *expected_dispute
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    guard.active_challenge = Pubkey::default();
    guard.active_challenge_id = [0; 32];
    guard.active_dispute = Pubkey::default();
    guard.last_updated_slot = slot;
    Ok(())
}

pub(super) fn load_canonical_update_challenge_guard(
    program_id: &Pubkey,
    month: &Pubkey,
    claim: &Pubkey,
    claim_id: &[u8; 32],
    guard_info: &AccountInfo,
) -> Result<OracleUpdateChallengeGuard, ProgramError> {
    let guard: OracleUpdateChallengeGuard = load_exact_zero_padded_state(
        guard_info,
        program_id,
        OracleUpdateChallengeGuard::LEN,
        VaultError::InvalidOracleUpdateAccount,
    )?;
    let (expected, bump) = derive_oracle_update_challenge_guard_pda(program_id, month, claim);
    if !guard.is_initialized
        || guard.bump != bump
        || guard.account_discriminator != OracleUpdateChallengeGuard::ACCOUNT_DISCRIMINATOR
        || guard.account_version != OracleUpdateChallengeGuard::ACCOUNT_VERSION
        || *guard_info.key != expected
        || guard.month != *month
        || guard.claim != *claim
        || guard.claim_id != *claim_id
        || crate::pubkey_is_default(&guard.challenge)
        || crate::bytes32_is_zero(&guard.challenge_id)
        || guard.created_slot == 0
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    Ok(guard)
}

pub(super) fn validate_settlement_signer_registry_initialization_targets(
    program_id: &Pubkey,
    registry_info: &AccountInfo,
    signer_set_info: &AccountInfo,
) -> ProgramResult {
    validate_create_only_program_account_target(program_id, registry_info)?;
    validate_create_only_program_account_target(program_id, signer_set_info)
}

/// Closed set of persistent account types that may use the generic canonical loader. Adding a new
/// caller requires declaring its exact allocation here, preventing prefix-compatible reads by
/// construction.
pub(super) trait CanonicalStateLength {
    const CANONICAL_LEN: usize;
}

macro_rules! impl_canonical_state_length {
    ($($state:ty),+ $(,)?) => {
        $(
            impl CanonicalStateLength for $state {
                const CANONICAL_LEN: usize = <$state>::LEN;
            }
        )+
    };
}

impl_canonical_state_length!(
    Market,
    OracleSourceChallenge,
    OracleSourceState,
    OracleSupportPosition,
    OracleRecipeWeightManifest,
    OracleSkuCoverageManifest,
    OracleSkuCoverageRecord,
    OracleOpeningClaim,
);

#[inline(never)]
pub(super) fn validate_state_account_allocation(
    account_info: &AccountInfo,
    program_id: &Pubkey,
    expected_len: usize,
    invalid_error: VaultError,
) -> ProgramResult {
    if account_info.owner != program_id || account_info.data_len() != expected_len {
        return Err(invalid_error.into());
    }
    Ok(())
}

pub(super) fn load_state<T>(
    account_info: &AccountInfo,
    program_id: &Pubkey,
) -> Result<T, ProgramError>
where
    T: crate::fixed_codec::FixedStateDecode + CanonicalStateLength,
{
    validate_state_account_allocation(
        account_info,
        program_id,
        T::CANONICAL_LEN,
        VaultError::InvalidConfigAccount,
    )?;
    let data = account_info.try_borrow_data()?;
    // SAFETY: The exact canonical allocation was checked above.
    unsafe { T::decode_fixed(&data) }.map_err(|_| VaultError::InvalidConfigAccount.into())
}

pub(super) fn load_exact_zero_padded_state<T>(
    account_info: &AccountInfo,
    program_id: &Pubkey,
    expected_len: usize,
    invalid_error: VaultError,
) -> Result<T, ProgramError>
where
    T: crate::fixed_codec::FixedStateDecode,
{
    validate_state_account_allocation(account_info, program_id, expected_len, invalid_error)?;
    let data = account_info.try_borrow_data()?;
    // SAFETY: The exact requested allocation was checked above.
    unsafe { T::decode_fixed(&data) }.map_err(|_| invalid_error.into())
}

pub(super) fn load_valid_oracle_opening_challenge(
    program_id: &Pubkey,
    month: &Pubkey,
    challenge_info: &AccountInfo,
) -> Result<OracleOpeningClaimChallenge, ProgramError> {
    let mut challenge: OracleOpeningClaimChallenge = load_exact_zero_padded_state(
        challenge_info,
        program_id,
        OracleOpeningClaimChallenge::LEN,
        VaultError::InvalidOracleChallengeAccount,
    )?;
    let (expected, bump) = derive_oracle_opening_claim_challenge_pda(
        program_id,
        month,
        &challenge.claim,
        &challenge.challenge_id,
    );
    if !challenge.is_initialized
        || !challenge.has_valid_account_layout()
        || challenge.month != *month
        || crate::bytes32_is_zero(&challenge.challenge_id)
        || crate::pubkey_is_default(&challenge.claim)
        || crate::pubkey_is_default(&challenge.source)
        || crate::bytes32_is_zero(&challenge.source_id)
        || crate::pubkey_is_default(&challenge.challenger)
        || *challenge_info.key != expected
        || challenge.bump != bump
    {
        return Err(VaultError::InvalidOracleChallengeAccount.into());
    }
    challenge.stamp_current_account_layout();
    Ok(challenge)
}

pub(super) fn load_valid_oracle_update_claim(
    program_id: &Pubkey,
    month: &Pubkey,
    claim_info: &AccountInfo,
) -> Result<OracleUpdateClaimData, ProgramError> {
    let claim = load_valid_oracle_update_claim_v2_from_account(
        program_id,
        month,
        &Pubkey::default(),
        claim_info,
    )?;
    if claim.claim.prior_state == 0
        || claim.claim.new_state == 0
        || claim.claim.status == OracleClaimStatus::Committed
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    Ok(claim.claim)
}

pub(super) fn load_valid_oracle_update_claim_v2_from_account(
    program_id: &Pubkey,
    month: &Pubkey,
    expected_source: &Pubkey,
    claim_info: &AccountInfo,
) -> Result<OracleUpdateClaimV2, ProgramError> {
    let claim: OracleUpdateClaimV2 = load_exact_zero_padded_state(
        claim_info,
        program_id,
        OracleUpdateClaimV2::LEN,
        VaultError::InvalidOracleUpdateAccount,
    )?;
    let (expected, bump) = derive_oracle_update_claim_v2_pda(
        program_id,
        month,
        &claim.claim.source,
        &claim.claim.claimant,
        &claim.claim.claim_id,
    );
    let committed_shape = claim.claim.status == OracleClaimStatus::Committed
        && claim.claim.prior_state == 0
        && claim.claim.new_state == 0
        && claim.claim.source_time == 0
        && crate::bytes32_is_zero(&claim.claim.evidence_hash)
        && crate::bytes32_is_zero(&claim.claim.archive_url_hash)
        && claim.freshness_reward_multiplier == 1
        && claim.revealed_slot == 0;
    let revealed_or_terminal_shape = !matches!(
        claim.claim.status,
        OracleClaimStatus::Open | OracleClaimStatus::Committed
    ) && claim.claim.prior_state != 0
        && claim.claim.new_state != 0
        && claim.claim.source_time != 0
        && !crate::bytes32_is_zero(&claim.claim.evidence_hash)
        && !crate::bytes32_is_zero(&claim.claim.archive_url_hash)
        && matches!(
            claim.freshness_reward_multiplier,
            1 | crate::constants::ORACLE_FRESH_UPDATE_REWARD_MULTIPLIER
        )
        && claim.revealed_slot >= claim.earliest_reveal_slot
        && claim.revealed_slot <= claim.reveal_deadline_slot;
    let expired_shape = claim.claim.status == OracleClaimStatus::TimedOut
        && claim.claim.prior_state == 0
        && claim.claim.new_state == 0
        && claim.claim.source_time == 0
        && crate::bytes32_is_zero(&claim.claim.evidence_hash)
        && crate::bytes32_is_zero(&claim.claim.archive_url_hash)
        && claim.freshness_reward_multiplier == 1
        && claim.revealed_slot == 0
        && claim.claim.escrow_disposition == OracleEscrowDisposition::Refunded;
    if !claim.claim.is_initialized
        || claim.claim.account_discriminator != OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR
        || claim.claim.account_version != OracleUpdateClaimV2::ACCOUNT_VERSION
        || claim.claim.month != *month
        || crate::bytes32_is_zero(&claim.claim.claim_id)
        || crate::pubkey_is_default(&claim.claim.source)
        || (!crate::pubkey_is_default(&*expected_source) && claim.claim.source != *expected_source)
        || crate::bytes32_is_zero(&claim.claim.source_id)
        || crate::pubkey_is_default(&claim.claim.claimant)
        || claim.claim.stake == 0
        || crate::bytes32_is_zero(&claim.commit_hash)
        || claim.commit_slot == 0
        || claim.earliest_reveal_slot
            != claim
                .commit_slot
                .checked_add(ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS)
                .ok_or(VaultError::ArithmeticOverflow)?
        || claim.reveal_deadline_slot
            != claim
                .earliest_reveal_slot
                .checked_add(ORACLE_UPDATE_REVEAL_WINDOW_SLOTS)
                .ok_or(VaultError::ArithmeticOverflow)?
        || (!committed_shape && !revealed_or_terminal_shape && !expired_shape)
        || *claim_info.key != expected
        || claim.claim.bump != bump
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    Ok(claim)
}

pub(super) fn store_oracle_update_claim(
    account_info: &AccountInfo,
    claim: &OracleUpdateClaimData,
) -> ProgramResult {
    if account_info.data_len() != OracleUpdateClaimV2::LEN {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    let data = account_info.try_borrow_data()?;
    // SAFETY: The exact claim allocation was checked above.
    let mut current = unsafe {
        <OracleUpdateClaimV2 as crate::fixed_codec::FixedStateDecode>::decode_fixed(&data)
    }
    .map_err(|_| VaultError::InvalidOracleUpdateAccount)?;
    drop(data);
    current.claim = claim.clone();
    current.claim.account_discriminator = OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR;
    current.claim.account_version = OracleUpdateClaimV2::ACCOUNT_VERSION;
    store_state(account_info, &current)
}

pub(super) fn oracle_update_claim_samba_checkpoint_active(
    program_id: &Pubkey,
    account_info: &AccountInfo,
) -> Result<bool, ProgramError> {
    if account_info.data_len() != OracleUpdateClaimV2::LEN {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    let current: OracleUpdateClaimV2 = load_exact_zero_padded_state(
        account_info,
        program_id,
        OracleUpdateClaimV2::LEN,
        VaultError::InvalidOracleUpdateAccount,
    )?;
    Ok(current.samba_checkpoint_active)
}

pub(super) fn load_valid_oracle_active_weight_manifest(
    program_id: &Pubkey,
    month: &Pubkey,
    manifest_info: &AccountInfo,
) -> Result<OracleActiveWeightManifest, ProgramError> {
    let manifest: OracleActiveWeightManifest = load_exact_zero_padded_state(
        manifest_info,
        program_id,
        OracleActiveWeightManifest::LEN,
        VaultError::InvalidOracleActiveWeightManifest,
    )?;
    let (expected, bump) = derive_oracle_active_weight_manifest_pda(program_id, month);
    if !manifest.is_initialized
        || manifest.account_discriminator != OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR
        || manifest.account_version != OracleActiveWeightManifest::ACCOUNT_VERSION
        || manifest.month != *month
        || manifest.bump != bump
        || *manifest_info.key != expected
    {
        return Err(VaultError::InvalidOracleActiveWeightManifest.into());
    }
    Ok(manifest)
}

pub(super) fn load_valid_oracle_bucket_median(
    program_id: &Pubkey,
    month: &Pubkey,
    bucket_info: &AccountInfo,
) -> Result<OracleBucketMedianState, ProgramError> {
    let bucket: OracleBucketMedianState = load_exact_zero_padded_state(
        bucket_info,
        program_id,
        OracleBucketMedianState::LEN,
        VaultError::InvalidOracleMedian,
    )?;
    let (expected, bump) = derive_oracle_bucket_median_pda(program_id, month, &bucket.bucket_id);
    if *bucket_info.key != expected
        || !bucket.is_initialized
        || bucket.bump != bump
        || bucket.account_discriminator != OracleBucketMedianState::ACCOUNT_DISCRIMINATOR
        || bucket.account_version != OracleBucketMedianState::ACCOUNT_VERSION
        || bucket.month != *month
        || crate::bytes32_is_zero(&bucket.bucket_id)
        || bucket.bucket_weight_bps == 0
        || bucket.bucket_weight_bps > 10_000
        || u32::from(bucket.bucket_weight_start_bps) + u32::from(bucket.bucket_weight_bps) > 10_000
        || bucket.frozen_source_count == 0
        || bucket.active_source_count == 0
        || bucket.active_source_count > bucket.frozen_source_count
        || bucket.eligible_source_count > bucket.active_source_count
    {
        return Err(VaultError::InvalidOracleMedian.into());
    }
    Ok(bucket)
}

pub(super) fn validate_active_oracle_source(
    program_id: &Pubkey,
    month: &OracleMonthState,
    month_key: &Pubkey,
    manifest_info: &AccountInfo,
    source: &OracleSourceState,
) -> ProgramResult {
    let manifest = load_valid_oracle_active_weight_manifest(program_id, month_key, manifest_info)?;
    ensure_oracle_opening_resolution_complete(month)?;
    if manifest.phase != OracleRecipeWeightPhase::Finalized
        || manifest.rolling_manifest_hash != month.active_weight_manifest_hash
        || manifest.expected_source_count != month.frozen_source_count
        || manifest.processed_source_count != manifest.expected_source_count
        || manifest.expected_group_count != month.active_weight_group_count
        || manifest.processed_group_count != manifest.expected_group_count
        || manifest.processed_bucket_weight_bps != 10_000
    {
        return Err(VaultError::OracleActiveWeightSchemeUnverified.into());
    }
    if source.status == OracleSourceStatus::Inactive {
        if source.observation_count == 0 && crate::bytes32_is_zero(&source.rolling_observation_hash)
        {
            return Ok(());
        }
    } else if source.status == OracleSourceStatus::Active
        && source.baseline_state != 0
        && source.current_state != 0
        && source.observation_count > 0
        && !crate::bytes32_is_zero(&source.rolling_observation_hash)
    {
        return Ok(());
    }
    Err(VaultError::InvalidOracleObservation.into())
}

pub(super) fn load_valid_oracle_update_challenge(
    program_id: &Pubkey,
    month: &Pubkey,
    challenge_info: &AccountInfo,
) -> Result<OracleUpdateChallenge, ProgramError> {
    let mut challenge: OracleUpdateChallenge = load_exact_zero_padded_state(
        challenge_info,
        program_id,
        OracleUpdateChallenge::LEN,
        VaultError::InvalidOracleUpdateAccount,
    )?;
    let (expected, bump) = derive_oracle_update_challenge_pda(
        program_id,
        month,
        &challenge.claim,
        &challenge.challenge_id,
    );
    if !challenge.is_initialized
        || !challenge.has_valid_account_layout()
        || challenge.month != *month
        || crate::bytes32_is_zero(&challenge.challenge_id)
        || crate::pubkey_is_default(&challenge.claim)
        || crate::bytes32_is_zero(&challenge.claim_id)
        || crate::pubkey_is_default(&challenge.challenger)
        || challenge.alternative_state == 0
        || challenge.bond == 0
        || challenge.required_bond == 0
        || crate::bytes32_is_zero(&challenge.evidence_hash)
        || *challenge_info.key != expected
        || challenge.bump != bump
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    challenge.stamp_current_account_layout();
    Ok(challenge)
}
