# Oracle Long-Term Agreement Fix

Implementation brief. Self-contained. Plugs into the Boidian Oracle v2 mechanism ("heads on pikes": stake-delegated source weights, weighted median per component, bond-and-challenge as the only verifier). This document does not change that mechanism; it defines what the mechanism measures for memory markets so that long-term agreements (LTAs) are modeled instead of ignored.

---

## 1. Problem

Memory is sold through three channels that price differently:

- **Spot** — inventory bought now. Moves first.
- **Contract** — periodic (monthly/quarterly) wholesale negotiation, supplier → OEM/ODM. Re-fixes every cycle.
- **LTA** — multi-year agreements with committed volume. Price changes only at resets, often inside a band.

Today RAMX and NANDX are built from public reference pages, which means they measure the contract/spot buyer only. Suppliers have moved a large share of forward capacity under LTAs. Left alone, the oracle will keep measuring the uncommitted buyer and call it RAM, because that is the evidence fetchers can reach. Left fully open, a small delegator pool could drift to any one channel.

The fix is a **mold**: a fixed structure Amoeba sets at listing, inside which delegators choose sources freely. Amoeba never reviews a submission.

---

## 2. Decisions (final)

1. Every product in a bucket gets **three components**, one per channel, each with its own target sentence, source board, and weighted median.
2. Channels are **mutually exclusive by hierarchy**: LTA, then Contract, then Spot. A purchase lands in exactly one, or none.
3. Eligibility is a **challenge outcome**, never a review. A claim that doesn't match its channel's sentence is a false claim.
4. A **fixed-mix procurement reference** combines the three channel medians using a declared mix. It is named exactly that, not "the price of memory."
5. The mix is **set at listing, frozen per contract**, and either backed by matched evidence or labeled an assumed scenario. Nothing settles on an assumed scenario.
6. **Missing weight is never redistributed.** A channel below threshold makes the reference unavailable; it does not shift its mix to other channels.
7. **Stage 1:** existing RAMX/NANDX contracts settle on the **Contract** channel. Spot, LTA, and the reference are displayed alongside.
8. **Stage 2:** a second instrument per bucket on the fixed-mix reference, gated on a populated LTA board, a matched-evidence mix, and demonstrated demand.
9. Source thresholds count **independent origins**, not pages.
10. Amoeba's only levers: products and basket weights, channel sentences, mix, and settlement channel — all at listing, in public. Between listings Amoeba is a delegator like anyone else.

---

## 3. Channel definitions

### 3.1 Classification hierarchy

Apply top-down. First match wins.

| Order | Channel | A purchase belongs here if… |
|---|---|---|
| 1 | **LTA** | it is made under a supply agreement of ≥ 1 year with committed volume. Includes prices newly fixed or reset under such an agreement. A newly signed LTA is LTA. |
| 2 | **Contract** | it is a negotiated wholesale purchase, supplier → OEM/ODM, delivered in the observation period, and is **not** under an LTA. |
| 3 | **Spot** | it is an immediate-inventory purchase outside both. |
| — | none | it fits none of the above (unterminated quotes, resales without terms, etc.). Not eligible anywhere. |

### 3.2 Target sentence template

Each component's sentence is frozen per listed contract. Bracketed terms are **fixed at listing**, never left as "standard."

**Spot**
> [Currency] per [unit], [product], immediately available inventory, open-market trade, lot size ≥ [N], delivery within [D days], [delivery basis], as of the observation date.

**Contract**
> [Currency] per [unit], [product], supplier → OEM/ODM purchase not under an LTA, delivered during the observation [period], volume tier [X], [delivery basis], [payment terms], price fixed during this negotiation period.

**LTA**
> [Currency] per [unit], [product], price in effect for deliveries during the observation [period] under an agreement of ≥ 1 year with committed volume, volume tier [X], [delivery basis]. Evidence shows the price in effect, the period it covers, the next reset date, and any floor/ceiling.

Observation periods are set per product from how that product's prices are actually fixed and published (monthly for most DRAM/NAND references; quarterly for some).

