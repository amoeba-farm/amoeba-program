use super::*;
use crate::instruction::ProposeOracleSourceParams;
use crate::processor::oracle_membership::load_complete_bucket_source_index;

/// Accounts: payer/authority, config, target market, target month, registry,
/// new period, System; for a successor append parent market, parent month and
/// canonical absent target reward schedule. Opt-in must precede period funding.
/// A root only enables future checkpointing; it does not reinterpret its opening.
pub(super) fn register_period(program: &Pubkey, a: &[AccountInfo], root: bool) -> ProgramResult {
    if a.len() != if root { 7 } else { 10 } {
        return invalid();
    }
    validate_current_oracle_authority_allow_paused(program, &a[0], &a[1])?;
    let (market, month) = load_valid_market_and_oracle_month(program, &a[2], &a[3])?;
    let underlying = market.instrument.underlying_id;
    let expiry = market.instrument.expiry_ts;
    let registry_pda = address(program, REGISTRY_SEED, &underlying);
    let period_pda = address(program, PERIOD_SEED, a[3].key.as_ref());
    let timestamp = now()?;
    let (predecessor, predecessor_recipe, expected_imports) = if root {
        if !matches!(
            month.phase,
            OraclePhase::Game | OraclePhase::Settled | OraclePhase::Closed
        ) || month.recipe_hash == [0; 32]
            || month.frozen_source_count == 0
        {
            return invalid();
        }
        validate_canonical_system_zero_pda_proof(&registry_pda.0, &a[4])?;
        (Pubkey::default(), [0; 32], 0)
    } else {
        let registry: Registry = load(program, &a[4], registry_pda)?;
        let (parent_market, parent) = load_valid_market_and_oracle_month(program, &a[7], &a[8])?;
        let schedule = crate::state::derive_oracle_usdc_reward_schedule_pda(program, a[3].key).0;
        validate_canonical_system_zero_pda_proof(&schedule, &a[9])?;
        // Registration precedes any participant activity. Late coverage may subsequently
        // move the published effective opening boundary under the unchanged schedule rules.
        if registry.underlying != underlying
            || registry.latest_period != *a[8].key
            || parent_market.instrument.underlying_id != underlying
            || parent_market.instrument.expiry_ts != registry.latest_expiry
            || (expiry, a[3].key.to_bytes())
                <= (registry.latest_expiry, registry.latest_period.to_bytes())
            || !matches!(
                parent.phase,
                OraclePhase::Game | OraclePhase::Settled | OraclePhase::Closed
            )
            || parent.recipe_hash == [0; 32]
            || parent.frozen_source_count == 0
            || !market.paused
            || market.total_position_collateral_locked != 0
            || market.mint_accounting.total_issued != 0
            || month.phase != OraclePhase::SourceSubmission
            || timestamp >= month.scramble_start_ts
            || month.source_count != 0
            || month.frozen_source_count != 0
            || month.pending_resolution_count != 0
            || month.weight_scheme_version != 0
        {
            return invalid();
        }
        (*a[8].key, parent.recipe_hash, parent.frozen_source_count)
    };
    let period = Period {
        header: header::<Period>(period_pda.1),
        month: *a[3].key,
        underlying,
        predecessor,
        predecessor_recipe,
        expiry,
        registered_at: timestamp,
        expected_imports,
        next_import: 0,
    };
    create(
        program,
        &a[0],
        &a[5],
        &a[6],
        &[PERIOD_SEED, a[3].key.as_ref(), &[period_pda.1]],
        &period,
    )?;
    let registry = Registry {
        header: header::<Registry>(registry_pda.1),
        underlying,
        latest_period: *a[3].key,
        latest_expiry: expiry,
    };
    if root {
        create(
            program,
            &a[0],
            &a[4],
            &a[6],
            &[REGISTRY_SEED, &underlying, &[registry_pda.1]],
            &registry,
        )
    } else {
        save(program, &a[4], &registry)
    }
}

fn next_parent(
    program: &Pubkey,
    target_market: &Market,
    period: &Period,
    parent_market_info: &AccountInfo,
    parent_month_info: &AccountInfo,
    parent_source_info: &AccountInfo,
    index_info: &AccountInfo,
    bucket_info: &AccountInfo,
) -> Result<OracleSourceState, ProgramError> {
    if period.predecessor != *parent_month_info.key || period.next_import >= period.expected_imports
    {
        return invalid();
    }
    let (market, month) =
        load_valid_market_and_oracle_month(program, parent_market_info, parent_month_info)?;
    let source = load_valid_oracle_source(program, parent_month_info.key, parent_source_info)?;
    if market.instrument.underlying_id != target_market.instrument.underlying_id
        || market.instrument.underlying_id != period.underlying
        || month.recipe_hash != period.predecessor_recipe
        || month.frozen_source_count != period.expected_imports
        || !matches!(
            month.phase,
            OraclePhase::Game | OraclePhase::Settled | OraclePhase::Closed
        )
    {
        return invalid();
    }
    let bucket = load_complete_bucket_source_index(
        program,
        parent_month_info.key,
        &month,
        &source.bucket_id,
        index_info,
        bucket_info,
    )?;
    let offset = period
        .next_import
        .checked_sub(bucket.first_source_index)
        .ok_or(VaultError::InvalidOracleWeightOrder)?;
    if offset >= bucket.source_count || bucket.source_ids[usize::from(offset)] != source.source_id {
        return invalid();
    }
    Ok(source)
}

