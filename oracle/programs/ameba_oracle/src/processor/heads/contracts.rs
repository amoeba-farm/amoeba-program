//! Contracts: selectors 20 to 22, and 28 (a quarterly period). Accounts: the matching builders
//! in `instruction::heads`.
//!
//! The order of a contract's life, per component:
//!
//! 1. `ListContract` (20) freezes the board into the contract: items, mixes, sentences, the
//!    settlement target, the listing terms commitment and the sources live then with their
//!    openings. The accumulation window runs from the listing to the freeze, the start of the
//!    settlement window (`time::settlement_window_start` of the expiry): from then on the levels
//!    are being measured, so the weights stop moving.
//! 2. After expiry plus the challenge window, `SettleComponent` (21) records every source's
//!    level (and its level as of the freeze) and its weight, its head's own time-weighted
//!    delegation over the window, in order and in chunks; the call that completes the record
//!    settles the component exactly as `settle::settle_component` does: the weighted median of
//!    the eligible sources at their raw weights, no caps (the user, 2026-10-29), or the
//!    degraded path.
//! 3. `FinalizeContract` (22): per item the fixed-mix reference (every channel at its own
//!    settlement, live or last valid, at its fixed share), the index
//!    (`channels::settlement_change_ppm`) and its value; the period becomes Settled with the
//!    digest of the settlement record.

use super::accounts::{
    add, bad_list, bad_state, expect_key, load_head_of, overflow, roles, system, Res,
};
use super::transitions as t;
use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::instruction::heads::{
    CreateQuarterPeriodParams, HeadsListedMix, ListContractParams, SettleComponentParams,
};
use crate::state::heads::{
    find_heads_contract_address, find_heads_weights_address, heads_level_record,
    heads_listing_mix_commitment, heads_mix_commitment_field, heads_settlement_digest,
    load_heads_account, load_heads_weights_header, store_heads_account, zeroed_box, Head,
    HeadsBoard, HeadsChannel, HeadsContract, HeadsContractStatus, HeadsSettlementTarget,
    HeadsWeightSource, HeadsWeightsHeader, HeadsWeightsLayout, HeadsWeightsPhase, CAPTURE_JOINED,
    CAPTURE_RECORDED, COMPONENT_FREEZE_CHECKED, COMPONENT_FREEZE_PASSED, COMPONENT_FREEZE_RECORDED,
    COMPONENT_LAST_VALID, COMPONENT_LIVE, COMPONENT_SETTLED, DEGRADED_FREEZE_CHECK_FAILED,
    DEGRADED_NONE, DEGRADED_TOO_FEW_ORIGINS, DEGRADED_TOO_LITTLE_WEIGHT, HEADS_CHANNELS,
    HEADS_CONTRACT_RING, HEADS_CONTRACT_SEED, HEADS_MAX_COMPONENTS, HEADS_SETTLE_MAX_BATCH,
    HEADS_WEIGHTS_SEED, HEADS_WEIGHT_SOURCE_FLAGS_OFFSET, LIVE_DUPLICATE, LIVE_MEMBER,
    REFERENCE_ASSUMED, REFERENCE_AVAILABLE, REFERENCE_COMPUTED, RING_ACCUMULATING, RING_COLLECTING,
    RING_EMPTY, RING_FREEZE_CAPTURED, RING_SETTLED, WEIGHT_SOURCE_DUPLICATE,
    WEIGHT_SOURCE_ELIGIBLE, WEIGHT_SOURCE_FREEZE_LEVEL, WEIGHT_SOURCE_JOINED, WEIGHT_SOURCE_LEVEL,
    WEIGHT_SOURCE_LISTED, WEIGHT_SOURCE_RETIRED,
};
use ameba_oracle_heads::channels::{
    self, Channel, ChannelReading, ItemSettlement, Mix, MixBasis, Reference,
};
use ameba_oracle_heads::settle::{self, ComponentSettlement, Degraded, SourceInput};
use ameba_oracle_heads::{time, Params};

/// Source records a new weights account starts with, and what each growth adds (a growth stays
/// far inside the 10 KiB a single instruction may add to an account).
const INITIAL_SOURCES: usize = 4;
const GROW_SOURCES: usize = 8;

fn channel_of(byte: u8) -> Res<Channel> {
    Channel::from_code(byte).ok_or_else(bad_state)
}

fn basis_of(byte: u8) -> Res<MixBasis> {
    MixBasis::from_code(byte).ok_or_else(bad_state)
}

fn target_of(target: HeadsSettlementTarget) -> channels::SettlementTarget {
    match target {
        HeadsSettlementTarget::ContractChannel => channels::SettlementTarget::ContractChannel,
        HeadsSettlementTarget::FixedMixReference => channels::SettlementTarget::FixedMixReference,
    }
}

/// Item `item`'s mix from a `[u16; 24]` laid out at `item * 3 + channel`.
fn mix_at(mixes: &[u16], item: usize) -> Mix {
    core::array::from_fn(|ch| mixes[item * HEADS_CHANNELS + ch])
}

