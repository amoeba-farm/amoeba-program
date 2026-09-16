use super::*;

/// A field in a flat fixed-state codec.
///
/// `native_offset` is the offset in the Rust value, while the wire offset is
/// the cumulative offset of earlier descriptors.  The codec therefore copies
/// only declared fields and never treats native padding as serialized data.
#[derive(Clone, Copy)]
pub(crate) struct FlatFieldDescriptor {
    pub(crate) native_offset: usize,
    pub(crate) wire_len: usize,
    pub(crate) bool_marker: bool,
}

impl FlatFieldDescriptor {
    pub(crate) const fn new(native_offset: usize, wire_len: usize, bool_marker: bool) -> Self {
        Self {
            native_offset,
            wire_len,
            bool_marker,
        }
    }
}

/// The immutable metadata shared by all flat state codecs.
#[derive(Clone, Copy)]
pub(crate) struct FlatCodecDescriptor {
    pub(crate) native_size: usize,
    pub(crate) wire_len: usize,
    pub(crate) fields: &'static [FlatFieldDescriptor],
}

impl FlatCodecDescriptor {
    pub(crate) const fn new(
        native_size: usize,
        wire_len: usize,
        fields: &'static [FlatFieldDescriptor],
    ) -> Self {
        Self {
            native_size,
            wire_len,
            fields,
        }
    }
}

/// Types admitted by `fixed_state_deserialize_flat!`.
///
/// This is deliberately an opt-in marker.  In particular, enums, options and
/// nested records have no implementation, so accidentally adding one to the
/// flat macro is a compile error rather than a new wire-format guess.
pub(crate) trait FlatFieldType {
    const WIRE_LEN: usize;
    const BOOL_MARKER: bool;
}

macro_rules! flat_scalar_type {
    ($($type:ty),+ $(,)?) => {
        $(
            impl FlatFieldType for $type {
                const WIRE_LEN: usize = core::mem::size_of::<$type>();
                const BOOL_MARKER: bool = false;
            }
        )+
    };
}

impl FlatFieldType for bool {
    const WIRE_LEN: usize = 1;
    const BOOL_MARKER: bool = true;
}

flat_scalar_type!(u8, u16, u32, i32, u64, u128, i64);

impl FlatFieldType for Pubkey {
    const WIRE_LEN: usize = 32;
    const BOOL_MARKER: bool = false;
}

impl<T: FlatFieldType, const LENGTH: usize> FlatFieldType for [T; LENGTH] {
    const WIRE_LEN: usize = T::WIRE_LEN * LENGTH;
    const BOOL_MARKER: bool = T::BOOL_MARKER;
}

/// Copy the declared flat fields from a native value to the Borsh wire.
///
/// # Safety
/// `native` must point to an initialized value whose allocation is at least
/// `descriptor.native_size` bytes. `output` must point to `output_len` writable
/// bytes. The descriptor's native offsets must describe that allocation.
#[inline(never)]
pub(crate) unsafe fn encode_flat_fields(
    descriptor: &FlatCodecDescriptor,
    native: *const u8,
    output: *mut u8,
    output_len: usize,
) -> bool {
    if output_len < descriptor.wire_len {
        return false;
    }

    let mut wire_offset = 0usize;
    for field in descriptor.fields {
        let Some(wire_end) = wire_offset.checked_add(field.wire_len) else {
            return false;
        };
        let Some(native_end) = field.native_offset.checked_add(field.wire_len) else {
            return false;
        };
        if wire_end > descriptor.wire_len || native_end > descriptor.native_size {
            return false;
        }

        // The compile-time marker trait proves this is a primitive or a byte
        // representation of one; the runtime check above proves both ranges.
        core::ptr::copy_nonoverlapping(
            native.add(field.native_offset),
            output.add(wire_offset),
            field.wire_len,
        );
        wire_offset = wire_end;
    }

    wire_offset == descriptor.wire_len
}

/// Decode declared flat fields into uninitialized native storage.
///
/// Validation is completed before any typed value is assumed initialized.  In
/// particular, every bool marker is checked for the exact Borsh values 0 or 1
/// before the corresponding byte can become part of a `T`.
///
/// # Safety
/// `destination` must point to writable `MaybeUninit<T>` storage whose
/// allocation is at least `descriptor.native_size` bytes. It must not alias
/// `data` for a nonzero copy.
#[inline(never)]
pub(crate) unsafe fn decode_flat_fields(
    descriptor: &FlatCodecDescriptor,
    data: &[u8],
    destination: *mut u8,
) -> std::io::Result<()> {
    if data.len() < descriptor.wire_len {
        return Err(invalid_fixed_borsh());
    }

    // First pass: check every descriptor and validate bool bytes while the
    // destination is still untyped storage.
    let mut wire_offset = 0usize;
    for field in descriptor.fields {
        let Some(wire_end) = wire_offset.checked_add(field.wire_len) else {
            return Err(invalid_fixed_borsh());
        };
        let Some(native_end) = field.native_offset.checked_add(field.wire_len) else {
            return Err(invalid_fixed_borsh());
        };
        if wire_end > descriptor.wire_len || native_end > descriptor.native_size {
            return Err(invalid_fixed_borsh());
        }
        if field.bool_marker && data[wire_offset..wire_end].iter().any(|byte| *byte > 1) {
            return Err(invalid_fixed_borsh());
        }
        wire_offset = wire_end;
    }
    if wire_offset != descriptor.wire_len
        || data[descriptor.wire_len..].iter().any(|byte| *byte != 0)
    {
        return Err(invalid_fixed_borsh());
    }

    // Second pass: all input validation has completed, so no invalid bool or
    // other invalid typed representation can be observed through `T`.
    wire_offset = 0;
    for field in descriptor.fields {
        core::ptr::copy_nonoverlapping(
            data.as_ptr().add(wire_offset),
            destination.add(field.native_offset),
            field.wire_len,
        );
        wire_offset += field.wire_len;
    }
    Ok(())
}
