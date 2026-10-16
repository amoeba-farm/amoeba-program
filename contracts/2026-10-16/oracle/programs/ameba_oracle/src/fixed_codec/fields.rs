use super::*;

pub(crate) trait FixedField: Sized {
    fn read(input: &mut FixedCursor<'_>) -> Self;
    fn write(&self, output: &mut FixedWriter<'_>);
}

/// Object-safe account writer used by the runtime store path. The generated implementation writes
/// the same field sequence as Borsh directly into the already-sized account buffer, avoiding a
/// temporary `Vec` and one generic `store_state` copy per concrete account type.
pub(crate) trait FixedStateEncode {
    fn maximum_encoded_len(&self) -> usize;
    fn encode_fixed(&self, data: &mut [u8]);
}

pub(crate) trait FixedStateDecode: Sized {
    #[cfg_attr(not(test), allow(dead_code))]
    const REQUIRED_DATA_LEN: usize;

    /// # Safety
    /// `data` must contain at least `Self::REQUIRED_DATA_LEN` bytes.
    unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self>;

    /// Internal account loaders select their existing program error. Returning
    /// only success/failure avoids carrying an unused IO error alongside state.
    ///
    /// # Safety
    /// `data` must contain at least `Self::REQUIRED_DATA_LEN` bytes.
    #[inline(always)]
    unsafe fn decode_fixed_option(data: &[u8]) -> Option<Self> {
        Self::decode_fixed(data).ok()
    }
}

impl<T: FixedStateEncode + ?Sized> FixedStateEncode for Box<T> {
    #[inline(always)]
    fn maximum_encoded_len(&self) -> usize {
        self.as_ref().maximum_encoded_len()
    }

    #[inline(always)]
    fn encode_fixed(&self, data: &mut [u8]) {
        self.as_ref().encode_fixed(data);
    }
}

macro_rules! primitive_fixed_field {
    ($type:ty, $reader:ident, $writer:ident) => {
        primitive_fixed_field!($type, $reader, $writer, always);
    };
    ($type:ty, $reader:ident, $writer:ident, $writer_inline:ident) => {
        impl FixedField for $type {
            // Keep the shared cursor read out of line in SBF builds. Inlining this
            // shim into every generated state decoder materially duplicates code.
            #[inline(never)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                input.$reader()
            }

            #[inline($writer_inline)]
            fn write(&self, output: &mut FixedWriter<'_>) {
                output.$writer(*self);
            }
        }
    };
}

primitive_fixed_field!(bool, bool, bool);
primitive_fixed_field!(u8, u8, u8);
primitive_fixed_field!(u16, u16, u16);
primitive_fixed_field!(u32, u32, u32);
primitive_fixed_field!(i32, i32, i32);
primitive_fixed_field!(u64, u64, u64);
primitive_fixed_field!(u128, u128, u128);
primitive_fixed_field!(i64, i64, i64);

impl FixedField for Pubkey {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        input.pubkey()
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        output.pubkey(self);
    }
}

impl FixedField for Option<Pubkey> {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        input.optional_pubkey()
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        output.optional_pubkey(self);
    }
}

impl<const LENGTH: usize> FixedField for [u8; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        input.bytes()
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        if LENGTH == 32 {
            // SAFETY: This branch is monomorphized only for the 32-byte array implementation.
            output.bytes32(unsafe { &*(self.as_ptr().cast::<[u8; 32]>()) });
        } else {
            output.bytes(self);
        }
    }
}

impl<const LENGTH: usize> FixedField for [Pubkey; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let mut value = [Pubkey::new_from_array([0; 32]); LENGTH];
        let byte_len = LENGTH * 32;
        // `Pubkey` is repr(transparent) over `[u8; 32]`, and every byte pattern is valid.
        unsafe {
            std::ptr::copy_nonoverlapping(
                input.data.as_ptr().add(input.offset),
                value.as_mut_ptr().cast::<u8>(),
                byte_len,
            );
        }
        input.offset += byte_len;
        value
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        let byte_len = LENGTH * 32;
        // `Pubkey` is repr(transparent) over `[u8; 32]`.
        let bytes = unsafe { std::slice::from_raw_parts(self.as_ptr().cast::<u8>(), byte_len) };
        output.raw(bytes);
    }
}

