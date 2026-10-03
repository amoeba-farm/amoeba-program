use super::*;

pub(super) fn validate_pack_writer_account_privileges(
    program_id: &Pubkey,
    tag: VaultInstructionTag,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let count = accounts.len();
    let valid_count = match tag {
        VaultInstructionTag::PublishWriterGroupSettlementV1 => match payload {
            [] => count == 13,
            bytes if bytes == crate::writer_settlement_handoff::HANDOFF_PAYLOAD => count == 15,
            _ => return Err(ProgramError::InvalidInstructionData),
        },
        VaultInstructionTag::SetCollectiveMarketPausedV1 => count == 9,
        VaultInstructionTag::ReconcileWriterSupplyV1 => count == 16,
        VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => {
            (9..=9 + crate::constants::WRITER_MAX_LIVE_SERIES).contains(&count)
        }
        _ => return Ok(()),
    };
    if !valid_count {
        return Err(VaultError::InvalidAccountList.into());
    }
    for (index, account) in accounts.iter().enumerate() {
        let signer = index == 0;
        let writable = match tag {
            VaultInstructionTag::PublishWriterGroupSettlementV1 => {
                matches!(index, 2 | 3) || (count == 15 && matches!(index, 0 | 13))
            }
            VaultInstructionTag::SetCollectiveMarketPausedV1 => index == 5,
            VaultInstructionTag::ReconcileWriterSupplyV1 => {
                matches!(index, 2 | 4 | 6 | 7 | 8 | 9 | 10)
            }
            VaultInstructionTag::FinalizeWriterSleeveSettlementV1 => {
                matches!(index, 2 | 4)
                    || (index == count - 1
                        && account.key
                            == &crate::compressed_custody::derive_compressed_custody(
                                program_id,
                                crate::compressed_custody::CustodyKind::WriterCash,
                                accounts[6].key,
                            )
                            .0)
            }
            _ => matches!(index, 2 | 4),
        };
        if account.is_signer != signer
            || (account.is_writable != writable && !(signer && account.is_writable))
            || accounts[..index]
                .iter()
                .any(|other| crate::pubkey_eq(other.key, account.key))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    Ok(())
}
