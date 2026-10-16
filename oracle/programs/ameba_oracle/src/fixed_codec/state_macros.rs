use super::*;

#[cold]
pub(crate) fn invalid_fixed_borsh() -> std::io::Error {
    std::io::ErrorKind::InvalidData.into()
}

/// Generates a one-bounds-check slice decoder while retaining a field-for-field generic-reader
/// fallback for Borsh tooling. Runtime account loaders call `deserialize(&mut &[u8])`, so only the
/// compact fixed decoder is linked into the SBF artifact.
#[cfg(test)]
pub(crate) trait ReferenceBorsh {
    fn reference_borsh_bytes(&self) -> Vec<u8>;
}

#[allow(unused_macros)]
macro_rules! fixed_state_deserialize {
    ($type:ident, $serialized_len:expr, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        impl $type {
            // Account decoding accepts zero padding; Borsh decoding consumes an
            // exact prefix. Both paths share the same field reader and domains.
            #[inline(never)]
            fn __read_fixed_fields(data: &[u8], exact: bool) -> std::io::Result<Self> {
                let mut input = FixedCursor::new(data);
                let value = <Self as FixedField>::read(&mut input);
                if exact && input.offset != data.len() {
                    return Err(invalid_fixed_borsh());
                }
                input.finish_borsh()?;
                Ok(value)
            }
            // Both account stores and generic Borsh writers share this body.
            // Return the real offset so the Borsh length check stays intact.
            #[inline(never)]
            fn __write_fixed_fields(&self, data: &mut [u8]) -> usize {
                let mut output = FixedWriter::new(data);
                <Self as FixedField>::write(self, &mut output);
                output.offset
            }
        }

        impl FixedField for $type {
            #[inline(always)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                Self {
                    $($field: <$field_type as FixedField>::read(input),)+
                }
            }

            #[inline(always)]
            fn write(&self, output: &mut FixedWriter<'_>) {
                $(<$field_type as FixedField>::write(&self.$field, output);)+
            }
        }

        impl FixedStateEncode for $type {
            #[inline(always)]
            fn maximum_encoded_len(&self) -> usize {
                $serialized_len
            }

            #[inline(never)]
            fn encode_fixed(&self, data: &mut [u8]) {
                let written = self.__write_fixed_fields(data);
                debug_assert_eq!(written, $serialized_len);
            }
        }

        impl FixedStateDecode for $type {
            const REQUIRED_DATA_LEN: usize = $serialized_len;

            #[inline(never)]
            unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
                Self::__read_fixed_fields(data, false)
            }
        }

        #[cfg(test)]
        impl ReferenceBorsh for $type {
            fn reference_borsh_bytes(&self) -> Vec<u8> {
                let mut encoded = Vec::new();
                $(
                    <$field_type as BorshSerialize>::serialize(&self.$field, &mut encoded)
                        .expect("serialize reference Borsh field");
                )+
                encoded
            }
        }

        impl BorshSerialize for $type {
            #[inline(never)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let mut encoded = vec![0u8; $serialized_len];
                if self.__write_fixed_fields(&mut encoded) != $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
                writer.write_all(&encoded)
            }
        }

        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                if data.len() < $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
                let (encoded, remaining) = data.split_at($serialized_len);
                let value = Self::__read_fixed_fields(encoded, true)?;
                *data = remaining;
                Ok(value)
            }

            #[inline(never)]
            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                let mut encoded = vec![0u8; $serialized_len];
                reader.read_exact(&mut encoded)?;
                let mut data = encoded.as_slice();
                Self::deserialize(&mut data)
            }
        }
    };
}

pub(crate) use fixed_state_deserialize;

