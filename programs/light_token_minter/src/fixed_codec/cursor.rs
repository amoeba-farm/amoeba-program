use super::*;

/// Reads one rigid Borsh field from an instruction or variable-length leaf without routing
/// through `std::io::Read`. The exact same bytes are consumed, while short inputs use the
/// program's uniform invalid-data error instead of allocating a formatted EOF message.
#[inline(never)]
pub(crate) fn deserialize_slice_bytes<const LENGTH: usize>(
    data: &mut &[u8],
) -> std::io::Result<[u8; LENGTH]> {
    if data.len() < LENGTH {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut value = [0; LENGTH];
    // SAFETY: The length check proves both source and fixed destination cover `LENGTH` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), value.as_mut_ptr(), LENGTH);
        *data = std::slice::from_raw_parts(data.as_ptr().add(LENGTH), data.len() - LENGTH);
    }
    Ok(value)
}

/// Checked cursor for bounded variable-length Borsh layouts. Unlike `FixedCursor`, this cursor
/// may be constructed over any slice: the first malformed field sticks an invalid bit and all
/// following reads become harmless zero values. That keeps the runtime codec compact while the
/// final check preserves ordinary Borsh rejection semantics.
pub(crate) struct CheckedCursor<'a> {
    remaining: &'a [u8],
    pub(crate) invalid: bool,
}

impl<'a> CheckedCursor<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            remaining: data,
            invalid: false,
        }
    }

    #[inline(never)]
    pub(crate) fn bytes<const LENGTH: usize>(&mut self) -> [u8; LENGTH] {
        if self.invalid || self.remaining.len() < LENGTH {
            self.invalid = true;
            return [0; LENGTH];
        }
        let mut value = [0; LENGTH];
        // SAFETY: The length check proves both buffers cover `LENGTH` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(self.remaining.as_ptr(), value.as_mut_ptr(), LENGTH);
            self.remaining = std::slice::from_raw_parts(
                self.remaining.as_ptr().add(LENGTH),
                self.remaining.len() - LENGTH,
            );
        }
        value
    }

    #[inline(always)]
    pub(crate) fn u8(&mut self) -> u8 {
        self.bytes::<1>()[0]
    }

    #[inline(always)]
    pub(crate) fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn pubkey(&mut self) -> Pubkey {
        Pubkey::new_from_array(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn boolean(&mut self) -> bool {
        match self.u8() {
            0 => false,
            1 => true,
            _ => {
                self.invalid = true;
                false
            }
        }
    }

    #[inline(never)]
    pub(crate) fn slice(&mut self, length: usize) -> &'a [u8] {
        if self.invalid || self.remaining.len() < length {
            self.invalid = true;
            return &[];
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        value
    }

    #[inline(never)]
    pub(crate) fn vec(&mut self, length: usize) -> Vec<u8> {
        self.slice(length).to_vec()
    }

    #[inline(never)]
    pub(crate) fn bounded_string(&mut self, maximum: usize) -> String {
        let length = self.u32() as usize;
        if length > maximum {
            self.invalid = true;
            return String::new();
        }
        match String::from_utf8(self.vec(length)) {
            Ok(value) => value,
            Err(_) => {
                self.invalid = true;
                String::new()
            }
        }
    }

    #[inline(always)]
    pub(crate) fn bounded_count(&mut self, maximum: usize) -> usize {
        let count = self.u32() as usize;
        if count > maximum {
            self.invalid = true;
            0
        } else {
            count
        }
    }

    #[inline(always)]
    pub(crate) fn remaining_len(&self) -> usize {
        self.remaining.len()
    }

    pub(crate) fn finish_exact(self) -> std::io::Result<()> {
        if self.invalid || !self.remaining.is_empty() {
            Err(std::io::ErrorKind::InvalidData.into())
        } else {
            Ok(())
        }
    }

    pub(crate) fn finish(self) -> std::io::Result<&'a [u8]> {
        if self.invalid {
            Err(std::io::ErrorKind::InvalidData.into())
        } else {
            Ok(self.remaining)
        }
    }
}

