//! Immutable public preimages and permanent publication receipts. No economic rights.
use super::*;
use crate::compact_error::CompactAccountInfo;
use solana_program::hash::hashv;

const BYTES_SEED: &[u8] = b"g3-evidence-bytes-v1";
const LINK_SEED: &[u8] = b"g3-evidence-link-v1";
const HEADER: usize = 76;
pub(super) const CHUNK: usize = 192;
pub(super) const DEFINITION_MAX: usize = 4096;
const LINK_LEN: usize = 251;

/// Terminal cleanup reads this to authenticate a link before reclaiming its rent.
pub(super) fn validate_cleanup_link(
    program: &Pubkey,
    month: &Pubkey,
    info: &AccountInfo,
) -> ProgramResult {
    if info.owner != program || info.executable || info.data_len() != LINK_LEN {
        return invalid();
    }
    let d = info.try_data()?;
    let event = read_key(&d[70..102]);
    let commitment: [u8; 32] = d[187..219]
        .try_into()
        .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?;
    let (key, bump) = link_address(program, &event, d[166], &commitment);
    if *info.key != key
        || d[..6] != [b'O', b'E', b'L', 1, 1, bump]
        || d[6..38] != month.to_bytes()
        || d[166] > 5
        || commitment == [0; 32]
        || crate::pubkey_is_default(&event)
    {
        return invalid();
    }
    Ok(())
}

/// The month must be the canonical month of the supplied period: its PDA is derived from the
/// period key and the period's expiry, exactly as the month initializer creates it.
fn require_evidence_month_open(
    program: &Pubkey,
    period: &AccountInfo,
    info: &AccountInfo,
) -> ProgramResult {
    let month: OracleMonthState = load_exact_zero_padded_state(
        info,
        program,
        OracleMonthState::LEN,
        OracleError::InvalidOracleState,
    )?;
    let anchor = load_valid_period_anchor(program, period).or_else(|_| invalid())?;
    let (key, bump) = derive_oracle_month_pda(program, period.key, anchor.expiry_ts);
    if info.executable
        || *info.key != key
        || !month.is_initialized
        || month.bump != bump
        || month.period != *period.key
        || month.account_discriminator != OracleMonthState::ACCOUNT_DISCRIMINATOR
        || month.account_version != OracleMonthState::ACCOUNT_VERSION
        || month.phase == OraclePhase::Closed
    {
        return invalid();
    }
    Ok(())
}

