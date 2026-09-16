//! Experimental wire codec only, not a new deployed account or Light domain.
//! Month/source must come from the authenticated enclosing access contract.
//! Values and timestamps retain all 64 bits; timestamp width adapts without a cap.

const COUNT: usize = 32;
const BODY: usize = 582;
const ALLOCATION: usize = 592;

fn number(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

struct WireCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> WireCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(len)?;
        let bytes = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(bytes)
    }

    fn read_u64(&mut self, width: usize) -> Option<u64> {
        if width > 8 {
            return None;
        }
        let bytes = self.take(width)?;
        let mut value = [0u8; 8];
        value[..width].copy_from_slice(bytes);
        Some(u64::from_le_bytes(value))
    }

    fn is_exhausted(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

fn required_width(delta: u64) -> usize {
    if delta == 0 {
        0
    } else {
        (64 - delta.leading_zeros()).div_ceil(8) as usize
    }
}

/// Decodes and checks the one canonical transport form directly into the complete logical
/// projection. The previous implementation reconstructed a logical Vec and called `pack` again;
/// this cursor performs the same canonicality checks while reading the wire bytes, so malformed
/// or non-minimal input does not allocate a second wire buffer.
fn decode_canonical(
    wire: &[u8],
    month: &[u8; 32],
    source: &[u8; 32],
    logical: &mut [u8; ALLOCATION],
) -> Option<()> {
    let count = usize::from(*wire.get(6)?);
    let width = usize::from(*wire.get(7)?);
    if count > COUNT || width > 8 || (count <= 2 && width != 0) || (count > 2 && width == 0) {
        return None;
    }
    let expected = 8 + count * 8 + count.min(2) * 8 + count.saturating_sub(2) * width;
    if wire.len() != expected {
        return None;
    }

    logical.fill(0);
    logical[..6].copy_from_slice(&wire[..6]);
    logical[6..38].copy_from_slice(month);
    logical[38..70].copy_from_slice(source);

    let mut cursor = WireCursor::new(&wire[8..]);
    for i in 0..count {
        let state = cursor.read_u64(8)?;
        if state == 0 {
            return None;
        }
        logical[70 + i * 8..78 + i * 8].copy_from_slice(&state.to_le_bytes());
    }

    let mut previous = 0u64;
    for i in 0..count.min(2) {
        let time = cursor.read_u64(8)?;
        if time == 0 || time <= previous {
            return None;
        }
        logical[326 + i * 8..334 + i * 8].copy_from_slice(&time.to_le_bytes());
        previous = time;
    }

    let base = if count > 1 { previous } else { 0 };
    let mut last_delta = 0u64;
    for i in 2..count {
        let delta = cursor.read_u64(width)?;
        let time = base.checked_add(delta)?;
        if time == 0 || time <= previous {
            return None;
        }
        logical[326 + i * 8..334 + i * 8].copy_from_slice(&time.to_le_bytes());
        previous = time;
        last_delta = delta;
    }
    if !cursor.is_exhausted() || (count > 2 && required_width(last_delta) != width) {
        return None;
    }
    Some(())
}

pub fn pack(logical: &[u8]) -> Option<Vec<u8>> {
    if logical.len() != ALLOCATION || logical[BODY..].iter().any(|b| *b != 0) {
        return None;
    }
    let mut count: usize = 0;
    let mut previous = 0;
    let mut empty = false;
    for i in 0..COUNT {
        let state = number(logical, 70 + i * 8)?;
        let time = number(logical, 326 + i * 8)?;
        if state == 0 && time == 0 {
            empty = true;
        } else {
            if empty || state == 0 || time == 0 || time <= previous {
                return None;
            }
            previous = time;
            count += 1;
        }
    }
    let base = if count > 1 { number(logical, 334)? } else { 0 };
    let delta = if count > 2 {
        previous.checked_sub(base)?
    } else {
        0
    };
    let width = if delta == 0 {
        0
    } else {
        (64 - delta.leading_zeros()).div_ceil(8) as usize
    };
    let mut wire =
        Vec::with_capacity(8 + count * 8 + count.min(2) * 8 + count.saturating_sub(2) * width);
    wire.extend_from_slice(&logical[..6]);
    wire.push(count as u8);
    wire.push(width as u8);
    wire.extend_from_slice(&logical[70..70 + count * 8]);
    wire.extend_from_slice(&logical[326..326 + count.min(2) * 8]);
    for i in 2..count {
        let delta = number(logical, 326 + i * 8)?.checked_sub(base)?;
        wire.extend_from_slice(&delta.to_le_bytes()[..width]);
    }
    Some(wire)
}

/// Packs the 582-byte compact projection used by the compressed-state leaf envelope.
///
/// Month and source are authenticated context fields in this projection. They are intentionally
/// omitted from the wire and rebound by the program after the source PDA has been checked.
pub fn pack_compact(compact: &[u8]) -> Option<Vec<u8>> {
    if compact.len() != BODY {
        return None;
    }
    let mut logical = vec![0; ALLOCATION];
    logical[..BODY].copy_from_slice(compact);
    pack(&logical)
}

/// Unpacks a transport value into the canonical 582-byte projection with zero identity
/// placeholders. The caller must bind month and source from authenticated current accounts before
/// validating or materializing the leaf.
pub fn unpack_compact(wire: &[u8]) -> Option<Vec<u8>> {
    unpack_compact_with_context(wire, &[0; 32], &[0; 32])
}

/// Unpacks a transport value into the canonical 582-byte projection with an authenticated month
/// and source context.
pub fn unpack_compact_with_context(
    wire: &[u8],
    month: &[u8; 32],
    source: &[u8; 32],
) -> Option<Vec<u8>> {
    let mut logical = [0u8; ALLOCATION];
    decode_canonical(wire, month, source, &mut logical)?;
    Some(logical[..BODY].to_vec())
}
