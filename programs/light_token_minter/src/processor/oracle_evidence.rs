//! Immutable public preimages and permanent publication receipts. No economic rights.
use super::*;
use solana_program::hash::hashv;

const BYTES_SEED: &[u8] = b"g3-evidence-bytes-v1";
const LINK_SEED: &[u8] = b"g3-evidence-link-v1";
const HEADER: usize = 76;
pub(super) const CHUNK: usize = 192;
pub(super) const DEFINITION_MAX: usize = 4096;
const LINK_LEN: usize = 251;

fn invalid<T>() -> Result<T, ProgramError> {
    Err(VaultError::InvalidOracleOpeningEvidence.into())
}
fn bytes_address(program: &Pubkey, payer: &Pubkey, kind: u8, hash: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            BYTES_SEED,
            payer.as_ref(),
            &[kind],
            hash,
        ],
        program,
    )
}
fn link_address(program: &Pubkey, event: &Pubkey, role: u8, hash: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            LINK_SEED,
            event.as_ref(),
            &[role],
            hash,
        ],
        program,
    )
}
fn max_bytes(kind: u8) -> Result<usize, ProgramError> {
    match kind {
        1 | 3 => Ok(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES),
        2 => Ok(DEFINITION_MAX),
        _ => invalid(),
    }
}
fn read_key(data: &[u8]) -> Pubkey {
    Pubkey::new_from_array(data.try_into().expect("validated key slice"))
}
fn validate_object(program: &Pubkey, info: &AccountInfo) -> Result<(), ProgramError> {
    if info.owner != program || info.executable || info.is_signer || info.data_len() < HEADER {
        return invalid();
    }
    let d = info.try_borrow_data()?;
    let payer = read_key(&d[6..38]);
    let kind = d[38];
    let hash: &[u8; 32] = d[39..71]
        .try_into()
        .map_err(|_| VaultError::InvalidOracleOpeningEvidence)?;
    let total = usize::from(u16::from_le_bytes([d[71], d[72]]));
    let written = usize::from(u16::from_le_bytes([d[73], d[74]]));
    let (key, bump) = bytes_address(program, &payer, kind, hash);
    if d[..5] != [b'O', b'E', b'B', 1, 1]
        || d[5] != bump
        || *info.key != key
        || payer == Pubkey::default()
        || *hash == [0; 32]
        || total == 0
        || total > max_bytes(kind)?
        || d.len() != HEADER + total
        || written > total
        || d[75] > 1
        || (d[75] == 1) != (written == total)
        || d[HEADER + written..].iter().any(|b| *b != 0)
    {
        return invalid();
    }
    Ok(())
}
/// Public locations must not contain URL userinfo or common embedded credentials.
/// Exact bytes are retained; validation never normalizes a committed URL.
pub(super) fn public_locator(url: &str) -> ProgramResult {
    if url.is_empty()
        || url.len() > ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES
        || url.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\')
    {
        return invalid();
    }
    let tail = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or(VaultError::InvalidOracleOpeningEvidence)?;
    let authority = tail.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.starts_with(':') {
        return invalid();
    }
    // Inspect percent-encoded query names without changing the committed bytes.
    let mut scanned = Vec::with_capacity(url.len());
    let mut i = 0;
    let input = url.as_bytes();
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            let high = (input[i + 1] as char).to_digit(16);
            let low = (input[i + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                let byte = (high * 16 + low) as u8;
                if byte <= b' ' || byte == 0x7f || byte == b'\\' {
                    return invalid();
                }
                scanned.push(byte.to_ascii_lowercase());
                i += 3;
                continue;
            }
        }
        scanned.push(input[i].to_ascii_lowercase());
        i += 1;
    }
    let lower = String::from_utf8_lossy(&scanned);
    if [
        "access_token=",
        "api_key=",
        "apikey=",
        "authorization=",
        "password=",
        "secret=",
        "x-amz-signature=",
        "token=",
    ]
    .iter()
    .any(|key| lower.contains(key))
    {
        return invalid();
    }
    Ok(())
}
fn committed_hash(kind: u8, bytes: &[u8]) -> Result<[u8; 32], ProgramError> {
    match kind {
        1 => {
            let text =
                std::str::from_utf8(bytes).map_err(|_| VaultError::InvalidOracleOpeningEvidence)?;
            public_locator(text)?;
            Ok(hashv(&[b"locator", bytes]).to_bytes())
        }
        // Full SHA-256 input transcript, including any structured serialization/domain.
        // This does not reinterpret an opaque definition commitment as H(display text).
        2 => Ok(hashv(&[bytes]).to_bytes()),
        3 => {
            let text =
                std::str::from_utf8(bytes).map_err(|_| VaultError::InvalidOracleOpeningEvidence)?;
            let (_, target) = parse_oracle_opening_archive_url(text)?;
            public_locator(target)?;
            Ok(derive_oracle_opening_archive_url_hash(text))
        }
        _ => invalid(),
    }
}
/// Current G3 publication transports the URL only through its sealed object.
/// Reject inline URL bytes: there is one current payload interpretation.
pub(super) fn publication_url(
    program: &Pubkey,
    object: &AccountInfo,
    wire_url: &str,
) -> Result<String, ProgramError> {
    if !wire_url.is_empty() {
        return invalid();
    }
    validate_object(program, object)?;
    let d = object.try_borrow_data()?;
    if d[38] != 3 || d[75] != 1 || committed_hash(3, &d[HEADER..])? != d[39..71] {
        return invalid();
    }
    Ok(std::str::from_utf8(&d[HEADER..])
        .map_err(|_| VaultError::InvalidOracleOpeningEvidence)?
        .to_owned())
}
#[allow(clippy::too_many_arguments)]
pub(super) fn write(
    program: &Pubkey,
    a: &[AccountInfo],
    kind: u8,
    hash: [u8; 32],
    total: u16,
    offset: u16,
    bytes: &[u8],
) -> ProgramResult {
    if a.len() != 3
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_writable
        || a[1].is_signer
        || bytes.is_empty()
        || bytes.len() > CHUNK
        || total == 0
        || usize::from(total) > max_bytes(kind)?
        || hash == [0; 32]
    {
        return invalid();
    }
    validate_system_program(&a[2])?;
    let (key, bump) = bytes_address(program, a[0].key, kind, &hash);
    if *a[1].key != key {
        return invalid();
    }
    let start = usize::from(offset);
    let end = start
        .checked_add(bytes.len())
        .ok_or(VaultError::ArithmeticOverflow)?;
    if end > usize::from(total) {
        return invalid();
    }
    if a[1].owner != program {
        if offset != 0 {
            return invalid();
        }
        validate_create_only_program_account_target(program, &a[1])?;
        create_program_account(
            &a[0],
            &a[1],
            &a[2],
            program,
            HEADER + usize::from(total),
            &[BYTES_SEED, a[0].key.as_ref(), &[kind], &hash, &[bump]],
        )?;
        let mut d = a[1].try_borrow_mut_data()?;
        d[..6].copy_from_slice(&[b'O', b'E', b'B', 1, 1, bump]);
        d[6..38].copy_from_slice(a[0].key.as_ref());
        d[38] = kind;
        d[39..71].copy_from_slice(&hash);
        d[71..73].copy_from_slice(&total.to_le_bytes());
    }
    validate_object(program, &a[1])?;
    let mut d = a[1].try_borrow_mut_data()?;
    if d[6..38] != a[0].key.to_bytes()
        || d[38] != kind
        || d[39..71] != hash
        || d[71..73] != total.to_le_bytes()
    {
        return invalid();
    }
    let written = usize::from(u16::from_le_bytes([d[73], d[74]]));
    if start < written {
        return if end <= written && d[HEADER + start..HEADER + end] == *bytes {
            Ok(())
        } else {
            invalid()
        };
    }
    if start != written || d[75] != 0 {
        return invalid();
    }
    // Validate complete candidate before altering even the local account view.
    if end == usize::from(total) {
        let mut candidate = d[HEADER..HEADER + start].to_vec();
        candidate.extend_from_slice(bytes);
        if committed_hash(kind, &candidate)? != hash {
            return invalid();
        }
    }
    d[HEADER + start..HEADER + end].copy_from_slice(bytes);
    d[73..75].copy_from_slice(&(end as u16).to_le_bytes());
    d[75] = u8::from(end == usize::from(total));
    Ok(())
}
pub(super) fn close_draft(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 2 || !a[0].is_signer {
        return invalid();
    }
    validate_object(program, &a[1])?;
    {
        let d = a[1].try_borrow_data()?;
        if d[75] != 0 || d[6..38] != a[0].key.to_bytes() {
            return invalid();
        }
    }
    close_program_account(program, &a[1], &a[0])
}
#[allow(clippy::too_many_arguments)]
pub(super) fn publish<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    object: &AccountInfo<'a>,
    link: &AccountInfo<'a>,
    month: &Pubkey,
    source: &Pubkey,
    event: &Pubkey,
    parent: &Pubkey,
    role: u8,
    attempt: u32,
    value: u64,
    time: u64,
    evidence_hash: [u8; 32],
    commitment: [u8; 32],
    source_id: [u8; 32],
) -> ProgramResult {
    if !payer.is_signer || !payer.is_writable || !link.is_writable || link.is_signer || role > 5 {
        return invalid();
    }
    validate_object(program, object)?;
    let kind = if role < 2 { role + 1 } else { 3 };
    {
        let d = object.try_borrow_data()?;
        if d[75] != 1
            || d[38] != kind
            || d[39..71] != commitment
            || committed_hash(kind, &d[HEADER..])? != commitment
        {
            return invalid();
        }
    }
    let (key, bump) = link_address(program, event, role, &evidence_hash);
    if *link.key != key {
        return invalid();
    }
    let mut bytes = Vec::with_capacity(LINK_LEN);
    bytes.extend_from_slice(&[b'O', b'E', b'L', 1, 1, bump]);
    for key in [month, source, event, object.key, parent] {
        bytes.extend_from_slice(key.as_ref());
    }
    bytes.push(role);
    bytes.extend_from_slice(&attempt.to_le_bytes());
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes.extend_from_slice(&time.to_le_bytes());
    bytes.extend_from_slice(&evidence_hash);
    bytes.extend_from_slice(&source_id);
    if link.owner == program {
        let d = link.try_borrow_data()?;
        if link.executable
            || d.len() != LINK_LEN
            || d[..102] != bytes[..102]
            || d[134..167] != bytes[134..167]
            || d[171..] != bytes[171..]
        {
            return invalid();
        }
        // The original object is permanent. Another publisher may provide an
        // independently sealed copy of the exact same commitment without
        // replacing this pointer or blocking a valid opening retry.
        return Ok(());
    }
    validate_create_only_program_account_target(program, link)?;
    create_program_account(
        payer,
        link,
        system,
        program,
        LINK_LEN,
        &[LINK_SEED, event.as_ref(), &[role], &evidence_hash, &[bump]],
    )?;
    link.try_borrow_mut_data()?.copy_from_slice(&bytes);
    Ok(())
}

