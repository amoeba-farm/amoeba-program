//! Admin-signed reclamation of terminal oracle bookkeeping once the period is Settled.
//! Financial accounts are never cleanup targets, so every bond, escrow and reward stays
//! claimable; bucket medians wait for frozen reward entitlements. A permanent receipt binds the
//! recipient and commits every removed account's bytes and SOL.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::instruction::TerminalCleanupParams;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::hash::hashv;
mod targets;

pub(super) const RECEIPT_SEED: &[u8] = b"g3-terminal-cleanup-v1";
const RECEIPT_LEN: usize = 186;
const DOMAIN: &[u8] = b"ameba-terminal-cleanup-v1";

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, BorshSerialize)]
    pub struct CleanupReceipt {
        pub initialized: bool,
        pub bump: u8,
        pub discriminator: [u8; 3],
        pub version: u8,
        pub month: Pubkey,
        pub settlement: Pubkey,
        pub period: Pubkey,
        pub recipient: Pubkey,
        pub closed_count: u32,
        pub refunded_lamports: u64,
        pub archive_hash: [u8; 32],
        pub started_slot: u64,
    }
}

pub(super) fn receipt_address(program: &Pubkey, month: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CURRENT_STATE_NAMESPACE_SEED, RECEIPT_SEED, month.as_ref()],
        program,
    )
}

fn save_receipt(program: &Pubkey, info: &AccountInfo, value: &CleanupReceipt) -> ProgramResult {
    if info.owner != program
        || info.executable
        || !info.is_writable
        || info.data_len() != RECEIPT_LEN
    {
        return Err(OracleError::InvalidPda.into());
    }
    let mut data = info.try_data_mut()?;
    let mut output = &mut data[..];
    value
        .serialize(&mut output)
        .map_err(|_| OracleError::InvalidOracleState)?;
    if !output.is_empty() {
        return Err(OracleError::InvalidOracleState.into());
    }
    Ok(())
}

pub(super) fn account_count(params: &TerminalCleanupParams) -> Result<usize, ProgramError> {
    if params.kind > 6
        || (params.kind != 3 && params.bucket_id != [0; 32])
        || (params.kind == 3 && params.bucket_id == [0; 32])
        || (params.kind == 0
            && (params.expected_data_hash != [0; 32] || params.expected_lamports != 0))
        || (params.kind != 0
            && (params.expected_data_hash == [0; 32] || params.expected_lamports == 0))
    {
        return Err(OracleError::InvalidInstructionData.into());
    }
    Ok(match params.kind {
        0 => 10,
        1 | 2 | 5 => 12,
        _ => 11,
    })
}

fn validate_metas(a: &[AccountInfo], count: usize) -> ProgramResult {
    if a.len() != count {
        return Err(OracleError::InvalidAccountList.into());
    }
    for (i, info) in a.iter().enumerate() {
        let alias = i == 5 && info.key == a[0].key;
        let empty_successor = i == 9 && info.key == a[7].key;
        if info.is_signer != (i == 0 || alias)
            || info.is_writable != matches!(i, 0 | 3 | 5 | 6 | 10)
            || (info.executable && i != 7 && !empty_successor)
            || (a[..i].iter().any(|prior| prior.key == info.key) && !alias && !empty_successor)
        {
            return Err(OracleError::InvalidAccountList.into());
        }
    }
    validate_system_program(&a[7])?;
    if crate::pubkey_is_default(a[5].key) {
        return Err(OracleError::InvalidAccountList.into());
    }
    Ok(())
}

