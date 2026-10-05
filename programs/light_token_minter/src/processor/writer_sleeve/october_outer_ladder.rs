//! Append the eight outer October series without rewriting existing markets.
//! Two bounded amendments keep the complete transaction below 64 accounts.
use super::*;
use crate::instruction::{ExtendOctoberLadderParams, InstallOctoberLadderParams};
use crate::oracle_parent_proxy::september_bootstrap::{EXPIRY, OCTOBER_EXPIRY, PROGRAM};
use crate::state::{OptionKind, WriterDlmmSeriesPolicyV1};

const UNIT: u64 = 1_000_000;
const WIDTH: u64 = 5 * UNIT;
const LEVELS: [u64; 9] = [100, 90, 95, 105, 110, 80, 85, 115, 120];

fn invalid<T>() -> Result<T, ProgramError> {
    Err(VaultError::InvalidWriterLifecycle.into())
}

fn series_id(product: u8, side: usize, ordinal: usize) -> [u8; 32] {
    let prefix: &[u8] = if product == 0 {
        b"RAMX-202610-"
    } else {
        b"NANDX-202610-"
    };
    let suffix: &[u8] = if side == 0 { b"CALL-0" } else { b"PUT-0" };
    let mut id = [0; 32];
    id[..prefix.len()].copy_from_slice(prefix);
    id[prefix.len()..prefix.len() + suffix.len()].copy_from_slice(suffix);
    id[prefix.len() + suffix.len()] = b'1' + ordinal as u8;
    id
}

fn validate_old_series(
    program: &Pubkey,
    product: u8,
    index: usize,
    record: &WriterSeriesRecordV1,
    mint_info: &AccountInfo,
    pool_info: &AccountInfo,
) -> ProgramResult {
    let side = index / 5;
    let ordinal = index % 5;
    let strike = LEVELS[ordinal] * UNIT;
    let expected_id = series_id(product, side, ordinal);
    let expected_market = derive_market_pda(program, &expected_id).0;
    let expected_mint = derive_contract_mint_pda(program, &expected_market).0;
    if !record.active
        || record.series_id != expected_id
        || record.market != expected_market
        || record.contract_mint != expected_mint
        || *mint_info.key != expected_mint
        || record.option_kind
            != if side == 0 {
                OptionKind::CallSpread
            } else {
                OptionKind::PutSpread
            }
        || record.strike_price_atomic != strike
        || record.cap_or_floor_price_atomic
            != if side == 0 {
                strike + WIDTH
            } else {
                strike - WIDTH
            }
        || record.contract_size_atoms != UNIT
        || record.max_payout_per_contract_atoms != WIDTH
    {
        return invalid();
    }
    let mint = validate_mint_account(mint_info, &spl_token_program_id())?;
    if !mint.is_initialized
        || mint.decimals != 6
        || mint.supply != 0
        || mint.mint_authority != COption::Some(expected_market)
        || mint.freeze_authority != COption::None
    {
        return invalid();
    }
    let pool = crate::ameba_dlmm_state::derive_ameba_dlmm_pool_pda(program, &expected_market).0;
    validate_canonical_system_zero_pda_proof(&pool, pool_info)
}

#[inline(never)]
fn new_record(
    program: &Pubkey,
    product: u8,
    side: usize,
    ordinal: usize,
    a: &[AccountInfo],
    sleeve: &Pubkey,
    group_info: &AccountInfo,
    group: &WriterSettlementGroupV1,
) -> Result<WriterSeriesRecordV1, ProgramError> {
    let mut market = load_valid_market(program, &a[0])?;
    let mint = validate_canonical_market_mint(&a[0], &mut market, &a[1], 0)?;
    let strike = LEVELS[ordinal] * UNIT;
    if !market.paused
        || market.market_id != series_id(product, side, ordinal)
        || market.instrument.kind
            != if side == 0 {
                OptionKind::CallSpread
            } else {
                OptionKind::PutSpread
            }
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != OCTOBER_EXPIRY
        || market.collateral_mint != group.settlement_mint
        || market.instrument.contract_size != UNIT
        || market.instrument.strike_price != strike
        || market.instrument.cap_price
            != if side == 0 {
                strike + WIDTH
            } else {
                strike - WIDTH
            }
        || market.instrument.max_payout_per_contract != WIDTH
        || market.total_position_collateral_locked != 0
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
        || mint.supply != 0
    {
        return invalid();
    }
    let pool = crate::ameba_dlmm_state::derive_ameba_dlmm_pool_pda(program, a[0].key).0;
    validate_canonical_system_zero_pda_proof(&pool, &a[2])?;
    Ok(WriterSeriesRecordV1 {
        active: true,
        option_kind: market.instrument.kind,
        custody_status: WriterSeriesCustodyStatus::Absent,
        settlement_status: WriterSeriesSettlementStatus::Open,
        reserved: [1, 1, 0, 0],
        series_id: market.market_id,
        market: *a[0].key,
        contract_mint: *a[1].key,
        retirement_custody: derive_writer_retirement_custody_pda(program, sleeve, a[0].key).0,
        strike_price_atomic: strike,
        cap_or_floor_price_atomic: market.instrument.cap_price,
        contract_size_atoms: UNIT,
        max_payout_per_contract_atoms: WIDTH,
        payoff_digest: writer_payoff_digest(group_info.key, &market, a[0].key, a[1].key),
        ..WriterSeriesRecordV1::EMPTY
    })
}

