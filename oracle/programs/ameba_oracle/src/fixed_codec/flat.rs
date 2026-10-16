use super::*;

/// A field in a flat fixed-state codec.
///
/// `native_offset` is the offset in the Rust value, while the wire offset is
/// the cumulative offset of earlier descriptors.  The codec therefore copies
/// only declared fields and never treats native padding as serialized data.
#[derive(Clone, Copy)]
pub(crate) struct FlatFieldDescriptor {
    native_offset: u16,
    wire_len: u16,
    /// Zero admits every byte; otherwise bit N admits exactly byte N (N < 16).
    byte_domain: u16,
}

impl FlatFieldDescriptor {
    pub(crate) const fn new(native_offset: usize, wire_len: usize, byte_domain: u16) -> Self {
        // These are compile-time offsets and lengths of fixed account fields,
        // never user-selected quantities. Check before narrowing metadata.
        assert!(native_offset <= u16::MAX as usize);
        assert!(wire_len <= u16::MAX as usize);
        Self {
            native_offset: native_offset as u16,
            wire_len: wire_len as u16,
            byte_domain,
        }
    }
}

const _: () = assert!(core::mem::size_of::<FlatFieldDescriptor>() == 6);

/// The immutable metadata shared by all flat state codecs.
#[derive(Clone, Copy)]
pub(crate) struct FlatCodecDescriptor {
    wire_len: usize,
    fields: &'static [FlatFieldDescriptor],
}

impl FlatCodecDescriptor {
    pub(crate) const fn new(
        native_size: usize,
        wire_len: usize,
        fields: &'static [FlatFieldDescriptor],
    ) -> Self {
        // Production descriptors are constants. Validate immutable metadata
        // once at construction instead of repeating its checks on every load.
        // Private fields keep safe callers from bypassing these invariants.
        let mut wire_offset = 0usize;
        let mut index = 0usize;
        while index < fields.len() {
            let field = &fields[index];
            let start = field.native_offset as usize;
            let length = field.wire_len as usize;
            let end = start + length; // Both operands are bounded u16 values.
            assert!(end <= native_size, "flat field exceeds native allocation");
            assert!(
                length <= wire_len - wire_offset,
                "flat fields exceed wire length"
            );
            wire_offset += length;
            let mut prior = 0usize;
            while prior < index {
                let other = &fields[prior];
                let other_start = other.native_offset as usize;
                let other_end = other_start + other.wire_len as usize;
                assert!(
                    length == 0 || other.wire_len == 0 || end <= other_start || other_end <= start,
                    "flat fields overlap"
                );
                prior += 1;
            }
            index += 1;
        }
        assert!(
            wire_offset == wire_len,
            "flat fields do not cover wire length"
        );
        Self { wire_len, fields }
    }
}

/// Types admitted by `fixed_state_deserialize_flat!`.
///
/// This is deliberately an opt-in marker. Only primitives and the explicitly
/// audited repr(u8) enums below qualify. Nested records use explicit leaf lists.
/// Options and unaudited layouts fail to compile rather than guessing bytes.
pub(crate) trait FlatFieldType {
    const WIRE_LEN: usize;
    const BYTE_DOMAIN: u16;
}

macro_rules! flat_scalar_type {
    ($($type:ty),+ $(,)?) => {
        $(
            impl FlatFieldType for $type {
                const WIRE_LEN: usize = core::mem::size_of::<$type>();
                const BYTE_DOMAIN: u16 = 0;
            }
        )+
    };
}

impl FlatFieldType for bool {
    const WIRE_LEN: usize = 1;
    const BYTE_DOMAIN: u16 = 0b11;
}

flat_scalar_type!(u8, u16, u32, i32, u64, u128, i64);

impl FlatFieldType for Pubkey {
    const WIRE_LEN: usize = 32;
    const BYTE_DOMAIN: u16 = 0;
}

impl<T: FlatFieldType, const LENGTH: usize> FlatFieldType for [T; LENGTH] {
    const WIRE_LEN: usize = T::WIRE_LEN * LENGTH;
    const BYTE_DOMAIN: u16 = T::BYTE_DOMAIN;
}

