//! Exact Borsh bytes for native sealed plans without nested generic IO writers.
//! The derived codec remains an independent host-test oracle.
use super::{delta, settlement, Plan};
use crate::compressed_custody::CompressedCustodyV1;
use crate::fixed_codec::{CheckedCursor, CursorField};
use crate::ProgramError;

fn rows<T>(input: &mut CheckedCursor<'_>, read: fn(&mut CheckedCursor<'_>) -> T) -> Vec<T> {
    let count = input.u32() as usize;
    // Bound the initial allocation, not the accepted number of native rows.
    let capacity = count.min((4096 / core::mem::size_of::<T>().max(1)).max(1));
    let mut result = Vec::with_capacity(capacity);
    for _ in 0..count {
        let row = read(input);
        if input.invalid {
            break;
        }
        result.push(row);
    }
    result
}

fn context(input: &mut CheckedCursor<'_>) -> settlement::SettlementContext {
    settlement::SettlementContext {
        cash: CompressedCustodyV1::read(input),
        hot_before: input.u64(),
        cold_before: input.u64(),
        quote_delta: i128::read(input),
        required_cash: input.u64(),
        sleeve_group: input.pubkey(),
        sleeve_bump: input.u8(),
    }
}

fn asset(input: &mut CheckedCursor<'_>) -> settlement::SettlementAsset {
    settlement::SettlementAsset {
        market_id: input.bytes(),
        market_bump: input.u8(),
        mint_before: input.u64(),
        staging_before: input.u64(),
        fresh: input.u64(),
        retired: input.u64(),
        market_option_delta: i128::read(input),
        market_quote_delta: i128::read(input),
    }
}

fn read_run(input: &mut CheckedCursor<'_>) -> delta::Run {
    delta::Run {
        offset: input.u32(),
        bytes: Vec::<u8>::read(input),
    }
}

fn delta(input: &mut CheckedCursor<'_>) -> delta::Delta {
    delta::Delta {
        role: input.u8(),
        data_len: input.u32(),
        runs: rows(input, read_run),
        slots: Vec::<u32>::read(input),
    }
}

pub(super) fn decode(data: &[u8]) -> Result<Plan, ProgramError> {
    let mut input = CheckedCursor::new(data);
    let plan = Plan {
        time: CursorField::read(&mut input),
        contexts: rows(&mut input, context),
        assets: rows(&mut input, asset),
        quote_delta: i128::read(&mut input),
        fee: input.u64(),
        asset_quote: rows(&mut input, i128::read),
        deltas: rows(&mut input, delta),
    };
    input.finish_exact().map_err(|_| invalid())?;
    Ok(plan)
}
fn invalid() -> ProgramError {
    ProgramError::InvalidAccountData
}

fn count(value: usize) -> Result<u32, ProgramError> {
    u32::try_from(value).map_err(|_| invalid())
}

fn add(size: &mut usize, bytes: usize) -> Result<(), ProgramError> {
    *size = size.checked_add(bytes).ok_or_else(invalid)?;
    Ok(())
}

fn add_rows(size: &mut usize, rows: usize, stride: usize) -> Result<(), ProgramError> {
    count(rows)?;
    add(size, 4)?;
    add(size, rows.checked_mul(stride).ok_or_else(invalid)?)
}

fn encoded_len(plan: &Plan) -> Result<usize, ProgramError> {
    let mut size = 24 + 1 + 1;
    if plan.time.close_deadline.is_some() {
        add(&mut size, 8)?;
    }
    if plan.time.month_boundary.is_some() {
        add(&mut size, 8)?;
    }
    add_rows(
        &mut size,
        plan.contexts.len(),
        CompressedCustodyV1::LEN + 73,
    )?;
    add_rows(&mut size, plan.assets.len(), 97)?;
    add(&mut size, 16 + 8)?;
    add_rows(&mut size, plan.asset_quote.len(), 16)?;
    add_rows(&mut size, plan.deltas.len(), 0)?;
    for delta in &plan.deltas {
        add(&mut size, 1 + 4)?;
        add_rows(&mut size, delta.runs.len(), 0)?;
        for run in &delta.runs {
            add(&mut size, 4)?;
            add_rows(&mut size, run.bytes.len(), 1)?;
        }
        add_rows(&mut size, delta.slots.len(), 4)?;
    }
    Ok(size)
}

