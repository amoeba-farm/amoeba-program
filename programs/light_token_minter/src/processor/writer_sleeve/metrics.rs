use super::*;

#[inline]
pub(super) fn writer_math_error(_error: WriterMathError) -> ProgramError {
    VaultError::WriterArithmeticAdmissionFailed.into()
}

pub(in crate::processor) fn writer_book_math_series(
    book: &WriterSeriesBookV1,
) -> Result<Vec<WriterSeries>, ProgramError> {
    if book.series_count == 0
        || usize::from(book.series_count) > crate::constants::WRITER_MAX_LIVE_SERIES
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut result = Vec::with_capacity(usize::from(book.series_count));
    for record in book.records.iter().take(usize::from(book.series_count)) {
        if !record.active {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
        result.push(WriterSeries {
            kind: record.option_kind,
            strike_price_atomic: record.strike_price_atomic,
            cap_price_atomic: record.cap_or_floor_price_atomic,
            contract_size_atoms: record.contract_size_atoms,
            max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
            external_oi_atoms: record.external_open_interest_atoms,
        });
    }
    Ok(result)
}

pub(in crate::processor) fn recompute_writer_metrics(
    sleeve: &mut WriterSleeveV1,
    book: &WriterSeriesBookV1,
    snapshot: &WriterPolicySnapshotV1,
    security_cap_atoms: Option<u64>,
    enforce_solvency_and_drawdown: bool,
) -> ProgramResult {
    if snapshot.policy_hash != sleeve.policy_hash
        || snapshot.policy_version != sleeve.policy_version
        || snapshot.series_family_hash != writer_series_family_hash(book)
        || snapshot.security_mode != sleeve.security_mode
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let count = usize::from(book.series_count);
    if count == 0 || count > crate::constants::WRITER_MAX_LIVE_SERIES {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut series = [WriterSeries::EMPTY; crate::constants::WRITER_SERIES_STORAGE_CAPACITY];
    for (index, record) in book.records.iter().take(count).enumerate() {
        if !record.active {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
        series[index] = WriterSeries {
            kind: record.option_kind,
            strike_price_atomic: record.strike_price_atomic,
            cap_price_atomic: record.cap_or_floor_price_atomic,
            contract_size_atoms: record.contract_size_atoms,
            max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
            external_oi_atoms: record.external_open_interest_atoms,
        };
    }
    let (reserve, exposure) = calculate_writer_metrics(
        &series[..count],
        snapshot,
        sleeve.security_mode,
        sleeve.accounted_asset_atoms,
        sleeve.locked_primary_premium_atoms,
        sleeve.writer_principal_atoms,
        security_cap_atoms,
        enforce_solvency_and_drawdown,
    )?;
    sleeve.exact_reserve_atoms = reserve.reserve_atoms;
    sleeve.lower_tail_reserve_atoms = reserve.lower_tail_reserve_atoms;
    sleeve.upper_tail_reserve_atoms = reserve.upper_tail_reserve_atoms;
    sleeve.security_exposure_atoms = exposure;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn calculate_writer_metrics(
    series: &[WriterSeries],
    snapshot: &WriterPolicySnapshotV1,
    security_mode: WriterSecurityMode,
    accounted_asset_atoms: u64,
    locked_primary_premium_atoms: u64,
    writer_principal_atoms: u64,
    security_cap_atoms: Option<u64>,
    enforce_solvency_and_drawdown: bool,
) -> Result<(crate::writer_sleeve_math::WriterReserveSummary, u64), ProgramError> {
    let reserve = exact_reserve(
        series,
        snapshot.lower_tail_max_settlement_atomic,
        snapshot.upper_tail_min_settlement_atomic,
    )
    .map_err(writer_math_error)?;
    let math_security_mode = match security_mode {
        WriterSecurityMode::GrossExternalMaxPayout => {
            WriterMathSecurityMode::GrossExternalMaximumPayout
        }
        WriterSecurityMode::ExactExternalEnvelope => WriterMathSecurityMode::ExactExternalEnvelope,
    };
    let exposure = calculate_security_exposure(math_security_mode, series, reserve.reserve_atoms)
        .map_err(writer_math_error)?;
    if security_cap_atoms.is_some_and(|cap| exposure > cap) {
        return Err(VaultError::WriterSecurityCapExceeded.into());
    }
    if enforce_solvency_and_drawdown {
        let required_assets = reserve
            .reserve_atoms
            .checked_add(snapshot.operational_buffer_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if accounted_asset_atoms < required_assets || writer_principal_atoms == 0 {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        let checks = drawdown_checks(
            &reserve,
            locked_primary_premium_atoms,
            writer_principal_atoms,
            snapshot.worst_drawdown_limit,
            snapshot.lower_drawdown_limit,
            snapshot.upper_drawdown_limit,
        )
        .map_err(writer_math_error)?;
        if !checks.all_pass() {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
    }
    Ok((reserve, exposure))
}

pub(super) fn writer_activation_assets_are_sufficient(
    sleeve: &WriterSleeveV1,
) -> Result<bool, ProgramError> {
    let principal_and_premium = sleeve
        .writer_principal_atoms
        .checked_add(sleeve.locked_primary_premium_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    // Before any direct Flat burn A == W + B. A holder-authorized burn decrements W and S but
    // leaves the forfeited settlement assets locked as derived writer surplus. That surplus must
    // not make an otherwise valid Funding sleeve impossible to activate.
    Ok(sleeve.accounted_asset_atoms >= principal_and_premium)
}