#[inline(never)]
pub(super) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    params: TerminalCleanupParams,
) -> ProgramResult {
    validate_metas(a, account_count(&params)?)?;
    let config = load_canonical_vault_config(program, &a[1])?;
    if config.admin != *a[0].key {
        return Err(OracleError::Unauthorized.into());
    }
    let (anchor, mut month) = load_valid_period_and_oracle_month(program, &a[2], &a[3])?;
    if !matches!(month.phase, OraclePhase::Settled | OraclePhase::Closed)
        || month.pending_resolution_count != 0
        || month.settlement_status != OracleSettlementStatus::Final
        || month.finalized_at_ts == 0
        || month.settlement_record != Some(*a[4].key)
        || anchor.period.status != PeriodStatus::Settled
    {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    ensure_settlement_finalization_ready_at(anchor.expiry_ts, current_unix_timestamp()?)?;
    let settlement = load_valid_settlement_record_v2(program, a[2].key, &anchor, &a[4])?;
    if settlement.oracle_month != *a[3].key {
        return Err(OracleError::InvalidSettlementRecord.into());
    }
    let (address, bump) = receipt_address(program, a[3].key);
    if address != *a[6].key {
        return Err(OracleError::InvalidPda.into());
    }
    if params.kind == 0 {
        carry::require_cleanup_dependencies(program, &anchor, a[3].key, &a[8], &a[9])?;
        validate_create_only_program_account_target(program, &a[6])?;
        let slot = crate::compact_error::slot()?;
        let receipt = CleanupReceipt {
            initialized: true,
            bump,
            discriminator: *b"TRC",
            version: 1,
            month: *a[3].key,
            settlement: *a[4].key,
            period: *a[2].key,
            recipient: *a[5].key,
            closed_count: 0,
            refunded_lamports: 0,
            started_slot: slot,
            archive_hash: hashv(&[
                DOMAIN,
                program.as_ref(),
                a[3].key.as_ref(),
                a[4].key.as_ref(),
                a[2].key.as_ref(),
                a[5].key.as_ref(),
            ])
            .to_bytes(),
        };
        create_program_account(
            &a[0],
            &a[6],
            &a[7],
            program,
            RECEIPT_LEN,
            &[RECEIPT_SEED, a[3].key.as_ref(), &[bump]],
        )?;
        save_receipt(program, &a[6], &receipt)?;
        month.phase = OraclePhase::Closed;
        month.last_updated_slot = slot;
        return store_oracle_month_state(&a[3], &month);
    }
    if month.phase != OraclePhase::Closed {
        return Err(OracleError::InvalidOraclePhase.into());
    }
    if a[6].owner != program || a[6].data_len() != RECEIPT_LEN {
        return Err(OracleError::InvalidPda.into());
    }
    let mut receipt =
        CleanupReceipt::try_from_slice(&a[6].try_data()?).map_err(|_| OracleError::InvalidPda)?;
    if !receipt.initialized
        || receipt.bump != bump
        || receipt.discriminator != *b"TRC"
        || receipt.version != 1
        || receipt.month != *a[3].key
        || receipt.settlement != *a[4].key
        || receipt.period != *a[2].key
        || receipt.recipient != *a[5].key
        || receipt.started_slot == 0
    {
        return Err(OracleError::InvalidPda.into());
    }
    targets::validate(program, a[3].key, &month, &a[10..], &params)?;
    let hash = hashv(&[&a[10].try_data()?]).to_bytes();
    let lamports = a[10].lamports();
    if hash != params.expected_data_hash || lamports != params.expected_lamports {
        return Err(OracleError::InvalidOracleState.into());
    }
    receipt.closed_count = receipt
        .closed_count
        .checked_add(1)
        .ok_or(OracleError::ArithmeticOverflow)?;
    receipt.refunded_lamports = receipt
        .refunded_lamports
        .checked_add(lamports)
        .ok_or(OracleError::ArithmeticOverflow)?;
    receipt.archive_hash = hashv(&[
        DOMAIN,
        &receipt.archive_hash,
        &[params.kind],
        a[10].key.as_ref(),
        &hash,
        &lamports.to_le_bytes(),
    ])
    .to_bytes();
    close_program_account(program, &a[10], &a[5])?;
    save_receipt(program, &a[6], &receipt)
}
