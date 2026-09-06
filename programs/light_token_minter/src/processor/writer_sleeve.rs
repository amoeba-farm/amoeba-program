use super::*;
use crate::{
    constants::{
        MAX_AMOEBA_DLMM_BINS_PER_SWAP, MAX_AMOEBA_DLMM_BIN_COUNT, MAX_AMOEBA_DLMM_SWAP_FEE_BPS,
    },
    instruction::{
        BeginWriterCloseV1Params, ClaimCollectiveLongV1Params, ClaimWriterFlatResidualV1Params,
        CleanupWriterCustodyV1Params, CommitWriterAuctionV1Params,
        FinalizeOrAbortWriterAuctionV1Params, InitializeWriterPolicyRegistryV1Params,
        ManageWriterPolicyAuthorityActionV1, ManageWriterPolicyAuthorityV1Params,
        PlaceWriterBidV1Params, PlanWriterAuctionChunkV1Params,
        ProcessWriterCloseCancellationV1Params, ReconcileWriterSupplyV1Params,
        RevealWriterAuctionV1Params, SealWriterPolicyV1Params, SetCollectiveMarketPausedV1Params,
        WriterAmountV1Params, WriterSeriesIndexV1Params,
    },
    state::{
        WriterAuctionPriorityRule, WriterAuctionStatus, WriterAuctionV1, WriterBidIndexRecordV1,
        WriterBidIndexV1, WriterBidStatus, WriterBidV1, WriterCloseRequestStatus,
        WriterCloseRequestV1, WriterPolicyRegistryV1, WriterPolicySnapshotV1,
        WriterReserveRoundingMode, WriterSecurityMode, WriterSeriesBookV1,
        WriterSeriesCustodyStatus, WriterSeriesRecordV1, WriterSeriesSettlementStatus,
        WriterSettlementGroupStatus, WriterSettlementGroupV1, WriterSleeveStatus, WriterSleeveV1,
    },
    writer_sleeve_math::{
        drawdown_checks, exact_reserve, maximum_safe_issue_quantity, proportional_close_preview,
        security_exposure as calculate_security_exposure, WriterIssueAdmissionLimits,
        WriterMathError, WriterSecurityMode as WriterMathSecurityMode, WriterSeries,
    },
};

mod accounts;
mod auction;
mod close;
mod funding;
mod reconcile;
mod settlement;

use accounts::*;

const WRITER_SERIES_FAMILY_HASH_DOMAIN: &[u8] = b"ameba-writer-series-family-v1";
const WRITER_BOOK_HASH_DOMAIN: &[u8] = b"ameba-writer-book-v1";
const WRITER_PAYOFF_HASH_DOMAIN: &[u8] = b"ameba-writer-payoff-v1";
const WRITER_RISK_HASH_DOMAIN: &[u8] = b"ameba-writer-risk-v1";
const WRITER_POLICY_HASH_DOMAIN: &[u8] = b"ameba-writer-policy-v1";
const WRITER_COVERAGE_HASH_DOMAIN: &[u8] = b"ameba-writer-coverage-v1";
const WRITER_SERIES_FAMILY_HASH_MAX_BYTES: usize =
    WRITER_SERIES_FAMILY_HASH_DOMAIN.len() + 4 + crate::constants::WRITER_MAX_LIVE_SERIES * 64;
const WRITER_BOOK_HASH_MAX_BYTES: usize = WRITER_BOOK_HASH_DOMAIN.len()
    + 32
    + 32
    + 4
    + crate::constants::WRITER_MAX_LIVE_SERIES * (32 + 32 + 7 * 8);
const WRITER_MAX_PACK_ACCOUNTS: usize = 13 + 3 * crate::constants::WRITER_MAX_LIVE_SERIES;

// This append path is shared by the bounded family/book encoders. Keeping it
// out of line avoids duplicating the same checked copy at every SBF call site.
#[inline(never)]
fn append_hash_bytes<const N: usize>(buffer: &mut [u8; N], len: &mut usize, value: &[u8]) {
    let end = *len + value.len();
    buffer[*len..end].copy_from_slice(value);
    *len = end;
}

#[inline(never)]
fn decode_and_process<T: BorshDeserialize>(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
    handler: fn(&Pubkey, &[AccountInfo], T) -> ProgramResult,
) -> ProgramResult {
    let params: T = decode_instruction_payload(payload)?;
    handler(program_id, accounts, params)
}

#[inline(never)]
fn empty_and_process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
    handler: fn(&Pubkey, &[AccountInfo]) -> ProgramResult,
) -> ProgramResult {
    expect_empty_payload(payload)?;
    handler(program_id, accounts)
}

