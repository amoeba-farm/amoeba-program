//! The channel layer's tests (the brief `oracle/docs/lta-spec.md`, section 8.H, as far as they
//! are arithmetic).

use crate::channels::{
    self, ChallengeReason, Channel, ChannelReading, ItemSettlement, LtaPrint, LtaTerms, MixBasis,
    OriginEntry, PurchaseFacts, Reference, Refusal, SettlementTarget, Unavailable,
};
use crate::outcome::{self, Challenge};
use crate::settle::{self, ComponentSettlement, Degraded, SourceInput};
use crate::Params;

const AMBA: u64 = 1_000_000_000;
const DAY: u64 = 86_400;

fn origin(n: u8) -> [u8; 32] {
    [n; 32]
}

/// A board that needs three independent origins per component (the default is one).
fn three_origins() -> Params {
    let mut p = Params::spec_defaults();
    p.min_live_sources = 3;
    p
}

fn source(head: u32, origin_byte: u8, change: i64) -> SourceInput {
    SourceInput {
        live: true,
        change_ppm: Some(change),
        weight: 10_000 * AMBA,
        origin: origin(origin_byte),
        live_at: 1_000 + u64::from(head),
        head_id: head,
    }
}

/// The brief's classification examples (section 8.H) and the edges of its hierarchy, as a
/// table: `(case, facts, channel)`. The same cases, in words, are the mold's
/// `classificationExamples` (`governance/channel_molds.v1.json`), which ameba-commodity-spec's
/// tests run through [`channels::classify`].
#[test]
fn classification_follows_the_hierarchy() {
    let agreed = PurchaseFacts {
        agreed: true,
        ..Default::default()
    };
    let lta = |months| PurchaseFacts {
        agreement_months: months,
        committed_volume: true,
        negotiated_wholesale: true,
        delivered_in_period: true,
        ..agreed
    };
    let negotiated = PurchaseFacts {
        negotiated_wholesale: true,
        delivered_in_period: true,
        ..agreed
    };
    let table: [(&str, PurchaseFacts, Option<Channel>); 9] = [
        ("a newly signed LTA", lta(36), Some(Channel::Lta)),
        ("a reset under an old LTA", lta(48), Some(Channel::Lta)),
        (
            "a periodic OEM negotiation",
            negotiated,
            Some(Channel::Contract),
        ),
        (
            "a distributor lot",
            PurchaseFacts {
                immediate_inventory: true,
                delivered_in_period: true,
                ..agreed
            },
            Some(Channel::Spot),
        ),
        (
            "an unterminated quote",
            PurchaseFacts {
                agreed: false,
                immediate_inventory: true,
                ..Default::default()
            },
            None,
        ),
        (
            "a one-year agreement is long enough",
            lta(12),
            Some(Channel::Lta),
        ),
        (
            "an eleven-month agreement is not an LTA",
            lta(11),
            Some(Channel::Contract),
        ),
        (
            "a frame agreement without committed volume",
            PurchaseFacts {
                agreement_months: 24,
                ..negotiated
            },
            Some(Channel::Contract),
        ),
        (
            "a negotiated purchase delivered outside the period, not from stock",
            PurchaseFacts {
                delivered_in_period: false,
                ..negotiated
            },
            None,
        ),
    ];
    for (case, facts, expected) in table {
        assert_eq!(channels::classify(&facts), expected, "{case}");
    }
    // LTA outranks the others: a purchase under an LTA delivered from stock is still LTA.
    let from_stock_under_lta = PurchaseFacts {
        immediate_inventory: true,
        ..lta(24)
    };
    assert_eq!(
        channels::classify(&from_stock_under_lta),
        Some(Channel::Lta)
    );
}

