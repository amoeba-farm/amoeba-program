use super::*;
use crate::compact_error::CompactAccountInfo;
use borsh::BorshSerialize;

fn account(key: Pubkey, owner: Pubkey, data: Vec<u8>, writable: bool) -> AccountInfo<'static> {
    AccountInfo::new(
        Box::leak(Box::new(key)),
        false,
        writable,
        Box::leak(Box::new(1_000_000)),
        Box::leak(data.into_boxed_slice()),
        Box::leak(Box::new(owner)),
        false,
        0,
    )
}

#[allow(clippy::too_many_arguments)] // Independent checkpoint identity and event fields.
fn checkpoint(
    program: &Pubkey,
    source: Pubkey,
    month: Pubkey,
    previous: Pubkey,
    sequence: u32,
    observed_at: u64,
    accepted_at: u64,
    value: u64,
) -> AccountInfo<'static> {
    let event = Pubkey::new_unique();
    let (key, bump) = checkpoint_address(program, &source, &event);
    let record = Checkpoint {
        header: header::<Checkpoint>(bump),
        source,
        month,
        event,
        previous,
        sequence,
        accepted_at,
        value,
        observed_at,
        evidence_hash: [3; 32],
        archive_hash: [4; 32],
        contributor: Pubkey::new_unique(),
        origin_source: source,
        origin_checkpoint: key,
    };
    account(key, *program, record.try_to_vec().unwrap(), false)
}

fn selection(
    program: &Pubkey,
    parent: Pubkey,
    parent_month: Pubkey,
    head: Pubkey,
    count: u32,
) -> (AccountInfo<'static>, AccountInfo<'static>) {
    let source = Pubkey::new_unique();
    let (key, bump) = address(program, SOURCE_SEED, source.as_ref());
    let carry = CarrySource {
        header: header::<CarrySource>(bump),
        month: Pubkey::new_unique(),
        source,
        parent_month,
        parent_source: parent,
        definition_hash: [1; 32],
        cursor: head,
        selected_checkpoint: Pubkey::default(),
        origin_source: Pubkey::default(),
        origin_checkpoint: Pubkey::default(),
        contributor: Pubkey::default(),
        evidence_hash: [0; 32],
        archive_hash: [0; 32],
        cutoff: 100,
        deadline: 200,
        value: 0,
        observed_at: 0,
        accepted_at: 0,
        remaining: count,
        selected_sequence: 0,
        status: SELECTING,
    };
    (
        account(source, system_program::id(), vec![], false),
        account(key, *program, carry.try_to_vec().unwrap(), true),
    )
}

#[test]
fn critical_selection_is_ordered_cutoff_bound_and_keeps_original_provenance() {
    let program = Pubkey::new_unique();
    let parent = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let first = checkpoint(&program, parent, month, Pubkey::default(), 1, 80, 110, 10);
    let chosen = checkpoint(&program, parent, month, *first.key, 2, 90, 199, 20);
    let too_late = checkpoint(&program, parent, month, *chosen.key, 3, 100, 200, 999);
    let too_new = checkpoint(&program, parent, month, *too_late.key, 4, 101, 201, 888);
    let (source, carry) = selection(&program, parent, month, *too_new.key, 4);
    let before = carry.try_data().unwrap().to_vec();
    assert!(scan_checkpoint(&program, &[source.clone(), carry.clone(), chosen.clone()]).is_err());
    assert_eq!(carry.try_data().unwrap().as_ref(), before.as_slice());
    for next in [&too_new, &too_late, &chosen, &first] {
        let immutable = next.try_data().unwrap().to_vec();
        scan_checkpoint(&program, &[source.clone(), carry.clone(), next.clone()]).unwrap();
        assert_eq!(next.try_data().unwrap().as_ref(), immutable.as_slice());
    }
    let selected = load_carry(&program, source.key, &carry).unwrap();
    assert_eq!(
        (
            selected.remaining,
            selected.value,
            selected.observed_at,
            selected.accepted_at
        ),
        (0, 20, 90, 199)
    );
    assert_eq!(selected.selected_checkpoint, *chosen.key);
    let original = load_checkpoint(&program, &chosen, &parent).unwrap();
    assert_eq!(selected.origin_checkpoint, original.origin_checkpoint);
    assert_eq!(selected.contributor, original.contributor);
    assert_eq!(
        (selected.evidence_hash, selected.archive_hash),
        (original.evidence_hash, original.archive_hash)
    );
    assert!(scan_checkpoint(&program, &[source, carry, first]).is_err()); // No replay or double count.
}

