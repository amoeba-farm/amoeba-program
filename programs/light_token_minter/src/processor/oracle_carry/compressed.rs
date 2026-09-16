//! Local pilot: retain the exact classic carry record bytes inside Light leaves.
//! This module changes storage only; the acceptance and selection rules stay in
//! their original handlers. Domains are appended after observations (11).
use super::*;

pub(in crate::processor) fn validate_carry_body(
    domain: CompressedStateDomain,
    data: &[u8],
) -> ProgramResult {
    match domain {
        CompressedStateDomain::OracleCarryJournal => {
            let value =
                Journal::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
            if !valid_header::<Journal>(&value.header)
                || value.count == 0
                || value.count != value.observation_count
                || value.head == Pubkey::default()
                || value.observation_count == 0
                || value.rolling_observation_hash == [0; 32]
            {
                return invalid();
            }
        }
        CompressedStateDomain::OracleCarryCheckpoint => {
            let value =
                Checkpoint::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
            if !valid_header::<Checkpoint>(&value.header)
                || value.sequence == 0
                || value.value == 0
                || value.observed_at == 0
                || value.accepted_at < value.observed_at
                || value.evidence_hash == [0; 32]
                || value.archive_hash == [0; 32]
                || value.contributor == Pubkey::default()
                || value.origin_source == Pubkey::default()
                || value.origin_checkpoint == Pubkey::default()
                || (value.sequence == 1) != (value.previous == Pubkey::default())
            {
                return invalid();
            }
        }
        _ => return invalid(),
    }
    Ok(())
}

fn valid_header<T: Record>(header: &Header) -> bool {
    header.initialized && header.discriminator == T::DISCRIMINATOR && header.version == T::VERSION
}

pub(in crate::processor) fn validate_carry_payload(
    program: &Pubkey,
    canonical: &Pubkey,
    domain: CompressedStateDomain,
    data: &[u8],
) -> ProgramResult {
    validate_carry_body(domain, data)?;
    let (expected, bump) = match domain {
        CompressedStateDomain::OracleCarryJournal => {
            let value =
                Journal::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
            (
                address(program, JOURNAL_SEED, value.source.as_ref()),
                value.header.bump,
            )
        }
        CompressedStateDomain::OracleCarryCheckpoint => {
            let value =
                Checkpoint::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
            (
                checkpoint_address(program, &value.source, &value.event),
                value.header.bump,
            )
        }
        _ => return invalid(),
    };
    if expected.0 != *canonical || expected.1 != bump {
        return invalid();
    }
    Ok(())
}

pub(in crate::processor) fn materialize_carry<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    domain: CompressedStateDomain,
    data: &[u8],
) -> ProgramResult {
    validate_carry_payload(program, target.key, domain, data)?;
    match domain {
        CompressedStateDomain::OracleCarryJournal => {
            let value =
                Journal::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
            create(
                program,
                payer,
                target,
                system,
                &[JOURNAL_SEED, value.source.as_ref(), &[value.header.bump]],
                &value,
            )
        }
        CompressedStateDomain::OracleCarryCheckpoint => {
            let value =
                Checkpoint::try_from_slice(data).map_err(|_| VaultError::InvalidOracleState)?;
            create(
                program,
                payer,
                target,
                system,
                &[
                    CHECKPOINT_SEED,
                    value.source.as_ref(),
                    value.event.as_ref(),
                    &[value.header.bump],
                ],
                &value,
            )
        }
        _ => invalid(),
    }
}
