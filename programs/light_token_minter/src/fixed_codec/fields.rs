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
    const REQUIRED_DATA_LEN: usize;

    /// # Safety
    /// `data` must contain at least `Self::REQUIRED_DATA_LEN` bytes.
    unsafe fn decode_fixed(data: &[u8]) -> std::io::Result<Self>;
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
        impl FixedField for $type {
            // Keep the shared cursor read out of line in SBF builds. Inlining this
            // shim into every generated state decoder materially duplicates code.
            #[inline(never)]
            fn read(input: &mut FixedCursor<'_>) -> Self {
                input.$reader()
            }

            #[inline(always)]
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

fixed_enum_field!(OptionKind {
    0 => OptionKind::CallSpread,
    1 => OptionKind::PutSpread,
});
fixed_enum_field!(ManageWriterPolicyAuthorityActionV1 {
    0 => ManageWriterPolicyAuthorityActionV1::Propose,
    1 => ManageWriterPolicyAuthorityActionV1::Activate,
    2 => ManageWriterPolicyAuthorityActionV1::Cancel,
});
fixed_enum_field!(WriterSecurityMode {
    0 => WriterSecurityMode::GrossExternalMaxPayout,
    1 => WriterSecurityMode::ExactExternalEnvelope,
});
fixed_enum_field!(WriterReserveRoundingMode {
    0 => WriterReserveRoundingMode::AggregateBookCeiling,
});
fixed_enum_field!(WriterAuctionPriorityRule {
    0 => WriterAuctionPriorityRule::PayAsBidPriceThenSeriesProRata,
});
fixed_enum_field!(WriterSettlementGroupStatus {
    0 => WriterSettlementGroupStatus::Anchored,
    1 => WriterSettlementGroupStatus::Active,
    2 => WriterSettlementGroupStatus::Settled,
    3 => WriterSettlementGroupStatus::Closed,
});
fixed_enum_field!(WriterSleeveStatus {
    0 => WriterSleeveStatus::Draft,
    1 => WriterSleeveStatus::PolicyFrozen,
    2 => WriterSleeveStatus::Funding,
    3 => WriterSleeveStatus::Active,
    4 => WriterSleeveStatus::CloseStaging,
    5 => WriterSleeveStatus::Expired,
    6 => WriterSleeveStatus::SettlementFinalized,
    7 => WriterSleeveStatus::Closed,
});
fixed_enum_field!(WriterSeriesCustodyStatus {
    0 => WriterSeriesCustodyStatus::Absent,
    1 => WriterSeriesCustodyStatus::Open,
    2 => WriterSeriesCustodyStatus::Closed,
});
fixed_enum_field!(WriterSeriesSettlementStatus {
    0 => WriterSeriesSettlementStatus::Open,
    1 => WriterSeriesSettlementStatus::Frozen,
    2 => WriterSeriesSettlementStatus::Exhausted,
});
fixed_enum_field!(WriterAuctionStatus {
    0 => WriterAuctionStatus::Committed,
    1 => WriterAuctionStatus::Bidding,
    2 => WriterAuctionStatus::Revealed,
    3 => WriterAuctionStatus::Planning,
    4 => WriterAuctionStatus::Executing,
    5 => WriterAuctionStatus::Finalized,
    6 => WriterAuctionStatus::Refundable,
    7 => WriterAuctionStatus::Closed,
});
fixed_enum_field!(WriterBidStatus {
    0 => WriterBidStatus::Empty,
    1 => WriterBidStatus::Funded,
    2 => WriterBidStatus::Cancelled,
    3 => WriterBidStatus::Planned,
    4 => WriterBidStatus::Executed,
    5 => WriterBidStatus::Refundable,
    6 => WriterBidStatus::Refunded,
});
fixed_enum_field!(WriterBidDeliveryMode {
    0 => WriterBidDeliveryMode::LightToken,
    1 => WriterBidDeliveryMode::ClassicSpl,
});
fixed_enum_field!(WriterCloseRequestStatus {
    0 => WriterCloseRequestStatus::Collecting,
    1 => WriterCloseRequestStatus::Complete,
    2 => WriterCloseRequestStatus::Finalized,
    3 => WriterCloseRequestStatus::Cancelling,
    4 => WriterCloseRequestStatus::Cancelled,
});

impl FixedField for WriterSeriesRecordV1 {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        Self {
            active: FixedField::read(input),
            option_kind: FixedField::read(input),
            custody_status: FixedField::read(input),
            settlement_status: FixedField::read(input),
            reserved: FixedField::read(input),
            series_id: FixedField::read(input),
            market: FixedField::read(input),
            contract_mint: FixedField::read(input),
            retirement_custody: FixedField::read(input),
            strike_price_atomic: FixedField::read(input),
            cap_or_floor_price_atomic: FixedField::read(input),
            contract_size_atoms: FixedField::read(input),
            max_payout_per_contract_atoms: FixedField::read(input),
            total_physical_supply_atoms: FixedField::read(input),
            issuer_controlled_atoms: FixedField::read(input),
            external_open_interest_atoms: FixedField::read(input),
            primary_premium_collected_atoms: FixedField::read(input),
            settlement_external_oi_snapshot_atoms: FixedField::read(input),
            settlement_liability_initial_atoms: FixedField::read(input),
            settlement_liability_remaining_atoms: FixedField::read(input),
            payoff_digest: FixedField::read(input),
        }
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        FixedField::write(&self.active, output);
        FixedField::write(&self.option_kind, output);
        FixedField::write(&self.custody_status, output);
        FixedField::write(&self.settlement_status, output);
        FixedField::write(&self.reserved, output);
        FixedField::write(&self.series_id, output);
        FixedField::write(&self.market, output);
        FixedField::write(&self.contract_mint, output);
        FixedField::write(&self.retirement_custody, output);
        FixedField::write(&self.strike_price_atomic, output);
        FixedField::write(&self.cap_or_floor_price_atomic, output);
        FixedField::write(&self.contract_size_atoms, output);
        FixedField::write(&self.max_payout_per_contract_atoms, output);
        FixedField::write(&self.total_physical_supply_atoms, output);
        FixedField::write(&self.issuer_controlled_atoms, output);
        FixedField::write(&self.external_open_interest_atoms, output);
        FixedField::write(&self.primary_premium_collected_atoms, output);
        FixedField::write(&self.settlement_external_oi_snapshot_atoms, output);
        FixedField::write(&self.settlement_liability_initial_atoms, output);
        FixedField::write(&self.settlement_liability_remaining_atoms, output);
        FixedField::write(&self.payoff_digest, output);
    }
}