/// The contract bound to its board, its slot in the board's ring still holding it.
fn load_contract_of(
    program_id: &Pubkey,
    info: &AccountInfo,
    board_key: &Pubkey,
    board: &HeadsBoard,
) -> Res<Box<HeadsContract>> {
    let contract: Box<HeadsContract> = load_heads_account(program_id, info)?;
    let slot = usize::from(contract.ring_slot);
    if contract.board != *board_key
        || board.contract_periods[slot] != contract.period
        || !matches!(
            board.contract_status[slot],
            RING_ACCUMULATING | RING_COLLECTING
        )
    {
        return Err(OracleError::InvalidHeadsAccount.into());
    }
    Ok(contract)
}

// Selector 20
// ------------------------------------------------------------------------------------------

/// Selector 20, the oracle authority: lists a Forming period of the board's product that has no
/// v1 month. The contract freezes the items' mixes for its quarter (`params.items`, the
/// quarter holding its expiry's last second), which the sealed listing terms document must
/// commit to (`mixCommitment`, `heads_listing_mix_commitment`); the board's own mixes are the
/// default and display only. Every component's sentence is set and the basket weights add up to
/// 10,000; a fixed-mix target needs every item's mix valid on an assumed or matched basis
/// (`channels::listing_allowed`); each component the target settles on (`channels::settles`:
/// every channel its item's mix weights, or Contract alone) has `min_live_sources` independent
/// origins (`channels::component_lists`); a display component lists however few it has. The
/// period becomes Live from `max(now, planned listing)`, which is no later than the contract's
/// first sample instant (`samples::instants`).
#[inline(never)]
pub(super) fn list_contract(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: ListContractParams,
) -> ProgramResult {
    roles(accounts, b"WrRwrwwrr")?;
    let [payer_info, config_info, authority_info, period_info, month_info, board_info, contract_info, terms_info, system_info] =
        accounts
    else {
        return Err(bad_list());
    };
    system(system_info)?;
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    if anchor.period.status != PeriodStatus::Forming || anchor.period.effective_listing_ts != 0 {
        return Err(OracleError::InvalidPeriodTransition.into());
    }
    let mut board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    if anchor.underlying_id != board.product_id {
        return Err(OracleError::InvalidHeadsAccount.into());
    }
    let (month_key, _) = derive_oracle_month_pda(program_id, period_info.key, anchor.expiry_ts);
    validate_canonical_system_zero_pda_proof(&month_key, month_info)
        .map_err(|_| ProgramError::from(OracleError::InvalidOracleMonthAccount))?;
    crate::processor::oracle_evidence::sealed_evidence(
        program_id,
        terms_info,
        2,
        &params.terms_hash,
    )
    .map_err(|_| ProgramError::from(OracleError::InvalidHeadsEvidence))?;
    let (contract_key, contract_bump) = find_heads_contract_address(program_id, period_info.key);
    expect_key(contract_info, &contract_key)?;
    validate_create_only_program_account_target(program_id, contract_info)?;

    let now = t::now()?;
    let listing_ts = now.max(anchor.period.planned_listing_ts);
    let p = board.params.to_params();
    let freeze_ts = time::settlement_window_start(anchor.expiry_ts, p.settlement_business_days);
    let freeze_window = time::settlement_window_start(freeze_ts, p.settlement_business_days);
    // Listed no later than its first sample instant: each head's slot starts from its print at
    // the listing, which is its level at every instant from then on; listed later, today's print
    // would be dated back into samples already past (round 3 R8).
    let first_sample = super::samples::instants(freeze_window, freeze_ts, anchor.expiry_ts)[0];
    if params.base == 0 || listing_ts > first_sample || freeze_ts > anchor.expiry_ts {
        return Err(bad_state());
    }
    let items = usize::from(board.item_count);
    let components = usize::from(board.component_count);
    let weights: u64 = board.item_weight_bps[..items]
        .iter()
        .map(|w| u64::from(*w))
        .sum();
    if params.items.len() != items {
        return Err(bad_state());
    }
    let mixes = params
        .items
        .iter()
        .map(|m| Ok((m.mix_bps, basis_of(m.mix_basis as u8)?)))
        .collect::<Res<Vec<(Mix, MixBasis)>>>()?;
    let target = target_of(params.settlement_target);
    if weights != ameba_oracle_heads::BPS
        || components != items * HEADS_CHANNELS
        || board.component_sentence_hashes[..components]
            .iter()
            .any(crate::bytes32_is_zero)
        || !channels::listing_allowed(target, &mixes)
    {
        return Err(bad_state());
    }
    // The quarter's mixes are the ones the sealed listing terms document commits to.
    let (year, quarter) = time::contract_quarter(anchor.expiry_ts);
    let listed: Vec<_> = params.items.iter().map(HeadsListedMix::as_pair).collect();
    let field = heads_mix_commitment_field(&heads_listing_mix_commitment(
        &board.product_id,
        time::quarter_start(year, quarter),
        &board.item_bucket_ids[..items],
        &listed,
    ));
    if !terms_info
        .try_data()?
        .windows(field.len())
        .any(|window| window == field)
    {
        return Err(OracleError::InvalidHeadsEvidence.into());
    }
    let slot = (0..HEADS_CONTRACT_RING)
        .find(|&i| matches!(board.contract_status[i], RING_EMPTY | RING_SETTLED))
        .ok_or(OracleError::HeadsCapacityExhausted)?;

    create_program_account(
        payer_info,
        contract_info,
        system_info,
        program_id,
        HeadsContract::LEN,
        &[
            HEADS_CONTRACT_SEED,
            period_info.key.as_ref(),
            &[contract_bump],
        ],
    )?;
    // Built on the heap: the 4.6 KB account does not fit an SBF stack frame.
    let mut contract: Box<HeadsContract> = zeroed_box();
    contract.discriminator = HeadsContract::DISCRIMINATOR;
    contract.version = HeadsContract::VERSION;
    contract.bump = contract_bump;
    contract.period = *period_info.key;
    contract.board = *board_info.key;
    contract.product_id = board.product_id;
    contract.ring_slot = slot as u8;
    contract.status = HeadsContractStatus::Accumulating;
    contract.settlement_target = params.settlement_target;
    contract.params = board.params;
    contract.params_version = board.params_version;
    contract.base = params.base;
    contract.listing_ts = listing_ts;
    contract.freeze_ts = freeze_ts;
    contract.expiry_ts = anchor.expiry_ts;
    contract.terms_hash = params.terms_hash;
    contract.item_count = board.item_count;
    contract.item_bucket_ids = board.item_bucket_ids;
    contract.item_weight_bps = board.item_weight_bps;
    for (i, m) in params.items.iter().enumerate() {
        contract.item_mix_bps[i * HEADS_CHANNELS..(i + 1) * HEADS_CHANNELS]
            .copy_from_slice(&m.mix_bps);
        contract.item_mix_basis[i] = m.mix_basis as u8;
    }
    contract.component_count = board.component_count;
    contract.component_items = board.component_items;
    contract.component_channels = board.component_channels;
    contract.component_sentence_hashes = board.component_sentence_hashes;
    for c in 0..components {
        let channel = channel_of(board.component_channels[c])?;
        let (mix, _) = mixes
            .get(usize::from(board.component_items[c]))
            .ok_or_else(bad_state)?;
        // The live sources whose price is in effect at the listing: an LTA price past its reset
        // is no opening and no origin (round 4 N5).
        let (sources, origins) = board.listing_coverage(c, listing_ts);
        if !channels::component_lists(
            usize::from(origins),
            p.min_live_sources,
            channels::settles(target, channel, mix),
        ) {
            return Err(bad_state());
        }
        contract.component_listed_sources[c] = sources;
        contract.component_listed_origins[c] = origins;
    }
    let mut listed = 0;
    for e in 0..usize::from(board.live_count) {
        // The slot's membership is this contract's from now on: the listed sources only, every
        // live one with a price in effect, backed or not (round 10 review, N1: a dip in backing at
        // the listing never shuts an honest source out of the contract; only backed members
        // count as origins, here and at the freeze).
        board.live_flags[e] &= !(LIVE_MEMBER << slot);
        if !board.live_in_effect(e, listing_ts) {
            continue;
        }
        board.live_flags[e] |= LIVE_MEMBER << slot;
        contract.listed_head_ids[listed] = board.live_head_ids[e];
        contract.listed_components[listed] = board.live_components[e];
        contract.listed_flags[listed] = board.live_flags[e] & LIVE_DUPLICATE;
        contract.listed_openings[listed] = board.live_values[e];
        listed += 1;
    }
    contract.listed_count = listed as u16;
    contract.listed_by = *authority_info.key;
    contract.updated_ts = now;

    board.contract_periods[slot] = *period_info.key;
    board.contract_listing_ts[slot] = listing_ts;
    board.contract_freeze_ts[slot] = freeze_ts;
    board.contract_expiry_ts[slot] = anchor.expiry_ts;
    board.contract_freeze_window_ts[slot] = freeze_window;
    board.contract_cutoff_ts[slot] = add(anchor.expiry_ts, p.challenge_window_secs)?;
    board.contract_gens[slot] = board.contract_count.checked_add(1).ok_or_else(overflow)?;
    board.contract_status[slot] = RING_ACCUMULATING;
    board.contract_flags[slot] = 0;
    board.contract_freeze_origins[slot * HEADS_MAX_COMPONENTS..(slot + 1) * HEADS_MAX_COMPONENTS]
        .fill(0);
    board.contract_joined[slot * HEADS_MAX_COMPONENTS..(slot + 1) * HEADS_MAX_COMPONENTS].fill(0);
    board.contract_count = board.contract_count.checked_add(1).ok_or_else(overflow)?;
    board.updated_ts = now;

    write_period_live(program_id, period_info, &anchor, listing_ts)?;
    store_heads_account(contract_info, contract.as_ref())?;
    store_heads_account(board_info, board.as_ref())
}