/// Enforce the effective account privileges and key separation emitted by the canonical writer
/// builders.
///
/// Solana unions privileges for duplicate account keys before program entry. Requiring both the
/// positive and negative privilege bits rejects cleared required privileges and effective
/// escalation, except for the runtime's unavoidable writable promotion of a required signer used
/// as the transaction fee payer. Raw account-key uniqueness closes the semantic-alias case that
/// effective privileges alone cannot observe. The only protocol-defined alias is the canonical
/// Flat reconcile shape, where the sleeve is also the target authority.
fn validate_pack_writer_account_privileges(
    tag: VaultInstructionTag,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let count = accounts.len();
    let fixed_count = match tag {
        VaultInstructionTag::DepositWriterPrincipalV1 => Some(17),
        VaultInstructionTag::WithdrawWriterPrincipalV1 => Some(14),
        VaultInstructionTag::CommitWriterAuctionV1 => Some(14),
        VaultInstructionTag::PlaceWriterBidV1 => Some(13),
        VaultInstructionTag::CancelOrRefundWriterBidV1 => Some(9),
        VaultInstructionTag::RevealWriterAuctionV1 => Some(6),
        VaultInstructionTag::ExecuteWriterAuctionFillV1 => Some(26),
        VaultInstructionTag::BeginWriterCloseV1 => Some(15),
        VaultInstructionTag::DepositWriterCloseBasketV1 => Some(13),
        VaultInstructionTag::ClaimCollectiveLongV1 => Some(20),
        VaultInstructionTag::ClaimWriterFlatResidualV1 => Some(17),
        VaultInstructionTag::ReconcileWriterSupplyV1 => Some(15),
        _ => None,
    };
    if let Some(expected) = fixed_count {
        if count != expected {
            return Err(VaultError::InvalidAccountList.into());
        }
    } else {
        let valid_dynamic_count = match tag {
            VaultInstructionTag::FinalizeWriterCloseV1 => {
                (16..=13 + 3 * crate::constants::WRITER_MAX_LIVE_SERIES).contains(&count)
                    && (count - 13).is_multiple_of(3)
            }
            VaultInstructionTag::ProcessWriterCloseCancellationV1 => {
                matches!(count, 11 | 13)
            }
            VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => {
                (9..=8 + crate::constants::WRITER_MAX_LIVE_SERIES).contains(&count)
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
            VaultInstructionTag::PlaceWriterBidV1 => matches!(index, 0 | 2 | 3 | 4 | 7 | 8),
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
                matches!(index, 2 | 4 | 6 | 7 | 8 | 10 | 11) || index >= 13
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

#[inline(never)]
pub(super) fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: VaultInstructionTag,
    payload: &[u8],
    compressed_state_transport: bool,
) -> ProgramResult {
    if compressed_state_transport {
        return Err(VaultError::InvalidInstructionData.into());
    }
    validate_pack_writer_account_privileges(tag, accounts, payload)?;
    match tag {
        VaultInstructionTag::InitializeWriterPolicyRegistryV1 => {
            decode_and_process::<InitializeWriterPolicyRegistryV1Params>(
                program_id,
                accounts,
                payload,
                process_initialize_policy_registry,
            )
        }
        VaultInstructionTag::ManageWriterPolicyAuthorityV1 => {
            decode_and_process::<ManageWriterPolicyAuthorityV1Params>(
                program_id,
                accounts,
                payload,
                process_manage_policy_authority,
            )
        }
        VaultInstructionTag::InitializeWriterSettlementGroupV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            process_initialize_settlement_group,
        ),
        VaultInstructionTag::InitializeWriterSleeveV1 => {
            empty_and_process(program_id, accounts, payload, process_initialize_sleeve)
        }
        VaultInstructionTag::RegisterWriterSeriesV1 => {
            empty_and_process(program_id, accounts, payload, process_register_series)
        }
        VaultInstructionTag::SealWriterPolicyV1 => decode_and_process::<SealWriterPolicyV1Params>(
            program_id,
            accounts,
            payload,
            process_seal_policy,
        ),
        VaultInstructionTag::OpenWriterFundingV1 => {
            empty_and_process(program_id, accounts, payload, process_open_funding)
        }
        VaultInstructionTag::DepositWriterPrincipalV1 => funding::process_deposit_writer_principal(
            program_id,
            accounts,
            WriterAmountV1Params {
                amount_atoms: decode_u64_payload(payload)?,
            },
        ),
        VaultInstructionTag::WithdrawWriterPrincipalV1 => {
            funding::process_withdraw_writer_principal(
                program_id,
                accounts,
                WriterAmountV1Params {
                    amount_atoms: decode_u64_payload(payload)?,
                },
            )
        }
        VaultInstructionTag::ActivateWriterSleeveV1 => {
            empty_and_process(program_id, accounts, payload, process_activate_sleeve)
        }
        VaultInstructionTag::SetCollectiveMarketPausedV1 => process_set_collective_market_paused(
            program_id,
            accounts,
            SetCollectiveMarketPausedV1Params {
                paused: decode_bool_payload(payload)?,
            },
        ),
        VaultInstructionTag::ReconcileWriterSupplyV1 => {
            let [series_index, target_kind] = payload else {
                return Err(VaultError::InvalidInstructionData.into());
            };
            reconcile::process_reconcile_writer_supply(
                program_id,
                accounts,
                ReconcileWriterSupplyV1Params {
                    series_index: *series_index,
                    target_kind: *target_kind,
                },
            )
        }
        VaultInstructionTag::CleanupWriterCustodyV1 => reconcile::process_cleanup_writer_custody(
            program_id,
            accounts,
            CleanupWriterCustodyV1Params {
                series_index: decode_u8_payload(payload)?,
            },
        ),
        VaultInstructionTag::CommitWriterAuctionV1 => {
            decode_and_process::<CommitWriterAuctionV1Params>(
                program_id,
                accounts,
                payload,
                auction::process_commit_writer_auction,
            )
        }
        VaultInstructionTag::PlaceWriterBidV1 => decode_and_process::<PlaceWriterBidV1Params>(
            program_id,
            accounts,
            payload,
            auction::process_place_writer_bid,
        ),
        VaultInstructionTag::CancelOrRefundWriterBidV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            auction::process_cancel_or_refund_writer_bid,
        ),
        VaultInstructionTag::RevealWriterAuctionV1 => {
            decode_and_process::<RevealWriterAuctionV1Params>(
                program_id,
                accounts,
                payload,
                auction::process_reveal_writer_auction,
            )
        }
        VaultInstructionTag::PlanWriterAuctionChunkV1 => {
            let bytes: [u8; 2] = payload
                .try_into()
                .map_err(|_| VaultError::InvalidInstructionData)?;
            auction::process_plan_writer_auction_chunk(
                program_id,
                accounts,
                PlanWriterAuctionChunkV1Params {
                    max_records: u16::from_le_bytes(bytes),
                },
            )
        }
        VaultInstructionTag::ExecuteWriterAuctionFillV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            auction::process_execute_writer_auction_fill,
        ),
        VaultInstructionTag::FinalizeOrAbortWriterAuctionV1 => {
            auction::process_finalize_or_abort_writer_auction(
                program_id,
                accounts,
                FinalizeOrAbortWriterAuctionV1Params {
                    abort: decode_bool_payload(payload)?,
                },
            )
        }
        VaultInstructionTag::BeginWriterCloseV1 => decode_and_process::<BeginWriterCloseV1Params>(
            program_id,
            accounts,
            payload,
            close::process_begin_writer_close,
        ),
        VaultInstructionTag::DepositWriterCloseBasketV1 => {
            close::process_deposit_writer_close_claim(
                program_id,
                accounts,
                WriterSeriesIndexV1Params {
                    series_index: decode_u8_payload(payload)?,
                },
            )
        }
        VaultInstructionTag::FinalizeWriterCloseV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            close::process_finalize_writer_close,
        ),
        VaultInstructionTag::ProcessWriterCloseCancellationV1 => {
            close::process_writer_close_cancellation(
                program_id,
                accounts,
                ProcessWriterCloseCancellationV1Params {
                    selector: decode_u8_payload(payload)?,
                },
            )
        }
        VaultInstructionTag::PublishWriterGroupSettlementV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            settlement::process_publish_writer_group_settlement,
        ),
        VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            settlement::process_finalize_writer_sleeve_settlement,
        ),
        VaultInstructionTag::ClaimCollectiveLongV1 => settlement::process_claim_collective_long(
            program_id,
            accounts,
            ClaimCollectiveLongV1Params {
                claim_atoms: decode_u64_payload(payload)?,
            },
        ),
        VaultInstructionTag::ClaimWriterFlatResidualV1 => {
            settlement::process_claim_writer_flat_residual(
                program_id,
                accounts,
                ClaimWriterFlatResidualV1Params {
                    flat_atoms: decode_u64_payload(payload)?,
                },
            )
        }
        VaultInstructionTag::CloseWriterSleeveV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            settlement::process_close_writer_sleeve,
        ),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

#[inline(always)]
fn writer_security_mode_byte(value: WriterSecurityMode) -> u8 {
    match value {
        WriterSecurityMode::GrossExternalMaxPayout => 0,
        WriterSecurityMode::ExactExternalEnvelope => 1,
    }
}

#[inline(always)]
fn writer_auction_rule_byte(value: WriterAuctionPriorityRule) -> u8 {
    match value {
        WriterAuctionPriorityRule::PayAsBidPriceThenSeriesProRata => 0,
    }
}

pub(super) fn writer_series_family_hash(book: &WriterSeriesBookV1) -> [u8; 32] {
    let count = usize::from(book.series_count);
    let mut bytes = [0u8; WRITER_SERIES_FAMILY_HASH_MAX_BYTES];
    let mut len = 0usize;
    append_hash_bytes(&mut bytes, &mut len, WRITER_SERIES_FAMILY_HASH_DOMAIN);
    append_hash_bytes(
        &mut bytes,
        &mut len,
        &u32::from(book.series_count).to_le_bytes(),
    );
    for record in book.records.iter().take(count) {
        append_hash_bytes(&mut bytes, &mut len, &record.series_id);
        append_hash_bytes(&mut bytes, &mut len, &record.payoff_digest);
    }
    hashv(&[&bytes[..len]]).to_bytes()
}

pub(super) fn writer_book_digest(book: &WriterSeriesBookV1) -> [u8; 32] {
    let count = usize::from(book.series_count);
    let mut bytes = [0u8; WRITER_BOOK_HASH_MAX_BYTES];
    let mut len = 0usize;
    append_hash_bytes(&mut bytes, &mut len, WRITER_BOOK_HASH_DOMAIN);
    append_hash_bytes(&mut bytes, &mut len, book.sleeve.as_ref());
    append_hash_bytes(&mut bytes, &mut len, book.settlement_group.as_ref());
    append_hash_bytes(
        &mut bytes,
        &mut len,
        &u32::from(book.series_count).to_le_bytes(),
    );
    for record in book.records.iter().take(count) {
        append_hash_bytes(&mut bytes, &mut len, &record.series_id);
        append_hash_bytes(&mut bytes, &mut len, &record.payoff_digest);
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.total_physical_supply_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.issuer_controlled_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.external_open_interest_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.primary_premium_collected_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.settlement_external_oi_snapshot_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.settlement_liability_initial_atoms.to_le_bytes(),
        );
        append_hash_bytes(
            &mut bytes,
            &mut len,
            &record.settlement_liability_remaining_atoms.to_le_bytes(),
        );
    }
    hashv(&[&bytes[..len]]).to_bytes()
}

pub(super) fn writer_group_commitment(
    group_key: &Pubkey,
    group: &WriterSettlementGroupV1,
) -> [u8; 32] {
    // Solana's hashv feeds these slices into one SHA-256 state in order, exactly
    // matching a hash of their concatenation without a 512-byte stack buffer.
    // The byte-contract regression below pins that equivalence.
    hashv(&[
        b"ameba-writer-settlement-group-v1",
        group_key.as_ref(),
        &group.underlying_id,
        &group.expiry_ts.to_le_bytes(),
        group.settlement_mint.as_ref(),
        group.anchor_market.as_ref(),
        group.anchor_oracle_month.as_ref(),
        &group.oracle_methodology_version.to_le_bytes(),
        &group.product_manifest_root,
        &group.coverage_manifest_hash,
        &group.recipe_hash,
        &group.settlement_source_digest,
        &group.active_weight_manifest_hash,
        &group.security_cap_atoms.to_le_bytes(),
        group.signer_registry.as_ref(),
        group.signer_set.as_ref(),
        &group.signer_set_version.to_le_bytes(),
        &group.signer_set_hash,
        &group.settlement_ts.to_le_bytes(),
    ])
    .to_bytes()
}

