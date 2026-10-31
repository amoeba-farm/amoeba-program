#!/usr/bin/env python3
"""Generates the heads account structs and their flat codec invocations from one table, so
every declared wire length is the exact sum of its fields. Output: Rust source on stdout."""
import re
import sys

SIZES = {
    "u8": 1, "bool": 1, "u16": 2, "u32": 4, "i32": 4, "u64": 8, "i64": 8, "u128": 16,
    "Pubkey": 32,
    "HeadStatus": 1, "HeadAccessClass": 1, "HeadClaimKind": 1, "HeadClaimStatus": 1,
    "HeadChallengeKind": 1, "HeadChallengeRuling": 1, "HeadsContractStatus": 1,
    "HeadsWeightsPhase": 1, "HeadsChannel": 1,
    "HeadsSettlementTarget": 1, "HeadChallengeReason": 1,
}
ENUMS = {k for k in SIZES if k[0].isupper() and k != "Pubkey"}

PARAMS = [
    ("slash_rate_bps", "u16", "Fraction of a delegation's remaining principal one strike slashes (burned). Spec: 10%."),
    ("max_strikes", "u8", "Lost challenges before a source retires (spec: 2); at most `HEAD_STRIKE_CAPACITY`."),
    ("proposer_slash_bps", "u16", "What a lost source challenge takes of the proposer's delegation. Spec: 100%."),
    ("prize_cut_bps", "u16", "Share of the board's inflow that funds the fetcher prize pool. Spec: 20%."),
    ("base_fill_prize", "u64", "`fill_prize = base_fill_prize + fill_prize_slope_ppb * delegation / 1e9` (USDC atoms)."),
    ("fill_prize_slope_ppb", "u64", "Fill prize slope, parts per billion of the head's delegation."),
    ("base_update_prize", "u64", "`update_prize = base_update_prize + update_prize_slope_ppb * delegation / 1e9`."),
    ("update_prize_slope_ppb", "u64", "Update prize slope, parts per billion of the source's delegation."),
    ("claim_bond", "u64", "The USDC bond a claim (fill or update) carries."),
    ("challenge_bond", "u64", "The least USDC bond a claim or source challenge carries."),
    ("challenge_window_secs", "u64", "How long a claim (or a source case) stays open to challenge."),
    ("challenge_extension_secs", "u64", "How far a late challenge pushes the window out so it can be judged."),
    ("head_ttl_secs", "u64", "An Open head that has not activated by then expires. Spec: 14 days."),
    ("cooldown_secs", "u64", "The cooldown on undelegation. Spec: 14 days."),
    ("min_delegators", "u16", "Delegators a head needs to activate. Spec: 2."),
    ("min_live_sources", "u16", "Independent origins a component needs to list and to settle on live data (default 1: no hard number limit, the user 2026-10-29)."),
    ("min_settle_weight", "u64", "Raw weight (time-weighted delegation) a component's eligible sources need to settle on live data."),
    ("alpha_bps", "u16", "`min_post = max(floor_post, alpha * D / max(n, 1))`. Spec: 0.25."),
    ("floor_post", "u64", "The least delegation a post locks (AMBA atoms)."),
    ("beta_bps", "u16", "`activation_threshold = max(floor_activate, beta * m)`. Spec: 0.5."),
    ("floor_activate", "u64", "The least delegation a head needs to activate (AMBA atoms)."),
    ("settlement_business_days", "u8", "Business days before expiry whose prints make a source's settlement level."),
]

HDR = [
    ("discriminator", "[u8; 8]", "Account discriminator (`Self::DISCRIMINATOR`)."),
    ("version", "u8", "Layout version (`Self::VERSION`)."),
    ("bump", "u8", "Canonical PDA bump, checked on every load."),
]

CAPTURES = [
    ("capture_periods", "[Pubkey; 4]", "Per board contract-ring slot: the contract period these captures belong to (default: none). A slot whose period differs from the board's is stale and restarts."),
    ("capture_listing", "[u128; 4]", "The live-gated integral captured at that contract's listing."),
    ("capture_freeze", "[u128; 4]", "The live-gated integral captured at that contract's freeze."),
    ("capture_flags", "[u8; 4]", "Per slot: `CAPTURE_LISTING`, `CAPTURE_FREEZE` (and on a head `CAPTURE_JOINED`)."),
]