// Selector 28
// ------------------------------------------------------------------------------------------

/// Selector 28, the oracle authority: a Forming period of the board's product expiring at a
/// calendar-quarter boundary (00:00 UTC on Jan 1, Apr 1, Jul 1 or Oct 1), independent of v1's
/// monthly ladder: the same Period PDA and consumer layout, six-decimal values. Its planned
/// listing is before the expiry and at or before the quarter's start or now, so the current
/// quarter can list at once.
#[inline(never)]
pub(super) fn create_quarter_period(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: CreateQuarterPeriodParams,
) -> ProgramResult {
    roles(accounts, b"WrRrwr")?;
    let [payer_info, config_info, authority_info, board_info, period_info, system_info] = accounts
    else {
        return Err(bad_list());
    };
    system(system_info)?;
    validate_current_oracle_authority_allow_paused(program_id, authority_info, config_info)?;
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let now = t::now()?;
    let (year, quarter) = time::contract_quarter(params.expiry_ts);
    if board.product_id != params.product_id
        || !time::is_quarter_boundary(params.expiry_ts)
        || params.expiry_ts <= now
        || params.planned_listing_ts >= params.expiry_ts
        || params.planned_listing_ts > time::quarter_start(year, quarter).max(now)
    {
        return Err(OracleError::InvalidPeriodSchedule.into());
    }
    let (key, bump) = ameba_oracle_interface::find_period_address(
        program_id,
        &params.product_id,
        params.expiry_ts,
    );
    expect_key(period_info, &key)?;
    let state = Period::new_forming(
        bump,
        params.product_id,
        params.planned_listing_ts,
        params.expiry_ts,
        PERIOD_VALUE_DECIMALS,
    );
    state
        .validate()
        .map_err(|_| OracleError::InvalidPeriodSchedule)?;
    create_new_program_account(
        payer_info,
        period_info,
        system_info,
        program_id,
        Period::LEN,
        &[
            ameba_oracle_interface::PERIOD_SEED,
            &params.product_id,
            &params.expiry_ts.to_le_bytes(),
            &[bump],
        ],
    )?;
    period_info
        .try_borrow_mut_data()?
        .copy_from_slice(&state.encode());
    Ok(())
}

