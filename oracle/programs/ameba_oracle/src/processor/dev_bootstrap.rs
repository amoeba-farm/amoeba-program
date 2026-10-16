//! Localnet dev bootstrap (`localnet-dev-authority` only). The dev chain needs Live periods
//! for expiries whose formation calendar has already passed, so these oracle-authority
//! instructions write the state formation would leave at the start of Game directly:
//!
//! - a month in Game with its base value, zero delta and a finalized coverage manifest, and the
//!   period moved Forming -> Live through the same writer tag 80 uses;
//! - Active sources with their first observation, checkpoint and journal, and the locator link of
//!   a sealed evidence object written through the ordinary evidence selector;
//! - at or after expiry, with no grace, the month's settlement: the value from its stored base and
//!   delta, a settlement record, and the period moved Live -> Settled through the same writer the
//!   ordinary settlement uses.
//!
//! Every instruction logs the dev marker. Months carry a dev recipe marker, so neither the source
//! bootstrap nor the dev settlement ever applies to a month formed by the ordinary instructions.

use super::*;
use crate::constants::SETTLEMENT_V2_PDA_SEED;
use crate::instruction::{
    find_dev_council_config_address, DevBootstrapMonthParams, DevBootstrapSourceParams,
    DevCompleteFormationParams, DevCouncilConfigParams, DEV_BOOTSTRAP_MONTH_TAG,
    DEV_BOOTSTRAP_SOURCE_TAG, DEV_COMPLETE_FORMATION_TAG, DEV_COUNCIL_CONFIG_TAG,
    DEV_SETTLEMENT_DIGEST_DOMAIN, DEV_SETTLE_MONTH_TAG,
};
use crate::state::{derive_settlement_v2_pda, SettlementRecordV2};

pub(super) const DEV_AUTHORITY_LOG_MARKER: &str = "AMEBA_ORACLE_LOCALNET_DEV_AUTHORITY";
const DEV_MONTH_MARKER_DOMAIN: &[u8] = b"ameba-oracle-localnet-dev-month-v1";
const DEV_OPENING_MARKER_DOMAIN: &[u8] = b"ameba-oracle-localnet-dev-opening-v1";

/// The recipe, weight-manifest and active-weight hashes of a bootstrapped month.
pub(super) fn dev_month_marker(month: &Pubkey) -> [u8; 32] {
    hashv(&[DEV_MONTH_MARKER_DOMAIN, month.as_ref()]).to_bytes()
}

fn dev_opening_marker(source: &Pubkey) -> [u8; 32] {
    hashv(&[DEV_OPENING_MARKER_DOMAIN, source.as_ref()]).to_bytes()
}

/// A source created by tag 241: its opening evidence is the dev opening marker of its address,
/// which no ordinary opening produces. Such a source has no candidate support.
pub(super) fn is_dev_bootstrapped_source(key: &Pubkey, source: &OracleSourceState) -> bool {
    source.opening_evidence_hash == dev_opening_marker(key)
}

/// The period's settlement digest for a dev settlement (see [`DEV_SETTLEMENT_DIGEST_DOMAIN`]).
pub(super) fn dev_settlement_digest(
    period: &Pubkey,
    month: &Pubkey,
    expiry_ts: u64,
    base_oracle_atomic: u64,
    index_delta_bps: i64,
    value: u64,
    settled_ts: u64,
) -> [u8; 32] {
    hashv(&[
        DEV_SETTLEMENT_DIGEST_DOMAIN,
        period.as_ref(),
        month.as_ref(),
        &expiry_ts.to_le_bytes(),
        &base_oracle_atomic.to_le_bytes(),
        &index_delta_bps.to_le_bytes(),
        &value.to_le_bytes(),
        &settled_ts.to_le_bytes(),
    ])
    .to_bytes()
}

