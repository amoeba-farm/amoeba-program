//! One consumed, council-authorized September document bootstrap. The ordinary
//! Game source/observation/recipe layouts and all trading/custody rules survive.
//! Parent rows, not aliases, carry economic weight. No participant funds or
//! invented public ballots are created. Tag 30 remains governance-gated.
use super::*;
use crate::constants::*;
use crate::oracle_parent_proxy::september_bootstrap::{
    SeptemberBootstrapPlan, DOMAIN, EXPIRY, MAX_PAYOUT_PER_CONTRACT_ATOMS, PROGRAM,
};
use crate::oracle_parent_proxy::{
    CfmRegistry, NANDX_REGISTRY_HASH, NANDX_TERMINAL_ROOT, RAMX_REGISTRY_HASH, RAMX_TERMINAL_ROOT,
    REGISTRY_DOMAIN,
};
use crate::state::*;
use borsh::{BorshDeserialize, BorshSerialize};

const SEED: &[u8] = b"g3-september-bootstrap-v1";
const RECEIPT_LEN: usize = 242;

#[derive(BorshSerialize, BorshDeserialize)]
struct Receipt {
    magic: [u8; 4],
    version: u8,
    bump: u8,
    market_index: u8,
    cursor: u8,
    finished: bool,
    month: Pubkey,
    plan_hash: [u8; 32],
    registry_hash: [u8; 32],
    initial_month_hash: [u8; 32],
    proposer: Pubkey,
    council_epoch: u64,
    seats_hash: [u8; 32],
    approving_seats: u8,
    begun_at: u64,
    original_scramble: u64,
    original_listing: u64,
    finished_at: u64,
}

fn invalid<T>() -> Result<T, ProgramError> {
    Err(VaultError::InvalidOracleState.into())
}
fn address(program: &Pubkey, month: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CURRENT_STATE_NAMESPACE_SEED, SEED, month.as_ref()],
        program,
    )
}
fn save(info: &AccountInfo, value: &Receipt) -> ProgramResult {
    if info.data_len() != RECEIPT_LEN {
        return invalid();
    }
    let mut data = info.try_borrow_mut_data()?;
    let mut output = &mut data[..];
    value
        .serialize(&mut output)
        .map_err(|_| VaultError::InvalidOracleState)?;
    if !output.is_empty() {
        return invalid();
    }
    Ok(())
}
fn receipt(
    program: &Pubkey,
    info: &AccountInfo,
    month: &Pubkey,
    market: u8,
    digest: &[u8; 32],
) -> Result<Receipt, ProgramError> {
    let (key, bump) = address(program, month);
    if info.owner != program
        || info.executable
        || info.key != &key
        || info.data_len() != RECEIPT_LEN
    {
        return invalid();
    }
    let r = Receipt::try_from_slice(&info.try_borrow_data()?)
        .map_err(|_| VaultError::InvalidOracleState)?;
    if r.magic != *b"SCBR"
        || r.version != 1
        || r.bump != bump
        || r.month != *month
        || r.market_index != market
        || r.plan_hash != *digest
        || r.council_epoch == 0
        || r.approving_seats.count_ones() != 3
        || r.approving_seats & !31 != 0
        || r.cursor > if market < 2 { 13 } else { 22 }
        || r.begun_at == 0
    {
        return invalid();
    }
    Ok(r)
}

fn context(
    program: &Pubkey,
    a: &[AccountInfo],
    market_index: u8,
    digest: &[u8; 32],
) -> Result<
    (
        Market,
        OracleMonthState,
        SeptemberBootstrapPlan,
        CfmRegistry,
    ),
    ProgramError,
