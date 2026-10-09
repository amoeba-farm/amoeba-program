//! A permissionless native projector seals exact source-bound settlement terms.
//! Only this projector emits deltas; no instruction accepts caller-supplied runs.
use super::*;
use crate::atomic_projection::{self as cache, delta};
use crate::compact_error::CompactAccountInfo;
use crate::fixed_codec::FixedStateEncode;
mod codec;
mod context_codec;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::hash::hash;
mod size_log;

#[derive(BorshSerialize, BorshDeserialize)]
struct Plan {
    time: cache::TimeBounds,
    contexts: Vec<settlement::SettlementContext>,
    assets: Vec<settlement::SettlementAsset>,
    quote_delta: i128,
    fee: u64,
    asset_quote: Vec<i128>,
    deltas: Vec<delta::Delta>,
}
fn fill(action: &cache::ProjectionAction) -> &wire::Fill {
    match action {
        cache::ProjectionAction::Fill(p) => p,
        cache::ProjectionAction::Close(p) => &p.route,
    }
}
fn proof_role(p: &wire::Fill) -> usize {
    wire::COMMON
        + usize::from(p.contexts) * wire::CONTEXT_ACCOUNTS
        + 4 * p.legs.len()
        + usize::from(p.delegate_accounts)
        + usize::from(p.custody_accounts)
}
fn close_terms(action: &cache::ProjectionAction) -> Option<(u64, &[u8], bool)> {
    match action {
        cache::ProjectionAction::Close(p) => {
            Some((p.minimum_net_quote, &p.delegated_inputs, p.pay_fee_in_usdc))
        }
        _ => None,
    }
}
fn load_cache(
    program: &Pubkey,
    a: &[AccountInfo],
    p: &wire::Fill,
) -> Result<(usize, cache::Header), ProgramError> {
    let role = proof_role(p);
    if p.proof_accounts != 1
        || role >= a.len()
        || a[role].owner != program
        || a[role].executable
        || a[role].is_signer
        || !a[role].is_writable
    {
        return Err(invalid());
    }
    let h = cache::decode_program_owned_header(program, &a[role])?;
    if h.payer != *a[2].key
        || h.identity.order != *a[1].key
        || h.proof_count as usize != p.batches.len()
    {
        return Err(invalid());
    }
    Ok((role, h))
}
pub(super) fn is_cache(a: &[AccountInfo], p: &wire::Fill) -> Result<bool, ProgramError> {
    let role = proof_role(p);
    if p.proof_accounts != 1 || role >= a.len() {
        return Ok(false);
    }
    Ok(a[role].try_data()?.get(..8) == Some(cache::MAGIC.as_slice()))
}
fn cache_proofs(
    h: &cache::Header,
    data: &[u8],
    p: &wire::Fill,
) -> Result<Vec<[u8; 128]>, ProgramError> {
    let mut proofs = Vec::with_capacity(p.batches.len());
    for (i, b) in p.batches.iter().enumerate() {
        let proof = cache::proof_at(h, data, i)?;
        if data[cache::HEADER_LEN + i / 8] & (1 << (i % 8)) == 0 {
            return Err(invalid());
        }
        let present = b.proof.is_some() || b.proof_account != crate::atomic_proof::INLINE;
        if !present && proof != [0; 128]
            || b.proof.is_some_and(|inline| inline != proof)
            || b.proof_account != crate::atomic_proof::INLINE && b.proof_account != 0
        {
            return Err(invalid());
        }
        proofs.push(proof);
    }
    if cache::proofs_hash(&proofs)? != h.identity.proofs_hash {
        return Err(invalid());
    }
    Ok(proofs)
}
fn month_bounds(now: u64) -> Result<(u64, u64), ProgramError> {
    let (year, month, day, seconds) = utc_calendar_parts(now)?;
    let start = now
        .checked_sub(u64::from(day - 1) * 86_400 + seconds)
        .ok_or_else(invalid)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return Err(invalid()),
    };
    Ok((start, start.checked_add(days * 86_400).ok_or_else(invalid)?))
}
fn time_bounds(
    program: &Pubkey,
    a: &[AccountInfo],
    projected: &ProjectedAction,
    action: &cache::ProjectionAction,
) -> Result<cache::TimeBounds, ProgramError> {
    let now = current_unix_timestamp()?;
    let mut valid_from = 0;
    let mut valid_until = u64::MAX;
    for (i, asset) in projected.assets.iter().enumerate() {
        let leg = &projected.p.legs[i];
        let listing = if leg.context_index == wire::NO_CONTEXT {
            load_oracle_month_state(&a[projected.asset_start + 4 * i + 2], program)?.listing_ts
        } else {
            projected.contexts[usize::from(leg.context_index)]
                .anchor_month
                .as_ref()
                .ok_or_else(invalid)?
                .listing_ts
        };
        valid_from = valid_from.max(listing);
        valid_until = valid_until.min(asset.market.instrument.expiry_ts);
    }
    let month_boundary = if projected.contexts.is_empty() {
        None
    } else {
        let (start, end) = month_bounds(now)?;
        valid_from = valid_from.max(start);
        Some(end)
    };
    let close_deadline = match action {
        cache::ProjectionAction::Close(p) => Some(p.deadline_ts),
        _ => {
            valid_until = valid_until.min(projected.order.expiry_ts);
            None
        }
    };
    let time = cache::TimeBounds {
        prepared_slot: projected.slot,
        valid_from,
        valid_until,
        close_deadline,
        month_boundary,
    };
    if !time.admits(now) {
        return Err(invalid());
    }
    Ok(time)
}
// Canonical fixed codecs establish these Borsh offsets. Differential tests
// compare them to zero/MAX serialization; slot values are never searched.
const POOL_SLOT: usize = 324;
const PAGE_SLOT: usize = 562;
const POSITION_SLOT: usize = 160;
const SLEEVE_SLOT: usize = 505;
const BOOK_SLOT: usize = 112;
fn asset_slots(x: &Asset) -> Result<Vec<u32>, ProgramError> {
    let mut slots = vec![];
    if !x.resident_present {
        return Ok(slots);
    }
    let mut offset = x.market.maximum_encoded_len() + 13;
    if x.normal_updated {
        slots.push((offset + POOL_SLOT) as u32);
    }
    offset += x.resident.pool.maximum_encoded_len() + 1;
    if let Some(book) = &x.resident.book {
        offset += book.maximum_encoded_len();
    }
    offset += 4;
    for record in &x.resident.records {
        offset += record.maximum_encoded_len();
    }
    offset += 4;
    for page in &x.resident.bin_pages {
        if x.touched_pages.contains(&page.page_index) {
            slots.push((offset + PAGE_SLOT) as u32);
        }
        offset += page.maximum_encoded_len();
    }
    offset += 1;
    if x.writer_updated {
        if x.resident.writer_position.is_none() {
            return Err(invalid());
        }
        slots.push((offset + POSITION_SLOT) as u32);
    }
    Ok(slots)
}
fn emit<F>(
    a: &[AccountInfo],
    role: usize,
    used: Option<usize>,
    slots: Vec<u32>,
    scratch: &mut Vec<u8>,
    serialize: F,
) -> Result<delta::Delta, ProgramError>
where
    F: FnOnce(&mut Vec<u8>) -> ProgramResult,
{
    scratch.clear();
    serialize(scratch)?;
    finish_emit(a, role, used, slots, scratch)
}
// Share source bounds and diffing across serializers rather than duplicating
// this body into each closure instantiation of the permissionless projector.
#[inline(never)]
fn finish_emit(
    a: &[AccountInfo],
    role: usize,
    used: Option<usize>,
    slots: Vec<u32>,
    scratch: &[u8],
) -> Result<delta::Delta, ProgramError> {
    let before = a[role].try_data()?;
    // Routing changes existing rows, never membership or encoded payload size.
    if scratch.len() != used.unwrap_or(before.len()) || scratch.len() > before.len() {
        return Err(invalid());
    }
    let result =
        delta::diff_allocated(role, &before[..scratch.len()], scratch, before.len(), slots)?;
    Ok(result)
}
fn serialize_asset(x: &Asset, out: &mut Vec<u8>) -> ProgramResult {
    fixed(x.market.as_ref(), out)?;
    if x.resident_present {
        out.extend_from_slice(&crate::market_router_account::MAGIC);
        out.push(crate::market_router_account::VERSION);
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        let body = out.len();
        resident_bytes(&x.resident, out)?;
        let size = u32::try_from(out.len() - body).map_err(|_| invalid())?;
        out[start..start + 4].copy_from_slice(&size.to_le_bytes());
    }
    Ok(())
}
fn fixed<T: FixedStateEncode + ?Sized>(state: &T, out: &mut Vec<u8>) -> ProgramResult {
    let start = out.len();
    let end = start
        .checked_add(state.maximum_encoded_len())
        .ok_or_else(invalid)?;
    out.resize(end, 0);
    state.encode_fixed(&mut out[start..end]);
    Ok(())
}
fn resident_bytes(state: &router::ResidentRouterState, out: &mut Vec<u8>) -> ProgramResult {
    fixed(&state.pool, out)?;
    out.push(u8::from(state.book.is_some()));
    if let Some(book) = &state.book {
        fixed(book, out)?;
    }
    out.extend_from_slice(
        &u32::try_from(state.records.len())
            .map_err(|_| invalid())?
            .to_le_bytes(),
    );
    for record in &state.records {
        fixed(record, out)?;
    }
    out.extend_from_slice(
        &u32::try_from(state.bin_pages.len())
            .map_err(|_| invalid())?
            .to_le_bytes(),
    );
    for page in &state.bin_pages {
        fixed(page, out)?;
    }
    out.push(u8::from(state.writer_position.is_some()));
    if let Some(position) = &state.writer_position {
        fixed(position, out)?;
    }
    for amount in [
        state.pool_option,
        state.pool_quote,
        state.book_option,
        state.book_quote,
        state.pool_hot_option,
        state.pool_hot_quote,
        state.book_hot_option,
        state.book_hot_quote,
    ] {
        out.extend_from_slice(&amount.to_le_bytes());
    }
    Ok(())
}
fn make_plan(
    program: &Pubkey,
    a: &[AccountInfo],
    mut x: ProjectedAction,
    action: &cache::ProjectionAction,
) -> Result<Plan, ProgramError> {
    let time = time_bounds(program, a, &x, action)?;
    let contexts = x
        .contexts
        .iter()
        .map(settlement::SettlementContext::from_projected)
        .collect::<Result<Vec<_>, _>>()?;
    let assets = x
        .assets
        .iter()
        .map(settlement::SettlementAsset::from_projected)
        .collect::<Result<Vec<_>, _>>()?;
    let asset_quote = x.assets.iter().map(|a| a.quote_delta).collect();
    let mut deltas = vec![];
    let mut high = vec![];
    for (i, asset) in x.assets.iter_mut().enumerate() {
        let role = x.asset_start + 4 * i;
        // Project has not written or invoked CPI against these source accounts.
        // Reuse the strict envelope bound authenticated by its private loader.
        let used = asset.source_used_len;
        let slots = asset_slots(asset)?;
        deltas.push(emit(a, role, Some(used), slots, &mut high, |out| {
            serialize_asset(asset, out)
        })?);
    }
    for (i, c) in x.contexts.iter_mut().enumerate() {
        let role = context_role(i);
        // The authenticated projectors only mutate rows selected by this
        // immutable action. Copy all other canonical source rows byte-for-byte;
        // full mutable headers/totals and selected rows still use native codecs.
        let mut selected: context_codec::Selected = [false; crate::capped_strip::SERIES];
        for (leg, asset) in x.p.legs.iter().zip(&x.assets) {
            if usize::from(leg.context_index) == i {
                *selected.get_mut(asset.series_index).ok_or_else(invalid)? = true;
            }
        }
        deltas.push(emit(
            a,
            role,
            None,
            if c.writer_updated {
                vec![SLEEVE_SLOT as u32]
            } else {
                vec![]
            },
            &mut high,
            |out| fixed(c.state.sleeve.as_ref(), out),
        )?);
        deltas.push(emit(
            a,
            role + 2,
            None,
            if c.writer_updated {
                vec![BOOK_SLOT as u32]
            } else {
                vec![]
            },
            &mut high,
            |out| {
                context_codec::encode_book(
                    c.state.book.as_ref(),
                    &a[role + 2].try_data()?,
                    &selected,
                    out,
                )
            },
        )?);
        deltas.push(emit(a, role + 4, None, vec![], &mut high, |out| {
            fixed(c.policy.as_ref(), out)
        })?);
        if let Some(lane) = &c.lane {
            deltas.push(emit(
                a,
                role + 6,
                None,
                if c.primary_updated {
                    vec![(crate::capped_strip::LEN - 8) as u32]
                } else {
                    vec![]
                },
                &mut high,
                |out| {
                    context_codec::encode_lane(
                        lane.as_ref(),
                        &a[role + 6].try_data()?,
                        &selected,
                        out,
                    )
                },
            )?);
        }
    }
    let quote_delta = x.quote_delta;
    let fee = x.fee;
    // SBF's invocation-local bump allocator never reclaims memory. This exact
    // ProjectedAction owns only native values, Boxes and Vecs; no AccountInfo,
    // reference-counted state or borrow guards. Avoid walking the now-consumed
    // large source graph solely to issue no-op deallocations. Review this site
    // if a field with observable Drop behavior is added. Hosts drop normally.
    #[cfg(target_os = "solana")]
    core::mem::forget(x);
    Ok(Plan {
        time,
        contexts,
        assets,
        quote_delta,
        fee,
        asset_quote,
        deltas,
    })
}
pub(in crate::processor) fn prepare(
    program: &Pubkey,
    a: &[AccountInfo],
    action: cache::ProjectionAction,
) -> ProgramResult {
    let p = fill(&action);
    if a.len() < wire::COMMON {
        return Err(invalid());
    }
    let (role, mut header) = load_cache(program, a, p)?;
    if a.iter()
        .any(|info| info.is_writable && info.key != a[2].key && info.key != a[role].key)
        || header.status == cache::CONSUMED
        || header.identity.action_hash != cache::action_hash(&action)?
        || header.identity.source_hash != cache::source_hash(program, a, p)?
    {
        return Err(invalid());
    }
    if header.status == cache::SEALED {
        return if cache::decode_time_bounds(&header, &a[role].try_data()?)?
            .admits(current_unix_timestamp()?)
        {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    let proofs = cache_proofs(&header, &a[role].try_data()?, p)?;
    if let cache::ProjectionAction::Close(close) = &action {
        if current_unix_timestamp()? > close.deadline_ts {
            return Err(invalid());
        }
        let order =
            crate::processor::multi_order::load_readonly(program, &a[1], a[0].key, &p.nonce)?;
        if mo::assets(&order.legs)
            .ok_or_else(invalid)?
            .iter()
            .any(|v| v.delta <= 0)
        {
            return Err(invalid());
        }
    }
    let projected = project(
        program,
        a,
        p.clone(),
        close_terms(&action),
        true,
        Some(&proofs),
    )?;
    let plan = make_plan(program, a, projected, &action)?;
    let bytes = codec::encode(&plan)?;
    // As above, Plan contains only plain native values and owned buffers. Its
    // complete checked encoding is retained; the SBF heap dies at invocation
    // return, so nested buffer deallocation has no effect on memory lifetime.
    #[cfg(target_os = "solana")]
    core::mem::forget(plan);
    let required = header
        .payload_start()?
        .checked_add(bytes.len())
        .ok_or_else(invalid)?;
    if required > a[role].data_len() {
        size_log::log(b"atomic_projection_required_bytes: ", required as u64);
        return Err(ProgramError::AccountDataTooSmall);
    }
    header.status = cache::SEALED;
    header.payload_bytes = u32::try_from(bytes.len()).map_err(|_| invalid())?;
    header.payload_hash = hash(&bytes).to_bytes();
    let mut data = a[role].try_data_mut()?;
    data[header.payload_start()?..required].copy_from_slice(&bytes);
    header
        .serialize(&mut &mut data[..cache::HEADER_LEN])
        .map_err(|_| invalid())?;
    size_log::log(b"atomic_projection_sealed_bytes: ", required as u64);
    Ok(())
}
pub(super) fn execute(
    program: &Pubkey,
    a: &[AccountInfo],
    action: cache::ProjectionAction,
) -> ProgramResult {
    let p = fill(&action);
    if a.len() < wire::COMMON || !a[2].is_signer || !a[2].is_writable || a[2].executable {
        return Err(invalid());
    }
    let (role, header) = load_cache(program, a, p)?;
    if header.status != cache::SEALED
        || header.identity.action_hash != cache::action_hash(&action)?
        || header.identity.source_hash != cache::source_hash(program, a, p)?
    {
        return Err(invalid());
    }
    let mut plan = {
        let data = a[role].try_data()?;
        codec::decode(&data[header.payload_start()?..header.used_bytes()?])?
    };
    if !plan.time.admits(current_unix_timestamp()?) {
        return Err(invalid());
    }
    let mut order = crate::processor::multi_order::load(program, &a[1], a[0].key, &p.nonce)?;
    let close = close_terms(&action);
    if order.status
        != if close.is_some() {
            mo::FILLED
        } else {
            mo::OPEN
        }
    {
        return Err(invalid());
    }
    let net = mo::assets(&order.legs).ok_or_else(invalid)?;
    if plan.contexts.len() != usize::from(p.contexts)
        || plan.assets.len() != net.len()
        || plan.asset_quote.len() != net.len()
    {
        return Err(invalid());
    }
    let asset_start = wire::COMMON + usize::from(p.contexts) * wire::CONTEXT_ACCOUNTS;
    let delegate_start = asset_start + 4 * net.len();
    let custody_start = delegate_start + usize::from(p.delegate_accounts);
    let merkle_start = role + 1;
    let mut allowed = vec![false; a.len()];
    for i in 0..net.len() {
        allowed[asset_start + 4 * i] = true;
    }
    for i in 0..net.len() {
        let n = asset_start + 4 * i;
        if !a[n].is_writable || !a[n + 1].is_writable {
            return Err(invalid());
        }
    }
    for c in 0..usize::from(p.contexts) {
        let n = context_role(c);
        if [0, 2, 4, 8, 9].iter().any(|i| !a[n + i].is_writable) {
            return Err(invalid());
        }
        for offset in [0, 2, 4, 6] {
            allowed[n + offset] = true;
        }
    }
    let mut seen = vec![false; a.len()];
    for d in &plan.deltas {
        let n = usize::from(d.role);
        if n >= a.len()
            || !allowed[n]
            || seen[n]
            || a[n].owner != program
            || !a[n].is_writable
            || a[n].executable
        {
            return Err(invalid());
        }
        seen[n] = true;
        delta::validate(d, a[n].data_len())?;
    }
    if a[merkle_start..]
        .iter()
        .any(|a| !a.is_writable || a.is_signer || a.executable)
    {
        return Err(invalid());
    }
    let mut resolved = p.clone();
    let proofs = cache_proofs(&header, &a[role].try_data()?, p)?;
    for (i, b) in resolved.batches.iter_mut().enumerate() {
        if b.proof_account != crate::atomic_proof::INLINE {
            b.proof = Some(proofs[i]);
            b.proof_account = crate::atomic_proof::INLINE;
        }
    }
    let inputs = parse_inputs(&resolved, net.len())?;
    let slot = crate::compact_error::slot()?;
    settlement::settle(
        program,
        a,
        &resolved,
        &order,
        &net,
        &inputs,
        &mut plan.contexts,
        &plan.assets,
        asset_start,
        delegate_start,
        custody_start,
        merkle_start,
        plan.quote_delta,
        plan.fee,
        close.map(|(_, flags, _)| flags),
    )?;
    for d in &plan.deltas {
        delta::apply(d, &mut a[usize::from(d.role)].try_data_mut()?, slot)?;
    }
    for (i, c) in plan.contexts.iter().enumerate() {
        let role = context_role(i) + 9;
        if a[role].owner == program {
            cash_custody::store(&a[role], &c.cash)?;
        } else if c.cash.quote_atoms != 0 {
            return Err(invalid());
        }
    }
    if close.is_none() {
        order.status = mo::FILLED;
        order.filled_quote_delta = plan.quote_delta;
        order.filled_slot = slot;
        crate::processor::multi_order::store(&a[1], &order)?;
    }
    let event = wire::Receipt {
        owner: order.owner,
        custody_owner: order.custody_owner,
        order: *a[1].key,
        close: close.is_some(),
        slot,
        quote_delta: plan.quote_delta,
        sponsor_fee: plan.fee,
        assets: net
            .iter()
            .enumerate()
            .map(|(i, n)| wire::AssetReceipt {
                market: n.market,
                mint: n.mint,
                option_delta: if close.is_some() { -n.delta } else { n.delta },
                quote_delta: plan.asset_quote[i],
            })
            .collect(),
    };
    solana_program::log::sol_log_data(&[
        b"ATOPTFIL",
        &borsh::to_vec(&event).map_err(|_| invalid())?,
    ]);
    // The authenticated final payer is the rent owner recorded at Reserve.
    // Refund only after every financial CPI and write; any late failure rolls
    // back both settlement and this credit. The absent cache prevents replay.
    close_program_account(program, &a[role], &a[2])
}
