//! Preserve the cache-sizing log protocol without pulling in general formatting.

fn decimal(mut value: u64, bytes: &mut [u8; 20]) -> &[u8] {
    let mut start = bytes.len();
    loop {
        start -= 1;
        bytes[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            return &bytes[start..];
        }
    }
}

pub(super) fn log(prefix: &[u8], value: u64) {
    let mut digits = [0; 20];
    let digits = decimal(value, &mut digits);
    let mut message = [0; 64];
    let end = prefix.len() + digits.len();
    message[..prefix.len()].copy_from_slice(prefix);
    message[prefix.len()..end].copy_from_slice(digits);
    // Both private callers supply ASCII literals; decimal emits only ASCII digits.
    let message = unsafe { core::str::from_utf8_unchecked(&message[..end]) };
    solana_program::log::sol_log(message);
}
