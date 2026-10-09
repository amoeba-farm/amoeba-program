//! Permissionless native projection preparation. It grants no transfer authority:
//! the financial instruction authenticates unchanged sources and verifies Light.
use crate::compact_error::CompactAccountInfo;
use crate::fixed_codec::{CheckedCursor, CursorField};
use crate::ProgramError;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    hash::{hash, hashv},
    pubkey::Pubkey,
};

pub const SEED: &[u8] = b"atomic-projection-v1";
pub const MAGIC: [u8; 8] = *b"ATMPROJ1";
pub const VERSION: u8 = 1;
pub const PROJECTOR_VERSION: u32 = 1;
pub const COMMON: usize = 4;
pub const HEADER_LEN: usize = 215;
pub const RESERVED: u8 = 0;
pub const SEALED: u8 = 1;
pub const CONSUMED: u8 = 2;

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct Identity {
    pub order: Pubkey,
    pub action_hash: [u8; 32],
    pub source_hash: [u8; 32],
    pub proofs_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum ProjectionAction {
    Fill(crate::atomic_option_route::Fill),
    Close(crate::atomic_option_route::Close),
}

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum Action {
    Reserve {
        identity: Identity,
        proof_count: u32,
        allocation_bytes: u32,
    },
    ProofChunk {
        identity: Identity,
        start: u32,
        proofs: Vec<[u8; 128]>,
    },
    Project {
        action: ProjectionAction,
    },
    Reclaim {
        identity: Identity,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct Header {
    pub magic: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub status: u8,
    pub projector_version: u32,
    pub payer: Pubkey,
    pub identity: Identity,
    pub proof_count: u32,
    pub payload_bytes: u32,
    pub payload_hash: [u8; 32],
}
/// First fields of the sealed native payload. All bounds come from existing
/// trade/oracle/month terms; preparation adds no arbitrary time-to-live.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct TimeBounds {
    pub prepared_slot: u64,
    pub valid_from: u64,
    pub valid_until: u64,
    pub close_deadline: Option<u64>,
    pub month_boundary: Option<u64>,
}
impl TimeBounds {
    pub fn admits(&self, now: u64) -> bool {
        now >= self.valid_from
            && now < self.valid_until
            && self.close_deadline.is_none_or(|end| now <= end)
            && self.month_boundary.is_none_or(|end| now < end)
    }
}
pub fn decode_time_bounds(header: &Header, data: &[u8]) -> Result<TimeBounds, ProgramError> {
    if header.status == RESERVED {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut payload = data
        .get(header.payload_start()?..header.used_bytes()?)
        .ok_or(ProgramError::InvalidAccountData)?;
    crate::fixed_codec::cursor_deserialize(&mut payload)
        .map_err(|_| ProgramError::InvalidAccountData)
}
impl Header {
    pub fn proof_bitmap_bytes(&self) -> Result<usize, ProgramError> {
        usize::try_from(self.proof_count)
            .map_err(|_| ProgramError::InvalidAccountData)?
            .checked_add(7)
            .map(|n| n / 8)
            .ok_or(ProgramError::InvalidAccountData)
    }
    pub fn proofs_start(&self) -> Result<usize, ProgramError> {
        HEADER_LEN
            .checked_add(self.proof_bitmap_bytes()?)
            .ok_or(ProgramError::InvalidAccountData)
    }
    pub fn payload_start(&self) -> Result<usize, ProgramError> {
        self.proofs_start()?
            .checked_add(
                usize::try_from(self.proof_count)
                    .map_err(|_| ProgramError::InvalidAccountData)?
                    .checked_mul(128)
                    .ok_or(ProgramError::InvalidAccountData)?,
            )
            .ok_or(ProgramError::InvalidAccountData)
    }
    pub fn used_bytes(&self) -> Result<usize, ProgramError> {
        self.payload_start()?
            .checked_add(self.payload_bytes as usize)
            .ok_or(ProgramError::InvalidAccountData)
    }
}

// Inspect every unused byte through the runtime memory primitive. A byte loop
// over a grown cache spends SBF compute even when its complete tail is zero.
// Keep a fixed read-only buffer instead of allocating from the transaction heap.
fn zero_padding(data: &[u8]) -> bool {
    static ZEROES: [u8; 1024] = [0; 1024];
    data.chunks(ZEROES.len())
        .all(|chunk| solana_program::program_memory::sol_memcmp(chunk, &ZEROES, chunk.len()) == 0)
}

/// Borrowed transport decoder avoids copying full proof/cache allocations.
pub fn decode_header(program: &Pubkey, key: &Pubkey, data: &[u8]) -> Result<Header, ProgramError> {
    decode_header_inner(program, key, data, false)
}

/// Native caches are created only by selector 27, which establishes the
/// canonical address and immutable stored bump. Proof uploads, growth and
/// sealing preserve those identity fields. Generic compressed-state
/// materialization has no atomic-projection domain or cache write authority.
/// Keep account authentication here so a caller cannot omit it accidentally.
pub(crate) fn decode_program_owned_header(
    program: &Pubkey,
    account: &solana_program::account_info::AccountInfo,
) -> Result<Header, ProgramError> {
    if account.owner != program || account.executable {
        return Err(ProgramError::InvalidAccountData);
    }
    decode_header_inner(program, account.key, &account.try_data()?, true)
}

#[inline(never)]
fn decode_header_inner(
    program: &Pubkey,
    key: &Pubkey,
    data: &[u8],
    program_owned: bool,
) -> Result<Header, ProgramError> {
    if data.len() < HEADER_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let h = crate::fixed_codec::cursor_from_slice::<Header>(&data[..HEADER_LEN])
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let (expected, bump) = if program_owned {
        let expected = Pubkey::create_program_address(
            &[
                crate::constants::CURRENT_STATE_NAMESPACE_SEED,
                SEED,
                h.payer.as_ref(),
                h.identity.order.as_ref(),
                &h.identity.action_hash,
                &h.identity.source_hash,
                &h.identity.proofs_hash,
                &[h.bump],
            ],
            program,
        )
        .map_err(|_| ProgramError::InvalidAccountData)?;
        (expected, h.bump)
    } else {
        derive(program, &h.payer, &h.identity)
    };
    if h.magic != MAGIC
        || h.version != VERSION
        || h.projector_version != PROJECTOR_VERSION
        || h.status > CONSUMED
        || *key != expected
        || h.bump != bump
        || h.status != RESERVED && h.used_bytes()? > data.len()
        || h.status == RESERVED && (h.payload_bytes != 0 || h.payload_hash != [0; 32])
        || !zero_padding(&data[h.used_bytes()?.min(data.len())..])
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let bitmap = &data[HEADER_LEN..h.proofs_start()?.min(data.len())];
    if bitmap.len() == h.proof_bitmap_bytes()?
        && h.proof_count % 8 != 0
        && bitmap
            .last()
            .is_some_and(|b| *b & (!0u8 << (h.proof_count % 8)) != 0)
    {
        return Err(ProgramError::InvalidAccountData);
    }
    if h.status != RESERVED {
        for n in 0..h.proof_count as usize {
            if bitmap[n / 8] & (1 << (n % 8)) == 0 {
                return Err(ProgramError::InvalidAccountData);
            }
        }
        let proofs = &data[h.proofs_start()?..h.payload_start()?];
        if proof_bytes_hash(h.proof_count, proofs)? != h.identity.proofs_hash
            || hash(&data[h.payload_start()?..h.used_bytes()?]).to_bytes() != h.payload_hash
        {
            return Err(ProgramError::InvalidAccountData);
        }
    }
    Ok(h)
}
pub fn proof_bytes_hash(count: u32, bytes: &[u8]) -> Result<[u8; 32], ProgramError> {
    if usize::try_from(count).ok().and_then(|n| n.checked_mul(128)) != Some(bytes.len()) {
        return Err(ProgramError::InvalidAccountData);
    }
    let count = count.to_le_bytes();
    let mut parts = Vec::with_capacity(bytes.len() / 128 + 2);
    parts.push(b"atomic-projection-proofs-v1".as_slice());
    parts.push(count.as_slice());
    parts.extend(bytes.chunks_exact(128));
    Ok(hashv(&parts).to_bytes())
}
pub fn proof_at(header: &Header, data: &[u8], ordinal: usize) -> Result<[u8; 128], ProgramError> {
    if ordinal >= header.proof_count as usize {
        return Err(ProgramError::InvalidAccountData);
    }
    let start = header
        .proofs_start()?
        .checked_add(
            ordinal
                .checked_mul(128)
                .ok_or(ProgramError::InvalidAccountData)?,
        )
        .ok_or(ProgramError::InvalidAccountData)?;
    data.get(start..start + 128)
        .and_then(|v| v.try_into().ok())
        .ok_or(ProgramError::InvalidAccountData)
}
impl CursorField for Identity {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        Self {
            order: c.pubkey(),
            action_hash: c.bytes(),
            source_hash: c.bytes(),
            proofs_hash: c.bytes(),
        }
    }
}
impl CursorField for Header {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        Self {
            magic: c.bytes(),
            version: c.u8(),
            bump: c.u8(),
            status: c.u8(),
            projector_version: c.u32(),
            payer: c.pubkey(),
            identity: Identity::read(c),
            proof_count: c.u32(),
            payload_bytes: c.u32(),
            payload_hash: c.bytes(),
        }
    }
}
impl CursorField for TimeBounds {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        Self {
            prepared_slot: c.u64(),
            valid_from: c.u64(),
            valid_until: c.u64(),
            close_deadline: Option::<u64>::read(c),
            month_boundary: Option::<u64>::read(c),
        }
    }
}
impl CursorField for ProjectionAction {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        match c.u8() {
            0 => Self::Fill(crate::atomic_option_route::Fill::read(c)),
            1 => Self::Close(crate::atomic_option_route::Close::read(c)),
            _ => {
                c.invalid = true;
                Self::Fill(crate::atomic_option_route::Fill {
                    nonce: [0; 32],
                    contexts: 0,
                    legs: vec![],
                    delegate_accounts: 0,
                    custody_accounts: 0,
                    proof_accounts: 0,
                    merkle_accounts: 0,
                    batches: vec![],
                })
            }
        }
    }
}
impl CursorField for Action {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        match c.u8() {
            0 => Self::Reserve {
                identity: Identity::read(c),
                proof_count: c.u32(),
                allocation_bytes: c.u32(),
            },
            1 => Self::ProofChunk {
                identity: Identity::read(c),
                start: c.u32(),
                proofs: Vec::<[u8; 128]>::read(c),
            },
            2 => Self::Project {
                action: ProjectionAction::read(c),
            },
            3 => Self::Reclaim {
                identity: Identity::read(c),
            },
            _ => {
                c.invalid = true;
                Self::Reclaim {
                    identity: Identity::read(c),
                }
            }
        }
    }
}