ACCOUNTS = [
    dict(
        name="HeadsConfig",
        doc="""The family's singleton: the AMBA mint and token program, and the AMBA vault, the classic or
Token-2022 associated token account of the `heads-authority` PDA for that mint (the client creates it
with the associated-token program; the program validates it and never creates it).

Seeds: `[ameba-oracle-v1, heads-config]`.""",
        disc="HDCONFIG", len=256,
        fields=HDR + [
            ("authority_bump", "u8", "Canonical bump of the `heads-authority` PDA that owns the vault."),
            ("amba_decimals", "u8", "The AMBA mint's decimals, read from the mint at initialization."),
            ("amba_mint", "Pubkey", "The AMBA mint."),
            ("amba_token_program", "Pubkey", "Its token program: classic SPL Token or Token-2022."),
            ("amba_vault", "Pubkey", "The vault: `ATA(heads-authority, amba_mint, amba_token_program)`."),
            ("amba_accounted", "u64", "Every AMBA atom the family owes someone: free, delegated and held stake and undelivered source-challenge awards. The vault holds at least this plus `pending_amba_burn`."),
            ("pending_amba_burn", "u64", "Slashed AMBA not yet burned (`BurnPending`)."),
            ("amba_burned_total", "u64", "AMBA burned so far."),
            ("board_count", "u32", "Boards created."),
            ("created_ts", "u64", "Unix seconds."),
            ("updated_ts", "u64", "Unix seconds."),
        ],
    ),
    dict(
        name="HeadsBoard",
        doc="""One product's board (spec "bucket"): parameters, items, components, money, the live-source
list and the ring of open contracts.

Items are the product's basket members (today's commodity buckets) with their basket weights and
their channel mix. Components are `(item, channel)` pairs in the mold's order, component index =
item * 3 + channel (`docs/lta.md`): each has its own frozen target sentence, its own heads and its
own weighted median. Every item has its three components; a component whose sentence is still zero
takes no heads and cannot list. Items and sentences change only between listings (`SetItem`,
`SetComponent`); a component's item and channel never change, since heads are posted against it.

Seeds: `[ameba-oracle-v1, heads-board, product_id]`.""",
        disc="HDBOARD_", len=10240, version=3,
        fields=HDR + [
            ("product_id", "[u8; 32]", "The product (ASCII, zero padded), as in its periods."),
            ("params", "HeadsParams", "Current parameters; a contract copies them when it lists."),
            ("params_version", "u32", "Starts at 1; `SetBoardParams` increments it."),
            ("item_count", "u8", "Items in use, 1 to `HEADS_MAX_ITEMS`."),
            ("item_bucket_ids", "[[u8; 32]; 8]", "Per item: its commodity bucket id (`commodity-spec`)."),
            ("item_weight_bps", "[u16; 8]", "Per item: basket weight; a listing needs the items' weights to add up to 10,000."),
            ("item_mix_bps", "[u16; 24]", "Per item, at `item * 3 + channel`: the fixed-mix reference's channel shares (spot, contract, LTA), adding up to 10,000."),
            ("item_mix_basis", "[u8; 8]", "Per item: `HeadsMixBasis` of the mix as a byte (none, assumed scenario, matched evidence)."),
            ("component_count", "u8", "Components in use: three per item (`item_count * 3`)."),
            ("component_items", "[u8; 24]", "Per component: its item index (`index / 3`)."),
            ("component_channels", "[u8; 24]", "Per component: `HeadsChannel` as a byte (`index % 3`: spot, contract, LTA)."),
            ("component_sentence_hashes", "[[u8; 32]; 24]", "Per component: commitment of its target sentence with the bracketed terms filled (a sealed kind-2 evidence object whose commitment is this hash); zero until set."),
            ("component_live_sources", "[u16; 24]", "Per component: live sources."),
            ("component_live_origins", "[u16; 24]", "Per component: distinct origins among its live sources (duplicates excluded); `min_live_sources` counts these."),
            ("head_count", "u32", "Heads posted; the next head's id."),
            ("total_delegation", "u64", "`D` of spec section 6: active delegation on every head that is not retired or expired."),
            ("live_delegation", "u64", "Active (and fresh) delegation on Live heads that are not duplicates: the reward index's denominator."),
            ("live_count", "u16", "Entries in the live-source list."),
            ("live_head_ids", "[u32; 96]", "Live-source list: head id."),
            ("live_components", "[u8; 96]", "Live-source list: component index."),
            ("live_flags", "[u8; 96]", "Live-source list: `LIVE_DUPLICATE` when another live source of the same component has the same origin and precedes it (earlier `live_since_ts`, then lower head id: `channels::duplicate_flags`). Recomputed whenever a source goes live or retires or an accepted print declares a new origin."),
            ("live_since_ts", "[u64; 96]", "Live-source list: when the head went live (duplicate precedence)."),
            ("live_delegations", "[u64; 96]", "Live-source list: the head's active delegation (thresholds use their median)."),
            ("live_values", "[u64; 96]", "Live-source list: the head's current value (a listing reads its openings here)."),
            ("live_origin_ids", "[[u8; 32]; 96]", "Live-source list: the origin of the head's last accepted print."),
            ("live_reward_offsets", "[u128; 96]", "Live-source list: the head's delegator reward index, encoded so funding never touches the list: `reward_index - offset` while the entry earns (not a duplicate), the frozen value `offset` while it does not. A flip re-encodes it (`transitions::set_live_flags`)."),
            ("reward_index", "u128", "Delegator reward index (`rewards::INDEX_SCALE` fixed point)."),
            ("reserve", "u64", "Delegator share funded while nothing was live, distributed by the next funding with live delegation (USDC atoms)."),
            ("delegator_funded_total", "u64", "USDC routed to delegators."),
            ("delegator_paid_total", "u64", "USDC claimed by delegators."),
            ("prize_funded_total", "u64", "USDC routed to prize epochs."),
            ("prize_paid_total", "u64", "USDC paid as prizes."),
            ("pending_usdc_burn", "u64", "Burned USDC (losing challenge bonds of a falling claim) still in the bond vault (`BurnPending`)."),
            ("usdc_burned_total", "u64", "USDC burned so far."),
            ("contract_periods", "[Pubkey; 4]", "Contract ring: the period of each open contract (default: empty slot)."),
            ("contract_listing_ts", "[u64; 4]", "Contract ring: listing time."),
            ("contract_freeze_ts", "[u64; 4]", "Contract ring: freeze time (end of the accumulation window)."),
            ("contract_expiry_ts", "[u64; 4]", "Contract ring: expiry."),
            ("contract_status", "[u8; 4]", "Contract ring: the slot's state, `RING_EMPTY`, `RING_ACCUMULATING`, `RING_COLLECTING` or `RING_SETTLED`."),
            ("contract_flags", "[u8; 4]", "Contract ring: `RING_FREEZE_CAPTURED` once the components' origins at freeze are captured."),
            ("contract_freeze_origins", "[u16; 96]", "Contract ring, at `slot * 24 + component`: distinct live origins as of the freeze. The first instruction after the freeze that changes `component_live_origins` captures them first (the counts were constant since the last change, so the capture is exact)."),
            ("contract_count", "u32", "Contracts listed."),
            ("created_ts", "u64", "Unix seconds."),
            ("updated_ts", "u64", "Unix seconds."),
            ("contract_freeze_window_ts", "[u64; 4]", "Contract ring: start of the freeze level's window (`time::settlement_window_start` of the freeze on the contract's parameters)."),
            ("contract_gens", "[u32; 4]", "Contract ring: the contract's generation, `contract_count` when it listed (unique per board, never reused like a slot). Exposure to a contract is held by generation: a source case records the generations open when it opened, a delegation those it contributed to."),
            ("contract_joined", "[u16; 96]", "Contract ring, at `slot * 24 + component`: sources that went live after the listing and before the freeze (each marks itself `CAPTURE_JOINED`). With the listed ones they are the contract's members: `SettleComponent` settles a component only once it recorded all of them, retired or not."),
            ("ruling_deadline_secs", "u64", "How long after its window a claim or source case may wait for its rulings; past it an unruled one is void (every bond back, nothing prints, no strike). 0: `DEFAULT_RULING_DEADLINE_SECS`. A protection against funds locked for ever (re-review #13)."),
            ("contract_cutoff_ts", "[u64; 4]", "Contract ring: the cutoff K, the expiry plus the contract's challenge window: admission to the contract is decided from facts as of K, and records open after it."),
            ("live_reset_ts", "[u32; 96]", "Live list: the entry's LTA price's next reset (unix seconds, at most `u32::MAX`; 0: none, and always 0 off the LTA channel). A listing counts and opens an entry only while its price is in effect (`HeadsBoard::live_in_effect`, round 4 N5)."),
            ("live_delegators", "[u8; 96]", "Live list: the head's delegators (`Head::delegators`, at most 255; `min_delegators` is at most 255). With `live_delegations` they say whether the entry still meets activation backing (`HeadsBoard::live_backed`): listing and freeze coverage count only entries that do (round 6 review, finding 1)."),
        ],
    ),
    dict(
        name="HeadsStake",
        doc="""One owner's AMBA in the family: free stake, and counters of what it has delegated and holds in
cooldown or escrow.

Seeds: `[ameba-oracle-v1, heads-stake, owner]`.""",
        disc="HDSTAKE_", len=192,
        fields=HDR + [
            ("owner", "Pubkey", "The owner."),
            ("free", "u64", "Staked and not delegated: `Unstake` withdraws it at once."),
            ("delegated_total", "u64", "AMBA moved from free stake onto delegations, ever."),
            ("returned_total", "u64", "AMBA released from delegations back to free stake, ever (what is out is `delegated_total - returned_total` less slashes; the delegations hold the detail)."),
            ("staked_in_total", "u64", "AMBA credited by `Stake` (what arrived, transfer fee off)."),
            ("unstaked_out_total", "u64", "AMBA sent out by `Unstake`."),
            ("awarded_total", "u64", "AMBA received from won source challenges."),
            ("delegation_count", "u32", "Open `Delegation` accounts."),
            ("created_ts", "u64", "Unix seconds."),
            ("updated_ts", "u64", "Unix seconds."),
        ],
    ),
    dict(
        name="Head",
        doc="""A proposed source (spec "Head"), a source once Live. It keeps the live-gated integral of its
total delegation and that integral's captures at each open contract's listing and freeze, its strikes
with the opening time of the case that caused each (delegations apply them lazily), its reward-index
window, its accepted prints and the bookkeeping of its claims and of its current source case.

Seeds: `[ameba-oracle-v1, head, board, head_id (u32 LE)]`.""",
        disc="HDHEAD__", len=4096, version=2,
        fields=HDR + [
            ("board", "Pubkey", "The board."),
            ("head_id", "u32", "Position on the board (0-based)."),
            ("component", "u8", "Component index on the board."),
            ("channel", "HeadsChannel", "The component's channel, fixed when the head is posted."),
            ("access_class", "HeadAccessClass", "How a claim obtains the value."),
            ("status", "HeadStatus", "Open, Active, Live, Retired or Expired."),
            ("name_len", "u8", "Bytes of `name` in use (UTF-8, at most `HEAD_NAME_MAX_BYTES`); the rest are zero."),
            ("name", "[u8; 64]", "Human-readable name, e.g. \"Mouser DDR5-4800 16Gb spot\"."),
            ("spec_hash", "[u8; 32]", "Commitment of the sealed kind-2 evidence object that holds the spec (it restates the component's target sentence)."),
            ("spec_object", "Pubkey", "That evidence object."),
            ("proposer", "Pubkey", "Who posted the head."),
            ("delegation", "u64", "Active delegation (AMBA atoms)."),
            ("delegators", "u16", "Delegations with active delegation."),
            ("accrual_since_ts", "u64", "When `accrual_integral` was last brought up to date."),
            ("accrual_integral", "u128", "`integral of delegation * live(t) dt` up to `accrual_since_ts` (atom-seconds), `live(t)` the head's live interval `[live_ts, retired_ts)`."),
            ("posted_ts", "u64", "Unix seconds."),
            ("expires_ts", "u64", "`posted_ts + head_ttl_secs`: an Open head past it expires."),
            ("active_ts", "u64", "When it activated (0: never)."),
            ("live_ts", "u64", "When its fill stood (0: never)."),
            ("retired_ts", "u64", "When it retired or expired (0: never)."),
        ] + CAPTURES + [
            ("strike_count", "u8", "Strikes recorded, at most `HEAD_STRIKE_CAPACITY`."),
            ("strike_opened_ts", "[u64; 8]", "Per strike: when the source case that caused it opened (first challenge). A delegation is exposed to it only if it was active then; the proposer skips it (it paid `proposer_slash_bps` at resolution)."),
            ("strike_ts", "[u64; 8]", "Per strike: when it was recorded; lazy application moves the accruals at this time."),
            ("reward_index_frozen", "u128", "The head's delegator reward index frozen when it left the live list (retired); while Live the board's entry holds it (`transitions::head_reward_index`)."),
            ("value", "u64", "Current value (the last standing print), six-decimal atoms."),
            ("value_source_ts", "u64", "Its source time."),
            ("value_claim_seq", "u32", "The claim that set it."),
            ("origin_id", "[u8; 32]", "The origin its last accepted print declared (hash of the underlying report's identity)."),
            ("duplicate", "bool", "An earlier live source of the same component has the same origin: no weight, no origin count, no delegator share. Recomputed when claims finalize or sources retire."),
            ("lta_effective_from_ts", "u64", "LTA channel: when the last accepted print's price took effect (0: none)."),
            ("lta_next_reset_ts", "u64", "LTA channel: its next reset (0: none); past it the print is no current level."),
            ("lta_floor", "u64", "LTA channel: its floor (0: none)."),
            ("lta_ceiling", "u64", "LTA channel: its ceiling (0: none)."),
            ("fill_value", "u64", "The value the standing fill set (the opening of contracts it joins mid-window)."),
            ("fill_source_ts", "u64", "Its source time."),
            ("print_count", "u32", "Accepted prints so far; the ring holds the last `HEAD_PRINT_CAPACITY` at `index % capacity`."),
            ("print_values", "[u64; 32]", "Accepted prints: value."),
            ("print_source_ts", "[u64; 32]", "Accepted prints: source time."),
            ("print_effective_from_ts", "[u64; 32]", "Accepted prints: an LTA print's effective date (0: none; zero off the LTA channel)."),
            ("print_next_reset_ts", "[u64; 32]", "Accepted prints: an LTA print's next reset (0: none); `channels::lta_level` drops a print past its own reset."),
            ("print_floor", "[u64; 32]", "Accepted prints: an LTA print's floor (0: none)."),
            ("print_ceiling", "[u64; 32]", "Accepted prints: an LTA print's ceiling (0: none)."),
            ("print_claim_seq", "[u32; 32]", "Accepted prints: the claim that made it. The ring is for display: settlement reads the contract samples (`sample_values`)."),
            ("reveals", "u32", "Claims revealed so far: a claim's `reveal_index`. (These 16 bytes held `capture_expiry_seq` before 2026-10-30.)"),
            ("finalized_reveals", "u32", "Revealed claims finalized so far, in whatever order: `reveals - finalized_reveals` are open."),
            ("last_prize_day", "u32", "The UTC day (`source_ts / 86,400`) of the last update that earned a prize: one update prize per head per day, whatever the sampling."),
            ("samples_reserved", "[u8; 4]", "Zero (held `sample_count` before round 3; kept so the 2,688-byte layout stays this one's prefix)."),
            ("adm_open", "[u16; 4]", "Per ring slot: the open claims settlement waits for (`samples::waits`): those revealed before the slot started (`restart_reveals`; a contract may admit them), and those revealed since that the contract admits (`samples::admitted`: committed before its expiry, window ended and every challenge ruled by its cutoff K, standing). (These 8 bytes held `capture_expiry_open` before 2026-10-30.)"),
            ("claim_seq", "u32", "Claims committed; the next claim's seq."),
            ("open_claims", "u16", "Claims committed or revealed and not yet finalized or refunded."),
            ("open_fill", "bool", "A fill claim is committed or in its window (competing fills challenge it)."),
            ("open_fill_seq", "u32", "That fill claim's seq."),
            ("unresolved_cases", "u16", "Source cases that have not resolved (claim challenges open none: they never strike)."),
            ("unresolved_since_ts", "u64", "When the current run of unresolved cases began (0 when none)."),
            ("source_case", "u32", "Number of the current or last source case (0: none)."),
            ("source_case_status", "u8", "`SOURCE_CASE_NONE`, `_OPEN`, `_STOOD` or `_FELL`."),
            ("source_case_opened_ts", "u64", "When the case's first challenge arrived."),
            ("source_case_window_end_ts", "u64", "The case's challenge window, late challenges extending it."),
            ("source_case_challenges", "u16", "Challenges in the case."),
            ("source_case_ruled", "u16", "Of them, ruled."),
            ("source_case_won", "u16", "Of them, won."),
            ("source_case_last_winner", "u16", "Index of the last won challenge: it takes the pro-rata remainder (`outcome::pro_rata`)."),
            ("source_case_settled_winners", "u16", "Won challenges already paid."),
            ("source_case_bond_total", "u64", "Sum of the case's bonds."),
            ("source_case_winning_bonds", "u64", "Sum of the won challenges' bonds."),
            ("source_case_proposer_slashed", "u64", "AMBA the falling case took from the proposer's delegation."),
            ("source_case_amba_paid", "u64", "Of it, paid to winners so far."),
            ("updated_ts", "u64", "Unix seconds."),
            ("strike_case", "[u32; 8]", "Per strike: the number of the source case that caused it; delegations compare case numbers, not times."),
            ("strike_open_gens", "[u32; 32]", "Per strike, at `strike * 4 + slot`: the generation of the ring contract unsettled in that slot when its case opened, the head a member of it then (0: none; round 14 review, N1). Escrow kept for one of them answers for the strike."),
            ("source_case_open_gens", "[u32; 4]", "The current case's open ring generations, as `strike_open_gens` keeps them."),
            ("source_case_deadline_ts", "u64", "The current case's ruling deadline, fixed when it opened (its first window's end plus the board's ruling deadline); late challenges extend the window only up to it. From it no ruling is taken and the case resolves on the rulings it has, each unruled challenge timing out alone (its bond back): early review B2, B5, B6."),
            ("source_case_ruled_bonds", "u64", "Sum of the case's ruled challenges' bonds: the case resolves on these."),
            ("sample_values", "[u64; 40]", "Per ring slot, at `slot * 10 + j`: the contract's samples at fixed instants (`samples::instants`: five in the freeze level's window, then five in the settlement window), each the value of the print deciding it so far (0: none), whose level there is the value unless past `sample_resets`; so the print ring never decides settlement."),
            ("sample_ts", "[u64; 40]", "Per sample: the source time of the print deciding it so far, the latest the contract admits at or before its instant (0: none). Latest by source time, then by what it says (`samples::Key`: value, then reset), so the order claims finalize in never matters (round 3 R1; round 5)."),
            ("sample_resets", "[u32; 40]", "Per sample: that print's LTA reset (unix seconds, at most `u32::MAX`; 0: none). (A hash rank in version 1.)"),
            ("standing_origin", "[[u8; 32]; 4]", "Per ring slot: the origin of the print deciding its last instant (the expiry): the latest the contract admits, the provenance it settles on."),
            ("freeze_origin", "[[u8; 32]; 4]", "Per ring slot: the origin of the print deciding its freeze instant, the fallback's provenance."),
            ("restart_reveals", "[u32; 4]", "Per ring slot: `reveals` when the slot started on its contract. A claim revealed before (`reveal_index` below it) counts in `adm_open` while it is open: the contract may admit it."),
            ("unbacked_since_ts", "u64", "Since when the head has been below activation backing (`floor_activate` with `min_delegators`), continuously: set by the delegation change that took it below, cleared (0) by the one that brought it back. On a full list a Live head unbacked for `cooldown_secs` gives up its slot even as a contract's member (round 11 review, finding 3). 0 on a head of before: `ExpireHead` starts its time."),
            ("evicted_ts", "u64", "When the head left the live list without retiring (0: never): on a full list after `cooldown_secs` unbacked, a member of an unsettled contract or with a case open (round 12 review, N1 and N2). It stays Live for everything else: a member of its contracts, eligible and weighing as if listed, printing, its delegations uncooled, open to source cases. It is no origin, joins no contract, takes no slot and does not come back; `ExpireHead` retires it once none of its contracts is unsettled and no case is open."),
        ],
    ),
    dict(
        name="Delegation",
        doc="""One owner's AMBA on one head. Active delegation carries weight and earns while the head is
Live; undelegation moves it into `held` (`liability::Held`), which comes back after the cooldown
except for the escrow the remaining strikes and the open exposure still need. It keeps the same
live-gated integral as its head, captured lazily at each open contract's listing and freeze.

Seeds: `[ameba-oracle-v1, head-delegation, head, owner]`.""",
        disc="HDDELEGN", len=1024, version=3,
        fields=HDR + [
            ("head", "Pubkey", "The head."),
            ("owner", "Pubkey", "The owner."),
            ("board", "Pubkey", "The head's board."),
            ("is_proposer", "bool", "The head's proposer: a lost source challenge takes `proposer_slash_bps` of it, and its escrow is `liability::proposer_escrow_required`."),
            ("active", "u64", "Active delegation exposed since `active_since_ts`."),
            ("active_since_ts", "u64", "When `active` last went from zero to positive."),
            ("fresh", "u64", "Delegation added while the head had unresolved cases: a strike whose case opened before `fresh_since_ts` does not touch it. It joins `active` once no such case remains."),
            ("fresh_since_ts", "u64", "When `fresh` was first added."),
            ("accrual_since_ts", "u64", "When `accrual_integral` was last brought up to date."),
            ("accrual_integral", "u128", "`integral of (active + fresh) * live(t) dt` up to `accrual_since_ts`."),
        ] + CAPTURES + [
            ("applied_strikes", "u8", "Strikes of the head already applied to this delegation."),
            ("held_principal", "u64", "What the undelegated part would hold if still delegated (strikes come off it): the sum of `held_exposed`."),
            ("held", "u64", "AMBA still locked (cooling, then escrow), at most `held_principal`; 0 clears the held state."),
            ("cooldown_end_ts", "u64", "When the latest cooling withdrawal's cooldown ends (0: none yet)."),
            ("exit_ts", "u64", "When it last undelegated (0: never)."),
            ("reward_index", "u128", "The board reward index up to which rewards were accrued."),
            ("rewards_accrued", "u64", "Unclaimed delegator rewards (USDC atoms)."),
            ("rewards_claimed", "u64", "Rewards claimed so far."),
            ("slashed_total", "u64", "AMBA slashed (burned) by strikes."),
            ("proposer_slashed", "u64", "AMBA a lost source challenge took from it (paid to the winners)."),
            ("created_ts", "u64", "Unix seconds."),
            ("updated_ts", "u64", "Unix seconds."),
            ("rent_payer", "Pubkey", "Who paid this account's rent: closing returns it to them, never to the owner (zero on an account made before the field existed: its owner paid)."),
            ("held_exempt", "u64", "Zero since version 2."),
            ("case_ref", "u32", "The open case `held_case_adjust` is for (0: none): set while the delegation acts during a case, cleared once that case resolved and its strike applied."),
            ("active_since_case", "u32", "The head's case number (`source_case`) when `active` last went from zero to positive: only later cases strike it. Case numbers order cases against stake, whatever the timestamps."),
            ("fresh_since_case", "u32", "The head's case number when `fresh` was first added (the case it is exempt from)."),
            ("exit_case", "u32", "The head's case number at the last undelegation."),
            ("capture_gen", "[u32; 4]", "Per capture slot: the generation (`HeadsBoard::contract_gens`) of the contract those captures belong to."),
            ("held_gens", "[u32; 4]", "Per ring slot: the generation of the unsettled contract that `weighed[i]` and bit `i` of the held masks name (0: none)."),
            ("held_free", "u64", "Zero since version 2."),
            ("held_exposed", "[u64; 16]", "The held principal by the contracts it weighed in (round 6 review, findings 3, 4 and 6): index = mask, bit `i` the contract of generation `held_gens[i]`; index 0, none still unsettled. Every mask has its own entry, so no exposure is ever joined to another; a case opened while one of a mask's contracts is unsettled strikes that mask's principal."),
            ("held_case_adjust", "[u64; 16]", "For the open case `case_ref`, per mask: of a mask meeting the case's contracts (`source_case_open_gens`), the part exempt from it (left from `fresh` during it); of any other mask, the part answering it by number (left from `active` during it, or reached through a contract that settled since)."),
            ("weighed", "[u64; 4]", "Per ring slot: of `active + fresh`, the stake that weighed in the contract of generation `held_gens[i]` (delegated while its window ran and the head was live). It is the oldest of it: a withdrawal takes the newest first."),
            ("cooling", "[u64; 4]", "Withdrawals still cooling, by cooldown end (`cooling_end`); a fifth distinct end joins the two closest ends at the later one."),
            ("cooling_end", "[u64; 4]", "When each `cooling` amount's cooldown ends (0: free)."),
            ("dust", "bool", "Its weight is below its share of the activation floor (`floor_activate / min_delegators`, on a board needing two delegators or more) as of its last change, so it is not one of the head's delegators (round 9 review). False on an account of before: such a delegation counted while its weight was positive."),
        ],
    ),
    dict(
        name="HeadClaim",
        doc="""A fill or an update of a head: commit, reveal, the challenge window, and the aggregates that
reproduce `outcome::resolve_claim` without loading every challenge (challenges per claim are
unbounded; each one is paid by its own `SettleChallenge`, the index-last winner after every other).

Seeds: `[ameba-oracle-v1, head-claim, head, seq (u32 LE)]`.""",
        disc="HDCLAIM_", len=512,
        fields=HDR + [
            ("head", "Pubkey", "The head."),
            ("board", "Pubkey", "Its board."),
            ("seq", "u32", "Position among the head's claims."),
            ("kind", "HeadClaimKind", "Fill or update."),
            ("status", "HeadClaimStatus", "Committed, revealed, stood, fell or expired."),
            ("fetcher", "Pubkey", "Who filed it: anyone, for a fill of an Active head or an update of a Live one."),
            ("commit_hash", "[u8; 32]", "`heads_claim_commitment` of the revealed fields."),
            ("commit_slot", "u64", "Slot of the commit; the reveal comes in a later slot."),
            ("commit_ts", "u64", "Unix seconds."),
            ("reveal_deadline_slot", "u64", "Last slot of the reveal (v1's reveal window)."),
            ("value", "u64", "Revealed value, six-decimal atoms."),
            ("source_ts", "u64", "Revealed source time."),
            ("origin_id", "[u8; 32]", "Revealed origin: the fetcher-declared hash of the underlying report's identity (challengeable)."),
            ("lta_effective_from_ts", "u64", "LTA channel: when the price took effect (0: none; always 0 off the LTA channel)."),
            ("lta_next_reset_ts", "u64", "LTA channel: the next reset (0: none)."),
            ("lta_floor", "u64", "LTA channel: the floor (0: none)."),
            ("lta_ceiling", "u64", "LTA channel: the ceiling (0: none)."),
            ("evidence_kind", "u8", "0 none, or the sealed evidence object's kind (2, 3 or 4)."),
            ("evidence_commitment", "[u8; 32]", "That object's commitment (zero for none)."),
            ("evidence_object", "Pubkey", "That object (default for none)."),
            ("bond", "u64", "USDC bond (`claim_bond`)."),
            ("revealed_ts", "u64", "When it was revealed: the challenge window starts."),
            ("window_end_ts", "u64", "End of the challenge window, late challenges extending it."),
            ("challenge_count", "u16", "Challenges filed."),
            ("ruled_count", "u16", "Of them, ruled."),
            ("won_count", "u16", "Of them, won."),
            ("last_winner", "u16", "Index of the last won challenge (pro-rata remainder)."),
            ("settled_count", "u16", "Challenges paid."),
            ("settled_winners", "u16", "Won challenges paid."),
            ("challenge_bond_total", "u64", "Sum of the challenges' bonds."),
            ("winning_bond_total", "u64", "Sum of the won challenges' bonds."),
            ("shares_paid", "u64", "Claim-bond shares paid to winners so far."),
            ("bonds_burned", "u64", "Losing challengers' bonds burned when the claim fell."),
            ("to_claimant", "u64", "USDC paid to the fetcher when the claim stood."),
            ("finalized_ts", "u64", "When `FinalizeClaim` ran (0: not yet)."),
            ("prize_points", "u64", "The prize it earned when it stood (USDC atoms before pro-rata scaling)."),
            ("epoch", "u32", "Calendar month (`time::month_id`) the prize belongs to."),
            ("prize_paid", "u64", "Prize paid."),
            ("prize_claimed", "bool", "`ClaimPrize` ran."),
            ("rent_payer", "Pubkey", "Who paid this account's rent: closing returns it to them, never to the owner (zero on an account made before the field existed: its owner paid)."),
            ("reveal_index", "u32", "Its place among the head's reveals (`Head::reveals`): a ring slot that started after it waits for it (`Head::restart_reveals`)."),
            ("last_ruled_ts", "u64", "When its last challenge was ruled (0: none): ruled after a contract's cutoff, it is never admitted to that contract."),
            ("ruling_deadline_ts", "u64", "Its ruling deadline, fixed by its first challenge (the window's end then plus the board's ruling deadline); late challenges extend the window only up to it. From it no ruling is taken and the claim resolves on the rulings it has, each unruled challenge timing out alone (its bond back); a claim with one stands for its bonds but prints nothing (early review B2, B5, B6)."),
            ("ruled_bond_total", "u64", "Sum of its ruled challenges' bonds: the claim resolves on these."),
        ],
    ),
    dict(
        name="HeadChallenge",
        doc="""A bonded challenge of a claim (target: the claim) or of a source (target: the head). It is
ruled by three current council seats; `SettleChallenge` pays it and closes it, so the same
challenger can challenge the same target again in a later case.

Seeds: `[ameba-oracle-v1, head-challenge, target, challenger]`.""",
        disc="HDCHALLG", len=384,
        fields=HDR + [
            ("target", "Pubkey", "The claim, or the head for a source challenge."),
            ("head", "Pubkey", "The head."),
            ("challenger", "Pubkey", "Who filed it."),
            ("kind", "HeadChallengeKind", "Claim or source."),
            ("reason", "HeadChallengeReason", "The ground: value or other, off channel, duplicate origin, expired observation. A label only: same bond, resolution and slash."),
            ("index", "u16", "Position among the claim's (or the source case's) challenges."),
            ("case_number", "u32", "The claim's seq, or the head's source case number."),
            ("bond", "u64", "USDC bond (at least `challenge_bond`)."),
            ("alternative_value", "u64", "A claim challenge's alternative value (0: none); a competing fill carries its value here."),
            ("alternative_source_ts", "u64", "Its source time."),
            ("evidence_kind", "u8", "0 none, or the sealed evidence object's kind (2, 3 or 4)."),
            ("evidence_commitment", "[u8; 32]", "That object's commitment."),
            ("evidence_object", "Pubkey", "That object (default for none)."),
            ("reason_hash", "[u8; 32]", "Commitment of the challenger's reason text."),
            ("opened_ts", "u64", "Unix seconds."),
            ("ruling", "HeadChallengeRuling", "Pending, won or lost."),
            ("ruled_ts", "u64", "When it was ruled."),
            ("ruled_epoch", "u64", "The council epoch of the ruling."),
            ("payout_usdc", "u64", "USDC paid to the challenger."),
            ("payout_amba", "u64", "AMBA paid to the challenger's stake (won source challenge)."),
            ("settled", "bool", "Paid."),
            ("rent_payer", "Pubkey", "Who paid this account's rent: closing returns it to them, never to the owner (zero on an account made before the field existed: its owner paid)."),
        ],
    ),
    dict(
        name="HeadsContract",
        doc="""One contract (a `Period` of the product) listed and settled on the board: what it listed with,
frozen (parameters, items with basket weights and mix, components with their sentences, the
settlement target, the listing terms document, the live sources and their openings), and the
settlement record (per component its channel median or degraded flag, per item its settled
change and its fixed-mix reference, with the channels it holds at their last valid value).

`FixedMixReference` settles `index = sum over items of basket weight * the item's
reference`, the reference being `sum over channels of mix share * that component's change`,
each component live or at its degraded path's last valid value, so the mix never shifts; it
lists on assumed or matched mixes (the user, 2026-10-29: "SPOT LTA AND CONTRACT NEED TO BE
WEIGHTED AGAINST EACHOTHER"). `ContractChannel` (the brief's Stage 1, still a valid target)
settles on each item's Contract component alone and records the reference for display.

Seeds: `[ameba-oracle-v1, heads-contract, period]`.""",
        disc="HDCONTRT", len=4608,
        fields=HDR + [
            ("period", "Pubkey", "The period."),
            ("board", "Pubkey", "The board."),
            ("product_id", "[u8; 32]", "The product."),
            ("ring_slot", "u8", "Its slot in the board's contract ring."),
            ("status", "HeadsContractStatus", "Accumulating, collecting or settled."),
            ("settlement_target", "HeadsSettlementTarget", "What settles: each item's fixed-mix reference, or its Contract channel alone."),
            ("params", "HeadsParams", "The board's parameters when it listed."),
            ("params_version", "u32", "Their version."),
            ("base", "u64", "Six-decimal base value set at listing."),
            ("listing_ts", "u64", "Start of the accumulation window (the period's effective listing)."),
            ("freeze_ts", "u64", "End of the accumulation window."),
            ("expiry_ts", "u64", "The period's expiry."),
            ("terms_hash", "[u8; 32]", "Commitment of the listing terms document (a sealed kind-2 object: products, basket weights, sentences with terms, mix with basis and citation, settlement target, the disclosure sentence)."),
            ("item_count", "u8", "The board's items at listing."),
            ("item_bucket_ids", "[[u8; 32]; 8]", "Per item: bucket id."),
            ("item_weight_bps", "[u16; 8]", "Per item: basket weight, frozen at listing."),
            ("item_mix_bps", "[u16; 24]", "Per item, at `item * 3 + channel`: the mix, frozen at listing."),
            ("item_mix_basis", "[u8; 8]", "Per item: `HeadsMixBasis` byte, frozen at listing."),
            ("item_change_ppm", "[i64; 8]", "Per item: the change that settles (its reference, or its Contract component's under `ContractChannel`)."),
            ("item_reference_ppm", "[i64; 8]", "Per item: the fixed-mix reference, `sum over channels of mix share * component change` (each live or at its last valid value), when available."),
            ("item_reference_flags", "[u8; 8]", "Per item: `REFERENCE_*` flags (computed, available, assumed)."),
            ("item_reference_reason", "[u8; 8]", "Per item: `REFERENCE_REASON_*` (`channels::Unavailable::code`) when the reference is unavailable (2 mix basis none, 4 invalid mix)."),
            ("item_degraded_channels", "[u8; 8]", "Per item: bit `1 << channel` for each channel the mix weights that settled on its degraded path, so the reference holds its last valid value at its fixed share (`channels::Reference::degraded_mask`)."),
            ("component_count", "u8", "The board's components at listing."),
            ("component_items", "[u8; 24]", "Per component: item index."),
            ("component_channels", "[u8; 24]", "Per component: `HeadsChannel` byte."),
            ("component_sentence_hashes", "[[u8; 32]; 24]", "Per component: its target sentence's commitment, frozen at listing."),
            ("component_listed_sources", "[u16; 24]", "Per component: live sources at listing."),
            ("component_listed_origins", "[u16; 24]", "Per component: distinct live origins at listing."),
            ("component_freeze_origins", "[u16; 24]", "Per component: distinct live origins at freeze (`COMPONENT_FREEZE_CHECKED` once recorded, `COMPONENT_FREEZE_PASSED` when `channels::origins_pass`)."),
            ("component_degraded", "[u8; 24]", "Per component: `DEGRADED_*` (`settle::Degraded`) when it settled on its last valid value, else 0."),
            ("component_duplicates", "[u16; 24]", "Per component: otherwise eligible sources dropped at settlement as duplicate origins."),
            ("component_flags", "[u8; 24]", "Per component: `COMPONENT_*` flags of the settlement record."),
            ("component_change_ppm", "[i64; 24]", "Per component: its channel median change."),
            ("component_freeze_change_ppm", "[i64; 24]", "Per component: the change at freeze (degraded rule 2's fallback)."),
            ("component_eligible", "[u16; 24]", "Per component: eligible sources at settlement, one per independent origin."),
            ("component_weight", "[u64; 24]", "Per component: the eligible sources' raw weight."),
            ("components_settled", "u8", "Components settled."),
            ("listed_count", "u16", "Sources live at listing with a price in effect then, backed or not (an LTA price past its reset is not listed: round 4 N5; an unbacked one is a member but no origin: round 10 review, N1)."),
            ("listed_head_ids", "[u32; 96]", "Sources live at listing: head id."),
            ("listed_components", "[u8; 96]", "Sources live at listing: component index."),
            ("listed_flags", "[u8; 96]", "Sources live at listing: `LIVE_DUPLICATE` as the board had it."),
            ("listed_openings", "[u64; 96]", "Sources live at listing: opening (their value then)."),
            ("index_change_ppm", "i64", "`settle::index_change_ppm` of the items."),
            ("value", "u64", "`settle::settlement_value(base, index_change_ppm)`."),
            ("digest", "[u8; 32]", "Digest of the settlement record, written to the period."),
            ("settled_ts", "u64", "Unix seconds (0: not settled)."),
            ("listed_by", "Pubkey", "The oracle authority that listed it."),
            ("updated_ts", "u64", "Unix seconds."),
        ],
    ),
    dict(
        name="HeadsEpoch",
        doc="""A calendar month of one board's fetcher prizes: the pool funded during it and the prize points
earned by claims that stood during it. Prizes pay out after it ends, scaled pro rata when the pool is
short (`rewards::prize_payout`).

Seeds: `[ameba-oracle-v1, heads-epoch, board, month_id (u32 LE)]`.""",
        disc="HDEPOCH_", len=160,
        fields=HDR + [
            ("board", "Pubkey", "The board."),
            ("month_id", "u32", "`time::month_id` of the month."),
            ("start_ts", "u64", "`time::month_start(month_id)`."),
            ("end_ts", "u64", "`time::month_start(month_id + 1)`."),
            ("prize_pool", "u64", "USDC funded into the month's prizes."),
            ("prize_points", "u64", "Prize points earned in the month."),
            ("prize_paid", "u64", "Prizes paid."),
            ("claims", "u32", "Claims that earned points."),
            ("claims_paid", "u32", "Of them, paid."),
            ("updated_ts", "u64", "Unix seconds."),
            ("rolled_out", "u64", "Prize money no prize could claim, moved to a later month's pool once the month ended (`RollOverPrizes`)."),
            ("rolled_in", "u64", "Prize money rolled in from ended months."),
        ],
    ),
    dict(
        name="HeadsWeightsHeader",
        doc="""The fixed head of a `HeadsWeights` account (one contract's component): its source records
follow (`HeadsWeightsLayout`). The first `SettleComponent` of the component creates it from the
contract's listed sources; sources that went live inside the window are appended by passing their
heads. A source's weight is its head's own time-weighted delegation over the window: no caps, so no
per-owner record is needed (the user, 2026-10-29). The call that completes the record reads in full
only the records that can count (live at the cutoff, or at the freeze): at most the board's live
list each, however many sources joined and retired over the window (round 3 R5).

Seeds of the account: `[ameba-oracle-v1, heads-weights, contract, component (u8)]`.""",
        disc="HDWEIGHT", len=192,
        fields=HDR + [
            ("contract", "Pubkey", "The contract."),
            ("board", "Pubkey", "The board."),
            ("component", "u8", "Component index."),
            ("channel", "HeadsChannel", "The component's channel."),
            ("phase", "HeadsWeightsPhase", "Recording levels, or settled."),
            ("bucket_id", "[u8; 32]", "The component's bucket id."),
            ("window_secs", "u64", "`freeze_ts - listing_ts`: integrals divide by it into raw weights."),
            ("source_count", "u16", "Source records in use."),
            ("source_capacity", "u16", "Source records allocated."),
            ("levels_recorded", "u16", "Sources whose settlement level is recorded, in record order."),
            ("eligible_sources", "u16", "Sources eligible at settlement, one per independent origin."),
            ("duplicate_sources", "u16", "Otherwise eligible sources dropped as duplicate origins."),
            ("weight_total", "u64", "The eligible sources' raw weight."),
            ("updated_ts", "u64", "Unix seconds."),
            ("listed_recorded", "u16", "Records of sources the contract listed: the component is complete once these are all of its listed sources and `joined_recorded` all its joined ones, each recorded once (the head's `CAPTURE_RECORDED`), so completeness reads no record (round 3 R5)."),
            ("joined_recorded", "u16", "Records of sources that joined the window (`HeadsBoard::contract_joined`)."),
        ],
    ),
    dict(
        name="HeadsWeightSource",
        doc="""One source record of a `HeadsWeights` account (`HEADS_WEIGHT_SOURCE_STRIDE` bytes, zero padded).""",
        disc=None, len=160,
        fields=[
            ("head_id", "u32", "The head."),
            ("flags", "u8", "`WEIGHT_SOURCE_*` flags."),
            ("origin_id", "[u8; 32]", "The head's origin at settlement (`settle::SourceInput::origin`)."),
            ("live_at", "u64", "When the head went live (`settle::SourceInput::live_at`: the earlier of two sources with one origin keeps it)."),
            ("opening", "u64", "Its opening: its value at listing, or its fill value if it went live later."),
            ("integral", "u128", "The head's live-gated delegation integral over the window (`transitions::head_window_integral`)."),
            ("weight", "u64", "Its raw weight, `time::average(integral, window_secs)` (`settle::SourceInput::weight`)."),
            ("level", "u64", "Settlement level (`settle::temporal_level` over prints of claims committed before expiry; an LTA print past its own next reset is no current level)."),
            ("freeze_level", "u64", "Level as of the freeze (the fallback)."),
            ("change_ppm", "i64", "`settle::change_ppm(opening, level)`."),
            ("freeze_change_ppm", "i64", "`settle::change_ppm(opening, freeze_level)`."),
            ("freeze_origin_id", "[u8; 32]", "The head's origin at the freeze: the fallback's duplicates use it."),
        ],
    ),
]

