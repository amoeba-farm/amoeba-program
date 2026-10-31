use crate::liability::{self, Held};
use crate::outcome::{self, Challenge};
use crate::settle::{self, ComponentSettlement, SourceInput};
use crate::{median, rewards, thresholds, time, Params};

const AMBA: u64 = 1_000_000_000;

#[test]
fn weighted_median_with_equal_weights_is_the_equal_weight_median() {
    assert_eq!(
        median::weighted_median(&mut [(5, 1), (-3, 1), (9, 1)]),
        Some(5)
    );
    // Even count: the middle pair's midpoint, rounded toward zero like the oracle's median.
    assert_eq!(
        median::weighted_median(&mut [(1, 1), (2, 1), (4, 1), (8, 1)]),
        Some(3)
    );
    assert_eq!(median::weighted_median(&mut [(-1, 1), (-4, 1)]), Some(-2));
    assert_eq!(median::weighted_median(&mut [(-1, 1), (-2, 1)]), Some(-1));
    assert_eq!(median::weighted_median(&mut [(7, 0)]), None);
    assert_eq!(median::weighted_median(&mut []), None);
}

#[test]
fn a_heavy_source_cannot_drag_the_median_past_the_rest() {
    // 49% weight at +500 ppm cannot move the median off the 51% that sits at +10.
    let mut pairs = [(500, 49), (10, 26), (11, 25)];
    assert_eq!(median::weighted_median(&mut pairs), Some(11));
    // Zero weights are skipped, also when looking for the midpoint's partner.
    let mut pairs = [(1, 1), (100, 0), (3, 1)];
    assert_eq!(median::weighted_median(&mut pairs), Some(2));
}

#[test]
fn a_source_weighs_exactly_its_delegation() {
    // No source cap and no address cap (the user, 2026-10-29): the spec's 90/5/5 stays 90/5/5,
    // so the heavy source holds the median.
    let p = Params::spec_defaults();
    let live = |head: u32, change, weight| SourceInput {
        live: true,
        change_ppm: Some(change),
        weight,
        origin: [head as u8 + 1; 32],
        live_at: 1_000 + u64::from(head),
        head_id: head,
    };
    let sources = [
        live(0, 900, 9_000 * AMBA),
        live(1, 100, 500 * AMBA),
        live(2, 200, 500 * AMBA),
    ];
    assert_eq!(
        settle::settle_component(&p, &sources, 0, true),
        ComponentSettlement::Live {
            change_ppm: 900,
            eligible_sources: 3,
            duplicate_sources: 0,
            weight: 10_000 * AMBA,
        }
    );
}

#[test]
fn liability_and_escrow() {
    let p = Params::spec_defaults();
    assert_eq!(p.max_liability_bps(), 1_900);
    assert_eq!(liability::escrow_required(1_000, 1_000, 2), 190);
    assert_eq!(liability::escrow_required(1_000, 1_000, 1), 100);
    assert_eq!(
        liability::proposer_escrow_required(1_000, 1_000, 2, 10_000),
        1_000
    );
    // A 50% proposer slash with two strikes left: both cases can take 500 + 250 (review
    // 2026-10-30 #1), not just the first case's 500.
    assert_eq!(
        liability::proposer_escrow_required(1_000, 1_000, 2, 5_000),
        750
    );
    // An undelegated 1,000: 810 back after the cooldown, 190 held; one strike burns 100 and
    // leaves 90 needed for the last strike, so nothing more is released.
    let mut held = Held {
        principal: 1_000,
        held: 190,
    };
    assert_eq!(held.strike(1_000), 100);
    assert_eq!(
        held,
        Held {
            principal: 900,
            held: 90
        }
    );
    assert_eq!(
        held.releasable(liability::escrow_required(900, 1_000, 1)),
        0
    );
    assert_eq!(held.strike(1_000), 90);
    assert_eq!(held.held, 0);
}

#[test]
fn claim_accounting_conserves_and_pays_only_bonds() {
    let challenges = [
        Challenge {
            bond: 30,
            won: true,
        },
        Challenge {
            bond: 10,
            won: false,
        },
        Challenge {
            bond: 60,
            won: true,
        },
    ];
    let r = outcome::resolve_claim(100, &challenges);
    assert!(r.falls);
    // Winners split the claim bond 1:2 and get their bonds back; the loser's bond burns.
    assert_eq!(r.to_challengers, vec![30 + 33, 0, 60 + 67]);
    assert_eq!(r.burned, 10);
    assert_eq!(r.to_claimant, 0);
    let paid: u64 = r.to_challengers.iter().sum::<u64>() + r.to_claimant + r.burned;
    assert_eq!(paid, 100 + 100);

    let stands = outcome::resolve_claim(
        100,
        &[Challenge {
            bond: 25,
            won: false,
        }],
    );
    assert!(!stands.falls);
    assert_eq!(stands.to_claimant, 125);
}