> {
    if !cfg!(feature = "mainnet-v3")
        || *program != PROGRAM
        || *program != crate::id()
        || market_index > 3
        || current_unix_timestamp()? >= EXPIRY
    {
        return invalid();
    }
    validate_current_account_creation_payer(&a[0])?;
    let (market, month) = load_valid_market_and_oracle_month(program, &a[1], &a[2])?;
    validate_launch_market(&market)?;
    let product = market_index / 2;
    let underlying: &[u8] = if product == 0 {
        b"ram-standardized-baskets"
    } else {
        b"nand-standardized-baskets"
    };
    let side = if market_index % 2 == 0 {
        OptionKind::CallSpread
    } else {
        OptionKind::PutSpread
    };
    if market.instrument.expiry_ts != EXPIRY
        || market.instrument.kind != side
        || !padded_ascii_underlying_matches(&market.instrument.underlying_id, underlying)
        || market.instrument.max_payout_per_contract != MAX_PAYOUT_PER_CONTRACT_ATOMS
        || !market.paused
        || market.total_position_collateral_locked != 0
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
        || month.is_cfm_parent_proxy()
        || month.pending_resolution_count != 0
    {
        return invalid();
    }
    let plan =
        oracle_evidence::with_sealed_definition_preimage(program, &a[4], digest, |preimage| {
            let bytes = preimage
                .strip_prefix(DOMAIN)
                .and_then(|bytes| bytes.strip_prefix(program.as_ref()))
                .ok_or(VaultError::InvalidOracleState)?;
            SeptemberBootstrapPlan::decode(program, bytes)
        })?;
    if plan.digest() != *digest {
        return invalid();
    }
    let registry_hash = if product == 0 {
        RAMX_REGISTRY_HASH
    } else {
        NANDX_REGISTRY_HASH
    };
    let registry = oracle_evidence::with_sealed_definition_preimage(
        program,
        &a[5],
        &registry_hash,
        |bytes| {
            CfmRegistry::decode(
                bytes
                    .strip_prefix(REGISTRY_DOMAIN)
                    .ok_or(VaultError::InvalidOracleState)?,
            )
        },
    )?;
    if registry.product() != product {
        return invalid();
    }
    Ok((market, month, plan, registry))
}

fn metadata(
    registry: &CfmRegistry,
    row: usize,
    month: Pubkey,
    proposer: Pubkey,
) -> Result<OracleSourceState, ProgramError> {
    let (id, locator, definition, weight) = registry.parent_at(row)?;
    Ok(OracleSourceState {
        month,
        source_id: id,
        bucket_id: id,
        proposer,
        source_type_hash: hashv(&[b"amoeba-cfm-parent-source-v1"]).to_bytes(),
        canonical_locator_hash: locator,
        source_definition_hash: definition,
        bucket_weight_bps: weight,
        ..OracleSourceState::default()
    })
}
fn hashes(registry: &CfmRegistry, month: &Pubkey) -> Result<([u8; 32], [u8; 32]), ProgramError> {
    let count = registry.parent_count() as u16;
    let mut h = initial_oracle_weight_manifest_hash(month, count, count);
    for row in 0..registry.parent_count() {
        let s = metadata(registry, row, *month, Pubkey::default())?;
        h = advance_oracle_weight_manifest_hash(&h, &s.bucket_id, &s, s.bucket_weight_bps);
    }
    Ok((h, canonical_recipe_digest(month, &h)))
}

/// Exact targets are checked before creation. This is also compatible with the
/// compressed wrapper's absent canonical materialization views.
fn create<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    size: usize,
    seed: &[u8],
    scope: &[&[u8]],
) -> Result<u8, ProgramError> {
    let mut seeds = Vec::with_capacity(2 + scope.len());
    seeds.extend_from_slice(&[CURRENT_STATE_NAMESPACE_SEED, seed]);
    seeds.extend_from_slice(scope);
    let (key, bump) = Pubkey::find_program_address(&seeds, program);
    if target.key != &key || target.is_signer {
        return invalid();
    }
    validate_create_only_program_account_target(program, target)?;
    let b = [bump];
    let mut signing = Vec::with_capacity(2 + scope.len());
    signing.push(seed);
    signing.extend_from_slice(scope);
    signing.push(&b);
    create_program_account(payer, target, system, program, size, &signing)?;
    Ok(bump)
}

