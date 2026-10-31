// @generated from the field table of oracle v2's heads layouts by `gen_layouts.py`. Wire
// lengths are the exact sums of the fields; the codec checks them at compile time.

/// A board's parameters (spec section 7), the program's mirror of `ameba_oracle_heads::Params` with
/// a fixed wire layout. Amounts: AMBA atoms for delegation and fetcher stakes, USDC atoms for bonds
/// and prizes; rates in basis points; durations in seconds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HeadsParams {
    /// Fraction of a delegation's remaining principal one strike slashes (burned). Spec: 10%.
    pub slash_rate_bps: u16,
    /// Lost challenges before a source retires (spec: 2); at most `HEAD_STRIKE_CAPACITY`.
    pub max_strikes: u8,
    /// What a lost source challenge takes of the proposer's delegation. Spec: 100%.
    pub proposer_slash_bps: u16,
    /// Share of the board's inflow that funds the fetcher prize pool. Spec: 20%.
    pub prize_cut_bps: u16,
    /// `fill_prize = base_fill_prize + fill_prize_slope_ppb * delegation / 1e9` (USDC atoms).
    pub base_fill_prize: u64,
    /// Fill prize slope, parts per billion of the head's delegation.
    pub fill_prize_slope_ppb: u64,
    /// `update_prize = base_update_prize + update_prize_slope_ppb * delegation / 1e9`.
    pub base_update_prize: u64,
    /// Update prize slope, parts per billion of the source's delegation.
    pub update_prize_slope_ppb: u64,
    /// The USDC bond a claim (fill or update) carries.
    pub claim_bond: u64,
    /// The least USDC bond a claim or source challenge carries.
    pub challenge_bond: u64,
    /// How long a claim (or a source case) stays open to challenge.
    pub challenge_window_secs: u64,
    /// How far a late challenge pushes the window out so it can be judged.
    pub challenge_extension_secs: u64,
    /// An Open head that has not activated by then expires. Spec: 14 days.
    pub head_ttl_secs: u64,
    /// The cooldown on undelegation. Spec: 14 days.
    pub cooldown_secs: u64,
    /// Delegators a head needs to activate. Spec: 2.
    pub min_delegators: u16,
    /// Independent origins a component needs to list and to settle on live data (default 1: no hard
    /// number limit, the user 2026-10-29).
    pub min_live_sources: u16,
    /// Raw weight (time-weighted delegation) a component's eligible sources need to settle on live
    /// data.
    pub min_settle_weight: u64,
    /// `min_post = max(floor_post, alpha * D / max(n, 1))`. Spec: 0.25.
    pub alpha_bps: u16,
    /// The least delegation a post locks (AMBA atoms).
    pub floor_post: u64,
    /// `activation_threshold = max(floor_activate, beta * m)`. Spec: 0.5.
    pub beta_bps: u16,
    /// The least delegation a head needs to activate (AMBA atoms).
    pub floor_activate: u64,
    /// Business days before expiry whose prints make a source's settlement level.
    pub settlement_business_days: u8,
}

impl HeadsParams {
    /// Wire bytes.
    pub const LEN: usize = 120;
}

fixed_state_deserialize_flat!(HeadsParams, 120, {
    slash_rate_bps: u16,
    max_strikes: u8,
    proposer_slash_bps: u16,
    prize_cut_bps: u16,
    base_fill_prize: u64,
    fill_prize_slope_ppb: u64,
    base_update_prize: u64,
    update_prize_slope_ppb: u64,
    claim_bond: u64,
    challenge_bond: u64,
    challenge_window_secs: u64,
    challenge_extension_secs: u64,
    head_ttl_secs: u64,
    cooldown_secs: u64,
    min_delegators: u16,
    min_live_sources: u16,
    min_settle_weight: u64,
    alpha_bps: u16,
    floor_post: u64,
    beta_bps: u16,
    floor_activate: u64,
    settlement_business_days: u8,
});

impl From<&HeadsParams> for ameba_oracle_heads::Params {
    fn from(p: &HeadsParams) -> Self {
        Self {
            slash_rate_bps: p.slash_rate_bps,
            max_strikes: p.max_strikes,
            proposer_slash_bps: p.proposer_slash_bps,
            prize_cut_bps: p.prize_cut_bps,
            base_fill_prize: p.base_fill_prize,
            fill_prize_slope_ppb: p.fill_prize_slope_ppb,
            base_update_prize: p.base_update_prize,
            update_prize_slope_ppb: p.update_prize_slope_ppb,
            claim_bond: p.claim_bond,
            challenge_bond: p.challenge_bond,
            challenge_window_secs: p.challenge_window_secs,
            challenge_extension_secs: p.challenge_extension_secs,
            head_ttl_secs: p.head_ttl_secs,
            cooldown_secs: p.cooldown_secs,
            min_delegators: p.min_delegators,
            min_live_sources: p.min_live_sources,
            min_settle_weight: p.min_settle_weight,
            alpha_bps: p.alpha_bps,
            floor_post: p.floor_post,
            beta_bps: p.beta_bps,
            floor_activate: p.floor_activate,
            settlement_business_days: p.settlement_business_days,
        }
    }
}

impl From<&ameba_oracle_heads::Params> for HeadsParams {
    fn from(p: &ameba_oracle_heads::Params) -> Self {
        Self {
            slash_rate_bps: p.slash_rate_bps,
            max_strikes: p.max_strikes,
            proposer_slash_bps: p.proposer_slash_bps,
            prize_cut_bps: p.prize_cut_bps,
            base_fill_prize: p.base_fill_prize,
            fill_prize_slope_ppb: p.fill_prize_slope_ppb,
            base_update_prize: p.base_update_prize,
            update_prize_slope_ppb: p.update_prize_slope_ppb,
            claim_bond: p.claim_bond,
            challenge_bond: p.challenge_bond,
            challenge_window_secs: p.challenge_window_secs,
            challenge_extension_secs: p.challenge_extension_secs,
            head_ttl_secs: p.head_ttl_secs,
            cooldown_secs: p.cooldown_secs,
            min_delegators: p.min_delegators,
            min_live_sources: p.min_live_sources,
            min_settle_weight: p.min_settle_weight,
            alpha_bps: p.alpha_bps,
            floor_post: p.floor_post,
            beta_bps: p.beta_bps,
            floor_activate: p.floor_activate,
            settlement_business_days: p.settlement_business_days,
        }
    }
}

/// The family's singleton: the AMBA mint and token program, and the AMBA vault, the classic or
/// Token-2022 associated token account of the `heads-authority` PDA for that mint (the client
/// creates it
/// with the associated-token program; the program validates it and never creates it).
///
/// Seeds: `[ameba-oracle-v1, heads-config]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsConfig {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// Canonical bump of the `heads-authority` PDA that owns the vault.
    pub authority_bump: u8,
    /// The AMBA mint's decimals, read from the mint at initialization.
    pub amba_decimals: u8,
    /// The AMBA mint.
    pub amba_mint: Pubkey,
    /// Its token program: classic SPL Token or Token-2022.
    pub amba_token_program: Pubkey,
    /// The vault: `ATA(heads-authority, amba_mint, amba_token_program)`.
    pub amba_vault: Pubkey,
    /// Every AMBA atom the family owes someone: free, delegated and held stake and undelivered
    /// source-challenge awards. The vault holds at least this plus `pending_amba_burn`.
    pub amba_accounted: u64,
    /// Slashed AMBA not yet burned (`BurnPending`).
    pub pending_amba_burn: u64,
    /// AMBA burned so far.
    pub amba_burned_total: u64,
    /// Boards created.
    pub board_count: u32,
    /// Unix seconds.
    pub created_ts: u64,
    /// Unix seconds.
    pub updated_ts: u64,
}

impl Default for HeadsConfig {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            authority_bump: 0,
            amba_decimals: 0,
            amba_mint: Pubkey::new_from_array([0; 32]),
            amba_token_program: Pubkey::new_from_array([0; 32]),
            amba_vault: Pubkey::new_from_array([0; 32]),
            amba_accounted: 0,
            pending_amba_burn: 0,
            amba_burned_total: 0,
            board_count: 0,
            created_ts: 0,
            updated_ts: 0,
        }
    }
}

impl HeadsConfig {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 256;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 152;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDCONFIG";
    pub const VERSION: u8 = 1;
}

fixed_state_deserialize_flat!(HeadsConfig, 152, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    authority_bump: u8,
    amba_decimals: u8,
    amba_mint: Pubkey,
    amba_token_program: Pubkey,
    amba_vault: Pubkey,
    amba_accounted: u64,
    pending_amba_burn: u64,
    amba_burned_total: u64,
    board_count: u32,
    created_ts: u64,
    updated_ts: u64,
});