pub(super) struct CollectiveDlmmContext {
    pub sleeve_status: WriterSleeveStatus,
    pub group_status: WriterSettlementGroupStatus,
    pub active_weight_manifest_hash: [u8; 32],
    pub anchor_month_settled: bool,
    pub option_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub expiry_ts: u64,
    pub tick_size_quote_atomic: u64,
    pub maximum_price_quote_atomic: u64,
    pub maximum_bin_id: u16,
    pub swap_fee_bps: u16,
    pub maximum_bins_per_swap: u8,
}

struct CollectiveGroupBinding {
    sleeve_status: WriterSleeveStatus,
    group_status: WriterSettlementGroupStatus,
    active_weight_manifest_hash: [u8; 32],
    underlying_id: [u8; 32],
    expiry_ts: u64,
    settlement_mint: Pubkey,
    anchor_market: Pubkey,
    anchor_oracle_month: Pubkey,
    series_id: [u8; 32],
    contract_mint: Pubkey,
}

struct CollectiveMarketBinding {
    option_mint: Pubkey,
    quote_mint: Pubkey,
    expiry_ts: u64,
    tick_size_quote_atomic: u64,
    maximum_price_quote_atomic: u64,
    maximum_bin_id: u16,
    swap_fee_bps: u16,
    maximum_bins_per_swap: u8,
}

pub(super) fn load_collective_settlement_group_for_dlmm(
    program_id: &Pubkey,
    group_info: &AccountInfo,
) -> Result<Box<WriterSettlementGroupV1>, ProgramError> {
    load_writer_settlement_group(program_id, group_info)
}

