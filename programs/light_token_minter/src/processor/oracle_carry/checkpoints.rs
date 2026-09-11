use super::*;

/// Internal acceptance output, never deserialized from instruction bytes. The caller
/// is the existing authenticated claim/emergency-resolution handler after its decision.
pub(in crate::processor) struct AcceptedEvent {
    pub event: Pubkey,
    pub value: u64,
    pub observed_at: u64,
    pub evidence_hash: [u8; 32],
    pub archive_hash: [u8; 32],
    pub contributor: Pubkey,
}

/// Mandatory atomic hook for ordinary opening/update, accepted challenge alternatives,
/// sAMBA emergency acceptance and the feature-gated Devnet opening acceptance.
/// Tail accounts: source journal, create-only event checkpoint, System program.
/// A failure must propagate through the accepting transaction, never be swallowed.
pub(in crate::processor) fn record_fresh_accept<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    source_key: &Pubkey,
    before: &OracleSourceState,
    after: &OracleSourceState,
    observations: &OracleSourceObservations,
    event: AcceptedEvent,
    tail: &[AccountInfo<'a>],
) -> ProgramResult {
    if after.month != before.month
        || after.source_id != before.source_id
        || after.observation_count
            != before
                .observation_count
                .checked_add(1)
                .ok_or(VaultError::ArithmeticOverflow)?
        || after.rolling_observation_hash
            != hashv(&[
                crate::constants::ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
                &before.rolling_observation_hash,
                after.month.as_ref(),
                &after.source_id,
                &[after.observation_count],
                &event.value.to_le_bytes(),
                &event.observed_at.to_le_bytes(),
                &event.evidence_hash,
                &event.archive_hash,
            ])
            .to_bytes()
    {
        return invalid();
    }
    append_checkpoint(
        program,
        payer,
        source_key,
        after,
        observations,
        event,
        tail,
        Some((before.observation_count, before.rolling_observation_hash)),
        None,
    )
}

pub(super) fn append_checkpoint<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    source_key: &Pubkey,
    source: &OracleSourceState,
    observations: &OracleSourceObservations,
    event: AcceptedEvent,
    tail: &[AccountInfo<'a>],
    previous_state: Option<(u8, [u8; 32])>,
    origin: Option<(Pubkey, Pubkey)>,
) -> ProgramResult {
    if tail.len() != 3
        || source.status != OracleSourceStatus::Active
        || derive_oracle_source_pda(program, &source.month, &source.source_id).0 != *source_key
        || observations.month != source.month
        || observations.source != *source_key
        || event.event == Pubkey::default()
        || event.value == 0
        || event.observed_at == 0
        || event.evidence_hash == [0; 32]
        || event.archive_hash == [0; 32]
        || event.contributor == Pubkey::default()
    {
        return invalid();
    }
    validate_oracle_observation_shape(source, observations)?;
    let last = usize::from(source.observation_count)
        .checked_sub(1)
        .ok_or(VaultError::InvalidOracleObservation)?;
    let timestamp = now()?;
    if observations.states[last] != event.value
        || observations.source_times[last] != event.observed_at
        || event.observed_at > timestamp
    {
        return invalid();
    }
    let journal_pda = address(program, JOURNAL_SEED, source_key.as_ref());
    let new_journal = tail[0].owner == &system_program::id();
    let (previous, count) = if new_journal {
        validate_canonical_system_zero_pda_proof(&journal_pda.0, &tail[0])?;
        (Pubkey::default(), 0)
    } else {
        let journal = load_journal(program, source_key, &tail[0])?;
        // No skipped acceptance is repairable by silently jumping the journal forward.
        if Some((journal.observation_count, journal.rolling_observation_hash)) != previous_state {
            return invalid();
        }
        (journal.head, journal.count)
    };
    let record_pda = checkpoint_address(program, source_key, &event.event);
    let (origin_source, origin_checkpoint) = origin.unwrap_or((*source_key, record_pda.0));
    if origin_source == Pubkey::default() || origin_checkpoint == Pubkey::default() {
        return invalid();
    }
    let sequence = count.checked_add(1).ok_or(VaultError::ArithmeticOverflow)?;
    let checkpoint = Checkpoint {
        header: header::<Checkpoint>(record_pda.1),
        source: *source_key,
        month: source.month,
        event: event.event,
        previous,
        sequence,
        accepted_at: timestamp,
        value: event.value,
        observed_at: event.observed_at,
        evidence_hash: event.evidence_hash,
        archive_hash: event.archive_hash,
        contributor: event.contributor,
        origin_source,
        origin_checkpoint,
    };
    create(
        program,
        payer,
        &tail[1],
        &tail[2],
        &[
            CHECKPOINT_SEED,
            source_key.as_ref(),
            event.event.as_ref(),
            &[record_pda.1],
        ],
        &checkpoint,
    )?;
    let journal = Journal {
        header: header::<Journal>(journal_pda.1),
        source: *source_key,
        head: record_pda.0,
        rolling_observation_hash: source.rolling_observation_hash,
        count: sequence,
        observation_count: source.observation_count,
    };
    if new_journal {
        create(
            program,
            payer,
            &tail[0],
            &tail[2],
            &[JOURNAL_SEED, source_key.as_ref(), &[journal_pda.1]],
            &journal,
        )
    } else {
        save(program, &tail[0], &journal)
    }
}