def size(t):
    m = re.fullmatch(r"\[(.+); (\d+)\]", t)
    if m:
        return size(m.group(1)) * int(m.group(2))
    if t == "HeadsParams":
        return sum(size(pt) for _, pt, _ in PARAMS)
    return SIZES[t]


def default(t):
    m = re.fullmatch(r"\[(.+); (\d+)\]", t)
    if m:
        inner, n = m.group(1), m.group(2)
        if inner == "Pubkey":
            return f"[Pubkey::new_from_array([0; 32]); {n}]"
        if inner == "[u8; 32]":
            return f"[[0; 32]; {n}]"
        return f"[0; {n}]"
    if t in ("u8", "u16", "u32", "i32", "u64", "i64", "u128"):
        return "0"
    if t == "bool":
        return "false"
    if t == "Pubkey":
        return "Pubkey::new_from_array([0; 32])"
    if t == "HeadsParams":
        return "HeadsParams::zero()"
    if t in ENUMS:
        return f"{t}::default()"
    raise ValueError(t)


def wrap(text, indent):
    out = []
    for para in text.split("\n"):
        line = indent + "///"
        if not para:
            out.append(line)
            continue
        words = para.split(" ")
        cur = line
        for w in words:
            if len(cur) + 1 + len(w) > 100 and cur != line:
                out.append(cur)
                cur = line + " " + w
            else:
                cur = cur + " " + w
        out.append(cur)
    return "\n".join(out)