/// One product's board (spec "bucket"): parameters, items, components, money, the live-source
/// list and the ring of open contracts.
///
/// Items are the product's basket members (today's commodity buckets) with their basket weights and
/// their channel mix. Components are `(item, channel)` pairs in the mold's order, component index =
/// item * 3 + channel (`docs/lta.md`): each has its own frozen target sentence, its own heads and
/// its
/// own weighted median. Every item has its three components; a component whose sentence is still
/// zero
/// takes no heads and cannot list. Items and sentences change only between listings (`SetItem`,
/// `SetComponent`); a component's item and channel never change, since heads are posted against it.
///
/// Seeds: `[ameba-oracle-v1, heads-board, product_id]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsBoard {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The product (ASCII, zero padded), as in its periods.
    pub product_id: [u8; 32],
    /// Current parameters; a contract copies them when it lists.
    pub params: HeadsParams,
    /// Starts at 1; `SetBoardParams` increments it.
    pub params_version: u32,
    /// Items in use, 1 to `HEADS_MAX_ITEMS`.
    pub item_count: u8,
    /// Per item: its commodity bucket id (`commodity-spec`).
    pub item_bucket_ids: [[u8; 32]; 8],
    /// Per item: basket weight; a listing needs the items' weights to add up to 10,000.
    pub item_weight_bps: [u16; 8],
    /// Per item, at `item * 3 + channel`: the fixed-mix reference's channel shares (spot, contract,
    /// LTA), adding up to 10,000.
    pub item_mix_bps: [u16; 24],
    /// Per item: `HeadsMixBasis` of the mix as a byte (none, assumed scenario, matched evidence).
    pub item_mix_basis: [u8; 8],
    /// Components in use: three per item (`item_count * 3`).
    pub component_count: u8,
    /// Per component: its item index (`index / 3`).
    pub component_items: [u8; 24],
    /// Per component: `HeadsChannel` as a byte (`index % 3`: spot, contract, LTA).
    pub component_channels: [u8; 24],
    /// Per component: commitment of its target sentence with the bracketed terms filled (a sealed
    /// kind-2 evidence object whose commitment is this hash); zero until set.
    pub component_sentence_hashes: [[u8; 32]; 24],
    /// Per component: live sources.
    pub component_live_sources: [u16; 24],
    /// Per component: distinct origins among its live sources (duplicates excluded);
    /// `min_live_sources` counts these.
    pub component_live_origins: [u16; 24],
    /// Heads posted; the next head's id.
    pub head_count: u32,
    /// `D` of spec section 6: active delegation on every head that is not retired or expired.
    pub total_delegation: u64,
    /// Active (and fresh) delegation on Live heads that are not duplicates: the reward index's
    /// denominator.
    pub live_delegation: u64,
    /// Entries in the live-source list.
    pub live_count: u16,
    /// Live-source list: head id.
    pub live_head_ids: [u32; 96],
    /// Live-source list: component index.
    pub live_components: [u8; 96],
    /// Live-source list: `LIVE_DUPLICATE` when another live source of the same component has the
    /// same origin and precedes it (earlier `live_since_ts`, then lower head id:
    /// `channels::duplicate_flags`). Recomputed whenever a source goes live or retires or an
    /// accepted print declares a new origin.
    pub live_flags: [u8; 96],
    /// Live-source list: when the head went live (duplicate precedence).
    pub live_since_ts: [u64; 96],
    /// Live-source list: the head's active delegation (thresholds use their median).
    pub live_delegations: [u64; 96],
    /// Live-source list: the head's current value (a listing reads its openings here).
    pub live_values: [u64; 96],
    /// Live-source list: the origin of the head's last accepted print.
    pub live_origin_ids: [[u8; 32]; 96],
    /// Live-source list: the head's delegator reward index, encoded so funding never touches the
    /// list: `reward_index - offset` while the entry earns (not a duplicate), the frozen value
    /// `offset` while it does not. A flip re-encodes it (`transitions::set_live_flags`).
    pub live_reward_offsets: [u128; 96],
    /// Delegator reward index (`rewards::INDEX_SCALE` fixed point).
    pub reward_index: u128,
    /// Delegator share funded while nothing was live, distributed by the next funding with live
    /// delegation (USDC atoms).
    pub reserve: u64,
    /// USDC routed to delegators.
    pub delegator_funded_total: u64,
    /// USDC claimed by delegators.
    pub delegator_paid_total: u64,
    /// USDC routed to prize epochs.
    pub prize_funded_total: u64,
    /// USDC paid as prizes.
    pub prize_paid_total: u64,
    /// Burned USDC (losing challenge bonds of a falling claim) still in the bond vault
    /// (`BurnPending`).
    pub pending_usdc_burn: u64,
    /// USDC burned so far.
    pub usdc_burned_total: u64,
    /// Contract ring: the period of each open contract (default: empty slot).
    pub contract_periods: [Pubkey; 4],
    /// Contract ring: listing time.
    pub contract_listing_ts: [u64; 4],
    /// Contract ring: freeze time (end of the accumulation window).
    pub contract_freeze_ts: [u64; 4],
    /// Contract ring: expiry.
    pub contract_expiry_ts: [u64; 4],
    /// Contract ring: the slot's state, `RING_EMPTY`, `RING_ACCUMULATING`, `RING_COLLECTING` or
    /// `RING_SETTLED`.
    pub contract_status: [u8; 4],
    /// Contract ring: `RING_FREEZE_CAPTURED` once the components' origins at freeze are captured.
    pub contract_flags: [u8; 4],
    /// Contract ring, at `slot * 24 + component`: distinct live origins as of the freeze. The first
    /// instruction after the freeze that changes `component_live_origins` captures them first (the
    /// counts were constant since the last change, so the capture is exact).
    pub contract_freeze_origins: [u16; 96],
    /// Contracts listed.
    pub contract_count: u32,
    /// Unix seconds.
    pub created_ts: u64,
    /// Unix seconds.
    pub updated_ts: u64,
    /// Contract ring: start of the freeze level's window (`time::settlement_window_start` of the
    /// freeze on the contract's parameters).
    pub contract_freeze_window_ts: [u64; 4],
    /// Contract ring: the contract's generation, `contract_count` when it listed (unique per board,
    /// never reused like a slot). Exposure to a contract is held by generation: a source case
    /// records the generations open when it opened, a delegation those it contributed to.
    pub contract_gens: [u32; 4],
    /// Contract ring, at `slot * 24 + component`: sources that went live after the listing and
    /// before the freeze (each marks itself `CAPTURE_JOINED`). With the listed ones they are the
    /// contract's members: `SettleComponent` settles a component only once it recorded all of them,
    /// retired or not.
    pub contract_joined: [u16; 96],
    /// How long after its window a claim or source case may wait for its rulings; past it an
    /// unruled one is void (every bond back, nothing prints, no strike). 0:
    /// `DEFAULT_RULING_DEADLINE_SECS`. A protection against funds locked for ever (re-review #13).
    pub ruling_deadline_secs: u64,
    /// Contract ring: the cutoff K, the expiry plus the contract's challenge window: admission to
    /// the contract is decided from facts as of K, and records open after it.
    pub contract_cutoff_ts: [u64; 4],
    /// Live list: the entry's LTA price's next reset (unix seconds, at most `u32::MAX`; 0: none,
    /// and always 0 off the LTA channel). A listing counts and opens an entry only while its price
    /// is in effect (`HeadsBoard::live_in_effect`, round 4 N5).
    pub live_reset_ts: [u32; 96],
    /// Live list: the head's delegators (`Head::delegators`, at most 255; `min_delegators` is at
    /// most 255). With `live_delegations` they say whether the entry still meets activation backing
    /// (`HeadsBoard::live_backed`): listing and freeze coverage count only entries that do (round 6
    /// review, finding 1).
    pub live_delegators: [u8; 96],
}

#[cfg(not(target_os = "solana"))]
impl Default for HeadsBoard {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            product_id: [0; 32],
            params: HeadsParams::zero(),
            params_version: 0,
            item_count: 0,
            item_bucket_ids: [[0; 32]; 8],
            item_weight_bps: [0; 8],
            item_mix_bps: [0; 24],
            item_mix_basis: [0; 8],
            component_count: 0,
            component_items: [0; 24],
            component_channels: [0; 24],
            component_sentence_hashes: [[0; 32]; 24],
            component_live_sources: [0; 24],
            component_live_origins: [0; 24],
            head_count: 0,
            total_delegation: 0,
            live_delegation: 0,
            live_count: 0,
            live_head_ids: [0; 96],
            live_components: [0; 96],
            live_flags: [0; 96],
            live_since_ts: [0; 96],
            live_delegations: [0; 96],
            live_values: [0; 96],
            live_origin_ids: [[0; 32]; 96],
            live_reward_offsets: [0; 96],
            reward_index: 0,
            reserve: 0,
            delegator_funded_total: 0,
            delegator_paid_total: 0,
            prize_funded_total: 0,
            prize_paid_total: 0,
            pending_usdc_burn: 0,
            usdc_burned_total: 0,
            contract_periods: [Pubkey::new_from_array([0; 32]); 4],
            contract_listing_ts: [0; 4],
            contract_freeze_ts: [0; 4],
            contract_expiry_ts: [0; 4],
            contract_status: [0; 4],
            contract_flags: [0; 4],
            contract_freeze_origins: [0; 96],
            contract_count: 0,
            created_ts: 0,
            updated_ts: 0,
            contract_freeze_window_ts: [0; 4],
            contract_gens: [0; 4],
            contract_joined: [0; 96],
            ruling_deadline_secs: 0,
            contract_cutoff_ts: [0; 4],
            live_reset_ts: [0; 96],
            live_delegators: [0; 96],
        }
    }
}

impl HeadsBoard {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 10240;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 10194;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDBOARD_";
    pub const VERSION: u8 = 3;
}

#[cfg(not(target_os = "solana"))]
fixed_state_deserialize_flat!(HeadsBoard, 10194, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    product_id: [u8; 32],
    params: HeadsParams,
    params_version: u32,
    item_count: u8,
    item_bucket_ids: [[u8; 32]; 8],
    item_weight_bps: [u16; 8],
    item_mix_bps: [u16; 24],
    item_mix_basis: [u8; 8],
    component_count: u8,
    component_items: [u8; 24],
    component_channels: [u8; 24],
    component_sentence_hashes: [[u8; 32]; 24],
    component_live_sources: [u16; 24],
    component_live_origins: [u16; 24],
    head_count: u32,
    total_delegation: u64,
    live_delegation: u64,
    live_count: u16,
    live_head_ids: [u32; 96],
    live_components: [u8; 96],
    live_flags: [u8; 96],
    live_since_ts: [u64; 96],
    live_delegations: [u64; 96],
    live_values: [u64; 96],
    live_origin_ids: [[u8; 32]; 96],
    live_reward_offsets: [u128; 96],
    reward_index: u128,
    reserve: u64,
    delegator_funded_total: u64,
    delegator_paid_total: u64,
    prize_funded_total: u64,
    prize_paid_total: u64,
    pending_usdc_burn: u64,
    usdc_burned_total: u64,
    contract_periods: [Pubkey; 4],
    contract_listing_ts: [u64; 4],
    contract_freeze_ts: [u64; 4],
    contract_expiry_ts: [u64; 4],
    contract_status: [u8; 4],
    contract_flags: [u8; 4],
    contract_freeze_origins: [u16; 96],
    contract_count: u32,
    created_ts: u64,
    updated_ts: u64,
    contract_freeze_window_ts: [u64; 4],
    contract_gens: [u32; 4],
    contract_joined: [u16; 96],
    ruling_deadline_secs: u64,
    contract_cutoff_ts: [u64; 4],
    live_reset_ts: [u32; 96],
    live_delegators: [u8; 96],
}, flat {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    product_id: [u8; 32],
    params.slash_rate_bps: u16,
    params.max_strikes: u8,
    params.proposer_slash_bps: u16,
    params.prize_cut_bps: u16,
    params.base_fill_prize: u64,
    params.fill_prize_slope_ppb: u64,
    params.base_update_prize: u64,
    params.update_prize_slope_ppb: u64,
    params.claim_bond: u64,
    params.challenge_bond: u64,
    params.challenge_window_secs: u64,
    params.challenge_extension_secs: u64,
    params.head_ttl_secs: u64,
    params.cooldown_secs: u64,
    params.min_delegators: u16,
    params.min_live_sources: u16,
    params.min_settle_weight: u64,
    params.alpha_bps: u16,
    params.floor_post: u64,
    params.beta_bps: u16,
    params.floor_activate: u64,
    params.settlement_business_days: u8,
    params_version: u32,
    item_count: u8,
    item_bucket_ids: [[u8; 32]; 8],
    item_weight_bps: [u16; 8],
    item_mix_bps: [u16; 24],
    item_mix_basis: [u8; 8],
    component_count: u8,
    component_items: [u8; 24],
    component_channels: [u8; 24],
    component_sentence_hashes: [[u8; 32]; 24],
    component_live_sources: [u16; 24],
    component_live_origins: [u16; 24],
    head_count: u32,
    total_delegation: u64,
    live_delegation: u64,
    live_count: u16,
    live_head_ids: [u32; 96],
    live_components: [u8; 96],
    live_flags: [u8; 96],
    live_since_ts: [u64; 96],
    live_delegations: [u64; 96],
    live_values: [u64; 96],
    live_origin_ids: [[u8; 32]; 96],
    live_reward_offsets: [u128; 96],
    reward_index: u128,
    reserve: u64,
    delegator_funded_total: u64,
    delegator_paid_total: u64,
    prize_funded_total: u64,
    prize_paid_total: u64,
    pending_usdc_burn: u64,
    usdc_burned_total: u64,
    contract_periods: [Pubkey; 4],
    contract_listing_ts: [u64; 4],
    contract_freeze_ts: [u64; 4],
    contract_expiry_ts: [u64; 4],
    contract_status: [u8; 4],
    contract_flags: [u8; 4],
    contract_freeze_origins: [u16; 96],
    contract_count: u32,
    created_ts: u64,
    updated_ts: u64,
    contract_freeze_window_ts: [u64; 4],
    contract_gens: [u32; 4],
    contract_joined: [u16; 96],
    ruling_deadline_secs: u64,
    contract_cutoff_ts: [u64; 4],
    live_reset_ts: [u32; 96],
    live_delegators: [u8; 96],
});

