use super::*;
use crate::compact_error::CompactAccountInfo;
pub(super) fn upload(
    program: &Pubkey,
    a: &[AccountInfo],
    p: crate::atomic_proof::Upload,
) -> ProgramResult {
    use crate::atomic_proof as proof;
    if a.len() != 3
        || !a[0].is_signer
        || !a[0].is_writable
        || a[0].executable
        || !a[1].is_writable
        || a[1].is_signer
        || a[1].executable
        || !crate::is_system_program(a[2].key)
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let (key, bump) = proof::derive_bundle(program, &p.proofs)?;
    if *a[1].key != key {
        return Err(ProgramError::InvalidAccountData);
    }
    if a[1].owner == program {
        if proof::decode_bundle(program, a[1].key, &a[1].try_data()?)? != p.proofs {
            return Err(ProgramError::InvalidAccountData);
        }
        return Ok(());
    }
    if !crate::is_system_program(a[1].owner) || !a[1].data_is_empty() {
        return Err(ProgramError::InvalidAccountData);
    }
    let digest = proof::bundle_hash(&p.proofs)?;
    let bump = [bump];
    let data = proof::encode_bundle(program, &p.proofs, a[0].key)?;
    let seed = if p.proofs.len() == 1 {
        proof::SEED
    } else {
        proof::BUNDLE_SEED
    };
    invoke_create_or_allocate_account(
        &a[0],
        &a[1],
        &a[2],
        program,
        data.len(),
        &[CURRENT_STATE_NAMESPACE_SEED, seed, &digest, &bump],
    )?;
    a[1].try_data_mut()?.copy_from_slice(&data);
    Ok(())
}

pub(super) fn reclaim(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() != 2
        || !a[0].is_signer
        || !a[0].is_writable
        || a[0].executable
        || !a[1].is_writable
        || a[1].is_signer
        || a[1].executable
        || a[1].owner != program
        || a[0].key == a[1].key
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let payer = crate::atomic_proof::rent_payer(program, a[1].key, &a[1].try_data()?)?;
    if payer != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    close_program_account(program, &a[1], &a[0])
}