/// The first 15 accounts match OCL1. Next are ten (mint, absent pool) pairs in
/// the old book order, then eight (market, mint, absent pool) triples for 06..09.
/// Existing records and terms are preserved, ordered CALL01..09, PUT01..09.
#[inline(never)]
pub(in crate::processor) fn process_extend_october_ladder(
    program: &Pubkey,
    a: &[AccountInfo],
    params: ExtendOctoberLadderParams,
) -> ProgramResult {
    if !cfg!(feature = "mainnet-v3")
        || *program != PROGRAM
        || *program != crate::id()
        || a.len() != 59
        || params.product > 1
    {
        return invalid();
    }
    for (index, info) in a.iter().enumerate() {
        let alias = index == 1 && a[0].key == info.key;
        if info.is_signer != matches!(index, 0 | 1 | 11..=13)
            || info.is_writable != (matches!(index, 0 | 3..=6 | 8..=9) || alias)
            || (info.executable && index != 14)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if a[..index]
            .iter()
            .enumerate()
            .any(|(old, other)| other.key == info.key && !(old == 0 && alias))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    if *a[14].key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let clock = Clock::get()?;
    let now =
        u64::try_from(clock.unix_timestamp).map_err(|_| VaultError::InvalidWriterLifecycle)?;
    if !(EXPIRY..OCTOBER_EXPIRY).contains(&now) {
        return invalid();
    }
    let config = load_canonical_vault_config(program, &a[2])?;
    let mut registry = load_writer_policy_registry(program, &a[3], a[2].key)?;
    if config.admin != *a[0].key || registry.policy_authority != *a[1].key {
        return Err(VaultError::Unauthorized.into());
    }
    let council = oracle_council::current_council(&a[10])?;
    let mut mask = 0u8;
    for seat in &a[11..14] {
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
    let WriterPolicyContext {
        mut group,
        mut sleeve,
        mut book,
        mut snapshot,
    } = load_writer_policy_context(program, &a[4], &a[5], &a[6], &a[7], Some(a[3].key))?;
    let underlying: &[u8] = if params.product == 0 {
        b"ram-standardized-baskets"
    } else {
        b"nand-standardized-baskets"
    };
    if group.status != WriterSettlementGroupStatus::Active
        || sleeve.status != WriterSleeveStatus::Active
        || !book.frozen
        || book.series_count != 10
        || group.series_count != 10
        || sleeve.series_count != 10
        || group.sleeve != *a[4].key
        || group.expiry_ts != OCTOBER_EXPIRY
        || !padded_ascii_underlying_matches(&group.underlying_id, underlying)
        || group.anchor_market != book.records[0].market
        || group.anchor_oracle_month
            != derive_oracle_month_pda(program, &book.records[0].market, OCTOBER_EXPIRY).0
        || group.finalized_slot != 0
        || group.settlement_price_atomic != 0
        || group.final_settlement_commitment != [0; 32]
        || sleeve.vault_config != *a[2].key
        || sleeve.policy_registry != *a[3].key
        || sleeve.policy_snapshot != *a[7].key
        || sleeve.policy_hash != snapshot.policy_hash
        || sleeve.scenario_set_hash != snapshot.scenario_set_hash
        || sleeve.risk_limit_hash != snapshot.risk_limit_hash
        || sleeve.security_mode != snapshot.security_mode
        || sleeve.operational_buffer_atoms != snapshot.operational_buffer_atoms
        || params.expected_policy_hash != snapshot.policy_hash
        || params.expected_book_digest != book.book_digest
        || book.book_digest != writer_book_digest(&book)
        || snapshot.series_family_hash != writer_series_family_hash(&book)
        || registry.latest_policy_version.checked_add(1) != Some(params.next_policy_version)
    {
        return invalid();
    }
    let mut policy = dlmm::load_policy(program, &a[9], &a[4], &snapshot, &book, true)?;
    october_ladder::require_unissued(&sleeve, &book, &policy)?;
    let policy_input = InstallOctoberLadderParams {
        product: params.product,
        next_policy_version: params.next_policy_version,
        expected_book_digest: params.expected_book_digest,
        expected_policy_hash: params.expected_policy_hash,
        beta_ppm: params.beta_ppm,
        lambda_ppm: params.lambda_ppm,
        model_margin_vector_hash: params.model_margin_vector_hash,
        execution_cost_vector_hash: params.execution_cost_vector_hash,
        series_prices: [0; 160],
    };
    let mut signed = october_ladder::original_policy(&snapshot, &policy_input);
    if signed.risk_limit_hash != writer_risk_limit_hash(&signed)
        || signed.policy_hash
            != writer_policy_hash(
                program,
                a[4].key,
                a[5].key,
                &snapshot.series_family_hash,
                &signed,
            )
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let (new_snapshot_key, bump) =
        derive_writer_policy_snapshot_pda(program, a[4].key, params.next_policy_version);
    if *a[8].key != new_snapshot_key {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program, &a[8])?;
    let old_records = book.records[..10].to_vec();
    let old_terms = policy.series[..10].to_vec();
    for (index, record) in old_records.iter().enumerate() {
        validate_old_series(
            program,
            params.product,
            index,
            record,
            &a[15 + index * 2],
            &a[16 + index * 2],
        )?;
    }
    for side in 0..2 {
        for ordinal in 0..9 {
            let destination = side * 9 + ordinal;
            if ordinal < 5 {
                book.records[destination] = old_records[side * 5 + ordinal];
                policy.series[destination] = old_terms[side * 5 + ordinal];
            } else {
                let new_index = side * 4 + ordinal - 5;
                let start = 35 + new_index * 3;
                let record = new_record(
                    program,
                    params.product,
                    side,
                    ordinal,
                    &a[start..start + 3],
                    a[4].key,
                    &a[5],
                    &group,
                )?;
                let bytes = &params.series_prices[new_index * 16..(new_index + 1) * 16];
                let terms = WriterDlmmSeriesPolicyV1 {
                    conservative_claim_value_atoms: u64::from_le_bytes(
                        bytes[..8].try_into().unwrap(),
                    ),
                    seller_floor_quote_atoms: u64::from_le_bytes(bytes[8..].try_into().unwrap()),
                    ..old_terms[side * 5]
                };
                dlmm::validate_series_terms(&record, &terms)?;
                book.records[destination] = record;
                policy.series[destination] = terms;
            }
        }
    }
    book.series_count = 18;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = clock.slot;
    snapshot.bump = bump;
    snapshot.policy_version = params.next_policy_version;
    snapshot.series_family_hash = writer_series_family_hash(&book);
    signed.policy_version = params.next_policy_version;
    snapshot.policy_hash = writer_policy_hash(
        program,
        a[4].key,
        a[5].key,
        &snapshot.series_family_hash,
        &signed,
    );
    snapshot.created_slot = clock.slot;
    snapshot.sealed_slot = clock.slot;
    sleeve.policy_snapshot = new_snapshot_key;
    sleeve.policy_version = snapshot.policy_version;
    sleeve.policy_hash = snapshot.policy_hash;
    sleeve.series_count = 18;
    sleeve.last_updated_slot = clock.slot;
    group.series_count = 18;
    group.last_updated_slot = clock.slot;
    registry.latest_policy_version = snapshot.policy_version;
    registry.last_updated_slot = clock.slot;
    policy.policy_snapshot = new_snapshot_key;
    policy.series_count = 18;
    policy.appended_series_count = 18;
    policy.sealed_slot = clock.slot;
    let mut digest = dlmm::initial_policy_hash(&policy, &snapshot);
    for index in 0..18 {
        digest = dlmm::append_policy_hash(
            &digest,
            index as u8,
            &book.records[index],
            &policy.series[index],
        );
    }
    policy.expected_policy_hash = digest;
    policy.rolling_policy_hash = digest;
    create_program_account(
        &a[0],
        &a[8],
        &a[14],
        program,
        WriterPolicySnapshotV1::LEN,
        &[
            crate::constants::WRITER_POLICY_SNAPSHOT_PDA_SEED,
            a[4].key.as_ref(),
            &snapshot.policy_version.to_le_bytes(),
            &[bump],
        ],
    )?;
    store_state(&a[3], registry.as_ref())?;
    store_state(&a[4], sleeve.as_ref())?;
    store_state(&a[5], group.as_ref())?;
    store_state(&a[6], book.as_ref())?;
    store_state(&a[8], snapshot.as_ref())?;
    store_state(&a[9], policy.as_ref())
}
