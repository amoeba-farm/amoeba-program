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
/// audited repr(u8) enums below qualify, as does the audited padding-free
/// four-u64 writer policy row. Other nested records use explicit leaf lists.
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
flat_enum_type!(OracleUsdcRewardSchedulePhase {
    Building = 0, Funded = 1, EntitlementsFinalized = 2, Aborted = 3,
});
flat_enum_type!(OracleEscrowDisposition {
    Unsettled = 0, Refunded = 1, Slashed = 2, Transferred = 3,
});
flat_enum_type!(OracleBucketMedianStatus {
    Live = 0, Dirty = 1, SettlementReady = 2, GraceRequired = 3,
    EmergencyRequired = 4, EmergencyDefaulted = 5, EmergencyRejected = 6,
});
flat_enum_type!(OracleRecipeWeightPhase {
    Collecting = 0, ReadyToFinalize = 2, Finalized = 3,
});

flat_enum_type!(WriterSecurityMode {
    GrossExternalMaxPayout = 0, ExactExternalEnvelope = 1,
});
flat_enum_type!(WriterReserveRoundingMode {
    AggregateBookCeiling = 0,
});
flat_enum_type!(WriterSettlementGroupStatus {
    Anchored = 0, Active = 1, Settled = 2, Closed = 3, FundingExpired = 4,
});
flat_enum_type!(WriterSleeveStatus {
    Draft = 0, PolicyFrozen = 1, Funding = 2, Active = 3,
    Expired = 5, SettlementFinalized = 6, Closed = 7, FundingRefunds = 8,
});
flat_enum_type!(AmoebaDlmmPoolStatus {
    Pending = 0, Active = 1, Paused = 2, Settled = 3, Closed = 4,
});
flat_enum_type!(CompressionState {
    Uninitialized = 0, Decompressed = 1, Compressed = 2,
});
flat_enum_type!(OptionKind {
    CallSpread = 0, PutSpread = 1,
});
flat_enum_type!(SettlementStyle {
    CashSettledMonthly = 0,
});
flat_enum_type!(ManageWriterPolicyAuthorityActionV1 {
    Propose = 0, Activate = 1, Cancel = 2,
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
