use super::{FixedCursor, FixedField, FixedStateDecode, FixedStateEncode, FixedWriter};
use crate::capped_strip::{Lane, Row, LEN};
use solana_program::pubkey::Pubkey;

impl FixedStateDecode for Lane {
    const REQUIRED_DATA_LEN: usize = LEN;

    #[inline(never)]
    unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
        let mut input = FixedCursor::new(data);
        let value = Self {
            magic: <[u8; 8]>::read(&mut input),
            version: u8::read(&mut input),
            bump: u8::read(&mut input),
            sleeve: Pubkey::read(&mut input),
            group: Pubkey::read(&mut input),
            book: Pubkey::read(&mut input),
            policy_hash: <[u8; 32]>::read(&mut input),
            series_count: u8::read(&mut input),
            rows: core::array::from_fn(|_| Row {
                bin_count: u8::read(&mut input),
                bins: core::array::from_fn(|_| crate::state::WriterDlmmBinV1::read(&mut input)),
            }),
            last_updated_slot: u64::read(&mut input),
        };
        input.finish_borsh()?;
        Ok(value)
    }
}

impl FixedStateEncode for Lane {
    fn maximum_encoded_len(&self) -> usize {
        LEN
    }

    #[inline(never)]
    fn encode_fixed(&self, data: &mut [u8]) {
        let mut output = FixedWriter::new(data);
        self.magic.write(&mut output);
        self.version.write(&mut output);
        self.bump.write(&mut output);
        self.sleeve.write(&mut output);
        self.group.write(&mut output);
        self.book.write(&mut output);
        self.policy_hash.write(&mut output);
        self.series_count.write(&mut output);
        for row in &self.rows {
            row.bin_count.write(&mut output);
            for bin in &row.bins {
                bin.write(&mut output);
            }
        }
        self.last_updated_slot.write(&mut output);
        debug_assert_eq!(output.offset, LEN);
    }
}