#[cfg(target_os = "solana")]
heads_heap_codec!(HeadsBoard, 10194, flat {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    product_id: [u8; 32],
    params.slash_rate_bps: u16,
    params.max_strikes: u8,
    params.proposer_slash_bps: u16,
    params.prize_cut_bps: u16,
    params.base_fill_prize: u64,
    params.fill_prize_slope_ppb: u64,
    params.base_update_prize: u64,
    params.update_prize_slope_ppb: u64,
    params.claim_bond: u64,
    params.challenge_bond: u64,
    params.challenge_window_secs: u64,
    params.challenge_extension_secs: u64,
    params.head_ttl_secs: u64,
    params.cooldown_secs: u64,
    params.min_delegators: u16,
    params.min_live_sources: u16,
    params.min_settle_weight: u64,
    params.alpha_bps: u16,
    params.floor_post: u64,
    params.beta_bps: u16,
    params.floor_activate: u64,
    params.settlement_business_days: u8,
    params_version: u32,
    item_count: u8,
    item_bucket_ids: [[u8; 32]; 8],
    item_weight_bps: [u16; 8],
    item_mix_bps: [u16; 24],
    item_mix_basis: [u8; 8],
    component_count: u8,
    component_items: [u8; 24],
    component_channels: [u8; 24],
    component_sentence_hashes: [[u8; 32]; 24],
    component_live_sources: [u16; 24],
    component_live_origins: [u16; 24],
    head_count: u32,
    total_delegation: u64,
    live_delegation: u64,
    live_count: u16,
    live_head_ids: [u32; 96],
    live_components: [u8; 96],
    live_flags: [u8; 96],
    live_since_ts: [u64; 96],
    live_delegations: [u64; 96],
    live_values: [u64; 96],
    live_origin_ids: [[u8; 32]; 96],
    live_reward_offsets: [u128; 96],
    reward_index: u128,
    reserve: u64,
    delegator_funded_total: u64,
    delegator_paid_total: u64,
    prize_funded_total: u64,
    prize_paid_total: u64,
    pending_usdc_burn: u64,
    usdc_burned_total: u64,
    contract_periods: [Pubkey; 4],
    contract_listing_ts: [u64; 4],
    contract_freeze_ts: [u64; 4],
    contract_expiry_ts: [u64; 4],
    contract_status: [u8; 4],
    contract_flags: [u8; 4],
    contract_freeze_origins: [u16; 96],
    contract_count: u32,
    created_ts: u64,
    updated_ts: u64,
    contract_freeze_window_ts: [u64; 4],
    contract_gens: [u32; 4],
    contract_joined: [u16; 96],
    ruling_deadline_secs: u64,
    contract_cutoff_ts: [u64; 4],
    live_reset_ts: [u32; 96],
    live_delegators: [u8; 96],
});

/// One owner's AMBA in the family: free stake, and counters of what it has delegated and holds in
/// cooldown or escrow.
///
/// Seeds: `[ameba-oracle-v1, heads-stake, owner]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsStake {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The owner.
    pub owner: Pubkey,
    /// Staked and not delegated: `Unstake` withdraws it at once.
    pub free: u64,
    /// AMBA moved from free stake onto delegations, ever.
    pub delegated_total: u64,
    /// AMBA released from delegations back to free stake, ever (what is out is `delegated_total -
    /// returned_total` less slashes; the delegations hold the detail).
    pub returned_total: u64,
    /// AMBA credited by `Stake` (what arrived, transfer fee off).
    pub staked_in_total: u64,
    /// AMBA sent out by `Unstake`.
    pub unstaked_out_total: u64,
    /// AMBA received from won source challenges.
    pub awarded_total: u64,
    /// Open `Delegation` accounts.
    pub delegation_count: u32,
    /// Unix seconds.
    pub created_ts: u64,
    /// Unix seconds.
    pub updated_ts: u64,
}

impl Default for HeadsStake {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            owner: Pubkey::new_from_array([0; 32]),
            free: 0,
            delegated_total: 0,
            returned_total: 0,
            staked_in_total: 0,
            unstaked_out_total: 0,
            awarded_total: 0,
            delegation_count: 0,
            created_ts: 0,
            updated_ts: 0,
        }
    }
}

impl HeadsStake {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 192;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 110;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDSTAKE_";
    pub const VERSION: u8 = 1;
}

fixed_state_deserialize_flat!(HeadsStake, 110, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    owner: Pubkey,
    free: u64,
    delegated_total: u64,
    returned_total: u64,
    staked_in_total: u64,
    unstaked_out_total: u64,
    awarded_total: u64,
    delegation_count: u32,
    created_ts: u64,
    updated_ts: u64,
});

/// A proposed source (spec "Head"), a source once Live. It keeps the live-gated integral of its
/// total delegation and that integral's captures at each open contract's listing and freeze, its
/// strikes
/// with the opening time of the case that caused each (delegations apply them lazily), its
/// reward-index
/// window, its accepted prints and the bookkeeping of its claims and of its current source case.
///
/// Seeds: `[ameba-oracle-v1, head, board, head_id (u32 LE)]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Head {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The board.
    pub board: Pubkey,
    /// Position on the board (0-based).
    pub head_id: u32,
    /// Component index on the board.
    pub component: u8,
    /// The component's channel, fixed when the head is posted.
    pub channel: HeadsChannel,
    /// How a claim obtains the value.
    pub access_class: HeadAccessClass,
    /// Open, Active, Live, Retired or Expired.
    pub status: HeadStatus,
    /// Bytes of `name` in use (UTF-8, at most `HEAD_NAME_MAX_BYTES`); the rest are zero.
    pub name_len: u8,
    /// Human-readable name, e.g. "Mouser DDR5-4800 16Gb spot".
    pub name: [u8; 64],
    /// Commitment of the sealed kind-2 evidence object that holds the spec (it restates the
    /// component's target sentence).
    pub spec_hash: [u8; 32],
    /// That evidence object.
    pub spec_object: Pubkey,
    /// Who posted the head.
    pub proposer: Pubkey,
    /// Active delegation (AMBA atoms).
    pub delegation: u64,
    /// Delegations with active delegation.
    pub delegators: u16,
    /// When `accrual_integral` was last brought up to date.
    pub accrual_since_ts: u64,
    /// `integral of delegation * live(t) dt` up to `accrual_since_ts` (atom-seconds), `live(t)` the
    /// head's live interval `[live_ts, retired_ts)`.
    pub accrual_integral: u128,
    /// Unix seconds.
    pub posted_ts: u64,
    /// `posted_ts + head_ttl_secs`: an Open head past it expires.
    pub expires_ts: u64,
    /// When it activated (0: never).
    pub active_ts: u64,
    /// When its fill stood (0: never).
    pub live_ts: u64,
    /// When it retired or expired (0: never).
    pub retired_ts: u64,
    /// Per board contract-ring slot: the contract period these captures belong to (default: none).
    /// A slot whose period differs from the board's is stale and restarts.
    pub capture_periods: [Pubkey; 4],
    /// The live-gated integral captured at that contract's listing.
    pub capture_listing: [u128; 4],
    /// The live-gated integral captured at that contract's freeze.
    pub capture_freeze: [u128; 4],
    /// Per slot: `CAPTURE_LISTING`, `CAPTURE_FREEZE` (and on a head `CAPTURE_JOINED`).
    pub capture_flags: [u8; 4],
    /// Strikes recorded, at most `HEAD_STRIKE_CAPACITY`.
    pub strike_count: u8,
    /// Per strike: when the source case that caused it opened (first challenge). A delegation is
    /// exposed to it only if it was active then; the proposer skips it (it paid
    /// `proposer_slash_bps` at resolution).
    pub strike_opened_ts: [u64; 8],
    /// Per strike: when it was recorded; lazy application moves the accruals at this time.
    pub strike_ts: [u64; 8],
    /// The head's delegator reward index frozen when it left the live list (retired); while Live
    /// the board's entry holds it (`transitions::head_reward_index`).
    pub reward_index_frozen: u128,
    /// Current value (the last standing print), six-decimal atoms.
    pub value: u64,
    /// Its source time.
    pub value_source_ts: u64,
    /// The claim that set it.
    pub value_claim_seq: u32,
    /// The origin its last accepted print declared (hash of the underlying report's identity).
    pub origin_id: [u8; 32],
    /// An earlier live source of the same component has the same origin: no weight, no origin
    /// count, no delegator share. Recomputed when claims finalize or sources retire.
    pub duplicate: bool,
    /// LTA channel: when the last accepted print's price took effect (0: none).
    pub lta_effective_from_ts: u64,
    /// LTA channel: its next reset (0: none); past it the print is no current level.
    pub lta_next_reset_ts: u64,
    /// LTA channel: its floor (0: none).
    pub lta_floor: u64,
    /// LTA channel: its ceiling (0: none).
    pub lta_ceiling: u64,
    /// The value the standing fill set (the opening of contracts it joins mid-window).
    pub fill_value: u64,
    /// Its source time.
    pub fill_source_ts: u64,
    /// Accepted prints so far; the ring holds the last `HEAD_PRINT_CAPACITY` at `index % capacity`.
    pub print_count: u32,
    /// Accepted prints: value.
    pub print_values: [u64; 32],
    /// Accepted prints: source time.
    pub print_source_ts: [u64; 32],
    /// Accepted prints: an LTA print's effective date (0: none; zero off the LTA channel).
    pub print_effective_from_ts: [u64; 32],
    /// Accepted prints: an LTA print's next reset (0: none); `channels::lta_level` drops a print
    /// past its own reset.
    pub print_next_reset_ts: [u64; 32],
    /// Accepted prints: an LTA print's floor (0: none).
    pub print_floor: [u64; 32],
    /// Accepted prints: an LTA print's ceiling (0: none).
    pub print_ceiling: [u64; 32],
    /// Accepted prints: the claim that made it. The ring is for display: settlement reads the
    /// contract samples (`sample_values`).
    pub print_claim_seq: [u32; 32],
    /// Claims revealed so far: a claim's `reveal_index`. (These 16 bytes held `capture_expiry_seq`
    /// before 2026-10-30.)
    pub reveals: u32,
    /// Revealed claims finalized so far, in whatever order: `reveals - finalized_reveals` are open.
    pub finalized_reveals: u32,
    /// The UTC day (`source_ts / 86,400`) of the last update that earned a prize: one update prize
    /// per head per day, whatever the sampling.
    pub last_prize_day: u32,
    /// Zero (held `sample_count` before round 3; kept so the 2,688-byte layout stays this one's
    /// prefix).
    pub samples_reserved: [u8; 4],
    /// Per ring slot: the open claims settlement waits for (`samples::waits`): those revealed
    /// before the slot started (`restart_reveals`; a contract may admit them), and those revealed
    /// since that the contract admits (`samples::admitted`: committed before its expiry, window
    /// ended and every challenge ruled by its cutoff K, standing). (These 8 bytes held
    /// `capture_expiry_open` before 2026-10-30.)
    pub adm_open: [u16; 4],
    /// Claims committed; the next claim's seq.
    pub claim_seq: u32,
    /// Claims committed or revealed and not yet finalized or refunded.
    pub open_claims: u16,
    /// A fill claim is committed or in its window (competing fills challenge it).
    pub open_fill: bool,
    /// That fill claim's seq.
    pub open_fill_seq: u32,
    /// Source cases that have not resolved (claim challenges open none: they never strike).
    pub unresolved_cases: u16,
    /// When the current run of unresolved cases began (0 when none).
    pub unresolved_since_ts: u64,
    /// Number of the current or last source case (0: none).
    pub source_case: u32,
    /// `SOURCE_CASE_NONE`, `_OPEN`, `_STOOD` or `_FELL`.
    pub source_case_status: u8,
    /// When the case's first challenge arrived.
    pub source_case_opened_ts: u64,
    /// The case's challenge window, late challenges extending it.
    pub source_case_window_end_ts: u64,
    /// Challenges in the case.
    pub source_case_challenges: u16,
    /// Of them, ruled.
    pub source_case_ruled: u16,
    /// Of them, won.
    pub source_case_won: u16,
    /// Index of the last won challenge: it takes the pro-rata remainder (`outcome::pro_rata`).
    pub source_case_last_winner: u16,
    /// Won challenges already paid.
    pub source_case_settled_winners: u16,
    /// Sum of the case's bonds.
    pub source_case_bond_total: u64,
    /// Sum of the won challenges' bonds.
    pub source_case_winning_bonds: u64,
    /// AMBA the falling case took from the proposer's delegation.
    pub source_case_proposer_slashed: u64,
    /// Of it, paid to winners so far.
    pub source_case_amba_paid: u64,
    /// Unix seconds.
    pub updated_ts: u64,
    /// Per strike: the number of the source case that caused it; delegations compare case numbers,
    /// not times.
    pub strike_case: [u32; 8],
    /// Per strike, at `strike * 4 + slot`: the generation of the ring contract unsettled in that
    /// slot when its case opened, the head a member of it then (0: none; round 14 review, N1).
    /// Escrow kept for one of them answers for the strike.
    pub strike_open_gens: [u32; 32],
    /// The current case's open ring generations, as `strike_open_gens` keeps them.
    pub source_case_open_gens: [u32; 4],
    /// The current case's ruling deadline, fixed when it opened (its first window's end plus the
    /// board's ruling deadline); late challenges extend the window only up to it. From it no ruling
    /// is taken and the case resolves on the rulings it has, each unruled challenge timing out
    /// alone (its bond back): early review B2, B5, B6.
    pub source_case_deadline_ts: u64,
    /// Sum of the case's ruled challenges' bonds: the case resolves on these.
    pub source_case_ruled_bonds: u64,
    /// Per ring slot, at `slot * 10 + j`: the contract's samples at fixed instants
    /// (`samples::instants`: five in the freeze level's window, then five in the settlement
    /// window), each the value of the print deciding it so far (0: none), whose level there is the
    /// value unless past `sample_resets`; so the print ring never decides settlement.
    pub sample_values: [u64; 40],
    /// Per sample: the source time of the print deciding it so far, the latest the contract admits
    /// at or before its instant (0: none). Latest by source time, then by what it says
    /// (`samples::Key`: value, then reset), so the order claims finalize in never matters (round 3
    /// R1; round 5).
    pub sample_ts: [u64; 40],
    /// Per sample: that print's LTA reset (unix seconds, at most `u32::MAX`; 0: none). (A hash rank
    /// in version 1.)
    pub sample_resets: [u32; 40],
    /// Per ring slot: the origin of the print deciding its last instant (the expiry): the latest
    /// the contract admits, the provenance it settles on.
    pub standing_origin: [[u8; 32]; 4],
    /// Per ring slot: the origin of the print deciding its freeze instant, the fallback's
    /// provenance.
    pub freeze_origin: [[u8; 32]; 4],
    /// Per ring slot: `reveals` when the slot started on its contract. A claim revealed before
    /// (`reveal_index` below it) counts in `adm_open` while it is open: the contract may admit it.
    pub restart_reveals: [u32; 4],
    /// Since when the head has been below activation backing (`floor_activate` with
    /// `min_delegators`), continuously: set by the delegation change that took it below, cleared
    /// (0) by the one that brought it back. On a full list a Live head unbacked for `cooldown_secs`
    /// gives up its slot even as a contract's member (round 11 review, finding 3). 0 on a head of
    /// before: `ExpireHead` starts its time.
    pub unbacked_since_ts: u64,
    /// When the head left the live list without retiring (0: never): on a full list after
    /// `cooldown_secs` unbacked, a member of an unsettled contract or with a case open (round 12
    /// review, N1 and N2). It stays Live for everything else: a member of its contracts, eligible
    /// and weighing as if listed, printing, its delegations uncooled, open to source cases. It is
    /// no origin, joins no contract, takes no slot and does not come back; `ExpireHead` retires it
    /// once none of its contracts is unsettled and no case is open.
    pub evicted_ts: u64,
}