/// Generates the shared table-driven codec for a flat, rigid state layout.
///
/// The marker trait admits primitive values, bools, Pubkeys, explicitly audited
/// repr(u8) enums, and their arrays. The native offsets are used only to locate
/// fields; wire offsets remain the declared Borsh field order, so native Rust
/// padding never enters the account format.
macro_rules! fixed_state_deserialize_flat {
    ($type:ident, $serialized_len:expr, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        $crate::fixed_codec::fixed_state_deserialize_flat!(
            $type, $serialized_len,
            { $($field: $field_type),+ },
            flat { $($field: $field_type),+ }
        );
    };
    (
        $type:ident, $serialized_len:expr,
        { $($field:ident: $field_type:ty),+ $(,)? },
        flat { $($($leaf:ident).+: $leaf_type:ty),+ $(,)? }
    ) => {
        // The explicit leaf list can flatten a nested record while retaining
        // its ordinary typed field codec and independent Borsh reference. Each
        // leaf receives its own offset and domain check; nested Rust padding
        // is never copied to or from the wire.
        const _: fn(&$type) = |value| {
            $(let _: &$leaf_type = &value.$($leaf).+;)+
        };
        const _: () = {
            assert!(
                $serialized_len
                    == 0usize $(+ <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN)+
            );
            $(
                assert!(
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN
                        == core::mem::size_of::<$leaf_type>()
                );
                assert!(
                    core::mem::offset_of!($type, $($leaf).+)
                        + core::mem::size_of::<$leaf_type>()
                        <= core::mem::size_of::<$type>()
                );
            )+
        };

        impl $type {
            const __FLAT_CODEC_FIELDS: &'static [$crate::fixed_codec::FlatFieldDescriptor] = &[
                $($crate::fixed_codec::FlatFieldDescriptor::new(
                    core::mem::offset_of!($type, $($leaf).+),
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN,
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::BYTE_DOMAIN,
                ),)+
            ];

            const __FLAT_CODEC: $crate::fixed_codec::FlatCodecDescriptor =
                $crate::fixed_codec::FlatCodecDescriptor::new(
                core::mem::size_of::<$type>(),
                $serialized_len,
                Self::__FLAT_CODEC_FIELDS,
            );

            #[inline(never)]
            fn __write_flat_fields(&self, data: &mut [u8]) -> bool {
                // The descriptor admits only fields with a byte-identical
                // native representation and checks both allocation bounds.
                unsafe {
                    $crate::fixed_codec::encode_flat_fields(
                        &Self::__FLAT_CODEC,
                        (self as *const Self).cast::<u8>(),
                        data.as_mut_ptr(),
                        data.len(),
                    )
                }
            }
        }

        impl FixedField for $type {
            #[inline(always)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                Self {
                    $($field: <$field_type as FixedField>::read(input),)+
                }
            }

            #[inline(always)]
            fn write(&self, output: &mut FixedWriter<'_>) {
                $(<$field_type as FixedField>::write(&self.$field, output);)+
            }
        }

        impl FixedStateEncode for $type {
            #[inline(always)]
            fn maximum_encoded_len(&self) -> usize {
                $serialized_len
            }

            #[inline(never)]
            fn encode_fixed(&self, data: &mut [u8]) {
                let encoded = self.__write_flat_fields(data);
                debug_assert!(encoded);
            }
        }

        impl FixedStateDecode for $type {
            const REQUIRED_DATA_LEN: usize = $serialized_len;

            #[inline(always)]
            unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
                Self::decode_fixed_option(data).ok_or_else(invalid_fixed_borsh)
            }

            #[inline(never)]
            unsafe fn decode_fixed_option(data: &[u8]) -> Option<Self> {
                let mut value = core::mem::MaybeUninit::<$type>::uninit();
                $crate::fixed_codec::decode_flat_fields(
                    &$type::__FLAT_CODEC,
                    data,
                    value.as_mut_ptr().cast::<u8>(),
                ).ok()?;
                // SAFETY: the descriptor covers every field (including each
                // audited nested leaf). decode_flat_fields validates the exact
                // bool/enum domains and initializes all fields before success.
                // Rust padding remains uninitialized and is never serialized.
                Some(value.assume_init())
            }
        }

        #[cfg(test)]
        impl ReferenceBorsh for $type {
            fn reference_borsh_bytes(&self) -> Vec<u8> {
                let mut encoded = Vec::new();
                $(
                    <$field_type as BorshSerialize>::serialize(&self.$field, &mut encoded)
                        .expect("serialize reference Borsh field");
                )+
                encoded
            }
        }

        impl BorshSerialize for $type {
            #[inline(never)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let mut encoded = vec![0u8; $serialized_len];
                if !self.__write_flat_fields(&mut encoded) {
                    return Err(invalid_fixed_borsh());
                }
                writer.write_all(&encoded)
            }
        }

        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                if data.len() < $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
                let (encoded, remaining) = data.split_at($serialized_len);
                // The flat descriptor's field sum is checked against this exact
                // prefix length at compile time. Share its bounds and byte-domain
                // validation with account loads; advance only after success.
                let value = unsafe { <Self as FixedStateDecode>::decode_fixed(encoded)? };
                *data = remaining;
                Ok(value)
            }

            #[inline(never)]
            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                let mut encoded = vec![0u8; $serialized_len];
                reader.read_exact(&mut encoded)?;
                let mut data = encoded.as_slice();
                Self::deserialize(&mut data)
            }
        }
    };
}

