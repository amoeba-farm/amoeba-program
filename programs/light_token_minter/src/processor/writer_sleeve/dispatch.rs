use super::*;

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

#[inline(never)]
pub(in crate::processor) fn process_instruction(
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
    if matches!(
        tag,
        VaultInstructionTag::ScopedCollectiveSettlementV1
            | VaultInstructionTag::ScopedPositionSettlementV1
    ) {
        if payload.len() != 1 || payload[0] > 2 {
            return Err(VaultError::InvalidInstructionData.into());
        }
        return if tag == VaultInstructionTag::ScopedCollectiveSettlementV1 {
            settlement::process_scoped_collective_settlement(program_id, accounts, payload[0])
        } else {
            crate::processor::ameba_dlmm::process_scoped_position_settlement(
                program_id, accounts, payload[0],
            )
        };
    }
    match tag {
        VaultInstructionTag::InitializeWriterPolicyRegistryV1 => {
            decode_and_process::<InitializeWriterPolicyRegistryV1Params>(
                program_id,
                accounts,
                payload,
                process_initialize_policy_registry,
            )
        }
        VaultInstructionTag::ManageWriterDlmmV1 => {
            let action =
                crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::decode_exact(payload)
                    .map_err(|_| VaultError::InvalidInstructionData)?;
            match action {
                crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::BeginPolicy(_)
                | crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::AppendPolicySeries {
                    ..
                }
                | crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::SealPolicy => {
                    dlmm::process_policy_action(program_id, accounts, action)
                }
                crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::InitializePosition {
                    series_index,
                } => dlmm::process_initialize_position(program_id, accounts, series_index),
                _ => dlmm::process_liquidity_action(program_id, accounts, action),
            }
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
        VaultInstructionTag::PrepareWriterBidIndexV1 => {
            decode_and_process::<PrepareWriterBidIndexV1Params>(
                program_id,
                accounts,
                payload,
                bid_index_preparation::process_prepare_writer_bid_index,
            )
        }
        VaultInstructionTag::CommitWriterAuctionV1 => {
            // Historical wire decoding remains available, but new auction creation is
            // permanently retired by writer-owned native DLMM primary distribution.
            // Existing auction fills, finalization and refunds retain their handlers.
            Err(VaultError::InvalidInstructionData.into())
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
