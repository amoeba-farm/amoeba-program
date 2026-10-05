//! One bounded October amendment, before any issuance. The original oracle
//! anchors, immutable policy snapshot and contributor records are untouched.
use super::*;
use crate::instruction::InstallOctoberLadderParams;
use crate::oracle_parent_proxy::september_bootstrap::{EXPIRY, OCTOBER_EXPIRY, PROGRAM};
use crate::state::{OptionKind, WriterDlmmPolicyV1, WriterDlmmSeriesPolicyV1};

const COUNT: usize = 10;
const UNIT: u64 = 1_000_000;
const WIDTH: u64 = 5 * UNIT;
const STRIKES: [u64; 5] = [100, 90, 95, 105, 110];

fn invalid<T>() -> Result<T, ProgramError> {
    Err(VaultError::InvalidWriterLifecycle.into())
}

fn series_id(product: u8, index: usize) -> [u8; 32] {
    let prefix: &[u8] = if product == 0 {
        b"RAMX-202610-"
    } else {
        b"NANDX-202610-"
    };
    let side: &[u8] = if index < 5 { b"CALL-0" } else { b"PUT-0" };
    let mut id = [0; 32];
    id[..prefix.len()].copy_from_slice(prefix);
    id[prefix.len()..prefix.len() + side.len()].copy_from_slice(side);
    id[prefix.len() + side.len()] = b'1' + (index % 5) as u8;
    id
}

pub(super) fn require_unissued(
    sleeve: &WriterSleeveV1,
    book: &WriterSeriesBookV1,
    policy: &WriterDlmmPolicyV1,
) -> ProgramResult {
    if book.individual != crate::individual_writer::IndividualTotals::default()
        || sleeve.accounted_asset_atoms != sleeve.writer_principal_atoms
        || [
            sleeve.locked_primary_premium_atoms,
            sleeve.exact_reserve_atoms,
            sleeve.upper_tail_reserve_atoms,
            sleeve.lower_tail_reserve_atoms,
            sleeve.security_exposure_atoms,
            sleeve.long_liability_initial_atoms,
            sleeve.long_liability_remaining_atoms,
            sleeve.writer_residual_initial_atoms,
            sleeve.writer_residual_remaining_atoms,
            sleeve.settlement_principal_atoms,
            sleeve.unclaimed_principal_atoms,
            sleeve.stranded_surplus_atoms,
            sleeve.settlement_finalized_slot,
            policy.monthly_spent_atoms,
            policy.total_pool_quote_atoms,
            policy.total_uncommitted_quote_atoms,
        ]
        .iter()
        .any(|v| *v != 0)
        || policy.series_monthly_spent_atoms.iter().any(|v| *v != 0)
        || policy.series_pool_inventory_atoms.iter().any(|v| *v != 0)
        || book.records[..usize::from(book.series_count)]
            .iter()
            .any(|r| {
                r.settlement_status != WriterSeriesSettlementStatus::Open
                    || [
                        r.total_physical_supply_atoms,
                        r.issuer_controlled_atoms,
                        r.external_open_interest_atoms,
                        r.primary_premium_collected_atoms,
                        r.settlement_external_oi_snapshot_atoms,
                        r.settlement_liability_initial_atoms,
                        r.settlement_liability_remaining_atoms,
                    ]
                    .iter()
                    .any(|v| *v != 0)
            })
    {
        return invalid();
    }
    Ok(())
}