pub(crate) use fixed_state_deserialize_flat;

/// Variant of `fixed_state_deserialize!` for a context-bound physical account.
///
/// The omitted fields remain part of the logical Rust state, but the account wire format stores
/// only the listed fields. Loaders must bind the omitted values from authenticated PDA/context
/// accounts before validating or exposing the logical value. This keeps the compact format
/// explicit instead of silently relying on a larger decoder's zero tail.
#[allow(unused_macros)]
macro_rules! fixed_state_deserialize_compact {
    (
        $type:ident,
        $serialized_len:expr,
        { $($field:ident: $field_type:ty),+ $(,)? },
        defaults { $($omitted:ident: $omitted_type:ty),+ $(,)? }
    ) => {
        const _: fn(&$type) = |value| {
            $(let _: &$field_type = &value.$field;)+
            $(let _: &$omitted_type = &value.$omitted;)+
        };
        const _: () = {
            assert!($serialized_len == 0usize
                $(+ <$field_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN)+);
            $(assert!(<$field_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN
                == core::mem::size_of::<$field_type>());)+
        };
        impl $type {
            const __COMPACT_FLAT_FIELDS: &'static [$crate::fixed_codec::FlatFieldDescriptor] = &[
                $($crate::fixed_codec::FlatFieldDescriptor::new(
                    core::mem::offset_of!($type, $field),
                    <$field_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN,
                    <$field_type as $crate::fixed_codec::FlatFieldType>::BYTE_DOMAIN,
                ),)+
            ];
            const __COMPACT_FLAT_CODEC: $crate::fixed_codec::FlatCodecDescriptor =
                $crate::fixed_codec::FlatCodecDescriptor::new(
                    core::mem::size_of::<$type>(),
                    $serialized_len,
                    Self::__COMPACT_FLAT_FIELDS,
                );

            // Account decoding accepts zero padding; Borsh decoding consumes an
            // exact prefix. Both paths share the same field reader and domains.
            #[inline(never)]
            fn __read_fixed_fields(data: &[u8], exact: bool) -> std::io::Result<Self> {
                if exact && data.len() != $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
                // Initialize every field by its own Default, just as the old
                // typed reader did for omitted fields. The flat decoder only
                // replaces admitted scalar fields after checking the full
                // input; omitted identity fields keep their exact defaults.
                let mut value = Self {
                    $($field: <$field_type as Default>::default(),)+
                    $($omitted: <$omitted_type as Default>::default(),)+
                };
                // SAFETY: the descriptor checks scalar layouts, native bounds
                // and non-overlap. `value` is already a fully initialized Self;
                // no omitted field, reference, allocation or padding is copied.
                unsafe {
                    $crate::fixed_codec::decode_flat_fields(
                        &Self::__COMPACT_FLAT_CODEC,
                        data,
                        (&mut value as *mut Self).cast::<u8>(),
                    )
                }?;
                Ok(value)
            }
            #[inline(never)]
            fn __write_fixed_fields(&self, data: &mut [u8]) -> usize {
                // SAFETY: only the declared admitted scalar fields are read;
                // the same checked descriptor preserves their original order.
                let written = unsafe {
                    $crate::fixed_codec::encode_flat_fields(
                        &Self::__COMPACT_FLAT_CODEC,
                        (self as *const Self).cast::<u8>(),
                        data.as_mut_ptr(),
                        data.len(),
                    )
                };
                if written { $serialized_len } else { 0 }
            }
        }

        impl FixedField for $type {
            #[inline(always)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                Self {
                    $($field: <$field_type as FixedField>::read(input),)+
                    $($omitted: <$omitted_type as Default>::default(),)+
                }
            }

            #[inline(always)]
            fn write(&self, output: &mut FixedWriter<'_>) {
                $(<$field_type as FixedField>::write(&self.$field, output);)+
            }
        }

        impl FixedStateEncode for $type {
            #[inline(always)]
            fn maximum_encoded_len(&self) -> usize {
                $serialized_len
            }

            #[inline(never)]
            fn encode_fixed(&self, data: &mut [u8]) {
                let written = self.__write_fixed_fields(data);
                debug_assert_eq!(written, $serialized_len);
            }
        }

        impl FixedStateDecode for $type {
            const REQUIRED_DATA_LEN: usize = $serialized_len;

            #[inline(never)]
            unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
                Self::__read_fixed_fields(data, false)
            }
        }

        #[cfg(test)]
        impl ReferenceBorsh for $type {
            fn reference_borsh_bytes(&self) -> Vec<u8> {
                let mut encoded = Vec::new();
                $(
                    <$field_type as BorshSerialize>::serialize(&self.$field, &mut encoded)
                        .expect("serialize compact reference Borsh field");
                )+
                encoded
            }
        }

        impl BorshSerialize for $type {
            #[inline(never)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let mut encoded = vec![0u8; $serialized_len];
                if self.__write_fixed_fields(&mut encoded) != $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
                writer.write_all(&encoded)
            }
        }

        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                if data.len() < $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
                let (encoded, remaining) = data.split_at($serialized_len);
                let value = Self::__read_fixed_fields(encoded, true)?;
                *data = remaining;
                Ok(value)
            }

            #[inline(never)]
            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                let mut encoded = vec![0u8; $serialized_len];
                reader.read_exact(&mut encoded)?;
                let mut data = encoded.as_slice();
                Self::deserialize(&mut data)
            }
        }
    };
}