#[test]
fn critical_no_eligible_checkpoint_cannot_invent_an_opening() {
    let program = Pubkey::new_unique();
    let parent = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    for (observed_at, accepted_at, eligible) in
        [(101, 150, false), (100, 200, false), (100, 199, true)]
    {
        let head = checkpoint(
            &program,
            parent,
            month,
            Pubkey::default(),
            1,
            observed_at,
            accepted_at,
            90,
        );
        let (source, carry) = selection(&program, parent, month, *head.key, 1);
        scan_checkpoint(&program, &[source.clone(), carry.clone(), head]).unwrap();
        let result = load_carry(&program, source.key, &carry).unwrap();
        assert_eq!(result.remaining, 0);
        if eligible {
            assert_eq!(result.status, SELECTING); // Eligible, but still requires the explicit freeze.
            assert_ne!(result.selected_checkpoint, Pubkey::default());
            assert_eq!((result.value, result.observed_at), (90, 100));
        } else {
            assert_eq!(result.status, NO_ELIGIBLE_CHECKPOINT);
            assert_eq!(result.selected_checkpoint, Pubkey::default());
            assert_eq!((result.value, result.observed_at), (0, 0));
        }
    }
}

#[test]
fn critical_fresh_claims_and_import_guards_prevent_carry() {
    let program = Pubkey::new_unique();
    let source_key = Pubkey::new_unique();
    let source = OracleSourceState {
        month: Pubkey::new_unique(),
        source_id: [1; 32],
        canonical_locator_hash: [2; 32],
        source_definition_hash: [3; 32],
        ..Default::default()
    };
    let (claim_key, bump) = derive_oracle_opening_claim_pda(&program, &source.month, &source_key);
    for (status, escrow, allowed) in [
        (
            OracleOpeningClaimStatus::Accepted,
            OracleEscrowDisposition::Refunded,
            false,
        ),
        (
            OracleOpeningClaimStatus::Pending,
            OracleEscrowDisposition::Unsettled,
            false,
        ),
        (
            OracleOpeningClaimStatus::Challenged,
            OracleEscrowDisposition::Unsettled,
            false,
        ),
        (
            OracleOpeningClaimStatus::Rejected,
            OracleEscrowDisposition::Unsettled,
            false,
        ),
        (
            OracleOpeningClaimStatus::Rejected,
            OracleEscrowDisposition::Refunded,
            true,
        ),
        (
            OracleOpeningClaimStatus::TimedOut,
            OracleEscrowDisposition::Refunded,
            true,
        ),
    ] {
        let claim = OracleOpeningClaim {
            is_initialized: true,
            bump,
            month: source.month,
            source: source_key,
            source_id: source.source_id,
            canonical_locator_hash: source.canonical_locator_hash,
            source_definition_hash: source.source_definition_hash,
            archive_url_hash: [4; 32],
            evidence_hash: [5; 32],
            status,
            escrow_disposition: escrow,
            ..Default::default()
        };
        let mut data = claim.try_to_vec().unwrap();
        data.resize(OracleOpeningClaim::LEN, 0);
        let info = account(claim_key, program, data, false);
        assert_eq!(
            require_no_fresh_claim(&program, &source_key, &source, &info).is_ok(),
            allowed,
            "{status:?}/{escrow:?}"
        );
    }
    let absent = account(claim_key, system_program::id(), vec![], false);
    require_no_fresh_claim(&program, &source_key, &source, &absent).unwrap();
    let wrong_absence = account(Pubkey::new_unique(), system_program::id(), vec![], false);
    assert!(require_no_fresh_claim(&program, &source_key, &source, &wrong_absence).is_err());
    let (period_key, bump) = address(&program, PERIOD_SEED, source.month.as_ref());
    let mut period = CarryPeriod {
        header: header::<CarryPeriod>(bump),
        month: source.month,
        underlying: [1; 32],
        predecessor: Pubkey::new_unique(),
        predecessor_recipe: [2; 32],
        expiry: 1000,
        registered_at: 10,
        expected_imports: 2,
        next_import: 1,
    };
    let info = account(period_key, program, period.try_to_vec().unwrap(), true);
    assert!(require_import_complete(&program, &source.month, &info).is_err());
    period.next_import = 2;
    save(&program, &info, &period).unwrap();
    require_import_complete(&program, &source.month, &info).unwrap();
}

