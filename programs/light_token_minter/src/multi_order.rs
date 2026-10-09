//! Funded, exact asset-vector exchanges. Market routes and strategy geometry are
//! deliberately outside this primitive. Every transfer belongs to one fill.
use crate::fixed_codec::{CheckedCursor, CursorField};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const SEED: &[u8] = b"atomic-multi-order-v1";
pub const MAGIC: [u8; 8] = *b"MULTORD1";
pub const VERSION: u8 = 1;
pub const OPEN: u8 = 0;
pub const FILLED: u8 = 1;
pub const CANCELLED: u8 = 2;
pub const EXPIRED: u8 = 3;
pub const COMMON: usize = 16;
/// Four option mints and the common quote fit Light's five-mint sum check.
pub const OPTIONS_PER_BATCH: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize)]
pub enum QuoteBound {
    MaximumDebit(u64),
    MinimumCredit(u64),
}
impl BorshDeserialize for QuoteBound {
    fn deserialize_reader<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        match u8::deserialize_reader(r)? {
            0 => Ok(Self::MaximumDebit(u64::deserialize_reader(r)?)),
            1 => Ok(Self::MinimumCredit(u64::deserialize_reader(r)?)),
            _ => Err(std::io::ErrorKind::InvalidData.into()),
        }
    }
}
impl CursorField for QuoteBound {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        match c.u8() {
            0 => Self::MaximumDebit(c.u64()),
            1 => Self::MinimumCredit(c.u64()),
            _ => {
                c.invalid = true;
                Self::MaximumDebit(0)
            }
        }
    }
}
impl QuoteBound {
    pub fn escrow(self) -> u64 {
        match self {
            Self::MaximumDebit(v) => v,
            Self::MinimumCredit(_) => 0,
        }
    }
    pub fn admits(self, owner_quote_delta: i128) -> bool {
        match self {
            Self::MaximumDebit(v) => owner_quote_delta >= -i128::from(v),
            Self::MinimumCredit(v) => owner_quote_delta >= i128::from(v),
        }
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Leg { pub market_index: u8, pub side: u8, pub quantity: u64 }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct BoundLeg {
        pub market: Pubkey, pub mint: Pubkey, pub expiry_ts: u64,
        pub side: u8, pub quantity: u64,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Input {
        /// 0 = quote; options are the first-occurrence mint order of the legs.
        pub asset: u8,
        /// 0 = action actor; 1 = the order escrow PDA.
        pub party: u8,
        pub amount: u64,
        pub delegated: bool,
        pub witness: crate::ameba_dlmm_instruction::CompressedSwapLeafWitnessV1,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Batch {
        pub proof: Option<[u8; 128]>, pub inputs: Vec<Input>,
        pub output_tree: u8, pub output_queue: u8,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Place {
        pub nonce: [u8; 32], pub quote_bound: QuoteBound,
        /// Owner consent for canonical owner/mint expiry-settlement delegation.
        pub settlement_delegate: bool,
        pub classic_quote_amount: u64,
        pub legs: Vec<Leg>, pub merkle_accounts: u8, pub batches: Vec<Batch>,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Fill {
        pub nonce: [u8; 32], pub quote_delta: i128,
        pub merkle_accounts: u8, pub batches: Vec<Batch>,
    }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Exit { pub nonce: [u8; 32], pub merkle_accounts: u8, pub batches: Vec<Batch> }
}
crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct ConsolidateWriterCash {
        pub expected_hash: [u8;32],
        pub merkle_accounts: u8,
        pub batch: Batch,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Place(Place),
    Fill(Fill),
    Cancel(Exit),
    Expire(Exit),
    PrepareRouter(crate::market_router::Prepare),
    FillFromOptionRoutes(crate::atomic_option_route::Fill),
    UploadProof(crate::atomic_proof::Upload),
    ConsolidateWriterCash(ConsolidateWriterCash),
    PrepareProjection(crate::atomic_projection::Action),
    ReclaimProof,
}
impl BorshSerialize for Action {
    fn serialize<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        match self {
            Self::Place(v) => {
                18u8.serialize(w)?;
                v.serialize(w)
            }
            Self::Fill(v) => {
                19u8.serialize(w)?;
                v.serialize(w)
            }
            Self::Cancel(v) => {
                20u8.serialize(w)?;
                v.serialize(w)
            }
            Self::Expire(v) => {
                21u8.serialize(w)?;
                v.serialize(w)
            }
            Self::PrepareRouter(v) => {
                23u8.serialize(w)?;
                v.serialize(w)
            }
            Self::FillFromOptionRoutes(v) => {
                24u8.serialize(w)?;
                v.serialize(w)
            }
            Self::UploadProof(v) => {
                25u8.serialize(w)?;
                v.serialize(w)
            }
            Self::ConsolidateWriterCash(v) => {
                26u8.serialize(w)?;
                v.serialize(w)
            }
            Self::PrepareProjection(v) => {
                27u8.serialize(w)?;
                v.serialize(w)
            }
            Self::ReclaimProof => 28u8.serialize(w),
        }
    }
}
impl Action {
    pub(crate) fn from_reader<R: std::io::Read>(tag: u8, r: &mut R) -> std::io::Result<Self> {
        match tag {
            18 => Ok(Self::Place(Place::deserialize_reader(r)?)),
            19 => Ok(Self::Fill(Fill::deserialize_reader(r)?)),
            20 => Ok(Self::Cancel(Exit::deserialize_reader(r)?)),
            21 => Ok(Self::Expire(Exit::deserialize_reader(r)?)),
            23 => Ok(Self::PrepareRouter(
                crate::market_router::Prepare::deserialize_reader(r)?,
            )),
            24 => Ok(Self::FillFromOptionRoutes(
                crate::atomic_option_route::Fill::deserialize_reader(r)?,
            )),
            25 => Ok(Self::UploadProof(
                crate::atomic_proof::Upload::deserialize_reader(r)?,
            )),
            26 => Ok(Self::ConsolidateWriterCash(
                ConsolidateWriterCash::deserialize_reader(r)?,
            )),
            27 => Ok(Self::PrepareProjection(
                crate::atomic_projection::Action::deserialize_reader(r)?,
            )),
            28 => Ok(Self::ReclaimProof),
            _ => Err(std::io::ErrorKind::InvalidData.into()),
        }
    }
    pub(crate) fn from_cursor(tag: u8, c: &mut CheckedCursor<'_>) -> Self {
        match tag {
            18 => Self::Place(Place::read(c)),
            19 => Self::Fill(Fill::read(c)),
            20 => Self::Cancel(Exit::read(c)),
            21 => Self::Expire(Exit::read(c)),
            23 => Self::PrepareRouter(crate::market_router::Prepare::read(c)),
            24 => Self::FillFromOptionRoutes(crate::atomic_option_route::Fill::read(c)),
            25 => Self::UploadProof(crate::atomic_proof::Upload::read(c)),
            26 => Self::ConsolidateWriterCash(ConsolidateWriterCash::read(c)),
            27 => Self::PrepareProjection(crate::atomic_projection::Action::read(c)),
            28 => Self::ReclaimProof,
            _ => {
                c.invalid = true;
                Self::Cancel(Exit {
                    nonce: [0; 32],
                    merkle_accounts: 0,
                    batches: vec![],
                })
            }
        }
    }
}
impl BorshDeserialize for Action {
    fn deserialize_reader<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        Self::from_reader(u8::deserialize_reader(r)?, r)
    }
    fn deserialize(data: &mut &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_deserialize(data)
    }
    fn try_from_slice(data: &[u8]) -> std::io::Result<Self> {
        crate::fixed_codec::cursor_from_slice(data)
    }
}
impl CursorField for Action {
    fn read(c: &mut CheckedCursor<'_>) -> Self {
        Self::from_cursor(c.u8(), c)
    }
}

crate::fixed_codec::compact_borsh_struct! {
    #[derive(Clone, Debug, Eq, PartialEq, BorshSerialize)]
    pub struct Order {
        pub magic: [u8;8], pub version: u8, pub bump: u8, pub status: u8,
        pub owner: Pubkey, pub custody_owner: Pubkey, pub nonce: [u8;32], pub quote_mint: Pubkey,
        pub quote_bound: QuoteBound, pub settlement_delegate: bool,
        pub expiry_ts: u64,
        /// Actual authenticated owner quote change, populated only on full fill.
        pub filled_quote_delta: i128, pub filled_slot: u64,
        pub legs: Vec<BoundLeg>,
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Asset {
    pub market: Pubkey,
    pub mint: Pubkey,
    pub delta: i128,
}

pub fn derive(program: &Pubkey, owner: &Pubkey, nonce: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::constants::CURRENT_STATE_NAMESPACE_SEED,
            SEED,
            owner.as_ref(),
            nonce,
        ],
        program,
    )
}
impl Order {
    /// Authenticate the existing expiry consent for one exact positive filled
    /// asset. Account ownership is checked by the caller before decoding.
    /// The terminal Order PDA is the common capability; it never owns tokens.
    pub fn settlement_scope(
        &self,
        program: &Pubkey,
        key: &Pubkey,
        custody: &Pubkey,
        market: &Pubkey,
        mint: &Pubkey,
        amount: u64,
    ) -> Option<u64> {
        let (expected, bump) = derive(program, &self.owner, &self.nonce);
        if *key != expected
            || self.bump != bump
            || self.magic != MAGIC
            || self.version != VERSION
            || self.status != FILLED
            || !self.settlement_delegate
            || self.custody_owner != *custody
            || self.custody_owner != self.owner
                && self.custody_owner != crate::trading_session::derive(program, &self.owner).0
            || self.expiry_ts != self.legs.iter().map(|l| l.expiry_ts).min()?
            || amount == 0
        {
            return None;
        }
        let net = assets(&self.legs)?;
        let asset = net
            .iter()
            .find(|x| x.market == *market && x.mint == *mint)?;
        if asset.delta <= 0 || i128::from(amount) > asset.delta {
            return None;
        }
        let expiry = self
            .legs
            .iter()
            .find(|l| l.market == *market && l.mint == *mint)?
            .expiry_ts;
        self.legs
            .iter()
            .filter(|l| l.market == *market && l.mint == *mint)
            .all(|l| l.expiry_ts == expiry)
            .then_some(expiry)
    }
}
/// Checked final owner asset changes; repeated buy/sell legs may net against
/// each other. Their canonical identities and positive quantities are retained.
pub fn assets(legs: &[BoundLeg]) -> Option<Vec<Asset>> {
    if legs.is_empty() {
        return None;
    }
    let mut result: Vec<Asset> = Vec::new();
    for leg in legs {
        if leg.side > 1
            || leg.quantity == 0
            || crate::pubkey_is_default(&leg.market)
            || crate::pubkey_is_default(&leg.mint)
        {
            return None;
        }
        let delta = if leg.side == 0 {
            i128::from(leg.quantity)
        } else {
            -i128::from(leg.quantity)
        };
        if let Some(a) = result.iter_mut().find(|a| a.mint == leg.mint) {
            if a.market != leg.market {
                return None;
            }
            a.delta = a.delta.checked_add(delta)?;
        } else {
            result.push(Asset {
                market: leg.market,
                mint: leg.mint,
                delta,
            });
        }
    }
    for a in &result {
        u64::try_from(a.delta.unsigned_abs()).ok()?;
    }
    Some(result)
}