/// Permissionless exact-preimage recovery only. This changes no source/claim,
/// timestamp, observation, economic right or verdict, and cannot read hidden commits.
pub(super) fn backfill_source(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 8 {
        return invalid();
    }
    let source = load_valid_oracle_source(program, a[1].key, &a[2])?;
    for (role, commitment, object, link) in [
        (0, source.canonical_locator_hash, 3, 4),
        (1, source.source_definition_hash, 5, 6),
    ] {
        publish(
            program,
            &a[0],
            &a[7],
            &a[object],
            &a[link],
            a[1].key,
            a[2].key,
            a[2].key,
            a[2].key,
            role,
            0,
            0,
            0,
            commitment,
            commitment,
            source.source_id,
        )?;
    }
    Ok(())
}
pub(super) fn backfill_claim(program: &Pubkey, a: &[AccountInfo], role: u8) -> ProgramResult {
    if a.len() != 8 {
        return invalid();
    }
    let source = load_valid_oracle_source(program, a[1].key, &a[2])?;
    let (attempt, value, time, evidence, archive) = match role {
        2 => {
            let c = load_valid_oracle_opening_claim(program, a[1].key, a[2].key, &a[3], &source)?;
            if a[3].key != a[4].key {
                return invalid();
            }
            (
                c.attempt,
                c.opening_state,
                c.source_time,
                c.evidence_hash,
                c.archive_url_hash,
            )
        }
        3 => {
            let c = load_valid_oracle_opening_challenge(program, a[1].key, &a[3])?;
            if c.source != *a[2].key
                || c.source_id != source.source_id
                || c.claim != *a[4].key
                || c.canonical_locator_hash != source.canonical_locator_hash
                || c.source_definition_hash != source.source_definition_hash
            {
                return invalid();
            }
            (
                c.claim_attempt,
                c.alternative_opening_state,
                c.alternative_source_time,
                c.evidence_hash,
                c.archive_url_hash,
            )
        }
        4 => {
            let c =
                load_valid_oracle_update_claim_v2_from_account(program, a[1].key, a[2].key, &a[3])?;
            if a[3].key != a[4].key
                || c.claim.status == OracleClaimStatus::Committed
                || c.revealed_at_ts == 0
            {
                return invalid();
            }
            (
                0,
                c.claim.new_state,
                c.claim.source_time,
                c.claim.evidence_hash,
                c.claim.archive_url_hash,
            )
        }
        5 => {
            let c = load_valid_oracle_update_challenge(program, a[1].key, &a[3])?;
            let parent =
                load_valid_oracle_update_claim_v2_from_account(program, a[1].key, a[2].key, &a[4])?;
            if c.claim != *a[4].key || c.claim_id != parent.claim.claim_id {
                return invalid();
            }
            (
                0,
                c.alternative_state,
                c.alternative_source_time,
                c.evidence_hash,
                c.archive_url_hash,
            )
        }
        _ => return invalid(),
    };
    if value == 0 || time == 0 || evidence == [0; 32] || archive == [0; 32] {
        return invalid();
    }
    // Authenticate the exact archived source/value/time formula, not just the URL digest.
    validate_object(program, &a[5])?;
    {
        let d = a[5].try_borrow_data()?;
        let url = std::str::from_utf8(&d[HEADER..])
            .map_err(|_| VaultError::InvalidOracleOpeningEvidence)?;
        validate_oracle_opening_archive_binding(url, time, &source.canonical_locator_hash)?;
        let expected = if role < 4 {
            derive_oracle_opening_evidence_hash(
                a[1].key,
                a[2].key,
                value,
                time,
                &source.canonical_locator_hash,
                &source.source_definition_hash,
                url,
            )
        } else {
            derive_oracle_update_evidence_hash(a[1].key, a[2].key, &source, value, time, url)
        };
        if expected != evidence {
            return invalid();
        }
    }
    publish(
        program,
        &a[0],
        &a[7],
        &a[5],
        &a[6],
        a[1].key,
        a[2].key,
        a[3].key,
        a[4].key,
        role,
        attempt,
        value,
        time,
        evidence,
        archive,
        source.source_id,
    )
}
