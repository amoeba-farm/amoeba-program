use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;
use std::mem::MaybeUninit;

use crate::{
    compression::{
        apply_compressed_state_leaf_mutations, derive_compressed_state_leaf_address,
        CompressedStateLeafClose, CompressedStateLeafCreate, CompressedStateLeafReadOnly,
        CompressedStateLeafUpdate,
    },
    constants::{
        MAX_COMPRESSED_INNER_INSTRUCTION_BYTES, MAX_COMPRESSED_STATE_SESSION_RECORDS,
        ORACLE_SAMBA_WINNING_VOTE_PDA_SEED, ORACLE_SKU_COVERAGE_RECORD_PDA_SEED,
        ORACLE_SOURCE_PDA_SEED, ORACLE_SUPPORT_POSITION_PDA_SEED,
        ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED, ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED,
        ORACLE_USDC_SKU_POOL_PDA_SEED, ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
    },
    error::VaultError,
    fixed_codec::{
        invalid_fixed_borsh, FixedCursor, FixedField, FixedStateDecode, FixedStateEncode,
        FixedWriter,
    },
    instruction::{CompressedStateAccess, ExecuteCompressedStateParams, VaultInstructionTag},
    state::{
        derive_oracle_samba_vote_settlement_pda, derive_oracle_samba_winning_vote_pda,
        derive_oracle_sku_coverage_record_pda, derive_oracle_usdc_reward_receipt_pda,
        derive_oracle_usdc_reward_registration_pda, derive_oracle_usdc_reward_schedule_pda,
        derive_oracle_usdc_reward_vault_pda, derive_oracle_usdc_sku_pool_pda,
        derive_oracle_usdc_source_reward_pda, CompressedAmebaStateLeaf, CompressedStateDomain,
        OracleEscrowDisposition, OracleSambaVoteSettlementReceipt, OracleSambaWinningVote,
        OracleSkuCoverageRecord, OracleSourceState, OracleSourceStatus, OracleSupportPosition,
        OracleUsdcRewardKind, OracleUsdcRewardReceipt, OracleUsdcRewardRegistration,
        OracleUsdcRewardSchedule, OracleUsdcSkuPool, OracleUsdcSourceReward,
    },
};

struct CaptureVec<T> {
    items: [MaybeUninit<T>; MAX_COMPRESSED_STATE_SESSION_RECORDS],
    len: u8,
}

impl<T> CaptureVec<T> {
    #[inline(always)]
    fn new() -> Self {
        Self {
            items: [const { MaybeUninit::uninit() }; MAX_COMPRESSED_STATE_SESSION_RECORDS],
            len: 0,
        }
    }

    #[inline(always)]
    fn push(&mut self, value: T) {
        debug_assert!(usize::from(self.len) < self.items.len());
        self.items[usize::from(self.len)].write(value);
        self.len += 1;
    }

    #[inline(always)]
    fn as_slice(&self) -> &[T] {
        // SAFETY: `push` initializes every element below `len`, and the session-wide access bound
        // prevents capacity overflow.
        unsafe { std::slice::from_raw_parts(self.items.as_ptr().cast::<T>(), self.len.into()) }
    }
}

impl<T> Drop for CaptureVec<T> {
    fn drop(&mut self) {
        for item in &mut self.items[..usize::from(self.len)] {
            // SAFETY: Every element below `len` was initialized exactly once by `push`.
            unsafe { item.assume_init_drop() };
        }
    }
}