#[inline(never)]
fn load_collective_group_binding(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    group_info: &AccountInfo,
    book_info: &AccountInfo,
    market_key: &Pubkey,
) -> Result<CollectiveGroupBinding, ProgramError> {
    let WriterBookContext {
        group,
        sleeve,
        book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let record = book.records[..usize::from(book.series_count)]
        .iter()
        .find(|record| record.market == *market_key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    if group.sleeve != *sleeve_info.key || sleeve.series_book != *book_info.key {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(CollectiveGroupBinding {
        sleeve_status: sleeve.status,
        group_status: group.status,
        active_weight_manifest_hash: group.active_weight_manifest_hash,
        underlying_id: group.underlying_id,
        expiry_ts: group.expiry_ts,
        settlement_mint: group.settlement_mint,
        anchor_market: group.anchor_market,
        anchor_oracle_month: group.anchor_oracle_month,
        series_id: record.series_id,
        contract_mint: record.contract_mint,
    })
}

#[inline(never)]
fn load_collective_market_binding(
    program_id: &Pubkey,
    market_info: &AccountInfo,
    binding: &CollectiveGroupBinding,
) -> Result<CollectiveMarketBinding, ProgramError> {
    let market = load_valid_market(program_id, market_info)?;
    validate_instrument_definition(&market.instrument)
        .map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    validate_market_parameters(&market.params, market.instrument.max_payout_per_contract)
        .map_err(|_| VaultError::InvalidAmoebaDlmmGrid)?;
    let option_mint = market
        .long_contract_mint
        .ok_or(VaultError::InvalidContractMint)?;
    let maximum_bin_id = market
        .instrument
        .max_payout_per_contract
        .checked_div(market.params.tick_size)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(VaultError::InvalidAmoebaDlmmGrid)?;
    let maximum_bins_per_swap = market
        .params
        .max_fills_per_instruction
        .min(MAX_AMOEBA_DLMM_BINS_PER_SWAP);
    if market.market_id != binding.series_id
        || option_mint != binding.contract_mint
        || market.instrument.underlying_id != binding.underlying_id
        || market.instrument.expiry_ts != binding.expiry_ts
        || market.collateral_mint != binding.settlement_mint
        || market.params.tick_size == 0
        || market.instrument.max_payout_per_contract == 0
        || !market
            .instrument
            .max_payout_per_contract
            .is_multiple_of(market.params.tick_size)
        || maximum_bin_id == 0
        || maximum_bin_id > MAX_AMOEBA_DLMM_BIN_COUNT
        || maximum_bins_per_swap == 0
        || market.params.taker_fee_bps > MAX_AMOEBA_DLMM_SWAP_FEE_BPS
        || !market.mint_accounting.has_canonical_layout()
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(CollectiveMarketBinding {
        option_mint,
        quote_mint: market.collateral_mint,
        expiry_ts: market.instrument.expiry_ts,
        tick_size_quote_atomic: market.params.tick_size,
        maximum_price_quote_atomic: market.instrument.max_payout_per_contract,
        maximum_bin_id,
        swap_fee_bps: market.params.taker_fee_bps,
        maximum_bins_per_swap,
    })
}

#[inline(never)]
fn load_collective_anchor_month_status(
    program_id: &Pubkey,
    anchor_month_info: &AccountInfo,
    binding: &CollectiveGroupBinding,
) -> Result<bool, ProgramError> {
    let anchor_month = load_oracle_month_state(anchor_month_info, program_id)?;
    let (expected_month, expected_bump) =
        derive_oracle_month_pda(program_id, &binding.anchor_market, binding.expiry_ts);
    if binding.anchor_oracle_month != *anchor_month_info.key
        || *anchor_month_info.key != expected_month
        || !anchor_month.is_initialized
        || anchor_month.bump != expected_bump
        || anchor_month.market != binding.anchor_market
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    Ok(anchor_month.settlement_record.is_some()
        || anchor_month.settlement_status == OracleSettlementStatus::Final)
}

/// Load the immutable collective binding needed by the secondary DLMM lanes. The pool remains
/// Market-addressed, while this companion context proves that its Market belongs to exactly one
/// sleeve and that its oracle clock is the sleeve's canonical anchor month.
#[inline(never)]
pub(super) fn load_collective_dlmm_context(
    program_id: &Pubkey,
    sleeve_info: &AccountInfo,
    group_info: &AccountInfo,
    book_info: &AccountInfo,
    market_info: &AccountInfo,
    anchor_month_info: &AccountInfo,
) -> Result<CollectiveDlmmContext, ProgramError> {
    let binding = load_collective_group_binding(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        market_info.key,
    )?;
    let market = load_collective_market_binding(program_id, market_info, &binding)?;
    let anchor_month_settled =
        load_collective_anchor_month_status(program_id, anchor_month_info, &binding)?;
    Ok(CollectiveDlmmContext {
        sleeve_status: binding.sleeve_status,
        group_status: binding.group_status,
        active_weight_manifest_hash: binding.active_weight_manifest_hash,
        anchor_month_settled,
        option_mint: market.option_mint,
        quote_mint: market.quote_mint,
        expiry_ts: market.expiry_ts,
        tick_size_quote_atomic: market.tick_size_quote_atomic,
        maximum_price_quote_atomic: market.maximum_price_quote_atomic,
        maximum_bin_id: market.maximum_bin_id,
        swap_fee_bps: market.swap_fee_bps,
        maximum_bins_per_swap: market.maximum_bins_per_swap,
    })
}

fn writer_payoff_digest(
    group: &Pubkey,
    market: &Market,
    market_key: &Pubkey,
    contract_mint: &Pubkey,
) -> [u8; 32] {
    let kind = match market.instrument.kind {
        crate::state::OptionKind::CallSpread => 0,
        crate::state::OptionKind::PutSpread => 1,
    };
    hashv(&[
        WRITER_PAYOFF_HASH_DOMAIN,
        group.as_ref(),
        &market.market_id,
        market_key.as_ref(),
        contract_mint.as_ref(),
        &market.instrument.underlying_id,
        &market.instrument.expiry_ts.to_le_bytes(),
        &market.instrument.strike_price.to_le_bytes(),
        &market.instrument.cap_price.to_le_bytes(),
        &market.instrument.contract_size.to_le_bytes(),
        &market.instrument.max_payout_per_contract.to_le_bytes(),
        &[kind],
    ])
    .to_bytes()
}

fn writer_risk_limit_hash(params: &SealWriterPolicyV1Params) -> [u8; 32] {
    hashv(&[
        WRITER_RISK_HASH_DOMAIN,
        &params.drawdown_scale.to_le_bytes(),
        &params.worst_drawdown_limit.to_le_bytes(),
        &params.upper_drawdown_limit.to_le_bytes(),
        &params.lower_drawdown_limit.to_le_bytes(),
        &params.lower_tail_max_settlement_atomic.to_le_bytes(),
        &params.upper_tail_min_settlement_atomic.to_le_bytes(),
        &params.operational_buffer_atoms.to_le_bytes(),
        &[writer_security_mode_byte(params.security_mode)],
        &params.primary_fee_bps.to_le_bytes(),
        &params.max_auction_issue_atoms.to_le_bytes(),
        &params.max_close_flat_atoms.to_le_bytes(),
        &[params.v2_feature_flags],
    ])
    .to_bytes()
}

fn writer_policy_hash(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    group: &Pubkey,
    series_family_hash: &[u8; 32],
    params: &SealWriterPolicyV1Params,
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(384);
    bytes.extend_from_slice(WRITER_POLICY_HASH_DOMAIN);
    bytes.extend_from_slice(program_id.as_ref());
    bytes.extend_from_slice(sleeve.as_ref());
    bytes.extend_from_slice(group.as_ref());
    bytes.extend_from_slice(&params.policy_version.to_le_bytes());
    bytes.extend_from_slice(&params.regime_input_version.to_le_bytes());
    bytes.extend_from_slice(&params.scenario_set_hash);
    bytes.extend_from_slice(&params.risk_limit_hash);
    bytes.extend_from_slice(series_family_hash);
    bytes.extend_from_slice(&params.beta_ppm.to_le_bytes());
    bytes.extend_from_slice(&params.lambda_ppm.to_le_bytes());
    bytes.extend_from_slice(&params.model_margin_vector_hash);
    bytes.extend_from_slice(&params.execution_cost_vector_hash);
    bytes.extend_from_slice(&params.drawdown_scale.to_le_bytes());
    bytes.extend_from_slice(&params.worst_drawdown_limit.to_le_bytes());
    bytes.extend_from_slice(&params.upper_drawdown_limit.to_le_bytes());
    bytes.extend_from_slice(&params.lower_drawdown_limit.to_le_bytes());
    bytes.extend_from_slice(&params.lower_tail_max_settlement_atomic.to_le_bytes());
    bytes.extend_from_slice(&params.upper_tail_min_settlement_atomic.to_le_bytes());
    bytes.extend_from_slice(&params.operational_buffer_atoms.to_le_bytes());
    bytes.extend_from_slice(&params.primary_fee_bps.to_le_bytes());
    bytes.push(writer_auction_rule_byte(params.auction_priority_rule));
    bytes.push(writer_security_mode_byte(params.security_mode));
    bytes.push(params.v2_feature_flags);
    hashv(&[bytes.as_slice()]).to_bytes()
}

fn writer_coverage_manifest_hash(coverage: &OracleSkuCoverageManifest) -> [u8; 32] {
    hashv(&[
        WRITER_COVERAGE_HASH_DOMAIN,
        coverage.month.as_ref(),
        &coverage.required_sku_root,
        &coverage.required_sku_count.to_le_bytes(),
        &coverage.covered_sku_count.to_le_bytes(),
        &coverage.planned_scramble_start_ts.to_le_bytes(),
        &coverage.planned_listing_ts.to_le_bytes(),
        &[u8::from(coverage.coverage_finalized)],
        &coverage.coverage_complete_ts.to_le_bytes(),
    ])
    .to_bytes()
}

#[inline]
fn writer_math_error(_error: WriterMathError) -> ProgramError {
    VaultError::WriterArithmeticAdmissionFailed.into()
}

pub(super) fn writer_book_math_series(
    book: &WriterSeriesBookV1,
) -> Result<Vec<WriterSeries>, ProgramError> {
    if book.series_count == 0
        || usize::from(book.series_count) > crate::constants::WRITER_MAX_LIVE_SERIES
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut result = Vec::with_capacity(usize::from(book.series_count));
    for record in book.records.iter().take(usize::from(book.series_count)) {
        if !record.active {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
        result.push(WriterSeries {
            kind: record.option_kind,
            strike_price_atomic: record.strike_price_atomic,
            cap_price_atomic: record.cap_or_floor_price_atomic,
            contract_size_atoms: record.contract_size_atoms,
            max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
            external_oi_atoms: record.external_open_interest_atoms,
        });
    }
    Ok(result)
}

pub(super) fn recompute_writer_metrics(
    sleeve: &mut WriterSleeveV1,
    book: &WriterSeriesBookV1,
    snapshot: &WriterPolicySnapshotV1,
    security_cap_atoms: Option<u64>,
    enforce_solvency_and_drawdown: bool,
) -> ProgramResult {
    if snapshot.policy_hash != sleeve.policy_hash
        || snapshot.policy_version != sleeve.policy_version
        || snapshot.series_family_hash != writer_series_family_hash(book)
        || snapshot.security_mode != sleeve.security_mode
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let count = usize::from(book.series_count);
    if count == 0 || count > crate::constants::WRITER_MAX_LIVE_SERIES {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut series = [WriterSeries::EMPTY; crate::constants::WRITER_SERIES_STORAGE_CAPACITY];
    for (index, record) in book.records.iter().take(count).enumerate() {
        if !record.active {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
        series[index] = WriterSeries {
            kind: record.option_kind,
            strike_price_atomic: record.strike_price_atomic,
            cap_price_atomic: record.cap_or_floor_price_atomic,
            contract_size_atoms: record.contract_size_atoms,
            max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
            external_oi_atoms: record.external_open_interest_atoms,
        };
    }
    let (reserve, exposure) = calculate_writer_metrics(
        &series[..count],
        snapshot,
        sleeve.security_mode,
        sleeve.accounted_asset_atoms,
        sleeve.locked_primary_premium_atoms,
        sleeve.writer_principal_atoms,
        security_cap_atoms,
        enforce_solvency_and_drawdown,
    )?;
    sleeve.exact_reserve_atoms = reserve.reserve_atoms;
    sleeve.lower_tail_reserve_atoms = reserve.lower_tail_reserve_atoms;
    sleeve.upper_tail_reserve_atoms = reserve.upper_tail_reserve_atoms;
    sleeve.security_exposure_atoms = exposure;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn calculate_writer_metrics(
    series: &[WriterSeries],
    snapshot: &WriterPolicySnapshotV1,
    security_mode: WriterSecurityMode,
    accounted_asset_atoms: u64,
    locked_primary_premium_atoms: u64,
    writer_principal_atoms: u64,
    security_cap_atoms: Option<u64>,
    enforce_solvency_and_drawdown: bool,
) -> Result<(crate::writer_sleeve_math::WriterReserveSummary, u64), ProgramError> {
    let reserve = exact_reserve(
        series,
        snapshot.lower_tail_max_settlement_atomic,
        snapshot.upper_tail_min_settlement_atomic,
    )
    .map_err(writer_math_error)?;
    let math_security_mode = match security_mode {
        WriterSecurityMode::GrossExternalMaxPayout => {
            WriterMathSecurityMode::GrossExternalMaximumPayout
        }
        WriterSecurityMode::ExactExternalEnvelope => WriterMathSecurityMode::ExactExternalEnvelope,
    };
    let exposure = calculate_security_exposure(math_security_mode, series, reserve.reserve_atoms)
        .map_err(writer_math_error)?;
    if security_cap_atoms.is_some_and(|cap| exposure > cap) {
        return Err(VaultError::WriterSecurityCapExceeded.into());
    }
    if enforce_solvency_and_drawdown {
        let required_assets = reserve
            .reserve_atoms
            .checked_add(snapshot.operational_buffer_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if accounted_asset_atoms < required_assets || writer_principal_atoms == 0 {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        let checks = drawdown_checks(
            &reserve,
            locked_primary_premium_atoms,
            writer_principal_atoms,
            snapshot.worst_drawdown_limit,
            snapshot.lower_drawdown_limit,
            snapshot.upper_drawdown_limit,
        )
        .map_err(writer_math_error)?;
        if !checks.all_pass() {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
    }
    Ok((reserve, exposure))
}

fn writer_activation_assets_are_sufficient(sleeve: &WriterSleeveV1) -> Result<bool, ProgramError> {
    let principal_and_premium = sleeve
        .writer_principal_atoms
        .checked_add(sleeve.locked_primary_premium_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    // Before any direct Flat burn A == W + B. A holder-authorized burn decrements W and S but
    // leaves the forfeited settlement assets locked as derived writer surplus. That surplus must
    // not make an otherwise valid Funding sleeve impossible to activate.
    Ok(sleeve.accounted_asset_atoms >= principal_and_premium)
}

#[allow(clippy::too_many_arguments)]
fn create_classic_token_pda<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    token_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    authority: &Pubkey,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    validate_create_only_program_account_target(program_id, token_info)?;
    create_program_account(
        payer_info,
        token_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        signer_seeds,
    )?;
    invoke_token_initialize_account3(token_program_info, token_info, mint_info, authority)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_or_create_writer_retirement_custody<'a>(
    program_id: &Pubkey,
    payer_info: &AccountInfo<'a>,
    sleeve_info: &AccountInfo<'a>,
    market_info: &AccountInfo<'a>,
    custody_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
) -> Result<TokenAccount, ProgramError> {
    let (expected, bump) =
        derive_writer_retirement_custody_pda(program_id, sleeve_info.key, market_info.key);
    if *custody_info.key != expected {
        return Err(VaultError::InvalidPda.into());
    }
    if custody_info.owner == token_program_info.key {
        validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
        return validate_token_account(custody_info);
    }
    validate_create_only_program_account_target(program_id, custody_info)?;
    create_program_account(
        payer_info,
        custody_info,
        system_program_info,
        token_program_info.key,
        TokenAccount::LEN,
        &[
            crate::constants::WRITER_RETIREMENT_CUSTODY_PDA_SEED,
            sleeve_info.key.as_ref(),
            market_info.key.as_ref(),
            &[bump],
        ],
    )?;
    invoke_token_initialize_account3(token_program_info, custody_info, mint_info, sleeve_info.key)?;
    validate_vault_token_account(custody_info, mint_info.key, sleeve_info.key)?;
    validate_token_account(custody_info)
}

pub(super) fn process_initialize_policy_registry(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: InitializeWriterPolicyRegistryV1Params,
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let fee_vault_info = &accounts[3];
    let settlement_mint_info = &accounts[4];
    let token_program_info = &accounts[5];
    let system_program_info = &accounts[6];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *token_program_info.key != spl_token_program_id() {
        return Err(VaultError::InvalidTokenProgram.into());
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    if crate::pubkey_is_default(&params.policy_authority)
        || params.rotation_delay_slots < crate::constants::MIN_WRITER_POLICY_ROTATION_DELAY_SLOTS
        || config.usdc_mint != *settlement_mint_info.key
    {
        return Err(VaultError::InvalidWriterPolicyRegistry.into());
    }
    validate_collateral_mint_account(settlement_mint_info, token_program_info.key)?;

    let (expected_registry, registry_bump) = derive_writer_policy_registry_pda(program_id);
    let (expected_fee_vault, fee_vault_bump) =
        derive_writer_protocol_fee_vault_pda(program_id, registry_info.key);
    if *registry_info.key != expected_registry || *fee_vault_info.key != expected_fee_vault {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, registry_info)?;
    create_program_account(
        admin_info,
        registry_info,
        system_program_info,
        program_id,
        WriterPolicyRegistryV1::LEN,
        &[
            crate::constants::WRITER_POLICY_REGISTRY_PDA_SEED,
            &[registry_bump],
        ],
    )?;
    create_classic_token_pda(
        program_id,
        admin_info,
        fee_vault_info,
        settlement_mint_info,
        registry_info.key,
        token_program_info,
        system_program_info,
        &[
            crate::constants::WRITER_PROTOCOL_FEE_VAULT_PDA_SEED,
            registry_info.key.as_ref(),
            &[fee_vault_bump],
        ],
    )?;
    validate_vault_token_account(fee_vault_info, settlement_mint_info.key, registry_info.key)?;
    if validate_token_account(fee_vault_info)?.amount != 0 {
        return Err(VaultError::InvalidWriterPolicyRegistry.into());
    }
    let slot = Clock::get()?.slot;
    let registry = WriterPolicyRegistryV1 {
        is_initialized: true,
        bump: registry_bump,
        account_discriminator: WriterPolicyRegistryV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterPolicyRegistryV1::ACCOUNT_VERSION,
        vault_config: *config_info.key,
        policy_authority: params.policy_authority,
        pending_policy_authority: Pubkey::default(),
        protocol_fee_vault: *fee_vault_info.key,
        pending_activation_slot: 0,
        rotation_delay_slots: params.rotation_delay_slots,
        latest_policy_version: 0,
        last_updated_slot: slot,
        reserved: [0; 32],
    };
    store_state(registry_info, &registry)
}

pub(super) fn process_manage_policy_authority(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ManageWriterPolicyAuthorityV1Params,
) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let actor_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    if !actor_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    let mut registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let slot = Clock::get()?.slot;
    match params.action {
        ManageWriterPolicyAuthorityActionV1::Propose => {
            if *actor_info.key != config.admin
                || crate::pubkey_is_default(&params.new_authority)
                || params.new_authority == registry.policy_authority
                || !crate::pubkey_is_default(&registry.pending_policy_authority)
            {
                return Err(VaultError::Unauthorized.into());
            }
            registry.pending_policy_authority = params.new_authority;
            registry.pending_activation_slot = slot
                .checked_add(registry.rotation_delay_slots)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        ManageWriterPolicyAuthorityActionV1::Activate => {
            if params.new_authority != registry.pending_policy_authority
                || *actor_info.key != registry.pending_policy_authority
                || slot < registry.pending_activation_slot
            {
                return Err(VaultError::InvalidWriterLifecycle.into());
            }
            registry.policy_authority = registry.pending_policy_authority;
            registry.pending_policy_authority = Pubkey::default();
            registry.pending_activation_slot = 0;
        }
        ManageWriterPolicyAuthorityActionV1::Cancel => {
            if *actor_info.key != config.admin
                || crate::pubkey_is_default(&registry.pending_policy_authority)
            {
                return Err(VaultError::Unauthorized.into());
            }
            registry.pending_policy_authority = Pubkey::default();
            registry.pending_activation_slot = 0;
        }
    }
    registry.last_updated_slot = slot;
    store_state(registry_info, &registry)
}

pub(super) fn process_initialize_settlement_group(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let group_info = &accounts[2];
    let market_info = &accounts[3];
    let month_info = &accounts[4];
    let settlement_mint_info = &accounts[5];
    let signer_registry_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key || config.usdc_mint != *settlement_mint_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    validate_collateral_mint_account(settlement_mint_info, &spl_token_program_id())?;
    let (market, month) = load_valid_market_and_oracle_month(program_id, market_info, month_info)?;
    let signer_registry =
        load_canonical_settlement_signer_registry(program_id, signer_registry_info)?;
    if !market.paused
        || market.total_position_collateral_locked != 0
        || market.mint_accounting.total_issued != 0
        || market.mint_accounting.total_consumed != 0
        || market.mint_accounting.total_burned != 0
        || market.collateral_mint != *settlement_mint_info.key
        || month.settlement_record.is_some()
        || matches!(month.phase, OraclePhase::Settled | OraclePhase::Closed)
    {
        return Err(VaultError::InvalidWriterSettlementGroup.into());
    }
    let (expected_group, bump) = derive_writer_settlement_group_pda(
        program_id,
        &market.instrument.underlying_id,
        market.instrument.expiry_ts,
        settlement_mint_info.key,
    );
    if *group_info.key != expected_group {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, group_info)?;
    create_program_account(
        admin_info,
        group_info,
        system_program_info,
        program_id,
        WriterSettlementGroupV1::LEN,
        &[
            crate::constants::WRITER_SETTLEMENT_GROUP_PDA_SEED,
            &market.instrument.underlying_id,
            &market.instrument.expiry_ts.to_le_bytes(),
            settlement_mint_info.key.as_ref(),
            &[bump],
        ],
    )?;
    let sleeve = derive_writer_sleeve_pda(program_id, group_info.key).0;
    let group = WriterSettlementGroupV1 {
        is_initialized: true,
        bump,
        account_discriminator: WriterSettlementGroupV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterSettlementGroupV1::ACCOUNT_VERSION,
        underlying_id: market.instrument.underlying_id,
        expiry_ts: market.instrument.expiry_ts,
        settlement_mint: *settlement_mint_info.key,
        anchor_market: *market_info.key,
        anchor_oracle_month: *month_info.key,
        oracle_methodology_version: crate::constants::WRITER_ORACLE_METHODOLOGY_VERSION,
        product_manifest_root: [0; 32],
        coverage_manifest_hash: [0; 32],
        recipe_hash: [0; 32],
        settlement_source_digest: [0; 32],
        active_weight_manifest_hash: [0; 32],
        security_cap_atoms: 0,
        signer_registry: *signer_registry_info.key,
        signer_set: Pubkey::default(),
        signer_set_version: 0,
        signer_set_hash: [0; 32],
        settlement_ts: market.instrument.expiry_ts,
        settlement_price_atomic: 0,
        final_settlement_commitment: [0; 32],
        submitted_by: Pubkey::default(),
        finalized_slot: 0,
        sleeve,
        status: WriterSettlementGroupStatus::Anchored,
        series_count: 0,
        reserved: [0; 6],
        last_updated_slot: Clock::get()?.slot,
    };
    let _ = signer_registry;
    store_state(group_info, &group)
}

pub(super) fn process_initialize_sleeve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 16 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let group_info = &accounts[3];
    let sleeve_info = &accounts[4];
    let book_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    let settlement_mint_info = &accounts[8];
    let flat_interface_info = &accounts[9];
    let light_program_info = &accounts[10];
    let cpi_authority_info = &accounts[11];
    let token_program_info = &accounts[12];
    let system_program_info = &accounts[13];
    let compressible_config_info = &accounts[14];
    let rent_sponsor_info = &accounts[15];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    validate_writer_compression_accounts(
        light_program_info,
        cpi_authority_info,
        token_program_info,
        system_program_info,
        compressible_config_info,
        rent_sponsor_info,
    )?;
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key
        || config.usdc_mint != *settlement_mint_info.key
        || *settlement_mint_info.key == *flat_mint_info.key
    {
        return Err(VaultError::Unauthorized.into());
    }
    let registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    let mut group = load_writer_settlement_group(program_id, group_info)?;
    if group.status != WriterSettlementGroupStatus::Anchored
        || group.settlement_mint != *settlement_mint_info.key
        || group.sleeve != *sleeve_info.key
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let (expected_sleeve, sleeve_bump) = derive_writer_sleeve_pda(program_id, group_info.key);
    let (expected_book, book_bump) = derive_writer_series_book_pda(program_id, sleeve_info.key);
    let (expected_vault, vault_bump) =
        derive_writer_sleeve_usdc_vault_pda(program_id, sleeve_info.key);
    let (expected_flat_mint, flat_mint_bump) =
        derive_writer_flat_mint_pda(program_id, sleeve_info.key);
    let expected_interface =
        light_token_instruction::get_spl_interface_pda_and_bump(flat_mint_info.key).0;
    if *sleeve_info.key != expected_sleeve
        || *book_info.key != expected_book
        || *sleeve_vault_info.key != expected_vault
        || *flat_mint_info.key != expected_flat_mint
        || *flat_interface_info.key != expected_interface
    {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, sleeve_info)?;
    validate_create_only_program_account_target(program_id, book_info)?;
    validate_create_only_program_account_target(program_id, flat_mint_info)?;
    if flat_interface_info.owner != &system_program::id()
        || flat_interface_info.executable
        || flat_interface_info.data_len() != 0
    {
        return Err(VaultError::InvalidSplInterfaceAccount.into());
    }

    create_program_account(
        admin_info,
        sleeve_info,
        system_program_info,
        program_id,
        WriterSleeveV1::LEN,
        &[
            crate::constants::WRITER_SLEEVE_PDA_SEED,
            group_info.key.as_ref(),
            &[sleeve_bump],
        ],
    )?;
    create_program_account(
        admin_info,
        book_info,
        system_program_info,
        program_id,
        WriterSeriesBookV1::LEN,
        &[
            crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
            sleeve_info.key.as_ref(),
            &[book_bump],
        ],
    )?;
    create_classic_token_pda(
        program_id,
        admin_info,
        sleeve_vault_info,
        settlement_mint_info,
        sleeve_info.key,
        token_program_info,
        system_program_info,
        &[
            crate::constants::WRITER_SLEEVE_USDC_VAULT_PDA_SEED,
            sleeve_info.key.as_ref(),
            &[vault_bump],
        ],
    )?;
    create_program_account(
        admin_info,
        flat_mint_info,
        system_program_info,
        token_program_info.key,
        Mint::LEN,
        &[
            crate::constants::WRITER_FLAT_MINT_PDA_SEED,
            sleeve_info.key.as_ref(),
            &[flat_mint_bump],
        ],
    )?;
    invoke_token_initialize_mint2(
        token_program_info,
        flat_mint_info,
        sleeve_info.key,
        None,
        MarketMintAccounting::CANONICAL_DECIMALS,
    )?;
    invoke_create_spl_interface_pda(
        admin_info,
        flat_interface_info,
        system_program_info,
        flat_mint_info,
        token_program_info,
        cpi_authority_info,
        light_program_info,
    )?;
    validate_vault_token_account(sleeve_vault_info, settlement_mint_info.key, sleeve_info.key)?;
    let flat_mint = validate_mint_account(flat_mint_info, token_program_info.key)?;
    let flat_interface = validate_token_account(flat_interface_info)?;
    if flat_mint.supply != 0
        || flat_mint.decimals != MarketMintAccounting::CANONICAL_DECIMALS
        || flat_mint.mint_authority != COption::Some(*sleeve_info.key)
        || flat_mint.freeze_authority != COption::None
        || flat_interface.mint != *flat_mint_info.key
        || flat_interface.owner != cpi_authority()
        || flat_interface.amount != 0
        || flat_interface.state != AccountState::Initialized
    {
        return Err(VaultError::InvalidMint.into());
    }
    let slot = Clock::get()?.slot;
    let mut book = WriterSeriesBookV1 {
        is_initialized: true,
        bump: book_bump,
        account_discriminator: WriterSeriesBookV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterSeriesBookV1::ACCOUNT_VERSION,
        sleeve: *sleeve_info.key,
        settlement_group: *group_info.key,
        series_count: 0,
        max_series: crate::constants::WRITER_MAX_LIVE_SERIES as u8,
        frozen: false,
        reserved: [0; 7],
        book_digest: [0; 32],
        last_updated_slot: slot,
        records: vec![
            WriterSeriesRecordV1::EMPTY;
            crate::constants::WRITER_SERIES_STORAGE_CAPACITY
        ]
        .into_boxed_slice()
        .try_into()
        .map_err(|_| VaultError::ArithmeticOverflow)?,
    };
    book.book_digest = writer_book_digest(&book);
    let sleeve = WriterSleeveV1 {
        is_initialized: true,
        bump: sleeve_bump,
        account_discriminator: WriterSleeveV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterSleeveV1::ACCOUNT_VERSION,
        vault_config: *config_info.key,
        underlying_id: group.underlying_id,
        expiry_ts: group.expiry_ts,
        settlement_mint: *settlement_mint_info.key,
        settlement_group: *group_info.key,
        series_book: *book_info.key,
        usdc_vault: *sleeve_vault_info.key,
        flat_mint: *flat_mint_info.key,
        flat_spl_interface: *flat_interface_info.key,
        flat_staging: derive_writer_flat_staging_pda(program_id, sleeve_info.key).0,
        flat_burn_custody: derive_writer_flat_burn_custody_pda(program_id, sleeve_info.key).0,
        policy_registry: *registry_info.key,
        policy_snapshot: Pubkey::default(),
        policy_version: 0,
        policy_hash: [0; 32],
        scenario_set_hash: [0; 32],
        risk_limit_hash: [0; 32],
        writer_principal_atoms: 0,
        locked_primary_premium_atoms: 0,
        accounted_asset_atoms: 0,
        exact_reserve_atoms: 0,
        upper_tail_reserve_atoms: 0,
        lower_tail_reserve_atoms: 0,
        flat_par_supply_atoms: 0,
        security_exposure_atoms: 0,
        long_liability_initial_atoms: 0,
        long_liability_remaining_atoms: 0,
        flat_residual_initial_atoms: 0,
        flat_residual_remaining_atoms: 0,
        flat_supply_snapshot_atoms: 0,
        flat_claim_supply_remaining_atoms: 0,
        stranded_surplus_atoms: 0,
        operational_buffer_atoms: 0,
        auction_nonce: 0,
        close_nonce: 0,
        series_count: 0,
        status: WriterSleeveStatus::Draft,
        security_mode: WriterSecurityMode::GrossExternalMaxPayout,
        v2_feature_flags: 0,
        active_auction: None,
        active_close_request: None,
        settlement_finalized_slot: 0,
        last_updated_slot: slot,
        reserved: [0; 32],
    };
    group.last_updated_slot = slot;
    let _ = registry;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(group_info, &group)
}

pub(super) fn process_register_series(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 7 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let market_info = &accounts[5];
    let contract_mint_info = &accounts[6];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterBookContext {
        mut group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    if group.sleeve != *sleeve_info.key
        || sleeve.series_book != *book_info.key
        || sleeve.status != WriterSleeveStatus::Draft
        || group.status != WriterSettlementGroupStatus::Anchored
        || book.frozen
        || usize::from(book.series_count) >= crate::constants::WRITER_MAX_LIVE_SERIES
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    let mint = validate_canonical_market_mint(market_info, &mut market, contract_mint_info, 0)?;
    let expected_max_payout = match market.instrument.kind {
        crate::state::OptionKind::CallSpread => market
            .instrument
            .cap_price
            .checked_sub(market.instrument.strike_price),
        crate::state::OptionKind::PutSpread => market
            .instrument
            .strike_price
            .checked_sub(market.instrument.cap_price),
    }
    .ok_or(VaultError::InvalidMarketConfig)?;
    if !market.paused
        || market.market_id == [0; 32]
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != group.expiry_ts
        || market.collateral_mint != group.settlement_mint
        || market.instrument.contract_size != MarketMintAccounting::CANONICAL_ATOMIC_SCALE
        || market.instrument.max_payout_per_contract != expected_max_payout
        || market.total_position_collateral_locked != 0
        || market.mint_accounting.total_issued != 0
        || market.mint_accounting.total_consumed != 0
        || market.mint_accounting.total_burned != 0
        || mint.supply != 0
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let index = usize::from(book.series_count);
    if index != 0 && book.records[index - 1].series_id >= market.market_id {
        return Err(VaultError::InvalidWriterSeriesOrder.into());
    }
    let retirement =
        derive_writer_retirement_custody_pda(program_id, sleeve_info.key, market_info.key).0;
    let record = WriterSeriesRecordV1 {
        active: true,
        option_kind: market.instrument.kind,
        custody_status: WriterSeriesCustodyStatus::Absent,
        settlement_status: WriterSeriesSettlementStatus::Open,
        reserved: [0; 4],
        series_id: market.market_id,
        market: *market_info.key,
        contract_mint: *contract_mint_info.key,
        retirement_custody: retirement,
        strike_price_atomic: market.instrument.strike_price,
        cap_or_floor_price_atomic: market.instrument.cap_price,
        contract_size_atoms: market.instrument.contract_size,
        max_payout_per_contract_atoms: market.instrument.max_payout_per_contract,
        total_physical_supply_atoms: 0,
        issuer_controlled_atoms: 0,
        external_open_interest_atoms: 0,
        primary_premium_collected_atoms: 0,
        settlement_external_oi_snapshot_atoms: 0,
        settlement_liability_initial_atoms: 0,
        settlement_liability_remaining_atoms: 0,
        payoff_digest: writer_payoff_digest(
            group_info.key,
            &market,
            market_info.key,
            contract_mint_info.key,
        ),
    };
    let candidate_series = WriterSeries {
        kind: record.option_kind,
        strike_price_atomic: record.strike_price_atomic,
        cap_price_atomic: record.cap_or_floor_price_atomic,
        contract_size_atoms: record.contract_size_atoms,
        max_payout_per_contract_atoms: record.max_payout_per_contract_atoms,
        external_oi_atoms: 0,
    };
    for existing in &book.records[..index] {
        let existing_series = WriterSeries {
            kind: existing.option_kind,
            strike_price_atomic: existing.strike_price_atomic,
            cap_price_atomic: existing.cap_or_floor_price_atomic,
            contract_size_atoms: existing.contract_size_atoms,
            max_payout_per_contract_atoms: existing.max_payout_per_contract_atoms,
            external_oi_atoms: 0,
        };
        if candidate_series.same_instrument(&existing_series) {
            return Err(VaultError::InvalidWriterSeriesBook.into());
        }
    }
    book.records[index] = record;
    book.series_count = book
        .series_count
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    sleeve.series_count = book.series_count;
    group.series_count = book.series_count;
    let slot = Clock::get()?.slot;
    book.last_updated_slot = slot;
    book.book_digest = writer_book_digest(&book);
    sleeve.last_updated_slot = slot;
    group.last_updated_slot = slot;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(group_info, &group)
}

pub(super) fn process_seal_policy(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SealWriterPolicyV1Params,
) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let registry_info = &accounts[2];
    let sleeve_info = &accounts[3];
    let group_info = &accounts[4];
    let book_info = &accounts[5];
    let snapshot_info = &accounts[6];
    let system_program_info = &accounts[7];
    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program_info.key != system_program::id() {
        return Err(VaultError::InvalidSystemProgram.into());
    }
    let _config = load_canonical_vault_config(program_id, config_info)?;
    let mut registry = load_writer_policy_registry(program_id, registry_info, config_info.key)?;
    if registry.policy_authority != *authority_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterBookContext {
        mut group,
        mut sleeve,
        mut book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let expected_policy_version = registry
        .latest_policy_version
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if group.sleeve != *sleeve_info.key
        || sleeve.status != WriterSleeveStatus::Draft
        || group.status != WriterSettlementGroupStatus::Anchored
        || book.frozen
        || book.series_count == 0
        || params.policy_version != expected_policy_version
        || params.regime_input_version == 0
        || params.beta_ppm == 0
        || params.beta_ppm >= crate::constants::WRITER_RATIO_SCALE_PPM as u32
        || params.drawdown_scale != crate::constants::WRITER_RATIO_SCALE_PPM
        || params.worst_drawdown_limit > params.drawdown_scale
        || params.upper_drawdown_limit > params.drawdown_scale
        || params.lower_drawdown_limit > params.drawdown_scale
        || params.lower_tail_max_settlement_atomic >= params.upper_tail_min_settlement_atomic
        || params.max_auction_issue_atoms == 0
        || params.max_close_flat_atoms == 0
        || params.primary_fee_bps > 10_000
        || params.security_mode != WriterSecurityMode::GrossExternalMaxPayout
        || params.reserve_rounding_mode != WriterReserveRoundingMode::AggregateBookCeiling
        || params.auction_priority_rule != WriterAuctionPriorityRule::PayAsBidPriceThenSeriesProRata
        || params.v2_feature_flags != 0
        || crate::bytes32_is_zero(&params.scenario_set_hash)
        || crate::bytes32_is_zero(&params.model_margin_vector_hash)
        || crate::bytes32_is_zero(&params.execution_cost_vector_hash)
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let series_family_hash = writer_series_family_hash(&book);
    if params.risk_limit_hash != writer_risk_limit_hash(&params)
        || params.policy_hash
            != writer_policy_hash(
                program_id,
                sleeve_info.key,
                group_info.key,
                &series_family_hash,
                &params,
            )
    {
        return Err(VaultError::InvalidWriterPolicySnapshot.into());
    }
    let (expected_snapshot, bump) =
        derive_writer_policy_snapshot_pda(program_id, sleeve_info.key, params.policy_version);
    if *snapshot_info.key != expected_snapshot {
        return Err(VaultError::InvalidPda.into());
    }
    validate_create_only_program_account_target(program_id, snapshot_info)?;
    create_program_account(
        authority_info,
        snapshot_info,
        system_program_info,
        program_id,
        WriterPolicySnapshotV1::LEN,
        &[
            crate::constants::WRITER_POLICY_SNAPSHOT_PDA_SEED,
            sleeve_info.key.as_ref(),
            &params.policy_version.to_le_bytes(),
            &[bump],
        ],
    )?;
    let slot = Clock::get()?.slot;
    let snapshot = WriterPolicySnapshotV1 {
        is_initialized: true,
        bump,
        account_discriminator: WriterPolicySnapshotV1::ACCOUNT_DISCRIMINATOR,
        account_version: WriterPolicySnapshotV1::ACCOUNT_VERSION,
        sleeve: *sleeve_info.key,
        registry: *registry_info.key,
        policy_version: params.policy_version,
        regime_input_version: params.regime_input_version,
        policy_hash: params.policy_hash,
        scenario_set_hash: params.scenario_set_hash,
        risk_limit_hash: params.risk_limit_hash,
        series_family_hash,
        security_mode: params.security_mode,
        reserve_rounding_mode: params.reserve_rounding_mode,
        auction_priority_rule: params.auction_priority_rule,
        v2_feature_flags: params.v2_feature_flags,
        max_series: crate::constants::WRITER_MAX_LIVE_SERIES as u8,
        reserved_0: 0,
        primary_fee_bps: params.primary_fee_bps,
        reserved_1: [0; 2],
        drawdown_scale: params.drawdown_scale,
        worst_drawdown_limit: params.worst_drawdown_limit,
        upper_drawdown_limit: params.upper_drawdown_limit,
        lower_drawdown_limit: params.lower_drawdown_limit,
        lower_tail_max_settlement_atomic: params.lower_tail_max_settlement_atomic,
        upper_tail_min_settlement_atomic: params.upper_tail_min_settlement_atomic,
        operational_buffer_atoms: params.operational_buffer_atoms,
        max_auction_issue_atoms: params.max_auction_issue_atoms,
        max_close_flat_atoms: params.max_close_flat_atoms,
        created_slot: slot,
        sealed_slot: slot,
        reserved: [0; 32],
    };
    sleeve.policy_snapshot = *snapshot_info.key;
    sleeve.policy_version = params.policy_version;
    sleeve.policy_hash = params.policy_hash;
    sleeve.scenario_set_hash = params.scenario_set_hash;
    sleeve.risk_limit_hash = params.risk_limit_hash;
    sleeve.operational_buffer_atoms = params.operational_buffer_atoms;
    sleeve.security_mode = params.security_mode;
    sleeve.v2_feature_flags = params.v2_feature_flags;
    sleeve.status = WriterSleeveStatus::PolicyFrozen;
    sleeve.last_updated_slot = slot;
    book.frozen = true;
    book.book_digest = writer_book_digest(&book);
    book.last_updated_slot = slot;
    group.last_updated_slot = slot;
    registry.latest_policy_version = params.policy_version;
    registry.last_updated_slot = slot;
    store_state(snapshot_info, &snapshot)?;
    store_state(book_info, &book)?;
    store_state(sleeve_info, &sleeve)?;
    store_state(group_info, &group)?;
    store_state(registry_info, &registry)
}

pub(super) fn process_open_funding(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    if accounts.len() != 8 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let sleeve_vault_info = &accounts[6];
    let flat_mint_info = &accounts[7];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterPolicyContext {
        group,
        mut sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    if group.sleeve != *sleeve_info.key
        || sleeve.usdc_vault != *sleeve_vault_info.key
        || sleeve.flat_mint != *flat_mint_info.key
        || sleeve.policy_snapshot != *snapshot_info.key
        || sleeve.status != WriterSleeveStatus::PolicyFrozen
        || group.status != WriterSettlementGroupStatus::Anchored
        || !book.frozen
        || book.series_count == 0
        || validate_token_account(sleeve_vault_info)?.amount != sleeve.accounted_asset_atoms
        || snapshot.policy_hash != sleeve.policy_hash
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    sleeve.status = WriterSleeveStatus::Funding;
    sleeve.last_updated_slot = Clock::get()?.slot;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_activate_sleeve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    if accounts.len() != 16 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let snapshot_info = &accounts[5];
    let anchor_market_info = &accounts[6];
    let month_info = &accounts[7];
    let coverage_info = &accounts[8];
    let active_weight_info = &accounts[9];
    let recipe_info = &accounts[10];
    let settlement_source_info = &accounts[11];
    let signer_registry_info = &accounts[12];
    let signer_set_info = &accounts[13];
    let sleeve_vault_info = &accounts[14];
    let flat_mint_info = &accounts[15];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key || config.paused {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterPolicyContext {
        mut group,
        mut sleeve,
        book,
        snapshot,
    } = load_writer_policy_context(
        program_id,
        sleeve_info,
        group_info,
        book_info,
        snapshot_info,
        None,
    )?;
    let (anchor_market, month) =
        load_valid_market_and_oracle_month(program_id, anchor_market_info, month_info)?;
    ensure_oracle_game_window(&anchor_market, &month)?;
    let coverage =
        load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
    let active =
        load_valid_oracle_active_weight_manifest(program_id, month_info.key, active_weight_info)?;
    ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
    ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
    let recipe = load_valid_oracle_recipe_weight_manifest(program_id, month_info.key, recipe_info)?;
    let settlement_sources = load_valid_oracle_settlement_source_manifest(
        program_id,
        month_info.key,
        settlement_source_info,
    )?;
    let signer_registry =
        load_canonical_settlement_signer_registry(program_id, signer_registry_info)?;
    let signer_set = load_canonical_settlement_signer_set(
        program_id,
        signer_registry_info.key,
        signer_set_info,
    )?;
    validate_vault_token_account(sleeve_vault_info, &sleeve.settlement_mint, sleeve_info.key)?;
    validate_writer_flat_mint(sleeve_info, &sleeve, flat_mint_info)?;
    let physical_assets = validate_token_account(sleeve_vault_info)?.amount;
    let anchor_registered = book
        .records
        .iter()
        .take(usize::from(book.series_count))
        .any(|record| record.market == *anchor_market_info.key);
    recompute_writer_metrics(
        &mut sleeve,
        &book,
        &snapshot,
        Some(active.max_open_interest_payout),
        true,
    )?;
    let required_assets = sleeve
        .exact_reserve_atoms
        .checked_add(snapshot.operational_buffer_atoms)
        .ok_or(VaultError::ArithmeticOverflow)?;
    let activation_assets_sufficient = writer_activation_assets_are_sufficient(&sleeve)?;
    if group.sleeve != *sleeve_info.key
        || group.anchor_market != *anchor_market_info.key
        || group.anchor_oracle_month != *month_info.key
        || group.signer_registry != *signer_registry_info.key
        || signer_registry.current_set != *signer_set_info.key
        || signer_registry.current_version != signer_set.version
        || sleeve.status != WriterSleeveStatus::Funding
        || group.status != WriterSettlementGroupStatus::Anchored
        || !book.frozen
        || sleeve.policy_snapshot != *snapshot_info.key
        || snapshot.policy_hash != sleeve.policy_hash
        || snapshot.series_family_hash != writer_series_family_hash(&book)
        || !anchor_registered
        || sleeve.writer_principal_atoms == 0
        || sleeve.writer_principal_atoms != sleeve.flat_par_supply_atoms
        || !activation_assets_sufficient
        || sleeve.accounted_asset_atoms < required_assets
        || physical_assets < sleeve.accounted_asset_atoms
        || recipe.phase != OracleRecipeWeightPhase::Finalized
        || recipe.recipe_hash != month.recipe_hash
        || recipe.processed_source_count != recipe.expected_source_count
        || recipe.processed_bucket_count != recipe.expected_bucket_count
        || settlement_sources.phase != OracleRecipeWeightPhase::Finalized
        || settlement_sources.processed_source_count != settlement_sources.expected_source_count
        || settlement_sources.processed_bucket_count != settlement_sources.expected_bucket_count
        || active.max_open_interest_payout == 0
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let slot = Clock::get()?.slot;
    group.product_manifest_root = coverage.required_sku_root;
    group.coverage_manifest_hash = writer_coverage_manifest_hash(&coverage);
    group.recipe_hash = recipe.recipe_hash;
    group.settlement_source_digest = settlement_sources.rolling_source_digest;
    group.active_weight_manifest_hash = active.rolling_manifest_hash;
    group.security_cap_atoms = active.max_open_interest_payout;
    group.signer_set = *signer_set_info.key;
    group.signer_set_version = signer_set.version;
    group.signer_set_hash = signer_set.set_hash;
    group.status = WriterSettlementGroupStatus::Active;
    group.last_updated_slot = slot;
    sleeve.status = WriterSleeveStatus::Active;
    sleeve.last_updated_slot = slot;
    store_state(group_info, &group)?;
    store_state(sleeve_info, &sleeve)
}

pub(super) fn process_set_collective_market_paused(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SetCollectiveMarketPausedV1Params,
) -> ProgramResult {
    if accounts.len() != 9 {
        return Err(VaultError::InvalidAccountList.into());
    }
    let admin_info = &accounts[0];
    let config_info = &accounts[1];
    let sleeve_info = &accounts[2];
    let group_info = &accounts[3];
    let book_info = &accounts[4];
    let market_info = &accounts[5];
    let month_info = &accounts[6];
    let coverage_info = &accounts[7];
    let active_weight_info = &accounts[8];
    if !admin_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let config = load_canonical_vault_config(program_id, config_info)?;
    if config.admin != *admin_info.key {
        return Err(VaultError::Unauthorized.into());
    }
    let WriterBookContext {
        group,
        sleeve,
        book,
    } = load_writer_book_context(program_id, sleeve_info, group_info, book_info)?;
    let record = book
        .records
        .iter()
        .take(usize::from(book.series_count))
        .find(|record| record.market == *market_info.key)
        .ok_or(VaultError::InvalidWriterSeriesBook)?;
    if group.sleeve != *sleeve_info.key {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    let mut market = load_valid_market(program_id, market_info)?;
    if record.series_id != market.market_id
        || Some(record.contract_mint) != market.long_contract_mint
        || market.instrument.underlying_id != group.underlying_id
        || market.instrument.expiry_ts != group.expiry_ts
    {
        return Err(VaultError::InvalidWriterSeriesBook.into());
    }
    if !params.paused {
        if config.paused
            || sleeve.status != WriterSleeveStatus::Active
            || group.status != WriterSettlementGroupStatus::Active
        {
            return Err(VaultError::InvalidWriterLifecycle.into());
        }
        let month = load_oracle_month_state(month_info, program_id)?;
        let (expected_month, expected_bump) =
            derive_oracle_month_pda(program_id, &group.anchor_market, group.expiry_ts);
        if *month_info.key != group.anchor_oracle_month
            || *month_info.key != expected_month
            || !month.is_initialized
            || month.bump != expected_bump
            || month.market != group.anchor_market
        {
            return Err(VaultError::InvalidOracleMonthAccount.into());
        }
        // All registered Markets share the group's expiry, so the canonical anchor month's
        // Game/timing window is safely checked against this target Market without inventing a
        // second settlement world or requiring an extra, undocumented account meta.
        ensure_oracle_game_window(&market, &month)?;
        let coverage =
            load_valid_oracle_sku_coverage_manifest(program_id, month_info.key, coverage_info)?;
        let active = load_valid_oracle_active_weight_manifest(
            program_id,
            month_info.key,
            active_weight_info,
        )?;
        ensure_finalized_oracle_active_weight_manifest(&month, &active)?;
        ensure_finalized_oracle_issue_sku_coverage(&month, &coverage, &active)?;
        if active.rolling_manifest_hash != group.active_weight_manifest_hash
            || writer_coverage_manifest_hash(&coverage) != group.coverage_manifest_hash
        {
            return Err(VaultError::InvalidWriterSettlementGroup.into());
        }
    }
    market.paused = params.paused;
    store_state(market_info, &market)
}