// The weights account
// ------------------------------------------------------------------------------------------

/// One component's `HeadsWeights` account, its header decoded.
struct Weights<'i, 'a> {
    info: &'i AccountInfo<'a>,
    header: HeadsWeightsHeader,
    layout: HeadsWeightsLayout,
}

impl<'i, 'a> Weights<'i, 'a> {
    /// Loads the component's weights account, creating it (the payer funds it) on first use.
    #[allow(clippy::too_many_arguments)]
    fn open(
        program_id: &Pubkey,
        payer: &AccountInfo<'a>,
        info: &'i AccountInfo<'a>,
        system_info: &AccountInfo<'a>,
        contract_key: &Pubkey,
        contract: &HeadsContract,
        component: u8,
        now: u64,
    ) -> Res<Self> {
        let (key, bump) = find_heads_weights_address(program_id, contract_key, component);
        expect_key(info, &key)?;
        if info.owner == program_id {
            let (header, layout) = load_heads_weights_header(program_id, info)?;
            if header.contract != *contract_key || header.component != component {
                return Err(OracleError::InvalidHeadsAccount.into());
            }
            return Ok(Self {
                info,
                header,
                layout,
            });
        }
        system(system_info)?;
        let layout = HeadsWeightsLayout {
            source_capacity: INITIAL_SOURCES,
        };
        create_program_account(
            payer,
            info,
            system_info,
            program_id,
            layout.len(),
            &[
                HEADS_WEIGHTS_SEED,
                contract_key.as_ref(),
                &[component],
                &[bump],
            ],
        )?;
        let c = usize::from(component);
        let item = usize::from(contract.component_items[c]);
        let header = HeadsWeightsHeader {
            discriminator: HeadsWeightsHeader::DISCRIMINATOR,
            version: HeadsWeightsHeader::VERSION,
            bump,
            contract: *contract_key,
            board: contract.board,
            component,
            channel: HeadsChannel::from_byte(contract.component_channels[c])
                .ok_or_else(bad_state)?,
            phase: HeadsWeightsPhase::Recording,
            bucket_id: contract.item_bucket_ids[item],
            window_secs: contract
                .freeze_ts
                .checked_sub(contract.listing_ts)
                .ok_or_else(overflow)?,
            source_capacity: INITIAL_SOURCES as u16,
            updated_ts: now,
            ..HeadsWeightsHeader::default()
        };
        let weights = Self {
            info,
            header,
            layout,
        };
        weights.store_header()?;
        Ok(weights)
    }

    fn store_header(&self) -> ProgramResult {
        self.header.pack_account(&mut self.info.try_data_mut()?)
    }

    fn sources(&self) -> usize {
        usize::from(self.header.source_count)
    }

    fn source(&self, index: usize) -> Res<HeadsWeightSource> {
        HeadsWeightSource::unpack(&self.info.try_data()?[self.layout.source_range(index)])
    }

    fn set_source(&self, index: usize, source: &HeadsWeightSource) -> ProgramResult {
        source.pack(&mut self.info.try_data_mut()?[self.layout.source_range(index)])
    }

    /// Writes a record's flags byte alone (the rest of it unchanged).
    fn set_flags(&self, index: usize, flags: u8) -> ProgramResult {
        let at = self.layout.source_range(index).start + HEADS_WEIGHT_SOURCE_FLAGS_OFFSET;
        *self
            .info
            .try_data_mut()?
            .get_mut(at)
            .ok_or_else(bad_state)? = flags;
        Ok(())
    }

