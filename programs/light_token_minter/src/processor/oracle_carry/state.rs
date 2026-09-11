//! Prospective companion records. No existing funded account layout is reinterpreted.
use super::*;
use borsh::{BorshDeserialize, BorshSerialize};

pub(super) const REGISTRY_SEED: &[u8] = b"oracle-carry-registry";
pub(super) const PERIOD_SEED: &[u8] = b"oracle-carry-period";
pub(super) const SOURCE_SEED: &[u8] = b"oracle-carry-source";
pub(super) const JOURNAL_SEED: &[u8] = b"oracle-knowledge";
pub(super) const CHECKPOINT_SEED: &[u8] = b"oracle-checkpoint";
pub(super) const VERSION: u8 = 1;

#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub(super) struct Header {
    discriminator: [u8; 3],
    version: u8,
    initialized: bool,
    bump: u8,
}

pub(super) trait Record: BorshDeserialize + BorshSerialize {
    const DISCRIMINATOR: [u8; 3];
    const LEN: usize;
    fn header(&self) -> &Header;
}

macro_rules! record {
    ($name:ident, $disc:literal, $len:expr, {$($field:ident: $ty:ty),* $(,)?}) => {
        #[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
        pub(super) struct $name {
            pub header: Header,
            $(pub $field: $ty,)*
        }
        impl Record for $name {
            const DISCRIMINATOR: [u8; 3] = *$disc;
            const LEN: usize = 6 + $len;
            fn header(&self) -> &Header { &self.header }
        }
    };
}

record!(Registry, b"OCR", 32 + 32 + 8, {
    underlying: [u8; 32], latest_period: Pubkey, latest_expiry: u64,
});
record!(Period, b"OCP", 32 * 4 + 8 * 2 + 2 * 2, {
    month: Pubkey, underlying: [u8; 32], predecessor: Pubkey,
    predecessor_recipe: [u8; 32], expiry: u64, registered_at: u64,
    expected_imports: u16, next_import: u16,
});
record!(Journal, b"OKJ", 32 * 3 + 4 + 1, {
    source: Pubkey, head: Pubkey, rolling_observation_hash: [u8; 32],
    count: u32, observation_count: u8,
});
record!(Checkpoint, b"OKC", 32 * 9 + 8 * 3 + 4, {
    source: Pubkey, month: Pubkey, event: Pubkey, previous: Pubkey,
    sequence: u32, accepted_at: u64, value: u64, observed_at: u64,
    evidence_hash: [u8; 32], archive_hash: [u8; 32], contributor: Pubkey,
    origin_source: Pubkey, origin_checkpoint: Pubkey,
});

pub(super) const IMPORTED: u8 = 0;
pub(super) const SELECTING: u8 = 1;
pub(super) const NO_ELIGIBLE_CHECKPOINT: u8 = 2;
pub(super) const FROZEN_CARRY: u8 = 3;
pub(super) const FRESH_OPENING: u8 = 4;

record!(CarrySource, b"OCS", 32 * 12 + 8 * 5 + 4 * 2 + 1, {
    month: Pubkey, source: Pubkey, parent_month: Pubkey, parent_source: Pubkey,
    definition_hash: [u8; 32], cursor: Pubkey, selected_checkpoint: Pubkey,
    origin_source: Pubkey, origin_checkpoint: Pubkey, contributor: Pubkey,
    evidence_hash: [u8; 32], archive_hash: [u8; 32],
    cutoff: u64, deadline: u64, value: u64, observed_at: u64, accepted_at: u64,
    remaining: u32, selected_sequence: u32, status: u8,
});

pub(super) fn address(program: &Pubkey, seed: &[u8], key: &[u8]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CURRENT_STATE_NAMESPACE_SEED, seed, key], program)
}

pub(super) fn checkpoint_address(
    program: &Pubkey,
    source: &Pubkey,
    event: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            CHECKPOINT_SEED,
            source.as_ref(),
            event.as_ref(),
        ],
        program,
    )
}

