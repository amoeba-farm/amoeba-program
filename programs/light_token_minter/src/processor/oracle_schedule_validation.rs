use super::*;

pub(super) fn validate_rulebook_schedule_params(
    market: &Market,
    params: &InitializeOracleMonthV3Params,
) -> ProgramResult {
    let expected_listing_ts = params
        .scramble_start_ts
        .checked_add(ORACLE_PRE_LISTING_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let current_ts = current_unix_timestamp()?;
    if !market.paused
        || market.long_contract_mint.is_none()
        || market.mint_accounting != MarketMintAccounting::canonical_empty()
        || params.scramble_start_ts == 0
        || params.listing_ts != expected_listing_ts
        || params.listing_ts >= market.instrument.expiry_ts
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    ensure_oracle_month_initialization_review_window(current_ts, market.instrument.expiry_ts)
}

pub(super) fn ensure_oracle_month_initialization_review_window(
    current_ts: u64,
    expiry_ts: u64,
) -> ProgramResult {
    let preserved_post_submission_seconds = ORACLE_KILL_WINDOW_SECONDS
        .checked_add(ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS)
        .and_then(|value| value.checked_add(ORACLE_OPENING_WINDOW_SECONDS))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let earliest_safe_listing = current_ts
        .checked_add(preserved_post_submission_seconds)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if earliest_safe_listing >= expiry_ts {
        return Err(VaultError::OracleSkuCoverageExtensionTooLate.into());
    }
    Ok(())
}

pub(super) fn validate_rolling_rulebook_schedule_params(
    market: &Market,
    params: &InitializeOracleMonthV3Params,
) -> ProgramResult {
    validate_rolling_three_month_maturity(params.listing_ts, market.instrument.expiry_ts)?;
    validate_rulebook_schedule_params(market, params)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OracleMaturityLadderTransition {
    SameRung,
    Advance,
}

pub(super) fn validate_oracle_maturity_ladder_transition(
    registry: &OracleMaturityLadderRegistry,
    planned_listing_ts: u64,
    planned_expiry_ts: u64,
    current_ts: u64,
) -> Result<OracleMaturityLadderTransition, ProgramError> {
    validate_rolling_three_month_maturity(registry.planned_listing_ts, registry.planned_expiry_ts)?;
    validate_rolling_three_month_maturity(planned_listing_ts, planned_expiry_ts)?;
    if planned_listing_ts == registry.planned_listing_ts
        && planned_expiry_ts == registry.planned_expiry_ts
    {
        return Ok(OracleMaturityLadderTransition::SameRung);
    }
    if current_ts < registry.planned_listing_ts {
        return Err(VaultError::InvalidOracleMaturityLadder.into());
    }

    let (prior_listing_year, prior_listing_month, _, _) =
        utc_calendar_parts(registry.planned_listing_ts)?;
    let (next_listing_year, next_listing_month, _, _) = utc_calendar_parts(planned_listing_ts)?;
    let (prior_expiry_year, prior_expiry_month, _, _) =
        utc_calendar_parts(registry.planned_expiry_ts)?;
    let (next_expiry_year, next_expiry_month, _, _) = utc_calendar_parts(planned_expiry_ts)?;
    let prior_listing_month_index = prior_listing_year
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(prior_listing_month) - 1))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let next_listing_month_index = next_listing_year
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(next_listing_month) - 1))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let prior_expiry_month_index = prior_expiry_year
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(prior_expiry_month) - 1))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let next_expiry_month_index = next_expiry_year
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(next_expiry_month) - 1))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let expected_next_listing_month_index = prior_listing_month_index
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let expected_next_expiry_month_index = prior_expiry_month_index
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if next_listing_month_index != expected_next_listing_month_index
        || next_expiry_month_index != expected_next_expiry_month_index
    {
        return Err(VaultError::InvalidOracleMaturityLadder.into());
    }
    Ok(OracleMaturityLadderTransition::Advance)
}

pub(super) fn validate_rolling_three_month_maturity(
    listing_ts: u64,
    expiry_ts: u64,
) -> ProgramResult {
    let (listing_year, listing_month, listing_day, listing_second_of_day) =
        utc_calendar_parts(listing_ts)?;
    let (expiry_year, expiry_month, expiry_day, expiry_second_of_day) =
        utc_calendar_parts(expiry_ts)?;
    let listing_month_index = listing_year
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(listing_month) - 1))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let expiry_month_index = expiry_year
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(expiry_month) - 1))
        .ok_or(VaultError::ArithmeticOverflow)?;
    let expected_expiry_month_index = listing_month_index
        .checked_add(ORACLE_ROLLING_MATURITY_MONTHS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if listing_day != 1
        || expiry_day != 1
        || listing_second_of_day != ORACLE_MONTH_ROLL_SECOND_UTC
        || listing_second_of_day != expiry_second_of_day
        || expiry_month_index != expected_expiry_month_index
    {
        return Err(VaultError::InvalidOracleMaturityLadder.into());
    }
    Ok(())
}