def emit_params():
    wire = sum(size(t) for _, t, _ in PARAMS)
    s = []
    s.append(wrap("A board's parameters (spec section 7), the program's mirror of `ameba_oracle_heads::Params` with a fixed wire layout. Amounts: AMBA atoms for delegation and fetcher stakes, USDC atoms for bonds and prizes; rates in basis points; durations in seconds.", ""))
    s.append("#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]")
    s.append("pub struct HeadsParams {")
    for n, t, d in PARAMS:
        s.append(wrap(d, "    "))
        s.append(f"    pub {n}: {t},")
    s.append("}")
    s.append("")
    s.append("impl HeadsParams {")
    s.append(f"    /// Wire bytes.\n    pub const LEN: usize = {wire};")
    s.append("}")
    s.append("")
    s.append(f"fixed_state_deserialize_flat!(HeadsParams, {wire}, {{")
    for n, t, _ in PARAMS:
        s.append(f"    {n}: {t},")
    s.append("});")
    s.append("")
    s.append("impl From<&HeadsParams> for ameba_oracle_heads::Params {")
    s.append("    fn from(p: &HeadsParams) -> Self {")
    s.append("        Self {")
    for n, _, _ in PARAMS:
        s.append(f"            {n}: p.{n},")
    s.append("        }\n    }\n}")
    s.append("")
    s.append("impl From<&ameba_oracle_heads::Params> for HeadsParams {")
    s.append("    fn from(p: &ameba_oracle_heads::Params) -> Self {")
    s.append("        Self {")
    for n, _, _ in PARAMS:
        s.append(f"            {n}: p.{n},")
    s.append("        }\n    }\n}")
    return "\n".join(s), wire