fn invalid<T>() -> Result<T, ProgramError> {
    Err(OracleError::InvalidOracleOpeningEvidence.into())
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
/// Object kinds: 1 a public locator, 2 a source definition, 3 a Wayback archive URL, 4 an
/// authenticated evidence record (`ORACLE_EVIDENCE_RECORD_KIND`). Kinds 3 and 4 are the two forms
/// of a claim's archive evidence.
fn max_bytes(kind: u8) -> Result<usize, ProgramError> {
    match kind {
        1 | 3 => Ok(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES),
        2 => Ok(DEFINITION_MAX),
        ORACLE_EVIDENCE_RECORD_KIND => Ok(ORACLE_EVIDENCE_RECORD_MAX_BYTES),
        _ => invalid(),
    }
}
fn read_key(data: &[u8]) -> Pubkey {
    let mut key = [0; 32];
    key.copy_from_slice(data);
    Pubkey::new_from_array(key)
}
fn validate_object(program: &Pubkey, info: &AccountInfo) -> Result<(), ProgramError> {
    if info.owner != program || info.executable || info.is_signer || info.data_len() < HEADER {
        return invalid();
    }
    let d = info.try_data()?;
    let payer = read_key(&d[6..38]);
    let kind = d[38];
    let hash: &[u8; 32] = d[39..71]
        .try_into()
        .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?;
    let total = usize::from(u16::from_le_bytes([d[71], d[72]]));
    let written = usize::from(u16::from_le_bytes([d[73], d[74]]));
    let (key, bump) = bytes_address(program, &payer, kind, hash);
    if d[..5] != [b'O', b'E', b'B', 1, 1]
        || d[5] != bump
        || *info.key != key
        || crate::pubkey_is_default(&payer)
        || *hash == [0; 32]
        || total == 0
        || total > max_bytes(kind)?
        || d.len() != HEADER + total
        || written > total
        || d[75] > 1
        || (d[75] == 1) != (written == total)
        || !crate::bytes_are_zero(&d[HEADER + written..])
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
    // Byte-level parsing: every delimiter and credential key is ASCII, so this matches the
    // previous `str` search exactly without linking UTF-8 search/lossy-conversion code.
    let input = url.as_bytes();
    let tail = input
        .strip_prefix(b"https://")
        .or_else(|| input.strip_prefix(b"http://"))
        .ok_or(OracleError::InvalidOracleOpeningEvidence)?;
    let authority_len = tail
        .iter()
        .position(|b| matches!(b, b'/' | b'?' | b'#'))
        .unwrap_or(tail.len());
    let authority = tail.get(..authority_len).unwrap_or(&[]);
    if authority.is_empty() || authority.contains(&b'@') || authority.first() == Some(&b':') {
        return invalid();
    }
    // Inspect percent-encoded query names without changing the committed bytes.
    let mut scanned = Vec::with_capacity(url.len());
    let mut i = 0;
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
    // A lossy UTF-8 view never merges or splits ASCII runs, so searching the raw bytes for
    // these ASCII keys is equivalent to the former `from_utf8_lossy(..).contains(key)`.
    // Every key ends in `=`, so an occurrence is exactly a `=` whose prefix ends with the key:
    // one linear pass, comparing keys only at the (rare) `=` bytes.
    const CREDENTIAL_KEYS: [&[u8]; 8] = [
        b"access_token=",
        b"api_key=",
        b"apikey=",
        b"authorization=",
        b"password=",
        b"secret=",
        b"x-amz-signature=",
        b"token=",
    ];
    for (end, byte) in scanned.iter().enumerate() {
        if *byte == b'=' {
            let prefix = &scanned[..=end];
            if CREDENTIAL_KEYS.iter().any(|key| prefix.ends_with(key)) {
                return invalid();
            }
        }
    }
    Ok(())
}
fn committed_hash(kind: u8, bytes: &[u8]) -> Result<[u8; 32], ProgramError> {
    match kind {
        1 => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?;
            public_locator(text)?;
            Ok(hashv(&[b"locator", bytes]).to_bytes())
        }
        // Full SHA-256 input transcript, including any structured serialization/domain.
        // This does not reinterpret an opaque definition commitment as H(display text).
        2 => Ok(hashv(&[bytes]).to_bytes()),
        // Kind 3 seals only a Wayback URL: an evidence record never seals as kind 3.
        3 => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?;
            let (_, target) = parse_oracle_opening_archive_url(text)?;
            public_locator(target)?;
            Ok(derive_oracle_opening_archive_url_hash(text))
        }
        // Kind 4 seals only an evidence record (its parser applies `public_locator` to `u`), and
        // commits to it exactly like kind 3, so every archive hash downstream (claims,
        // challenges, observation chains, checkpoints, update commitments) is the same function
        // of the text. The forms cannot collide (disjoint prefixes), and the object PDA seeds the
        // kind.
        ORACLE_EVIDENCE_RECORD_KIND => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?;
            parse_oracle_evidence_record(text)?;
            Ok(derive_oracle_opening_archive_url_hash(text))
        }
        _ => invalid(),
    }
}
/// Exact immutable kind-2 preimage for a pinned prospective registry.
#[cfg(test)]
pub(super) fn sealed_definition_preimage(
    program: &Pubkey,
    object: &AccountInfo,
    expected_commitment: &[u8; 32],
) -> Result<Vec<u8>, ProgramError> {
    with_sealed_definition_preimage(program, object, expected_commitment, |bytes| {
        Ok(bytes.to_vec())
    })
}

