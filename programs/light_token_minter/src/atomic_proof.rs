//! Immutable proof transport. Preparation changes no financial or source state;
//! the financial instruction still verifies the proof through Light.
use crate::compact_error::CompactAccountInfo;
use crate::ProgramError;
use borsh::BorshSerialize;
use solana_program::{
    account_info::AccountInfo,
    hash::{hash, hashv},
    pubkey::Pubkey,
};
pub const SEED: &[u8] = b"atomic-proof-v1";
pub const MAGIC: [u8; 8] = *b"ATMPRF01";
pub const VERSION: u8 = 2;
pub const RENT_PAYER_LEN: usize = 32;
pub const LEN: usize = 170 + RENT_PAYER_LEN;
pub const BUNDLE_SEED: &[u8] = b"atomic-proof-bundle-v1";
pub const BUNDLE_MAGIC: [u8; 8] = *b"ATMPRF02";
pub const BUNDLE_HEADER_LEN: usize = 46;
pub const INLINE: u8 = 255;
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone,Debug,Eq,PartialEq,BorshSerialize)]
    pub struct Upload{pub proofs:Vec<[u8;128]>}
}
/// Ordered cardinality includes zero placeholders for index-only batches.
pub fn bundle_hash(proofs: &[[u8; 128]]) -> Result<[u8; 32], ProgramError> {
    let count = u32::try_from(proofs.len()).map_err(|_| ProgramError::InvalidInstructionData)?;
    if count == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    if count == 1 {
        return Ok(hash(&proofs[0]).to_bytes());
    }
    let count = count.to_le_bytes();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(proofs.len() + 1);
    parts.push(&count);
    parts.extend(proofs.iter().map(|p| p.as_slice()));
    Ok(hashv(&parts).to_bytes())
}
pub fn derive_bundle(program: &Pubkey, proofs: &[[u8; 128]]) -> Result<(Pubkey, u8), ProgramError> {
    let digest = bundle_hash(proofs)?;
    let seed = if proofs.len() == 1 { SEED } else { BUNDLE_SEED };
    Ok(Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            seed,
            &digest,
        ],
        program,
    ))
}
pub fn encode_bundle(
    program: &Pubkey,
    proofs: &[[u8; 128]],
    payer: &Pubkey,
) -> Result<Vec<u8>, ProgramError> {
    let (_, bump) = derive_bundle(program, proofs)?;
    if proofs.len() == 1 {
        return Ok(encode(program, &proofs[0], payer).to_vec());
    }
    let len = proofs
        .len()
        .checked_mul(128)
        .and_then(|v| v.checked_add(BUNDLE_HEADER_LEN + RENT_PAYER_LEN))
        .ok_or(ProgramError::InvalidInstructionData)?;
    let mut data = vec![0; len];
    data[..8].copy_from_slice(&BUNDLE_MAGIC);
    data[8] = VERSION;
    data[9] = bump;
    data[10..42].copy_from_slice(&bundle_hash(proofs)?);
    data[42..46].copy_from_slice(&(proofs.len() as u32).to_le_bytes());
    for (target, proof) in data[46..].chunks_exact_mut(128).zip(proofs) {
        target.copy_from_slice(proof);
    }
    data[len - RENT_PAYER_LEN..].copy_from_slice(payer.as_ref());
    Ok(data)
}
pub fn decode_bundle(
    program: &Pubkey,
    key: &Pubkey,
    data: &[u8],
) -> Result<Vec<[u8; 128]>, ProgramError> {
    if data.len() < 42 + RENT_PAYER_LEN || data[8] != VERSION {
        return Err(ProgramError::InvalidAccountData);
    }
    let proofs = if data[..8] == MAGIC {
        if data.len() != LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        vec![data[42..data.len() - RENT_PAYER_LEN]
            .try_into()
            .map_err(|_| ProgramError::InvalidAccountData)?]
    } else if data[..8] == BUNDLE_MAGIC {
        if data.len() < BUNDLE_HEADER_LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut count_bytes = [0; 4];
        count_bytes.copy_from_slice(&data[42..46]);
        let count = u32::from_le_bytes(count_bytes) as usize;
        if count < 2
            || count
                .checked_mul(128)
                .and_then(|n| n.checked_add(BUNDLE_HEADER_LEN + RENT_PAYER_LEN))
                != Some(data.len())
        {
            return Err(ProgramError::InvalidAccountData);
        }
        data[46..data.len() - RENT_PAYER_LEN]
            .chunks_exact(128)
            .map(|p| {
                let mut proof = [0; 128];
                proof.copy_from_slice(p);
                proof
            })
            .collect()
    } else {
        return Err(ProgramError::InvalidAccountData);
    };
    let (expected, bump) = derive_bundle(program, &proofs)?;
    if *key != expected || data[9] != bump || data[10..42] != bundle_hash(&proofs)? {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(proofs)
}
pub fn derive(program: &Pubkey, proof: &[u8; 128]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            &hash(proof).to_bytes(),
        ],
        program,
    )
}
pub fn encode(program: &Pubkey, proof: &[u8; 128], payer: &Pubkey) -> [u8; LEN] {
    let mut bytes = [0; LEN];
    bytes[..8].copy_from_slice(&MAGIC);
    bytes[8] = VERSION;
    bytes[9] = derive(program, proof).1;
    bytes[10..42].copy_from_slice(&hash(proof).to_bytes());
    bytes[42..LEN - RENT_PAYER_LEN].copy_from_slice(proof);
    bytes[LEN - RENT_PAYER_LEN..].copy_from_slice(payer.as_ref());
    bytes
}
pub fn decode(program: &Pubkey, key: &Pubkey, data: &[u8]) -> Result<[u8; 128], ProgramError> {
    if data.len() != LEN || data[..8] != MAGIC || data[8] != VERSION {
        return Err(ProgramError::InvalidAccountData);
    }
    let proof: [u8; 128] = data[42..LEN - RENT_PAYER_LEN]
        .try_into()
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let (expected, bump) = derive(program, &proof);
    if *key != expected || data[9] != bump || data[10..42] != hash(&proof).to_bytes() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(proof)
}
/// The first uploader's refund address is immutable program-owned account data.
/// Reusing the same proof never replaces the original payer.
pub fn rent_payer(program: &Pubkey, key: &Pubkey, data: &[u8]) -> Result<Pubkey, ProgramError> {
    decode_bundle(program, key, data)?;
    let mut payer = [0; RENT_PAYER_LEN];
    payer.copy_from_slice(&data[data.len() - RENT_PAYER_LEN..]);
    Ok(Pubkey::new_from_array(payer))
}
pub fn load_at(
    program: &Pubkey,
    account: &AccountInfo,
    ordinal: usize,
) -> Result<[u8; 128], ProgramError> {
    if account.owner != program || account.executable || account.is_signer || account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }
    let proofs = decode_bundle(program, account.key, &account.try_data()?)?;
    proofs
        .get(if proofs.len() == 1 { 0 } else { ordinal })
        .copied()
        .ok_or(ProgramError::InvalidAccountData)
}
pub fn load(program: &Pubkey, account: &AccountInfo) -> Result<[u8; 128], ProgramError> {
    load_at(program, account, 0)
}
pub fn load_for_batch(
    program: &Pubkey,
    account: &AccountInfo,
    ordinal: usize,
    batch_count: usize,
) -> Result<[u8; 128], ProgramError> {
    if account.owner != program || account.executable || account.is_signer || account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }
    let proofs = decode_bundle(program, account.key, &account.try_data()?)?;
    if proofs.len() != 1 && proofs.len() != batch_count {
        return Err(ProgramError::InvalidAccountData);
    }
    if ordinal >= batch_count {
        return Err(ProgramError::InvalidAccountData);
    }
    proofs
        .get(if proofs.len() == 1 { 0 } else { ordinal })
        .copied()
        .ok_or(ProgramError::InvalidAccountData)
}
