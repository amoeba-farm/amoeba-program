//! Fully compressed option entitlement. The leaf itself is the replay-protected claim.
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub const CLAIM_WINDOW_SECONDS: u64 = 72 * 60 * 60;
pub const SPONSORED_REDEMPTION_FEE_ATOMS: u64 = 10_000;
pub const RETIREMENT_SEED: &[u8] = b"compressed-retirement";

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CompressedOptionClaim {
    pub amount: u64,
    pub has_delegate: bool,
    pub leaf_index: u32,
    pub root_index: u16,
    pub prove_by_index: bool,
    pub proof: Option<[u8; 128]>,
}

impl CompressedOptionClaim {
    pub fn valid(&self) -> bool {
        self.amount > 0
            && if self.prove_by_index {
                self.root_index == 0
            } else {
                self.proof.is_some()
            }
    }
}

/// A single aggregate proof authenticates the option and optional WriterCash
/// input. A zero cash amount only adds authenticated reserve backing; payout
/// still comes from the sleeve's existing vault.
#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CompressedCashOptionClaim {
    pub option: CompressedOptionClaim,
    pub cash_amount: u64,
    pub cash_leaf_index: u32,
    pub cash_root_index: u16,
    pub cash_prove_by_index: bool,
}

impl CompressedCashOptionClaim {
    pub fn valid(&self) -> bool {
        self.option.valid()
            && if self.cash_amount == 0 {
                self.cash_leaf_index == 0 && self.cash_root_index == 0 && !self.cash_prove_by_index
            } else if self.cash_prove_by_index {
                self.cash_root_index == 0
            } else {
                self.option.proof.is_some()
            }
    }
}

pub fn retirement_owner(program: &Pubkey, sleeve: &Pubkey, market: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            RETIREMENT_SEED,
            sleeve.as_ref(),
            market.as_ref(),
        ],
        program,
    )
    .0
}

/// Future compressed series declare a 72-hour window after finalization.
pub fn deadline(policy: [u8; 4], finalized_ts: u64) -> Option<u64> {
    match policy {
        [1, 1, 0, 0] if finalized_ts != 0 => finalized_ts.checked_add(CLAIM_WINDOW_SECONDS),
        _ => None,
    }
}

/// The fee is taken from this claim's payout, never from another holder's reserve.
pub fn payout_parts(payout: u64, sponsored: bool) -> Option<(u64, u64)> {
    let fee = if sponsored {
        SPONSORED_REDEMPTION_FEE_ATOMS
    } else {
        0
    };
    payout.checked_sub(fee).map(|net| (net, fee))
}

/// Fixed Transfer2 shape: one regular v3 input, one undelegated compressed sink.
/// No compression/decompression operation, TLV, token account, rent or top-up exists.
pub(crate) fn retire_instruction(
    keys: &[Pubkey; 25],
    claim: &CompressedOptionClaim,
    keeper: bool,
) -> Instruction {
    let mut data = vec![101, 0, 0, 0, 0, 2, 0, 0, 0, 0];
    if let Some(proof) = claim.proof {
        data.push(1);
        data.extend_from_slice(&proof);
    } else {
        data.push(0);
    }
    data.extend_from_slice(&1u32.to_le_bytes());
    data.push(3); // packed holder
    data.extend_from_slice(&claim.amount.to_le_bytes());
    data.extend_from_slice(&[u8::from(claim.has_delegate), 5, 4, 3, 0, 1]);
    data.extend_from_slice(&claim.leaf_index.to_le_bytes());
    data.push(u8::from(claim.prove_by_index));
    data.extend_from_slice(&claim.root_index.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.push(6); // permanently locked compressed retirement owner
    data.extend_from_slice(&claim.amount.to_le_bytes());
    data.extend_from_slice(&[0, 0, 4, 3, 0, 0, 0, 0]);
    let indices = [16, 0, 13, 17, 18, 19, 15, 20, 21, 22, 1, 7, 23, 24];
    Instruction {
        program_id: keys[12],
        data,
        accounts: indices
            .iter()
            .map(|&i| AccountMeta {
                pubkey: keys[i],
                is_signer: i == 0 || (i == 1 && !keeper) || (i == 23 && keeper),
                is_writable: matches!(i, 0 | 20 | 21 | 22),
            })
            .collect(),
    }
}