#[test]
fn a_self_challenge_never_pays() {
    // One owner holds a claim and the challenge against it. Their USDC nets to zero (the claim
    // bond moves between their own wallets) and nothing else moves: no gain.
    let p = Params::spec_defaults();
    let (bond, challenge) = (p.claim_bond, p.challenge_bond);
    let r = outcome::resolve_claim(
        bond,
        &[Challenge {
            bond: challenge,
            won: true,
        }],
    );
    let usdc_net = r.to_challengers[0] as i128 - challenge as i128 - bond as i128;
    assert_eq!(usdc_net, 0);
    // With a second challenger who loses, the self-challenger's own losing bond burns.
    let r = outcome::resolve_claim(
        bond,
        &[
            Challenge {
                bond: challenge,
                won: true,
            },
            Challenge {
                bond: challenge,
                won: false,
            },
        ],
    );
    assert_eq!(r.burned, challenge);

    // A proposer who source-challenges their own bogus head collects at most their own
    // delegation back.
    let s = outcome::resolve_source(
        &[Challenge {
            bond: challenge,
            won: true,
        }],
        5_000,
        10_000,
    );
    assert_eq!(s.proposer_slashed, 5_000);
    assert_eq!(s.amba_to_challengers, vec![5_000]);
    assert_eq!(s.bonds_to_challengers, vec![challenge]);
}

#[test]
fn source_accounting() {
    let challenges = [
        Challenge {
            bond: 10,
            won: true,
        },
        Challenge {
            bond: 30,
            won: true,
        },
    ];
    let s = outcome::resolve_source(&challenges, 1_001, 10_000);
    assert_eq!(s.amba_to_challengers, vec![250, 751]);
    let stands = outcome::resolve_source(
        &[Challenge {
            bond: 10,
            won: false,
        }],
        1_001,
        10_000,
    );
    assert_eq!(stands.to_proposer, 10);
    assert_eq!(stands.proposer_slashed, 0);
}

#[test]
fn thresholds_follow_the_board() {
    let p = Params::spec_defaults();
    // Empty bucket: the floors.
    assert_eq!(
        thresholds::min_post(p.floor_post, p.alpha_bps, 0, 0),
        p.floor_post
    );
    assert_eq!(
        thresholds::activation_threshold(p.floor_activate, p.beta_bps, &mut []),
        p.floor_activate
    );
    // 400k delegated over 4 live sources: posting takes a quarter of 100k.
    assert_eq!(
        thresholds::min_post(0, 2_500, 400_000 * AMBA, 4),
        25_000 * AMBA
    );
    let mut live = [10 * AMBA, 40 * AMBA, 30 * AMBA, 20 * AMBA];
    assert_eq!(
        thresholds::activation_threshold(0, 5_000, &mut live),
        12_500_000_000
    );
    assert!(
        !thresholds::activates(100, 1, 50, 2),
        "the proposer alone cannot activate"
    );
    assert!(thresholds::activates(100, 2, 50, 2));
}

#[test]
fn an_ended_months_surplus_is_what_no_prize_can_claim() {
    // Covered: everything above the points, whatever is paid so far.
    assert_eq!(rewards::prize_surplus(20, 5, 0, false), 15);
    assert_eq!(rewards::prize_surplus(20, 0, 0, true), 20);
    // Short: only the rounding left once every prize is paid, nothing before.
    assert_eq!(rewards::prize_surplus(10, 30, 9, false), 0);
    assert_eq!(rewards::prize_surplus(10, 30, 9, true), 1);
}

#[test]
fn prizes_scale_down_when_the_pool_runs_dry() {
    assert_eq!(rewards::split_inflow(1_000, 2_000), (200, 800));
    assert_eq!(rewards::prize_payout(50, 100, 200), 50);
    assert_eq!(rewards::prize_payout(50, 100, 40), 20);
    let mut p = Params::spec_defaults();
    p.fill_prize_slope_ppb = 2_000;
    assert_eq!(
        rewards::fill_prize(&p, 1_000_000_000),
        p.base_fill_prize + 2_000
    );
}

#[test]
fn reward_index_pays_pro_rata() {
    let index = rewards::index_after(0, 1_000_000, 3 * AMBA).unwrap();
    let a = rewards::earned(AMBA, 0, index);
    let b = rewards::earned(2 * AMBA, 0, index);
    assert!(a + b <= 1_000_000 && 1_000_000 - (a + b) <= 2);
    assert_eq!(rewards::index_after(0, 5, 0), None);
    // Huge index steps saturate rather than overflow.
    assert_eq!(rewards::earned(u64::MAX, 0, u128::MAX), u64::MAX);
}

