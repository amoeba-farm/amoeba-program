//! `BuybackMarkV1`: the create-only per-sleeve price history of the Earn Fund
//! buy-back mark, read and written in place (never decoded whole).
//!
//! Layout version 2 (two-sided pricing; little-endian), `n = series_count ≤
//! 18`, length `336 + 424 n`:
//!
//! | Off | Field |
//! |---:|---|
//! | 0 | initialized u8, bump u8, discriminator `BBM`, version u8 |
//! | 6 | sleeve |
//! | 38 | series book |
//! | 70 | writer DLMM policy |
//! | 102 | rent payer |
//! | 134 | series_count u8 |
//! | 135 | head_all_viable u8 |
//! | 136 | created_ts u64 |
//! | 144 | head_ts u64 |
//! | 152 | head_slot u64 |
//! | 160 | head_book_slot u64 (the book's `last_updated_slot` at the head) |
//! | 168 | head_liability u64 (`L`) |
//! | 176 | spend_month_start_ts u64 (policy month the spend counters belong to) |
//! | 184 | last_bucket u64 |
//! | 192 | sample_count u64 |
//! | 200 | head_upper_liability u64 (`L_hi ≤ L`, the entry-side liability) |
//! | 208 + 104 i | series i: pool, writer position, cum_premium u64, cum_spend u64 (turnover counters already counted, whole USD), head_price u64 (`B_i`), head_bid_price u64 (`b_i`), viable u8, valid_buckets u8, 6 zero bytes |
//! | 208 + 104 n + (4 + 10 n) k | ring bucket k (slot `index mod 32`): index u32, then per series max_ask_bin u16 (`0xFFFF` = no valid ask), min_bid_bin u16 (`0` = no executable bid), flags u8 (bit0 sampled, bit1 every sample valid), zero u8, sale_usd u16, buy_usd u16 |
use crate::buyback_mark_math::{BucketEntry, MAX_MARK_SERIES, RING_BUCKETS};
use crate::constants::CURRENT_STATE_NAMESPACE_SEED;
use solana_program::pubkey::Pubkey;

pub const BUYBACK_MARK_SEED: &[u8] = b"buyback-mark-v1";
pub const BUYBACK_MARK_DISCRIMINATOR: [u8; 3] = *b"BBM";
pub const BUYBACK_MARK_VERSION: u8 = 2;
pub const MARK_HEADER_LEN: usize = 208;
pub const MARK_SERIES_LEN: usize = 104;
/// Bytes per series in one ring bucket.
pub const MARK_CELL_LEN: usize = 10;

pub const OFF_SLEEVE: usize = 6;
pub const OFF_BOOK: usize = 38;
pub const OFF_POLICY: usize = 70;
pub const OFF_RENT_PAYER: usize = 102;
pub const OFF_SERIES_COUNT: usize = 134;
pub const OFF_HEAD_ALL_VIABLE: usize = 135;
pub const OFF_CREATED_TS: usize = 136;
pub const OFF_HEAD_TS: usize = 144;
pub const OFF_HEAD_SLOT: usize = 152;
pub const OFF_HEAD_BOOK_SLOT: usize = 160;
pub const OFF_HEAD_LIABILITY: usize = 168;
pub const OFF_SPEND_MONTH: usize = 176;
pub const OFF_LAST_BUCKET: usize = 184;
pub const OFF_SAMPLE_COUNT: usize = 192;
pub const OFF_HEAD_UPPER_LIABILITY: usize = 200;

pub const S_POOL: usize = 0;
pub const S_POSITION: usize = 32;
pub const S_CUM_PREMIUM: usize = 64;
pub const S_CUM_SPEND: usize = 72;
pub const S_HEAD_PRICE: usize = 80;
pub const S_HEAD_BID_PRICE: usize = 88;
pub const S_VIABLE: usize = 96;
pub const S_VALID_BUCKETS: usize = 97;

/// Separate unpublished working copy; existing layout-2 marks remain unchanged.
pub const BUYBACK_ROUND_SEED: &[u8] = b"buyback-mark-round-v1";
pub const ROUND_HEADER_LEN: usize = 128;
pub const ROUND_COMMITMENT: usize = 70;
pub const ROUND_TS: usize = 102;
pub const ROUND_SLOT: usize = 110;
pub const ROUND_NEXT: usize = 118;
pub const ROUND_PENDING: usize = 119;
pub const ROUND_COUNT: usize = 120;
pub const ROUND_FRESH: usize = 121;

pub const fn round_mark_offset(count: usize) -> usize {
    ROUND_HEADER_LEN + 64 * count
}

pub const fn round_len(count: usize) -> usize {
    round_mark_offset(count) + mark_len(count)
}

pub fn derive_buyback_round(program: &Pubkey, sleeve: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            BUYBACK_ROUND_SEED,
            sleeve.as_ref(),
        ],
        program,
    )
}