pub(crate) use fixed_state_deserialize_compact;

/// Generates a byte-identical Borsh slice decoder for a rigid instruction payload. Production
/// dispatch decodes from `&[u8]`, while the reader fallback preserves ordinary Borsh tooling.
#[cfg(test)]
pub(crate) trait ReferenceInstructionDeserialize: Sized {
    const WIRE_LEN: usize;
    fn reference_deserialize(data: &mut &[u8]) -> std::io::Result<Self>;
}

macro_rules! fixed_instruction_deserialize {
    ($type:ident, $serialized_len:expr, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        $crate::fixed_codec::fixed_instruction_deserialize!(
            $type, $serialized_len,
            { $($field: $field_type),+ },
            flat { $($field: $field_type),+ }
        );
    };
    (
        $type:ident, $serialized_len:expr,
        { $($field:ident: $field_type:ty),+ $(,)? },
        flat { $($($leaf:ident).+: $leaf_type:ty),+ $(,)? }
    ) => {
        const _: fn(&$type) = |value| {
            $(let _: &$leaf_type = &value.$($leaf).+;)+
        };
        const _: () = {
            assert!(
                $serialized_len
                    == 0usize $(+ <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN)+
            );
            $(
                assert!(
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN
                        == core::mem::size_of::<$leaf_type>()
                );
            )+
        };
        impl $type {
            const __INSTRUCTION_FIELDS: &'static [$crate::fixed_codec::FlatFieldDescriptor] = &[
                $($crate::fixed_codec::FlatFieldDescriptor::new(
                    core::mem::offset_of!($type, $($leaf).+),
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::WIRE_LEN,
                    <$leaf_type as $crate::fixed_codec::FlatFieldType>::BYTE_DOMAIN,
                ),)+
            ];
            const __INSTRUCTION_CODEC: $crate::fixed_codec::FlatCodecDescriptor =
                $crate::fixed_codec::FlatCodecDescriptor::new(
                    core::mem::size_of::<$type>(),
                    $serialized_len,
                    Self::__INSTRUCTION_FIELDS,
                );
        }
        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                if data.len() < $serialized_len {
                    return Err($crate::fixed_codec::invalid_fixed_borsh());
                }
                let (encoded, remaining) = data.split_at($serialized_len);
                let mut value = core::mem::MaybeUninit::<Self>::uninit();
                // The descriptor copies only the declared typed leaves. All
                // restricted byte domains are checked before any value is
                // initialized. Every field is covered by the descriptor,
                // including the audited nested leaves; native padding remains
                // uninitialized and is never read or serialized.
                let value = unsafe {
                    $crate::fixed_codec::decode_flat_fields(
                        &Self::__INSTRUCTION_CODEC,
                        encoded,
                        value.as_mut_ptr().cast::<u8>(),
                    )?;
                    value.assume_init()
                };
                *data = remaining;
                Ok(value)
            }

            #[inline(never)]
            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                let mut encoded = [0u8; $serialized_len];
                reader.read_exact(&mut encoded)?;
                let mut data = encoded.as_slice();
                Self::deserialize(&mut data)
            }
        }
        // Retain the former typed reader independently of the descriptor so
        // differential tests check every field, domain, and prefix boundary.
        #[cfg(test)]
        impl $crate::fixed_codec::ReferenceInstructionDeserialize for $type {
            const WIRE_LEN: usize = $serialized_len;
            fn reference_deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                if data.len() < $serialized_len {
                    return Err($crate::fixed_codec::invalid_fixed_borsh());
                }
                let (encoded, remaining) = data.split_at($serialized_len);
                let mut input = $crate::fixed_codec::FixedCursor::new(encoded);
                let value = Self {
                    $($field: <$field_type as $crate::fixed_codec::FixedField>::read(&mut input),)+
                };
                input.finish_borsh()?;
                *data = remaining;
                Ok(value)
            }
        }
    };
}