impl FixedField for [WriterSeriesRecordV1; WRITER_SERIES_STORAGE_CAPACITY] {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        core::array::from_fn(|_| WriterSeriesRecordV1::read(input))
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for record in self {
            record.write(output);
        }
    }
}

impl FixedField for Box<[WriterSeriesRecordV1; WRITER_SERIES_STORAGE_CAPACITY]> {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        (0..WRITER_SERIES_STORAGE_CAPACITY)
            .map(|_| WriterSeriesRecordV1::read(input))
            .collect::<Vec<_>>()
            .into_boxed_slice()
            .try_into()
            .expect("fixed writer series capacity")
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for record in self.iter() {
            record.write(output);
        }
    }
}

impl FixedField for WriterBidIndexRecordV1 {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        Self {
            occupied: FixedField::read(input),
            status: FixedField::read(input),
            series_index: FixedField::read(input),
            reserved: FixedField::read(input),
            bid_price_per_contract_atoms: FixedField::read(input),
            requested_contract_atoms: FixedField::read(input),
            accepted_contract_atoms: FixedField::read(input),
            executed_contract_atoms: FixedField::read(input),
            escrowed_atoms: FixedField::read(input),
            bid: FixedField::read(input),
            bidder: FixedField::read(input),
            order_id: FixedField::read(input),
        }
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        FixedField::write(&self.occupied, output);
        FixedField::write(&self.status, output);
        FixedField::write(&self.series_index, output);
        FixedField::write(&self.reserved, output);
        FixedField::write(&self.bid_price_per_contract_atoms, output);
        FixedField::write(&self.requested_contract_atoms, output);
        FixedField::write(&self.accepted_contract_atoms, output);
        FixedField::write(&self.executed_contract_atoms, output);
        FixedField::write(&self.escrowed_atoms, output);
        FixedField::write(&self.bid, output);
        FixedField::write(&self.bidder, output);
        FixedField::write(&self.order_id, output);
    }
}

impl FixedField for [WriterBidIndexRecordV1; WRITER_BID_STORAGE_CAPACITY] {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        core::array::from_fn(|_| WriterBidIndexRecordV1::read(input))
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for record in self {
            record.write(output);
        }
    }
}