impl<const LENGTH: usize> FixedField for [[u8; 32]; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let mut value = [[0; 32]; LENGTH];
        let byte_len = LENGTH * 32;
        unsafe {
            std::ptr::copy_nonoverlapping(
                input.data.as_ptr().add(input.offset),
                value.as_mut_ptr().cast::<u8>(),
                byte_len,
            );
        }
        input.offset += byte_len;
        value
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        let byte_len = LENGTH * 32;
        let bytes = unsafe { std::slice::from_raw_parts(self.as_ptr().cast::<u8>(), byte_len) };
        output.raw(bytes);
    }
}

impl<const LENGTH: usize> FixedField for [u64; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let mut value = [0; LENGTH];
        let byte_len = LENGTH * core::mem::size_of::<u64>();
        // Solana SBF is little-endian, so its in-memory integer representation
        // is byte-identical to the rigid Borsh array encoding.
        unsafe {
            std::ptr::copy_nonoverlapping(
                input.data.as_ptr().add(input.offset),
                value.as_mut_ptr().cast::<u8>(),
                byte_len,
            );
        }
        input.offset += byte_len;
        value
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        let byte_len = LENGTH * core::mem::size_of::<u64>();
        let bytes = unsafe { std::slice::from_raw_parts(self.as_ptr().cast::<u8>(), byte_len) };
        output.raw(bytes);
    }
}

impl<const LENGTH: usize> FixedField for [i64; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let mut value = [0; LENGTH];
        let byte_len = LENGTH * core::mem::size_of::<i64>();
        // Solana SBF is little-endian, so the signed in-memory representation is byte-identical
        // to the rigid Borsh array encoding.
        unsafe {
            std::ptr::copy_nonoverlapping(
                input.data.as_ptr().add(input.offset),
                value.as_mut_ptr().cast::<u8>(),
                byte_len,
            );
        }
        input.offset += byte_len;
        value
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        let byte_len = LENGTH * core::mem::size_of::<i64>();
        let bytes = unsafe { std::slice::from_raw_parts(self.as_ptr().cast::<u8>(), byte_len) };
        output.raw(bytes);
    }
}

impl<const LENGTH: usize> FixedField for [u128; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let mut value = [0; LENGTH];
        let byte_len = LENGTH * core::mem::size_of::<u128>();
        unsafe {
            std::ptr::copy_nonoverlapping(
                input.data.as_ptr().add(input.offset),
                value.as_mut_ptr().cast::<u8>(),
                byte_len,
            );
        }
        input.offset += byte_len;
        value
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        let byte_len = LENGTH * core::mem::size_of::<u128>();
        let bytes = unsafe { std::slice::from_raw_parts(self.as_ptr().cast::<u8>(), byte_len) };
        output.raw(bytes);
    }
}

impl<const LENGTH: usize> FixedField for [u16; LENGTH] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        let mut value = [0; LENGTH];
        for item in &mut value {
            *item = input.u16();
        }
        value
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for item in self {
            output.u16(*item);
        }
    }
}

impl FixedField for [u32; 3] {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        [input.u32(), input.u32(), input.u32()]
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        output.u32(self[0]);
        output.u32(self[1]);
        output.u32(self[2]);
    }
}

macro_rules! fixed_enum_field {
    ($type:ty { $($value:expr => $variant:path),+ $(,)? }) => {
        impl FixedField for $type {
            #[inline(never)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                match input.u8() {
                    $($value => $variant,)+
                    _ => {
                        input.invalid = true;
                        fixed_enum_field!(@first $($variant),+)
                    }
                }
            }

            #[inline(always)]
            fn write(&self, output: &mut FixedWriter<'_>) {
                let value = match self {
                    $($variant => $value,)+
                };
                output.u8(value);
            }
        }

    };
    (@first $first:path $(, $rest:path)*) => { $first };
}