#[inline(never)]
pub(super) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    operation: u8,
    market: u8,
    row: u8,
    digest: [u8; 32],
) -> ProgramResult {
    let expected = match operation {
        0 => 17,
        1 => 21,
        2 => 11,
        _ => return invalid(),
    };
    if a.len() != expected {
        return Err(VaultError::InvalidAccountList.into());
    }
    // Never let two differently typed writable targets alias, including absent PDAs.
    for (i, info) in a.iter().enumerate() {
        if a[..i].iter().any(|prior| prior.key == info.key) {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let (market_state, month, plan, registry) = context(program, a, market, &digest)?;
    match operation {
        0 => begin(program, a, market, digest, market_state, month, registry),
        1 => append(program, a, market, row, digest, month, plan, registry),
        2 => finish(program, a, market, digest, month, registry),
        _ => invalid(),
    }
}

/// payer, market, month, receipt, plan object, registry object, recipe, active,
/// reward schedule, coverage, product manifest, vault config, council config,
/// three distinct current seats, System.
#[inline(never)]
fn begin(
    program: &Pubkey,
    a: &[AccountInfo],
    market_index: u8,
    digest: [u8; 32],
    market: Market,
    mut month: OracleMonthState,
    registry: CfmRegistry,
) -> ProgramResult {
    let council = oracle_council::current_council(&a[12])?;
    let mut mask = 0u8;
    for seat in &a[13..16] {
        if !seat.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }
        let index = council
            .seats
            .iter()
            .position(|key| key == seat.key)
            .ok_or(VaultError::Unauthorized)?;
        let bit = 1 << index;
        if mask & bit != 0 {
            return Err(VaultError::Unauthorized.into());
        }
        mask |= bit;
    }
    let config = load_current_canonical_vault_config(program, &a[11])?;
    if month.authority != config.oracle_authority {
        return Err(VaultError::Unauthorized.into());
    }
    let coverage = load_valid_oracle_sku_coverage_manifest(program, a[2].key, &a[9])?;
    let product =
        load_valid_oracle_product_sku_manifest(program, &market.instrument.underlying_id, &a[10])?;
    let (terminal_count, terminal_root) = if market_index < 2 {
        (52, RAMX_TERMINAL_ROOT)
    } else {
        (48, NANDX_TERMINAL_ROOT)
    };
    if month.phase != OraclePhase::SourceSubmission
        || month.schedule_version != LAUNCH_SCHEDULE_VERSION
        || month.source_count != 0
        || month.frozen_source_count != 0
        || month.opened_source_count != 0
        || month.opening_resolved_source_count != 0
        || month.recipe_hash != [0; 32]
        || month.weight_manifest_hash != [0; 32]
        || month.active_weight_manifest_hash != [0; 32]
        || month.active_weight_group_count != 0
        || month.accepted_cash_update_count != 0
        || coverage.covered_sku_count != 0
        || coverage.coverage_finalized
        || coverage.coverage_complete_ts != 0
        || coverage.required_sku_count != terminal_count
        || coverage.required_sku_root != terminal_root
        || product.required_sku_count != terminal_count
        || product.required_sku_root != terminal_root
    {
        return invalid();
    }
    validate_system_program(&a[16])?;
    let now = current_unix_timestamp()?;
    let slot = Clock::get()?.slot;
    let count = registry.parent_count() as u16;
    let (manifest_hash, recipe_hash) = hashes(&registry, a[2].key)?;
    let r = Receipt {
        magic: *b"SCBR",
        version: 1,
        bump: create(
            program,
            &a[0],
            &a[3],
            &a[16],
            RECEIPT_LEN,
            SEED,
            &[a[2].key.as_ref()],
        )?,
        market_index,
        cursor: 0,
        finished: false,
        month: *a[2].key,
        plan_hash: digest,
        registry_hash: registry.digest(),
        initial_month_hash: hashv(&[&a[2].try_borrow_data()?]).to_bytes(),
        proposer: *a[0].key,
        council_epoch: council.epoch,
        seats_hash: council.digest,
        approving_seats: mask,
        begun_at: now,
        original_scramble: month.scramble_start_ts,
        original_listing: month.listing_ts,
        finished_at: 0,
    };
    let recipe = OracleRecipeWeightManifest {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[6],
            &a[16],
            OracleRecipeWeightManifest::LEN,
            ORACLE_RECIPE_WEIGHT_MANIFEST_PDA_SEED,
            &[a[2].key.as_ref()],
        )?,
        account_discriminator: OracleRecipeWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleRecipeWeightManifest::ACCOUNT_VERSION,
        month: *a[2].key,
        expected_source_count: count,
        expected_bucket_count: count,
        recipe_hash,
        rolling_manifest_hash: initial_oracle_weight_manifest_hash(a[2].key, count, count),
        ..OracleRecipeWeightManifest::default()
    };
    let active = OracleActiveWeightManifest {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[7],
            &a[16],
            OracleActiveWeightManifest::LEN,
            ORACLE_ACTIVE_WEIGHT_MANIFEST_PDA_SEED,
            &[a[2].key.as_ref()],
        )?,
        account_discriminator: OracleActiveWeightManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleActiveWeightManifest::ACCOUNT_VERSION,
        month: *a[2].key,
        expected_source_count: count,
        expected_group_count: count,
        max_open_interest_payout: u64::MAX,
        rolling_manifest_hash: initial_oracle_active_weight_hash(
            a[2].key,
            &recipe_hash,
            &manifest_hash,
            count,
            count,
        ),
        ..OracleActiveWeightManifest::default()
    };
    let schedule = OracleUsdcRewardSchedule {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[8],
            &a[16],
            OracleUsdcRewardSchedule::LEN,
            ORACLE_USDC_REWARD_SCHEDULE_PDA_SEED,
            &[a[2].key.as_ref()],
        )?,
        account_discriminator: OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcRewardSchedule::ACCOUNT_VERSION,
        month: *a[2].key,
        authority: month.authority,
        reward_vault: derive_oracle_usdc_reward_vault_pda(program).0,
        phase: OracleUsdcRewardSchedulePhase::Building,
        bounty_fee_sweep_finalized: true,
        last_updated_slot: slot,
        ..OracleUsdcRewardSchedule::default()
    };
    // Lock placement now; ordinary finalization cannot advance a collecting manifest.
    month.phase = OraclePhase::Opening;
    month.last_updated_slot = slot;
    save(&a[3], &r)?;
    store_state(&a[6], &recipe)?;
    store_state(&a[7], &active)?;
    store_state(&a[8], &schedule)?;
    store_oracle_month_state(&a[2], &month)
}

