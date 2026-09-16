//! One immutable checkpoint per history page; no lifetime sample eviction.
//! A caller proposes both central order statistics and proves them by walking
//! the entire snapshot. Guessing incorrectly cannot reserve the canonical path:
//! each proposal has its own deterministic account.
use super::*;
use crate::oracle_rank::MedianRank;
use borsh::BorshSerialize;

const SEED: &[u8] = b"g3-history-median-v1";

#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub(super) struct HistoryMedian {
    header: Header,
    source: Pubkey,
    snapshot: [u8; 32],
    cursor: Pubkey,
    payer: Pubkey,
    total: u32,
    remaining: u32,
    start: u64,
    end: u64,
    last_time: u64,
    standing: u64,
    rank: MedianRank,
    mode: u8,
    inherited: bool,
    complete: bool,
}
impl Record for HistoryMedian {
    const DISCRIMINATOR: [u8; 3] = *b"OHM";
    const LEN: usize = 213;
    fn header(&self) -> &Header {
        &self.header
    }
}

fn pda(
    program: &Pubkey,
    source: &Pubkey,
    payer: &Pubkey,
    snapshot: &[u8; 32],
    mode: u8,
    lower: u64,
    upper: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            source.as_ref(),
            payer.as_ref(),
            snapshot,
            &[mode],
            &lower.to_le_bytes(),
            &upper.to_le_bytes(),
        ],
        program,
    )
}

fn load_median(program: &Pubkey, info: &AccountInfo) -> Result<HistoryMedian, ProgramError> {
    if info.owner != program || info.executable || info.data_len() != HistoryMedian::LEN {
        return invalid();
    }
    let value = HistoryMedian::try_from_slice(&info.try_borrow_data()?)
        .map_err(|_| VaultError::InvalidOracleMedian)?;
    let value: HistoryMedian = load(
        program,
        info,
        pda(
            program,
            &value.source,
            &value.payer,
            &value.snapshot,
            value.mode,
            value.rank.lower,
            value.rank.upper,
        ),
    )?;
    if value.source == Pubkey::default()
        || value.payer == Pubkey::default()
        || value.snapshot == [0; 32]
        || value.mode > 1
        || value.total == 0
        || value.remaining > value.total
        || value.complete != (value.remaining == 0)
        || value.complete != (value.cursor == Pubkey::default())
        || value.start == 0
        || value.start > value.end
        || value.rank.count > value.total - value.remaining
    {
        return invalid();
    }
    Ok(value)
}

pub(super) fn begin_history_median(
    program: &Pubkey,
    a: &[AccountInfo],
    mode: u8,
    lower: u64,
    upper: u64,
) -> ProgramResult {
    if a.len() != 7 || mode > 1 || lower > upper {
        return invalid();
    }
    let (market, month) = load_valid_market_and_oracle_month(program, &a[1], &a[2])?;
    ensure_oracle_opening_resolution_complete(&month)?;
    ensure_settlement_finalization_ready_at(market.instrument.expiry_ts, now()?)?;
    if month.phase != OraclePhase::Game || month.settlement_status == OracleSettlementStatus::Final
    {
        return invalid();
    }
    let source = load_valid_oracle_source(program, a[2].key, &a[3])?;
    let journal = load_journal(program, a[3].key, &a[4])?;
    if source.status != OracleSourceStatus::Active
        || journal.count != source.observation_count
        || journal.rolling_observation_hash != source.rolling_observation_hash
    {
        return invalid();
    }
    let days = crate::constants::ORACLE_SETTLEMENT_FRESHNESS_BUSINESS_DAYS
        .checked_add(if mode == 1 {
            crate::constants::ORACLE_SETTLEMENT_FRESHNESS_GRACE_BUSINESS_DAYS
        } else {
            0
        })
        .ok_or(VaultError::ArithmeticOverflow)?;
    let address = pda(
        program,
        a[3].key,
        a[0].key,
        &source.rolling_observation_hash,
        mode,
        lower,
        upper,
    );
    let value = HistoryMedian {
        header: header::<HistoryMedian>(address.1),
        source: *a[3].key,
        snapshot: source.rolling_observation_hash,
        cursor: journal.head,
        payer: *a[0].key,
        total: source.observation_count,
        remaining: source.observation_count,
        start: oracle_settlement_window_start(market.instrument.expiry_ts, days)?,
        end: market.instrument.expiry_ts,
        last_time: 0,
        standing: 0,
        rank: MedianRank {
            lower,
            upper,
            ..Default::default()
        },
        mode,
        inherited: false,
        complete: false,
    };
    create(
        program,
        &a[0],
        &a[5],
        &a[6],
        &[
            SEED,
            a[3].key.as_ref(),
            a[0].key.as_ref(),
            &value.snapshot,
            &[mode],
            &lower.to_le_bytes(),
            &upper.to_le_bytes(),
            &[address.1],
        ],
        &value,
    )
}

