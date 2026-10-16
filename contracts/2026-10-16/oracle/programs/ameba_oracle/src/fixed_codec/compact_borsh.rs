//! Compact on-chain Borsh decoding for fixed-shape types.
//!
//! `#[derive(BorshDeserialize)]` decodes field by field through `std::io::Read`; every field
//! returns an `io::Result` whose value is then copied out, which made derived decoders one of the
//! larger code-size costs in the SBF artifact. [`compact_borsh!`] defines a type together with:
//!
//! * a `deserialize_reader` that is token-for-token what the derive would generate (it remains
//!   the generic path for off-chain callers and is the reference in tests), and
//! * a `deserialize` override for byte slices that reads the same fields, in the same order, from
//!   a [`CheckedCursor`]. Short input, a bool outside `0|1`, an option tag outside `0|1` or an
//!   unknown enum tag poisons the cursor exactly where Borsh would return an error.
//!
//! `try_from_slice` and `deserialize` (what the program uses) therefore accept exactly the same
//! byte strings, consume the same bytes and produce the same values as the derived decoder.

// Later instruction families decode their variable payloads through these readers; the
// differential harness below already covers the shared machinery.
#![allow(dead_code, unused_macros)]

use super::*;

/// One Borsh field decoded from a [`CheckedCursor`]; malformed input marks the cursor invalid.
pub(crate) trait CursorField: Sized {
    fn read(input: &mut CheckedCursor<'_>) -> Self;
}

macro_rules! primitive_cursor_field {
    ($($type:ty => $reader:ident),+ $(,)?) => {
        $(impl CursorField for $type {
            #[inline(always)]
            fn read(input: &mut CheckedCursor<'_>) -> Self {
                input.$reader()
            }
        })+
    };
}

primitive_cursor_field!(u8 => u8, u16 => u16, u32 => u32, u64 => u64, i64 => i64, bool => boolean);

impl CursorField for u128 {
    #[inline(always)]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        u128::from_le_bytes(input.bytes())
    }
}

impl CursorField for i128 {
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        i128::from_le_bytes(input.bytes())
    }
}

impl<const LENGTH: usize> CursorField for [u8; LENGTH] {
    #[inline(always)]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        input.bytes()
    }
}

impl CursorField for Pubkey {
    #[inline(always)]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        input.pubkey()
    }
}

impl<T: CursorField> CursorField for Option<T> {
    #[inline]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        match input.u8() {
            0 => None,
            1 => Some(T::read(input)),
            _ => {
                input.invalid = true;
                None
            }
        }
    }
}

/// Element types decoded one by one inside a `Vec` (everything except `u8`, whose vectors are
/// read as one byte slice, exactly as Borsh's `vec_from_reader` specialization does).
pub(crate) trait CursorVecElement: CursorField {}
impl<const LENGTH: usize> CursorVecElement for [u8; LENGTH] {}
impl CursorVecElement for Pubkey {}
impl CursorVecElement for u16 {}
impl CursorVecElement for u32 {}
impl CursorVecElement for u64 {}

impl CursorField for Vec<u8> {
    #[inline(never)]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        let len = input.u32() as usize;
        input.vec(len)
    }
}

impl<T: CursorVecElement> CursorField for Vec<T> {
    /// Borsh's `Vec<T>`: `u32` length, then elements; the initial capacity is Borsh's
    /// `hint::cautious` and only completely decoded elements are pushed.
    #[inline(never)]
    fn read(input: &mut CheckedCursor<'_>) -> Self {
        let len = input.u32();
        if input.invalid || len == 0 {
            return Vec::new();
        }
        let element_size = core::mem::size_of::<T>() as u32;
        let capacity = core::cmp::max(core::cmp::min(len, 4096 / element_size), 1) as usize;
        let mut values = Vec::with_capacity(capacity);
        for _ in 0..len {
            let value = T::read(input);
            if input.invalid {
                break;
            }
            values.push(value);
        }
        values
    }
}

/// Decode exactly one value occupying all of `data` (Borsh `try_from_slice` semantics).
#[inline(never)]
pub(crate) fn cursor_from_slice<T: CursorField>(data: &[u8]) -> std::io::Result<T> {
    let mut cursor = CheckedCursor::new(data);
    let value = T::read(&mut cursor);
    cursor.finish_exact()?;
    Ok(value)
}

/// Decode one value from `data` with the cursor, advancing `data` past the consumed bytes.
#[inline(always)]
pub(crate) fn cursor_deserialize<T: CursorField>(data: &mut &[u8]) -> std::io::Result<T> {
    let mut cursor = CheckedCursor::new(data);
    let value = T::read(&mut cursor);
    *data = cursor.finish()?;
    Ok(value)
}