/// Safe enrollment of an existing accepted opening/update: source/observations and
/// the accepted claim are authenticated. Its checkpoint finality begins NOW, never
/// at an invented historical acceptance time. An existing journal cannot be reset.
/// Accounts: payer, market, month, source, observations, accepted claim/challenge,
/// journal, checkpoint, System; accepted alternatives append their rejected claim.
/// Payload: kind (0 opening / 1 update / 2 accepted alternative), prior rolling hash.
pub(super) fn capture_current(
    program: &Pubkey,
    a: &[AccountInfo],
    kind: u8,
    previous_hash: [u8; 32],
) -> ProgramResult {
    if a.len() != (if kind == 2 { 10 } else { 9 }) {
        return invalid();
    }
    let (_, month) = load_valid_market_and_oracle_month(program, &a[1], &a[2])?;
    if !matches!(
        month.phase,
        OraclePhase::Game | OraclePhase::Settled | OraclePhase::Closed
    ) {
        return invalid();
    }
    let source = load_valid_oracle_source(program, a[2].key, &a[3])?;
    let observations = load_valid_oracle_source_observations(program, a[2].key, a[3].key, &a[4])?;
    validate_canonical_system_zero_pda_proof(
        &address(program, JOURNAL_SEED, a[3].key.as_ref()).0,
        &a[6],
    )?;
    let event = match kind {
        0 => {
            let claim =
                load_valid_oracle_opening_claim(program, a[2].key, a[3].key, &a[5], &source)?;
            if claim.status != OracleOpeningClaimStatus::Accepted
                || source.observation_count != 1
                || previous_hash != [0; 32]
            {
                return invalid();
            }
            AcceptedEvent {
                event: *a[5].key,
                value: claim.opening_state,
                observed_at: claim.source_time,
                evidence_hash: claim.evidence_hash,
                archive_hash: claim.archive_url_hash,
                contributor: claim.claimant,
            }
        }
        1 => {
            let claim =
                load_valid_oracle_update_claim_v2_from_account(program, a[2].key, a[3].key, &a[5])?;
            if claim.claim.status != OracleClaimStatus::Finalized || claim.samba_checkpoint_active {
                return invalid();
            }
            AcceptedEvent {
                event: *a[5].key,
                value: claim.claim.new_state,
                observed_at: claim.claim.source_time,
                evidence_hash: claim.claim.evidence_hash,
                archive_hash: claim.claim.archive_url_hash,
                contributor: claim.claim.claimant,
            }
        }
        2 => {
            let challenge = load_valid_oracle_update_challenge(program, a[2].key, &a[5])?;
            let claim =
                load_valid_oracle_update_claim_v2_from_account(program, a[2].key, a[3].key, &a[9])?;
            if challenge.status != OracleChallengeStatus::Accepted
                || challenge.claim != *a[9].key
                || challenge.claim_id != claim.claim.claim_id
                || claim.claim.status != OracleClaimStatus::Rejected
                || claim.samba_checkpoint_active
            {
                return invalid();
            }
            AcceptedEvent {
                event: *a[5].key,
                value: challenge.alternative_state,
                observed_at: challenge.alternative_source_time,
                evidence_hash: challenge.evidence_hash,
                archive_hash: challenge.archive_url_hash,
                contributor: challenge.challenger,
            }
        }
        _ => return invalid(),
    };
    if source.rolling_observation_hash
        != hashv(&[
            crate::constants::ORACLE_UPDATE_EVIDENCE_HASH_DOMAIN,
            &previous_hash,
            source.month.as_ref(),
            &source.source_id,
            &[source.observation_count],
            &event.value.to_le_bytes(),
            &event.observed_at.to_le_bytes(),
            &event.evidence_hash,
            &event.archive_hash,
        ])
        .to_bytes()
    {
        return invalid();
    }
    append_checkpoint(
        program,
        &a[0],
        a[3].key,
        &source,
        &observations,
        event,
        &a[6..9],
        None,
        None,
    )
}

pub(super) fn load_checkpoint(
    program: &Pubkey,
    info: &AccountInfo,
    source: &Pubkey,
) -> Result<Checkpoint, ProgramError> {
    // Decode the fixed header once to recover its immutable event seed, then fully validate.
    if info.owner != program || info.data_len() != Checkpoint::LEN || info.executable {
        return invalid();
    }
    let raw = <Checkpoint as borsh::BorshDeserialize>::try_from_slice(&info.try_borrow_data()?)
        .map_err(|_| VaultError::InvalidOracleState)?;
    let record: Checkpoint = load(
        program,
        info,
        checkpoint_address(program, source, &raw.event),
    )?;
    if record.source != *source
        || record.sequence == 0
        || record.value == 0
        || record.observed_at == 0
        || record.accepted_at < record.observed_at
        || record.evidence_hash == [0; 32]
        || record.archive_hash == [0; 32]
        || record.contributor == Pubkey::default()
        || record.origin_source == Pubkey::default()
        || record.origin_checkpoint == Pubkey::default()
        || (record.sequence == 1) != (record.previous == Pubkey::default())
    {
        return invalid();
    }
    Ok(record)
}
