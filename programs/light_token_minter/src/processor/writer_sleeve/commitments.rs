use super::*;

// This append path is shared by the bounded family/book encoders. Keeping it
// out of line avoids duplicating the same checked copy at every SBF call site.
#[inline(never)]
fn append_hash_bytes<const N: usize>(buffer: &mut [u8; N], len: &mut usize, value: &[u8]) {
    let end = *len + value.len();
    buffer[*len..end].copy_from_slice(value);
    *len = end;
}

#[inline(always)]
fn writer_security_mode_byte(value: WriterSecurityMode) -> u8 {
    match value {
        WriterSecurityMode::GrossExternalMaxPayout => 0,
        WriterSecurityMode::ExactExternalEnvelope => 1,
    }
}

#[inline(always)]
fn writer_auction_rule_byte(value: WriterAuctionPriorityRule) -> u8 {
    match value {
        WriterAuctionPriorityRule::PayAsBidPriceThenSeriesProRata => 0,
    }
}

pub(in crate::processor) fn writer_series_family_hash(book: &WriterSeriesBookV1) -> [u8; 32] {
    let count = usize::from(book.series_count);
    let mut bytes = [0u8; WRITER_SERIES_FAMILY_HASH_MAX_BYTES];
    let mut len = 0usize;
    append_hash_bytes(&mut bytes, &mut len, WRITER_SERIES_FAMILY_HASH_DOMAIN);
    append_hash_bytes(
        &mut bytes,
        &mut len,
        &u32::from(book.series_count).to_le_bytes(),
    );
    for record in book.records.iter().take(count) {
        append_hash_bytes(&mut bytes, &mut len, &record.series_id);
        append_hash_bytes(&mut bytes, &mut len, &record.payoff_digest);
    }
    hashv(&[&bytes[..len]]).to_bytes()
}

pub(in crate::processor) fn writer_book_digest(book: &WriterSeriesBookV1) -> [u8; 32] {
    writer_book_digest_inner(book, false)
}

pub(in crate::processor) fn writer_close_book_digest(book: &WriterSeriesBookV1) -> [u8; 32] {
    writer_book_digest_inner(book, true)
}

fn writer_book_digest_inner(book: &WriterSeriesBookV1, economic_only: bool) -> [u8; 32] {
    let count = usize::from(book.series_count);
    let mut bytes = [0u8; WRITER_BOOK_HASH_MAX_BYTES];
    let mut len = 0usize;
    append_hash_bytes(&mut bytes, &mut len, WRITER_BOOK_HASH_DOMAIN);
    append_hash_bytes(&mut bytes, &mut len, book.sleeve.as_ref());
    append_hash_bytes(&mut bytes, &mut len, book.settlement_group.as_ref());
    append_hash_bytes(
        &mut bytes,
        &mut len,
        &u32::from(book.series_count).to_le_bytes(),
    );
    for record in book.records.iter().take(count) {
        append_hash_bytes(&mut bytes, &mut len, &record.series_id);
        append_hash_bytes(&mut bytes, &mut len, &record.payoff_digest);
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &if economic_only {
                record.external_open_interest_atoms
            } else {
                record.total_physical_supply_atoms
            }
            .to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &if economic_only {
                0u64
            } else {
                record.issuer_controlled_atoms
            }
            .to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.external_open_interest_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.primary_premium_collected_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.settlement_external_oi_snapshot_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.settlement_liability_initial_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.settlement_liability_remaining_atoms.to_le_bytes(),
        );
    }
    hashv(&[&bytes[..len]]).to_bytes()
}

pub(in crate::processor) fn writer_group_commitment(
    group_key: &Pubkey,
    group: &WriterSettlementGroupV1,
) -> [u8; 32] {
    // Solana's hashv feeds these slices into one SHA-256 state in order, exactly
    // matching a hash of their concatenation without a 512-byte stack buffer.
    // The byte-contract regression below pins that equivalence.
    hashv(&[
        b"ameba-writer-settlement-group-v1",
        group_key.as_ref(),
        &group.underlying_id,
        &group.expiry_ts.to_le_bytes(),
        group.settlement_mint.as_ref(),
        group.anchor_market.as_ref(),
        group.anchor_oracle_month.as_ref(),
        &group.oracle_methodology_version.to_le_bytes(),
        &group.product_manifest_root,
        &group.coverage_manifest_hash,
        &group.recipe_hash,
        &group.settlement_source_digest,
        &group.active_weight_manifest_hash,
        &group.security_cap_atoms.to_le_bytes(),
        group.signer_registry.as_ref(),
        group.signer_set.as_ref(),
        &group.signer_set_version.to_le_bytes(),
        &group.signer_set_hash,
        &group.settlement_ts.to_le_bytes(),
    ])
    .to_bytes()
}