fn apply_checkpoint(
    value: &mut HistoryMedian,
    checkpoint_key: &Pubkey,
    checkpoint: &Checkpoint,
) -> ProgramResult {
    if value.complete
        || value.cursor != *checkpoint_key
        || checkpoint.source != value.source
        || checkpoint.sequence != value.remaining
        || (value.last_time != 0 && checkpoint.observed_at >= value.last_time)
    {
        return invalid();
    }
    let inherited = checkpoint.sequence == 1
        && (checkpoint.origin_source != value.source
            || checkpoint.origin_checkpoint != *checkpoint_key);
    if checkpoint.sequence == 1 {
        value.inherited = inherited;
    }
    if !inherited && (value.start..=value.end).contains(&checkpoint.observed_at) {
        value.rank.observe(checkpoint.value)?;
    }
    if value.standing == 0 && checkpoint.observed_at <= value.start {
        value.standing = checkpoint.value;
    }
    if inherited && value.standing == 0 && checkpoint.observed_at <= value.end {
        value.standing = checkpoint.value;
    }
    value.last_time = checkpoint.observed_at;
    value.remaining = value
        .remaining
        .checked_sub(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    value.cursor = checkpoint.previous;
    if value.remaining == 0 {
        if value.cursor != Pubkey::default() {
            return invalid();
        }
        if value.rank.count > 0 {
            value.rank.median()?;
        } else if value.rank.lower != 0 || value.rank.upper != 0 {
            return invalid();
        }
        value.complete = true;
    }
    Ok(())
}

pub(super) fn scan_history_median(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 4 || !a[0].is_signer {
        return invalid();
    }
    let mut value = load_median(program, &a[2])?;
    if value.source != *a[1].key {
        return invalid();
    }
    let checkpoint = load_checkpoint(program, &a[3], &value.source)?;
    apply_checkpoint(&mut value, a[3].key, &checkpoint)?;
    save(program, &a[2], &value)
}

pub(in crate::processor) fn verified_history_median(
    program: &Pubkey,
    info: &AccountInfo,
    source_key: &Pubkey,
    source: &OracleSourceState,
    start: u64,
    end: u64,
    mode: u8,
) -> Result<Option<u64>, ProgramError> {
    if info.is_writable || info.is_signer {
        return invalid();
    }
    let value = load_median(program, info)?;
    if !value.complete
        || value.source != *source_key
        || value.snapshot != source.rolling_observation_hash
        || value.total != source.observation_count
        || value.start != start
        || value.end != end
        || value.mode != mode
    {
        return invalid();
    }
    if value.rank.count > 0 {
        Ok(Some(value.rank.median()?))
    } else {
        Ok((value.standing != 0).then_some(value.standing))
    }
}

pub(super) fn close_history_median(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 2 || !a[0].is_signer || !a[0].is_writable || !a[1].is_writable {
        return invalid();
    }
    let value = load_median(program, &a[1])?;
    if value.payer != *a[0].key {
        return invalid();
    }
    let amount = a[1].lamports();
    let balance = a[0]
        .lamports()
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    **a[0].try_borrow_mut_lamports()? = balance;
    **a[1].try_borrow_mut_lamports()? = 0;
    a[1].resize(0)?;
    a[1].assign(&system_program::id());
    Ok(())
}