# Layouts too large for a 4 KiB SBF stack frame: on SBF they keep only the flat descriptor and the
# in-place encoder (`heads_heap_codec!`); `Default` and the by-value decoders are host-only.
HEAP_LAYOUTS = {"HeadsBoard", "Head", "HeadsContract"}


def emit(acc):
    name = acc["name"]
    wire = sum(size(t) for _, t, _ in acc["fields"])
    assert wire <= acc["len"], (name, wire, acc["len"])
    s = []
    s.append(wrap(acc["doc"], ""))
    s.append("#[derive(Clone, Debug, Eq, PartialEq)]")
    s.append(f"pub struct {name} {{")
    for n, t, d in acc["fields"]:
        s.append(wrap(d, "    "))
        s.append(f"    pub {n}: {t},")
    s.append("}")
    s.append("")
    heap = name in HEAP_LAYOUTS
    if heap:
        s.append('#[cfg(not(target_os = "solana"))]')
    s.append(f"impl Default for {name} {{")
    s.append("    fn default() -> Self {")
    s.append("        Self {")
    for n, t, _ in acc["fields"]:
        s.append(f"            {n}: {default(t)},")
    s.append("        }\n    }\n}")
    s.append("")
    s.append(f"impl {name} {{")
    what = "Account bytes" if acc["disc"] else "Record stride in bytes"
    s.append(f"    /// {what}; everything past `WIRE_LEN` is zero.")
    s.append(f"    pub const LEN: usize = {acc['len']};")
    s.append(f"    /// Bytes of the encoded fields.")
    s.append(f"    pub const WIRE_LEN: usize = {wire};")
    if acc["disc"]:
        s.append(f"    pub const DISCRIMINATOR: [u8; 8] = *b\"{acc['disc']}\";")
        s.append(f"    pub const VERSION: u8 = {acc.get('version', 1)};")
    s.append("}")
    s.append("")
    flat = any(t == "HeadsParams" for _, t, _ in acc["fields"])

    def leaves():
        out = []
        for n, t, _ in acc["fields"]:
            if t == "HeadsParams":
                for pn, pt, _ in PARAMS:
                    out.append(f"    {n}.{pn}: {pt},")
            else:
                out.append(f"    {n}: {t},")
        return out

    if heap:
        s.append('#[cfg(not(target_os = "solana"))]')
    s.append(f"fixed_state_deserialize_flat!({name}, {wire}, {{")
    for n, t, _ in acc["fields"]:
        s.append(f"    {n}: {t},")
    if flat:
        s.append("}, flat {")
        for n, t, _ in acc["fields"]:
            if t == "HeadsParams":
                for pn, pt, _ in PARAMS:
                    s.append(f"    {n}.{pn}: {pt},")
            else:
                s.append(f"    {n}: {t},")
    s.append("});")
    if heap:
        s.append("")
        s.append('#[cfg(target_os = "solana")]')
        s.append(f"heads_heap_codec!({name}, {wire}, flat {{")
        s.extend(leaves())
        s.append("});")
    return "\n".join(s), wire


if __name__ == "__main__":
    parts = []
    text, pw = emit_params()
    parts.append(text)
    report = [f"HeadsParams wire={pw}"]
    for acc in ACCOUNTS:
        text, wire = emit(acc)
        parts.append(text)
        report.append(f"{acc['name']} wire={wire} len={acc['len']}")
    print("// @generated from the field table of oracle v2's heads layouts by `gen_layouts.py`. Wire\n// lengths are the exact sums of the fields; the codec checks them at compile time.\n")
    print("\n\n".join(parts))
    print("\n".join(report), file=sys.stderr)
