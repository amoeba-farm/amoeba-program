//! Same Light V2 address derivation, through Solana's direct Keccak syscall.
//! Experimental adapter: compare both seed and address to the pinned SDK.
use light_sdk::address::AddressSeed;
use solana_program::pubkey::Pubkey;

pub fn hash_to_bn254_field_size_be(bytes: &[u8]) -> [u8; 32] {
    let mut value = solana_program::keccak::hashv(&[bytes, &[u8::MAX]]).to_bytes();
    value[0] = 0;
    value
}

pub fn derive_address(seeds: &[&[u8]], tree: &Pubkey, program: &Pubkey) -> ([u8; 32], AddressSeed) {
    assert!(seeds.len() <= 16);
    let suffix = [u8::MAX];
    let mut parts: [&[u8]; 17] = [&[]; 17];
    parts[..seeds.len()].copy_from_slice(seeds);
    parts[seeds.len()] = &suffix;
    let mut seed = solana_program::keccak::hashv(&parts[..seeds.len() + 1]).to_bytes();
    seed[0] = 0;
    let mut address =
        solana_program::keccak::hashv(&[&seed, tree.as_ref(), program.as_ref(), &suffix])
            .to_bytes();
    address[0] = 0;
    (address, AddressSeed(seed))
}