#[test]
fn codes_are_the_on_chain_bytes() {
    for (c, code) in Channel::ALL.into_iter().zip(0u8..) {
        assert_eq!(c.code(), code);
        assert_eq!(Channel::from_code(code), Some(c));
    }
    assert_eq!(Channel::from_code(3), None);
    for (b, code) in [MixBasis::None, MixBasis::Assumed, MixBasis::Matched]
        .into_iter()
        .zip(0u8..)
    {
        assert_eq!((b.code(), MixBasis::from_code(code)), (code, Some(b)));
    }
    assert_eq!(SettlementTarget::ContractChannel.code(), 0);
    assert_eq!(SettlementTarget::FixedMixReference.code(), 1);
    assert_eq!(SettlementTarget::from_code(2), None);
    for (r, code) in [
        ChallengeReason::ValueOrOther,
        ChallengeReason::OffChannel,
        ChallengeReason::DuplicateOrigin,
        ChallengeReason::ExpiredObservation,
    ]
    .into_iter()
    .zip(0u8..)
    {
        assert_eq!(
            (r.code(), ChallengeReason::from_code(code)),
            (code, Some(r))
        );
    }
    assert_eq!(ChallengeReason::from_code(4), None);
    // 1 and 3 are retired (a short or degraded channel enters at its last valid value).
    assert_eq!(
        Unavailable::MixBasisNone.code(),
        2,
        "the program's REFERENCE_REASON_MIX_BASIS_NONE"
    );
    assert_eq!(Unavailable::InvalidMix.code(), 4);
}

#[test]
fn three_sources_one_origin_fail_min_live_sources() {
    let p = three_origins();
    let entries: Vec<OriginEntry> = (0..3)
        .map(|head| OriginEntry {
            origin: origin(9),
            live_at: 1_000 + u64::from(head),
            head_id: head,
            counts: true,
        })
        .collect();
    assert_eq!(channels::duplicate_flags(&entries), vec![false, true, true]);
    assert_eq!(channels::distinct_origins(&entries), 1);
    assert!(!channels::origins_pass(1, p.min_live_sources));
    // The settling component cannot list.
    assert!(!channels::component_lists(1, p.min_live_sources, true));
    // Three pages of one report don't settle a component either.
    let sources = [source(0, 9, 10), source(1, 9, 20), source(2, 9, 30)];
    let s = settle::settle_component(&p, &sources, 5, true);
    assert_eq!(
        s,
        ComponentSettlement::LastValid {
            change_ppm: 5,
            reason: Degraded::TooFewOrigins
        }
    );
}

#[test]
fn duplicates_follow_the_earliest_live_source_in_any_order() {
    let e = |o, live_at, head_id, counts| OriginEntry {
        origin: origin(o),
        live_at,
        head_id,
        counts,
    };
    // Head 7 went live first, so it carries origin 1 whichever way the list is ordered; a tie
    // on live time goes to the lower head id.
    let entries = [e(1, 50, 3, true), e(1, 40, 7, true), e(2, 40, 9, true)];
    assert_eq!(
        channels::duplicate_flags(&entries),
        vec![true, false, false]
    );
    let reversed: Vec<OriginEntry> = entries.iter().rev().copied().collect();
    assert_eq!(
        channels::duplicate_flags(&reversed),
        vec![false, false, true]
    );
    let tie = [e(4, 10, 6, true), e(4, 10, 5, true)];
    assert_eq!(channels::duplicate_flags(&tie), vec![true, false]);
    // A source outside the counted set (retired, or without a level) neither is nor makes a
    // duplicate: the next counting source of that origin carries it.
    let retired_first = [e(1, 10, 1, false), e(1, 20, 2, true)];
    assert_eq!(
        channels::duplicate_flags(&retired_first),
        vec![false, false]
    );
    assert_eq!(channels::distinct_origins(&retired_first), 1);
}

#[test]
fn a_duplicate_carries_no_weight() {
    let p = Params::spec_defaults();
    // Origin 1 is published twice (heads 0 and 1). Counted per page it would hold half the
    // weight and pull the median to the midpoint of 200 and 9,000; counted per origin it is one
    // source of three.
    let sources = [
        source(0, 1, 9_000),
        source(1, 1, 9_000),
        source(2, 2, 100),
        source(3, 3, 200),
    ];
    let s = settle::settle_component(&p, &sources, 0, true);
    let ComponentSettlement::Live {
        change_ppm,
        eligible_sources,
        duplicate_sources,
        weight,
    } = s
    else {
        panic!("{s:?}")
    };
    assert_eq!((eligible_sources, duplicate_sources), (3, 1));
    assert_eq!(weight, 30_000 * AMBA, "the duplicate carries no weight");
    assert_eq!(change_ppm, 200);
    let (eligible, duplicate) = settle::eligibility(&sources);
    assert_eq!(eligible, vec![true, false, true, true]);
    assert_eq!(duplicate, vec![false, true, false, false]);
    // Without the dedup the duplicated origin would have moved the median.
    let mut distinct = sources;
    distinct[1].origin = origin(5);
    let s = settle::settle_component(&p, &distinct, 0, true);
    assert_eq!(s.change_ppm(), 4_600);
}

