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