pub(super) fn header<T: Record>(bump: u8) -> Header {
    Header {
        discriminator: T::DISCRIMINATOR,
        version: VERSION,
        initialized: true,
        bump,
    }
}

pub(super) fn load<T: Record>(
    program: &Pubkey,
    info: &AccountInfo,
    expected: (Pubkey, u8),
) -> Result<T, ProgramError> {
    if info.owner != program
        || info.executable
        || info.is_signer
        || info.key != &expected.0
        || info.data_len() != T::LEN
    {
        return invalid();
    }
    let value =
        T::try_from_slice(&info.try_borrow_data()?).map_err(|_| VaultError::InvalidOracleState)?;
    let h = value.header();
    if !h.initialized
        || h.discriminator != T::DISCRIMINATOR
        || h.version != VERSION
        || h.bump != expected.1
    {
        return invalid();
    }
    Ok(value)
}

pub(super) fn save<T: Record>(program: &Pubkey, info: &AccountInfo, value: &T) -> ProgramResult {
    if !info.is_writable || info.owner != program || info.executable || info.data_len() != T::LEN {
        return invalid();
    }
    let mut data = info.try_borrow_mut_data()?;
    let mut output = &mut data[..];
    value
        .serialize(&mut output)
        .map_err(|_| VaultError::InvalidOracleState)?;
    if !output.is_empty() {
        return invalid();
    }
    Ok(())
}

pub(super) fn create<'a, T: Record>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    seeds: &[&[u8]],
    value: &T,
) -> ProgramResult {
    if !payer.is_signer || !payer.is_writable || info.is_signer {
        return invalid();
    }
    // Check the full namespace before invoking System, including the supplied stored bump.
    let mut namespaced = vec![CURRENT_STATE_NAMESPACE_SEED];
    namespaced.extend_from_slice(seeds);
    let expected =
        Pubkey::create_program_address(&namespaced, program).map_err(|_| VaultError::InvalidPda)?;
    if info.key != &expected {
        return invalid();
    }
    validate_create_only_program_account_target(program, info)?;
    create_program_account(payer, info, system, program, T::LEN, seeds)?;
    save(program, info, value)
}

pub(super) fn now() -> Result<u64, ProgramError> {
    u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| VaultError::InvalidOracleState.into())
}

pub(super) fn invalid<T>() -> Result<T, ProgramError> {
    Err(VaultError::InvalidOracleState.into())
}

pub(super) fn load_period(
    program: &Pubkey,
    month: &Pubkey,
    info: &AccountInfo,
) -> Result<Period, ProgramError> {
    let period: Period = load(program, info, address(program, PERIOD_SEED, month.as_ref()))?;
    if period.month != *month || period.next_import > period.expected_imports {
        return invalid();
    }
    Ok(period)
}

pub(super) fn load_carry(
    program: &Pubkey,
    source: &Pubkey,
    info: &AccountInfo,
) -> Result<CarrySource, ProgramError> {
    let carry: CarrySource = load(
        program,
        info,
        address(program, SOURCE_SEED, source.as_ref()),
    )?;
    if carry.source != *source || carry.status > FRESH_OPENING {
        return invalid();
    }
    Ok(carry)
}

pub(super) fn load_journal(
    program: &Pubkey,
    source: &Pubkey,
    info: &AccountInfo,
) -> Result<Journal, ProgramError> {
    let journal: Journal = load(
        program,
        info,
        address(program, JOURNAL_SEED, source.as_ref()),
    )?;
    if journal.source != *source
        || journal.count == 0
        || journal.count > u32::from(journal.observation_count)
        || journal.head == Pubkey::default()
        || journal.observation_count == 0
        || journal.rolling_observation_hash == [0; 32]
    {
        return invalid();
    }
    Ok(journal)
}