/// The payer seed prevents an unrelated account creator from fixing the rent
/// refund recipient for another sponsor's deterministic projection account.
pub fn derive(program: &Pubkey, payer: &Pubkey, identity: &Identity) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            payer.as_ref(),
            identity.order.as_ref(),
            &identity.action_hash,
            &identity.source_hash,
            &identity.proofs_hash,
        ],
        program,
    )
}

/// Exact financial batch positions, including zero placeholders for index-only
/// batches. Unlike legacy singleton transport, this always binds cardinality.
pub fn proofs_hash(proofs: &[[u8; 128]]) -> Result<[u8; 32], ProgramError> {
    let count = u32::try_from(proofs.len())
        .map_err(|_| ProgramError::InvalidInstructionData)?
        .to_le_bytes();
    let mut parts = Vec::with_capacity(proofs.len() + 2);
    parts.push(b"atomic-projection-proofs-v1".as_slice());
    parts.push(count.as_slice());
    parts.extend(proofs.iter().map(|p| p.as_slice()));
    Ok(hashv(&parts).to_bytes())
}

/// Preserve proof presence and ordinals while normalizing only inline versus
/// staged transport. Proof bytes themselves are authenticated by proofs_hash.
pub fn action_hash(action: &ProjectionAction) -> Result<[u8; 32], ProgramError> {
    let mut action = action.clone();
    let fill = match &mut action {
        ProjectionAction::Fill(p) => p,
        ProjectionAction::Close(p) => &mut p.route,
    };
    fill.proof_accounts = 0;
    for batch in &mut fill.batches {
        let present = batch.proof.is_some() || batch.proof_account != crate::atomic_proof::INLINE;
        batch.proof = present.then_some([0; 128]);
        batch.proof_account = crate::atomic_proof::INLINE;
    }
    let bytes = borsh::to_vec(&action).map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok(hashv(&[
        b"atomic-projection-action-v1",
        &PROJECTOR_VERSION.to_le_bytes(),
        &bytes,
    ])
    .to_bytes())
}