/// Convert a Unix timestamp to proleptic-Gregorian UTC components without a
/// timezone or heap dependency. This is Howard Hinnant's civil-from-days
/// transform with the Unix epoch offset.
pub(super) fn utc_calendar_parts(timestamp: u64) -> Result<(i64, u32, u32, u64), ProgramError> {
    const SECONDS_PER_DAY: u64 = 86_400;
    const UNIX_EPOCH_CIVIL_OFFSET_DAYS: i64 = 719_468;
    let days_since_epoch =
        i64::try_from(timestamp / SECONDS_PER_DAY).map_err(|_| VaultError::ArithmeticOverflow)?;
    let shifted_days = days_since_epoch
        .checked_add(UNIX_EPOCH_CIVIL_OFFSET_DAYS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era
        .checked_add(era.checked_mul(400).ok_or(VaultError::ArithmeticOverflow)?)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year = year.checked_add(1).ok_or(VaultError::ArithmeticOverflow)?;
    }
    Ok((
        year,
        u32::try_from(month).map_err(|_| VaultError::ArithmeticOverflow)?,
        u32::try_from(day).map_err(|_| VaultError::ArithmeticOverflow)?,
        timestamp % SECONDS_PER_DAY,
    ))
}

pub(super) fn rulebook_schedule_boundaries(
    month: &OracleMonthState,
) -> Result<(u64, u64, u64), ProgramError> {
    if !month.has_rulebook_schedule() {
        return Err(VaultError::InvalidOracleState.into());
    }
    let placement_end = month
        .scramble_start_ts
        .checked_add(ORACLE_PLACEMENT_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let kill_end = placement_end
        .checked_add(ORACLE_KILL_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let scramble_end = kill_end
        .checked_add(ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let expected_listing = scramble_end
        .checked_add(ORACLE_OPENING_WINDOW_SECONDS)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if scramble_end
        != month
            .scramble_start_ts
            .checked_add(ORACLE_SCRAMBLE_WINDOW_SECONDS)
            .ok_or(VaultError::ArithmeticOverflow)?
        || month.listing_ts != expected_listing
    {
        return Err(VaultError::InvalidOracleState.into());
    }
    Ok((placement_end, kill_end, scramble_end))
}

pub(super) fn ensure_oracle_placement_window(month: &OracleMonthState) -> ProgramResult {
    if month.phase != OraclePhase::Scramble {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    let (placement_end, _, _) = rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    if now < month.scramble_start_ts || now >= placement_end {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_kill_window(month: &OracleMonthState) -> ProgramResult {
    if month.phase != OraclePhase::Scramble {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    let (placement_end, kill_end, _) = rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    if now < placement_end || now >= kill_end {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_resolution_freeze_window(month: &OracleMonthState) -> ProgramResult {
    if month.phase != OraclePhase::Scramble {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    let (_, kill_end, scramble_end) = rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    if now < kill_end || now >= scramble_end {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_opening_window(month: &OracleMonthState) -> ProgramResult {
    if month.phase != OraclePhase::Opening {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    let (_, _, opening_start) = rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    if now < opening_start || now >= month.listing_ts {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(super) fn validate_oracle_opening_evidence(
    market: &Market,
    month_state: &OracleMonthState,
    month: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    params: &SubmitOracleOpeningClaimParams,
) -> Result<[u8; 32], ProgramError> {
    validate_oracle_opening_evidence_fields(
        market,
        rulebook_schedule_boundaries(month_state)?.2,
        source,
        params.opening_state,
        params.source_time,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
        &params.archive_url,
    )?;
    Ok(derive_oracle_opening_evidence_hash(
        month,
        source_key,
        params.opening_state,
        params.source_time,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
        &params.archive_url,
    ))
}

pub(super) fn validate_oracle_opening_alternative_evidence(
    market: &Market,
    month_state: &OracleMonthState,
    month: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    params: &ChallengeOracleOpeningClaimParams,
) -> Result<[u8; 32], ProgramError> {
    validate_oracle_opening_evidence_fields(
        market,
        rulebook_schedule_boundaries(month_state)?.2,
        source,
        params.alternative_opening_state,
        params.alternative_source_time,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
        &params.archive_url,
    )?;
    Ok(derive_oracle_opening_evidence_hash(
        month,
        source_key,
        params.alternative_opening_state,
        params.alternative_source_time,
        &params.canonical_locator_hash,
        &params.source_definition_hash,
        &params.archive_url,
    ))
}

pub(super) fn oracle_opening_challenge_is_true_noop(
    claim: &OracleOpeningClaim,
    params: &ChallengeOracleOpeningClaimParams,
) -> bool {
    params.alternative_opening_state == claim.opening_state
        && params.alternative_source_time == claim.source_time
        && params.canonical_locator_hash == claim.canonical_locator_hash
        && params.source_definition_hash == claim.source_definition_hash
        && derive_oracle_opening_archive_url_hash(&params.archive_url) == claim.archive_url_hash
}

pub(super) fn validate_oracle_opening_evidence_fields(
    market: &Market,
    capture_start: u64,
    source: &OracleSourceState,
    opening_state: u64,
    source_time: u64,
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
    archive_url: &str,
) -> ProgramResult {
    let now = current_unix_timestamp()?;
    validate_oracle_opening_evidence_fields_at_time(
        market,
        capture_start,
        source,
        opening_state,
        source_time,
        canonical_locator_hash,
        source_definition_hash,
        archive_url,
        now,
    )
}

pub(super) fn validate_oracle_opening_evidence_fields_at_time(
    market: &Market,
    capture_start: u64,
    source: &OracleSourceState,
    opening_state: u64,
    source_time: u64,
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
    archive_url: &str,
    now: u64,
) -> ProgramResult {
    if opening_state == 0
        || source_time < capture_start
        || source_time > now
        || source_time > market.instrument.expiry_ts
        || crate::bytes32_is_zero(&*canonical_locator_hash)
        || *canonical_locator_hash != source.canonical_locator_hash
        || crate::bytes32_is_zero(&*source_definition_hash)
        || *source_definition_hash != source.source_definition_hash
    {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    }
    validate_oracle_opening_archive_binding(archive_url, source_time, canonical_locator_hash)
}

pub(super) fn validate_oracle_opening_archive_binding(
    archive_url: &str,
    source_time: u64,
    canonical_locator_hash: &[u8; 32],
) -> ProgramResult {
    let (capture_time, original_url) = parse_oracle_opening_archive_url(archive_url)?;
    let archived_locator_hash = hashv(&[b"locator", original_url.as_bytes()]).to_bytes();
    if capture_time != source_time || archived_locator_hash != *canonical_locator_hash {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    }
    Ok(())
}

pub(super) fn parse_oracle_opening_archive_url(
    archive_url: &str,
) -> Result<(u64, &str), ProgramError> {
    let bytes = archive_url.as_bytes();
    if bytes.len() > ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES
        || bytes.iter().any(|byte| *byte <= b' ' || *byte == 0x7f)
    {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    }
    let suffix = bytes
        .strip_prefix(ORACLE_OPENING_ARCHIVE_URL_PREFIX.as_bytes())
        .ok_or(VaultError::InvalidOracleOpeningEvidence)?;
    if suffix.len() <= 15 || suffix.get(14) != Some(&b'/') {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    }
    let original_url = suffix
        .get(15..)
        .ok_or(VaultError::InvalidOracleOpeningEvidence)?;
    let authority_and_path = if let Some(value) = original_url.strip_prefix(b"https://") {
        value
    } else if let Some(value) = original_url.strip_prefix(b"http://") {
        value
    } else {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    };
    let authority_end = authority_and_path
        .iter()
        .position(|byte| matches!(*byte, b'/' | b'?' | b'#'))
        .unwrap_or(authority_and_path.len());
    let authority = authority_and_path
        .get(..authority_end)
        .ok_or(VaultError::InvalidOracleOpeningEvidence)?;
    let host_start = authority
        .iter()
        .rposition(|byte| *byte == b'@')
        .map_or(0, |index| index + 1);
    let host_and_port = authority
        .get(host_start..)
        .ok_or(VaultError::InvalidOracleOpeningEvidence)?;
    if authority.is_empty() || host_and_port.is_empty() || host_and_port.starts_with(b":") {
        return Err(VaultError::InvalidOracleOpeningEvidence.into());
    }
    let capture_time = parse_wayback_utc_timestamp(&suffix[..14])
        .ok_or(VaultError::InvalidOracleOpeningEvidence)?;
    // The slice begins after an ASCII prefix, fourteen ASCII timestamp bytes, and an ASCII slash,
    // so it remains on a UTF-8 boundary within the original `&str`.
    let original_url = unsafe { std::str::from_utf8_unchecked(original_url) };
    Ok((capture_time, original_url))
}

pub(super) fn parse_wayback_utc_timestamp(timestamp: &[u8]) -> Option<u64> {
    if timestamp.len() != 14 || !timestamp.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = parse_ascii_decimal(&timestamp[0..4]);
    let month = parse_ascii_decimal(&timestamp[4..6]);
    let day = parse_ascii_decimal(&timestamp[6..8]);
    let hour = parse_ascii_decimal(&timestamp[8..10]);
    let minute = parse_ascii_decimal(&timestamp[10..12]);
    let second = parse_ascii_decimal(&timestamp[12..14]);
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || hour >= 24
        || minute >= 60
        || second >= 60
    {
        return None;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > days_in_month {
        return None;
    }
    const DAYS_BEFORE_MONTH: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap_days_before_year = |value: u64| {
        let prior = value - 1;
        prior / 4 - prior / 100 + prior / 400
    };
    let days = (year - 1970) * 365 + leap_days_before_year(year) - leap_days_before_year(1970)
        + DAYS_BEFORE_MONTH[(month - 1) as usize]
        + u64::from(leap_year && month > 2)
        + day
        - 1;
    Some(((days * 24 + hour) * 60 + minute) * 60 + second)
}

pub(super) fn parse_ascii_decimal(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |value, byte| value * 10 + u64::from(*byte - b'0'))
}

pub(super) fn derive_oracle_opening_evidence_hash(
    month: &Pubkey,
    source: &Pubkey,
    opening_state: u64,
    source_time: u64,
    canonical_locator_hash: &[u8; 32],
    source_definition_hash: &[u8; 32],
    archive_url: &str,
) -> [u8; 32] {
    let opening_state_bytes = opening_state.to_le_bytes();
    let source_time_bytes = source_time.to_le_bytes();
    hashv(&[
        ORACLE_OPENING_EVIDENCE_HASH_DOMAIN,
        month.as_ref(),
        source.as_ref(),
        &opening_state_bytes,
        &source_time_bytes,
        canonical_locator_hash,
        source_definition_hash,
        archive_url.as_bytes(),
    ])
    .to_bytes()
}

pub(super) fn derive_oracle_opening_archive_url_hash(archive_url: &str) -> [u8; 32] {
    hashv(&[
        ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN,
        archive_url.as_bytes(),
    ])
    .to_bytes()
}

pub(super) fn derive_oracle_update_evidence_hash(
    month: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    state: u64,
    source_time: u64,
    archive_url: &str,
) -> [u8; 32] {
    hashv(&[
        ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
        month.as_ref(),
        source_key.as_ref(),
        &source.source_id,
        &state.to_le_bytes(),
        &source_time.to_le_bytes(),
        &source.canonical_locator_hash,
        &source.source_definition_hash,
        archive_url.as_bytes(),
    ])
    .to_bytes()
}

pub(super) fn validate_oracle_update_evidence(
    market: &Market,
    month_state: &OracleMonthState,
    month: &Pubkey,
    source_key: &Pubkey,
    source: &OracleSourceState,
    state: u64,
    source_time: u64,
    evidence_hash: &[u8; 32],
    archive_url: &str,
) -> Result<[u8; 32], ProgramError> {
    let now = current_unix_timestamp()?;
    if state == 0
        || source_time < month_state.listing_ts
        || source_time > now
        || source_time > market.instrument.expiry_ts
        || crate::bytes32_is_zero(evidence_hash)
    {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    validate_oracle_opening_archive_binding(
        archive_url,
        source_time,
        &source.canonical_locator_hash,
    )
    .map_err(|_| ProgramError::from(VaultError::InvalidOracleUpdateAccount))?;
    let expected = derive_oracle_update_evidence_hash(
        month,
        source_key,
        source,
        state,
        source_time,
        archive_url,
    );
    if *evidence_hash != expected {
        return Err(VaultError::InvalidOracleUpdateAccount.into());
    }
    Ok(derive_oracle_opening_archive_url_hash(archive_url))
}

pub(super) fn ensure_oracle_freeze_window(
    market: &Market,
    month: &OracleMonthState,
) -> ProgramResult {
    if month.listing_ts >= market.instrument.expiry_ts {
        return Err(VaultError::InvalidOracleState.into());
    }
    ensure_oracle_resolution_freeze_window(month)
}

pub(super) fn ensure_oracle_game_transition_window(
    market: &Market,
    month: &OracleMonthState,
) -> ProgramResult {
    if month.phase != OraclePhase::Opening {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    if month.listing_ts >= market.instrument.expiry_ts
        || now < month.listing_ts
        || now >= market.instrument.expiry_ts
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}

pub(super) fn ensure_oracle_game_window(
    market: &Market,
    month: &OracleMonthState,
) -> ProgramResult {
    if month.phase != OraclePhase::Game {
        return Err(VaultError::InvalidOraclePhase.into());
    }
    rulebook_schedule_boundaries(month)?;
    let now = current_unix_timestamp()?;
    if month.listing_ts >= market.instrument.expiry_ts
        || now < month.listing_ts
        || now >= market.instrument.expiry_ts
    {
        return Err(VaultError::OracleTimingWindowClosed.into());
    }
    Ok(())
}