/// Reconstruct the original signed policy inputs. The four fields absent from
/// the snapshot are authenticated by recomputing its existing policy hash.
pub(super) fn original_policy(
    snapshot: &WriterPolicySnapshotV1,
    params: &InstallOctoberLadderParams,
) -> SealWriterPolicyV1Params {
    SealWriterPolicyV1Params {
        policy_version: snapshot.policy_version,
        regime_input_version: snapshot.regime_input_version,
        scenario_set_hash: snapshot.scenario_set_hash,
        risk_limit_hash: snapshot.risk_limit_hash,
        policy_hash: snapshot.policy_hash,
        model_margin_vector_hash: params.model_margin_vector_hash,
        execution_cost_vector_hash: params.execution_cost_vector_hash,
        beta_ppm: params.beta_ppm,
        lambda_ppm: params.lambda_ppm,
        security_mode: snapshot.security_mode,
        reserve_rounding_mode: snapshot.reserve_rounding_mode,
        v2_feature_flags: snapshot.v2_feature_flags,
        drawdown_scale: snapshot.drawdown_scale,
        worst_drawdown_limit: snapshot.worst_drawdown_limit,
        upper_drawdown_limit: snapshot.upper_drawdown_limit,
        lower_drawdown_limit: snapshot.lower_drawdown_limit,
        lower_tail_max_settlement_atomic: snapshot.lower_tail_max_settlement_atomic,
        upper_tail_min_settlement_atomic: snapshot.upper_tail_min_settlement_atomic,
        operational_buffer_atoms: snapshot.operational_buffer_atoms,
        max_issue_atoms: snapshot.max_issue_atoms,
    }
}

#[inline(never)]
fn amend_market(
    product: u8,
    index: usize,
    market: &mut Market,
    group: &WriterSettlementGroupV1,
) -> ProgramResult {
    let call = index < 5;
    let old = index % 5 == 0;
    let strike = STRIKES[index % 5] * UNIT;
    let width = if old { 12 * UNIT } else { WIDTH };
    let cap = if call { strike + width } else { strike - width };
    if !market.paused
        || market.market_id != series_id(product, index)
        || market.instrument.kind
            != if call {
                OptionKind::CallSpread
            } else {
                OptionKind::PutSpread
            }
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != OCTOBER_EXPIRY
        || market.collateral_mint != group.settlement_mint
        || market.instrument.contract_size != UNIT
        || market.instrument.strike_price != strike
        || market.instrument.cap_price != cap
        || market.instrument.max_payout_per_contract != width
        || market.total_position_collateral_locked != 0
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
    {
        return invalid();
    }
    market.instrument.cap_price = if call { strike + WIDTH } else { strike - WIDTH };
    market.instrument.max_payout_per_contract = WIDTH;
    Ok(())
}