// All writes remain bounds-checked even if a future schema edit misses the
// length calculation. One exact allocation also avoids Vec growth on SBF.
struct Writer<'a> {
    data: &'a mut [u8],
    offset: usize,
}
impl Writer<'_> {
    #[inline(always)]
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ProgramError> {
        let end = self.offset.checked_add(bytes.len()).ok_or_else(invalid)?;
        self.data
            .get_mut(self.offset..end)
            .ok_or_else(invalid)?
            .copy_from_slice(bytes);
        self.offset = end;
        Ok(())
    }
    #[inline(always)]
    fn u8(&mut self, value: u8) -> Result<(), ProgramError> {
        self.bytes(&[value])
    }
    #[inline(always)]
    fn u32(&mut self, value: u32) -> Result<(), ProgramError> {
        self.bytes(&value.to_le_bytes())
    }
    #[inline(always)]
    fn u64(&mut self, value: u64) -> Result<(), ProgramError> {
        self.bytes(&value.to_le_bytes())
    }
    #[inline(always)]
    fn i128(&mut self, value: i128) -> Result<(), ProgramError> {
        self.bytes(&value.to_le_bytes())
    }
    #[inline(always)]
    fn count(&mut self, value: usize) -> Result<(), ProgramError> {
        self.u32(count(value)?)
    }
    fn option(&mut self, value: Option<u64>) -> Result<(), ProgramError> {
        self.u8(u8::from(value.is_some()))?;
        if let Some(value) = value {
            self.u64(value)?;
        }
        Ok(())
    }
}

pub(super) fn encode(plan: &Plan) -> Result<Vec<u8>, ProgramError> {
    let mut bytes = vec![0; encoded_len(plan)?];
    let mut out = Writer {
        data: &mut bytes,
        offset: 0,
    };
    out.u64(plan.time.prepared_slot)?;
    out.u64(plan.time.valid_from)?;
    out.u64(plan.time.valid_until)?;
    out.option(plan.time.close_deadline)?;
    out.option(plan.time.month_boundary)?;
    out.count(plan.contexts.len())?;
    for context in &plan.contexts {
        let cash = &context.cash;
        out.u8(cash.version)?;
        out.u8(cash.bump)?;
        out.u8(match cash.kind {
            crate::compressed_custody::CustodyKind::Pool => 0,
            crate::compressed_custody::CustodyKind::OrderBook => 1,
            crate::compressed_custody::CustodyKind::WriterCash => 2,
        })?;
        out.bytes(cash.parent.as_ref())?;
        out.bytes(cash.option_mint.as_ref())?;
        out.bytes(cash.quote_mint.as_ref())?;
        out.u64(cash.option_atoms)?;
        out.u64(cash.quote_atoms)?;
        out.u64(context.hot_before)?;
        out.u64(context.cold_before)?;
        out.i128(context.quote_delta)?;
        out.u64(context.required_cash)?;
        out.bytes(context.sleeve_group.as_ref())?;
        out.u8(context.sleeve_bump)?;
    }
    out.count(plan.assets.len())?;
    for asset in &plan.assets {
        out.bytes(&asset.market_id)?;
        out.u8(asset.market_bump)?;
        out.u64(asset.mint_before)?;
        out.u64(asset.staging_before)?;
        out.u64(asset.fresh)?;
        out.u64(asset.retired)?;
        out.i128(asset.market_option_delta)?;
        out.i128(asset.market_quote_delta)?;
    }
    out.i128(plan.quote_delta)?;
    out.u64(plan.fee)?;
    out.count(plan.asset_quote.len())?;
    for amount in &plan.asset_quote {
        out.i128(*amount)?;
    }
    out.count(plan.deltas.len())?;
    for delta in &plan.deltas {
        out.u8(delta.role)?;
        out.u32(delta.data_len)?;
        out.count(delta.runs.len())?;
        for run in &delta.runs {
            out.u32(run.offset)?;
            out.count(run.bytes.len())?;
            out.bytes(&run.bytes)?;
        }
        out.count(delta.slots.len())?;
        for slot in &delta.slots {
            out.u32(*slot)?;
        }
    }
    if out.offset != out.data.len() {
        return Err(invalid());
    }
    Ok(bytes)
}