#[test]
fn calendar_and_window() {
    // 2026-11-01 00:00 UTC is a Sunday; five business days back is Monday 2026-10-26.
    let expiry = time::days_from_civil(2026, 11, 1) as u64 * 86_400;
    assert_eq!(expiry, 1_793_491_200);
    let start = time::settlement_window_start(expiry, 5);
    assert_eq!(start, time::days_from_civil(2026, 10, 26) as u64 * 86_400);
    let id = time::month_id(expiry);
    assert_eq!(time::month_start(id), expiry);
    assert_eq!(time::month_id(expiry - 1), id - 1);
    for days in [-1_000i64, 0, 10_000, 20_000, 50_000] {
        let (y, m, d) = time::civil_from_days(days);
        assert_eq!(time::days_from_civil(y, m, d), days);
    }
    let mut acc = time::Accrual::default();
    acc.set(10, 100);
    acc.set(20, 110);
    assert_eq!(acc.at(120), 10 * 10 + 20 * 10);
    assert_eq!(time::average(acc.at(120), 20), 15);
}

#[test]
fn contracts_cover_calendar_quarters() {
    // 2027-04-01 00:00 UTC: "Q1 2027" expires then.
    let apr = 1_806_537_600;
    assert!(time::is_quarter_boundary(apr));
    assert_eq!(time::contract_quarter(apr), (2027, 1));
    assert_eq!(time::quarter_start(2027, 1), 1_798_761_600);
    assert_eq!(time::quarter_start(2027, 2), apr);
    // 2027-01-01 closes Q4 2026; 2026-12-01 is no boundary but inside Q4 2026.
    assert_eq!(time::contract_quarter(1_798_761_600), (2026, 4));
    assert!(!time::is_quarter_boundary(1_796_083_200));
    assert_eq!(time::contract_quarter(1_796_083_200), (2026, 4));
    assert!(!time::is_quarter_boundary(apr + 1));
    assert!(!time::is_quarter_boundary(1_801_440_000)); // 2027-02-01
}

#[test]
fn settlement_values_and_index() {
    assert_eq!(settle::change_ppm(500_000_000, 510_000_000), Some(20_000));
    assert_eq!(settle::change_ppm(3, 2), Some(-333_333));
    assert_eq!(
        settle::index_change_ppm(&[(6_000, 10_000), (4_000, -5_000)]),
        Some(4_000)
    );
    assert_eq!(settle::index_change_ppm(&[(6_000, 10_000)]), None);
    assert_eq!(
        settle::settlement_value(500_000_000, 20_000),
        Some(510_000_000)
    );
    assert_eq!(settle::settlement_value(3, 166_666), Some(3));
    assert_eq!(settle::settlement_value(1, 500_000), Some(2)); // 1.5 rounds half up
    assert_eq!(settle::settlement_value(1, -2_000_000), Some(0));
    let prints = [(10, 100), (20, 104), (30, 101), (40, 103)];
    assert_eq!(settle::temporal_level(&prints, 15, 35), Some(102));
    assert_eq!(settle::temporal_level(&prints, 41, 50), Some(103));
    assert_eq!(settle::temporal_level(&prints, 1, 5), None);
}

#[test]
fn components_settle_live_or_degrade() {
    let mut p = Params::spec_defaults();
    let w = 10_000 * AMBA;
    let live = |head: u32, c, weight| SourceInput {
        live: true,
        change_ppm: Some(c),
        weight,
        origin: [head as u8 + 1; 32],
        live_at: 1_000 + u64::from(head),
        head_id: head,
    };
    let sources = [
        live(0, 100, w),
        live(1, 200, w),
        live(2, 300, w),
        live(3, 5_000, 3 * w),
    ];
    // Uncapped: the heavy source has half the weight, so the median is the midpoint of 300 and
    // 5,000.
    let s = settle::settle_component(&p, &sources, 0, true);
    assert_eq!(
        s,
        ComponentSettlement::Live {
            change_ppm: 2_650,
            eligible_sources: 4,
            duplicate_sources: 0,
            weight: 6 * w,
        }
    );
    // One source settles on its own at the default `min_live_sources` of 1.
    assert_eq!(p.min_live_sources, 1);
    assert!(p.validate());
    assert_eq!(
        settle::settle_component(&p, &sources[..1], 0, true).change_ppm(),
        100
    );
    // With a board's `min_live_sources` of 3, two eligible sources settle on the last valid
    // value, flagged.
    p.min_live_sources = 3;
    let retired = |head| SourceInput {
        live: false,
        ..live(head, 1, w)
    };
    let s = settle::settle_component(
        &p,
        &[live(0, 1, w), live(1, 2, w), retired(2), retired(3)],
        77,
        true,
    );
    assert_eq!(
        s,
        ComponentSettlement::LastValid {
            change_ppm: 77,
            reason: settle::Degraded::TooFewOrigins
        }
    );
    // Too little weight.
    p.min_live_sources = 1;
    p.min_settle_weight = 4 * w;
    assert_eq!(
        settle::settle_component(&p, &sources[..3], 5, true),
        ComponentSettlement::LastValid {
            change_ppm: 5,
            reason: settle::Degraded::TooLittleWeight
        }
    );
    p.min_live_sources = 0;
    assert!(!p.validate());
}
