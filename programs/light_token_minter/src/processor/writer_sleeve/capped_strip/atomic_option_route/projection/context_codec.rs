//! Private projection-only serializers. The action-local source transcript
//! authenticates `before`; strict source loaders have decoded canonical bytes.
//! Both writer projectors mutate only selected series rows. All other mutable
//! headers/totals are written completely, and ordinary codecs remain unchanged.
use crate::{
    capped_strip::{Lane, LEN as LANE_LEN, SERIES},
    compact_error::{ProgramError, ProgramResult},
    fixed_codec::{FixedField, FixedStateEncode, FixedWriter},
    state::WriterSeriesBookV1,
};

pub(super) type Selected = [bool; SERIES];
const BOOK_HEADER: usize = 120;
const BOOK_ROW: usize = 256;
const BOOK_TOTALS: usize = BOOK_HEADER + SERIES * BOOK_ROW;
const LANE_HEADER: usize = 139;
const LANE_ROW: usize = 145;

fn copy_source(before: &[u8], len: usize, out: &mut Vec<u8>) -> ProgramResult {
    if before.len() != len || !out.is_empty() {
        return Err(ProgramError::InvalidAccountData);
    }
    out.extend_from_slice(before);
    Ok(())
}

pub(super) fn encode_book(
    state: &WriterSeriesBookV1,
    before: &[u8],
    selected: &Selected,
    out: &mut Vec<u8>,
) -> ProgramResult {
    copy_source(before, WriterSeriesBookV1::LEN, out)?;
    let mut header = FixedWriter::new(&mut out[..BOOK_HEADER]);
    state.is_initialized.write(&mut header);
    state.bump.write(&mut header);
    state.account_discriminator.write(&mut header);
    state.account_version.write(&mut header);
    state.sleeve.write(&mut header);
    state.settlement_group.write(&mut header);
    state.series_count.write(&mut header);
    state.max_series.write(&mut header);
    state.frozen.write(&mut header);
    state.reserved.write(&mut header);
    state.book_digest.write(&mut header);
    state.last_updated_slot.write(&mut header);
    debug_assert_eq!(header.offset, BOOK_HEADER);
    for (index, touched) in selected.iter().enumerate() {
        if *touched {
            let start = BOOK_HEADER + index * BOOK_ROW;
            let mut row = FixedWriter::new(&mut out[start..start + BOOK_ROW]);
            state.records[index].write(&mut row);
            debug_assert_eq!(row.offset, BOOK_ROW);
        }
    }
    // This includes all individual writer obligations, settlement fields,
    // arrays and reserved bytes; only whole unselected series records copy.
    state.individual.encode_fixed(&mut out[BOOK_TOTALS..]);
    Ok(())
}

pub(super) fn encode_lane(
    state: &Lane,
    before: &[u8],
    selected: &Selected,
    out: &mut Vec<u8>,
) -> ProgramResult {
    copy_source(before, LANE_LEN, out)?;
    let mut header = FixedWriter::new(&mut out[..LANE_HEADER]);
    state.magic.write(&mut header);
    state.version.write(&mut header);
    state.bump.write(&mut header);
    state.sleeve.write(&mut header);
    state.group.write(&mut header);
    state.book.write(&mut header);
    state.policy_hash.write(&mut header);
    state.series_count.write(&mut header);
    debug_assert_eq!(header.offset, LANE_HEADER);
    for (index, touched) in selected.iter().enumerate() {
        if *touched {
            let start = LANE_HEADER + index * LANE_ROW;
            let mut row = FixedWriter::new(&mut out[start..start + LANE_ROW]);
            state.rows[index].bin_count.write(&mut row);
            for bin in &state.rows[index].bins {
                bin.write(&mut row);
            }
            debug_assert_eq!(row.offset, LANE_ROW);
        }
    }
    out[LANE_LEN - 8..].copy_from_slice(&state.last_updated_slot.to_le_bytes());
    Ok(())
}
