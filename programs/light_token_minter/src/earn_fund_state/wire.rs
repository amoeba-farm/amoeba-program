//! The on-chain decoder of `ManageEarnFundV1` payloads: a fixed-cursor reader
//! of exactly the bytes the Borsh codec defines (differentially tested
//! against it), without the generic `std::io::Read` machinery.
use super::*;
use crate::fixed_codec::CheckedCursor;

#[inline(never)]
pub(crate) fn proof(cursor: &mut CheckedCursor) -> Option<[u8; 128]> {
    match cursor.u8() {
        0 => None,
        1 => Some(cursor.bytes::<128>()),
        _ => {
            cursor.invalid = true;
            None
        }
    }
}

#[inline(never)]
pub(crate) fn tree_info(cursor: &mut CheckedCursor) -> PackedStateTreeInfo {
    PackedStateTreeInfo {
        root_index: cursor.u16(),
        prove_by_index: cursor.boolean(),
        merkle_tree_pubkey_index: cursor.u8(),
        queue_pubkey_index: cursor.u8(),
        leaf_index: cursor.u32(),
    }
}

#[inline(never)]
pub(crate) fn witness(cursor: &mut CheckedCursor) -> EarnFundPositionWitness {
    match cursor.u8() {
        0 => EarnFundPositionWitness::New {
            address_tree_info: PackedAddressTreeInfo {
                address_merkle_tree_pubkey_index: cursor.u8(),
                address_queue_pubkey_index: cursor.u8(),
                root_index: cursor.u16(),
            },
            output_state_tree_index: cursor.u8(),
        },
        1 => EarnFundPositionWitness::Live {
            tree_info: tree_info(cursor),
            output_state_tree_index: cursor.u8(),
            state: PositionLedger {
                shares: cursor.u64(),
                pending_atoms: cursor.u64(),
                pending_epoch: cursor.u64(),
                queue_shares: cursor.u64(),
                queue_batch_id: cursor.u64(),
                queue_paid_atoms: cursor.u64(),
                claimable_atoms: cursor.u64(),
                deposited_atoms: cursor.u64(),
                withdrawn_atoms: cursor.u64(),
            },
        },
        2 => EarnFundPositionWitness::Closed {
            tree_info: tree_info(cursor),
            output_state_tree_index: cursor.u8(),
        },
        _ => {
            cursor.invalid = true;
            EarnFundPositionWitness::Closed {
                tree_info: PackedStateTreeInfo::default(),
                output_state_tree_index: 0,
            }
        }
    }
}

#[inline(never)]
pub(crate) fn params(cursor: &mut CheckedCursor) -> FundParams {
    FundParams {
        buffer_bps: cursor.u16(),
        instant_daily_cap_bps: cursor.u16(),
        max_third_party_bps: cursor.u16(),
        min_tenor_secs: cursor.u64(),
        max_tenor_secs: cursor.u64(),
        min_epoch_secs: cursor.u64(),
        min_allocation_atoms: cursor.u64(),
        allocator: cursor.bytes(),
        paused: cursor.boolean(),
        // Struct expressions evaluate in source order: the cursor reads the
        // fields in wire order.
        buyback: cursor.bytes(),
    }
}

#[inline(never)]
pub(crate) fn source(c: &mut CheckedCursor) -> EarnFundDepositSource {
    match c.u8() {
        0 => EarnFundDepositSource::Classic,
        1 => EarnFundDepositSource::Compressed {
            amount: c.u64(),
            leaf_index: c.u32(),
            root_index: c.u16(),
            prove_by_index: c.boolean(),
            tree_index: c.u8(),
            queue_index: c.u8(),
            proof: proof(c),
        },
        _ => {
            c.invalid = true;
            EarnFundDepositSource::Classic
        }
    }
}

/// A Withdraw or Deposit mode byte (0..=2).
pub(crate) fn mode(c: &mut CheckedCursor) -> EarnFundWithdrawMode {
    match c.u8() {
        0 => EarnFundWithdrawMode::InstantElseQueue,
        1 => EarnFundWithdrawMode::QueueOnly,
        2 => EarnFundWithdrawMode::InstantOnly,
        _ => {
            c.invalid = true;
            EarnFundWithdrawMode::QueueOnly
        }
    }
}

pub(crate) fn op(c: &mut CheckedCursor) -> u8 {
    let op = c.u8();
    if op > 8 {
        c.invalid = true;
    }
    op
}