#[cfg(not(target_os = "solana"))]
impl Default for Head {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            board: Pubkey::new_from_array([0; 32]),
            head_id: 0,
            component: 0,
            channel: HeadsChannel::default(),
            access_class: HeadAccessClass::default(),
            status: HeadStatus::default(),
            name_len: 0,
            name: [0; 64],
            spec_hash: [0; 32],
            spec_object: Pubkey::new_from_array([0; 32]),
            proposer: Pubkey::new_from_array([0; 32]),
            delegation: 0,
            delegators: 0,
            accrual_since_ts: 0,
            accrual_integral: 0,
            posted_ts: 0,
            expires_ts: 0,
            active_ts: 0,
            live_ts: 0,
            retired_ts: 0,
            capture_periods: [Pubkey::new_from_array([0; 32]); 4],
            capture_listing: [0; 4],
            capture_freeze: [0; 4],
            capture_flags: [0; 4],
            strike_count: 0,
            strike_opened_ts: [0; 8],
            strike_ts: [0; 8],
            reward_index_frozen: 0,
            value: 0,
            value_source_ts: 0,
            value_claim_seq: 0,
            origin_id: [0; 32],
            duplicate: false,
            lta_effective_from_ts: 0,
            lta_next_reset_ts: 0,
            lta_floor: 0,
            lta_ceiling: 0,
            fill_value: 0,
            fill_source_ts: 0,
            print_count: 0,
            print_values: [0; 32],
            print_source_ts: [0; 32],
            print_effective_from_ts: [0; 32],
            print_next_reset_ts: [0; 32],
            print_floor: [0; 32],
            print_ceiling: [0; 32],
            print_claim_seq: [0; 32],
            reveals: 0,
            finalized_reveals: 0,
            last_prize_day: 0,
            samples_reserved: [0; 4],
            adm_open: [0; 4],
            claim_seq: 0,
            open_claims: 0,
            open_fill: false,
            open_fill_seq: 0,
            unresolved_cases: 0,
            unresolved_since_ts: 0,
            source_case: 0,
            source_case_status: 0,
            source_case_opened_ts: 0,
            source_case_window_end_ts: 0,
            source_case_challenges: 0,
            source_case_ruled: 0,
            source_case_won: 0,
            source_case_last_winner: 0,
            source_case_settled_winners: 0,
            source_case_bond_total: 0,
            source_case_winning_bonds: 0,
            source_case_proposer_slashed: 0,
            source_case_amba_paid: 0,
            updated_ts: 0,
            strike_case: [0; 8],
            strike_open_gens: [0; 32],
            source_case_open_gens: [0; 4],
            source_case_deadline_ts: 0,
            source_case_ruled_bonds: 0,
            sample_values: [0; 40],
            sample_ts: [0; 40],
            sample_resets: [0; 40],
            standing_origin: [[0; 32]; 4],
            freeze_origin: [[0; 32]; 4],
            restart_reveals: [0; 4],
            unbacked_since_ts: 0,
            evicted_ts: 0,
        }
    }
}

impl Head {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 4096;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 3855;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDHEAD__";
    pub const VERSION: u8 = 2;
}

#[cfg(not(target_os = "solana"))]
fixed_state_deserialize_flat!(Head, 3855, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    board: Pubkey,
    head_id: u32,
    component: u8,
    channel: HeadsChannel,
    access_class: HeadAccessClass,
    status: HeadStatus,
    name_len: u8,
    name: [u8; 64],
    spec_hash: [u8; 32],
    spec_object: Pubkey,
    proposer: Pubkey,
    delegation: u64,
    delegators: u16,
    accrual_since_ts: u64,
    accrual_integral: u128,
    posted_ts: u64,
    expires_ts: u64,
    active_ts: u64,
    live_ts: u64,
    retired_ts: u64,
    capture_periods: [Pubkey; 4],
    capture_listing: [u128; 4],
    capture_freeze: [u128; 4],
    capture_flags: [u8; 4],
    strike_count: u8,
    strike_opened_ts: [u64; 8],
    strike_ts: [u64; 8],
    reward_index_frozen: u128,
    value: u64,
    value_source_ts: u64,
    value_claim_seq: u32,
    origin_id: [u8; 32],
    duplicate: bool,
    lta_effective_from_ts: u64,
    lta_next_reset_ts: u64,
    lta_floor: u64,
    lta_ceiling: u64,
    fill_value: u64,
    fill_source_ts: u64,
    print_count: u32,
    print_values: [u64; 32],
    print_source_ts: [u64; 32],
    print_effective_from_ts: [u64; 32],
    print_next_reset_ts: [u64; 32],
    print_floor: [u64; 32],
    print_ceiling: [u64; 32],
    print_claim_seq: [u32; 32],
    reveals: u32,
    finalized_reveals: u32,
    last_prize_day: u32,
    samples_reserved: [u8; 4],
    adm_open: [u16; 4],
    claim_seq: u32,
    open_claims: u16,
    open_fill: bool,
    open_fill_seq: u32,
    unresolved_cases: u16,
    unresolved_since_ts: u64,
    source_case: u32,
    source_case_status: u8,
    source_case_opened_ts: u64,
    source_case_window_end_ts: u64,
    source_case_challenges: u16,
    source_case_ruled: u16,
    source_case_won: u16,
    source_case_last_winner: u16,
    source_case_settled_winners: u16,
    source_case_bond_total: u64,
    source_case_winning_bonds: u64,
    source_case_proposer_slashed: u64,
    source_case_amba_paid: u64,
    updated_ts: u64,
    strike_case: [u32; 8],
    strike_open_gens: [u32; 32],
    source_case_open_gens: [u32; 4],
    source_case_deadline_ts: u64,
    source_case_ruled_bonds: u64,
    sample_values: [u64; 40],
    sample_ts: [u64; 40],
    sample_resets: [u32; 40],
    standing_origin: [[u8; 32]; 4],
    freeze_origin: [[u8; 32]; 4],
    restart_reveals: [u32; 4],
    unbacked_since_ts: u64,
    evicted_ts: u64,
});