    /// The records that can count, as settlement inputs: those live at the cutoff (not retired by
    /// it), with their indices, and those live at the freeze (with a freeze level). Each is at most
    /// the component's members at its freeze: the listed sources, evicted ones included, and the
    /// joiners then live (each at most `HEADS_MAX_LIVE_SOURCES`), however many joined and left
    /// over the window; the others are read by their flags byte alone (round 3 R5). Inputs, not
    /// records, and exactly as many as count (round 13 review: up to 192 of each on the heap).
    #[allow(clippy::type_complexity)]
    fn counting(&self) -> Res<(Vec<usize>, Vec<SourceInput>, Vec<SourceInput>)> {
        let (mut lives, mut thens) = (0, 0);
        for i in 0..self.sources() {
            let f = self.flags(i)?;
            lives += usize::from(f & WEIGHT_SOURCE_RETIRED == 0);
            thens += usize::from(f & WEIGHT_SOURCE_FREEZE_LEVEL != 0);
        }
        let mut indices = Vec::with_capacity(lives);
        let mut live = Vec::with_capacity(lives);
        let mut at_freeze = Vec::with_capacity(thens);
        let data = self.info.try_data()?;
        for i in 0..self.sources() {
            let f = self.flags(i)?;
            let now = f & WEIGHT_SOURCE_RETIRED == 0;
            let then = f & WEIGHT_SOURCE_FREEZE_LEVEL != 0;
            if !now && !then {
                continue;
            }
            let range = self.layout.source_range(i);
            let source = HeadsWeightSource::unpack(data.get(range).ok_or_else(bad_state)?)?;
            if then {
                at_freeze.push(source_input(&source, true));
            }
            if now {
                indices.push(i);
                live.push(source_input(&source, false));
            }
        }
        Ok((indices, live, at_freeze))
    }

    /// A record's flags byte.
    fn flags(&self, index: usize) -> Res<u8> {
        let at = self.layout.source_range(index).start + HEADS_WEIGHT_SOURCE_FLAGS_OFFSET;
        self.info.try_data()?.get(at).copied().ok_or_else(bad_state)
    }

    /// Appends a source record for `head`, growing the account when it is full (the payer funds
    /// the rent): its opening is its value at listing when the contract listed it, else the value
    /// its fill set.
    fn append_source(
        &mut self,
        payer: &AccountInfo<'a>,
        system_info: &AccountInfo<'a>,
        contract: &HeadsContract,
        head: &Head,
    ) -> Res<usize> {
        if self.sources() >= self.layout.source_capacity {
            let next = HeadsWeightsLayout {
                source_capacity: self.layout.source_capacity + GROW_SOURCES,
            };
            let capacity = u16::try_from(next.source_capacity)
                .map_err(|_| ProgramError::from(OracleError::HeadsCapacityExhausted))?;
            system(system_info)?;
            let len = next.len();
            let rent = crate::compact_error::rent()?.minimum_balance(len);
            let lamports = self.info.lamports();
            if rent > lamports {
                invoke_system_transfer(payer, self.info, system_info, rent - lamports, &[])?;
            }
            self.info.resize(len)?;
            self.info.try_data_mut()?[self.layout.len()..len].fill(0);
            self.header.source_capacity = capacity;
            self.layout = next;
        }
        let listed = usize::from(contract.listed_count);
        let listed_at = (0..listed).find(|&i| {
            contract.listed_head_ids[i] == head.head_id
                && contract.listed_components[i] == head.component
        });
        let slot = usize::from(contract.ring_slot);
        let (flags, opening) = match listed_at {
            Some(i) => (WEIGHT_SOURCE_LISTED, contract.listed_openings[i]),
            // A member that went live inside the window, and nothing else.
            None if head.capture_periods[slot] == contract.period
                && head.capture_flags[slot] & CAPTURE_JOINED != 0 =>
            {
                (WEIGHT_SOURCE_JOINED, head.fill_value)
            }
            None => return Err(bad_state()),
        };
        let index = self.sources();
        if flags == WEIGHT_SOURCE_LISTED {
            self.header.listed_recorded = self
                .header
                .listed_recorded
                .checked_add(1)
                .ok_or_else(overflow)?;
        } else {
            self.header.joined_recorded = self
                .header
                .joined_recorded
                .checked_add(1)
                .ok_or_else(overflow)?;
        }
        self.set_source(
            index,
            &HeadsWeightSource {
                head_id: head.head_id,
                flags,
                origin_id: head.origin_id,
                live_at: head.live_ts,
                opening,
                ..HeadsWeightSource::default()
            },
        )?;
        self.header.source_count = self
            .header
            .source_count
            .checked_add(1)
            .ok_or_else(overflow)?;
        Ok(index)
    }
}

// Selector 21
// ------------------------------------------------------------------------------------------

