//! Independent bounded certificates for populations larger than the inline fast path.
use super::*;
use crate::oracle_rank::{encode_signed, MedianRank};
use borsh::BorshSerialize;

const SEED: &[u8] = b"g3-bucket-rank-v1";
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub(super) struct BucketRank {
    header: Header,
    root: Pubkey,
    month: Pubkey,
    snapshot: [u8; 32],
    payer: Pubkey,
    nonce: [u8; 32],
    last_source: [u8; 32],
    total: u16,
    processed: u16,
    eligible: u16,
    rank: MedianRank,
    mode: u8,
    complete: bool,
}
impl Record for BucketRank {
    const DISCRIMINATOR: [u8; 3] = *b"OBR";
    const LEN: usize = 242;
    fn header(&self) -> &Header {
        &self.header
    }
}
fn rank_pda(program: &Pubkey, root: &Pubkey, payer: &Pubkey, nonce: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            root.as_ref(),
            payer.as_ref(),
            nonce,
        ],
        program,
    )
}
fn load_rank(program: &Pubkey, info: &AccountInfo) -> Result<BucketRank, ProgramError> {
    if info.owner != program || info.executable || info.data_len() != BucketRank::LEN {
        return invalid();
    }
    let raw = BucketRank::try_from_slice(&info.try_borrow_data()?)
        .map_err(|_| VaultError::InvalidOracleMedian)?;
    let value: BucketRank = load(
        program,
        info,
        rank_pda(program, &raw.root, &raw.payer, &raw.nonce),
    )?;
    if value.root == Pubkey::default()
        || value.month == Pubkey::default()
        || value.payer == Pubkey::default()
        || value.total == 0
        || value.processed > value.total
        || value.eligible > value.processed
        || value.rank.count != u32::from(value.eligible)
        || value.mode > 2
        || value.complete != (value.processed == value.total)
    {
        return invalid();
    }
    Ok(value)
}
pub(super) fn begin_bucket_rank(
    program: &Pubkey,
    a: &[AccountInfo],
    mode: u8,
    lower: u64,
    upper: u64,
    nonce: [u8; 32],
) -> ProgramResult {
    if a.len() != 7 || mode > 2 || lower > upper {
        return invalid();
    }
    let (market, month) = load_valid_market_and_oracle_month(program, &a[1], &a[2])?;
    let root: crate::state::OracleBucketSourceIndex = load_exact_zero_padded_state(
        &a[4],
        program,
        crate::state::OracleBucketSourceIndex::LEN,
        VaultError::InvalidOracleWeightManifest,
    )?;
    let root = oracle_membership::load_complete_bucket_source_index(
        program,
        a[2].key,
        &month,
        &root.bucket_id,
        &a[3],
        &a[4],
    )?;
    if mode == 2 {
        ensure_oracle_opening_sources_terminal(&month)?;
    } else {
        if month.phase != OraclePhase::Game
            || month.settlement_status == OracleSettlementStatus::Final
        {
            return invalid();
        }
        ensure_oracle_opening_resolution_complete(&month)?;
        ensure_settlement_finalization_ready_at(market.instrument.expiry_ts, now()?)?;
    }
    let pda = rank_pda(program, a[4].key, a[0].key, &nonce);
    let value = BucketRank {
        header: header::<BucketRank>(pda.1),
        root: *a[4].key,
        month: *a[2].key,
        snapshot: initial_oracle_bucket_source_snapshot(a[2].key, &root.bucket_id, mode),
        payer: *a[0].key,
        nonce,
        last_source: [0; 32],
        total: root.source_count,
        processed: 0,
        eligible: 0,
        rank: MedianRank {
            lower,
            upper,
            ..Default::default()
        },
        mode,
        complete: false,
    };
    create(
        program,
        &a[0],
        &a[5],
        &a[6],
        &[SEED, a[4].key.as_ref(), a[0].key.as_ref(), &nonce, &[pda.1]],
        &value,
    )
}

