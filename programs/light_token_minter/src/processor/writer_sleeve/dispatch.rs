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
fn process_dlmm_action(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let action = crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::decode_exact(payload)
        .map_err(|_| VaultError::InvalidInstructionData)?;
    match action {
        crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::Individual(action) => individual::process(program_id, accounts, action),
        crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::EnableFullCollateralCapacity {
            expected_policy_hash,
        } => dlmm::process_enable_full_collateral_capacity(program_id, accounts, expected_policy_hash),
        crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::EnableSharedReserve {
            expected_policy_hash,
        } => dlmm::process_enable_shared_reserve(program_id, accounts, expected_policy_hash),
        crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::BeginPolicy(_)
        | crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::AppendPolicySeries { .. }
        | crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::SealPolicy => {
            dlmm::process_policy_action(program_id, accounts, action)
        }
        crate::writer_dlmm_instruction::ManageWriterDlmmV1Params::InitializePosition {
            series_index,
        } => dlmm::process_initialize_position(program_id, accounts, series_index),
        _ => dlmm::process_liquidity_action(program_id, accounts, action),
    }
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
        VaultInstructionTag::ManageDlmmOrdersV1 => {
            decode_and_process::<crate::dlmm_order_state::DlmmOrderAction>(
                program_id,
                accounts,
                payload,
                crate::processor::ameba_dlmm::orders::process,
            )
        }
        VaultInstructionTag::ManageWriterParticipationV2 => {
            decode_and_process::<crate::writer_participation_state::WriterParticipationActionV2>(
                program_id,
                accounts,
                payload,
                participation::process,
            )
        }
        VaultInstructionTag::InitializeWriterPolicyRegistryV1 => {
            decode_and_process::<InitializeWriterPolicyRegistryV1Params>(
                program_id,
                accounts,
                payload,
                process_initialize_policy_registry,
            )
        }
        VaultInstructionTag::ManageWriterDlmmV1 => {
            process_dlmm_action(program_id, accounts, payload)
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
            process_initialize_sleeve(program_id, accounts, decode_u64_payload(payload)?)
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
        VaultInstructionTag::PublishWriterGroupSettlementV1 => {
            settlement::process_publish_writer_group_settlement(program_id, accounts, payload)
        }
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
        VaultInstructionTag::CloseWriterSleeveV1 => empty_and_process(
            program_id,
            accounts,
            payload,
            settlement::process_close_writer_sleeve,
        ),
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}
