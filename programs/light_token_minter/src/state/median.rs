use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OracleBucketMedianStatus {
    #[default]
    Live = 0,
    Dirty = 1,
    SettlementReady = 2,
    GraceRequired = 3,
    EmergencyRequired = 4,
    EmergencyDefaulted = 5,
    EmergencyRejected = 6,
}
stable_borsh_enum!(OracleBucketMedianStatus {
    Live = 0,
    Dirty = 1,
    SettlementReady = 2,
    GraceRequired = 3,
    EmergencyRequired = 4,
    EmergencyDefaulted = 5,
    EmergencyRejected = 6
});

/// Canonical per-bucket median anchor. Source cash weights are deliberately absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleBucketMedianState {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub month: Pubkey,
    pub bucket_id: [u8; 32],
    /// Zero-based position in the active manifest's canonical ascending bucket walk.
    pub group_index: u16,
    /// Cumulative immutable product weight preceding this bucket.
    pub bucket_weight_start_bps: u16,
    pub bucket_weight_bps: u16,
    pub frozen_source_count: u16,
    pub active_source_count: u16,
    pub eligible_source_count: u16,
    pub status: OracleBucketMedianStatus,
    pub bucket_delta_bps: i64,
    pub last_recomputed_ts: u64,
    pub source_snapshot_hash: [u8; 32],
    /// Number of frozen sources visited in authenticated order, including inactive sources.
    pub recompute_processed_source_count: u16,
    pub last_recompute_source_id: [u8; 32],
    pub emergency_snapshot_slot: u64,
    pub emergency_snapshot_total_samba: u64,
    /// Temporary opening deltas accumulated while the active-source manifest is built.
    pub opening_source_deltas_bps: [i64; crate::constants::MAX_ORACLE_BUCKET_SOURCES],
}

impl Default for OracleBucketMedianState {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            month: Pubkey::default(),
            bucket_id: [0; 32],
            group_index: 0,
            bucket_weight_start_bps: 0,
            bucket_weight_bps: 0,
            frozen_source_count: 0,
            active_source_count: 0,
            eligible_source_count: 0,
            status: OracleBucketMedianStatus::Live,
            bucket_delta_bps: 0,
            last_recomputed_ts: 0,
            source_snapshot_hash: [0; 32],
            recompute_processed_source_count: 0,
            last_recompute_source_id: [0; 32],
            emergency_snapshot_slot: 0,
            emergency_snapshot_total_samba: 0,
            opening_source_deltas_bps: [0; crate::constants::MAX_ORACLE_BUCKET_SOURCES],
        }
    }
}

impl OracleBucketMedianState {
    pub const LEN: usize = 256;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OMB";
    pub const ACCOUNT_VERSION: u8 = 3;
}

pub fn derive_oracle_bucket_median_pda(
    program_id: &Pubkey,
    month: &Pubkey,
    bucket_id: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            ORACLE_BUCKET_MEDIAN_PDA_SEED,
            month.as_ref(),
            bucket_id,
        ],
        program_id,
    )
}