/// Define a struct whose slice decoding uses [`CursorField`]; see the module documentation.
/// Every field type must implement both `BorshDeserialize` and [`CursorField`].
#[allow(unused_macros)]
macro_rules! compact_borsh_struct {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($(#[$field_meta:meta])* $field_vis:vis $field:ident: $type:ty),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $($(#[$field_meta])* $field_vis $field: $type,)+
        }

        impl $crate::fixed_codec::CursorVecElement for $name {}

        impl $crate::fixed_codec::CursorField for $name {
            #[inline(never)]
            fn read(input: &mut $crate::fixed_codec::CheckedCursor<'_>) -> Self {
                Self {
                    $($field: <$type as $crate::fixed_codec::CursorField>::read(input),)+
                }
            }
        }

        impl ::borsh::BorshDeserialize for $name {
            fn deserialize_reader<R: ::std::io::Read>(reader: &mut R) -> ::std::io::Result<Self> {
                Ok(Self {
                    $($field: ::borsh::BorshDeserialize::deserialize_reader(reader)?,)+
                })
            }

            #[inline]
            fn deserialize(buf: &mut &[u8]) -> ::std::io::Result<Self> {
                $crate::fixed_codec::cursor_deserialize(buf)
            }

            /// Borsh's rule: decode, then reject any unread byte.
            #[inline]
            fn try_from_slice(data: &[u8]) -> ::std::io::Result<Self> {
                $crate::fixed_codec::cursor_from_slice(data)
            }
        }
    };
}
pub(crate) use compact_borsh_struct;

/// Differential checks between the compact slice decoder and the derive-equivalent reader.
#[cfg(test)]
pub(crate) mod equivalence {
    use borsh::{BorshDeserialize, BorshSerialize};

    /// What `#[derive(BorshDeserialize)]` plus Borsh's default `try_from_slice` did.
    fn reference_exact<T: BorshDeserialize>(data: &[u8]) -> Option<T> {
        let mut rest = data;
        let value = T::deserialize_reader(&mut rest).ok()?;
        rest.is_empty().then_some(value)
    }

    fn reference_prefix<T: BorshDeserialize>(data: &[u8]) -> Option<(T, usize)> {
        let mut rest = data;
        let value = T::deserialize_reader(&mut rest).ok()?;
        Some((value, data.len() - rest.len()))
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// Bytes biased toward 0/1 so bools, tags and short lengths are often valid.
        fn byte(&mut self) -> u8 {
            match self.next() % 8 {
                0..=3 => 0,
                4 => 1,
                5 => 2,
                _ => self.next() as u8,
            }
        }
    }

    fn assert_same<T: BorshDeserialize + BorshSerialize>(data: &[u8]) -> bool {
        assert_same_by::<T, _>(data, |v| v.try_to_vec().unwrap())
    }

    /// Core comparison; `key` identifies a decoded value (its encoding, or `Debug` text for
    /// types whose serializer is stricter than their decoder).
    fn assert_same_by<T: BorshDeserialize + BorshSerialize, K: PartialEq + core::fmt::Debug>(
        data: &[u8],
        key: impl Fn(&T) -> K,
    ) -> bool {
        let compact = T::try_from_slice(data).ok();
        let reference = reference_exact::<T>(data);
        assert_eq!(
            compact.as_ref().map(&key),
            reference.as_ref().map(&key),
            "try_from_slice diverged on {data:?}"
        );
        let mut rest = data;
        let compact_prefix = T::deserialize(&mut rest)
            .ok()
            .map(|v| (v, data.len() - rest.len()));
        let reference_prefix = reference_prefix::<T>(data);
        assert_eq!(
            compact_prefix.as_ref().map(|(v, n)| (key(v), *n)),
            reference_prefix.as_ref().map(|(v, n)| (key(v), *n)),
            "deserialize diverged on {data:?}"
        );
        if let Some(Ok(encoded)) = compact.map(|value| value.try_to_vec()) {
            assert_eq!(encoded, data, "accepted bytes must re-encode identically");
        }
        reference_prefix.is_some()
    }

