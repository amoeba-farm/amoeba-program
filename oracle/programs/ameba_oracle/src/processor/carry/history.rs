//! One immutable checkpoint per history page; no lifetime sample eviction.
//! A caller proposes both central order statistics and proves them by walking
//! the entire snapshot. Guessing incorrectly cannot reserve the canonical path:
//! each proposal has its own deterministic account.
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::oracle_rank::MedianRank;
use borsh::BorshSerialize;

const SEED: &[u8] = b"g3-history-median-v1";

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, BorshSerialize)]
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
    let value = HistoryMedian::try_from_slice(&info.try_data()?)
        .map_err(|_| OracleError::InvalidOracleMedian)?;
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
    if crate::pubkey_is_default(&value.source)
        || crate::pubkey_is_default(&value.payer)
        || value.snapshot == [0; 32]
        || value.mode > 1
        || value.total == 0
        || value.remaining > value.total
        || value.complete != (value.remaining == 0)
        || value.complete != (crate::pubkey_is_default(&value.cursor))
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
    let (anchor, month) = load_valid_period_and_oracle_month(program, &a[1], &a[2])?;
    ensure_oracle_opening_resolution_complete(&month)?;
    ensure_settlement_finalization_ready_at(anchor.expiry_ts, now()?)?;
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
        .ok_or(OracleError::ArithmeticOverflow)?;
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
        start: oracle_settlement_window_start(anchor.expiry_ts, days)?,
        end: anchor.expiry_ts,
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
        .ok_or(OracleError::ArithmeticOverflow)?;
    value.cursor = checkpoint.previous;
    if value.remaining == 0 {
        if !crate::pubkey_is_default(&value.cursor) {
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
        .ok_or(OracleError::ArithmeticOverflow)?;
    **a[0].try_lamports_mut()? = balance;
    **a[1].try_lamports_mut()? = 0;
    a[1].resize(0)?;
    a[1].assign(&system_program::id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(
        n: u32,
        inherited: bool,
        start: u64,
        end: u64,
    ) -> (HistoryMedian, Vec<(Pubkey, Checkpoint)>) {
        let source = Pubkey::new_unique();
        let program = Pubkey::new_unique();
        let mut pages = Vec::new();
        let mut previous = Pubkey::default();
        for sequence in 1..=n {
            let event = Pubkey::new_unique();
            let (key, bump) = checkpoint_address(&program, &source, &event);
            let value = u64::from((sequence * 7919) % 37) + 1;
            let checkpoint = Checkpoint {
                header: header::<Checkpoint>(bump),
                source,
                month: Pubkey::new_unique(),
                event,
                previous,
                sequence,
                accepted_at: u64::from(sequence) + 1000,
                value,
                observed_at: u64::from(sequence) * 10,
                evidence_hash: [3; 32],
                archive_hash: [4; 32],
                contributor: Pubkey::new_unique(),
                origin_source: if inherited && sequence == 1 {
                    Pubkey::new_unique()
                } else {
                    source
                },
                origin_checkpoint: key,
            };
            previous = key;
            pages.push((key, checkpoint));
        }
        let mut samples: Vec<_> = pages
            .iter()
            .filter(|(_, p)| {
                (!inherited || p.sequence != 1) && (start..=end).contains(&p.observed_at)
            })
            .map(|(_, p)| p.value)
            .collect();
        samples.sort_unstable();
        let (lower, upper) = if samples.is_empty() {
            (0, 0)
        } else {
            (samples[(samples.len() - 1) / 2], samples[samples.len() / 2])
        };
        (
            HistoryMedian {
                header: header::<HistoryMedian>(1),
                source,
                snapshot: [5; 32],
                cursor: previous,
                payer: Pubkey::new_unique(),
                total: n,
                remaining: n,
                start,
                end,
                last_time: 0,
                standing: 0,
                rank: MedianRank {
                    lower,
                    upper,
                    ..Default::default()
                },
                mode: 0,
                inherited,
                complete: false,
            },
            pages,
        )
    }
    #[test]
    fn complete_history_walk_matches_independent_reference_beyond_32() {
        for n in [1, 2, 32, 33, 64, 100] {
            for inherited in [false, true] {
                for (start, end) in [(1, 1000), (100, 500), (1500, 2000)] {
                    let (mut state, pages) = fixture(n, inherited, start, end);
                    for (key, page) in pages.iter().rev() {
                        apply_checkpoint(&mut state, key, page).unwrap();
                    }
                    assert!(state.complete);
                    assert_eq!(state.remaining, 0);
                    let mut samples: Vec<_> = pages
                        .iter()
                        .filter(|(_, p)| {
                            (!inherited || p.sequence != 1)
                                && (start..=end).contains(&p.observed_at)
                        })
                        .map(|(_, p)| p.value)
                        .collect();
                    samples.sort_unstable();
                    if !samples.is_empty() {
                        assert_eq!(
                            state.rank.median().unwrap(),
                            (samples[(samples.len() - 1) / 2] + samples[samples.len() / 2]) / 2
                        );
                    } else {
                        let standing = pages
                            .iter()
                            .rev()
                            .find(|(_, p)| p.observed_at <= start)
                            .map(|(_, p)| p.value)
                            .or_else(|| {
                                (inherited && pages[0].1.observed_at <= end)
                                    .then_some(pages[0].1.value)
                            })
                            .unwrap_or(0);
                        assert_eq!(state.standing, standing);
                    }
                    assert!(apply_checkpoint(&mut state, &pages[0].0, &pages[0].1).is_err());
                    assert_eq!(state.try_to_vec().unwrap().len(), HistoryMedian::LEN);
                }
            }
        }
    }
    #[test]
    fn missing_reordered_replayed_and_false_rank_history_cannot_complete() {
        let (state, pages) = fixture(100, false, 1, 1000);
        let mut candidate = state.clone();
        assert!(apply_checkpoint(&mut candidate, &pages[98].0, &pages[98].1).is_err());
        assert_eq!(candidate.try_to_vec().unwrap(), state.try_to_vec().unwrap());
        let mut bad = pages[99].1.clone();
        bad.sequence = 99;
        assert!(apply_checkpoint(&mut candidate, &pages[99].0, &bad).is_err());
        let mut bad = pages[99].1.clone();
        bad.source = Pubkey::new_unique();
        assert!(apply_checkpoint(&mut candidate, &pages[99].0, &bad).is_err());
        apply_checkpoint(&mut candidate, &pages[99].0, &pages[99].1).unwrap();
        assert!(apply_checkpoint(&mut candidate, &pages[99].0, &pages[99].1).is_err());
        let mut false_rank = state;
        false_rank.rank.lower = 999;
        false_rank.rank.upper = 999;
        for (key, page) in pages.iter().rev().take(99) {
            apply_checkpoint(&mut false_rank, key, page).unwrap();
        }
        assert!(apply_checkpoint(&mut false_rank, &pages[0].0, &pages[0].1).is_err());
        assert!(!false_rank.complete);
    }
    #[test]
    fn source_prefix_is_preserved_and_exhaustion_is_atomic() {
        let mut source = OracleSourceState::default();
        let mut observations = OracleSourceObservations {
            account_version: OracleSourceObservations::ACCOUNT_VERSION,
            ..Default::default()
        };
        let mut first_page = None;
        for n in 1..=100 {
            let state = n * 3;
            if n == 1 {
                source.baseline_state = state;
            }
            append_oracle_source_observation(
                &mut source,
                &mut observations,
                state,
                n * 10,
                &[1; 32],
                &[2; 32],
            )
            .unwrap();
            source.current_state = state;
            validate_oracle_observation_shape(&source, &observations).unwrap();
            if n == 32 {
                first_page = Some(observations.try_to_vec().unwrap());
            }
            if n > 32 {
                assert_eq!(
                    observations.try_to_vec().unwrap(),
                    *first_page.as_ref().unwrap()
                );
            }
        }
        assert_eq!(source.observation_count, 100);
        assert_eq!(source.latest_source_time, 1000);
        source.observation_count = u32::MAX;
        let before = source.try_to_vec().unwrap();
        assert!(append_oracle_source_observation(
            &mut source,
            &mut observations,
            301,
            1010,
            &[1; 32],
            &[2; 32]
        )
        .is_err());
        assert_eq!(source.try_to_vec().unwrap(), before);
    }
}

#[cfg(test)]
mod compact_borsh_equivalence {
    use super::*;
    use crate::fixed_codec::equivalence::check;

    #[test]
    fn compact_decoder_matches_derive_for_history_median() {
        check::<HistoryMedian>(0x5ef5, 320);
    }
}

/// Resume cursor of a verified history certificate; account layouts stay private.
pub fn history_median_cursor(
    program: &Pubkey,
    owner: &Pubkey,
    key: &Pubkey,
    data: &[u8],
) -> Option<Pubkey> {
    if owner != program || data.len() != HistoryMedian::LEN {
        return None;
    }
    let value = HistoryMedian::try_from_slice(data).ok()?;
    let (expected, bump) = pda(
        program,
        &value.source,
        &value.payer,
        &value.snapshot,
        value.mode,
        value.rank.lower,
        value.rank.upper,
    );
    if *key != expected
        || value.header.bump != bump
        || !value.header.initialized
        || value.header.discriminator != HistoryMedian::DISCRIMINATOR
        || value.header.version != HistoryMedian::VERSION
        || value.source == Pubkey::default()
        || value.payer == Pubkey::default()
        || value.snapshot == [0; 32]
        || value.mode > 1
        || value.start == 0
        || value.start > value.end
        || value.total == 0
        || value.remaining > value.total
        || value.rank.count > value.total - value.remaining
        || value.complete != (value.remaining == 0)
        || value.complete != (value.cursor == Pubkey::default())
    {
        return None;
    }
    Some(value.cursor)
}

pub fn history_median_address(
    program: &Pubkey,
    source: &Pubkey,
    payer: &Pubkey,
    snapshot: &[u8; 32],
    mode: u8,
    lower: u64,
    upper: u64,
) -> Pubkey {
    pda(program, source, payer, snapshot, mode, lower, upper).0
}
