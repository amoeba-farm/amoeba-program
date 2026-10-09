//! A consumed council bootstrap receipt authenticates the two exceptional CFM
//! cohorts. It permits earlier council application, never a different price rule.
use super::*;
use crate::compact_error::CompactAccountInfo;

pub(in crate::processor) fn validate_settlement_council_cohort(
    program: &Pubkey,
    market: &Market,
    month_key: &Pubkey,
    month: &OracleMonthState,
    info: &AccountInfo,
) -> ProgramResult {
    if !cfg!(feature = "mainnet-v3")
        || *program != PROGRAM
        || *program != crate::id()
        || info.is_signer
        || info.is_writable
        || info.data_len() != RECEIPT_LEN
    {
        return invalid();
    }
    let raw = info.try_data()?;
    let candidate = Receipt::try_from_slice(&raw).map_err(|_| VaultError::InvalidOracleState)?;
    drop(raw);
    let october = match market.instrument.expiry_ts {
        EXPIRY => false,
        OCTOBER_EXPIRY => true,
        _ => return invalid(),
    };
    let r = receipt(
        program,
        info,
        month_key,
        candidate.market_index,
        &candidate.plan_hash,
        october,
    )?;
    let product = r.market_index / 2;
    let count = if product == 0 { 13 } else { 22 };
    let registry_hash = if product == 0 {
        RAMX_REGISTRY_HASH
    } else {
        NANDX_REGISTRY_HASH
    };
    let underlying: &[u8] = if product == 0 {
        b"ram-standardized-baskets"
    } else {
        b"nand-standardized-baskets"
    };
    let kind = if r.market_index % 2 == 0 {
        OptionKind::CallSpread
    } else {
        OptionKind::PutSpread
    };
    let game = r
        .finished_at
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let amended_october_anchor = october
        && market.instrument.contract_size == 1_000_000
        && market.instrument.strike_price == 100_000_000
        && market.instrument.cap_price
            == if kind == OptionKind::CallSpread {
                105_000_000
            } else {
                95_000_000
            }
        && market.instrument.max_payout_per_contract == 5_000_000;
    if !r.finished
        || r.cursor != count
        || r.registry_hash != registry_hash
        || r.initial_month_hash == [0; 32]
        || crate::pubkey_is_default(&r.proposer)
        || r.seats_hash == [0; 32]
        || r.finished_at < r.begun_at
        || r.finished_at >= expiry(october)
        || (october && r.begun_at < EXPIRY)
        || market.instrument.kind != kind
        || !padded_ascii_underlying_matches(&market.instrument.underlying_id, underlying)
        || (market.instrument.max_payout_per_contract != MAX_PAYOUT_PER_CONTRACT_ATOMS
            && !amended_october_anchor)
        || month.phase != OraclePhase::Game
        || month.schedule_version != COUNCIL_BOOTSTRAP_SCHEDULE_VERSION
        || month.scramble_start_ts != game
        || month.listing_ts != game
        || month.source_count != u16::from(count)
        || month.frozen_source_count != u16::from(count)
        || month.opened_source_count != u16::from(count)
        || month.opening_resolved_source_count != u16::from(count)
        || month.active_weight_group_count != u16::from(count)
        || month.weight_scheme_version != 1
        || month.effective_weight_total_bps != 10_000
        || month.active_weight_scheme_version != OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION
        || month.recipe_hash == [0; 32]
        || month.weight_manifest_hash == [0; 32]
        || month.active_weight_manifest_hash == [0; 32]
    {
        return invalid();
    }
    Ok(())
}