### 3.3 Channel behavior (descriptions only)

Not eligibility conditions. Channels react at different speeds and can diverge because their buyers, commitments, and supply differ.

- Spot usually moves first and most, but can sit below a rising contract price.
- Contract is the figure the industry and supplier earnings calls treat as the price.
- LTA prices are flat between resets for price-fixing agreements; many have bands, not all; across staggered resets the channel is not one step. The contract − LTA spread reflects differing commitments and terms, not only signing timing.

---

## 4. Fixed-mix procurement reference

```
reference = m_spot · Spot_median + m_contract · Contract_median + m_lta · LTA_median
m_spot + m_contract + m_lta = 1
```

- Inputs are the oracle's delegation-weighted channel medians.
- Output is what a buyer with the declared mix would face. It is **not** the economy's volume-weighted average paid price and must not be described as such.
- **Mix basis.** Matched evidence means the same product population, delivery period, buyer/supplier population, volume unit, and transaction stage. Supplier capacity disclosures ("over half of 2027–28 capacity under LTA") are not matched evidence for a per-product current-quarter share. Where matched evidence is absent, the listing publishes the mix as **"assumed scenario"** or leaves the reference unavailable.
- **Degraded rule.** If any channel for a product has fewer than 3 independent live origins at freeze, the reference for that product is unavailable. No backfill, no mix shift.
- **Never** tuned for volatility.

---

## 5. Evidence rules

What a claim's evidence bundle must contain to survive challenge. Checked by challengers, not by Amoeba.

| Channel | Evidence shows |
|---|---|
| Spot | price, lot size, availability/delivery, as-of date, venue |
| Contract | price, product, delivery period, negotiation period, counterparty class (supplier → OEM/ODM) |
| LTA | price in effect, delivery period, agreement start and term, next reset, floor/ceiling if any, counterparty class |

Three rules on top:

1. **Origins, not pages.** Sources whose evidence traces to the same underlying report are one origin. A source republishing another live source's origin is challengeable as a duplicate, carries no weight toward `min_live_sources`, and earns no delegator share.
2. **Effective vs expired.** An old publication documenting a price still in effect is valid LTA evidence. An observation carried past its reset is expired and is a false claim. The bundle must show which.
3. **Checked at listing and at freeze.** Origin count and target match are verified when a contract lists and again when it freezes. A channel that passed at listing and fails at freeze goes to the degraded path; it does not settle on stale sources.

Mechanism claim, stated exactly: **demonstrably off-target submissions are challengeable and slashable.** This sorts what surfaces; it does not guarantee that private LTA evidence will surface.

---

## 6. What settles, and when

**Stage 1 — now.**
- Each listed RAMX/NANDX contract settles on its product basket's **Contract** channel. This measures periodic wholesale procurement *outside* LTAs. The listing says so in those words.
- Existing monthly specs are already Contract-channel specs. At the next listing they gain explicit bracketed terms and the "not under an LTA" clause.
- Spot and LTA channels, and the reference (when computable with a stated mix basis), are displayed side by side with the settling index from day one.

**Stage 2 — gated.**
- A second instrument per bucket settling on the fixed-mix reference (working name RAMX-M / NANDX-M).
- Gate: (a) LTA board populated with ≥ 3 independent origins per product, sustained across a full contract; (b) mix backed by matched evidence, not an assumed scenario; (c) demonstrated demand for the instrument.

**LTA-channel instrument.** Not planned. If ever considered, the test is whether the settlement's fixings are already determined at listing; a future over undetermined resets retains uncertainty, one over already-fixed prices does not.

**Not permitted.** Changing an existing contract's settlement channel, mix, sentences, or basket mid-life.

---

## 7. Amoeba's levers

At each listing, published with reasoning:
1. Products in the basket and basket weights.
2. The three channel sentences per product, with bracketed terms fixed.
3. The mix per product, with its basis (matched evidence, or "assumed scenario").
4. The settlement channel (Stage 1: Contract) or reference (Stage 2).