/// Digest one authenticated economic account preimage. Market callers may
/// supply only its strictly validated prefix+payload; allocated data_len is
/// still bound independently. Dynamic Merkle queues are never source preimages.
pub fn account_hash(
    key: &Pubkey,
    owner: &Pubkey,
    executable: bool,
    data_len: usize,
    data: &[u8],
) -> Result<[u8; 32], ProgramError> {
    let len = u64::try_from(data_len)
        .map_err(|_| ProgramError::InvalidAccountData)?
        .to_le_bytes();
    Ok(hashv(&[
        b"atomic-projection-account-v1",
        key.as_ref(),
        owner.as_ref(),
        &[u8::from(executable)],
        &len,
        data,
    ])
    .to_bytes())
}

pub fn ordered_source_hash(roles: &[(u32, Pubkey, [u8; 32])]) -> Result<[u8; 32], ProgramError> {
    let bytes = borsh::to_vec(roles).map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok(hashv(&[b"atomic-projection-sources-v1", &bytes]).to_bytes())
}

/// Hash-only roles bind destination/authority keys without treating changing
/// wallet lamports or program executable bytes as financial source data.
pub fn key_hash(key: &Pubkey) -> [u8; 32] {
    hash(key.as_ref()).to_bytes()
}

/// Source kinds are selected by the native financial frame, never uploaded by
/// callers. CPI interface balances are checked by settlement and are not quotes.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SourceBinding {
    pub role: u32,
    pub key: Pubkey,
    pub digest: [u8; 32],
    pub kind: u8,
    pub data_len: u32,
}
pub const SOURCE_RAW: u8 = 0;
pub const SOURCE_MARKET: u8 = 1;
pub const SOURCE_PLUMBING: u8 = 2;