/// A prechecked cursor for rigid program-account layouts. Callers must verify the exact account
/// allocation before constructing it. Each decoder below consumes no more than that allocation;
/// optional Borsh fields may consume less, and the common finish check rejects nonzero padding.
pub(crate) struct FixedCursor<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) offset: usize,
    pub(crate) invalid: bool,
}

pub(crate) struct FixedWriter<'a> {
    data: &'a mut [u8],
    pub(crate) offset: usize,
}

impl<'a> FixedWriter<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a mut [u8]) -> Self {
        Self { data, offset: 0 }
    }

    #[inline(always)]
    pub(crate) fn bytes<const LENGTH: usize>(&mut self, value: &[u8; LENGTH]) {
        // SAFETY: The generated codec's compile-time buffer is exactly the sum of its fields.
        unsafe {
            std::ptr::copy_nonoverlapping(
                value.as_ptr(),
                self.data.as_mut_ptr().add(self.offset),
                LENGTH,
            );
        }
        self.offset += LENGTH;
    }

    #[inline(never)]
    pub(crate) fn raw(&mut self, value: &[u8]) {
        let end = self.offset + value.len();
        self.data[self.offset..end].copy_from_slice(value);
        self.offset = end;
    }

    #[inline(never)]
    pub(crate) fn bytes32(&mut self, value: &[u8; 32]) {
        self.bytes(value);
    }

    #[inline(always)]
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    #[inline(always)]
    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    #[inline(always)]
    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    #[inline(always)]
    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    #[inline(always)]
    pub(crate) fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    #[inline(always)]
    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    #[inline(always)]
    pub(crate) fn u128(&mut self, value: u128) {
        self.bytes(&value.to_le_bytes());
    }

    #[inline(always)]
    pub(crate) fn i64(&mut self, value: i64) {
        self.bytes(&value.to_le_bytes());
    }

    #[inline(never)]
    pub(crate) fn pubkey(&mut self, value: &Pubkey) {
        self.bytes(&value.to_bytes());
    }

    #[inline(always)]
    pub(crate) fn optional_pubkey(&mut self, value: &Option<Pubkey>) {
        match value {
            None => self.u8(0),
            Some(key) => {
                self.u8(1);
                self.pubkey(key);
            }
        }
    }
}

impl<'a> FixedCursor<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            invalid: false,
        }
    }

    #[inline(always)]
    pub(crate) fn bytes<const LENGTH: usize>(&mut self) -> [u8; LENGTH] {
        let mut value = [0u8; LENGTH];
        // SAFETY: Every public decoder first requires the exact fixed account allocation. Its
        // field sequence is the Borsh layout for that account and its maximum consumed length is
        // covered by a golden maximum-layout test. No field value can increase consumption beyond
        // that maximum; options only consume fewer bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.data.as_ptr().add(self.offset),
                value.as_mut_ptr(),
                LENGTH,
            );
        }
        self.offset += LENGTH;
        value
    }

    #[inline(always)]
    pub(crate) fn u8(&mut self) -> u8 {
        self.bytes::<1>()[0]
    }

    #[inline(always)]
    pub(crate) fn bool(&mut self) -> bool {
        match self.u8() {
            0 => false,
            1 => true,
            _ => {
                self.invalid = true;
                false
            }
        }
    }

    #[inline(always)]
    pub(crate) fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn u128(&mut self) -> u128 {
        u128::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn pubkey(&mut self) -> Pubkey {
        Pubkey::new_from_array(self.bytes())
    }

    #[inline(always)]
    pub(crate) fn optional_pubkey(&mut self) -> Option<Pubkey> {
        match self.u8() {
            0 => None,
            1 => Some(self.pubkey()),
            _ => {
                self.invalid = true;
                None
            }
        }
    }

    #[inline(never)]
    pub(crate) fn finish_borsh(self) -> std::io::Result<()> {
        if self.invalid
            || self.offset > self.data.len()
            || self.data[self.offset..].iter().any(|byte| *byte != 0)
        {
            Err(invalid_fixed_borsh())
        } else {
            Ok(())
        }
    }
}