/// Decode while the authenticated account bytes are borrowed, without a heap copy. The carry
/// registry reads its pinned prospective definition through this.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn with_sealed_definition_preimage<T>(
    program: &Pubkey,
    object: &AccountInfo,
    expected_commitment: &[u8; 32],
    decode: impl FnOnce(&[u8]) -> Result<T, ProgramError>,
) -> Result<T, ProgramError> {
    validate_object(program, object)?;
    let data = object.try_data()?;
    if data[38] != 2
        || data[75] != 1
        || data[39..71] != expected_commitment[..]
        || committed_hash(2, &data[HEADER..])? != *expected_commitment
    {
        return invalid();
    }
    decode(&data[HEADER..])
}

/// The object kinds that carry a claim's archive evidence: a Wayback URL or an evidence record.
fn is_archive_kind(kind: u8) -> bool {
    kind == 3 || kind == ORACLE_EVIDENCE_RECORD_KIND
}

/// Current G3 publication transports the URL only through its sealed object.
/// Reject inline URL bytes: there is one current payload interpretation.
/// Opening and update claims (162, 163, 198, 168) read their archive evidence through this: a
/// sealed Wayback URL (kind 3) or authenticated evidence record (kind 4), returned as text.
pub(super) fn publication_url(
    program: &Pubkey,
    object: &AccountInfo,
    wire_url: &str,
) -> Result<String, ProgramError> {
    if !wire_url.is_empty() {
        return invalid();
    }
    validate_object(program, object)?;
    let d = object.try_data()?;
    let kind = d[38];
    if !is_archive_kind(kind) || d[75] != 1 || committed_hash(kind, &d[HEADER..])? != d[39..71] {
        return invalid();
    }
    Ok(std::str::from_utf8(&d[HEADER..])
        .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?
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
        .ok_or(OracleError::ArithmeticOverflow)?;
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
        let mut d = a[1].try_data_mut()?;
        d[..6].copy_from_slice(&[b'O', b'E', b'B', 1, 1, bump]);
        d[6..38].copy_from_slice(a[0].key.as_ref());
        d[38] = kind;
        d[39..71].copy_from_slice(&hash);
        d[71..73].copy_from_slice(&total.to_le_bytes());
    }
    validate_object(program, &a[1])?;
    let mut d = a[1].try_data_mut()?;
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
        let d = a[1].try_data()?;
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
        let d = object.try_data()?;
        // Archive roles (2-5: claims and challenges) also take the kind-4 form of archive
        // evidence, an authenticated evidence record. A source's locator and definition (roles 0
        // and 1) never do: a record is claim evidence, not a source's identity.
        let kind = if kind == 3 && d[38] == ORACLE_EVIDENCE_RECORD_KIND {
            ORACLE_EVIDENCE_RECORD_KIND
        } else {
            kind
        };
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
        let d = link.try_data()?;
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
    link.try_data_mut()?.copy_from_slice(&bytes);
    Ok(())
}