/// Records one source's settlement level and its level as of the freeze (`samples::levels`: the
/// medians of the contract's fixed samples), whether it retired by the contract's cutoff K and was
/// live at the freeze, its origins, and its weight: the head's own window integral over the window
/// (no caps: the delegation behind it, time-weighted).
fn record_level(
    source: &mut HeadsWeightSource,
    contract: &HeadsContract,
    window_secs: u64,
    head: &Head,
) -> ProgramResult {
    let slot = usize::from(contract.ring_slot);
    // Synced (`settle_component`): a head untouched since the listing started its slot on the
    // contract now, waiting for every claim it revealed before (round 3 R2: no slot is not
    // "nothing pending").
    if head.capture_periods[slot] != contract.period {
        return Err(bad_state());
    }
    // One rule for eligibility, waiting and printing: not retired by the cutoff K (re-review N4).
    // The record is the one the contract indication computes (`heads_level_record`, round 14
    // review, N2).
    let record = heads_level_record(head, contract, source.opening);
    if (record.eligible || record.live_at_freeze) && super::samples::waits(head, slot) {
        // A claim the contract admits is still open (or one from before the slot started): it
        // finalizes first (it always can, past K), so the record, the fallback's freeze level
        // included, is the same whoever calls what when (re-review N2, B1).
        return Err(bad_state());
    }
    source.origin_id = record.origin;
    source.freeze_origin_id = record.freeze_origin;
    source.flags &= !(WEIGHT_SOURCE_LEVEL | WEIGHT_SOURCE_FREEZE_LEVEL | WEIGHT_SOURCE_RETIRED);
    // One eligibility cutoff for the whole contract, when settlement opens: a source retired by
    // then is out, a later retirement changes nothing, in whatever order the sources are
    // recorded.
    if !record.eligible {
        source.flags |= WEIGHT_SOURCE_RETIRED;
    }
    source.live_at = head.live_ts;
    source.level = record.level.unwrap_or(0);
    source.change_ppm = record.change_ppm.unwrap_or(0);
    if record.change_ppm.is_some() {
        source.flags |= WEIGHT_SOURCE_LEVEL;
    }
    source.freeze_level = record.freeze_level.unwrap_or(0);
    source.freeze_change_ppm = record.freeze_change_ppm.unwrap_or(0);
    if record.freeze_change_ppm.is_some() {
        source.flags |= WEIGHT_SOURCE_FREEZE_LEVEL;
    }
    source.integral = t::head_window_integral(head, contract)?;
    source.weight = time::average(source.integral, window_secs);
    Ok(())
}

fn source_input(s: &HeadsWeightSource, at_freeze: bool) -> SourceInput {
    let (live, change, origin) = if at_freeze {
        let has = s.flags & WEIGHT_SOURCE_FREEZE_LEVEL != 0;
        (has, has.then_some(s.freeze_change_ppm), s.freeze_origin_id)
    } else {
        (
            s.flags & WEIGHT_SOURCE_RETIRED == 0,
            (s.flags & WEIGHT_SOURCE_LEVEL != 0).then_some(s.change_ppm),
            s.origin_id,
        )
    };
    SourceInput {
        live,
        change_ppm: change,
        weight: s.weight,
        origin,
        live_at: s.live_at,
        head_id: s.head_id,
    }
}

/// Writes a component's settlement into the contract's record.
fn record_settlement(
    contract: &mut HeadsContract,
    weights: &mut Weights,
    component: usize,
    settlement: &ComponentSettlement,
    freeze_change: Option<i64>,
) -> ProgramResult {
    let mut flags = contract.component_flags[component] | COMPONENT_SETTLED;
    match settlement {
        ComponentSettlement::Live {
            change_ppm,
            eligible_sources,
            duplicate_sources,
            weight,
        } => {
            flags |= COMPONENT_LIVE;
            contract.component_change_ppm[component] = *change_ppm;
            contract.component_eligible[component] = *eligible_sources;
            contract.component_duplicates[component] = *duplicate_sources;
            contract.component_weight[component] = *weight;
            contract.component_degraded[component] = DEGRADED_NONE;
            weights.header.weight_total = *weight;
        }
        ComponentSettlement::LastValid { change_ppm, reason } => {
            flags |= COMPONENT_LAST_VALID;
            contract.component_change_ppm[component] = *change_ppm;
            contract.component_eligible[component] = weights.header.eligible_sources;
            contract.component_duplicates[component] = weights.header.duplicate_sources;
            contract.component_weight[component] = 0;
            contract.component_degraded[component] = match reason {
                Degraded::FreezeCheckFailed => DEGRADED_FREEZE_CHECK_FAILED,
                Degraded::TooFewOrigins => DEGRADED_TOO_FEW_ORIGINS,
                Degraded::TooLittleWeight => DEGRADED_TOO_LITTLE_WEIGHT,
            };
        }
    }
    if let Some(change) = freeze_change {
        flags |= COMPONENT_FREEZE_RECORDED;
        contract.component_freeze_change_ppm[component] = change;
    }
    contract.component_flags[component] = flags;
    contract.components_settled = contract
        .components_settled
        .checked_add(1)
        .ok_or_else(overflow)?;
    weights.header.phase = HeadsWeightsPhase::Settled;
    Ok(())
}

/// Settles a component once the record holds every source of it the contract listed and every
/// one that joined the window (until then the call only records, so a component with many
/// sources settles over several calls): `settle::settle_component` on the recorded levels and
/// weights of the sources that can count (`Weights::counting`: bounded by the live list). Its degraded path settles
/// at the freeze value (degraded rule 2, `docs/heads.md` decision 8: the component's change at
/// the freeze on the same weights with the levels then and the sources live then, when that
/// settles live; zero, the listing value, otherwise, and always after a failed freeze check).
/// The members a component's settlement records: the sources the contract listed of it, and
/// those that joined its window (`HeadsBoard::contract_joined`).
fn required(board: &HeadsBoard, contract: &HeadsContract, component: usize) -> (usize, u16) {
    let listed = (0..usize::from(contract.listed_count))
        .filter(|&i| usize::from(contract.listed_components[i]) == component)
        .count();
    let joined =
        board.contract_joined[usize::from(contract.ring_slot) * HEADS_MAX_COMPONENTS + component];
    (listed, joined)
}