#[test]
fn listing_counts_origins_and_display_components_list_anyway() {
    // Settling: `min_live_sources` origins (1 by default; 3 on a board that sets it).
    assert!(channels::component_lists(1, 1, true));
    assert!(!channels::component_lists(0, 1, true));
    assert!(channels::component_lists(3, 3, true));
    assert!(!channels::component_lists(2, 3, true));
    // Display (a channel the target does not settle on): an empty board still lists.
    assert!(channels::component_lists(0, 3, false));
    // Which channels settle: the fixed mix on every channel it weights, the Contract-channel
    // target on Contract alone.
    let mix = [1_000, 5_000, 4_000];
    let contract = SettlementTarget::ContractChannel;
    assert!(channels::settles(contract, Channel::Contract, &mix));
    assert!(!channels::settles(contract, Channel::Spot, &mix));
    assert!(!channels::settles(contract, Channel::Lta, &mix));
    let fixed = SettlementTarget::FixedMixReference;
    assert!(Channel::ALL
        .iter()
        .all(|&c| channels::settles(fixed, c, &mix)));
    let no_spot = [0, 6_000, 4_000];
    assert!(!channels::settles(fixed, Channel::Spot, &no_spot));
    assert!(channels::settles(fixed, Channel::Contract, &no_spot));
    assert!(channels::settles(fixed, Channel::Lta, &no_spot));
    // The fixed mix lists on assumed and matched mixes, never on none or an invalid mix.
    assert!(channels::listing_allowed(
        contract,
        &[(mix, MixBasis::None)]
    ));
    assert!(channels::listing_allowed(
        fixed,
        &[(mix, MixBasis::Matched), (no_spot, MixBasis::Assumed)]
    ));
    assert!(channels::listing_allowed(
        fixed,
        &[(mix, MixBasis::Assumed)]
    ));
    assert!(!channels::listing_allowed(
        fixed,
        &[(mix, MixBasis::Assumed), (mix, MixBasis::None)]
    ));
    assert!(!channels::listing_allowed(
        fixed,
        &[([1, 2, 3], MixBasis::Matched)]
    ));
    assert!(!channels::listing_allowed(fixed, &[]));
}