// Every opt-in type below already declares #[repr(u8)] and the same explicit
// discriminants in state/*.rs and stable_borsh_enum!. Check native size,
// alignment and tags here; the exhaustive match makes added variants require
// an explicit codec review. No enum declaration or representation is changed.
macro_rules! flat_enum_type {
    ($type:ident { $($variant:ident = $tag:expr),+ $(,)? }) => {
        const _: () = {
            assert!(core::mem::size_of::<$type>() == 1);
            assert!(core::mem::align_of::<$type>() == 1);
            $(assert!($tag < 16); assert!($type::$variant as u8 == $tag);)+
        };
        const _: fn($type) -> u8 = |value| match value {
            $($type::$variant => $tag,)+
        };
        impl FlatFieldType for $type {
            const WIRE_LEN: usize = 1;
            const BYTE_DOMAIN: u16 = 0 $(| (1u16 << $tag))+;
        }
    };
}

flat_enum_type!(OracleSourceStatus {
    Candidate = 0, Frozen = 1, Inactive = 2, Rejected = 3,
    OpeningPending = 4, Active = 5, Merged = 6, TimedOut = 7,
});
flat_enum_type!(OracleChallengeStatus {
    Open = 0, RuleReview = 1, RuleReviewUnresolved = 2,
    Accepted = 3, Rejected = 4, Cancelled = 5,
});
flat_enum_type!(OracleClaimStatus {
    Open = 0, Committed = 1, Revealed = 2, Finalized = 3,
    Rejected = 4, TimedOut = 5,
});
flat_enum_type!(OracleOpeningClaimStatus {
    Empty = 0, Pending = 1, Challenged = 2, Accepted = 3,
    Rejected = 4, TimedOut = 5,
});
flat_enum_type!(OracleUsdcRewardKind {
    SourceProposer = 0, SourceSupport = 1, Opening = 2, Update = 3,
});
flat_enum_type!(OracleBucketMedianStatus {
    Live = 0, Dirty = 1, SettlementReady = 2, GraceRequired = 3,
    EmergencyRequired = 4, EmergencyDefaulted = 5, EmergencyRejected = 6,
    EmergencyConfirmed = 7,
});
flat_enum_type!(OracleRecipeWeightPhase {
    Collecting = 0, ReadyToFinalize = 2, Finalized = 3,
});
flat_enum_type!(OracleUsdcRewardSchedulePhase {
    Building = 0, Funded = 1, EntitlementsFinalized = 2, Aborted = 3,
});
flat_enum_type!(OracleEscrowDisposition {
    Unsettled = 0, Refunded = 1, Slashed = 2, Transferred = 3,
});
/// Keep the common scalar and key copies at a constant length so the SBF
/// compiler can emit them directly instead of calling memcpy for every field.
/// Larger arrays retain the same general copy path.
///
/// # Safety
/// The source and destination must satisfy `copy_nonoverlapping::<u8>` for
/// exactly `length` bytes. Neither pointer needs scalar alignment.
#[inline(always)]
unsafe fn copy_flat_field(source: *const u8, destination: *mut u8, length: usize) {
    match length {
        1 => core::ptr::copy_nonoverlapping(source, destination, 1),
        2 => core::ptr::copy_nonoverlapping(source, destination, 2),
        4 => core::ptr::copy_nonoverlapping(source, destination, 4),
        8 => core::ptr::copy_nonoverlapping(source, destination, 8),
        16 => core::ptr::copy_nonoverlapping(source, destination, 16),
        32 => core::ptr::copy_nonoverlapping(source, destination, 32),
        _ => core::ptr::copy_nonoverlapping(source, destination, length),
    }
}

/// Copy the declared flat fields from a native value to the Borsh wire.
///
/// # Safety
/// `native` must point to the initialized value whose allocation and fields
/// were supplied to the descriptor constructor. `output` must point to
/// `output_len` writable bytes. The two allocations must not overlap.
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
        let wire_len = usize::from(field.wire_len);
        let native_offset = usize::from(field.native_offset);
        // The constructor proves all metadata ranges and the exact wire sum;
        // the output length check covers every copy. Only declared typed
        // fields are copied, including their explicitly serialized padding.
        copy_flat_field(native.add(native_offset), output.add(wire_offset), wire_len);
        wire_offset += wire_len;
    }

    true
}