fixed_enum_field!(OracleEmergencyDisputeKind {
    0 => OracleEmergencyDisputeKind::Source,
    1 => OracleEmergencyDisputeKind::Update,
    2 => OracleEmergencyDisputeKind::Opening,
    3 => OracleEmergencyDisputeKind::BucketMedian,
});
fixed_enum_field!(OracleChallengeStatus {
    0 => OracleChallengeStatus::Open,
    1 => OracleChallengeStatus::RuleReview,
    2 => OracleChallengeStatus::RuleReviewUnresolved,
    3 => OracleChallengeStatus::Accepted,
    4 => OracleChallengeStatus::Rejected,
    5 => OracleChallengeStatus::Cancelled,
});
fixed_enum_field!(OracleSourceStatus {
    0 => OracleSourceStatus::Candidate,
    1 => OracleSourceStatus::Frozen,
    2 => OracleSourceStatus::Inactive,
    3 => OracleSourceStatus::Rejected,
    4 => OracleSourceStatus::OpeningPending,
    5 => OracleSourceStatus::Active,
    6 => OracleSourceStatus::Merged,
    7 => OracleSourceStatus::TimedOut,
});
fixed_enum_field!(OracleEscrowDisposition {
    0 => OracleEscrowDisposition::Unsettled,
    1 => OracleEscrowDisposition::Refunded,
    2 => OracleEscrowDisposition::Slashed,
    3 => OracleEscrowDisposition::Transferred,
});
fixed_enum_field!(OracleBucketMedianStatus {
    0 => OracleBucketMedianStatus::Live,
    1 => OracleBucketMedianStatus::Dirty,
    2 => OracleBucketMedianStatus::SettlementReady,
    3 => OracleBucketMedianStatus::GraceRequired,
    4 => OracleBucketMedianStatus::EmergencyRequired,
    5 => OracleBucketMedianStatus::EmergencyDefaulted,
    6 => OracleBucketMedianStatus::EmergencyRejected,
    7 => OracleBucketMedianStatus::EmergencyConfirmed,
});
fixed_enum_field!(OracleRecipeWeightPhase {
    0 => OracleRecipeWeightPhase::Collecting,
    2 => OracleRecipeWeightPhase::ReadyToFinalize,
    3 => OracleRecipeWeightPhase::Finalized,
});
fixed_enum_field!(OracleUsdcRewardSchedulePhase {
    0 => OracleUsdcRewardSchedulePhase::Building,
    1 => OracleUsdcRewardSchedulePhase::Funded,
    2 => OracleUsdcRewardSchedulePhase::EntitlementsFinalized,
    3 => OracleUsdcRewardSchedulePhase::Aborted,
});
fixed_enum_field!(OracleClaimStatus {
    0 => OracleClaimStatus::Open,
    1 => OracleClaimStatus::Committed,
    2 => OracleClaimStatus::Revealed,
    3 => OracleClaimStatus::Finalized,
    4 => OracleClaimStatus::Rejected,
    5 => OracleClaimStatus::TimedOut,
});
fixed_enum_field!(OracleUsdcRewardKind {
    0 => OracleUsdcRewardKind::SourceProposer,
    1 => OracleUsdcRewardKind::SourceSupport,
    2 => OracleUsdcRewardKind::Opening,
    3 => OracleUsdcRewardKind::Update,
});
fixed_enum_field!(OraclePhase {
    0 => OraclePhase::Uninitialized,
    1 => OraclePhase::Scramble,
    2 => OraclePhase::Game,
    3 => OraclePhase::Settled,
    4 => OraclePhase::Closed,
    5 => OraclePhase::Opening,
    6 => OraclePhase::SourceSubmission,
});
fixed_enum_field!(OracleSettlementStatus {
    0 => OracleSettlementStatus::Final,
    1 => OracleSettlementStatus::Provisional,
    2 => OracleSettlementStatus::FrozenPendingEvidence,
});
fixed_enum_field!(OracleOpeningClaimStatus {
    0 => OracleOpeningClaimStatus::Empty,
    1 => OracleOpeningClaimStatus::Pending,
    2 => OracleOpeningClaimStatus::Challenged,
    3 => OracleOpeningClaimStatus::Accepted,
    4 => OracleOpeningClaimStatus::Rejected,
    5 => OracleOpeningClaimStatus::TimedOut,
});
