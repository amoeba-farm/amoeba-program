use super::*;

/// Enforce the effective account privileges and key separation emitted by the canonical writer
/// builders.
///
/// Solana unions privileges for duplicate account keys before program entry. Requiring both the
/// positive and negative privilege bits rejects cleared required privileges and effective
/// escalation, except for the runtime's unavoidable writable promotion of a required signer used
/// as the transaction fee payer. Raw account-key uniqueness closes the semantic-alias case that
/// effective privileges alone cannot observe. The only protocol-defined alias is the canonical
/// Flat reconcile shape, where the sleeve is also the target authority.
pub(super) fn validate_pack_writer_account_privileges(
    tag: VaultInstructionTag,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let count = accounts.len();
    let fixed_count = match tag {
        VaultInstructionTag::DepositWriterPrincipalV1 => Some(18),
        VaultInstructionTag::WithdrawWriterPrincipalV1 => Some(14),
        VaultInstructionTag::CommitWriterAuctionV1 => Some(14),
        VaultInstructionTag::PrepareWriterBidIndexV1 => Some(10),
        VaultInstructionTag::PlaceWriterBidV1 => Some(19),
        VaultInstructionTag::CancelOrRefundWriterBidV1 => Some(9),
        VaultInstructionTag::RevealWriterAuctionV1 => Some(6),
        VaultInstructionTag::ExecuteWriterAuctionFillV1 => Some(26),
        VaultInstructionTag::BeginWriterCloseV1 => Some(15),
        VaultInstructionTag::DepositWriterCloseBasketV1 => Some(13),
        VaultInstructionTag::ClaimCollectiveLongV1 => Some(20),
        VaultInstructionTag::ClaimWriterFlatResidualV1 => Some(17),
        VaultInstructionTag::ReconcileWriterSupplyV1 => Some(16),
        _ => None,
    };
    if let Some(expected) = fixed_count {
        if count != expected {
            return Err(VaultError::InvalidAccountList.into());
        }
    } else {
        let valid_dynamic_count = match tag {
            VaultInstructionTag::FinalizeWriterCloseV1 => {
                (17..=14 + 3 * crate::constants::WRITER_MAX_LIVE_SERIES).contains(&count)
                    && (count - 14).is_multiple_of(3)
            }
            VaultInstructionTag::ProcessWriterCloseCancellationV1 => {
                matches!(count, 11 | 13)
            }
            VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => {
                (10..=9 + crate::constants::WRITER_MAX_LIVE_SERIES).contains(&count)
            }
            _ => return Ok(()),
        };
        if !valid_dynamic_count {
            return Err(VaultError::InvalidAccountList.into());
        }
    }

    for (index, account) in accounts.iter().enumerate() {
        let expected_signer = match tag {
            VaultInstructionTag::DepositWriterPrincipalV1
            | VaultInstructionTag::WithdrawWriterPrincipalV1
            | VaultInstructionTag::CommitWriterAuctionV1
            | VaultInstructionTag::PrepareWriterBidIndexV1
            | VaultInstructionTag::PlaceWriterBidV1
            | VaultInstructionTag::CancelOrRefundWriterBidV1
            | VaultInstructionTag::RevealWriterAuctionV1
            | VaultInstructionTag::ExecuteWriterAuctionFillV1
            | VaultInstructionTag::BeginWriterCloseV1
            | VaultInstructionTag::DepositWriterCloseBasketV1
            | VaultInstructionTag::FinalizeWriterCloseV1
            | VaultInstructionTag::ProcessWriterCloseCancellationV1
            | VaultInstructionTag::FinalizeWriterSleeveSettlementV1
            | VaultInstructionTag::ClaimCollectiveLongV1
            | VaultInstructionTag::ClaimWriterFlatResidualV1
            | VaultInstructionTag::ReconcileWriterSupplyV1 => index == 0,
            _ => false,
        };
        let expected_writable = match tag {
            VaultInstructionTag::DepositWriterPrincipalV1 => {
                matches!(index, 0 | 2 | 4 | 5 | 7 | 8 | 9 | 12 | 16)
            }
            VaultInstructionTag::WithdrawWriterPrincipalV1 => {
                matches!(index, 0 | 2 | 3 | 4 | 6 | 7 | 8 | 9)
            }
            VaultInstructionTag::CommitWriterAuctionV1 => matches!(index, 0 | 3 | 7 | 8 | 9),
            VaultInstructionTag::PrepareWriterBidIndexV1 => matches!(index, 0 | 8),
            VaultInstructionTag::PlaceWriterBidV1 => {
                matches!(index, 0 | 2 | 3 | 4 | 7 | 8 | 10 | 18)
            }
            VaultInstructionTag::CancelOrRefundWriterBidV1 => {
                matches!(index, 2..=6)
            }
            VaultInstructionTag::RevealWriterAuctionV1 => matches!(index, 3 | 4),
            VaultInstructionTag::ExecuteWriterAuctionFillV1 => matches!(
                index,
                0 | 2 | 4 | 6 | 7 | 8 | 9 | 10 | 11 | 13 | 14 | 15 | 16 | 17 | 20 | 24
            ),
            VaultInstructionTag::BeginWriterCloseV1 => {
                matches!(index, 0 | 2 | 6 | 7 | 8 | 9 | 10)
            }
            VaultInstructionTag::DepositWriterCloseBasketV1 => {
                matches!(index, 0 | 3 | 6 | 7 | 8)
            }
            VaultInstructionTag::FinalizeWriterCloseV1 => {
                matches!(index, 2 | 4 | 6 | 7 | 8 | 10 | 11) || index >= 14
            }
            VaultInstructionTag::ProcessWriterCloseCancellationV1 if count == 13 => {
                matches!(index, 0 | 1 | 3 | 6 | 7 | 8)
            }
            VaultInstructionTag::ProcessWriterCloseCancellationV1 => {
                matches!(index, 0 | 1 | 2 | 4 | 5 | 6)
            }
            VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => matches!(index, 2 | 4),
            VaultInstructionTag::ClaimCollectiveLongV1 => {
                matches!(index, 0 | 2 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 13 | 19)
            }
            VaultInstructionTag::ClaimWriterFlatResidualV1 => {
                matches!(index, 0 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 10 | 16)
            }
            VaultInstructionTag::ReconcileWriterSupplyV1 => {
                matches!(index, 2 | 4 | 6 | 7 | 8 | 9 | 10)
            }
            _ => false,
        };
        // The transaction fee payer is always promoted to writable by the runtime. Accept that
        // one unavoidable promotion when this role is already the required signer; every other
        // missing or additional effective privilege remains invalid.
        let writable_matches = account.is_writable == expected_writable
            || (expected_signer && !expected_writable && account.is_writable);
        if account.is_signer != expected_signer || !writable_matches {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    if count > WRITER_MAX_PACK_ACCOUNTS {
        return Err(VaultError::InvalidAccountList.into());
    }
    // A full Pubkey comparison lowers to an expensive 32-byte memory comparison on SBF. Most
    // canonical packs contain dozens of distinct keys, so first reject unequal eight-byte
    // prefixes with integer comparisons. A matching prefix still performs the full comparison,
    // preserving exact uniqueness semantics (prefix collisions are never treated as aliases).
    let mut key_prefixes = [0u64; WRITER_MAX_PACK_ACCOUNTS];
    for (index, account) in accounts.iter().enumerate() {
        let bytes = account.key.as_ref();
        // SAFETY: every Pubkey exposes 32 initialized bytes, and `read_unaligned` imposes no
        // alignment requirement. Byte order is irrelevant because this value is compared only
        // with prefixes loaded by this exact operation.
        key_prefixes[index] = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<u64>()) };
    }
    for left in 0..accounts.len() {
        for right in left + 1..accounts.len() {
            if key_prefixes[left] != key_prefixes[right]
                || accounts[left].key != accounts[right].key
            {
                continue;
            }
            let canonical_flat_reconcile_alias = tag
                == VaultInstructionTag::ReconcileWriterSupplyV1
                && payload == [0, 1]
                && left == 2
                && right == 6;
            if !canonical_flat_reconcile_alias {
                return Err(VaultError::InvalidAccountList.into());
            }
        }
    }
    Ok(())
}