#[test]
fn an_expired_lta_observation_is_a_false_claim_and_no_level() {
    let reset = 100 * DAY;
    let terms = LtaTerms {
        effective_from_ts: 10 * DAY,
        next_reset_ts: reset,
        floor: 0,
        ceiling: 0,
    };
    assert!(terms.valid());
    // An old publication of a price still in effect is valid evidence.
    assert!(terms.current_at(reset - 1));
    assert!(!terms.expired_at(reset - 1));
    // Carried past its reset it is expired: the ExpiredObservation challenge stands on the
    // claim's own terms, and the claim falls like any other.
    assert!(terms.expired_at(reset) && !terms.current_at(reset));
    let r = outcome::resolve_claim(
        1_000_000,
        &[Challenge {
            bond: 1_000_000,
            won: terms.expired_at(reset + DAY),
        }],
    );
    assert!(r.falls);
    // Not yet effective is not current either; no reset declared never expires.
    assert!(!terms.current_at(DAY));
    let open_ended = LtaTerms {
        next_reset_ts: 0,
        ..terms
    };
    assert!(open_ended.current_at(u64::MAX) && !open_ended.expired_at(u64::MAX));

    // As a level: the window is the five business days before expiry. A print published long
    // ago and still in effect is the level; once its reset has passed, nothing is.
    let print = |ts, value, effective, next_reset| LtaPrint {
        source_ts: ts,
        value,
        terms: LtaTerms {
            effective_from_ts: effective,
            next_reset_ts: next_reset,
            floor: 0,
            ceiling: 0,
        },
    };
    let (start, end) = (93 * DAY, 100 * DAY);
    let old = [print(20 * DAY, 500, 20 * DAY, 120 * DAY)];
    assert_eq!(channels::lta_level(&old, start, end), Some(500));
    let lapsed = [print(20 * DAY, 500, 20 * DAY, 95 * DAY)];
    assert_eq!(channels::lta_level(&lapsed, start, end), None);
    // A reset inside the window: the new price, posted in advance, is the level.
    let reset_inside = [
        print(20 * DAY, 500, 20 * DAY, 95 * DAY),
        print(90 * DAY, 540, 95 * DAY, 185 * DAY),
    ];
    assert_eq!(channels::lta_level(&reset_inside, start, end), Some(540));
    // An announced future price is not yet the level.
    let announced = [
        print(20 * DAY, 500, 20 * DAY, 120 * DAY),
        print(96 * DAY, 580, 120 * DAY, 210 * DAY),
    ];
    assert_eq!(channels::lta_level(&announced, start, end), Some(500));
    // Contract and Spot prints carry no terms and take the plain temporal median.
    assert_eq!(
        channels::channel_level(Channel::Contract, &lapsed, start, end),
        Some(500)
    );
    assert_eq!(
        channels::channel_level(Channel::Lta, &lapsed, start, end),
        None
    );

    // An expired source has no level, so it is not eligible: two current LTA sources are not
    // enough to settle the component.
    let p = three_origins();
    let mut sources = [source(0, 1, 10), source(1, 2, 20), source(2, 3, 30)];
    sources[2].change_ppm = channels::lta_level(&lapsed, start, end).map(|_| 30);
    let s = settle::settle_component(&p, &sources, -4, true);
    assert_eq!(
        s,
        ComponentSettlement::LastValid {
            change_ppm: -4,
            reason: Degraded::TooFewOrigins
        }
    );
}

#[test]
fn lta_terms_bands() {
    let band = LtaTerms {
        effective_from_ts: 0,
        next_reset_ts: 0,
        floor: 90,
        ceiling: 110,
    };
    assert!(band.valid());
    assert!(band.within_band(90) && band.within_band(110));
    assert!(!band.within_band(89) && !band.within_band(111));
    let floor_only = LtaTerms { ceiling: 0, ..band };
    assert!(floor_only.within_band(u64::MAX));
    assert!(!LtaTerms { floor: 120, ..band }.valid());
    assert!(!LtaTerms {
        effective_from_ts: 50,
        next_reset_ts: 50,
        ..band
    }
    .valid());
}

#[test]
fn listing_pass_freeze_fail_goes_to_the_degraded_path() {
    let p = three_origins();
    let min = p.min_live_sources;
    // Listed with three origins.
    assert!(channels::component_lists(3, min, true));
    // At freeze one source turned out to repeat another's origin: two origins, the check fails.
    let at_freeze = [
        OriginEntry {
            origin: origin(1),
            live_at: 1,
            head_id: 0,
            counts: true,
        },
        OriginEntry {
            origin: origin(2),
            live_at: 2,
            head_id: 1,
            counts: true,
        },
        OriginEntry {
            origin: origin(2),
            live_at: 3,
            head_id: 2,
            counts: true,
        },
    ];
    let origins = channels::distinct_origins(&at_freeze);
    assert_eq!(origins, 2);
    let freeze_passed = channels::origins_pass(origins, min);
    assert!(!freeze_passed);
    // By settlement the third source's update declares a new origin, so three are eligible;
    // the component still settles on its degraded path, not on sources that failed at freeze.
    let sources = [source(0, 1, 10), source(1, 2, 20), source(2, 3, 30)];
    let s = settle::settle_component(&p, &sources, 12, freeze_passed);
    assert_eq!(
        s,
        ComponentSettlement::LastValid {
            change_ppm: 12,
            reason: Degraded::FreezeCheckFailed
        }
    );
    assert!(settle::settle_component(&p, &sources, 12, true).is_live());
}

fn readings(changes: [i64; 3], degraded: [bool; 3]) -> [ChannelReading; 3] {
    [0, 1, 2].map(|i| ChannelReading {
        change_ppm: changes[i],
        degraded: degraded[i],
    })
}

