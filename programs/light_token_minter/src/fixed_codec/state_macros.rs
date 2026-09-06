use super::*;

#[cold]
pub(crate) fn invalid_fixed_borsh() -> std::io::Error {
    std::io::ErrorKind::InvalidData.into()
}

/// Generates a one-bounds-check slice decoder while retaining a field-for-field generic-reader
/// fallback for Borsh tooling. Runtime account loaders call `deserialize(&mut &[u8])`, so only the
/// compact fixed decoder is linked into the SBF artifact.

macro_rules! fixed_state_deserialize {
    ($type:ident, $serialized_len:expr, { $($field:ident: $field_type:ty),+ $(,)? }) => {
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
                let mut output = FixedWriter::new(data);
                <Self as FixedField>::write(self, &mut output);
                debug_assert_eq!(output.offset, $serialized_len);
            }
        }

        impl FixedStateDecode for $type {
            const REQUIRED_DATA_LEN: usize = $serialized_len;

            #[inline(never)]
            unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
                let mut input = FixedCursor::new(data);
                let value = <Self as FixedField>::read(&mut input);
                input.finish_borsh()?;
                Ok(value)
            }
        }

        impl BorshSerialize for $type {
            #[inline(never)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let mut encoded = vec![0u8; $serialized_len];
                let mut output = FixedWriter::new(&mut encoded);
                <Self as FixedField>::write(self, &mut output);
                if output.offset != $serialized_len {
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
                let mut input = FixedCursor::new(encoded);
                let value = <Self as FixedField>::read(&mut input);
                if input.invalid || input.offset != $serialized_len {
                    return Err(invalid_fixed_borsh());
                }
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

/// Generates a byte-identical Borsh slice decoder for a rigid instruction payload. Production
/// dispatch decodes from `&[u8]`, while the reader fallback preserves ordinary Borsh tooling.
macro_rules! fixed_instruction_deserialize {
    ($type:ident, $serialized_len:expr, { $($field:ident: $field_type:ty),+ $(,)? }) => {
        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
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

            #[inline(never)]
            fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
                let mut encoded = [0u8; $serialized_len];
                reader.read_exact(&mut encoded)?;
                let mut data = encoded.as_slice();
                Self::deserialize(&mut data)
            }
        }
    };
}

pub(crate) use fixed_instruction_deserialize;

macro_rules! variable_state_codec {
    ($type:ident, $maximum_len:expr, $encoded_len:ident, { $($field:ident: $field_type:ty),+ $(,)? }) => {
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
                let mut output = FixedWriter::new(data);
                <Self as FixedField>::write(self, &mut output);
                debug_assert!(output.offset <= $maximum_len);
            }
        }

        impl FixedStateDecode for $type {
            const REQUIRED_DATA_LEN: usize = $maximum_len;

            #[inline(never)]
            unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self> {
                let mut input = FixedCursor::new(data);
                let value = <Self as FixedField>::read(&mut input);
                input.finish_borsh()?;
                Ok(value)
            }
        }

        impl BorshSerialize for $type {
            #[inline(never)]
            fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
                let mut encoded = [0u8; $maximum_len];
                let encoded_len = {
                    let mut output = FixedWriter::new(&mut encoded);
                    <Self as FixedField>::write(self, &mut output);
                    output.offset
                };
                writer.write_all(&encoded[..encoded_len])
            }
        }

        impl BorshDeserialize for $type {
            #[inline(never)]
            fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
                let encoded_len = $encoded_len(data)?;
                let (encoded, remaining) = data.split_at(encoded_len);
                let mut input = FixedCursor::new(encoded);
                let value = <Self as FixedField>::read(&mut input);
                if input.invalid || input.offset != encoded_len {
                    return Err(invalid_fixed_borsh());
                }
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