pub(crate) use fixed_instruction_deserialize;

#[cfg(test)]
#[path = "instruction_flat_tests.rs"]
mod instruction_flat_tests;

macro_rules! variable_state_codec {
    ($type:ident, $maximum_len:expr, $encoded_len:ident, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        impl $type {
            // Account decoding accepts zero padding; Borsh decoding consumes an
            // exact prefix. Both paths share the same field reader and domains.
            #[inline(never)]
            fn __read_fixed_fields(data: &[u8], exact: bool) -> std::io::Result<Self> {
                let mut input = FixedCursor::new(data);
                let value = <Self as FixedField>::read(&mut input);
                if exact && input.offset != data.len() {
                    return Err(invalid_fixed_borsh());
                }
                input.finish_borsh()?;
                Ok(value)
            }
            #[inline(never)]
            fn __write_fixed_fields(&self, data: &mut [u8]) -> usize {
                let mut output = FixedWriter::new(data);
                <Self as FixedField>::write(self, &mut output);
                output.offset
            }
        }

        impl FixedField for $type {
            #[inline(always)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                Self {
                    $($field: <$field_type as FixedField>::read(input),)+
                }
            }

            #[inline(always)]
            fn write(&self, output: &mut FixedWriter<'_>) {
                $(<$field_type as FixedField>::write(&self.$field, output);)+
            }
        }

        impl FixedStateEncode for $type {
            #[inline(always)]
            fn maximum_encoded_len(&self) -> usize {
                $maximum_len
            }

            #[inline(never)]
            fn encode_fixed(&self, data: &mut [u8]) {
                let written = self.__write_fixed_fields(data);
                debug_assert!(written <= $maximum_len);
            }
        }

        impl FixedStateDecode for $type {
            const REQUIRED_DATA_LEN: usize = $maximum_len;

            #[inline(never)]
            unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
                Self::__read_fixed_fields(data, false)
            }
        }

        #[cfg(test)]
        impl ReferenceBorsh for $type {
            fn reference_borsh_bytes(&self) -> Vec<u8> {
                let mut encoded = Vec::new();
                $(
                    <$field_type as BorshSerialize>::serialize(&self.$field, &mut encoded)
                        .expect("serialize reference Borsh field");
                )+
                encoded
            }
        }

        impl BorshSerialize for $type {
            #[inline(never)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let mut encoded = [0u8; $maximum_len];
                let encoded_len = self.__write_fixed_fields(&mut encoded);
                writer.write_all(&encoded[..encoded_len])
            }
        }

        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                let encoded_len = $encoded_len(data)?;
                let (encoded, remaining) = data.split_at(encoded_len);
                let value = Self::__read_fixed_fields(encoded, true)?;
                *data = remaining;
                Ok(value)
            }

            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                Ok(Self {
                    $($field: <$field_type as BorshDeserialize>::deserialize_reader(reader)?,)+
                })
            }
        }
    };
}