#[cfg(target_os = "solana")]
heads_heap_codec!(Head, 3855, flat {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    board: Pubkey,
    head_id: u32,
    component: u8,
    channel: HeadsChannel,
    access_class: HeadAccessClass,
    status: HeadStatus,
    name_len: u8,
    name: [u8; 64],
    spec_hash: [u8; 32],
    spec_object: Pubkey,
    proposer: Pubkey,
    delegation: u64,
    delegators: u16,
    accrual_since_ts: u64,
    accrual_integral: u128,
    posted_ts: u64,
    expires_ts: u64,
    active_ts: u64,
    live_ts: u64,
    retired_ts: u64,
    capture_periods: [Pubkey; 4],
    capture_listing: [u128; 4],
    capture_freeze: [u128; 4],
    capture_flags: [u8; 4],
    strike_count: u8,
    strike_opened_ts: [u64; 8],
    strike_ts: [u64; 8],
    reward_index_frozen: u128,
    value: u64,
    value_source_ts: u64,
    value_claim_seq: u32,
    origin_id: [u8; 32],
    duplicate: bool,
    lta_effective_from_ts: u64,
    lta_next_reset_ts: u64,
    lta_floor: u64,
    lta_ceiling: u64,
    fill_value: u64,
    fill_source_ts: u64,
    print_count: u32,
    print_values: [u64; 32],
    print_source_ts: [u64; 32],
    print_effective_from_ts: [u64; 32],
    print_next_reset_ts: [u64; 32],
    print_floor: [u64; 32],
    print_ceiling: [u64; 32],
    print_claim_seq: [u32; 32],
    reveals: u32,
    finalized_reveals: u32,
    last_prize_day: u32,
    samples_reserved: [u8; 4],
    adm_open: [u16; 4],
    claim_seq: u32,
    open_claims: u16,
    open_fill: bool,
    open_fill_seq: u32,
    unresolved_cases: u16,
    unresolved_since_ts: u64,
    source_case: u32,
    source_case_status: u8,
    source_case_opened_ts: u64,
    source_case_window_end_ts: u64,
    source_case_challenges: u16,
    source_case_ruled: u16,
    source_case_won: u16,
    source_case_last_winner: u16,
    source_case_settled_winners: u16,
    source_case_bond_total: u64,
    source_case_winning_bonds: u64,
    source_case_proposer_slashed: u64,
    source_case_amba_paid: u64,
    updated_ts: u64,
    strike_case: [u32; 8],
    strike_open_gens: [u32; 32],
    source_case_open_gens: [u32; 4],
    source_case_deadline_ts: u64,
    source_case_ruled_bonds: u64,
    sample_values: [u64; 40],
    sample_ts: [u64; 40],
    sample_resets: [u32; 40],
    standing_origin: [[u8; 32]; 4],
    freeze_origin: [[u8; 32]; 4],
    restart_reveals: [u32; 4],
    unbacked_since_ts: u64,
    evicted_ts: u64,
});

/// One owner's AMBA on one head. Active delegation carries weight and earns while the head is
/// Live; undelegation moves it into `held` (`liability::Held`), which comes back after the cooldown
/// except for the escrow the remaining strikes and the open exposure still need. It keeps the same
/// live-gated integral as its head, captured lazily at each open contract's listing and freeze.
///
/// Seeds: `[ameba-oracle-v1, head-delegation, head, owner]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delegation {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The head.
    pub head: Pubkey,
    /// The owner.
    pub owner: Pubkey,
    /// The head's board.
    pub board: Pubkey,
    /// The head's proposer: a lost source challenge takes `proposer_slash_bps` of it, and its
    /// escrow is `liability::proposer_escrow_required`.
    pub is_proposer: bool,
    /// Active delegation exposed since `active_since_ts`.
    pub active: u64,
    /// When `active` last went from zero to positive.
    pub active_since_ts: u64,
    /// Delegation added while the head had unresolved cases: a strike whose case opened before
    /// `fresh_since_ts` does not touch it. It joins `active` once no such case remains.
    pub fresh: u64,
    /// When `fresh` was first added.
    pub fresh_since_ts: u64,
    /// When `accrual_integral` was last brought up to date.
    pub accrual_since_ts: u64,
    /// `integral of (active + fresh) * live(t) dt` up to `accrual_since_ts`.
    pub accrual_integral: u128,
    /// Per board contract-ring slot: the contract period these captures belong to (default: none).
    /// A slot whose period differs from the board's is stale and restarts.
    pub capture_periods: [Pubkey; 4],
    /// The live-gated integral captured at that contract's listing.
    pub capture_listing: [u128; 4],
    /// The live-gated integral captured at that contract's freeze.
    pub capture_freeze: [u128; 4],
    /// Per slot: `CAPTURE_LISTING`, `CAPTURE_FREEZE` (and on a head `CAPTURE_JOINED`).
    pub capture_flags: [u8; 4],
    /// Strikes of the head already applied to this delegation.
    pub applied_strikes: u8,
    /// What the undelegated part would hold if still delegated (strikes come off it): the sum of
    /// `held_exposed`.
    pub held_principal: u64,
    /// AMBA still locked (cooling, then escrow), at most `held_principal`; 0 clears the held state.
    pub held: u64,
    /// When the latest cooling withdrawal's cooldown ends (0: none yet).
    pub cooldown_end_ts: u64,
    /// When it last undelegated (0: never).
    pub exit_ts: u64,
    /// The board reward index up to which rewards were accrued.
    pub reward_index: u128,
    /// Unclaimed delegator rewards (USDC atoms).
    pub rewards_accrued: u64,
    /// Rewards claimed so far.
    pub rewards_claimed: u64,
    /// AMBA slashed (burned) by strikes.
    pub slashed_total: u64,
    /// AMBA a lost source challenge took from it (paid to the winners).
    pub proposer_slashed: u64,
    /// Unix seconds.
    pub created_ts: u64,
    /// Unix seconds.
    pub updated_ts: u64,
    /// Who paid this account's rent: closing returns it to them, never to the owner (zero on an
    /// account made before the field existed: its owner paid).
    pub rent_payer: Pubkey,
    /// Zero since version 2.
    pub held_exempt: u64,
    /// The open case `held_case_adjust` is for (0: none): set while the delegation acts during a
    /// case, cleared once that case resolved and its strike applied.
    pub case_ref: u32,
    /// The head's case number (`source_case`) when `active` last went from zero to positive: only
    /// later cases strike it. Case numbers order cases against stake, whatever the timestamps.
    pub active_since_case: u32,
    /// The head's case number when `fresh` was first added (the case it is exempt from).
    pub fresh_since_case: u32,
    /// The head's case number at the last undelegation.
    pub exit_case: u32,
    /// Per capture slot: the generation (`HeadsBoard::contract_gens`) of the contract those
    /// captures belong to.
    pub capture_gen: [u32; 4],
    /// Per ring slot: the generation of the unsettled contract that `weighed[i]` and bit `i` of the
    /// held masks name (0: none).
    pub held_gens: [u32; 4],
    /// Zero since version 2.
    pub held_free: u64,
    /// The held principal by the contracts it weighed in (round 6 review, findings 3, 4 and 6):
    /// index = mask, bit `i` the contract of generation `held_gens[i]`; index 0, none still
    /// unsettled. Every mask has its own entry, so no exposure is ever joined to another; a case
    /// opened while one of a mask's contracts is unsettled strikes that mask's principal.
    pub held_exposed: [u64; 16],
    /// For the open case `case_ref`, per mask: of a mask meeting the case's contracts
    /// (`source_case_open_gens`), the part exempt from it (left from `fresh` during it); of any
    /// other mask, the part answering it by number (left from `active` during it, or reached
    /// through a contract that settled since).
    pub held_case_adjust: [u64; 16],
    /// Per ring slot: of `active + fresh`, the stake that weighed in the contract of generation
    /// `held_gens[i]` (delegated while its window ran and the head was live). It is the oldest of
    /// it: a withdrawal takes the newest first.
    pub weighed: [u64; 4],
    /// Withdrawals still cooling, by cooldown end (`cooling_end`); a fifth distinct end joins the
    /// two closest ends at the later one.
    pub cooling: [u64; 4],
    /// When each `cooling` amount's cooldown ends (0: free).
    pub cooling_end: [u64; 4],
    /// Its weight is below its share of the activation floor (`floor_activate / min_delegators`, on
    /// a board needing two delegators or more) as of its last change, so it is not one of the
    /// head's delegators (round 9 review). False on an account of before: such a delegation counted
    /// while its weight was positive.
    pub dust: bool,
}

impl Default for Delegation {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            head: Pubkey::new_from_array([0; 32]),
            owner: Pubkey::new_from_array([0; 32]),
            board: Pubkey::new_from_array([0; 32]),
            is_proposer: false,
            active: 0,
            active_since_ts: 0,
            fresh: 0,
            fresh_since_ts: 0,
            accrual_since_ts: 0,
            accrual_integral: 0,
            capture_periods: [Pubkey::new_from_array([0; 32]); 4],
            capture_listing: [0; 4],
            capture_freeze: [0; 4],
            capture_flags: [0; 4],
            applied_strikes: 0,
            held_principal: 0,
            held: 0,
            cooldown_end_ts: 0,
            exit_ts: 0,
            reward_index: 0,
            rewards_accrued: 0,
            rewards_claimed: 0,
            slashed_total: 0,
            proposer_slashed: 0,
            created_ts: 0,
            updated_ts: 0,
            rent_payer: Pubkey::new_from_array([0; 32]),
            held_exempt: 0,
            case_ref: 0,
            active_since_case: 0,
            fresh_since_case: 0,
            exit_case: 0,
            capture_gen: [0; 4],
            held_gens: [0; 4],
            held_free: 0,
            held_exposed: [0; 16],
            held_case_adjust: [0; 16],
            weighed: [0; 4],
            cooling: [0; 4],
            cooling_end: [0; 4],
            dust: false,
        }
    }
}

impl Delegation {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 1024;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 969;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDDELEGN";
    pub const VERSION: u8 = 3;
}

fixed_state_deserialize_flat!(Delegation, 969, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    head: Pubkey,
    owner: Pubkey,
    board: Pubkey,
    is_proposer: bool,
    active: u64,
    active_since_ts: u64,
    fresh: u64,
    fresh_since_ts: u64,
    accrual_since_ts: u64,
    accrual_integral: u128,
    capture_periods: [Pubkey; 4],
    capture_listing: [u128; 4],
    capture_freeze: [u128; 4],
    capture_flags: [u8; 4],
    applied_strikes: u8,
    held_principal: u64,
    held: u64,
    cooldown_end_ts: u64,
    exit_ts: u64,
    reward_index: u128,
    rewards_accrued: u64,
    rewards_claimed: u64,
    slashed_total: u64,
    proposer_slashed: u64,
    created_ts: u64,
    updated_ts: u64,
    rent_payer: Pubkey,
    held_exempt: u64,
    case_ref: u32,
    active_since_case: u32,
    fresh_since_case: u32,
    exit_case: u32,
    capture_gen: [u32; 4],
    held_gens: [u32; 4],
    held_free: u64,
    held_exposed: [u64; 16],
    held_case_adjust: [u64; 16],
    weighed: [u64; 4],
    cooling: [u64; 4],
    cooling_end: [u64; 4],
    dust: bool,
});