/// Only fields that cannot be reconstructed from canonical instruction accounts are stored in
/// Light. This is deliberately byte-for-byte lossless for the mutable hot-account view presented to
/// the transition core.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleUsdcSkuPool {
    bucket_id: [u8; 32],
    source_reward_budget: u64,
    remaining_source_reward_budget: u64,
    opening_reward_budget: u64,
    remaining_opening_reward_budget: u64,
    update_reward_budget: u64,
    remaining_update_reward_budget: u64,
    proposer_reward_bps: u16,
    listing_bond: u64,
    support_bond: u64,
    opening_bond: u64,
    update_min_bond: u64,
    challenge_min_bond: u64,
    challenge_max_bond: u64,
    challenge_bond_bps: u16,
    registered_source_count: u32,
    registered_opening_count: u32,
    registered_update_count: u32,
    registered_update_reward_units: u32,
    last_updated_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleUsdcSourceReward {
    source: Pubkey,
    supporter_count: u32,
    registered: bool,
    terminal_status: OracleSourceStatus,
    opening_claim: Pubkey,
    last_updated_slot: u64,
    merged_into_source: Pubkey,
    max_merge_depth: u8,
    listing_escrow_counted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleSkuCoverageRecord {
    sku_id: [u8; 32],
    sku_index: u16,
    active_supported_source_count: u16,
    last_updated_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleUsdcRewardRegistration {
    sku_pool: Pubkey,
    subject: Pubkey,
    recipient: Pubkey,
    reward_units: u8,
    last_updated_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleUsdcRewardReceipt {
    kind: OracleUsdcRewardKind,
    subject: Pubkey,
    recipient: Pubkey,
    amount: u64,
    claimed_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleSambaWinningVote {
    base_entitlement: u64,
    registered_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleSambaVoteSettlementReceipt {
    amount: u64,
    disposition: OracleEscrowDisposition,
    settled_slot: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleSupportPosition {
    source: Pubkey,
    supporter: Pubkey,
    support_stake: u64,
    released: bool,
    escrow_disposition: OracleEscrowDisposition,
    failed_schedule_escrow_counted: bool,
    /// The source may already have been compacted when a retained merge-lineage reward is
    /// claimed. Keep its immutable logical id in the support leaf so materialization never
    /// requires a second source leaf merely to reconstruct the unchanged typed account.
    source_id: [u8; 32],
}

/// Mutable source state plus the immutable identities used by ordinary lifecycle checks. The
/// three large immutable descriptor hashes live in a separate read-only compressed leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleSourceState {
    source_id: [u8; 32],
    bucket_id: [u8; 32],
    proposer: Pubkey,
    baseline_state: u64,
    current_state: u64,
    listing_bond_locked: u64,
    support_stake_total: u64,
    bucket_weight_bps: u16,
    status: OracleSourceStatus,
    opening_submitted: bool,
    opening_evidence_hash: [u8; 32],
    last_finalized_step: u64,
    observation_count: u8,
    rolling_observation_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompactOracleSourceDescriptor {
    source_type_hash: [u8; 32],
    canonical_locator_hash: [u8; 32],
    source_definition_hash: [u8; 32],
}

crate::fixed_codec::fixed_state_deserialize!(CompactOracleUsdcSkuPool, 156, {
    bucket_id: [u8; 32],
    source_reward_budget: u64,
    remaining_source_reward_budget: u64,
    opening_reward_budget: u64,
    remaining_opening_reward_budget: u64,
    update_reward_budget: u64,
    remaining_update_reward_budget: u64,
    proposer_reward_bps: u16,
    listing_bond: u64,
    support_bond: u64,
    opening_bond: u64,
    update_min_bond: u64,
    challenge_min_bond: u64,
    challenge_max_bond: u64,
    challenge_bond_bps: u16,
    registered_source_count: u32,
    registered_opening_count: u32,
    registered_update_count: u32,
    registered_update_reward_units: u32,
    last_updated_slot: u64,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleUsdcSourceReward, 112, {
    source: Pubkey,
    supporter_count: u32,
    registered: bool,
    terminal_status: OracleSourceStatus,
    opening_claim: Pubkey,
    last_updated_slot: u64,
    merged_into_source: Pubkey,
    max_merge_depth: u8,
    listing_escrow_counted: bool,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleSkuCoverageRecord, 44, {
    sku_id: [u8; 32],
    sku_index: u16,
    active_supported_source_count: u16,
    last_updated_slot: u64,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleUsdcRewardRegistration, 105, {
    sku_pool: Pubkey,
    subject: Pubkey,
    recipient: Pubkey,
    reward_units: u8,
    last_updated_slot: u64,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleUsdcRewardReceipt, 81, {
    kind: OracleUsdcRewardKind,
    subject: Pubkey,
    recipient: Pubkey,
    amount: u64,
    claimed_slot: u64,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleSambaWinningVote, 16, {
    base_entitlement: u64,
    registered_slot: u64,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleSambaVoteSettlementReceipt, 17, {
    amount: u64,
    disposition: OracleEscrowDisposition,
    settled_slot: u64,
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleSupportPosition, 107, {
    source: Pubkey,
    supporter: Pubkey,
    support_stake: u64,
    released: bool,
    escrow_disposition: OracleEscrowDisposition,
    failed_schedule_escrow_counted: bool,
    source_id: [u8; 32],
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleSourceState, 205, {
    source_id: [u8; 32],
    bucket_id: [u8; 32],
    proposer: Pubkey,
    baseline_state: u64,
    current_state: u64,
    listing_bond_locked: u64,
    support_stake_total: u64,
    bucket_weight_bps: u16,
    status: OracleSourceStatus,
    opening_submitted: bool,
    opening_evidence_hash: [u8; 32],
    last_finalized_step: u64,
    observation_count: u8,
    rolling_observation_hash: [u8; 32],
});

crate::fixed_codec::fixed_state_deserialize!(CompactOracleSourceDescriptor, 96, {
    source_type_hash: [u8; 32],
    canonical_locator_hash: [u8; 32],
    source_definition_hash: [u8; 32],
});

impl CompactOracleUsdcSkuPool {}

impl CompactOracleUsdcSourceReward {}

impl CompactOracleUsdcRewardRegistration {}

impl CompactOracleUsdcRewardReceipt {}

impl CompactOracleSambaWinningVote {}

impl CompactOracleSambaVoteSettlementReceipt {}

impl CompactOracleSupportPosition {}

impl CompactOracleSourceState {}

impl CompactOracleSourceDescriptor {}

impl CompactOracleSkuCoverageRecord {}

mod contracts;
mod execute;
mod materialize;
mod validation;

pub(super) use contracts::*;
pub(super) use execute::*;
pub(super) use materialize::*;
pub(super) use validation::*;
