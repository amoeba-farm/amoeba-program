//! Indicative Game reference from accepted standing source values. This is an
//! off-chain read helper, not the temporal median used for final settlement.
use crate::{
    processor::{bucket_index_contribution_bps, deterministic_bucket_median},
    state::{OracleMonthState, OraclePhase, OracleSourceState, OracleSourceStatus},
};
use solana_program::pubkey::Pubkey;
use std::collections::{BTreeMap, BTreeSet};

/// Callers verify account owners and canonical PDAs. Require a complete source
/// inventory and the full product weight, so a partial read cannot move prices.
/// Pending claims never appear here: only on-chain accepted `current_state`.
pub fn game_delta_bps(
    month_key: &Pubkey,
    month: &OracleMonthState,
    sources: &[OracleSourceState],
) -> Option<i64> {
    if month.phase != OraclePhase::Game || sources.len() != usize::from(month.source_count) {
        return None;
    }
    let mut ids = BTreeSet::new();
    let mut buckets: BTreeMap<[u8; 32], (u16, Vec<i64>)> = BTreeMap::new();
    for s in sources {
        if !s.is_initialized || s.month != *month_key || !ids.insert(s.source_id) {
            return None;
        }
        let b = buckets
            .entry(s.bucket_id)
            .or_insert((s.bucket_weight_bps, vec![]));
        if b.0 != s.bucket_weight_bps {
            return None;
        }
        if s.status != OracleSourceStatus::Active || !s.opening_submitted {
            continue;
        }
        if s.baseline_state == 0 || s.current_state == 0 || s.observation_count == 0 {
            return None;
        }
        let delta = (i128::from(s.current_state) - i128::from(s.baseline_state))
            .checked_mul(10_000)?
            / i128::from(s.baseline_state);
        b.1.push(i64::try_from(delta).ok()?);
    }
    let mut weight = 0u32;
    let mut delta = 0i64;
    for (_, (w, mut values)) in buckets {
        weight = weight.checked_add(u32::from(w))?;
        let median = deterministic_bucket_median(&mut values).ok()?;
        delta = delta.checked_add(bucket_index_contribution_bps(w, median).ok()?)?;
    }
    (weight == 10_000).then_some(delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bucket_medians_use_accepted_prices_and_require_complete_inventory() {
        let key = Pubkey::new_unique();
        let m = OracleMonthState {
            phase: OraclePhase::Game,
            source_count: 4,
            ..Default::default()
        };
        let mut sources: Vec<_> = [110, 120, 1000, 90]
            .into_iter()
            .enumerate()
            .map(|(i, value)| OracleSourceState {
                is_initialized: true,
                month: key,
                source_id: [i as u8; 32],
                bucket_id: [u8::from(i == 3); 32],
                bucket_weight_bps: 5000,
                status: OracleSourceStatus::Active,
                opening_submitted: true,
                baseline_state: 100,
                current_state: value,
                observation_count: 2,
                ..Default::default()
            })
            .collect();
        assert_eq!(game_delta_bps(&key, &m, &sources), Some(500)); // +20%, -10%, equal buckets
        assert_eq!(game_delta_bps(&key, &m, &sources[..3]), None);
        sources[3].source_id = sources[0].source_id;
        assert_eq!(game_delta_bps(&key, &m, &sources), None);
        sources[3].source_id = [3; 32];
        sources[3].month = Pubkey::new_unique();
        assert_eq!(game_delta_bps(&key, &m, &sources), None);
    }
}