pub(super) fn definition_hash(underlying: &[u8; 32], source: &OracleSourceState) -> [u8; 32] {
    // source_definition_hash commits the observed field, currency, region, tier and other
    // economic terms. Descriptor and state must both be authenticated by the wrapper.
    hashv(&[
        b"oracle-carry-definition-v1",
        underlying,
        &source.source_id,
        &source.bucket_id,
        &source.source_type_hash,
        &source.canonical_locator_hash,
        &source.source_definition_hash,
    ])
    .to_bytes()
}

/// First eleven accounts are precisely ProposeOracleSourceV3's target-period accounts.
/// Append parent source, parent market, parent month, complete recipe index, bucket index,
/// target period and create-only target carry companion. Six compressed leaves in total.
pub(super) fn import_source(
    program: &Pubkey,
    a: &[AccountInfo],
    sku_index: u16,
    sku_proof: &[[u8; 32]],
) -> ProgramResult {
    if a.len() != 18 {
        return invalid();
    }
    let (market, _) = load_valid_market_and_oracle_month(program, &a[1], &a[2])?;
    let mut period = load_period(program, a[2].key, &a[16])?;
    let parent = next_parent(
        program, &market, &period, &a[12], &a[13], &a[11], &a[14], &a[15],
    )?;
    if parent.status != OracleSourceStatus::Active
        || parent.source_definition_hash == [0; 32]
        || parent.canonical_locator_hash == [0; 32]
        || parent.source_type_hash == [0; 32]
    {
        return invalid();
    }
    let sku = oracle_usdc::load_oracle_usdc_sku_for_month(program, a[2].key, &a[4])?;
    // Reuse ordinary creation to debit a NEW listing bond, require this period's funded
    // schedule/SKU and membership proof, and create empty observations/reward records.
    oracle_usdc::process_propose_oracle_source(
        program,
        &a[..11],
        ProposeOracleSourceParams {
            source_id: parent.source_id,
            bucket_id: parent.bucket_id,
            source_type_hash: parent.source_type_hash,
            canonical_locator_hash: parent.canonical_locator_hash,
            source_definition_hash: parent.source_definition_hash,
            listing_bond: sku.listing_bond,
        },
        (sku_index, sku_proof),
    )?;
    let companion_pda = address(program, SOURCE_SEED, a[5].key.as_ref());
    let carry = CarrySource {
        header: header::<CarrySource>(companion_pda.1),
        month: *a[2].key,
        source: *a[5].key,
        parent_month: *a[13].key,
        parent_source: *a[11].key,
        definition_hash: definition_hash(&period.underlying, &parent),
        cursor: Pubkey::default(),
        selected_checkpoint: Pubkey::default(),
        origin_source: Pubkey::default(),
        origin_checkpoint: Pubkey::default(),
        contributor: Pubkey::default(),
        evidence_hash: [0; 32],
        archive_hash: [0; 32],
        cutoff: 0,
        deadline: 0,
        value: 0,
        observed_at: 0,
        accepted_at: 0,
        remaining: 0,
        selected_sequence: 0,
        status: IMPORTED,
    };
    create(
        program,
        &a[0],
        &a[17],
        &a[9],
        &[SOURCE_SEED, a[5].key.as_ref(), &[companion_pda.1]],
        &carry,
    )?;
    period.next_import = period
        .next_import
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    save(program, &a[16], &period)
}

/// Accounts: target market/month/period, parent market/month/source, recipe index/bucket index, rent payer.
/// An inactive source can be skipped only at the exact next authenticated membership index.
pub(super) fn skip_inactive_source(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 9 {
        return invalid();
    }
    let (market, month) = load_valid_market_and_oracle_month(program, &a[0], &a[1])?;
    let mut period = load_period(program, a[1].key, &a[2])?;
    if month.phase != OraclePhase::SourceSubmission || month.weight_scheme_version != 0 {
        return invalid();
    }
    let source = next_parent(program, &market, &period, &a[3], &a[4], &a[5], &a[6], &a[7])?;
    if source.status != OracleSourceStatus::Inactive {
        return invalid();
    }
    period.next_import = period
        .next_import
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    save(program, &a[2], &period)
}

/// Mandatory public-proposal AND recipe-freeze hook. Canonical absence preserves
/// ordinary periods. Public proposals wait until the import walk completes, so a
/// pre-created source cannot squat an imported identity and strand the cursor.
/// The authenticated importer calls the private proposal core without this guard.
pub(in crate::processor) fn require_import_complete(
    program: &Pubkey,
    month: &Pubkey,
    info: &AccountInfo,
) -> ProgramResult {
    let pda = address(program, PERIOD_SEED, month.as_ref());
    if info.owner == &system_program::id() {
        return validate_canonical_system_zero_pda_proof(&pda.0, info);
    }
    let period = load_period(program, month, info)?;
    if period.next_import != period.expected_imports {
        return invalid();
    }
    Ok(())
}

/// Import is never fresh source discovery. A separate newly accepted opening/update
/// can retain its ordinary work reward; this hook only gates the discovery claim.
pub(in crate::processor) fn require_fresh_discovery(
    program: &Pubkey,
    source: &Pubkey,
    info: &AccountInfo,
) -> ProgramResult {
    validate_canonical_system_zero_pda_proof(
        &address(program, SOURCE_SEED, source.as_ref()).0,
        info,
    )
}
