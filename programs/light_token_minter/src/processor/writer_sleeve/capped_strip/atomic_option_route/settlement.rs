//! Typed financial packing with native-derived outputs and compressed carries.
use super::*;
use crate::regular_compressed_transfer::{self as transfer, HotCompression, InputLeaf, OutputLeaf};
use solana_program::instruction::AccountMeta;
mod queue_index;

/// The short fingerprint only avoids unequal-key comparisons. Equal keys are
/// still compared in full, including when two outer roles alias one account.
fn key_fingerprint(key: &Pubkey) -> u64 {
    let k = key.as_ref();
    u64::from_le_bytes([k[0], k[1], k[2], k[3], k[4], k[5], k[6], k[7]])
}
/// Read the actual canonical output queue immediately before the CPI. The
/// subsequently consumed bridge is still authenticated/nullified by Light's
/// queue inclusion-by-index path, including when other inputs use aged proofs.
fn next_output_index(queue: &AccountInfo, tree: &Pubkey) -> Result<u64, ProgramError> {
    queue_index::read(queue, tree)
}
struct RolePacking<'a> {
    keys: &'a [&'a Pubkey],
    fingerprints: &'a [u64],
    positions: Vec<Option<u8>>,
    roles: Vec<(usize, bool)>,
}

#[derive(Clone)]
struct FinalOutput {
    owner: usize,
    amount: u64,
    asset: usize,
    delegate: Option<usize>,
}
#[derive(Clone, Copy)]
struct Carry {
    asset: usize,
    amount: u64,
    leaf_index: u32,
    tree: u8,
    queue: u8,
}