pub(crate) use variable_state_codec;

#[cfg(test)]
mod shared_writer_tests {
    use super::*;
    use std::io::{ErrorKind, Write};

    struct ChunkedWriter {
        bytes: Vec<u8>,
        remaining: Option<usize>,
    }
    impl Write for ChunkedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let size = bytes.len().min(7).min(self.remaining.unwrap_or(usize::MAX));
            self.bytes.extend_from_slice(&bytes[..size]);
            if let Some(left) = &mut self.remaining {
                *left -= size;
            }
            Ok(size)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn check<T: BorshSerialize + FixedStateEncode + ReferenceBorsh>(value: &T) {
        let expected = value.reference_borsh_bytes();
        assert_eq!(value.try_to_vec().unwrap(), expected);
        let mut fixed = vec![0x6d; value.maximum_encoded_len() + 16];
        value.encode_fixed(&mut fixed[..value.maximum_encoded_len()]);
        assert_eq!(&fixed[..expected.len()], expected);
        assert!(fixed[expected.len()..].iter().all(|byte| *byte == 0x6d));
        let mut partial = ChunkedWriter {
            bytes: vec![],
            remaining: None,
        };
        value.serialize(&mut partial).unwrap();
        assert_eq!(partial.bytes, expected);
        for limit in [0, 1, expected.len() / 2, expected.len() - 1] {
            let mut blocked = ChunkedWriter {
                bytes: vec![],
                remaining: Some(limit),
            };
            assert_eq!(
                value.serialize(&mut blocked).unwrap_err().kind(),
                ErrorKind::WriteZero
            );
            assert_eq!(blocked.bytes, expected[..limit]);
        }
    }

    #[test]
    fn shared_writers_preserve_reference_bytes_padding_and_partial_io() {
        check(&OracleConfig {
            is_initialized: true,
            bump: 7,
            admin: Pubkey::new_unique(),
            oracle_authority: Pubkey::new_unique(),
            usdc_mint: Pubkey::new_unique(),
            bond_vault: Pubkey::new_unique(),
            paused: true,
            account_discriminator: OracleConfig::ACCOUNT_DISCRIMINATOR,
            account_version: OracleConfig::ACCOUNT_VERSION,
        });
        check(&OracleProductSkuDraft {
            draft_nonce: u64::MAX,
            last_updated_slot: u64::MAX,
            ..OracleProductSkuDraft::default()
        });
        let claim = OracleOpeningClaim {
            attempt: u32::MAX,
            stake: u64::MAX,
            evidence_hash: [171; 32],
            // These context fields are intentionally absent from the compact wire.
            month: Pubkey::new_unique(),
            source: Pubkey::new_unique(),
            ..OracleOpeningClaim::default()
        };
        check(&claim);
        for present in [false, true] {
            let month = OracleMonthState {
                settlement_record: present.then(Pubkey::new_unique),
                accepted_cash_update_count: u32::MAX,
                ..OracleMonthState::default()
            };
            assert_eq!(
                month.try_to_vec().unwrap().len(),
                if present { 299 } else { 267 }
            );
            check(&month);
        }
    }

    struct LengthMismatch {
        value: u8,
    }
    fixed_state_deserialize!(LengthMismatch, 2, { value: u8 });

    struct FlatReaderFixture {
        enabled: bool,
        phase: OracleRecipeWeightPhase,
        amount: u64,
    }
    fixed_state_deserialize_flat!(FlatReaderFixture, 10, {
        enabled: bool,
        phase: OracleRecipeWeightPhase,
        amount: u64,
    });