/// A fill or an update of a head: commit, reveal, the challenge window, and the aggregates that
/// reproduce `outcome::resolve_claim` without loading every challenge (challenges per claim are
/// unbounded; each one is paid by its own `SettleChallenge`, the index-last winner after every
/// other).
///
/// Seeds: `[ameba-oracle-v1, head-claim, head, seq (u32 LE)]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadClaim {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The head.
    pub head: Pubkey,
    /// Its board.
    pub board: Pubkey,
    /// Position among the head's claims.
    pub seq: u32,
    /// Fill or update.
    pub kind: HeadClaimKind,
    /// Committed, revealed, stood, fell or expired.
    pub status: HeadClaimStatus,
    /// Who filed it: anyone, for a fill of an Active head or an update of a Live one.
    pub fetcher: Pubkey,
    /// `heads_claim_commitment` of the revealed fields.
    pub commit_hash: [u8; 32],
    /// Slot of the commit; the reveal comes in a later slot.
    pub commit_slot: u64,
    /// Unix seconds.
    pub commit_ts: u64,
    /// Last slot of the reveal (v1's reveal window).
    pub reveal_deadline_slot: u64,
    /// Revealed value, six-decimal atoms.
    pub value: u64,
    /// Revealed source time.
    pub source_ts: u64,
    /// Revealed origin: the fetcher-declared hash of the underlying report's identity
    /// (challengeable).
    pub origin_id: [u8; 32],
    /// LTA channel: when the price took effect (0: none; always 0 off the LTA channel).
    pub lta_effective_from_ts: u64,
    /// LTA channel: the next reset (0: none).
    pub lta_next_reset_ts: u64,
    /// LTA channel: the floor (0: none).
    pub lta_floor: u64,
    /// LTA channel: the ceiling (0: none).
    pub lta_ceiling: u64,
    /// 0 none, or the sealed evidence object's kind (2, 3 or 4).
    pub evidence_kind: u8,
    /// That object's commitment (zero for none).
    pub evidence_commitment: [u8; 32],
    /// That object (default for none).
    pub evidence_object: Pubkey,
    /// USDC bond (`claim_bond`).
    pub bond: u64,
    /// When it was revealed: the challenge window starts.
    pub revealed_ts: u64,
    /// End of the challenge window, late challenges extending it.
    pub window_end_ts: u64,
    /// Challenges filed.
    pub challenge_count: u16,
    /// Of them, ruled.
    pub ruled_count: u16,
    /// Of them, won.
    pub won_count: u16,
    /// Index of the last won challenge (pro-rata remainder).
    pub last_winner: u16,
    /// Challenges paid.
    pub settled_count: u16,
    /// Won challenges paid.
    pub settled_winners: u16,
    /// Sum of the challenges' bonds.
    pub challenge_bond_total: u64,
    /// Sum of the won challenges' bonds.
    pub winning_bond_total: u64,
    /// Claim-bond shares paid to winners so far.
    pub shares_paid: u64,
    /// Losing challengers' bonds burned when the claim fell.
    pub bonds_burned: u64,
    /// USDC paid to the fetcher when the claim stood.
    pub to_claimant: u64,
    /// When `FinalizeClaim` ran (0: not yet).
    pub finalized_ts: u64,
    /// The prize it earned when it stood (USDC atoms before pro-rata scaling).
    pub prize_points: u64,
    /// Calendar month (`time::month_id`) the prize belongs to.
    pub epoch: u32,
    /// Prize paid.
    pub prize_paid: u64,
    /// `ClaimPrize` ran.
    pub prize_claimed: bool,
    /// Who paid this account's rent: closing returns it to them, never to the owner (zero on an
    /// account made before the field existed: its owner paid).
    pub rent_payer: Pubkey,
    /// Its place among the head's reveals (`Head::reveals`): a ring slot that started after it
    /// waits for it (`Head::restart_reveals`).
    pub reveal_index: u32,
    /// When its last challenge was ruled (0: none): ruled after a contract's cutoff, it is never
    /// admitted to that contract.
    pub last_ruled_ts: u64,
    /// Its ruling deadline, fixed by its first challenge (the window's end then plus the board's
    /// ruling deadline); late challenges extend the window only up to it. From it no ruling is
    /// taken and the claim resolves on the rulings it has, each unruled challenge timing out alone
    /// (its bond back); a claim with one stands for its bonds but prints nothing (early review B2,
    /// B5, B6).
    pub ruling_deadline_ts: u64,
    /// Sum of its ruled challenges' bonds: the claim resolves on these.
    pub ruled_bond_total: u64,
}

impl Default for HeadClaim {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            head: Pubkey::new_from_array([0; 32]),
            board: Pubkey::new_from_array([0; 32]),
            seq: 0,
            kind: HeadClaimKind::default(),
            status: HeadClaimStatus::default(),
            fetcher: Pubkey::new_from_array([0; 32]),
            commit_hash: [0; 32],
            commit_slot: 0,
            commit_ts: 0,
            reveal_deadline_slot: 0,
            value: 0,
            source_ts: 0,
            origin_id: [0; 32],
            lta_effective_from_ts: 0,
            lta_next_reset_ts: 0,
            lta_floor: 0,
            lta_ceiling: 0,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            evidence_object: Pubkey::new_from_array([0; 32]),
            bond: 0,
            revealed_ts: 0,
            window_end_ts: 0,
            challenge_count: 0,
            ruled_count: 0,
            won_count: 0,
            last_winner: 0,
            settled_count: 0,
            settled_winners: 0,
            challenge_bond_total: 0,
            winning_bond_total: 0,
            shares_paid: 0,
            bonds_burned: 0,
            to_claimant: 0,
            finalized_ts: 0,
            prize_points: 0,
            epoch: 0,
            prize_paid: 0,
            prize_claimed: false,
            rent_payer: Pubkey::new_from_array([0; 32]),
            reveal_index: 0,
            last_ruled_ts: 0,
            ruling_deadline_ts: 0,
            ruled_bond_total: 0,
        }
    }
}

impl HeadClaim {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 512;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 478;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDCLAIM_";
    pub const VERSION: u8 = 1;
}

fixed_state_deserialize_flat!(HeadClaim, 478, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    head: Pubkey,
    board: Pubkey,
    seq: u32,
    kind: HeadClaimKind,
    status: HeadClaimStatus,
    fetcher: Pubkey,
    commit_hash: [u8; 32],
    commit_slot: u64,
    commit_ts: u64,
    reveal_deadline_slot: u64,
    value: u64,
    source_ts: u64,
    origin_id: [u8; 32],
    lta_effective_from_ts: u64,
    lta_next_reset_ts: u64,
    lta_floor: u64,
    lta_ceiling: u64,
    evidence_kind: u8,
    evidence_commitment: [u8; 32],
    evidence_object: Pubkey,
    bond: u64,
    revealed_ts: u64,
    window_end_ts: u64,
    challenge_count: u16,
    ruled_count: u16,
    won_count: u16,
    last_winner: u16,
    settled_count: u16,
    settled_winners: u16,
    challenge_bond_total: u64,
    winning_bond_total: u64,
    shares_paid: u64,
    bonds_burned: u64,
    to_claimant: u64,
    finalized_ts: u64,
    prize_points: u64,
    epoch: u32,
    prize_paid: u64,
    prize_claimed: bool,
    rent_payer: Pubkey,
    reveal_index: u32,
    last_ruled_ts: u64,
    ruling_deadline_ts: u64,
    ruled_bond_total: u64,
});

/// A bonded challenge of a claim (target: the claim) or of a source (target: the head). It is
/// ruled by three current council seats; `SettleChallenge` pays it and closes it, so the same
/// challenger can challenge the same target again in a later case.
///
/// Seeds: `[ameba-oracle-v1, head-challenge, target, challenger]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadChallenge {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The claim, or the head for a source challenge.
    pub target: Pubkey,
    /// The head.
    pub head: Pubkey,
    /// Who filed it.
    pub challenger: Pubkey,
    /// Claim or source.
    pub kind: HeadChallengeKind,
    /// The ground: value or other, off channel, duplicate origin, expired observation. A label
    /// only: same bond, resolution and slash.
    pub reason: HeadChallengeReason,
    /// Position among the claim's (or the source case's) challenges.
    pub index: u16,
    /// The claim's seq, or the head's source case number.
    pub case_number: u32,
    /// USDC bond (at least `challenge_bond`).
    pub bond: u64,
    /// A claim challenge's alternative value (0: none); a competing fill carries its value here.
    pub alternative_value: u64,
    /// Its source time.
    pub alternative_source_ts: u64,
    /// 0 none, or the sealed evidence object's kind (2, 3 or 4).
    pub evidence_kind: u8,
    /// That object's commitment.
    pub evidence_commitment: [u8; 32],
    /// That object (default for none).
    pub evidence_object: Pubkey,
    /// Commitment of the challenger's reason text.
    pub reason_hash: [u8; 32],
    /// Unix seconds.
    pub opened_ts: u64,
    /// Pending, won or lost.
    pub ruling: HeadChallengeRuling,
    /// When it was ruled.
    pub ruled_ts: u64,
    /// The council epoch of the ruling.
    pub ruled_epoch: u64,
    /// USDC paid to the challenger.
    pub payout_usdc: u64,
    /// AMBA paid to the challenger's stake (won source challenge).
    pub payout_amba: u64,
    /// Paid.
    pub settled: bool,
    /// Who paid this account's rent: closing returns it to them, never to the owner (zero on an
    /// account made before the field existed: its owner paid).
    pub rent_payer: Pubkey,
}

impl Default for HeadChallenge {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            target: Pubkey::new_from_array([0; 32]),
            head: Pubkey::new_from_array([0; 32]),
            challenger: Pubkey::new_from_array([0; 32]),
            kind: HeadChallengeKind::default(),
            reason: HeadChallengeReason::default(),
            index: 0,
            case_number: 0,
            bond: 0,
            alternative_value: 0,
            alternative_source_ts: 0,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            evidence_object: Pubkey::new_from_array([0; 32]),
            reason_hash: [0; 32],
            opened_ts: 0,
            ruling: HeadChallengeRuling::default(),
            ruled_ts: 0,
            ruled_epoch: 0,
            payout_usdc: 0,
            payout_amba: 0,
            settled: false,
            rent_payer: Pubkey::new_from_array([0; 32]),
        }
    }
}

impl HeadChallenge {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 384;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 309;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDCHALLG";
    pub const VERSION: u8 = 1;
}

fixed_state_deserialize_flat!(HeadChallenge, 309, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    target: Pubkey,
    head: Pubkey,
    challenger: Pubkey,
    kind: HeadChallengeKind,
    reason: HeadChallengeReason,
    index: u16,
    case_number: u32,
    bond: u64,
    alternative_value: u64,
    alternative_source_ts: u64,
    evidence_kind: u8,
    evidence_commitment: [u8; 32],
    evidence_object: Pubkey,
    reason_hash: [u8; 32],
    opened_ts: u64,
    ruling: HeadChallengeRuling,
    ruled_ts: u64,
    ruled_epoch: u64,
    payout_usdc: u64,
    payout_amba: u64,
    settled: bool,
    rent_payer: Pubkey,
});

