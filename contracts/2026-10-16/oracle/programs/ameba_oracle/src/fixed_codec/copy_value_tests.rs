//! Typed value-copy coverage; account serialization is checked independently.
use crate::fixed_codec::ReferenceBorsh;
use crate::state::{OracleSourceState, OracleSourceStatus};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;
use std::fmt::Debug;

fn bytes<const N: usize>(seed: u64) -> [u8; N] {
    std::array::from_fn(|i| seed.wrapping_add((i as u64).wrapping_mul(17)) as u8)
}

fn verify<T>(mut original: T, mutate: impl FnOnce(&mut T))
where
    T: Copy + Clone + PartialEq + Debug + ReferenceBorsh + BorshSerialize + BorshDeserialize,
{
    assert!(!std::mem::needs_drop::<T>());
    let expected = original.reference_borsh_bytes();
    let copied = original; // Compile-time Copy assertion; original remains usable.

    // Exercise the explicit Clone implementation in this contract test.
    #[allow(clippy::clone_on_copy)]
    let cloned = original.clone();
    assert_eq!(copied, original);
    assert_eq!(cloned, original);
    assert_eq!(copied.reference_borsh_bytes(), expected);
    assert_eq!(cloned.reference_borsh_bytes(), expected);
    assert_eq!(copied.try_to_vec().unwrap(), expected);
    assert_eq!(cloned.try_to_vec().unwrap(), expected);
    assert_eq!(T::try_from_slice(&expected).unwrap(), original);
    mutate(&mut original);
    assert_ne!(original, copied);
    assert_eq!(copied, cloned);
    assert_eq!(copied.try_to_vec().unwrap(), expected);
}

fn oracle_source_state(seed: u64) -> OracleSourceState {
    OracleSourceState {
        is_initialized: (seed ^ 1u64) & 1 != 0,
        bump: (seed ^ 2u64) as u8,
        month: Pubkey::new_from_array(bytes(seed ^ 3u64)),
        source_id: bytes(seed ^ 4u64),
        bucket_id: bytes(seed ^ 5u64),
        source_type_hash: bytes(seed ^ 6u64),
        canonical_locator_hash: bytes(seed ^ 7u64),
        source_definition_hash: bytes(seed ^ 8u64),
        proposer: Pubkey::new_from_array(bytes(seed ^ 9u64)),
        baseline_state: seed ^ 10u64,
        current_state: seed ^ 11u64,
        listing_bond_locked: seed ^ 12u64,
        support_stake_total: seed ^ 13u64,
        bucket_weight_bps: (seed ^ 14u64) as u16,
        status: if (seed ^ 15u64) & 1 == 0 {
            OracleSourceStatus::Candidate
        } else {
            OracleSourceStatus::TimedOut
        },
        opening_submitted: (seed ^ 16u64) & 1 != 0,
        opening_evidence_hash: bytes(seed ^ 17u64),
        last_finalized_step: seed ^ 18u64,
        observation_count: (seed ^ 19u64) as u32,
        latest_source_time: seed ^ 20u64,
        rolling_observation_hash: bytes(seed ^ 21u64),
    }
}

#[test]
fn oracle_source_state_copy_clone_preserves_fields_and_wire() {
    for seed in [0, 1, u64::MAX] {
        verify(oracle_source_state(seed), |value| {
            value.is_initialized = !value.is_initialized;
        });
    }
}