Between listings: Amoeba may post heads and delegate under the same rules and slash as any staker. No other hand.

---

## 8. Implementation tasks

Ordered. Each item is independently shippable.

**A. Data model**
- [ ] Add `channel ∈ {SPOT, CONTRACT, LTA}` to Component. One product → three components.
- [ ] Add `target_sentence` (frozen per listed contract) and `terms` (the bracketed values) to Component.
- [ ] Add `mix {m_spot, m_contract, m_lta}` and `mix_basis ∈ {MATCHED, ASSUMED, NONE}` to each listed contract, per product.
- [ ] Add `settlement_target ∈ {CONTRACT_CHANNEL, FIXED_MIX_REFERENCE}` to each listed contract.
- [ ] Add `origin_id` to Source (hash of underlying report identity; fetcher-declared, challengeable).

**B. Source board**
- [ ] Heads are posted against a component, which fixes their channel. Board UI groups by product, then channel; LTA boards show open heads and stake piled on them even when empty.
- [ ] Head spec inherits the component's target sentence; fetcher sees the evidence table (§5) for that channel at fill time.

**C. Settlement**
- [ ] `min_live_sources` counts distinct `origin_id`s, not sources.
- [ ] Per-component eligibility check at listing (to list) and at freeze (to settle).
- [ ] Reference computation per product: unavailable if any channel < 3 origins or `mix_basis = NONE`; flagged if `mix_basis = ASSUMED`; never used for settlement unless `MATCHED`.
- [ ] Stage 1: settlement reads the Contract component's median only.

**D. Challenge grounds**
- [ ] Add challenge reason codes: `OFF_CHANNEL` (purchase classifies elsewhere or nowhere), `DUPLICATE_ORIGIN`, `EXPIRED_OBSERVATION`. Same bond, same resolution, same slash as any claim challenge. These are labels for the existing mechanism, not new flows.

**E. Listing terms document**
- [ ] A per-listing public page: products, basket weights, three sentences per product with terms filled, mix with basis and citation, settlement target, and the one-sentence disclosure "this contract measures periodic wholesale procurement outside LTAs."

**F. Display**
- [ ] Per product: settling index, with Spot / Contract / LTA medians and the reference (or "unavailable — [reason]") shown beside it. The empty LTA board and its open bounties are visible from the same view.

**G. Migration**
- [ ] Re-read every existing RAMX/NANDX source against the new Contract sentence. Sources that are actually spot references move to the Spot component. Sources that are republications collapse to one origin. Publish the before/after.

**H. Tests**
- [ ] Classification: a newly signed LTA → LTA; a reset under an old LTA → LTA; a periodic OEM negotiation → Contract; a distributor lot → Spot; an unterminated quote → none.
- [ ] Reference unavailable when any channel < 3 origins; unavailable when `mix_basis = NONE`; flagged when `ASSUMED`; settlement refuses `ASSUMED`.
- [ ] Three sources, one origin → fails `min_live_sources`.
- [ ] Expired LTA observation past reset → false claim on challenge.
- [ ] Listing-pass / freeze-fail → degraded path, not stale settlement.

---

## 9. Acceptance

Done when:
1. Every live product has three components with frozen sentences and filled terms on its current listing.
2. Stage 1 contracts settle on the Contract channel and the listing page says what that measures.
3. Spot, LTA, and reference display beside the index, with "unavailable" and its reason where applicable.
4. `min_live_sources` counts origins, and a duplicate-origin challenge resolves correctly on devnet.
5. The migration before/after for existing RAMX/NANDX sources is published.

---

## 10. Open items (not blocking Stage 1)

- Initial mix per product: find matched evidence, or launch the reference as a labeled assumed scenario.
- Whether LTA needs a banded / freely-resetting split. One channel until they visibly diverge.
- BATX: batteries do not have this three-channel structure; separate mold document.
- Whether origin independence should be promoted into the oracle mechanism spec as a formal rule for all buckets (recommended: yes).