    #[test]
    fn flat_borsh_reader_preserves_cursor_on_invalid_bytes_and_exact_prefix_on_success() {
        for enabled in [0u8, 1, 2, 255] {
            for phase in 0u8..=255 {
                let mut wire = vec![enabled, phase];
                wire.extend_from_slice(&0x0123_4567_89ab_cdefu64.to_le_bytes());
                wire.extend_from_slice(&[0xab; 7]);
                let mut input = wire.as_slice();
                let result = FlatReaderFixture::deserialize(&mut input);
                if enabled <= 1 && matches!(phase, 0 | 2 | 3) {
                    let value = result.unwrap();
                    assert_eq!(value.enabled, enabled == 1);
                    assert_eq!(value.phase as u8, phase);
                    assert_eq!(value.amount, 0x0123_4567_89ab_cdef);
                    assert_eq!(value.reference_borsh_bytes(), wire[..10]);
                    assert_eq!(input, &[0xab; 7]);
                } else {
                    assert!(result.is_err());
                    assert_eq!(input.as_ptr(), wire.as_ptr());
                    assert_eq!(input, wire);
                }
            }
        }
        let valid = [0u8; 10];
        for length in 0..valid.len() {
            let original = &valid[..length];
            let mut input = original;
            assert!(FlatReaderFixture::deserialize(&mut input).is_err());
            assert_eq!(input.as_ptr(), original.as_ptr());
            assert_eq!(input.len(), original.len());
        }
    }

    #[test]
    fn shared_borsh_writer_retains_actual_length_guard_before_external_write() {
        let mut external = vec![17, 23];
        assert_eq!(
            LengthMismatch { value: 99 }
                .serialize(&mut external)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
        assert_eq!(external, [17, 23]);
    }

    fn check_shared_reader<T: FixedStateDecode + BorshDeserialize + ReferenceBorsh>(value: &T) {
        let wire = value.reference_borsh_bytes();
        let mut prefixed = wire.clone();
        prefixed.extend_from_slice(&[0xa7; 9]);
        let mut input = prefixed.as_slice();
        let decoded = T::deserialize(&mut input).unwrap();
        assert_eq!(decoded.reference_borsh_bytes(), wire);
        assert_eq!(input, &[0xa7; 9]);
        let mut allocation = wire.clone();
        allocation.resize(T::REQUIRED_DATA_LEN.max(wire.len()) + 9, 0);
        let decoded = unsafe { T::decode_fixed(&allocation) }.unwrap();
        assert_eq!(decoded.reference_borsh_bytes(), wire);
        *allocation.last_mut().unwrap() = 1;
        assert!(unsafe { T::decode_fixed(&allocation) }.is_err());
        for end in 0..wire.len() {
            let original = &wire[..end];
            let mut input = original;
            assert!(T::deserialize(&mut input).is_err());
            assert_eq!(input.as_ptr(), original.as_ptr());
            assert_eq!(input.len(), original.len());
        }
    }

    #[test]
    fn shared_field_readers_preserve_strict_prefix_and_padded_account_semantics() {
        // A padded fixed decoder accepts this synthetic layout, while Borsh
        // must retain its exact written-field count guard.
        let padded = [99u8, 0];
        assert_eq!(
            unsafe { LengthMismatch::decode_fixed(&padded) }
                .unwrap()
                .value,
            99
        );
        let mut input = padded.as_slice();
        assert!(LengthMismatch::deserialize(&mut input).is_err());
        assert_eq!(input, padded);
        assert!(unsafe { LengthMismatch::decode_fixed(&[99, 1]) }.is_err());

        check_shared_reader(&OracleProductSkuDraft::default());
        check_shared_reader(&UserCollateral {
            available_balance: u64::MAX,
            ..UserCollateral::default()
        });
        check_shared_reader(&OracleOpeningClaim {
            // Compact context fields remain omitted from the wire.
            month: Pubkey::new_unique(),
            source: Pubkey::new_unique(),
            ..OracleOpeningClaim::default()
        });
        for present in [false, true] {
            let month = OracleMonthState {
                settlement_record: present.then(Pubkey::new_unique),
                accepted_cash_update_count: u32::MAX,
                ..OracleMonthState::default()
            };
            check_shared_reader(&month);
            let mut malformed = month.reference_borsh_bytes();
            malformed[0] = 2;
            let mut input = malformed.as_slice();
            assert!(OracleMonthState::deserialize(&mut input).is_err());
            assert_eq!(input.as_ptr(), malformed.as_ptr());
            assert_eq!(input.len(), malformed.len());
            malformed.resize(OracleMonthState::REQUIRED_DATA_LEN, 0);
            assert!(unsafe { OracleMonthState::decode_fixed(&malformed) }.is_err());
        }
    }
}