/// Common 0..8 as begin; source, observations, SKU reward policy, source reward,
/// median, journal, checkpoint, System, locator object, definition object, two links.
#[inline(never)]
fn append(
    program: &Pubkey,
    a: &[AccountInfo],
    market_index: u8,
    row: u8,
    digest: [u8; 32],
    mut month: OracleMonthState,
    plan: SeptemberBootstrapPlan,
    registry: CfmRegistry,
) -> ProgramResult {
    let mut r = receipt(program, &a[3], a[2].key, market_index, &digest)?;
    if r.finished
        || r.cursor != row
        || r.registry_hash != registry.digest()
        || month.phase != OraclePhase::Opening
        || month.source_count != u16::from(row)
        || month.pending_resolution_count != 0
    {
        return invalid();
    }
    let count = registry.parent_count() as u16;
    let mut recipe = load_valid_oracle_recipe_weight_manifest(program, a[2].key, &a[6])?;
    let mut active = load_valid_oracle_active_weight_manifest(program, a[2].key, &a[7])?;
    let mut schedule = oracle_usdc::load_oracle_usdc_reward_schedule(program, a[2].key, &a[8])?;
    if recipe.phase != OracleRecipeWeightPhase::Collecting
        || active.phase != OracleRecipeWeightPhase::Collecting
        || recipe.processed_source_count != u16::from(row)
        || active.processed_source_count != u16::from(row)
        || recipe.expected_source_count != count
        || active.expected_source_count != count
        || schedule.phase != OracleUsdcRewardSchedulePhase::Building
        || schedule.sku_pool_count != u16::from(row)
        || schedule.total_reward_budget != 0
        || schedule.outstanding_prelisting_escrow_count != 0
    {
        return invalid();
    }
    validate_system_program(&a[16])?;
    let now = current_unix_timestamp()?;
    let slot = Clock::get()?.slot;
    let (value, timestamp) = plan.opening(market_index / 2, row)?;
    if timestamp > now {
        return invalid();
    }
    let mut source = metadata(&registry, usize::from(row), *a[2].key, r.proposer)?;
    source.bump = create(
        program,
        &a[0],
        &a[9],
        &a[16],
        OracleSourceState::LEN,
        ORACLE_SOURCE_PDA_SEED,
        &[a[2].key.as_ref(), &source.source_id],
    )?;
    source.is_initialized = true;
    let before = Box::new(source.clone());
    source.status = OracleSourceStatus::Active;
    source.opening_submitted = true;
    source.baseline_state = value;
    source.current_state = value;
    source.opening_evidence_hash = digest;
    let mut observations = Box::new(OracleSourceObservations {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[10],
            &a[16],
            OracleSourceObservations::LEN,
            ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
            &[a[9].key.as_ref()],
        )?,
        account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        month: *a[2].key,
        source: *a[9].key,
        ..OracleSourceObservations::default()
    });
    // This is a document-snapshot commitment, NOT a fabricated archive URL.
    let snapshot = hashv(&[
        b"amoeba-council-document-snapshot-v1",
        &digest,
        &source.source_id,
        &value.to_le_bytes(),
        &timestamp.to_le_bytes(),
    ])
    .to_bytes();
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        value,
        timestamp,
        &digest,
        &snapshot,
    )?;
    let sku = OracleUsdcSkuPool {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[11],
            &a[16],
            OracleUsdcSkuPool::LEN,
            ORACLE_USDC_SKU_POOL_PDA_SEED,
            &[a[8].key.as_ref(), &source.bucket_id],
        )?,
        account_discriminator: OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcSkuPool::ACCOUNT_VERSION,
        month: *a[2].key,
        schedule: *a[8].key,
        bucket_id: source.bucket_id,
        proposer_reward_bps: 5000,
        listing_bond: 100,
        support_bond: 100,
        opening_bond: 100,
        update_min_bond: 100,
        challenge_min_bond: 100,
        challenge_max_bond: 100,
        challenge_bond_bps: 5000,
        registered_source_count: 1,
        last_updated_slot: slot,
        ..OracleUsdcSkuPool::default()
    };
    let reward = OracleUsdcSourceReward {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[12],
            &a[16],
            OracleUsdcSourceReward::LEN,
            ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
            &[a[8].key.as_ref(), a[9].key.as_ref()],
        )?,
        account_discriminator: OracleUsdcSourceReward::ACCOUNT_DISCRIMINATOR,
        account_version: OracleUsdcSourceReward::ACCOUNT_VERSION,
        month: *a[2].key,
        schedule: *a[8].key,
        sku_pool: *a[11].key,
        source: *a[9].key,
        source_id: source.source_id,
        proposer: r.proposer,
        registered: true,
        terminal_status: OracleSourceStatus::Active,
        last_updated_slot: slot,
        ..OracleUsdcSourceReward::default()
    };
    let median = OracleBucketMedianState {
        is_initialized: true,
        bump: create(
            program,
            &a[0],
            &a[13],
            &a[16],
            OracleBucketMedianState::LEN,
            ORACLE_BUCKET_MEDIAN_PDA_SEED,
            &[a[2].key.as_ref(), &source.bucket_id],
        )?,
        account_discriminator: OracleBucketMedianState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleBucketMedianState::ACCOUNT_VERSION,
        month: *a[2].key,
        bucket_id: source.bucket_id,
        group_index: u16::from(row),
        bucket_weight_start_bps: active.processed_bucket_weight_bps,
        bucket_weight_bps: source.bucket_weight_bps,
        frozen_source_count: 1,
        active_source_count: 1,
        eligible_source_count: 1,
        last_recomputed_ts: now,
        ..OracleBucketMedianState::default()
    };
    oracle_carry::record_fresh_accept(
        program,
        &a[0],
        a[9].key,
        &before,
        &source,
        &observations,
        oracle_carry::AcceptedEvent {
            event: *a[3].key,
            value,
            observed_at: timestamp,
            evidence_hash: digest,
            archive_hash: snapshot,
            contributor: r.proposer,
        },
        &a[14..17],
    )?;
    store_state(&a[9], &source)?;
    store_state(&a[10], observations.as_ref())?;
    store_state(&a[11], &sku)?;
    store_state(&a[12], &reward)?;
    store_state(&a[13], &median)?;
    oracle_evidence::backfill_source(
        program,
        &[
            a[0].clone(),
            a[2].clone(),
            a[9].clone(),
            a[17].clone(),
            a[19].clone(),
            a[18].clone(),
            a[20].clone(),
            a[16].clone(),
        ],
    )?;
    recipe.rolling_manifest_hash = advance_oracle_weight_manifest_hash(
        &recipe.rolling_manifest_hash,
        &source.bucket_id,
        &source,
        source.bucket_weight_bps,
    );
    recipe.processed_source_count += 1;
    recipe.processed_bucket_count += 1;
    recipe.declared_weight_total_bps = recipe
        .declared_weight_total_bps
        .checked_add(source.bucket_weight_bps)
        .ok_or(VaultError::ArithmeticOverflow)?;
    recipe.current_bucket_id = source.bucket_id;
    active.rolling_manifest_hash =
        advance_oracle_active_manifest_hash(&active.rolling_manifest_hash, &source);
    active.processed_source_count += 1;
    active.processed_group_count += 1;
    active.processed_bucket_weight_bps = recipe.declared_weight_total_bps;
    active.current_group_id = source.bucket_id;
    schedule.sku_pool_count += 1;
    schedule.registered_source_count += 1;
    schedule.last_updated_slot = slot;
    r.cursor += 1;
    month.source_count += 1;
    month.opened_source_count += 1;
    month.opening_resolved_source_count += 1;
    month.active_weight_group_count += 1;
    month.last_updated_slot = slot;
    if u16::from(r.cursor) == count {
        let (manifest_hash, recipe_hash) = hashes(&registry, a[2].key)?;
        if recipe.rolling_manifest_hash != manifest_hash
            || recipe.recipe_hash != recipe_hash
            || recipe.declared_weight_total_bps != 10_000
        {
            return invalid();
        }
        recipe.phase = OracleRecipeWeightPhase::Finalized;
        // Stay Collecting until finish: normal permissionless finalizers cannot
        // race the bootstrap's membership completion or change its activation time.
        month.frozen_source_count = count;
        month.weight_scheme_version = 1;
        month.effective_weight_total_bps = 10_000;
        month.weight_manifest_hash = manifest_hash;
        month.recipe_hash = recipe_hash;
    }
    save(&a[3], &r)?;
    store_state(&a[6], &recipe)?;
    store_state(&a[7], &active)?;
    store_state(&a[8], &schedule)?;
    store_oracle_month_state(&a[2], &month)
}