#[inline(never)]
pub(super) fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: u8,
    payload: &[u8],
) -> ProgramResult {
    solana_program::msg!(DEV_AUTHORITY_LOG_MARKER);
    match tag {
        DEV_BOOTSTRAP_MONTH_TAG => {
            let params: DevBootstrapMonthParams = decode_instruction_payload(payload)?;
            process_dev_bootstrap_month(program_id, accounts, params)
        }
        DEV_BOOTSTRAP_SOURCE_TAG => {
            let params: DevBootstrapSourceParams = decode_instruction_payload(payload)?;
            process_dev_bootstrap_source(program_id, accounts, params)
        }
        DEV_SETTLE_MONTH_TAG => {
            if !payload.is_empty() {
                return Err(OracleError::InvalidInstructionData.into());
            }
            process_dev_settle_month(program_id, accounts)
        }
        DEV_COUNCIL_CONFIG_TAG => {
            let params: DevCouncilConfigParams = decode_instruction_payload(payload)?;
            process_dev_council_config(program_id, accounts, params)
        }
        DEV_COMPLETE_FORMATION_TAG => {
            let params: DevCompleteFormationParams = decode_instruction_payload(payload)?;
            super::dev_formation::process(program_id, accounts, params)
        }
        _ => Err(OracleError::InvalidInstructionData.into()),
    }
}