pub fn valid_round(program: &Pubkey, key: &Pubkey, sleeve: &Pubkey, data: &[u8]) -> Option<usize> {
    let count = usize::from(*data.get(ROUND_COUNT)?);
    let (expected, bump) = derive_buyback_round(program, sleeve);
    (count > 0
        && count <= MAX_MARK_SERIES
        && data.len() == round_len(count)
        && data[0] == 1
        && data[1] == bump
        && data[2..5] == *b"BBR"
        && data[5] == 1
        && get_key(data, 6) == *sleeve
        && *key == expected
        && data[ROUND_NEXT] <= count as u8
        && data[ROUND_PENDING] <= 1)
        .then_some(count)
}

pub const fn mark_len(series: usize) -> usize {
    MARK_HEADER_LEN + MARK_SERIES_LEN * series + RING_BUCKETS * (4 + MARK_CELL_LEN * series)
}

pub fn derive_buyback_mark(program: &Pubkey, sleeve: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            BUYBACK_MARK_SEED,
            sleeve.as_ref(),
        ],
        program,
    )
}

pub const fn series_offset(index: usize) -> usize {
    MARK_HEADER_LEN + MARK_SERIES_LEN * index
}

pub const fn bucket_offset(series: usize, slot: usize) -> usize {
    MARK_HEADER_LEN + MARK_SERIES_LEN * series + (4 + MARK_CELL_LEN * series) * slot
}

#[inline(never)]
pub fn get_u16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

#[inline(never)]
pub fn get_u64(data: &[u8], at: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&data[at..at + 8]);
    u64::from_le_bytes(bytes)
}

#[inline(never)]
pub fn get_key(data: &[u8], at: usize) -> Pubkey {
    let mut bytes = [0; 32];
    bytes.copy_from_slice(&data[at..at + 32]);
    Pubkey::new_from_array(bytes)
}

#[inline(never)]
pub fn put_u16(data: &mut [u8], at: usize, value: u16) {
    data[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

#[inline(never)]
pub fn put_u64(data: &mut [u8], at: usize, value: u64) {
    data[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

#[inline(never)]
pub fn put_key(data: &mut [u8], at: usize, value: &Pubkey) {
    data[at..at + 32].copy_from_slice(value.as_ref());
}

/// The series count of the canonical mark of `sleeve` at `key`: valid
/// header and length, naming `sleeve`, stored at `PDA(sleeve)` with its bump.
pub fn valid_mark(program: &Pubkey, key: &Pubkey, sleeve: &Pubkey, data: &[u8]) -> Option<usize> {
    let count = valid_mark_series(data)?;
    (get_key(data, OFF_SLEEVE) == *sleeve
        && Pubkey::create_program_address(
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                BUYBACK_MARK_SEED,
                sleeve.as_ref(),
                &[data[1]],
            ],
            program,
        ) == Ok(*key))
    .then_some(count)
}

/// The series count of a mark whose header and exact length are valid.
pub fn valid_mark_series(data: &[u8]) -> Option<usize> {
    let count = usize::from(*data.get(OFF_SERIES_COUNT)?);
    (data.len() == mark_len(count)
        && count != 0
        && count <= MAX_MARK_SERIES
        && data[0] == 1
        && data[2..5] == BUYBACK_MARK_DISCRIMINATOR
        && data[5] == BUYBACK_MARK_VERSION)
        .then_some(count)
}

/// Ring entry `slot` of `series` (`count` series in the mark).
#[inline(never)]
pub fn read_bucket(data: &[u8], count: usize, slot: usize, series: usize) -> BucketEntry {
    let base = bucket_offset(count, slot);
    let mut index = [0; 4];
    index.copy_from_slice(&data[base..base + 4]);
    let at = base + 4 + MARK_CELL_LEN * series;
    let mut cell = [0; MARK_CELL_LEN];
    cell.copy_from_slice(&data[at..at + MARK_CELL_LEN]);
    BucketEntry {
        index: u64::from(u32::from_le_bytes(index)),
        max_ask_bin: u16::from_le_bytes([cell[0], cell[1]]),
        min_bid_bin: u16::from_le_bytes([cell[2], cell[3]]),
        flags: cell[4],
        sale_usd: u16::from_le_bytes([cell[6], cell[7]]),
        buy_usd: u16::from_le_bytes([cell[8], cell[9]]),
    }
}

/// Store a ring entry; the bucket index is shared by every series.
#[inline(never)]
pub fn write_bucket(
    data: &mut [u8],
    count: usize,
    slot: usize,
    series: usize,
    entry: &BucketEntry,
) {
    let base = bucket_offset(count, slot);
    let index = u32::try_from(entry.index).unwrap_or(u32::MAX);
    data[base..base + 4].copy_from_slice(&index.to_le_bytes());
    let [ask_lo, ask_hi] = entry.max_ask_bin.to_le_bytes();
    let [bid_lo, bid_hi] = entry.min_bid_bin.to_le_bytes();
    let [sale_lo, sale_hi] = entry.sale_usd.to_le_bytes();
    let [buy_lo, buy_hi] = entry.buy_usd.to_le_bytes();
    let at = base + 4 + MARK_CELL_LEN * series;
    data[at..at + MARK_CELL_LEN].copy_from_slice(&[
        ask_lo,
        ask_hi,
        bid_lo,
        bid_hi,
        entry.flags,
        0,
        sale_lo,
        sale_hi,
        buy_lo,
        buy_hi,
    ]);
}