/// admin/payer, policy authority, vault config, registry, sleeve, group, book,
/// old snapshot, new snapshot, DLMM policy, controller, three current seats,
/// System, then ten ordered (market, mint, absent DLMM pool) triples.
#[inline(never)]
pub(in crate::processor) fn process_install_october_ladder(
    program: &Pubkey,
    a: &[AccountInfo],
    params: InstallOctoberLadderParams,
) -> ProgramResult {
    if !cfg!(feature = "mainnet-v3")
        || *program != PROGRAM
        || *program != crate::id()
        || a.len() != 45
        || params.product > 1
    {
        return invalid();
    }
    for (i, info) in a.iter().enumerate() {
        let signer = matches!(i, 0 | 1 | 11..=13);
        let writable = matches!(i, 0 | 3..=6 | 8..=9)
            || (i >= 15 && (i - 15) % 3 == 0)
            || (i == 1 && a[0].key == info.key);
        if info.is_signer != signer || info.is_writable != writable || (info.executable && i != 14)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        for j in 0..i {
            if a[j].key == info.key && !(j == 0 && i == 1) {
                return Err(VaultError::InvalidAccountList.into());
            }
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
        || book.series_count != 2
        || group.sleeve != *a[4].key
        || group.expiry_ts != OCTOBER_EXPIRY
        || !padded_ascii_underlying_matches(&group.underlying_id, underlying)
        || group.anchor_market != *a[15].key
        || group.anchor_oracle_month
            != derive_oracle_month_pda(program, a[15].key, OCTOBER_EXPIRY).0
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
    require_unissued(&sleeve, &book, &policy)?;
    let mut signed = original_policy(&snapshot, &params);
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
    let old_records = [book.records[0], book.records[1]];
    let old_terms = [policy.series[0], policy.series[1]];
    let mut markets = Vec::with_capacity(COUNT);
    for index in 0..COUNT {
        let i = 15 + 3 * index;
        let mut market = load_valid_market(program, &a[i])?;
        if market.long_contract_mint != Some(*a[i + 1].key) {
            return invalid();
        }
        let mint = validate_canonical_market_mint(&a[i], &mut market, &a[i + 1], 0)?;
        if mint.supply != 0 {
            return invalid();
        }
        let pool = crate::ameba_dlmm_state::derive_ameba_dlmm_pool_pda(program, a[i].key).0;
        validate_canonical_system_zero_pda_proof(&pool, &a[i + 2])?;
        let side = usize::from(index >= 5);
        if index % 5 == 0 {
            let r = &old_records[side];
            if r.market != *a[i].key
                || r.contract_mint != *a[i + 1].key
                || r.series_id != market.market_id
                || r.option_kind != market.instrument.kind
                || r.strike_price_atomic != market.instrument.strike_price
                || r.cap_or_floor_price_atomic != market.instrument.cap_price
                || r.max_payout_per_contract_atoms != market.instrument.max_payout_per_contract
                || r.retirement_custody
                    != derive_writer_retirement_custody_pda(program, a[4].key, a[i].key).0
                || r.payoff_digest
                    != writer_payoff_digest(a[5].key, &market, a[i].key, a[i + 1].key)
            {
                return invalid();
            }
        }
        amend_market(params.product, index, &mut market, &group)?;
        let record = WriterSeriesRecordV1 {
            active: true,
            option_kind: market.instrument.kind,
            custody_status: if index % 5 == 0 {
                old_records[side].custody_status
            } else {
                WriterSeriesCustodyStatus::Absent
            },
            settlement_status: WriterSeriesSettlementStatus::Open,
            reserved: if index % 5 == 0 {
                old_records[side].reserved
            } else {
                [1, 1, 0, 0]
            },
            series_id: market.market_id,
            market: *a[i].key,
            contract_mint: *a[i + 1].key,
            retirement_custody: derive_writer_retirement_custody_pda(program, a[4].key, a[i].key).0,
            strike_price_atomic: market.instrument.strike_price,
            cap_or_floor_price_atomic: market.instrument.cap_price,
            contract_size_atoms: UNIT,
            max_payout_per_contract_atoms: WIDTH,
            payoff_digest: writer_payoff_digest(a[5].key, &market, a[i].key, a[i + 1].key),
            ..WriterSeriesRecordV1::EMPTY
        };
        let p = &params.series_prices[index * 16..(index + 1) * 16];
        let terms = WriterDlmmSeriesPolicyV1 {
            conservative_claim_value_atoms: u64::from_le_bytes(p[..8].try_into().unwrap()),
            seller_floor_quote_atoms: u64::from_le_bytes(p[8..].try_into().unwrap()),
            ..old_terms[side]
        };
        dlmm::validate_series_terms(&record, &terms)?;
        book.records[index] = record;
        policy.series[index] = terms;
        markets.push(market);
    }
    book.series_count = COUNT as u8;
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
    sleeve.series_count = COUNT as u8;
    sleeve.last_updated_slot = clock.slot;
    group.series_count = COUNT as u8;
    group.last_updated_slot = clock.slot;
    registry.latest_policy_version = snapshot.policy_version;
    registry.last_updated_slot = clock.slot;
    policy.policy_snapshot = new_snapshot_key;
    policy.series_count = COUNT as u8;
    policy.appended_series_count = COUNT as u8;
    policy.sealed_slot = clock.slot;
    let mut digest = dlmm::initial_policy_hash(&policy, &snapshot);
    for index in 0..COUNT {
        digest = dlmm::append_policy_hash(
            &digest,
            index as u8,
            &book.records[index],
            &policy.series[index],
        );
    }
    policy.expected_policy_hash = digest;
    policy.rolling_policy_hash = digest;
    // Every identity, current counter, term and signature is checked before CPI or writes.
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
    for (index, market) in markets.iter().enumerate() {
        store_state(&a[15 + 3 * index], market)?;
    }
    store_state(&a[3], registry.as_ref())?;
    store_state(&a[4], sleeve.as_ref())?;
    store_state(&a[5], group.as_ref())?;
    store_state(&a[6], book.as_ref())?;
    store_state(&a[8], snapshot.as_ref())?;
    store_state(&a[9], policy.as_ref())
}