/// Decode declared flat fields into uninitialized native storage.
///
/// Validation is completed before any typed value is assumed initialized.  In
/// particular, every bool and opted-in enum byte is checked against its exact
/// Borsh domain before the corresponding byte can become part of a `T`.
///
/// # Safety
/// `destination` must point to writable `MaybeUninit<T>` storage matching the
/// allocation and fields supplied to the descriptor constructor. It must not
/// alias `data` for a nonzero copy.
#[inline(never)]
pub(crate) unsafe fn decode_flat_fields(
    descriptor: &FlatCodecDescriptor,
    data: &[u8],
    destination: *mut u8,
) -> std::io::Result<()> {
    if data.len() < descriptor.wire_len {
        return Err(invalid_fixed_borsh());
    }

    // The constructor proved metadata ranges and the exact wire sum. Validate
    // every restricted input byte before any destination write.
    let mut wire_offset = 0usize;
    for field in descriptor.fields {
        let wire_len = usize::from(field.wire_len);
        let wire_end = wire_offset + wire_len;
        if field.byte_domain != 0
            && data[wire_offset..wire_end]
                .iter()
                .any(|byte| *byte >= 16 || field.byte_domain & (1u16 << *byte) == 0)
        {
            return Err(invalid_fixed_borsh());
        }
        wire_offset = wire_end;
    }
    if !crate::bytes_are_zero(&data[descriptor.wire_len..]) {
        return Err(invalid_fixed_borsh());
    }

    // Second pass: all input validation has completed, so no invalid bool, enum or
    // other invalid typed representation can be observed through `T`.
    wire_offset = 0;
    for field in descriptor.fields {
        copy_flat_field(
            data.as_ptr().add(wire_offset),
            destination.add(usize::from(field.native_offset)),
            usize::from(field.wire_len),
        );
        wire_offset += usize::from(field.wire_len);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_length_copies_preserve_unaligned_bytes_and_guards() {
        let source: Vec<u8> = (0..528).map(|index| (index * 37 + 19) as u8).collect();
        for length in (0..=80).chain([128, 256, 512]) {
            for source_offset in 0..8 {
                for destination_offset in 0..8 {
                    let mut actual = vec![0xa7; length + 16];
                    let mut expected = actual.clone();
                    expected[destination_offset..destination_offset + length]
                        .copy_from_slice(&source[source_offset..source_offset + length]);
                    unsafe {
                        copy_flat_field(
                            source.as_ptr().add(source_offset),
                            actual.as_mut_ptr().add(destination_offset),
                            length,
                        );
                    }
                    assert_eq!(actual, expected);
                }
            }
        }
    }

    const fn fields(domain: u16) -> [FlatFieldDescriptor; 2] {
        [
            FlatFieldDescriptor::new(1, 1, 0),
            FlatFieldDescriptor::new(4, 2, domain),
        ]
    }
    const DOMAINS: [u16; 8] = [0b11, 0xff, 0x3f, 0x0f, 0x7f, 0x0d, 0x1ef, 0x8001];
    const FIELDS: [[FlatFieldDescriptor; 2]; 8] = [
        fields(DOMAINS[0]),
        fields(DOMAINS[1]),
        fields(DOMAINS[2]),
        fields(DOMAINS[3]),
        fields(DOMAINS[4]),
        fields(DOMAINS[5]),
        fields(DOMAINS[6]),
        fields(DOMAINS[7]),
    ];

    #[test]
    fn exact_domains_validate_every_array_byte_before_any_destination_write() {
        assert_eq!(core::mem::size_of::<FlatFieldDescriptor>(), 6);
        for (index, domain) in DOMAINS.into_iter().enumerate() {
            let descriptor = FlatCodecDescriptor::new(8, 3, &FIELDS[index]);
            for byte in 0..=u8::MAX {
                let admitted = byte < 16 && domain & (1 << byte) != 0;
                for input in [[0x5a, byte, 0], [0x5a, 0, byte]] {
                    let mut destination = [0xa7; 8];
                    let result = unsafe {
                        decode_flat_fields(&descriptor, &input, destination.as_mut_ptr())
                    };
                    if admitted {
                        result.unwrap();
                        assert_eq!(
                            destination,
                            [0xa7, 0x5a, 0xa7, 0xa7, input[1], input[2], 0xa7, 0xa7]
                        );
                        let mut encoded = [0x19; 5];
                        assert!(unsafe {
                            encode_flat_fields(
                                &descriptor,
                                destination.as_ptr(),
                                encoded.as_mut_ptr(),
                                encoded.len(),
                            )
                        });
                        assert_eq!(encoded, [input[0], input[1], input[2], 0x19, 0x19]);
                    } else {
                        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
                        assert_eq!(destination, [0xa7; 8]);
                    }
                }
            }
        }
    }

    #[test]
    fn compact_descriptors_preserve_range_padding_and_truncation_checks() {
        let valid = FlatCodecDescriptor::new(8, 3, &FIELDS[0]);
        for input in [&[][..], &[1][..], &[1, 0][..], &[1, 0, 1, 9][..]] {
            let mut destination = [0xa7; 8];
            assert!(
                unsafe { decode_flat_fields(&valid, input, destination.as_mut_ptr()) }.is_err()
            );
            assert_eq!(destination, [0xa7; 8]);
        }
        let mut destination = [0xa7; 8];
        unsafe { decode_flat_fields(&valid, &[1, 0, 1, 0, 0], destination.as_mut_ptr()) }.unwrap();
        assert_eq!(destination, [0xa7, 1, 0xa7, 0xa7, 0, 1, 0xa7, 0xa7]);
        // Invalid static layouts now fail at their only constructor, before
        // they can be passed to either runtime codec.
        for (native_size, wire_len) in [(5, 3), (8, 2), (8, 4)] {
            assert!(std::panic::catch_unwind(|| {
                FlatCodecDescriptor::new(native_size, wire_len, &FIELDS[0])
            })
            .is_err());
        }
        let source = [0; 8];
        let mut short_output = [0x19; 2];
        assert!(!unsafe {
            encode_flat_fields(
                &valid,
                source.as_ptr(),
                short_output.as_mut_ptr(),
                short_output.len(),
            )
        });
        assert_eq!(short_output, [0x19; 2]);
    }

    #[test]
    fn descriptor_narrowing_is_checked_instead_of_wrapping() {
        let limit = usize::from(u16::MAX);
        let exact = FlatFieldDescriptor::new(limit, limit, 0);
        assert_eq!(usize::from(exact.native_offset), limit);
        assert_eq!(usize::from(exact.wire_len), limit);
        assert!(std::panic::catch_unwind(|| FlatFieldDescriptor::new(limit + 1, 1, 0)).is_err());
        assert!(std::panic::catch_unwind(|| FlatFieldDescriptor::new(1, limit + 1, 0)).is_err());
    }

    #[test]
    fn descriptor_constructor_rejects_overlaps_and_accepts_disjoint_native_order() {
        const OVERLAP: [FlatFieldDescriptor; 2] = [
            FlatFieldDescriptor::new(1, 3, 0),
            FlatFieldDescriptor::new(3, 2, 0),
        ];
        const DUPLICATE: [FlatFieldDescriptor; 2] = [
            FlatFieldDescriptor::new(1, 2, 0),
            FlatFieldDescriptor::new(1, 2, 0),
        ];
        assert!(std::panic::catch_unwind(|| FlatCodecDescriptor::new(8, 5, &OVERLAP)).is_err());
        assert!(std::panic::catch_unwind(|| FlatCodecDescriptor::new(8, 4, &DUPLICATE)).is_err());

        // Rust may reorder fields; wire order must remain independently declared.
        const REVERSED: [FlatFieldDescriptor; 2] = [
            FlatFieldDescriptor::new(4, 2, 0),
            FlatFieldDescriptor::new(1, 1, 0),
        ];
        let descriptor = FlatCodecDescriptor::new(6, 3, &REVERSED);
        let mut destination = [0xa7; 6];
        unsafe { decode_flat_fields(&descriptor, &[3, 5, 7], destination.as_mut_ptr()) }.unwrap();
        assert_eq!(destination, [0xa7, 7, 0xa7, 0xa7, 3, 5]);
        let mut wire = [0; 3];
        assert!(unsafe {
            encode_flat_fields(
                &descriptor,
                destination.as_ptr(),
                wire.as_mut_ptr(),
                wire.len(),
            )
        });
        assert_eq!(wire, [3, 5, 7]);
    }
}
