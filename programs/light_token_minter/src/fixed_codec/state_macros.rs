use super::*;

#[cold]
pub(crate) fn invalid_fixed_borsh() -> std::io::Error {
    std::io::ErrorKind::InvalidData.into()
}

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
    };
}

pub(crate) use fixed_instruction_deserialize;

macro_rules! variable_state_codec {
    (Market, $maximum_len:expr, $encoded_len:ident, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        variable_state_codec!(@impl Market, $maximum_len, $encoded_len, true, { $($field: $field_type),+ });
    };
    ($type:ident, $maximum_len:expr, $encoded_len:ident, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        variable_state_codec!(@impl $type, $maximum_len, $encoded_len, false, { $($field: $field_type),+ });
    };
    (@impl $type:ident, $maximum_len:expr, $encoded_len:ident, $preserves_router:expr, { $($field:ident: $field_type:ty),+ $(,)? }) => {
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
            fn preserves_market_router(&self) -> bool {
                $preserves_router
            }
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
