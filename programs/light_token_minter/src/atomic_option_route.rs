//! Typed normal-option routing against one authenticated atomic order.
//! This wire selects canonical source contexts and exact proof leaves; it cannot
//! execute caller-provided instructions or change the order's asset vector.
use crate::{
    ameba_dlmm_instruction::CompressedSwapLeafWitnessV1,
    fixed_codec::{CheckedCursor, CursorField},
};
use borsh::{BorshDeserialize, BorshSerialize};

pub const COMMON: usize = 14;
pub const CONTEXT_ACCOUNTS: usize = 10;
pub const NO_CONTEXT: u8 = 255;
/// Actual per-CPI capacities of the pinned Light Transfer2/v2-tree programs.
/// Orders may span any number of batches fitting the transaction resources.
pub const MAX_BATCH_INPUTS: usize = 8;
pub const MAX_BATCH_OUTPUTS: usize = 10;
pub const MAX_BATCH_MINTS: usize = 5;

/// All input authorities are program-controlled and authenticated by the order
/// or source state. The account index identifies the canonical context/asset,
/// never an arbitrary token owner supplied by a filler.
#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub enum Source {
    Escrow,
    Market(u8),
    WriterCash(u8),
}
impl BorshDeserialize for Source {
    fn deserialize_reader<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        match u8::deserialize_reader(r)? {
            0 => Ok(Self::Escrow),
            1 => Ok(Self::Market(u8::deserialize_reader(r)?)),
            2 => Ok(Self::WriterCash(u8::deserialize_reader(r)?)),
            _ => Err(std::io::ErrorKind::InvalidData.into()),
        }
    }
}
impl CursorField for Source {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        match c.u8() {
            0 => Self::Escrow,
            1 => Self::Market(c.u8()),
            2 => Self::WriterCash(c.u8()),
            _ => {
                c.invalid = true;
                Self::Escrow
            }
        }
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Input {
        pub asset:u8,
        pub source:Source,
        pub amount:u64,
        pub witness:CompressedSwapLeafWitnessV1,
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Batch {
    pub proof: Option<[u8; 128]>,
    /// 255 inline/index-only; otherwise an immutable proof-tail account.
    pub proof_account: u8,
    pub inputs: Vec<Input>,
    pub output_tree: u8,
    pub output_queue: u8,
    /// Canonical bitset with a u8 BYTE-length prefix. Even the loose bound
    /// of five outputs per u8-indexed asset plus one per u8 context and two
    /// quote outputs needs only 192 bytes. This excludes no wire capability.
    /// Only the CPI is selected; amounts, owners and mints stay native.
    pub output_mask: Vec<u8>,
}
impl BorshSerialize for Batch {
    fn serialize<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        self.proof.serialize(w)?;
        self.proof_account.serialize(w)?;
        self.inputs.serialize(w)?;
        self.output_tree.serialize(w)?;
        self.output_queue.serialize(w)?;
        let count =
            u8::try_from(self.output_mask.len()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        count.serialize(w)?;
        w.write_all(&self.output_mask)
    }
}
impl crate::fixed_codec::CursorVecElement for Batch {}
impl CursorField for Batch {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        let proof = CursorField::read(c);
        let proof_account = c.u8();
        let inputs = CursorField::read(c);
        let output_tree = c.u8();
        let output_queue = c.u8();
        let count = usize::from(c.u8());
        Self {
            proof,
            proof_account,
            inputs,
            output_tree,
            output_queue,
            output_mask: c.vec(count),
        }
    }
}
impl BorshDeserialize for Batch {
    fn deserialize_reader<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        let proof = BorshDeserialize::deserialize_reader(r)?;
        let proof_account = u8::deserialize_reader(r)?;
        let inputs = BorshDeserialize::deserialize_reader(r)?;
        let output_tree = u8::deserialize_reader(r)?;
        let output_queue = u8::deserialize_reader(r)?;
        let mut output_mask = vec![0; usize::from(u8::deserialize_reader(r)?)];
        r.read_exact(&mut output_mask)?;
        Ok(Self {
            proof,
            proof_account,
            inputs,
            output_tree,
            output_queue,
            output_mask,
        })
    }
    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_deserialize(buf)
    }
    fn try_from_slice(data: &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_from_slice(data)
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Leg {
        pub limit_bin_id:u16,
        pub context_index:u8,
        pub delegate_index:u8,
        /// Optional canonical classic retirement account in the custody tail.
        pub retirement_index:u8,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Fill {
        pub nonce:[u8;32],
        pub contexts:u8,
        /// First-occurrence net-asset order from the immutable funded order.
        pub legs:Vec<Leg>,
        pub delegate_accounts:u8,
        pub custody_accounts:u8,
        pub proof_accounts:u8,
        pub merkle_accounts:u8,
        /// Original proof inputs and final outputs may be partitioned across
        /// CPIs. Native compressed carries preserve per-mint conservation.
        pub batches:Vec<Batch>,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Close {
        pub route: Fill,
        pub minimum_net_quote:u64,
        pub deadline_ts:u64,
        /// Flattened batch input order: 0 undelegated, 1 this filled Order,
        /// 2 canonical custody-owner/mint settlement capability. Source-owned
        /// leaves and quote leaves must use 0.
        pub delegated_inputs:Vec<u8>,
        /// Charge the fixed sponsorship fee only when the user elects to pay
        /// transaction costs in USDC. SOL-funded closes leave this false.
        pub pay_fee_in_usdc:bool,
    }
}
/// Authenticate a routing schedule against the actual native output count.
/// Consumers can use this after projecting the authenticated source states.
pub fn output_masks_valid(batches: &[Batch], count: usize) -> bool {
    let bytes = count.div_ceil(8);
    let mut seen = vec![0u8; bytes];
    for batch in batches {
        if batch.output_mask.len() != bytes {
            return false;
        }
        for (i, mask) in batch.output_mask.iter().copied().enumerate() {
            if seen[i] & mask != 0 {
                return false;
            }
            seen[i] |= mask;
        }
    }
    for (i, mask) in seen.into_iter().enumerate() {
        let bits = (count - i * 8).min(8);
        if mask != ((1u16 << bits) - 1) as u8 {
            return false;
        }
    }
    true
}
/// Emitted only by the native financial instruction. Consumers must authenticate
/// the program and successful transaction before accepting an immediate receipt.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct Receipt {
    pub owner: solana_program::pubkey::Pubkey,
    pub custody_owner: solana_program::pubkey::Pubkey,
    pub order: solana_program::pubkey::Pubkey,
    pub close: bool,
    pub slot: u64,
    pub quote_delta: i128,
    pub sponsor_fee: u64,
    pub assets: Vec<AssetReceipt>,
}
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct AssetReceipt {
    pub market: solana_program::pubkey::Pubkey,
    pub mint: solana_program::pubkey::Pubkey,
    pub option_delta: i128,
    pub quote_delta: i128,
}