#[test]
fn critical_invalid_checkpoint_history_leaves_selection_unchanged() {
    let program = Pubkey::new_unique();
    let parent = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    for defect in [
        "owner",
        "source",
        "month",
        "sequence",
        "early_end",
        "extra_link",
    ] {
        let head = checkpoint(&program, parent, month, Pubkey::default(), 1, 90, 150, 42);
        let mut record = load_checkpoint(&program, &head, &parent).unwrap();
        let mut owner = program;
        let mut remaining = 1;
        match defect {
            "owner" => owner = Pubkey::new_unique(),
            "source" => record.source = Pubkey::new_unique(),
            "month" => record.month = Pubkey::new_unique(),
            "sequence" => record.sequence = 2,
            "early_end" => {
                record.sequence = 2;
                remaining = 2;
            }
            "extra_link" => record.previous = Pubkey::new_unique(),
            _ => unreachable!(),
        }
        let bad = account(*head.key, owner, record.try_to_vec().unwrap(), false);
        let (source, carry) = selection(&program, parent, month, *bad.key, remaining);
        let carry_before = carry.try_data().unwrap().to_vec();
        let checkpoint_before = bad.try_data().unwrap().to_vec();
        let balances_before = (source.lamports(), carry.lamports(), bad.lamports());

        assert_eq!(
            scan_checkpoint(&program, &[source.clone(), carry.clone(), bad.clone()]),
            Err(OracleError::InvalidOracleState.into()),
            "{defect}"
        );
        assert_eq!(
            carry.try_data().unwrap().as_ref(),
            carry_before.as_slice(),
            "{defect}"
        );
        assert_eq!(
            bad.try_data().unwrap().as_ref(),
            checkpoint_before.as_slice(),
            "{defect}"
        );
        assert_eq!(
            (source.lamports(), carry.lamports(), bad.lamports()),
            balances_before,
            "{defect}"
        );
    }

    // The same account shape with a complete, authenticated history must advance.
    let head = checkpoint(&program, parent, month, Pubkey::default(), 1, 90, 150, 42);
    let (source, carry) = selection(&program, parent, month, *head.key, 1);
    scan_checkpoint(&program, &[source.clone(), carry.clone(), head.clone()]).unwrap();
    let selected = load_carry(&program, source.key, &carry).unwrap();
    assert_eq!(
        (selected.remaining, selected.cursor),
        (0, Pubkey::default())
    );
    assert_eq!(
        (selected.selected_checkpoint, selected.value),
        (*head.key, 42)
    );
}

#[test]
fn critical_selection_prefers_observation_time_then_sequence_not_acceptance_order() {
    let program = Pubkey::new_unique();
    let parent = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let first = checkpoint(&program, parent, month, Pubkey::default(), 1, 95, 110, 10);
    let tied = checkpoint(&program, parent, month, *first.key, 2, 95, 120, 20);
    // Accepted most recently, but it describes older source evidence.
    let head = checkpoint(&program, parent, month, *tied.key, 3, 80, 190, 999);
    let (source, carry) = selection(&program, parent, month, *head.key, 3);

    for (next, expected_key, expected_value, expected_remaining) in [
        (&head, *head.key, 999, 2),
        (&tied, *tied.key, 20, 1),
        (&first, *tied.key, 20, 0),
    ] {
        scan_checkpoint(&program, &[source.clone(), carry.clone(), next.clone()]).unwrap();
        let selected = load_carry(&program, source.key, &carry).unwrap();
        assert_eq!(selected.selected_checkpoint, expected_key);
        assert_eq!(
            (selected.value, selected.remaining),
            (expected_value, expected_remaining)
        );
    }
    let selected = load_carry(&program, source.key, &carry).unwrap();
    assert_eq!(
        (
            selected.observed_at,
            selected.selected_sequence,
            selected.accepted_at
        ),
        (95, 2, 120)
    );
}

#[test]
fn critical_inherited_anchor_keeps_age_without_becoming_a_median_sample() {
    let mut source = OracleSourceState {
        baseline_state: 1000,
        current_state: 10,
        observation_count: 2,
        latest_source_time: 20,
        rolling_observation_hash: [1; 32],
        ..Default::default()
    };
    let mut observations = OracleSourceObservations {
        account_version: OracleSourceObservations::INHERITED_ANCHOR_VERSION,
        ..OracleSourceObservations::default()
    };
    observations.states[..2].copy_from_slice(&[1000, 10]);
    observations.source_times[..2].copy_from_slice(&[10, 20]);
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1, 30).unwrap(),
        Some(10)
    );
    observations.account_version = OracleSourceObservations::ACCOUNT_VERSION;
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1, 30).unwrap(),
        Some(505)
    );
    observations.account_version = OracleSourceObservations::INHERITED_ANCHOR_VERSION;
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1, 9).unwrap(),
        None
    );
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1, 15).unwrap(),
        Some(1000)
    );
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 30, 40).unwrap(),
        Some(10)
    );
    append_oracle_source_observation(&mut source, &mut observations, 30, 40, &[2; 32], &[3; 32])
        .unwrap();
    source.current_state = 30;
    assert_eq!(source.observation_count, 3);
    assert_eq!(observations.source_times[0], 10);
    assert_eq!(
        oracle_temporal_median_state(&source, &observations, 1, 50).unwrap(),
        Some(20)
    );
    assert!(append_oracle_source_observation(
        &mut source,
        &mut observations,
        30,
        40,
        &[2; 32],
        &[3; 32]
    )
    .is_err());
}