/// One contract (a `Period` of the product) listed and settled on the board: what it listed with,
/// frozen (parameters, items with basket weights and mix, components with their sentences, the
/// settlement target, the listing terms document, the live sources and their openings), and the
/// settlement record (per component its channel median or degraded flag, per item its settled
/// change and its fixed-mix reference, with the channels it holds at their last valid value).
///
/// `FixedMixReference` settles `index = sum over items of basket weight * the item's
/// reference`, the reference being `sum over channels of mix share * that component's change`,
/// each component live or at its degraded path's last valid value, so the mix never shifts; it
/// lists on assumed or matched mixes (the user, 2026-10-29: "SPOT LTA AND CONTRACT NEED TO BE
/// WEIGHTED AGAINST EACHOTHER"). `ContractChannel` (the brief's Stage 1, still a valid target)
/// settles on each item's Contract component alone and records the reference for display.
///
/// Seeds: `[ameba-oracle-v1, heads-contract, period]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsContract {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The period.
    pub period: Pubkey,
    /// The board.
    pub board: Pubkey,
    /// The product.
    pub product_id: [u8; 32],
    /// Its slot in the board's contract ring.
    pub ring_slot: u8,
    /// Accumulating, collecting or settled.
    pub status: HeadsContractStatus,
    /// What settles: each item's fixed-mix reference, or its Contract channel alone.
    pub settlement_target: HeadsSettlementTarget,
    /// The board's parameters when it listed.
    pub params: HeadsParams,
    /// Their version.
    pub params_version: u32,
    /// Six-decimal base value set at listing.
    pub base: u64,
    /// Start of the accumulation window (the period's effective listing).
    pub listing_ts: u64,
    /// End of the accumulation window.
    pub freeze_ts: u64,
    /// The period's expiry.
    pub expiry_ts: u64,
    /// Commitment of the listing terms document (a sealed kind-2 object: products, basket weights,
    /// sentences with terms, mix with basis and citation, settlement target, the disclosure
    /// sentence).
    pub terms_hash: [u8; 32],
    /// The board's items at listing.
    pub item_count: u8,
    /// Per item: bucket id.
    pub item_bucket_ids: [[u8; 32]; 8],
    /// Per item: basket weight, frozen at listing.
    pub item_weight_bps: [u16; 8],
    /// Per item, at `item * 3 + channel`: the mix, frozen at listing.
    pub item_mix_bps: [u16; 24],
    /// Per item: `HeadsMixBasis` byte, frozen at listing.
    pub item_mix_basis: [u8; 8],
    /// Per item: the change that settles (its reference, or its Contract component's under
    /// `ContractChannel`).
    pub item_change_ppm: [i64; 8],
    /// Per item: the fixed-mix reference, `sum over channels of mix share * component change` (each
    /// live or at its last valid value), when available.
    pub item_reference_ppm: [i64; 8],
    /// Per item: `REFERENCE_*` flags (computed, available, assumed).
    pub item_reference_flags: [u8; 8],
    /// Per item: `REFERENCE_REASON_*` (`channels::Unavailable::code`) when the reference is
    /// unavailable (2 mix basis none, 4 invalid mix).
    pub item_reference_reason: [u8; 8],
    /// Per item: bit `1 << channel` for each channel the mix weights that settled on its degraded
    /// path, so the reference holds its last valid value at its fixed share
    /// (`channels::Reference::degraded_mask`).
    pub item_degraded_channels: [u8; 8],
    /// The board's components at listing.
    pub component_count: u8,
    /// Per component: item index.
    pub component_items: [u8; 24],
    /// Per component: `HeadsChannel` byte.
    pub component_channels: [u8; 24],
    /// Per component: its target sentence's commitment, frozen at listing.
    pub component_sentence_hashes: [[u8; 32]; 24],
    /// Per component: live sources at listing.
    pub component_listed_sources: [u16; 24],
    /// Per component: distinct live origins at listing.
    pub component_listed_origins: [u16; 24],
    /// Per component: distinct live origins at freeze (`COMPONENT_FREEZE_CHECKED` once recorded,
    /// `COMPONENT_FREEZE_PASSED` when `channels::origins_pass`).
    pub component_freeze_origins: [u16; 24],
    /// Per component: `DEGRADED_*` (`settle::Degraded`) when it settled on its last valid value,
    /// else 0.
    pub component_degraded: [u8; 24],
    /// Per component: otherwise eligible sources dropped at settlement as duplicate origins.
    pub component_duplicates: [u16; 24],
    /// Per component: `COMPONENT_*` flags of the settlement record.
    pub component_flags: [u8; 24],
    /// Per component: its channel median change.
    pub component_change_ppm: [i64; 24],
    /// Per component: the change at freeze (degraded rule 2's fallback).
    pub component_freeze_change_ppm: [i64; 24],
    /// Per component: eligible sources at settlement, one per independent origin.
    pub component_eligible: [u16; 24],
    /// Per component: the eligible sources' raw weight.
    pub component_weight: [u64; 24],
    /// Components settled.
    pub components_settled: u8,
    /// Sources live at listing with a price in effect then, backed or not (an LTA price past its
    /// reset is not listed: round 4 N5; an unbacked one is a member but no origin: round 10 review,
    /// N1).
    pub listed_count: u16,
    /// Sources live at listing: head id.
    pub listed_head_ids: [u32; 96],
    /// Sources live at listing: component index.
    pub listed_components: [u8; 96],
    /// Sources live at listing: `LIVE_DUPLICATE` as the board had it.
    pub listed_flags: [u8; 96],
    /// Sources live at listing: opening (their value then).
    pub listed_openings: [u64; 96],
    /// `settle::index_change_ppm` of the items.
    pub index_change_ppm: i64,
    /// `settle::settlement_value(base, index_change_ppm)`.
    pub value: u64,
    /// Digest of the settlement record, written to the period.
    pub digest: [u8; 32],
    /// Unix seconds (0: not settled).
    pub settled_ts: u64,
    /// The oracle authority that listed it.
    pub listed_by: Pubkey,
    /// Unix seconds.
    pub updated_ts: u64,
}

#[cfg(not(target_os = "solana"))]
impl Default for HeadsContract {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            period: Pubkey::new_from_array([0; 32]),
            board: Pubkey::new_from_array([0; 32]),
            product_id: [0; 32],
            ring_slot: 0,
            status: HeadsContractStatus::default(),
            settlement_target: HeadsSettlementTarget::default(),
            params: HeadsParams::zero(),
            params_version: 0,
            base: 0,
            listing_ts: 0,
            freeze_ts: 0,
            expiry_ts: 0,
            terms_hash: [0; 32],
            item_count: 0,
            item_bucket_ids: [[0; 32]; 8],
            item_weight_bps: [0; 8],
            item_mix_bps: [0; 24],
            item_mix_basis: [0; 8],
            item_change_ppm: [0; 8],
            item_reference_ppm: [0; 8],
            item_reference_flags: [0; 8],
            item_reference_reason: [0; 8],
            item_degraded_channels: [0; 8],
            component_count: 0,
            component_items: [0; 24],
            component_channels: [0; 24],
            component_sentence_hashes: [[0; 32]; 24],
            component_listed_sources: [0; 24],
            component_listed_origins: [0; 24],
            component_freeze_origins: [0; 24],
            component_degraded: [0; 24],
            component_duplicates: [0; 24],
            component_flags: [0; 24],
            component_change_ppm: [0; 24],
            component_freeze_change_ppm: [0; 24],
            component_eligible: [0; 24],
            component_weight: [0; 24],
            components_settled: 0,
            listed_count: 0,
            listed_head_ids: [0; 96],
            listed_components: [0; 96],
            listed_flags: [0; 96],
            listed_openings: [0; 96],
            index_change_ppm: 0,
            value: 0,
            digest: [0; 32],
            settled_ts: 0,
            listed_by: Pubkey::new_from_array([0; 32]),
            updated_ts: 0,
        }
    }
}

impl HeadsContract {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 4608;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 3902;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDCONTRT";
    pub const VERSION: u8 = 1;
}

#[cfg(not(target_os = "solana"))]
fixed_state_deserialize_flat!(HeadsContract, 3902, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    period: Pubkey,
    board: Pubkey,
    product_id: [u8; 32],
    ring_slot: u8,
    status: HeadsContractStatus,
    settlement_target: HeadsSettlementTarget,
    params: HeadsParams,
    params_version: u32,
    base: u64,
    listing_ts: u64,
    freeze_ts: u64,
    expiry_ts: u64,
    terms_hash: [u8; 32],
    item_count: u8,
    item_bucket_ids: [[u8; 32]; 8],
    item_weight_bps: [u16; 8],
    item_mix_bps: [u16; 24],
    item_mix_basis: [u8; 8],
    item_change_ppm: [i64; 8],
    item_reference_ppm: [i64; 8],
    item_reference_flags: [u8; 8],
    item_reference_reason: [u8; 8],
    item_degraded_channels: [u8; 8],
    component_count: u8,
    component_items: [u8; 24],
    component_channels: [u8; 24],
    component_sentence_hashes: [[u8; 32]; 24],
    component_listed_sources: [u16; 24],
    component_listed_origins: [u16; 24],
    component_freeze_origins: [u16; 24],
    component_degraded: [u8; 24],
    component_duplicates: [u16; 24],
    component_flags: [u8; 24],
    component_change_ppm: [i64; 24],
    component_freeze_change_ppm: [i64; 24],
    component_eligible: [u16; 24],
    component_weight: [u64; 24],
    components_settled: u8,
    listed_count: u16,
    listed_head_ids: [u32; 96],
    listed_components: [u8; 96],
    listed_flags: [u8; 96],
    listed_openings: [u64; 96],
    index_change_ppm: i64,
    value: u64,
    digest: [u8; 32],
    settled_ts: u64,
    listed_by: Pubkey,
    updated_ts: u64,
}, flat {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    period: Pubkey,
    board: Pubkey,
    product_id: [u8; 32],
    ring_slot: u8,
    status: HeadsContractStatus,
    settlement_target: HeadsSettlementTarget,
    params.slash_rate_bps: u16,
    params.max_strikes: u8,
    params.proposer_slash_bps: u16,
    params.prize_cut_bps: u16,
    params.base_fill_prize: u64,
    params.fill_prize_slope_ppb: u64,
    params.base_update_prize: u64,
    params.update_prize_slope_ppb: u64,
    params.claim_bond: u64,
    params.challenge_bond: u64,
    params.challenge_window_secs: u64,
    params.challenge_extension_secs: u64,
    params.head_ttl_secs: u64,
    params.cooldown_secs: u64,
    params.min_delegators: u16,
    params.min_live_sources: u16,
    params.min_settle_weight: u64,
    params.alpha_bps: u16,
    params.floor_post: u64,
    params.beta_bps: u16,
    params.floor_activate: u64,
    params.settlement_business_days: u8,
    params_version: u32,
    base: u64,
    listing_ts: u64,
    freeze_ts: u64,
    expiry_ts: u64,
    terms_hash: [u8; 32],
    item_count: u8,
    item_bucket_ids: [[u8; 32]; 8],
    item_weight_bps: [u16; 8],
    item_mix_bps: [u16; 24],
    item_mix_basis: [u8; 8],
    item_change_ppm: [i64; 8],
    item_reference_ppm: [i64; 8],
    item_reference_flags: [u8; 8],
    item_reference_reason: [u8; 8],
    item_degraded_channels: [u8; 8],
    component_count: u8,
    component_items: [u8; 24],
    component_channels: [u8; 24],
    component_sentence_hashes: [[u8; 32]; 24],
    component_listed_sources: [u16; 24],
    component_listed_origins: [u16; 24],
    component_freeze_origins: [u16; 24],
    component_degraded: [u8; 24],
    component_duplicates: [u16; 24],
    component_flags: [u8; 24],
    component_change_ppm: [i64; 24],
    component_freeze_change_ppm: [i64; 24],
    component_eligible: [u16; 24],
    component_weight: [u64; 24],
    components_settled: u8,
    listed_count: u16,
    listed_head_ids: [u32; 96],
    listed_components: [u8; 96],
    listed_flags: [u8; 96],
    listed_openings: [u64; 96],
    index_change_ppm: i64,
    value: u64,
    digest: [u8; 32],
    settled_ts: u64,
    listed_by: Pubkey,
    updated_ts: u64,
});