#[test]
fn the_reference_weighs_the_channels_against_each_other() {
    let mix = [1_000, 5_000, 4_000];
    let live = readings([50_000, 20_000, -10_000], [false; 3]);
    // 0.1 * 5% + 0.5 * 2% + 0.4 * -1% = 1.1%.
    assert_eq!(
        channels::reference(&mix, MixBasis::Matched, &live),
        Reference::Available {
            change_ppm: 11_000,
            assumed: false,
            degraded: [false; 3],
        }
    );
    // Assumed: the same value, flagged.
    let assumed = channels::reference(&mix, MixBasis::Assumed, &live);
    assert_eq!(
        assumed,
        Reference::Available {
            change_ppm: 11_000,
            assumed: true,
            degraded: [false; 3],
        }
    );
    assert_eq!(assumed.degraded_mask(), 0);
    // No basis, or a mix that does not add up: unavailable, whatever the channels.
    assert_eq!(
        channels::reference(&mix, MixBasis::None, &live),
        Reference::Unavailable(Unavailable::MixBasisNone)
    );
    assert_eq!(
        channels::reference(&[1, 2, 3], MixBasis::Matched, &live),
        Reference::Unavailable(Unavailable::InvalidMix)
    );
    for u in [Unavailable::MixBasisNone, Unavailable::InvalidMix] {
        assert_eq!(Unavailable::from_code(u.code()), Some(u));
    }
    // The retired codes stay retired.
    assert_eq!(Unavailable::from_code(1), None);
    assert_eq!(Unavailable::from_code(3), None);
}

/// The brief's rule 6, as the user decided it (2026-10-29): a failing channel keeps its fixed
/// share at its last valid value. Nothing is redistributed and the mix never shifts.
#[test]
fn a_degraded_channel_keeps_its_share_at_its_last_valid_value() {
    let mix = [1_000, 5_000, 4_000];
    // LTA settled on its degraded path at its freeze value, -1%: still 40% of the blend.
    let lta_degraded = readings([50_000, 20_000, -10_000], [false, false, true]);
    let r = channels::reference(&mix, MixBasis::Assumed, &lta_degraded);
    assert_eq!(
        r,
        Reference::Available {
            change_ppm: 11_000,
            assumed: true,
            degraded: [false, false, true],
        }
    );
    assert_eq!(r.degraded_mask(), 0b100);
    // Short of origins at freeze: the listing value, 0, at its 40%. Not re-mixed onto the other
    // two (that would be 0.1/0.6 * 5% + 0.5/0.6 * 2% = 2.5%).
    let lta_failed = readings([50_000, 20_000, 0], [false, false, true]);
    assert_eq!(
        channels::reference(&mix, MixBasis::Matched, &lta_failed).change_ppm(),
        Some(15_000)
    );
    // A channel the mix gives no share is not flagged and moves nothing.
    let no_spot = [0, 6_000, 4_000];
    let spot_degraded = readings([999_999, 20_000, -10_000], [true, false, false]);
    let r = channels::reference(&no_spot, MixBasis::Matched, &spot_degraded);
    assert_eq!(r.change_ppm(), Some(8_000));
    assert_eq!(r.degraded_mask(), 0);
    // Readings come straight from a component's settlement, live or last valid.
    let last_valid = ComponentSettlement::LastValid {
        change_ppm: 3,
        reason: Degraded::TooLittleWeight,
    };
    assert_eq!(
        ChannelReading::new(&last_valid),
        ChannelReading {
            change_ppm: 3,
            degraded: true
        }
    );
    assert_eq!(channels::channel_mask(&[true, true, true]), 0b111);
}