/// Common 0..8; current empty terminal coverage; completed normal recipe index.
#[inline(never)]
fn finish(
    program: &Pubkey,
    a: &[AccountInfo],
    market_index: u8,
    digest: [u8; 32],
    mut month: OracleMonthState,
    registry: CfmRegistry,
) -> ProgramResult {
    let mut r = receipt(program, &a[3], a[2].key, market_index, &digest)?;
    let count = registry.parent_count() as u16;
    if r.finished
        || u16::from(r.cursor) != count
        || r.registry_hash != registry.digest()
        || month.phase != OraclePhase::Opening
        || month.source_count != count
        || month.frozen_source_count != count
        || month.opened_source_count != count
        || month.opening_resolved_source_count != count
        || month.active_weight_group_count != count
    {
        return invalid();
    }
    let recipe = load_valid_oracle_recipe_weight_manifest(program, a[2].key, &a[6])?;
    let mut active = load_valid_oracle_active_weight_manifest(program, a[2].key, &a[7])?;
    let mut schedule = oracle_usdc::load_oracle_usdc_reward_schedule(program, a[2].key, &a[8])?;
    let mut coverage = load_valid_oracle_sku_coverage_manifest(program, a[2].key, &a[9])?;
    let index = oracle_membership::load_recipe_index(program, a[2].key, &month, &a[10])?;
    if !index.complete
        || index.expected_bucket_count != count
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || active.phase != OracleRecipeWeightPhase::Collecting
        || active.processed_source_count != count
        || active.processed_group_count != count
        || active.processed_bucket_weight_bps != 10_000
        || active.max_open_interest_payout != u64::MAX
        || active.rolling_manifest_hash == [0; 32]
        || schedule.sku_pool_count != count
        || schedule.registered_source_count != u32::from(count)
        || schedule.total_reward_budget != 0
        || schedule.remaining_reward_budget != 0
        || schedule.outstanding_prelisting_escrow_count != 0
        || coverage.coverage_finalized
        || coverage.covered_sku_count != 0
    {
        return invalid();
    }
    let now = current_unix_timestamp()?;
    let slot = Clock::get()?.slot;
    let game = now.checked_add(1).ok_or(VaultError::ArithmeticOverflow)?;
    if game >= EXPIRY {
        return invalid();
    }
    // The original terminal root remains immutable in the governed product
    // manifest and pinned alias registry. This month uses the 13/22 economic rows.
    coverage.required_sku_root = registry.economic_root();
    coverage.required_sku_count = count;
    coverage.covered_sku_count = count;
    coverage.coverage_finalized = true;
    coverage.planned_scramble_start_ts = r.begun_at;
    coverage.planned_listing_ts = r.begun_at;
    coverage.coverage_complete_ts = now;
    coverage.last_updated_slot = slot;
    active.phase = OracleRecipeWeightPhase::Finalized;
    schedule.phase = OracleUsdcRewardSchedulePhase::Funded;
    schedule.last_updated_slot = slot;
    month.phase = OraclePhase::Game;
    month.schedule_version = COUNCIL_BOOTSTRAP_SCHEDULE_VERSION;
    month.scramble_start_ts = game;
    month.listing_ts = game;
    month.active_weight_scheme_version = OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION;
    month.active_weight_manifest_hash = active.rolling_manifest_hash;
    month.last_updated_slot = slot;
    ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
    rulebook_schedule_boundaries(&month)?;
    r.finished = true;
    r.finished_at = now;
    save(&a[3], &r)?;
    store_state(&a[7], &active)?;
    store_state(&a[8], &schedule)?;
    store_state(&a[9], &coverage)?;
    store_oracle_month_state(&a[2], &month)
}
