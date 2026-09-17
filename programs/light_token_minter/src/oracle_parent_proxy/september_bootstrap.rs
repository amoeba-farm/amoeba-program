//! Exact September council-bootstrap commitment. No account writes, synthetic
//! escrows or writes are exposed by this codec; processor::september_bootstrap
//! owns the consumed council-authorized transition into ordinary Game state.
use solana_program::{hash::hashv, program_error::ProgramError, pubkey::Pubkey};

use super::{invalid, NANDX_REGISTRY_HASH, RAMX_REGISTRY_HASH};

pub const DOMAIN: &[u8] = b"amoeba-september-cfm-bootstrap-v1";
pub const PROGRAM: Pubkey = solana_program::pubkey!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");
pub const EXPIRY: u64 = 1_790_812_800;
pub const MAX_PAYOUT_PER_CONTRACT_ATOMS: u64 = 12_000_000;
pub const LEN: usize = 702;
const ROWS_OFFSET: usize = 142;

/// Arrays are bounded before decoding. Order is RAMX's 13 and NANDX's 22
/// bytewise-sorted parent source ids, as authenticated by the pinned registries.
pub struct SeptemberBootstrapPlan {
    bytes: [u8; LEN],
    digest: [u8; 32],
}

impl SeptemberBootstrapPlan {
    pub fn decode(program: &Pubkey, bytes: &[u8]) -> Result<Self, ProgramError> {
        if *program != PROGRAM || bytes.len() != LEN {
            return invalid();
        }
        let data: [u8; LEN] = bytes
            .try_into()
            .map_err(|_| crate::error::VaultError::InvalidOracleState)?;
        if &data[..6] != b"SCB1\x01\x02"
            || data[6..38] == [0; 32]
            || number(&data, 38) != EXPIRY
            || data[46..78] != RAMX_REGISTRY_HASH
            || data[78..110] != NANDX_REGISTRY_HASH
        {
            return invalid();
        }
        for market in 0..4 {
            let cap = number(&data, 110 + market * 8);
            if cap != u64::MAX {
                return invalid();
            }
        }
        for index in 0..35 {
            let offset = ROWS_OFFSET + index * 16;
            let value = number(&data, offset);
            let timestamp = number(&data, offset + 8);
            if value == 0 || !(1_788_220_800..EXPIRY).contains(&timestamp) {
                return invalid();
            }
        }
        Ok(Self {
            digest: hashv(&[DOMAIN, program.as_ref(), &data]).to_bytes(),
            bytes: data,
        })
    }

    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// 0/1 = RAMX CALL/PUT, 2/3 = NANDX CALL/PUT. The user explicitly selected
    /// no oracle aggregate cap; u64::MAX leaves ordinary exposure arithmetic,
    /// writer solvency and custody enforcement intact. It is not a deposit.
    pub fn exposure_cap(&self, market: u8) -> Result<u64, ProgramError> {
        if market > 3 {
            return invalid();
        }
        Ok(number(&self.bytes, 110 + usize::from(market) * 8))
    }

    pub fn opening(&self, product: u8, parent_index: u8) -> Result<(u64, u64), ProgramError> {
        let index = match product {
            0 if parent_index < 13 => usize::from(parent_index),
            1 if parent_index < 22 => 13 + usize::from(parent_index),
            _ => return invalid(),
        };
        let offset = ROWS_OFFSET + index * 16;
        Ok((number(&self.bytes, offset), number(&self.bytes, offset + 8)))
    }
}

fn number(bytes: &[u8; LEN], offset: usize) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}