/// Financial terms emitted only by the authenticated whole-order projector.
/// Settlement does not need the quote search grid or the complete source books.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
pub(super) struct SettlementContext {
    pub(super) cash: cash_custody::CompressedCustodyV1,
    pub(super) hot_before: u64,
    pub(super) cold_before: u64,
    pub(super) quote_delta: i128,
    pub(super) required_cash: u64,
    pub(super) sleeve_group: Pubkey,
    pub(super) sleeve_bump: u8,
}
impl SettlementContext {
    pub(super) fn from_projected(c: &Context) -> Result<Self, ProgramError> {
        Ok(Self {
            cash: c.cash.clone(),
            hot_before: c.hot_before,
            cold_before: c.cold_before,
            quote_delta: c.quote_delta,
            required_cash: checked(
                c.state
                    .sleeve
                    .accounted_asset_atoms
                    .checked_sub(c.policy.total_pool_quote_atoms),
            )?,
            sleeve_group: c.state.sleeve.settlement_group,
            sleeve_bump: c.state.sleeve.bump,
        })
    }
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
pub(super) struct SettlementAsset {
    pub(super) market_id: [u8; 32],
    pub(super) market_bump: u8,
    pub(super) mint_before: u64,
    pub(super) staging_before: u64,
    pub(super) fresh: u64,
    pub(super) retired: u64,
    pub(super) market_option_delta: i128,
    pub(super) market_quote_delta: i128,
}
impl SettlementAsset {
    pub(super) fn from_projected(x: &Asset) -> Result<Self, ProgramError> {
        Ok(Self {
            market_id: x.market.market_id,
            market_bump: x.market.bump,
            mint_before: x.mint_before,
            staging_before: x.staging_before,
            fresh: x.fresh,
            retired: x.retired,
            market_option_delta: i128::from(x.resident.total_option()?)
                - i128::from(x.option_before),
            market_quote_delta: i128::from(x.resident.total_quote()?) - i128::from(x.quote_before),
        })
    }
}

fn market_signer_seeds<'a>(asset: &'a SettlementAsset, bump: &'a [u8; 1]) -> [&'a [u8]; 4] {
    [
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::MARKET_PDA_SEED,
        &asset.market_id,
        bump,
    ]
}
fn validate_output_masks(batches: &[wire::Batch], count: usize) -> ProgramResult {
    if wire::output_masks_valid(batches, count) {
        Ok(())
    } else {
        Err(invalid())
    }
}
fn selected(mask: &[u8], index: usize) -> bool {
    mask[index / 8] & (1 << (index % 8)) != 0
}
impl<'a> RolePacking<'a> {
    fn new(keys: &'a [&'a Pubkey], fingerprints: &'a [u64]) -> Self {
        Self {
            keys,
            fingerprints,
            positions: vec![None; keys.len()],
            roles: vec![
                (9, false),
                (2, true),
                (6, false),
                (10, false),
                (11, false),
                (12, false),
                (8, false),
            ],
        }
    }
    fn same(&self, left: usize, right: usize) -> bool {
        self.fingerprints[left] == self.fingerprints[right] && self.keys[left] == self.keys[right]
    }
    fn pack(&mut self, role: usize, signer: bool) -> Result<u8, ProgramError> {
        if let Some(index) = self.positions[role] {
            self.roles[7 + usize::from(index)].1 |= signer;
            return Ok(index);
        }
        let index = if let Some(i) = self.roles[7..]
            .iter()
            .position(|(r, _)| self.same(*r, role))
        {
            self.roles[i + 7].1 |= signer;
            i
        } else {
            let i = self.roles.len() - 7;
            // Check before caching so 255 remains a valid packed u8 index.
            u8::try_from(i).map_err(|_| invalid())?;
            self.roles.push((role, signer));
            i
        };
        let index = u8::try_from(index).map_err(|_| invalid())?;
        self.positions[role] = Some(index);
        Ok(index)
    }
    fn signed(&self, role: usize) -> bool {
        if let Some(index) = self.positions[role] {
            if self.roles[7 + usize::from(index)].1 {
                return true;
            }
            // Fixed Light roles intentionally remain outside the packed tail.
            return self.roles[..7]
                .iter()
                .any(|(r, s)| *s && self.same(*r, role));
        }
        self.roles.iter().any(|(r, s)| *s && self.same(*r, role))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn settle(
    program: &Pubkey,
    a: &[AccountInfo],
    p: &wire::Fill,
    order: &mo::Order,
    net: &[mo::Asset],
    inputs: &Inputs,
    contexts: &mut [SettlementContext],
    assets: &[SettlementAsset],
    asset_start: usize,
    delegate_start: usize,
    custody_start: usize,
    merkle_start: usize,
    quote_delta: i128,
    fee: u64,
    delegation: Option<&[u8]>,
) -> ProgramResult {
    let close = delegation.is_some();
    let owner_quote = add_signed(inputs.escrow[0], quote_delta)?
        .checked_sub(fee)
        .ok_or_else(invalid)?;
    let mut used_delegates = vec![false; usize::from(p.delegate_accounts)];
    for (i, leg) in p.legs.iter().enumerate() {
        let scope_used = delegation.is_some_and(|flags| {
            p.batches
                .iter()
                .flat_map(|b| &b.inputs)
                .zip(flags)
                .any(|(input, kind)| input.asset == i as u8 + 1 && *kind == 2)
        });
        if scope_used {
            let index = usize::from(leg.delegate_index);
            if index >= used_delegates.len()
                || *a[delegate_start + index].key
                    != crate::scoped_settlement::derive_collective_settlement_delegate(
                        program,
                        &order.custody_owner,
                        &net[i].mint,
                    )
                    .0
            {
                return Err(invalid());
            }
            used_delegates[index] = true;
        } else if leg.delegate_index != 255 {
            return Err(invalid());
        }
    }
    if used_delegates.iter().any(|v| !*v) {
        return Err(invalid());
    }
    let mut used_custody = vec![false; usize::from(p.custody_accounts)];
    for leg in &p.legs {
        if leg.retirement_index != 255 {
            used_custody[usize::from(leg.retirement_index)] = true;
        }
    }
    if used_custody.iter().any(|v| !*v) {
        return Err(invalid());
    }
    let market_options = assets
        .iter()
        .enumerate()
        .map(|(i, x)| add_signed(inputs.markets[i][1], x.market_option_delta))
        .collect::<Result<Vec<_>, ProgramError>>()?;
    let market_quotes = assets
        .iter()
        .enumerate()
        .map(|(i, x)| add_signed(inputs.markets[i][0], x.market_quote_delta))
        .collect::<Result<Vec<_>, ProgramError>>()?;
    let mut hot_draws = Vec::with_capacity(contexts.len());
    let mut cash_outputs = Vec::with_capacity(contexts.len());
    for (i, c) in contexts.iter_mut().enumerate() {
        let n = context_role(i);
        let selected = i128::from(inputs.cash[i])
            .checked_add(c.quote_delta)
            .ok_or_else(invalid)?;
        let hot_draw = if selected < 0 {
            u64::try_from(selected.unsigned_abs()).map_err(|_| invalid())?
        } else {
            0
        };
        if hot_draw > c.hot_before {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        let output = add_signed(inputs.cash[i], c.quote_delta + i128::from(hot_draw))?;
        c.cash.quote_atoms = add_signed(c.cold_before, c.quote_delta + i128::from(hot_draw))?;
        let physical = checked(
            c.hot_before
                .checked_sub(hot_draw)
                .and_then(|v| v.checked_add(c.cash.quote_atoms)),
        )?;
        if physical < c.required_cash {
            return Err(VaultError::WriterSolvencyViolation.into());
        }
        if a[n + 9].owner != program && c.cash.quote_atoms != 0 {
            let kind = [CustodyKind::WriterCash as u8];
            let bump = [c.cash.bump];
            invoke_create_or_allocate_account(
                &a[2],
                &a[n + 9],
                &a[8],
                program,
                cash_custody::CompressedCustodyV1::ACCOUNT_LEN,
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    cash_custody::COMPRESSED_CUSTODY_SEED,
                    &kind,
                    a[n + 8].key.as_ref(),
                    &bump,
                ],
            )?;
        }
        hot_draws.push(hot_draw);
        cash_outputs.push(output);
    }
    let cash_interface_bump = if hot_draws.iter().any(|v| *v != 0) {
        validate_spl_interface_account_with_bump(a[4].key, &a[13])?.1
    } else {
        0
    };
    let mut interface_bumps = vec![0; assets.len()];
    for (i, x) in assets.iter().enumerate() {
        if x.fresh == 0 {
            continue;
        }
        let n = asset_start + 4 * i;
        interface_bumps[i] = validate_spl_interface_account_with_bump(a[n + 1].key, &a[n + 3])?.1;
        // Initial asset loading already authenticated this exact staging PDA,
        // mint and authority. No intervening source projection can write token
        // accounts; retain that binding and re-read its physical balance.
        let staged = if a[n + 2].owner == a[7].key {
            validate_token_account(&a[n + 2])?
        } else {
            custody::load_or_create_market_staging_checked(
                program,
                &a[2],
                &a[n],
                &a[n + 2],
                &a[n + 1],
                &a[7],
                &a[8],
            )?
        };
        if staged.amount != x.staging_before {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        let bump = [x.market_bump];
        let seeds = market_signer_seeds(x, &bump);
        invoke_token_mint_to_checked(&a[7], &a[n + 1], &a[n + 2], &a[n], x.fresh, 6, &[&seeds])?;
    }
    let order_bump = [order.bump];
    let order_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        mo::SEED,
        order.owner.as_ref(),
        &order.nonce,
        &order_bump,
    ];
    let session_bump = [if close && order.custody_owner != order.owner {
        crate::trading_session::derive(program, &order.owner).1
    } else {
        0
    }];
    let session_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::trading_session::SEED,
        order.owner.as_ref(),
        &session_bump,
    ];
    let market_bumps = assets.iter().map(|x| [x.market_bump]).collect::<Vec<_>>();
    let market_seeds = assets
        .iter()
        .zip(&market_bumps)
        .map(|(x, b)| market_signer_seeds(x, b))
        .collect::<Vec<_>>();
    let cash_kind = [CustodyKind::WriterCash as u8];
    let cash_bumps = contexts.iter().map(|x| [x.cash.bump]).collect::<Vec<_>>();
    let cash_seeds = contexts
        .iter()
        .enumerate()
        .map(|(i, _)| {
            vec![
                CURRENT_STATE_NAMESPACE_SEED,
                cash_custody::COMPRESSED_CUSTODY_SEED,
                &cash_kind[..],
                a[context_role(i) + 8].key.as_ref(),
                &cash_bumps[i][..],
            ]
        })
        .collect::<Vec<_>>();
    let sleeve_bumps = contexts.iter().map(|x| [x.sleeve_bump]).collect::<Vec<_>>();
    let sleeve_seeds = contexts
        .iter()
        .zip(&sleeve_bumps)
        .map(|(c, b)| writer_sleeve_signer_seeds(&c.sleeve_group, b))
        .collect::<Vec<_>>();
    let mut final_outputs = Vec::<FinalOutput>::new();
    let mut final_out = |owner, amount, asset, delegate| {
        if amount != 0 {
            final_outputs.push(FinalOutput {
                owner,
                amount,
                asset,
                delegate,
            });
        }
    };
    final_out(0, owner_quote, 0, None);
    final_out(2, fee, 0, None);
    for (i, amount) in market_quotes.iter().copied().enumerate() {
        final_out(asset_start + 4 * i, amount, 0, None);
    }
    for (i, amount) in cash_outputs.iter().copied().enumerate() {
        final_out(context_role(i) + 9, amount, 0, None);
    }
    for i in 0..assets.len() {
        let quantity = u64::try_from(net[i].delta.unsigned_abs()).map_err(|_| invalid())?;
        let buy = !close && net[i].delta > 0;
        if buy && order.settlement_delegate {
            final_out(0, quantity, i + 1, Some(1));
            final_out(0, inputs.escrow[i + 1], i + 1, None);
        } else {
            let owned = if buy {
                checked(inputs.escrow[i + 1].checked_add(quantity))?
            } else {
                inputs.escrow[i + 1]
                    .checked_sub(quantity)
                    .ok_or_else(invalid)?
            };
            final_out(0, owned, i + 1, None);
        }
        final_out(asset_start + 4 * i, market_options[i], i + 1, None);
        if assets[i].retired != 0 {
            final_out(
                context_role(usize::from(p.legs[i].context_index)) + 1,
                assets[i].retired,
                i + 1,
                None,
            );
        }
    }
    validate_output_masks(&p.batches, final_outputs.len())?;
    let role_keys = a.iter().map(|info| info.key).collect::<Vec<_>>();
    let fingerprints = role_keys
        .iter()
        .map(|key| key_fingerprint(key))
        .collect::<Vec<_>>();
    let mint_role = |asset: usize| {
        if asset == 0 {
            4
        } else {
            asset_start + 4 * (asset - 1) + 1
        }
    };
    let mut input_offset = 0usize;
    let mut carries = Vec::<Carry>::new();
    let mut compressed = vec![false; assets.len() + 1];
    for (b, batch) in p.batches.iter().enumerate() {
        if batch.inputs.len() > wire::MAX_BATCH_INPUTS {
            return Err(invalid());
        }
        let mut touched = vec![false; assets.len() + 1];
        for input in &batch.inputs {
            touched[usize::from(input.asset)] = true;
        }
        for (i, out) in final_outputs.iter().enumerate() {
            if selected(&batch.output_mask, i) {
                touched[out.asset] = true;
            }
        }
        // A zero-mask, carry-only batch can combine fragments without any
        // caller-selected value movement. It still consumes real Light leaves.
        if batch.inputs.is_empty() && !touched.iter().any(|v| *v) {
            for carry in &carries {
                touched[carry.asset] = true;
            }
        }
        let mut packing = RolePacking::new(&role_keys, &fingerprints);
        let merkle = (merkle_start..a.len())
            .map(|r| packing.pack(r, false))
            .collect::<Result<Vec<_>, _>>()?;
        let mut leaves = Vec::new();
        let mut outputs = Vec::new();
        let mut compressions = Vec::new();
        let mut balances = vec![0u128; assets.len() + 1];
        for (j, input) in batch.inputs.iter().enumerate() {
            let asset = usize::from(input.asset);
            let w = &input.witness;
            let owner_role = match input.source {
                wire::Source::Escrow => {
                    if close {
                        0
                    } else {
                        1
                    }
                }
                wire::Source::Market(i) => asset_start + 4 * usize::from(i),
                wire::Source::WriterCash(i) => context_role(usize::from(i)) + 9,
            };
            let delegate_role = match delegation.map_or(0, |flags| flags[input_offset + j]) {
                0 => None,
                1 => Some(1),
                2 => Some(delegate_start + usize::from(p.legs[asset - 1].delegate_index)),
                _ => return Err(invalid()),
            };
            let delegate = delegate_role.map(|r| packing.pack(r, false)).transpose()?;
            leaves.push(InputLeaf {
                owner: packing.pack(owner_role, true)?,
                amount: input.amount,
                mint: packing.pack(mint_role(asset), false)?,
                has_delegate: delegate.is_some(),
                delegate: delegate.unwrap_or(0),
                tree: merkle[usize::from(w.tree_index)],
                queue: merkle[usize::from(w.queue_index)],
                leaf_index: w.leaf_index,
                root_index: w.root_index,
                prove_by_index: w.prove_by_index,
            });
            balances[asset] = balances[asset]
                .checked_add(u128::from(input.amount))
                .ok_or_else(invalid)?;
        }
        // A carry is an exact native-derived compressed leaf, not a caller
        // witness. Unrelated mints remain in the output queue until needed.
        let mut retained = Vec::new();
        for carry in carries.drain(..) {
            if !touched[carry.asset] || leaves.len() == wire::MAX_BATCH_INPUTS {
                retained.push(carry);
                continue;
            }
            leaves.push(InputLeaf {
                owner: packing.pack(1, true)?,
                amount: carry.amount,
                mint: packing.pack(mint_role(carry.asset), false)?,
                has_delegate: false,
                delegate: 0,
                tree: merkle[usize::from(carry.tree)],
                queue: merkle[usize::from(carry.queue)],
                leaf_index: carry.leaf_index,
                root_index: 0,
                prove_by_index: true,
            });
            balances[carry.asset] = balances[carry.asset]
                .checked_add(u128::from(carry.amount))
                .ok_or_else(invalid)?;
        }
        for (asset, used) in touched.iter().copied().enumerate() {
            if !used || compressed[asset] {
                continue;
            }
            let mint = packing.pack(mint_role(asset), false)?;
            if asset == 0 {
                for (i, draw) in hot_draws.iter().copied().enumerate() {
                    if draw == 0 {
                        continue;
                    }
                    compressions.push(HotCompression {
                        amount: draw,
                        mint,
                        source: packing.pack(context_role(i) + 8, false)?,
                        authority: packing.pack(context_role(i), true)?,
                        pool_account_index: packing.pack(13, false)?,
                        pool_index: 0,
                        bump: cash_interface_bump,
                        decimals: 6,
                    });
                    balances[0] = balances[0]
                        .checked_add(u128::from(draw))
                        .ok_or_else(invalid)?;
                }
            } else {
                let i = asset - 1;
                let amount = assets[i].fresh;
                if amount != 0 {
                    // Light compresses the classic-SPL staging balance through the SPL
                    // Token program, which its CPI can invoke only when it is passed.
                    packing.pack(7, false)?;
                    compressions.push(HotCompression {
                        amount,
                        mint,
                        source: packing.pack(asset_start + 4 * i + 2, false)?,
                        authority: packing.pack(asset_start + 4 * i, true)?,
                        pool_account_index: packing.pack(asset_start + 4 * i + 3, false)?,
                        pool_index: 0,
                        bump: interface_bumps[i],
                        decimals: 6,
                    });
                    balances[asset] = balances[asset]
                        .checked_add(u128::from(amount))
                        .ok_or_else(invalid)?;
                }
            }
            compressed[asset] = true;
        }
        for (i, out) in final_outputs.iter().enumerate() {
            if !selected(&batch.output_mask, i) {
                continue;
            }
            balances[out.asset] = balances[out.asset]
                .checked_sub(u128::from(out.amount))
                .ok_or_else(invalid)?;
            let delegate = out.delegate.map(|r| packing.pack(r, false)).transpose()?;
            outputs.push(OutputLeaf {
                owner: packing.pack(out.owner, false)?,
                amount: out.amount,
                mint: packing.pack(mint_role(out.asset), false)?,
                has_delegate: delegate.is_some(),
                delegate: delegate.unwrap_or(0),
            });
        }
        let residual = balances.iter().any(|n| *n != 0);
        if b + 1 == p.batches.len() && (residual || !retained.is_empty()) {
            return Err(invalid());
        }
        if residual {
            let next = next_output_index(
                &a[merkle_start + usize::from(batch.output_queue)],
                a[merkle_start + usize::from(batch.output_tree)].key,
            )?;
            for (asset, mut balance) in balances.into_iter().enumerate() {
                while balance != 0 {
                    let amount = balance.min(u128::from(u64::MAX)) as u64;
                    let leaf_index = next
                        .checked_add(outputs.len() as u64)
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(invalid)?;
                    outputs.push(OutputLeaf {
                        owner: packing.pack(1, false)?,
                        amount,
                        mint: packing.pack(mint_role(asset), false)?,
                        has_delegate: false,
                        delegate: 0,
                    });
                    retained.push(Carry {
                        asset,
                        amount,
                        leaf_index,
                        tree: batch.output_tree,
                        queue: batch.output_queue,
                    });
                    balance -= u128::from(amount);
                }
            }
        }
        // These are the measured limits of the pinned Transfer2/v2 tree
        // programs, not eligibility limits on assets, sources or fragments.
        if leaves.len() > wire::MAX_BATCH_INPUTS || outputs.len() > wire::MAX_BATCH_OUTPUTS {
            return Err(invalid());
        }
        // The loaded token program reports MintCacheCapacityExceeded (6144)
        // when a CPI uses more than five distinct mints, including hot actions.
        if !mint_cache_fits(
            leaves
                .iter()
                .map(|l| l.mint)
                .chain(outputs.iter().map(|o| o.mint))
                .chain(compressions.iter().map(|c| c.mint)),
        ) {
            return Err(invalid());
        }
        input_offset += batch.inputs.len();
        if leaves.is_empty() && compressions.is_empty() && outputs.is_empty() {
            return Err(invalid());
        }
        let roles = &packing.roles;
        let metas = roles
            .iter()
            .map(|(r, s)| AccountMeta {
                pubkey: *a[*r].key,
                is_signer: *s,
                is_writable: a[*r].is_writable,
            })
            .collect();
        let instruction = transfer::instruction_with_compressions(
            *a[5].key,
            metas,
            merkle[usize::from(batch.output_queue)],
            batch.proof,
            &leaves,
            &compressions,
            &outputs,
        )?;
        let mut infos = roles.iter().map(|(r, _)| a[*r].clone()).collect::<Vec<_>>();
        infos.push(a[5].clone());
        let mut signers = Vec::new();
        if packing.signed(1) {
            signers.push(order_seeds);
        }
        if close && order.custody_owner != order.owner && packing.signed(0) {
            signers.push(session_seeds);
        }
        for (i, seeds) in market_seeds.iter().enumerate() {
            if packing.signed(asset_start + 4 * i) {
                signers.push(seeds.as_slice());
            }
        }
        for (i, seeds) in cash_seeds.iter().enumerate() {
            if packing.signed(context_role(i) + 9) {
                signers.push(seeds.as_slice());
            }
        }
        for (i, seeds) in sleeve_seeds.iter().enumerate() {
            if packing.signed(context_role(i)) {
                signers.push(seeds.as_slice());
            }
        }
        invoke_signed(&instruction, &infos, &signers)?;
        carries = retained;
    }
    if assets
        .iter()
        .enumerate()
        .any(|(i, x)| x.fresh != 0 && !compressed[i + 1])
        || hot_draws.iter().any(|n| *n != 0) && !compressed[0]
        || !carries.is_empty()
    {
        return Err(invalid());
    }
    for (i, x) in assets.iter().enumerate() {
        let n = asset_start + 4 * i;
        let mint = validate_mint_account(&a[n + 1], a[7].key)?;
        if mint.supply != checked(x.mint_before.checked_add(x.fresh))? {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
        if x.fresh != 0 {
            if validate_token_account(&a[n + 2])?.amount != x.staging_before {
                return Err(VaultError::WriterSupplyMismatch.into());
            }
            if x.staging_before == 0 {
                let bump = [x.market_bump];
                let seeds = market_signer_seeds(x, &bump);
                invoke_token_close_account(&a[7], &a[n + 2], &a[2], &a[n], &[&seeds])?;
            }
        }
    }
    for (i, c) in contexts.iter().enumerate() {
        if validate_token_account(&a[context_role(i) + 8])?.amount != c.hot_before - hot_draws[i] {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
    }
    let _ = custody_start;
    Ok(())
}
fn mint_cache_fits(mints: impl Iterator<Item = u8>) -> bool {
    let mut bits = [0u8; 32];
    let mut count = 0usize;
    for mint in mints {
        let byte = usize::from(mint) / 8;
        let bit = 1 << (mint % 8);
        if bits[byte] & bit == 0 {
            bits[byte] |= bit;
            count += 1;
            if count > wire::MAX_BATCH_MINTS {
                return false;
            }
        }
    }
    true
}