#[inline(never)]
fn process_dev_bootstrap_month(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DevBootstrapMonthParams,
) -> ProgramResult {
    let [authority_info, payer_info, period_info, month_info, coverage_info, config_info, economics_info, system_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    if params.settlement_base_oracle_atomic == 0
        || crate::bytes32_is_zero(&params.required_sku_root)
        || params.required_sku_count == 0
        || params.required_sku_count > MAX_ORACLE_REQUIRED_SKUS
    {
        return Err(OracleError::InvalidOracleSkuCoverageManifest.into());
    }
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    if anchor.period.status != PeriodStatus::Forming || anchor.period.effective_listing_ts != 0 {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let economics = load_canonical_oracle_economics_config(program_id, economics_info)?;
    let (slot, now) = current_slot_and_unix_timestamp()?;
    let listing_ts = anchor.period.planned_listing_ts.max(now);
    if listing_ts >= anchor.expiry_ts {
        return Err(OracleError::OracleTimingWindowClosed.into());
    }
    let scramble_start_ts = listing_ts
        .checked_sub(ORACLE_PRE_LISTING_WINDOW_SECONDS)
        .ok_or(OracleError::ArithmeticOverflow)?;

    let (expected_month, month_bump) =
        derive_oracle_month_pda(program_id, period_info.key, anchor.expiry_ts);
    if *month_info.key != expected_month {
        return Err(OracleError::InvalidOracleMonthAccount.into());
    }
    if month_info.owner == program_id {
        return Err(OracleError::AlreadyInitialized.into());
    }
    let (expected_coverage, coverage_bump) =
        derive_oracle_sku_coverage_manifest_pda(program_id, month_info.key);
    if *coverage_info.key != expected_coverage {
        return Err(OracleError::InvalidOracleSkuCoverageManifest.into());
    }
    if coverage_info.owner == program_id {
        return Err(OracleError::AlreadyInitialized.into());
    }
    validate_create_only_program_account_target(program_id, month_info)?;
    validate_create_only_program_account_target(program_id, coverage_info)?;

    let marker = dev_month_marker(month_info.key);
    let month = OracleMonthState {
        is_initialized: true,
        bump: month_bump,
        account_discriminator: OracleMonthState::ACCOUNT_DISCRIMINATOR,
        account_version: OracleMonthState::ACCOUNT_VERSION,
        period: *period_info.key,
        authority: *authority_info.key,
        scramble_start_ts,
        listing_ts,
        phase: OraclePhase::Game,
        recipe_hash: marker,
        weight_scheme_version: 1,
        weight_manifest_hash: marker,
        active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
        active_weight_manifest_hash: marker,
        index_delta_bps: 0,
        last_updated_slot: slot,
        settlement_base_oracle_atomic: params.settlement_base_oracle_atomic,
        economics: economics.economics,
        candidate_count_tracking_version: OracleMonthState::CANDIDATE_COUNT_TRACKING_VERSION,
        active_weight_initialization_version:
            OracleMonthState::ACTIVE_WEIGHT_INITIALIZATION_VERSION,
        schedule_version: OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION,
        work_reward_currency_version: OracleMonthState::WORK_REWARD_CURRENCY_USDC_V1,
        ..OracleMonthState::default()
    };
    rulebook_schedule_boundaries(&month)?;
    let coverage = OracleSkuCoverageManifest {
        is_initialized: true,
        bump: coverage_bump,
        account_discriminator: OracleSkuCoverageManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSkuCoverageManifest::ACCOUNT_VERSION,
        month: *month_info.key,
        required_sku_root: params.required_sku_root,
        required_sku_count: params.required_sku_count,
        covered_sku_count: params.required_sku_count,
        planned_scramble_start_ts: scramble_start_ts,
        planned_listing_ts: listing_ts,
        coverage_finalized: true,
        coverage_complete_ts: scramble_start_ts,
        last_updated_slot: slot,
    };

    let expiry_seed = anchor.expiry_ts.to_le_bytes();
    create_program_account(
        payer_info,
        month_info,
        system_program_info,
        program_id,
        OracleMonthState::LEN,
        &[
            ORACLE_MONTH_PDA_SEED,
            period_info.key.as_ref(),
            &expiry_seed,
            &[month_bump],
        ],
    )?;
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
    write_period_live(program_id, period_info, &anchor, listing_ts)?;
    store_state(coverage_info, &coverage)?;
    store_oracle_month_state(month_info, &month)
}

#[inline(never)]
fn process_dev_bootstrap_source(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DevBootstrapSourceParams,
) -> ProgramResult {
    let [authority_info, payer_info, period_info, month_info, config_info, coverage_info, source_info, observations_info, locator_object_info, locator_link_info, _journal_info, _checkpoint_info, system_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    if [
        &params.source_id,
        &params.bucket_id,
        &params.source_type_hash,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
    ]
    .into_iter()
    .any(crate::bytes32_is_zero)
        || params.bucket_weight_bps == 0
        || params.bucket_weight_bps > 10_000
        || params.opening_state == 0
    {
        return Err(OracleError::InvalidOracleSourceAccount.into());
    }
    let (anchor, mut month) =
        load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    if month.phase != OraclePhase::Game || month.recipe_hash != dev_month_marker(month_info.key) {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    let (slot, now) = current_slot_and_unix_timestamp()?;
    if now >= anchor.expiry_ts {
        return Err(OracleError::OracleTimingWindowClosed.into());
    }
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    verify_oracle_sku_membership(
        &coverage,
        &params.bucket_id,
        params.sku_index,
        &params.sku_proof,
    )?;
    let weight_total = month
        .effective_weight_total_bps
        .checked_add(params.bucket_weight_bps)
        .ok_or(OracleError::ArithmeticOverflow)?;
    if weight_total > 10_000 {
        return Err(OracleError::InvalidOracleWeightManifest.into());
    }

    let (expected_source, source_bump) =
        derive_oracle_source_pda(program_id, month_info.key, &params.source_id);
    let (expected_observations, observations_bump) =
        derive_oracle_source_observations_pda(program_id, source_info.key);
    if *source_info.key != expected_source || *observations_info.key != expected_observations {
        return Err(OracleError::InvalidOracleSourceAccount.into());
    }
    if source_info.owner == program_id || observations_info.owner == program_id {
        return Err(OracleError::AlreadyInitialized.into());
    }
    validate_create_only_program_account_target(program_id, source_info)?;
    validate_create_only_program_account_target(program_id, observations_info)?;
    create_program_account(
        payer_info,
        source_info,
        system_program_info,
        program_id,
        OracleSourceState::LEN,
        &[
            ORACLE_SOURCE_PDA_SEED,
            month_info.key.as_ref(),
            &params.source_id,
            &[source_bump],
        ],
    )?;
    create_program_account(
        payer_info,
        observations_info,
        system_program_info,
        program_id,
        OracleSourceObservations::LEN,
        &[
            ORACLE_SOURCE_OBSERVATIONS_PDA_SEED,
            source_info.key.as_ref(),
            &[observations_bump],
        ],
    )?;

    let opening_marker = dev_opening_marker(source_info.key);
    let before = OracleSourceState {
        is_initialized: true,
        bump: source_bump,
        month: *month_info.key,
        source_id: params.source_id,
        bucket_id: params.bucket_id,
        source_type_hash: params.source_type_hash,
        canonical_locator_hash: params.canonical_locator_hash,
        source_definition_hash: params.source_definition_hash,
        proposer: *authority_info.key,
        bucket_weight_bps: params.bucket_weight_bps,
        status: OracleSourceStatus::Active,
        ..OracleSourceState::default()
    };
    let mut source = before;
    let mut observations = Box::new(OracleSourceObservations {
        is_initialized: true,
        bump: observations_bump,
        account_discriminator: OracleSourceObservations::ACCOUNT_DISCRIMINATOR,
        account_version: OracleSourceObservations::ACCOUNT_VERSION,
        month: *month_info.key,
        source: *source_info.key,
        ..OracleSourceObservations::default()
    });
    append_oracle_source_observation(
        &mut source,
        &mut observations,
        params.opening_state,
        month.listing_ts,
        &opening_marker,
        &params.canonical_locator_hash,
    )?;
    source.baseline_state = params.opening_state;
    source.current_state = params.opening_state;
    source.opening_submitted = true;
    source.opening_evidence_hash = opening_marker;

    super::oracle_evidence::publish(
        program_id,
        payer_info,
        system_program_info,
        locator_object_info,
        locator_link_info,
        month_info.key,
        source_info.key,
        source_info.key,
        source_info.key,
        0,
        0,
        0,
        0,
        params.canonical_locator_hash,
        params.canonical_locator_hash,
        params.source_id,
    )?;
    carry::record_fresh_accept(
        program_id,
        payer_info,
        source_info.key,
        &before,
        &source,
        &observations,
        carry::AcceptedEvent {
            event: *source_info.key,
            value: params.opening_state,
            observed_at: month.listing_ts,
            evidence_hash: opening_marker,
            archive_hash: params.canonical_locator_hash,
            contributor: *authority_info.key,
        },
        &accounts[10..13],
    )?;

    month.source_count = month
        .source_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    month.frozen_source_count = month.source_count;
    month.opened_source_count = month.source_count;
    month.opening_resolved_source_count = month.source_count;
    month.active_weight_group_count = month
        .active_weight_group_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    month.effective_weight_total_bps = weight_total;
    month.last_updated_slot = slot;
    store_state(source_info, &source)?;
    store_state(observations_info, observations.as_ref())?;
    store_oracle_month_state(month_info, &month)
}

/// Tag 242. The record carries signer-set version 0: no signer set attested this settlement.
#[inline(never)]
fn process_dev_settle_month(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [authority_info, payer_info, period_info, month_info, record_info, config_info, system_program_info] =
        accounts
    else {
        return Err(OracleError::InvalidAccountList.into());
    };
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    if !period_info.is_writable || !month_info.is_writable || !record_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    let (anchor, mut month) =
        load_valid_period_and_oracle_month(program_id, period_info, month_info)?;
    if month.recipe_hash != dev_month_marker(month_info.key) {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    let (expected_record, record_bump) =
        derive_settlement_v2_pda(program_id, period_info.key, month_info.key);
    if *record_info.key != expected_record {
        return Err(OracleError::InvalidSettlementAccount.into());
    }

    // A replay finds the period, month and record already settled together and changes nothing.
    if record_info.owner == program_id {
        let record: SettlementRecordV2 = load_exact_zero_padded_state(
            record_info,
            program_id,
            SettlementRecordV2::LEN,
            OracleError::InvalidSettlementAccount,
        )?;
        let period = &anchor.period;
        if record.is_initialized
            && record.bump == record_bump
            && record.account_discriminator == SettlementRecordV2::ACCOUNT_DISCRIMINATOR
            && record.account_version == SettlementRecordV2::ACCOUNT_VERSION
            && record.period == *period_info.key
            && record.oracle_month == *month_info.key
            && record.settlement_ts == anchor.expiry_ts
            && record.submitted_by == *authority_info.key
            && record.signer_set_version == 0
            && month.phase == OraclePhase::Settled
            && month.settlement_record == Some(*record_info.key)
            && period.status == PeriodStatus::Settled
            && period.settlement_value == record.settlement_price_atomic
            && period.settlement_digest == record.signed_leaf_commitment
        {
            return Ok(());
        }
        return Err(OracleError::SettlementRecordImmutable.into());
    }

    if month.phase != OraclePhase::Game
        || month.settlement_record.is_some()
        || month.settlement_status == OracleSettlementStatus::Final
        || month.finalized_at_ts != 0
    {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    ensure_oracle_opening_resolution_complete(&month)?;
    if month.pending_resolution_count != 0 {
        return Err(OracleError::InvalidOracleState.into());
    }
    let (slot, now) = current_slot_and_unix_timestamp()?;
    if now < anchor.expiry_ts {
        return Err(OracleError::PeriodNotExpired.into());
    }
    let value =
        index_delta_settlement_value(month.settlement_base_oracle_atomic, month.index_delta_bps)?;
    let digest = dev_settlement_digest(
        period_info.key,
        month_info.key,
        anchor.expiry_ts,
        month.settlement_base_oracle_atomic,
        month.index_delta_bps,
        value,
        now,
    );

    validate_create_only_program_account_target(program_id, record_info)?;
    create_program_account(
        payer_info,
        record_info,
        system_program_info,
        program_id,
        SettlementRecordV2::LEN,
        &[
            SETTLEMENT_V2_PDA_SEED,
            period_info.key.as_ref(),
            month_info.key.as_ref(),
            &[record_bump],
        ],
    )?;
    let record = SettlementRecordV2 {
        is_initialized: true,
        bump: record_bump,
        account_discriminator: SettlementRecordV2::ACCOUNT_DISCRIMINATOR,
        account_version: SettlementRecordV2::ACCOUNT_VERSION,
        period: *period_info.key,
        oracle_month: *month_info.key,
        settlement_ts: anchor.expiry_ts,
        settlement_price_atomic: value,
        signed_leaf_commitment: digest,
        submitted_by: *authority_info.key,
        signer_set_version: 0,
    };
    write_period_settled(program_id, period_info, &anchor, value, now, digest)?;
    store_state(record_info, &record)?;
    month.c_raw_bps = 10_000;
    month.g_camo_bps = 0;
    month.g_thin_bps = 0;
    month.c_settle_bps = 10_000;
    month.settlement_status = OracleSettlementStatus::Final;
    month.finalized_at_ts = now;
    month.phase = OraclePhase::Settled;
    month.settlement_record = Some(*record_info.key);
    month.last_updated_slot = slot;
    store_oracle_month_state(month_info, &month)
}

/// Tag 243. The admin creates the dev council Config at epoch 1, or rotates its seats to the next
/// epoch. The layout is the governance controller's 384-byte Config, with this program as its
/// controller, so the council decoder applies unchanged.
#[inline(never)]
fn process_dev_council_config(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: DevCouncilConfigParams,
) -> ProgramResult {
    const LEN: usize = 384;
    let [admin_info, payer_info, config_info, council_info, system_program_info] = accounts else {
        return Err(OracleError::InvalidAccountList.into());
    };
    if !admin_info.is_signer || !council_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    validate_current_account_creation_payer(payer_info)?;
    validate_system_program(system_program_info)?;
    let config = load_current_canonical_vault_config(program_id, config_info)?;
    if *admin_info.key != config.admin {
        return Err(OracleError::Unauthorized.into());
    }
    let (expected, bump) = find_dev_council_config_address(program_id);
    if *council_info.key != expected {
        return Err(OracleError::InvalidPda.into());
    }
    let epoch = if council_info.owner == program_id {
        if council_info.data_len() != LEN {
            return Err(OracleError::InvalidGovernanceGateData.into());
        }
        let current = super::council::decode_council(
            program_id,
            council_info.key,
            council_info.owner,
            &council_info.try_borrow_data()?,
        )?;
        current
            .epoch
            .checked_add(1)
            .ok_or(OracleError::ArithmeticOverflow)?
    } else {
        validate_create_only_program_account_target(program_id, council_info)?;
        invoke_create_or_allocate_account(
            payer_info,
            council_info,
            system_program_info,
            program_id,
            LEN,
            &[b"ameba-governance-v3", b"council", &[bump]],
        )?;
        1
    };
    let programdata = Pubkey::find_program_address(
        &[program_id.as_ref()],
        &solana_sdk_ids::bpf_loader_upgradeable::ID,
    )
    .0;
    let authority =
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"authority"], program_id).0;
    let mut data = [0u8; LEN];
    data[..8].copy_from_slice(b"AG3CFG01");
    data[8] = 1;
    data[9] = bump;
    data[10] = 1;
    data[11..43].copy_from_slice(program_id.as_ref());
    data[43..75].copy_from_slice(programdata.as_ref());
    data[75..107].copy_from_slice(authority.as_ref());
    data[107..139].copy_from_slice(admin_info.key.as_ref());
    for (index, seat) in params.seats.iter().enumerate() {
        data[139 + 32 * index..171 + 32 * index].copy_from_slice(seat.as_ref());
    }
    for (offset, value) in [
        (299, epoch),
        (307, 1),
        (315, 1_512_000),
        (323, 4_500),
        (331, 1_521_001),
        (339, 1),
    ] {
        data[offset..offset + 8].copy_from_slice(&u64::to_le_bytes(value));
    }
    // The decoder validates the seats and every field before anything is written.
    super::council::decode_council(program_id, council_info.key, program_id, &data)?;
    council_info.try_borrow_mut_data()?.copy_from_slice(&data);
    Ok(())
}