#[test]
fn the_fixed_mix_settles_on_assumed_and_matched_mixes() {
    let item = |w, contract, reference| ItemSettlement {
        basket_weight_bps: w,
        contract_change_ppm: contract,
        reference,
    };
    let assumed = Reference::Available {
        change_ppm: 30_000,
        assumed: true,
        degraded: [false, false, true],
    };
    let matched = Reference::Available {
        change_ppm: -10_000,
        assumed: false,
        degraded: [false; 3],
    };
    let items = [item(6_000, 10_000, assumed), item(4_000, -5_000, matched)];
    // Fixed mix: 0.6 * 3% + 0.4 * -1%, the assumed item included.
    assert_eq!(
        channels::settlement_change_ppm(SettlementTarget::FixedMixReference, &items),
        Ok(14_000)
    );
    assert_eq!(channels::stage2_change_ppm(&items), Ok(14_000));
    // The Contract-channel target reads the Contract components only.
    assert_eq!(
        channels::settlement_change_ppm(SettlementTarget::ContractChannel, &items),
        Ok(4_000)
    );
    // An unavailable reference (mix basis none) is refused under the fixed mix only.
    let none = Reference::Unavailable(Unavailable::MixBasisNone);
    let items = [item(6_000, 0, matched), item(4_000, 0, none)];
    assert_eq!(
        channels::stage2_change_ppm(&items),
        Err(Refusal::Unavailable {
            item: 1,
            reason: Unavailable::MixBasisNone
        })
    );
    assert_eq!(channels::stage1_change_ppm(&items), Ok(0));
    assert_eq!(
        channels::stage1_change_ppm(&[item(5_000, 1, matched)]),
        Err(Refusal::BasketWeights)
    );
    assert_eq!(
        channels::stage2_change_ppm(&[item(5_000, 1, matched)]),
        Err(Refusal::BasketWeights)
    );
}

/// From component settlements to the index: every settling channel of every item enters at its
/// own settlement, a degraded one at its last valid value.
#[test]
fn the_index_from_component_settlements() {
    let p = Params::spec_defaults();
    let live = |changes: [i64; 3]| {
        let sources = [
            source(0, 1, changes[0]),
            source(1, 2, changes[1]),
            source(2, 3, changes[2]),
        ];
        settle::settle_component(&p, &sources, 0, true)
    };
    let mixes = [[1_000u16, 5_000, 4_000], [2_000, 6_000, 2_000]];
    let settlements = [
        [
            live([10_000, 20_000, 30_000]),
            live([1_000, 2_000, 3_000]),
            // LTA lost its sources after the freeze: its freeze value, 7,000.
            ComponentSettlement::LastValid {
                change_ppm: 7_000,
                reason: Degraded::TooFewOrigins,
            },
        ],
        [
            live([-5_000, -5_000, -5_000]),
            live([4_000, 4_000, 4_000]),
            live([0, 0, 0]),
        ],
    ];
    let items: Vec<ItemSettlement> = (0..2)
        .map(|i| {
            let readings = [0, 1, 2].map(|c| ChannelReading::new(&settlements[i][c]));
            ItemSettlement {
                basket_weight_bps: [7_000, 3_000][i],
                contract_change_ppm: settlements[i][1].change_ppm(),
                reference: channels::reference(&mixes[i], MixBasis::Assumed, &readings),
            }
        })
        .collect();
    // Item 0: 0.1 * 2% + 0.5 * 0.2% + 0.4 * 0.7% = 0.58%; item 1: 0.2 * -0.5% + 0.6 * 0.4% = 0.14%.
    assert_eq!(items[0].reference.change_ppm(), Some(5_800));
    assert_eq!(items[0].reference.degraded_mask(), 0b100);
    assert_eq!(items[1].reference.change_ppm(), Some(1_400));
    // 0.7 * 0.58% + 0.3 * 0.14% = 0.448%.
    assert_eq!(channels::stage2_change_ppm(&items), Ok(4_480));
}

#[test]
fn the_mix_change_rounds_toward_zero() {
    assert_eq!(
        channels::mix_change_ppm(&[3_333, 3_333, 3_334], &[1, 1, 1]),
        1
    );
    assert_eq!(channels::mix_change_ppm(&[5_000, 5_000, 0], &[3, 0, 0]), 1);
    assert_eq!(
        channels::mix_change_ppm(&[5_000, 5_000, 0], &[-3, 0, 0]),
        -1
    );
    assert_eq!(
        channels::mix_change_ppm(&[10_000, 0, 0], &[i64::MIN, 0, 0]),
        i64::MIN
    );
    assert!(channels::mix_is_valid(&[0, 10_000, 0]));
    assert!(!channels::mix_is_valid(&[u16::MAX, 0, 0]));
}
