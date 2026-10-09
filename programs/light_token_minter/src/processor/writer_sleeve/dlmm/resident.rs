//! Writer rows retain their logical PDA after import into the live Market.
use super::*;
use crate::market_router::{self, FORWARDED_WRITER_VERSION};

pub(super) fn resolve_position(
    program: &Pubkey,
    info: &AccountInfo,
    market: &AccountInfo,
    external: WriterDlmmPositionV1,
) -> Result<WriterDlmmPositionV1, ProgramError> {
    let row = market_router::load(program, market)?.and_then(|s| s.writer_position);
    resolve_position_with(program, info, market.key, row, external)
}

/// `resolve_position` against the writer row of `market`'s already loaded
/// resident state (`None`: no row, or no resident state). A forwarded marker
/// resolves only to that row; an ordinary position only while there is none.
pub(in crate::processor) fn resolve_position_with(
    program: &Pubkey,
    info: &AccountInfo,
    market: &Pubkey,
    row: Option<WriterDlmmPositionV1>,
    mut external: WriterDlmmPositionV1,
) -> Result<WriterDlmmPositionV1, ProgramError> {
    if external.account_version != FORWARDED_WRITER_VERSION {
        if row.is_some() {
            return Err(VaultError::InvalidWriterSleeve.into());
        }
        return Ok(external);
    }
    external.account_version = WriterDlmmPositionV1::ACCOUNT_VERSION;
    let value = row.ok_or(VaultError::InvalidWriterSleeve)?;
    let (address, bump) = derive_writer_dlmm_position_pda(program, &value.pool, &value.sleeve);
    if !external.is_initialized
        || !external.has_current_layout()
        || *info.key != address
        || external.bump != bump
        || external.pool != value.pool
        || external.sleeve != value.sleeve
        || external.policy != value.policy
        || external.market != *market
        || external.market != value.market
        || external.series_index != value.series_index
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(value)
}

pub(super) fn position_is_resident(
    program: &Pubkey,
    market: &AccountInfo,
    position: &WriterDlmmPositionV1,
) -> Result<bool, ProgramError> {
    let Some(state) = market_router::load(program, market)? else {
        return Ok(false);
    };
    let Some(stored) = state.writer_position else {
        return Ok(false);
    };
    if stored.pool != position.pool
        || stored.sleeve != position.sleeve
        || stored.policy != position.policy
        || stored.market != position.market
        || stored.bump != position.bump
        || stored.series_index != position.series_index
    {
        return Err(VaultError::InvalidWriterSleeve.into());
    }
    Ok(true)
}

pub(super) fn store_position(
    program: &Pubkey,
    info: &AccountInfo,
    market: &AccountInfo,
    position: &WriterDlmmPositionV1,
) -> ProgramResult {
    if position_is_resident(program, market, position)? {
        let mut state =
            market_router::load(program, market)?.ok_or(VaultError::InvalidWriterSleeve)?;
        state.writer_position = Some(*position);
        market_router::store(market, &state)
    } else {
        store_state(info, position)
    }
}