pub(super) fn writer_payoff_digest(
    group: &Pubkey,
    market: &Market,
    market_key: &Pubkey,
    contract_mint: &Pubkey,
) -> [u8; 32] {
    let kind = match market.instrument.kind {
        crate::state::OptionKind::CallSpread => 0,
        crate::state::OptionKind::PutSpread => 1,
    };
    hashv(&[
        WRITER_PAYOFF_HASH_DOMAIN,
        group.as_ref(),
        &market.market_id,
        market_key.as_ref(),
        contract_mint.as_ref(),
        &market.instrument.underlying_id,
        &market.instrument.expiry_ts.to_le_bytes(),
        &market.instrument.strike_price.to_le_bytes(),
        &market.instrument.cap_price.to_le_bytes(),
        &market.instrument.contract_size.to_le_bytes(),
        &market.instrument.max_payout_per_contract.to_le_bytes(),
        &[kind],
    ])
    .to_bytes()
}

pub(super) fn writer_risk_limit_hash(params: &SealWriterPolicyV1Params) -> [u8; 32] {
    hashv(&[
        WRITER_RISK_HASH_DOMAIN,
        &params.drawdown_scale.to_le_bytes(),
        &params.worst_drawdown_limit.to_le_bytes(),
        &params.upper_drawdown_limit.to_le_bytes(),
        &params.lower_drawdown_limit.to_le_bytes(),
        &params.lower_tail_max_settlement_atomic.to_le_bytes(),
        &params.upper_tail_min_settlement_atomic.to_le_bytes(),
        &params.operational_buffer_atoms.to_le_bytes(),
        &[writer_security_mode_byte(params.security_mode)],
        &params.primary_fee_bps.to_le_bytes(),
        &params.max_auction_issue_atoms.to_le_bytes(),
        &params.max_close_flat_atoms.to_le_bytes(),
        &[params.v2_feature_flags],
    ])
    .to_bytes()
}

pub(super) fn writer_policy_hash(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    group: &Pubkey,
    series_family_hash: &[u8; 32],
    params: &SealWriterPolicyV1Params,
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(384);
    bytes.extend_from_slice(WRITER_POLICY_HASH_DOMAIN);
    bytes.extend_from_slice(program_id.as_ref());
    bytes.extend_from_slice(sleeve.as_ref());
    bytes.extend_from_slice(group.as_ref());
    bytes.extend_from_slice(&params.policy_version.to_le_bytes());
    bytes.extend_from_slice(&params.regime_input_version.to_le_bytes());
    bytes.extend_from_slice(&params.scenario_set_hash);
    bytes.extend_from_slice(&params.risk_limit_hash);
    bytes.extend_from_slice(series_family_hash);
    bytes.extend_from_slice(&params.beta_ppm.to_le_bytes());
    bytes.extend_from_slice(&params.lambda_ppm.to_le_bytes());
    bytes.extend_from_slice(&params.model_margin_vector_hash);
    bytes.extend_from_slice(&params.execution_cost_vector_hash);
    bytes.extend_from_slice(&params.drawdown_scale.to_le_bytes());
    bytes.extend_from_slice(&params.worst_drawdown_limit.to_le_bytes());
    bytes.extend_from_slice(&params.upper_drawdown_limit.to_le_bytes());
    bytes.extend_from_slice(&params.lower_drawdown_limit.to_le_bytes());
    bytes.extend_from_slice(&params.lower_tail_max_settlement_atomic.to_le_bytes());
    bytes.extend_from_slice(&params.upper_tail_min_settlement_atomic.to_le_bytes());
    bytes.extend_from_slice(&params.operational_buffer_atoms.to_le_bytes());
    bytes.extend_from_slice(&params.primary_fee_bps.to_le_bytes());
    bytes.push(writer_auction_rule_byte(params.auction_priority_rule));
    bytes.push(writer_security_mode_byte(params.security_mode));
    bytes.push(params.v2_feature_flags);
    hashv(&[bytes.as_slice()]).to_bytes()
}

pub(super) fn writer_coverage_manifest_hash(coverage: &OracleSkuCoverageManifest) -> [u8; 32] {
    hashv(&[
        WRITER_COVERAGE_HASH_DOMAIN,
        coverage.month.as_ref(),
        &coverage.required_sku_root,
        &coverage.required_sku_count.to_le_bytes(),
        &coverage.covered_sku_count.to_le_bytes(),
        &coverage.planned_scramble_start_ts.to_le_bytes(),
        &coverage.planned_listing_ts.to_le_bytes(),
        &[u8::from(coverage.coverage_finalized)],
        &coverage.coverage_complete_ts.to_le_bytes(),
    ])
    .to_bytes()
}