    /// Like `assert_same`, for types whose compact path is only their [`super::CursorField`]
    /// reader (nested inside other cursor decoders).
    fn assert_same_cursor<T: super::CursorField + BorshDeserialize + BorshSerialize>(data: &[u8]) {
        let compact = super::cursor_from_slice::<T>(data).ok();
        let reference = reference_exact::<T>(data);
        assert_eq!(
            compact.as_ref().map(|v| v.try_to_vec().unwrap()),
            reference.as_ref().map(|v| v.try_to_vec().unwrap()),
            "cursor reader diverged on {data:?}"
        );
        let mut rest = data;
        let compact_prefix = super::cursor_deserialize::<T>(&mut rest)
            .ok()
            .map(|v| (v, data.len() - rest.len()));
        assert_eq!(
            compact_prefix
                .as_ref()
                .map(|(v, n)| (v.try_to_vec().unwrap(), *n)),
            reference_prefix::<T>(data)
                .as_ref()
                .map(|(v, n)| (v.try_to_vec().unwrap(), *n)),
            "cursor prefix diverged on {data:?}"
        );
    }

    pub(crate) fn check_cursor<T: super::CursorField + BorshDeserialize + BorshSerialize>(
        seed: u64,
        max_len: usize,
        samples: &[Vec<u8>],
    ) {
        let mut rng = Rng(seed | 1);
        for _ in 0..4_000 {
            let len = (rng.next() as usize) % (max_len + 1);
            let data: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            assert_same_cursor::<T>(&data);
        }
        for sample in samples {
            assert!(
                reference_exact::<T>(sample).is_some(),
                "sample must be valid"
            );
            for_each_variation(sample, |data| assert_same_cursor::<T>(data));
        }
    }

    /// Every truncation, a one-byte extension, and every single-byte change (several values)
    /// of a valid encoding.
    pub(crate) fn for_each_variation(sample: &[u8], mut f: impl FnMut(&[u8])) {
        f(sample);
        for len in 0..sample.len() {
            f(&sample[..len]);
        }
        let mut longer = sample.to_vec();
        longer.push(0);
        f(&longer);
        for at in 0..sample.len() {
            for value in [0u8, 1, 2, 3, 0x7f, 0xff, sample[at].wrapping_add(1)] {
                let mut changed = sample.to_vec();
                changed[at] = value;
                f(&changed);
            }
        }
    }

    /// `check` plus exhaustive variations of known-valid samples.
    pub(crate) fn check_with_samples<T: BorshDeserialize + BorshSerialize>(
        seed: u64,
        max_len: usize,
        samples: &[Vec<u8>],
    ) {
        check::<T>(seed, max_len);
        for sample in samples {
            assert!(
                reference_exact::<T>(sample).is_some(),
                "sample must be valid"
            );
            for_each_variation(sample, |data| {
                assert_same::<T>(data);
            });
        }
    }

    /// Random buffers plus exhaustive sample variations, comparing decoded values by `Debug`.
    pub(crate) fn check_debug_with_samples<
        T: BorshDeserialize + BorshSerialize + core::fmt::Debug,
    >(
        seed: u64,
        max_len: usize,
        samples: &[Vec<u8>],
    ) {
        let key = |v: &T| format!("{v:?}");
        let mut rng = Rng(seed | 1);
        for _ in 0..4_000 {
            let len = (rng.next() as usize) % (max_len + 1);
            let data: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            assert_same_by::<T, _>(&data, key);
        }
        for sample in samples {
            assert!(
                reference_exact::<T>(sample).is_some(),
                "sample must be valid"
            );
            for_each_variation(sample, |data| {
                assert_same_by::<T, _>(data, key);
            });
        }
    }

    /// Random buffers, their prefixes/extensions, and single-byte mutations of every buffer that
    /// the reference accepts.
    pub(crate) fn check<T: BorshDeserialize + BorshSerialize>(seed: u64, max_len: usize) {
        let mut rng = Rng(seed | 1);
        let mut accepted = 0usize;
        for _ in 0..4_000 {
            let len = (rng.next() as usize) % (max_len + 1);
            let data: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            assert_same::<T>(&data);
            if let Some((_, used)) = reference_prefix::<T>(&data) {
                accepted += 1;
                let exact = &data[..used];
                assert_same::<T>(exact);
                if used > 0 {
                    assert_same::<T>(&exact[..used - 1]);
                }
                let mut longer = exact.to_vec();
                longer.push(rng.byte());
                assert_same::<T>(&longer);
                for _ in 0..4 {
                    let mut mutated = exact.to_vec();
                    if !mutated.is_empty() {
                        let at = (rng.next() as usize) % mutated.len();
                        mutated[at] = rng.next() as u8;
                    }
                    assert_same::<T>(&mutated);
                }
            }
        }
        assert!(accepted > 0, "generator never produced a valid encoding");
    }
}