fn settle_recorded(
    board: &HeadsBoard,
    contract: &mut HeadsContract,
    weights: &mut Weights,
    component: usize,
    p: &Params,
) -> ProgramResult {
    // Every source of it the contract listed, and every one that joined the window, retired or
    // not, each recorded once: the membership is fixed, so no caller chooses the record by
    // leaving one out. Counted as they are appended, so completeness reads no record.
    let (listed, members) = required(board, contract, component);
    if usize::from(weights.header.listed_recorded) != listed
        || weights.header.joined_recorded != members
    {
        return Ok(());
    }
    // Only the sources live at the cutoff can be eligible, and only those live at the freeze
    // have a freeze level: `settle_component` and `eligibility` give the others nothing, so they
    // are left out (round 3 R5: bounded by the live list, not by the window's turnover).
    let (indices, inputs, at_freeze) = weights.counting()?;
    let freeze_passed = contract.component_flags[component] & COMPONENT_FREEZE_PASSED != 0;
    let (settlement, eligible, duplicate) =
        settle::settle_recorded(p, &inputs, &at_freeze, freeze_passed);
    for (n, &i) in indices.iter().enumerate() {
        let mut flags = weights.flags(i)? & !(WEIGHT_SOURCE_ELIGIBLE | WEIGHT_SOURCE_DUPLICATE);
        if eligible[n] {
            flags |= WEIGHT_SOURCE_ELIGIBLE;
        }
        if duplicate[n] {
            flags |= WEIGHT_SOURCE_DUPLICATE;
        }
        weights.set_flags(i, flags)?;
    }
    weights.header.eligible_sources = eligible.iter().filter(|&&e| e).count() as u16;
    weights.header.duplicate_sources = duplicate.iter().filter(|&&d| d).count() as u16;
    let freeze_change = match settlement {
        ComponentSettlement::LastValid { change_ppm, .. } => Some(change_ppm),
        ComponentSettlement::Live { .. } => None,
    };
    record_settlement(contract, weights, component, &settlement, freeze_change)
}

/// Selector 21, after expiry plus the challenge window: records the levels of the component's
/// sources passed from `first_source` on, appending each to the record (the listed sources, and
/// any that went live in the window; each head is synced and marked recorded, so each source is
/// recorded once without reading the record), and settles the component on the call that records
/// the last one. A head with an open claim the contract may admit must wait for it; claims
/// committed after expiry are left out of the levels and never delay settlement.
#[inline(never)]
pub(super) fn settle_component(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SettleComponentParams,
) -> ProgramResult {
    if accounts.len() < 5 || accounts.len() > 5 + HEADS_SETTLE_MAX_BATCH {
        return Err(bad_list());
    }
    let (fixed, heads) = accounts.split_at(5);
    roles(fixed, b"Wrwwr")?;
    let [payer_info, board_info, contract_info, weights_info, system_info] = fixed else {
        return Err(bad_list());
    };
    system(system_info)?;
    let board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut contract = load_contract_of(program_id, contract_info, board_info.key, &board)?;
    let component = usize::from(params.component);
    let p = contract.params.to_params();
    let now = t::now()?;
    if contract.status == HeadsContractStatus::Settled
        || component >= usize::from(contract.component_count)
        || contract.component_flags[component] & COMPONENT_SETTLED != 0
        // Records only once the cutoff K has passed: everything at K is settled first.
        || now <= add(contract.expiry_ts, p.challenge_window_secs)?
    {
        return Err(bad_state());
    }
    if contract.component_flags[component] & COMPONENT_FREEZE_CHECKED == 0 {
        // The board captured its origins at the freeze if anything changed them since;
        // otherwise they are still what they were then.
        let slot = usize::from(contract.ring_slot);
        let origins = if board.contract_flags[slot] & RING_FREEZE_CAPTURED != 0 {
            board.contract_freeze_origins[slot * HEADS_MAX_COMPONENTS + component]
        } else {
            board.member_coverage(slot, component, contract.freeze_ts)
        };
        contract.component_freeze_origins[component] = origins;
        contract.component_flags[component] |= COMPONENT_FREEZE_CHECKED;
        if channels::origins_pass(usize::from(origins), p.min_live_sources) {
            contract.component_flags[component] |= COMPONENT_FREEZE_PASSED;
        }
    }
    let mut weights = Weights::open(
        program_id,
        payer_info,
        weights_info,
        system_info,
        contract_info.key,
        &contract,
        params.component,
        now,
    )?;
    if weights.header.phase != HeadsWeightsPhase::Recording
        || params.first_source != weights.header.levels_recorded
    {
        return Err(bad_state());
    }
    // A call records something, unless every member is recorded already (a component with no
    // member, say) and it settles (round 13 review, finding 2: an empty call that settles
    // nothing only stalls a keeper's progress).
    let (listed, joined) = required(&board, &contract, component);
    if heads.is_empty()
        && (usize::from(weights.header.listed_recorded) != listed
            || weights.header.joined_recorded != joined)
    {
        return Err(bad_list());
    }
    let window_secs = weights.header.window_secs;
    let slot = usize::from(contract.ring_slot);
    for (offset, head_info) in heads.iter().enumerate() {
        if !head_info.is_writable {
            return Err(bad_list());
        }
        let index = usize::from(params.first_source) + offset;
        let mut head = load_head_of(program_id, head_info, board_info.key)?;
        if usize::from(head.component) != component {
            return Err(bad_state());
        }
        // Brought up to now: its slot on the contract started (round 3 R2), its captures taken.
        t::sync_head(&board, &mut head, now)?;
        let index = if index < weights.sources() {
            if weights.source(index)?.head_id != head.head_id {
                return Err(bad_state());
            }
            index
        } else if head.capture_flags[slot] & CAPTURE_RECORDED != 0 {
            // Each source once, checked on its head: appending reads no record (round 3 R5).
            return Err(bad_state());
        } else {
            head.capture_flags[slot] |= CAPTURE_RECORDED;
            weights.append_source(payer_info, system_info, &contract, &head)?
        };
        let mut source = weights.source(index)?;
        record_level(&mut source, &contract, window_secs, &head)?;
        weights.set_source(index, &source)?;
        weights.header.levels_recorded += 1;
        head.updated_ts = now;
        store_heads_account(head_info, head.as_ref())?;
    }
    if weights.header.levels_recorded == weights.header.source_count {
        settle_recorded(&board, &mut contract, &mut weights, component, &p)?;
    }
    if contract.status == HeadsContractStatus::Accumulating {
        contract.status = HeadsContractStatus::Collecting;
    }
    contract.updated_ts = now;
    weights.header.updated_ts = now;
    weights.store_header()?;
    store_heads_account(contract_info, contract.as_ref())
}