pub fn source_bindings(
    program: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    fill: &crate::atomic_option_route::Fill,
) -> Result<Vec<SourceBinding>, ProgramError> {
    let asset_start = crate::atomic_option_route::COMMON
        + usize::from(fill.contexts) * crate::atomic_option_route::CONTEXT_ACCOUNTS;
    let delegate_start = asset_start + 4 * fill.legs.len();
    let proof_start =
        delegate_start + usize::from(fill.delegate_accounts) + usize::from(fill.custody_accounts);
    let expected =
        proof_start + usize::from(fill.proof_accounts) + usize::from(fill.merkle_accounts);
    if accounts.len() != expected || accounts.len() > 255 {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut sources = Vec::with_capacity(proof_start);
    for (role, info) in accounts[..proof_start].iter().enumerate() {
        // Role 4 is the order's quote mint. Its supply moves with every
        // unrelated mint or burn on the network, so it binds by key, owner,
        // executable flag and length only. The order's quote_mint key and the
        // classic SPL owner, initialization and decimals checked by Project
        // are immutable for an existing mint; settlement reads no mint data.
        let kind = if role == 0 || role == 2 || (4..14).contains(&role) {
            SOURCE_PLUMBING
        } else if role >= asset_start
            && role < delegate_start
            && (role - asset_start).is_multiple_of(4)
        {
            SOURCE_MARKET
        } else if role >= asset_start && role < delegate_start && (role - asset_start) % 4 == 3 {
            SOURCE_PLUMBING
        } else {
            SOURCE_RAW
        };
        let data = info.try_data()?;
        let preimage = match kind {
            SOURCE_PLUMBING => &[][..],
            SOURCE_MARKET => {
                if info.owner != program || info.executable {
                    return Err(ProgramError::InvalidAccountData);
                }
                let end = crate::market_router_account::payload_range(&data)?
                    .map_or(crate::state::Market::LEN, |r| r.end);
                &data[..end]
            }
            _ => data.as_ref(),
        };
        sources.push(SourceBinding {
            role: role as u32,
            key: *info.key,
            digest: account_hash(info.key, info.owner, info.executable, data.len(), preimage)?,
            kind,
            data_len: u32::try_from(data.len()).map_err(|_| ProgramError::InvalidAccountData)?,
        });
    }
    Ok(sources)
}
pub fn source_hash(
    program: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    fill: &crate::atomic_option_route::Fill,
) -> Result<[u8; 32], ProgramError> {
    let sources = source_bindings(program, accounts, fill)?;
    ordered_source_hash(
        &sources
            .iter()
            .map(|s| (s.role, s.key, s.digest))
            .collect::<Vec<_>>(),
    )
}

pub(crate) mod delta;