impl FixedField for Box<[WriterBidIndexRecordV1; WRITER_BID_STORAGE_CAPACITY]> {
    #[inline(never)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        (0..WRITER_BID_STORAGE_CAPACITY)
            .map(|_| WriterBidIndexRecordV1::read(input))
            .collect::<Vec<_>>()
            .into_boxed_slice()
            .try_into()
            .expect("fixed writer bid capacity")
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        for record in self.iter() {
            record.write(output);
        }
    }
}
fixed_enum_field!(AmoebaDlmmPoolStatus {
    0 => AmoebaDlmmPoolStatus::Pending,
    1 => AmoebaDlmmPoolStatus::Active,
    2 => AmoebaDlmmPoolStatus::Paused,
    3 => AmoebaDlmmPoolStatus::Settled,
    4 => AmoebaDlmmPoolStatus::Closed,
});
fixed_enum_field!(CompressionState {
    0 => CompressionState::Uninitialized,
    1 => CompressionState::Decompressed,
    2 => CompressionState::Compressed,
});

impl FixedField for RentConfig {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        Self {
            base_rent: input.u16(),
            compression_cost: input.u16(),
            lamports_per_byte_per_epoch: input.u8(),
            max_funded_epochs: input.u8(),
            max_top_up: input.u16(),
        }
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        output.u16(self.base_rent);
        output.u16(self.compression_cost);
        output.u8(self.lamports_per_byte_per_epoch);
        output.u8(self.max_funded_epochs);
        output.u16(self.max_top_up);
    }
}

impl FixedField for CompressionInfo {
    #[inline(always)]
    fn read(input: &mut FixedCursor<'_>) -> Self {
        Self {
            last_claimed_slot: input.u64(),
            lamports_per_write: input.u32(),
            config_version: input.u16(),
            state: <CompressionState as FixedField>::read(input),
            _padding: input.u8(),
            rent_config: <RentConfig as FixedField>::read(input),
        }
    }

    #[inline(always)]
    fn write(&self, output: &mut FixedWriter<'_>) {
        output.u64(self.last_claimed_slot);
        output.u32(self.lamports_per_write);
        output.u16(self.config_version);
        <CompressionState as FixedField>::write(&self.state, output);
        output.u8(self._padding);
        <RentConfig as FixedField>::write(&self.rent_config, output);
    }
}
fixed_enum_field!(OracleEmergencyDisputeKind {
    0 => OracleEmergencyDisputeKind::Source,
    1 => OracleEmergencyDisputeKind::Update,
    2 => OracleEmergencyDisputeKind::Opening,
    3 => OracleEmergencyDisputeKind::BucketMedian,
});
fixed_enum_field!(OracleBucketMedianStatus {
    0 => OracleBucketMedianStatus::Live,
    1 => OracleBucketMedianStatus::Dirty,
    2 => OracleBucketMedianStatus::SettlementReady,
    3 => OracleBucketMedianStatus::GraceRequired,
    4 => OracleBucketMedianStatus::EmergencyRequired,
    5 => OracleBucketMedianStatus::EmergencyDefaulted,
    6 => OracleBucketMedianStatus::EmergencyRejected,
});
fixed_enum_field!(OracleChallengeStatus {
    0 => OracleChallengeStatus::Open,
    1 => OracleChallengeStatus::RuleReview,
    2 => OracleChallengeStatus::RuleReviewUnresolved,
    3 => OracleChallengeStatus::Accepted,
    4 => OracleChallengeStatus::Rejected,
    5 => OracleChallengeStatus::Cancelled,
});
fixed_enum_field!(OracleRecipeWeightPhase {
    0 => OracleRecipeWeightPhase::Collecting,
    2 => OracleRecipeWeightPhase::ReadyToFinalize,
    3 => OracleRecipeWeightPhase::Finalized,
});
fixed_enum_field!(OracleSambaEmergencyPayoutMode {
    0 => OracleSambaEmergencyPayoutMode::Open,
    1 => OracleSambaEmergencyPayoutMode::Redistribute,
    2 => OracleSambaEmergencyPayoutMode::RefundAll,
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
fixed_enum_field!(OracleUsdcRewardSchedulePhase {
    0 => OracleUsdcRewardSchedulePhase::Building,
    1 => OracleUsdcRewardSchedulePhase::Funded,
    2 => OracleUsdcRewardSchedulePhase::EntitlementsFinalized,
    3 => OracleUsdcRewardSchedulePhase::Aborted,
});
fixed_enum_field!(SettlementStyle {
    0 => SettlementStyle::CashSettledMonthly,
});
fixed_enum_field!(OracleEmergencyVoteStatus {
    0 => OracleEmergencyVoteStatus::Empty,
    1 => OracleEmergencyVoteStatus::Committed,
    2 => OracleEmergencyVoteStatus::Revealed,
    3 => OracleEmergencyVoteStatus::Expired,
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