// Selector 22
// ------------------------------------------------------------------------------------------

/// Selector 22: with every component settled, each item's fixed-mix reference
/// (`channels::reference`: every channel at its own settlement, a degraded one at its last valid
/// value and fixed share, recorded in `item_degraded_channels`; flagged when assumed), the index
/// on the contract's target (`channels::settlement_change_ppm`, the references or the Contract
/// components) and its value (`settle::settlement_value`); the period becomes Settled with the
/// digest of the settlement record (`state::heads::heads_settlement_digest`).
#[inline(never)]
pub(super) fn finalize_contract(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    roles(accounts, b"www")?;
    let [board_info, contract_info, period_info] = accounts else {
        return Err(bad_list());
    };
    let mut board: Box<HeadsBoard> = load_heads_account(program_id, board_info)?;
    let mut contract = load_contract_of(program_id, contract_info, board_info.key, &board)?;
    expect_key(period_info, &contract.period)?;
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    if contract.status == HeadsContractStatus::Settled
        || contract.components_settled != contract.component_count
        || anchor.period.status != PeriodStatus::Live
    {
        return Err(bad_state());
    }
    let target = target_of(contract.settlement_target);
    let items = usize::from(contract.item_count);
    let mut settlements = Vec::with_capacity(items);
    for i in 0..items {
        // Every channel at its own settlement: live, or its degraded path's last valid value.
        let readings: [ChannelReading; HEADS_CHANNELS] = core::array::from_fn(|ch| {
            let c = i * HEADS_CHANNELS + ch;
            ChannelReading {
                change_ppm: contract.component_change_ppm[c],
                degraded: contract.component_flags[c] & COMPONENT_LIVE == 0,
            }
        });
        let reference = channels::reference(
            &mix_at(&contract.item_mix_bps, i),
            basis_of(contract.item_mix_basis[i])?,
            &readings,
        );
        contract.item_reference_flags[i] = REFERENCE_COMPUTED;
        contract.item_reference_ppm[i] = 0;
        contract.item_reference_reason[i] = 0;
        contract.item_degraded_channels[i] = reference.degraded_mask();
        match reference {
            Reference::Available {
                change_ppm,
                assumed,
                ..
            } => {
                contract.item_reference_flags[i] |= REFERENCE_AVAILABLE;
                if assumed {
                    contract.item_reference_flags[i] |= REFERENCE_ASSUMED;
                }
                contract.item_reference_ppm[i] = change_ppm;
            }
            Reference::Unavailable(reason) => {
                contract.item_reference_reason[i] = reason.code();
            }
        }
        let item = ItemSettlement {
            basket_weight_bps: contract.item_weight_bps[i],
            contract_change_ppm: contract.component_change_ppm[i * HEADS_CHANNELS + 1],
            reference,
        };
        contract.item_change_ppm[i] = item.change_ppm(target).map_err(|_| bad_state())?;
        settlements.push(item);
    }
    let change = channels::settlement_change_ppm(target, &settlements).map_err(|_| bad_state())?;
    let value = settle::settlement_value(contract.base, change).ok_or_else(overflow)?;
    let now = t::now()?;
    contract.index_change_ppm = change;
    contract.value = value;
    contract.digest = heads_settlement_digest(&contract);
    contract.status = HeadsContractStatus::Settled;
    contract.settled_ts = now;
    contract.updated_ts = now;
    write_period_settled(
        program_id,
        period_info,
        &anchor,
        value,
        now,
        contract.digest,
    )?;
    let slot = usize::from(contract.ring_slot);
    board.contract_status[slot] = RING_SETTLED;
    board.updated_ts = now;
    store_heads_account(contract_info, contract.as_ref())?;
    store_heads_account(board_info, board.as_ref())
}