#[cfg(target_os = "solana")]
heads_heap_codec!(HeadsContract, 3902, flat {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    period: Pubkey,
    board: Pubkey,
    product_id: [u8; 32],
    ring_slot: u8,
    status: HeadsContractStatus,
    settlement_target: HeadsSettlementTarget,
    params.slash_rate_bps: u16,
    params.max_strikes: u8,
    params.proposer_slash_bps: u16,
    params.prize_cut_bps: u16,
    params.base_fill_prize: u64,
    params.fill_prize_slope_ppb: u64,
    params.base_update_prize: u64,
    params.update_prize_slope_ppb: u64,
    params.claim_bond: u64,
    params.challenge_bond: u64,
    params.challenge_window_secs: u64,
    params.challenge_extension_secs: u64,
    params.head_ttl_secs: u64,
    params.cooldown_secs: u64,
    params.min_delegators: u16,
    params.min_live_sources: u16,
    params.min_settle_weight: u64,
    params.alpha_bps: u16,
    params.floor_post: u64,
    params.beta_bps: u16,
    params.floor_activate: u64,
    params.settlement_business_days: u8,
    params_version: u32,
    base: u64,
    listing_ts: u64,
    freeze_ts: u64,
    expiry_ts: u64,
    terms_hash: [u8; 32],
    item_count: u8,
    item_bucket_ids: [[u8; 32]; 8],
    item_weight_bps: [u16; 8],
    item_mix_bps: [u16; 24],
    item_mix_basis: [u8; 8],
    item_change_ppm: [i64; 8],
    item_reference_ppm: [i64; 8],
    item_reference_flags: [u8; 8],
    item_reference_reason: [u8; 8],
    item_degraded_channels: [u8; 8],
    component_count: u8,
    component_items: [u8; 24],
    component_channels: [u8; 24],
    component_sentence_hashes: [[u8; 32]; 24],
    component_listed_sources: [u16; 24],
    component_listed_origins: [u16; 24],
    component_freeze_origins: [u16; 24],
    component_degraded: [u8; 24],
    component_duplicates: [u16; 24],
    component_flags: [u8; 24],
    component_change_ppm: [i64; 24],
    component_freeze_change_ppm: [i64; 24],
    component_eligible: [u16; 24],
    component_weight: [u64; 24],
    components_settled: u8,
    listed_count: u16,
    listed_head_ids: [u32; 96],
    listed_components: [u8; 96],
    listed_flags: [u8; 96],
    listed_openings: [u64; 96],
    index_change_ppm: i64,
    value: u64,
    digest: [u8; 32],
    settled_ts: u64,
    listed_by: Pubkey,
    updated_ts: u64,
});

/// A calendar month of one board's fetcher prizes: the pool funded during it and the prize points
/// earned by claims that stood during it. Prizes pay out after it ends, scaled pro rata when the
/// pool is
/// short (`rewards::prize_payout`).
///
/// Seeds: `[ameba-oracle-v1, heads-epoch, board, month_id (u32 LE)]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsEpoch {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The board.
    pub board: Pubkey,
    /// `time::month_id` of the month.
    pub month_id: u32,
    /// `time::month_start(month_id)`.
    pub start_ts: u64,
    /// `time::month_start(month_id + 1)`.
    pub end_ts: u64,
    /// USDC funded into the month's prizes.
    pub prize_pool: u64,
    /// Prize points earned in the month.
    pub prize_points: u64,
    /// Prizes paid.
    pub prize_paid: u64,
    /// Claims that earned points.
    pub claims: u32,
    /// Of them, paid.
    pub claims_paid: u32,
    /// Unix seconds.
    pub updated_ts: u64,
    /// Prize money no prize could claim, moved to a later month's pool once the month ended
    /// (`RollOverPrizes`).
    pub rolled_out: u64,
    /// Prize money rolled in from ended months.
    pub rolled_in: u64,
}

impl Default for HeadsEpoch {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            board: Pubkey::new_from_array([0; 32]),
            month_id: 0,
            start_ts: 0,
            end_ts: 0,
            prize_pool: 0,
            prize_points: 0,
            prize_paid: 0,
            claims: 0,
            claims_paid: 0,
            updated_ts: 0,
            rolled_out: 0,
            rolled_in: 0,
        }
    }
}

impl HeadsEpoch {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 160;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 118;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDEPOCH_";
    pub const VERSION: u8 = 1;
}

fixed_state_deserialize_flat!(HeadsEpoch, 118, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    board: Pubkey,
    month_id: u32,
    start_ts: u64,
    end_ts: u64,
    prize_pool: u64,
    prize_points: u64,
    prize_paid: u64,
    claims: u32,
    claims_paid: u32,
    updated_ts: u64,
    rolled_out: u64,
    rolled_in: u64,
});

/// The fixed head of a `HeadsWeights` account (one contract's component): its source records
/// follow (`HeadsWeightsLayout`). The first `SettleComponent` of the component creates it from the
/// contract's listed sources; sources that went live inside the window are appended by passing
/// their
/// heads. A source's weight is its head's own time-weighted delegation over the window: no caps, so
/// no
/// per-owner record is needed (the user, 2026-10-29). The call that completes the record reads in
/// full
/// only the records that can count (live at the cutoff, or at the freeze): at most the board's live
/// list each, however many sources joined and retired over the window (round 3 R5).
///
/// Seeds of the account: `[ameba-oracle-v1, heads-weights, contract, component (u8)]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsWeightsHeader {
    /// Account discriminator (`Self::DISCRIMINATOR`).
    pub discriminator: [u8; 8],
    /// Layout version (`Self::VERSION`).
    pub version: u8,
    /// Canonical PDA bump, checked on every load.
    pub bump: u8,
    /// The contract.
    pub contract: Pubkey,
    /// The board.
    pub board: Pubkey,
    /// Component index.
    pub component: u8,
    /// The component's channel.
    pub channel: HeadsChannel,
    /// Recording levels, or settled.
    pub phase: HeadsWeightsPhase,
    /// The component's bucket id.
    pub bucket_id: [u8; 32],
    /// `freeze_ts - listing_ts`: integrals divide by it into raw weights.
    pub window_secs: u64,
    /// Source records in use.
    pub source_count: u16,
    /// Source records allocated.
    pub source_capacity: u16,
    /// Sources whose settlement level is recorded, in record order.
    pub levels_recorded: u16,
    /// Sources eligible at settlement, one per independent origin.
    pub eligible_sources: u16,
    /// Otherwise eligible sources dropped as duplicate origins.
    pub duplicate_sources: u16,
    /// The eligible sources' raw weight.
    pub weight_total: u64,
    /// Unix seconds.
    pub updated_ts: u64,
    /// Records of sources the contract listed: the component is complete once these are all of its
    /// listed sources and `joined_recorded` all its joined ones, each recorded once (the head's
    /// `CAPTURE_RECORDED`), so completeness reads no record (round 3 R5).
    pub listed_recorded: u16,
    /// Records of sources that joined the window (`HeadsBoard::contract_joined`).
    pub joined_recorded: u16,
}

impl Default for HeadsWeightsHeader {
    fn default() -> Self {
        Self {
            discriminator: [0; 8],
            version: 0,
            bump: 0,
            contract: Pubkey::new_from_array([0; 32]),
            board: Pubkey::new_from_array([0; 32]),
            component: 0,
            channel: HeadsChannel::default(),
            phase: HeadsWeightsPhase::default(),
            bucket_id: [0; 32],
            window_secs: 0,
            source_count: 0,
            source_capacity: 0,
            levels_recorded: 0,
            eligible_sources: 0,
            duplicate_sources: 0,
            weight_total: 0,
            updated_ts: 0,
            listed_recorded: 0,
            joined_recorded: 0,
        }
    }
}

impl HeadsWeightsHeader {
    /// Account bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 192;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 147;
    pub const DISCRIMINATOR: [u8; 8] = *b"HDWEIGHT";
    pub const VERSION: u8 = 1;
}

fixed_state_deserialize_flat!(HeadsWeightsHeader, 147, {
    discriminator: [u8; 8],
    version: u8,
    bump: u8,
    contract: Pubkey,
    board: Pubkey,
    component: u8,
    channel: HeadsChannel,
    phase: HeadsWeightsPhase,
    bucket_id: [u8; 32],
    window_secs: u64,
    source_count: u16,
    source_capacity: u16,
    levels_recorded: u16,
    eligible_sources: u16,
    duplicate_sources: u16,
    weight_total: u64,
    updated_ts: u64,
    listed_recorded: u16,
    joined_recorded: u16,
});

/// One source record of a `HeadsWeights` account (`HEADS_WEIGHT_SOURCE_STRIDE` bytes, zero padded).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadsWeightSource {
    /// The head.
    pub head_id: u32,
    /// `WEIGHT_SOURCE_*` flags.
    pub flags: u8,
    /// The head's origin at settlement (`settle::SourceInput::origin`).
    pub origin_id: [u8; 32],
    /// When the head went live (`settle::SourceInput::live_at`: the earlier of two sources with one
    /// origin keeps it).
    pub live_at: u64,
    /// Its opening: its value at listing, or its fill value if it went live later.
    pub opening: u64,
    /// The head's live-gated delegation integral over the window
    /// (`transitions::head_window_integral`).
    pub integral: u128,
    /// Its raw weight, `time::average(integral, window_secs)` (`settle::SourceInput::weight`).
    pub weight: u64,
    /// Settlement level (`settle::temporal_level` over prints of claims committed before expiry; an
    /// LTA print past its own next reset is no current level).
    pub level: u64,
    /// Level as of the freeze (the fallback).
    pub freeze_level: u64,
    /// `settle::change_ppm(opening, level)`.
    pub change_ppm: i64,
    /// `settle::change_ppm(opening, freeze_level)`.
    pub freeze_change_ppm: i64,
    /// The head's origin at the freeze: the fallback's duplicates use it.
    pub freeze_origin_id: [u8; 32],
}

impl Default for HeadsWeightSource {
    fn default() -> Self {
        Self {
            head_id: 0,
            flags: 0,
            origin_id: [0; 32],
            live_at: 0,
            opening: 0,
            integral: 0,
            weight: 0,
            level: 0,
            freeze_level: 0,
            change_ppm: 0,
            freeze_change_ppm: 0,
            freeze_origin_id: [0; 32],
        }
    }
}

impl HeadsWeightSource {
    /// Record stride in bytes; everything past `WIRE_LEN` is zero.
    pub const LEN: usize = 160;
    /// Bytes of the encoded fields.
    pub const WIRE_LEN: usize = 141;
}

fixed_state_deserialize_flat!(HeadsWeightSource, 141, {
    head_id: u32,
    flags: u8,
    origin_id: [u8; 32],
    live_at: u64,
    opening: u64,
    integral: u128,
    weight: u64,
    level: u64,
    freeze_level: u64,
    change_ppm: i64,
    freeze_change_ppm: i64,
    freeze_origin_id: [u8; 32],
});