/// Permissionless exact-preimage recovery only. This changes no source/claim,
/// timestamp, observation, economic right or verdict, and cannot read hidden commits.
///
/// Accounts: 0 payer signer; 1 month; 2 source; 3 locator object; 4 writable locator link;
/// 5 definition object; 6 writable definition link; 7 system program; 8 the month's period.
pub(super) fn backfill_source(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 9 {
        return invalid();
    }
    require_evidence_month_open(program, &a[8], &a[1])?;
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
/// Accounts: 0 payer signer; 1 month; 2 source; 3 claim or challenge; 4 parent claim;
/// 5 archive evidence object (a kind-3 Wayback URL or kind-4 evidence record); 6 writable link;
/// 7 system program; 8 the month's period.
pub(super) fn backfill_claim(program: &Pubkey, a: &[AccountInfo], role: u8) -> ProgramResult {
    if a.len() != 9 {
        return invalid();
    }
    require_evidence_month_open(program, &a[8], &a[1])?;
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
        let d = a[5].try_data()?;
        let url = std::str::from_utf8(&d[HEADER..])
            .map_err(|_| OracleError::InvalidOracleOpeningEvidence)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_definition_preimage_requires_owned_exact_hash_object() {
        let program = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let body = b"amoeba-carry-registry-v1-test-preimage";
        let commitment = committed_hash(2, body).unwrap();
        let (key, bump) = bytes_address(&program, &payer, 2, &commitment);
        let mut data = vec![0; HEADER + body.len()];
        data[..6].copy_from_slice(&[b'O', b'E', b'B', 1, 1, bump]);
        data[6..38].copy_from_slice(payer.as_ref());
        data[38] = 2;
        data[39..71].copy_from_slice(&commitment);
        data[71..73].copy_from_slice(&(body.len() as u16).to_le_bytes());
        data[73..75].copy_from_slice(&(body.len() as u16).to_le_bytes());
        data[75] = 1;
        data[HEADER..].copy_from_slice(body);
        let read = |bytes: &mut [u8], owner: &Pubkey, digest: &[u8; 32]| {
            let mut lamports = 1;
            let info = AccountInfo::new(&key, false, false, &mut lamports, bytes, owner, false, 0);
            sealed_definition_preimage(&program, &info, digest)
        };
        assert_eq!(
            read(&mut data.clone(), &program, &commitment).unwrap(),
            body
        );
        for i in 0..data.len() {
            let mut changed = data.clone();
            changed[i] ^= 1;
            assert!(
                read(&mut changed, &program, &commitment).is_err(),
                "byte {i}"
            );
        }
        assert!(read(&mut data.clone(), &Pubkey::new_unique(), &commitment).is_err());
        assert!(read(&mut data, &program, &[0; 32]).is_err());
    }
    #[test]
    fn exact_role_hashes_and_credential_rejection() {
        let locator = b"https://example.com/price?x=%2f";
        assert_eq!(
            committed_hash(1, locator).unwrap(),
            hashv(&[b"locator", locator]).to_bytes()
        );
        assert_ne!(
            committed_hash(1, locator).unwrap(),
            committed_hash(2, locator).unwrap()
        );
        assert!(public_locator("https://name:pass@example.com/x").is_err());
        assert!(public_locator("https://example.com/?api_key=secret").is_err());
        assert!(public_locator("https://example.com/?api%5Fkey=secret").is_err());
        assert!(public_locator("https://example.com/?access%5Ftoken=x").is_err());
        assert!(public_locator("https://example.com/price?x=%2f").is_ok());
        assert_eq!(HEADER, 76);
        assert_eq!(LINK_LEN, 6 + 32 * 6 + 1 + 4 + 8 + 8 + 32);
    }
    /// The pre-size-reduction `public_locator`, kept verbatim as the behavioral reference.
    fn reference_public_locator(url: &str) -> bool {
        if url.is_empty()
            || url.len() > ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES
            || url.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\')
        {
            return false;
        }
        let Some(tail) = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
        else {
            return false;
        };
        let authority = tail.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') || authority.starts_with(':') {
            return false;
        }
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
                        return false;
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
        ![
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
    }

    const LOCATOR_ATOMS: &[&str] = &[
        "https://",
        "http://",
        "HTTPS://",
        "a",
        "Z",
        "/",
        "?",
        "#",
        "@",
        ":",
        "%",
        "%5f",
        "%5F",
        "%41",
        "%ff",
        "%c3",
        "%a9",
        "%e2%82",
        "%80",
        "%20",
        "%7f",
        "%5c",
        "%3d",
        "=",
        "&",
        "_",
        "-",
        ".",
        "0",
        "f",
        "g",
        "token",
        "TOKEN",
        "api",
        "key",
        "apikey",
        "access",
        "authorization",
        "password",
        "secret",
        "x-amz-signature",
        "\u{e9}",
        "\u{20ac}",
        "\u{1f600}",
        "%ef%bf%bd",
        "example.com",
        " ",
        "\\",
        "\t",
    ];

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4096))]
        #[test]
        fn public_locator_matches_reference_on_structured_urls(
            https in proptest::bool::ANY,
            atoms in proptest::collection::vec(0..LOCATOR_ATOMS.len(), 0..40),
        ) {
            let mut url = String::from(if https { "https://" } else { "http://" });
            for atom in atoms {
                url.push_str(LOCATOR_ATOMS[atom]);
            }
            proptest::prop_assert_eq!(public_locator(&url).is_ok(), reference_public_locator(&url));
        }

        #[test]
        fn public_locator_matches_reference_on_arbitrary_strings(url in ".{0,600}") {
            proptest::prop_assert_eq!(public_locator(&url).is_ok(), reference_public_locator(&url));
        }
    }

    #[test]
    fn public_locator_rejects_at_maximum_length_boundary() {
        let base = "https://example.com/";
        let fill = ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES - base.len();
        let at_limit = format!("{base}{}", "a".repeat(fill));
        let over_limit = format!("{at_limit}a");
        assert_eq!(
            public_locator(&at_limit).is_ok(),
            reference_public_locator(&at_limit)
        );
        assert!(public_locator(&over_limit).is_err());
        assert!(!reference_public_locator(&over_limit));
        let credential = format!("{base}?{}token=", "a".repeat(fill - 7));
        assert!(public_locator(&credential).is_err());
        assert!(!reference_public_locator(&credential));
    }

    const DIGEST: &str = "abababababababababababababababababababababababababababababababab";

    fn record(method: &str, time: &str, digest: &str, locator: &str) -> String {
        format!("{ORACLE_EVIDENCE_RECORD_PREFIX}m={method};t={time};d={digest};u={locator}")
    }

    /// A sealed object of `kind` holding `body`, as `write` leaves it.
    fn sealed_object(program: &Pubkey, payer: &Pubkey, kind: u8, body: &[u8]) -> (Pubkey, Vec<u8>) {
        let commitment = committed_hash(kind, body).unwrap();
        let (key, bump) = bytes_address(program, payer, kind, &commitment);
        let mut data = vec![0; HEADER + body.len()];
        data[..6].copy_from_slice(&[b'O', b'E', b'B', 1, 1, bump]);
        data[6..38].copy_from_slice(payer.as_ref());
        data[38] = kind;
        data[39..71].copy_from_slice(&commitment);
        data[71..73].copy_from_slice(&(body.len() as u16).to_le_bytes());
        data[73..75].copy_from_slice(&(body.len() as u16).to_le_bytes());
        data[75] = 1;
        data[HEADER..].copy_from_slice(body);
        (key, data)
    }

    #[test]
    fn evidence_record_seals_as_kind_four_under_the_archive_url_commitment() {
        let text = record(
            "web",
            "1760900000",
            DIGEST,
            "https://example.com/price?x=%2f",
        );
        let commitment = committed_hash(ORACLE_EVIDENCE_RECORD_KIND, text.as_bytes()).unwrap();
        assert_eq!(
            commitment,
            hashv(&[ORACLE_OPENING_ARCHIVE_URL_HASH_DOMAIN, text.as_bytes()]).to_bytes()
        );
        assert_eq!(commitment, derive_oracle_opening_archive_url_hash(&text));
        let email = record("email", "1760900000", DIGEST, "https://example.com/price");
        assert!(committed_hash(4, email.as_bytes()).is_ok());
        assert_eq!(max_bytes(4).unwrap(), ORACLE_EVIDENCE_RECORD_MAX_BYTES);
        assert_eq!(ORACLE_EVIDENCE_RECORD_MAX_BYTES, 512);
        assert!(max_bytes(5).is_err());

        // Each archive kind seals only its own form.
        let wayback = "https://web.archive.org/web/20260701000000/https://example.com/price?x=%2f";
        assert!(committed_hash(3, text.as_bytes()).is_err());
        assert!(committed_hash(3, wayback.as_bytes()).is_ok());
        assert!(committed_hash(4, wayback.as_bytes()).is_err());
        // A record is neither a locator nor a definition commitment.
        assert!(committed_hash(1, text.as_bytes()).is_err());
        assert_ne!(committed_hash(2, text.as_bytes()).unwrap(), commitment);

        let upper = DIGEST.replacen('a', "A", 1);
        let long = format!("https://example.com/{}", "a".repeat(500));
        for bad in [
            record("web", "1760900000", &upper, "https://example.com/price"),
            record(
                "web",
                "1760900000",
                &DIGEST[1..],
                "https://example.com/price",
            ),
            record("wayback", "1760900000", DIGEST, "https://example.com/price"),
            record("web", "01760900000", DIGEST, "https://example.com/price"),
            record("web", "0", DIGEST, "https://example.com/price"),
            record(
                "web",
                "1760900000",
                DIGEST,
                "https://example.com/price?api_key=1",
            ),
            record(
                "web",
                "1760900000",
                DIGEST,
                "https://name:pass@example.com/price",
            ),
            record("web", "1760900000", DIGEST, &long),
            record("web", "1760900000", DIGEST, "https://example.com/a b"),
        ] {
            assert!(committed_hash(4, bad.as_bytes()).is_err(), "{bad}");
        }
        // The longest well-formed record (an `email` record, 20-digit time, 384-byte locator)
        // fits the 512-byte bound; anything past the bound is refused before parsing.
        let base = "https://example.com/";
        let at_limit = format!(
            "{base}{}",
            "a".repeat(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES - base.len())
        );
        let longest = record("email", "10000000000000000000", DIGEST, &at_limit);
        assert!(longest.len() <= ORACLE_EVIDENCE_RECORD_MAX_BYTES);
        assert!(committed_hash(4, longest.as_bytes()).is_ok());
        let over = format!("{longest}{}", "a".repeat(513 - longest.len()));
        assert_eq!(over.len(), 513);
        assert!(committed_hash(4, over.as_bytes()).is_err());
    }

    #[test]
    fn publication_reads_a_sealed_wayback_url_or_evidence_record() {
        let program = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let text = record("web", "1760900000", DIGEST, "https://example.com/price");
        let wayback = "https://web.archive.org/web/20260701000000/https://example.com/price";
        let read = |key: &Pubkey, bytes: &mut [u8], wire: &str| {
            let mut lamports = 1;
            let info =
                AccountInfo::new(key, false, false, &mut lamports, bytes, &program, false, 0);
            publication_url(&program, &info, wire)
        };
        for (kind, body) in [(4, text.as_str()), (3, wayback)] {
            let (key, data) = sealed_object(&program, &payer, kind, body.as_bytes());
            assert_eq!(read(&key, &mut data.clone(), "").unwrap(), body);
            // Inline wire bytes, an unsealed draft and a foreign kind byte are refused.
            assert!(read(&key, &mut data.clone(), body).is_err());
            let mut draft = data.clone();
            draft[75] = 0;
            draft[73..75].copy_from_slice(&0u16.to_le_bytes());
            draft[HEADER..].fill(0);
            assert!(read(&key, &mut draft, "").is_err());
            let mut relabelled = data.clone();
            relabelled[38] = 7 - kind;
            assert!(read(&key, &mut relabelled, "").is_err());
        }
        // Locator and definition objects are not archive evidence.
        let locator = b"https://example.com/price";
        let (key, data) = sealed_object(&program, &payer, 1, locator);
        assert!(read(&key, &mut data.clone(), "").is_err());
        let (key, data) = sealed_object(&program, &payer, 2, text.as_bytes());
        assert!(read(&key, &mut data.clone(), "").is_err());
        // A kind-4 object larger than its bound never validates.
        let (key, mut data) = sealed_object(&program, &payer, 4, text.as_bytes());
        data[71..73].copy_from_slice(&513u16.to_le_bytes());
        assert!(read(&key, &mut data, "").is_err());
    }

    /// `amoeba_evidence::parse_onchain_record` (`evidence/crates/evidence/src/lib.rs`) kept
    /// verbatim as the behavioral reference, with its `hash::parse_hex32` inlined.
    fn reference_parse_onchain_record(text: &str) -> Option<(&str, u64, [u8; 32], &str)> {
        fn parse_hex32(text: &str) -> Option<[u8; 32]> {
            if text.len() != 64 || text.bytes().any(|b| b.is_ascii_uppercase()) {
                return None;
            }
            let mut out = [0u8; 32];
            for (byte, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
                let high = (pair[0] as char).to_digit(16)?;
                let low = (pair[1] as char).to_digit(16)?;
                *byte = (high * 16 + low) as u8;
            }
            Some(out)
        }
        let rest = text.strip_prefix("amoeba-evidence-v1;")?;
        let rest = rest.strip_prefix("m=")?;
        let (method, rest) = rest.split_once(";t=")?;
        let (time, rest) = rest.split_once(";d=")?;
        let (digest, locator) = rest.split_once(";u=")?;
        if !matches!(method, "web" | "email")
            || time.is_empty()
            || time.len() > 20
            || !time.bytes().all(|c| c.is_ascii_digit())
            || (time.len() > 1 && time.starts_with('0'))
        {
            return None;
        }
        Some((method, time.parse().ok()?, parse_hex32(digest)?, locator))
    }

    /// What the program accepts: the reference's form, `t` nonzero, the oracle's locator rules on
    /// `u`, and the archive URL's byte rules and the record bound on the whole text.
    fn expected_record(text: &str) -> Option<(u64, &str)> {
        let (_, time, _, locator) = reference_parse_onchain_record(text)?;
        (text.len() <= ORACLE_EVIDENCE_RECORD_MAX_BYTES
            && !text.bytes().any(|b| b <= b' ' || b == 0x7f)
            && time != 0
            && reference_public_locator(locator))
        .then_some((time, locator))
    }

    const RECORD_PREFIXES: &[&str] = &[
        "amoeba-evidence-v1;",
        "amoeba-evidence-v1",
        "amoeba-evidence-v2;",
        "AMOEBA-evidence-v1;",
        "",
    ];
    const RECORD_METHODS: &[&str] = &[
        "m=web",
        "m=email",
        "m=wayback",
        "m=WEB",
        "m=",
        "web",
        "m=web;m=web",
        "m=em;ail",
        "m=web ",
    ];
    const RECORD_TIMES: &[&str] = &[
        ";t=1760900000",
        ";t=1",
        ";t=0",
        ";t=00",
        ";t=01",
        ";t=",
        ";t=18446744073709551615",
        ";t=18446744073709551616",
        ";t=99999999999999999999",
        ";t=100000000000000000000",
        ";t=1;t=2",
        ";t=1a",
        ";t=+1",
        ";t=-1",
        ";t= 1",
        "t=1",
    ];
    const RECORD_LOCATORS: &[&str] = &[
        ";u=https://example.com/price?x=1",
        ";u=http://example.com/x",
        ";u=https://example.com/x?token=1",
        ";u=https://example.com/x?api%5Fkey=1",
        ";u=",
        ";u=ftp://example.com/x",
        ";u=https://u@example.com/x",
        ";u=https://example.com/x;u=https://other.example/y",
        ";u=https://example.com/x;d=1;t=2",
        ";u=https://example.com/\u{e9}",
        ";u=https://example.com/ x",
        ";u=https://example.com/\t",
        ";u=https://example.com/\u{7f}",
        "u=https://example.com/x",
        ";u=https://:443/x",
    ];

    fn record_digest(choice: usize) -> String {
        match choice {
            0 => format!(";d={DIGEST}"),
            1 => format!(";d={}", "0123456789abcdef".repeat(4)),
            2 => format!(";d={}", DIGEST.replacen('b', "B", 1)),
            3 => format!(";d={}", &DIGEST[1..]),
            4 => format!(";d={DIGEST}0"),
            5 => format!(";d={}", DIGEST.replacen('b', "g", 1)),
            6 => ";d=".to_string(),
            7 => format!(";d={DIGEST};d={DIGEST}"),
            8 => format!("d={DIGEST}"),
            _ => format!(";d={}", "\u{e9}".repeat(32)),
        }
    }

    #[test]
    fn evidence_record_parser_matches_reference_on_every_structured_combination() {
        let (mut accepted, mut rejected) = (0, 0);
        for prefix in RECORD_PREFIXES {
            for method in RECORD_METHODS {
                for time in RECORD_TIMES {
                    for digest in 0..10 {
                        for locator in RECORD_LOCATORS {
                            let text =
                                format!("{prefix}{method}{time}{}{locator}", record_digest(digest));
                            let parsed = parse_oracle_evidence_record(&text).ok();
                            assert_eq!(parsed, expected_record(&text), "{text:?}");
                            if parsed.is_some() {
                                accepted += 1;
                            } else {
                                rejected += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(
            accepted > 0 && rejected > accepted,
            "{accepted} / {rejected}"
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4096))]
        #[test]
        fn evidence_record_parser_matches_reference_on_structured_records(
            prefix in 0..RECORD_PREFIXES.len() * 2,
            method in 0..RECORD_METHODS.len() * 2,
            time in 0..RECORD_TIMES.len() * 2,
            digest in 0..20usize,
            locator in 0..RECORD_LOCATORS.len() * 2,
            tail in proptest::collection::vec(0..LOCATOR_ATOMS.len(), 0..12),
            tail_len in 0..24usize,
            pad in 0..450usize,
        ) {
            // An index past a list picks its first (well-formed) entry, so about half of each
            // field is well formed and whole records are often valid.
            let pick = |list: &[&'static str], index: usize| {
                list.get(index).copied().unwrap_or(list[0])
            };
            let mut text = format!(
                "{}{}{}{}{}",
                pick(RECORD_PREFIXES, prefix),
                pick(RECORD_METHODS, method),
                pick(RECORD_TIMES, time),
                record_digest(if digest < 10 { digest } else { digest % 2 }),
                pick(RECORD_LOCATORS, locator),
            );
            for atom in tail.iter().take(tail_len.saturating_sub(12)) {
                text.push_str(LOCATOR_ATOMS[*atom]);
            }
            // Sometimes long enough to cross the locator and record bounds.
            if pad > 300 {
                text.push_str(&"a".repeat(pad));
            }
            proptest::prop_assert_eq!(
                parse_oracle_evidence_record(&text).ok(),
                expected_record(&text)
            );
        }

        #[test]
        fn evidence_record_parser_matches_reference_on_arbitrary_tails(tail in ".{0,520}") {
            for head in ["amoeba-evidence-v1;m=web;t=", "amoeba-evidence-v1;m=email;t=1760900000;d="] {
                let text = format!("{head}{tail}");
                proptest::prop_assert_eq!(
                    parse_oracle_evidence_record(&text).ok(),
                    expected_record(&text)
                );
            }
            proptest::prop_assert_eq!(
                parse_oracle_evidence_record(&tail).ok(),
                expected_record(&tail)
            );
        }
    }
}