pub(super) fn scan_bucket_rank(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 9 || !a[0].is_signer {
        return invalid();
    }
    let (market, month) = load_valid_market_and_oracle_month(program, &a[1], &a[2])?;
    let mut value = load_rank(program, &a[8])?;
    if value.complete || value.month != *a[2].key || value.root != *a[4].key {
        return invalid();
    }
    if value.mode == 2 {
        ensure_oracle_opening_sources_terminal(&month)?;
    } else {
        if month.phase != OraclePhase::Game
            || month.settlement_status == OracleSettlementStatus::Final
        {
            return invalid();
        }
        ensure_oracle_opening_resolution_complete(&month)?;
        ensure_settlement_finalization_ready_at(market.instrument.expiry_ts, now()?)?;
    }
    let source = load_valid_oracle_source(program, a[2].key, &a[5])?;
    let root = oracle_membership::load_complete_bucket_source_index(
        program,
        a[2].key,
        &month,
        &source.bucket_id,
        &a[3],
        &a[4],
    )?;
    if root.source_count != value.total
        || source.bucket_weight_bps != root.bucket_weight_bps
        || (value.processed > 0 && source.source_id <= value.last_source)
        || !matches!(
            source.status,
            OracleSourceStatus::Active | OracleSourceStatus::Inactive
        )
    {
        return invalid();
    }
    oracle_membership::require_member(
        program,
        a[4].key,
        &root,
        &a[6],
        value.processed,
        &source.source_id,
    )?;
    let current = if source.status == OracleSourceStatus::Inactive {
        if source.observation_count != 0 || source.current_state != 0 || source.baseline_state != 0
        {
            return invalid();
        }
        None
    } else if value.mode == 2 {
        if source.baseline_state == 0 || source.current_state == 0 || source.observation_count == 0
        {
            return invalid();
        }
        Some(source.current_state)
    } else {
        let days = crate::constants::ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS
            .checked_add(if value.mode == 1 {
                crate::constants::ORACLE_SETTLEMENT_FRESHNESS_GRACE_BUSINESS_DAYS
            } else {
                0
            })
            .ok_or(VaultError::ArithmeticOverflow)?;
        verified_history_median(
            program,
            &a[7],
            a[5].key,
            &source,
            oracle_settlement_window_start(market.instrument.expiry_ts, days)?,
            market.instrument.expiry_ts,
            value.mode,
        )?
    };
    if let Some(current) = current {
        value.rank.observe(encode_signed(source_delta_bps(
            source.baseline_state,
            current,
        )?))?;
        value.eligible = value
            .eligible
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    if source.status == OracleSourceStatus::Active {
        value.snapshot = advance_oracle_bucket_source_snapshot(&value.snapshot, &source);
    }
    value.last_source = source.source_id;
    value.processed = value
        .processed
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    if value.processed == value.total {
        if value.rank.count > 0 {
            value.rank.signed_median()?;
        } else if value.rank.lower != 0 || value.rank.upper != 0 {
            return invalid();
        }
        value.complete = true;
    }
    save(program, &a[8], &value)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn verified_bucket_rank(
    program: &Pubkey,
    info: &AccountInfo,
    root: &Pubkey,
    month: &Pubkey,
    snapshot: &[u8; 32],
    total: u16,
    eligible: u16,
    mode: u8,
) -> Result<i64, ProgramError> {
    if info.is_signer || info.is_writable {
        return invalid();
    }
    let value = load_rank(program, info)?;
    if !value.complete
        || value.root != *root
        || value.month != *month
        || value.snapshot != *snapshot
        || value.total != total
        || value.eligible != eligible
        || value.mode != mode
    {
        return invalid();
    }
    value.rank.signed_median()
}

pub(super) fn close_bucket_rank(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 2 || !a[0].is_signer || !a[0].is_writable || !a[1].is_writable {
        return invalid();
    }
    let value = load_rank(program, &a[1])?;
    if value.payer != *a[0].key {
        return invalid();
    }
    let total = a[0]
        .lamports()
        .checked_add(a[1].lamports())
        .ok_or(VaultError::ArithmeticOverflow)?;
    **a[0].try_borrow_mut_lamports()? = total;
    **a[1].try_borrow_mut_lamports()? = 0;
    a[1].resize(0)?;
    a[1].assign(&system_program::id());
    Ok(())
}
