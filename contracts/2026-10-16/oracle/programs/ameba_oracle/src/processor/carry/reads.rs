//! Verified off-chain views for the keeper's authenticated history walk.
use super::*;

pub struct JournalView {
    pub head: Pubkey,
    pub count: u32,
    pub rolling_observation_hash: [u8; 32],
}

pub struct CheckpointView {
    pub previous: Pubkey,
    pub sequence: u32,
    pub value: u64,
    pub observed_at: u64,
    pub inherited: bool,
}

fn decode<T: Record>(
    owner: &Pubkey,
    program: &Pubkey,
    key: &Pubkey,
    expected: (Pubkey, u8),
    data: &[u8],
) -> Option<T> {
    if owner != program || key != &expected.0 || data.len() != T::LEN {
        return None;
    }
    let value = T::try_from_slice(data).ok()?;
    let h = value.header();
    (h.initialized
        && h.discriminator == T::DISCRIMINATOR
        && h.version == T::VERSION
        && h.bump == expected.1)
        .then_some(value)
}

pub fn read_source_journal(
    program: &Pubkey,
    owner: &Pubkey,
    key: &Pubkey,
    source: &Pubkey,
    data: &[u8],
) -> Option<JournalView> {
    let j: Journal = decode(
        owner,
        program,
        key,
        address(program, JOURNAL_SEED, source.as_ref()),
        data,
    )?;
    if j.source != *source
        || j.count != j.observation_count
        || j.count == 0
        || j.head == Pubkey::default()
        || j.rolling_observation_hash == [0; 32]
    {
        return None;
    }
    Some(JournalView {
        head: j.head,
        count: j.count,
        rolling_observation_hash: j.rolling_observation_hash,
    })
}

pub fn read_source_checkpoint(
    program: &Pubkey,
    owner: &Pubkey,
    key: &Pubkey,
    source: &Pubkey,
    data: &[u8],
) -> Option<CheckpointView> {
    let candidate = Checkpoint::try_from_slice(data).ok()?;
    let c: Checkpoint = decode(
        owner,
        program,
        key,
        checkpoint_address(program, source, &candidate.event),
        data,
    )?;
    if c.source != *source
        || c.sequence == 0
        || c.value == 0
        || c.observed_at == 0
        || c.accepted_at < c.observed_at
        || c.evidence_hash == [0; 32]
        || c.archive_hash == [0; 32]
        || c.contributor == Pubkey::default()
        || c.origin_source == Pubkey::default()
        || c.origin_checkpoint == Pubkey::default()
        || (c.sequence == 1) != (c.previous == Pubkey::default())
    {
        return None;
    }
    Some(CheckpointView {
        previous: c.previous,
        sequence: c.sequence,
        value: c.value,
        observed_at: c.observed_at,
        inherited: c.sequence == 1 && (c.origin_source != *source || c.origin_checkpoint != *key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_reader_rejects_foreign_owner_wrong_pda_and_partial_snapshot() {
        let program = Pubkey::new_unique();
        let source = Pubkey::new_unique();
        let (key, bump) = address(&program, JOURNAL_SEED, source.as_ref());
        let mut j = Journal {
            header: header::<Journal>(bump),
            source,
            head: Pubkey::new_unique(),
            rolling_observation_hash: [1; 32],
            count: 37,
            observation_count: 37,
        };
        let bytes = borsh::to_vec(&j).unwrap();
        assert_eq!(
            read_source_journal(&program, &program, &key, &source, &bytes)
                .unwrap()
                .count,
            37
        );
        assert!(
            read_source_journal(&program, &Pubkey::new_unique(), &key, &source, &bytes).is_none()
        );
        assert!(
            read_source_journal(&program, &program, &Pubkey::new_unique(), &source, &bytes)
                .is_none()
        );
        assert!(
            read_source_journal(&program, &program, &key, &source, &bytes[..bytes.len() - 1])
                .is_none()
        );
        j.observation_count -= 1;
        assert!(read_source_journal(
            &program,
            &program,
            &key,
            &source,
            &borsh::to_vec(&j).unwrap()
        )
        .is_none());
    }
}
