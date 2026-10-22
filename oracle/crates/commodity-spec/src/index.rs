//! The fixed-base arithmetic index of the BATX brief (§4), as a deterministic reference
//! calculator with its diagnostics.
//!
//! ```text
//! R[c,s,t] = P[c,s,t] / P[c,s,t0]
//! C[c,t]   = sum_s( a[c,s] * R[c,s,t] )
//! I[t]     = 100 * sum_c( w[c] * C[c,t] )
//! ```
//!
//! `w` are the product's bucket weights, `a` the panel's within-bucket coefficients (bps of
//! the bucket, 10,000 per bucket), `P[t0]` each series' frozen base. Every intermediate keeps
//! 18 decimal places ([`SCALE`]); the result is rounded once, at output, to
//! [`OUTPUT_DECIMALS`] places, half up. A missing input is flagged and leaves the index unset:
//! no weight moves to another series or bucket, and nothing is carried forward, since no
//! carry-forward rule is approved (brief §7).
//!
//! The oracle settles a month with a different within-bucket rule: each source's change from
//! its opening in whole bps, the bucket's median of those, then `sum w * median` in whole
//! bps. With one series per bucket the two agree; `programs/ameba_oracle/tests/batx_index.rs`
//! runs the brief's acceptance vectors through the program's own functions.

use serde::Serialize;

use crate::buckets::Product;
use crate::time::days;

/// Fixed-point scale of every intermediate: 18 decimal places.
pub const SCALE: u128 = 1_000_000_000_000_000_000;
/// Decimal places of the reported index and diagnostics.
pub const OUTPUT_DECIMALS: u32 = 6;

/// One observed price of a series: the 6-decimal atoms of `price_atoms`, and its lineage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PricePoint<'a> {
    pub atoms: u64,
    pub observation_id: &'a str,
    /// The archive capture or evidence hash.
    pub evidence_reference: &'a str,
    /// The end of the economic period the price is for (`YYYY-MM-DD`).
    pub reference_period_end: &'a str,
}

/// One series of a contract's frozen panel.
#[derive(Clone, Copy, Debug)]
pub struct PanelSeries<'a> {
    pub series_id: &'a str,
    pub bucket: &'a str,
    /// `a[c,s]`, in bps of its bucket.
    pub coefficient_bps: u16,
    /// `P[c,s,t0]`, under the panel's common timing rule.
    pub base: Option<PricePoint<'a>>,
}

/// The price in force for a series at the calculation time.
#[derive(Clone, Copy, Debug)]
pub struct ObservedPrice<'a> {
    pub series_id: &'a str,
    pub point: PricePoint<'a>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexOptions<'a> {
    /// The calculation date (`YYYY-MM-DD`), for each observation's age.
    pub as_of: Option<&'a str>,
    /// Flag observations older than this many days. No stale-data limit is approved yet
    /// (brief §9), so callers leave it unset unless a contract defines one.
    pub stale_after_days: Option<u32>,
}

/// A condition that makes the index or a bucket unsettled, or that a reader should see.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "flag", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Flag {
    /// The product's bucket weights don't sum to 10,000.
    #[serde(rename_all = "camelCase")]
    WeightsNotWhole { sum_bps: u32 },
    /// A panel series names a bucket the product doesn't have.
    #[serde(rename_all = "camelCase")]
    UnknownBucket { series_id: String, bucket: String },
    /// A panel series is listed twice.
    #[serde(rename_all = "camelCase")]
    DuplicatePanelSeries { series_id: String },
    /// A bucket has no panel series.
    #[serde(rename_all = "camelCase")]
    BucketWithoutSeries { bucket: String },
    /// A bucket's coefficients don't sum to 10,000.
    #[serde(rename_all = "camelCase")]
    CoefficientsNotWhole { bucket: String, sum_bps: u32 },
    /// A panel series has no base price.
    #[serde(rename_all = "camelCase")]
    MissingBase { series_id: String, bucket: String },
    /// A panel series has no price in force: its bucket and the index stay unset.
    #[serde(rename_all = "camelCase")]
    MissingData { series_id: String, bucket: String },
    /// A price of zero: not an eligible positive price.
    #[serde(rename_all = "camelCase")]
    NonPositivePrice { series_id: String },
    /// A price for a series outside the panel: it changes nothing.
    #[serde(rename_all = "camelCase")]
    OutsidePanel { series_id: String },
    /// A second price for one series: only the first counts.
    #[serde(rename_all = "camelCase")]
    DuplicateObservation {
        series_id: String,
        observation_id: String,
    },
    /// The price in force is the base observation itself: nothing newer was observed.
    #[serde(rename_all = "camelCase")]
    NoNewObservation { series_id: String },
    /// Older than [`IndexOptions::stale_after_days`].
    #[serde(rename_all = "camelCase")]
    Stale { series_id: String, age_days: i64 },
    /// A relative above a billion times its base, outside the fixed-point range.
    #[serde(rename_all = "camelCase")]
    Overflow { series_id: String },
}

impl Flag {
    /// Whether it leaves the index unset.
    pub fn blocks_index(&self) -> bool {
        !matches!(
            self,
            Flag::OutsidePanel { .. }
                | Flag::DuplicateObservation { .. }
                | Flag::NoNewObservation { .. }
                | Flag::Stale { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesReport {
    pub series_id: String,
    pub coefficient_bps: u16,
    /// `R[c,s,t]`.
    pub relative: Option<String>,
    pub base_atoms: Option<u64>,
    pub current_atoms: Option<u64>,
    pub base_observation: Option<String>,
    pub current_observation: Option<String>,
    pub base_evidence: Option<String>,
    pub current_evidence: Option<String>,
    /// Days from the current price's period end to `as_of`.
    pub age_days: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryReport {
    pub bucket: String,
    pub weight_bps: u16,
    /// `C[c,t]`.
    pub relative: Option<String>,
    /// `100 * C[c,t]`: the bucket's own index.
    pub index: Option<String>,
    /// `100 * w[c] * C[c,t]`: its share of the total, in index points.
    pub contribution: Option<String>,
    /// `100 * w[c] * (C[c,t] - 1)`: how many index points it moved the total.
    pub change: Option<String>,
    pub series: Vec<SeriesReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    pub product: &'static str,
    /// `I[t]`, rounded to [`OUTPUT_DECIMALS`]; unset while any blocking flag stands.
    pub index: Option<String>,
    pub categories: Vec<CategoryReport>,
    pub flags: Vec<Flag>,
}

/// The brief's fixed-base index of `product` over its frozen `panel`, from the prices in
/// force (`observations`).
pub fn fixed_base_index(
    product: Product,
    panel: &[PanelSeries<'_>],
    observations: &[ObservedPrice<'_>],
    options: IndexOptions<'_>,
) -> IndexReport {
    let mut flags = Vec::new();
    let weight_sum: u32 = product
        .buckets()
        .iter()
        .map(|bucket| u32::from(bucket.weight_bps))
        .sum();
    if weight_sum != 10_000 {
        flags.push(Flag::WeightsNotWhole {
            sum_bps: weight_sum,
        });
    }
    let mut seen: Vec<&str> = Vec::new();
    for series in panel {
        if seen.contains(&series.series_id) {
            flags.push(Flag::DuplicatePanelSeries {
                series_id: series.series_id.to_string(),
            });
        }
        seen.push(series.series_id);
        if product.bucket(series.bucket).is_none() {
            flags.push(Flag::UnknownBucket {
                series_id: series.series_id.to_string(),
                bucket: series.bucket.to_string(),
            });
        }
    }
    let mut in_force: Vec<&ObservedPrice<'_>> = Vec::new();
    for observation in observations {
        if !panel
            .iter()
            .any(|series| series.series_id == observation.series_id)
        {
            flags.push(Flag::OutsidePanel {
                series_id: observation.series_id.to_string(),
            });
        } else if in_force
            .iter()
            .any(|other| other.series_id == observation.series_id)
        {
            flags.push(Flag::DuplicateObservation {
                series_id: observation.series_id.to_string(),
                observation_id: observation.point.observation_id.to_string(),
            });
        } else {
            in_force.push(observation);
        }
    }
    let as_of = options.as_of.and_then(days);

    let mut categories = Vec::new();
    // sum_c w[c] * C[c], in SCALE units times bps.
    let mut total: Option<u128> = Some(0);
    for bucket in product.buckets() {
        let members: Vec<&PanelSeries<'_>> = panel
            .iter()
            .filter(|series| series.bucket == bucket.id)
            .collect();
        let mut bucket_ok = true;
        if members.is_empty() {
            flags.push(Flag::BucketWithoutSeries {
                bucket: bucket.id.to_string(),
            });
            bucket_ok = false;
        }
        let coefficient_sum: u32 = members
            .iter()
            .map(|series| u32::from(series.coefficient_bps))
            .sum();
        if !members.is_empty() && coefficient_sum != 10_000 {
            flags.push(Flag::CoefficientsNotWhole {
                bucket: bucket.id.to_string(),
                sum_bps: coefficient_sum,
            });
            bucket_ok = false;
        }
        // sum_s a[c,s] * R[c,s], in SCALE units times bps.
        let mut weighted: u128 = 0;
        let mut series_reports = Vec::new();
        for series in members {
            let current = in_force
                .iter()
                .find(|observation| observation.series_id == series.series_id)
                .map(|observation| observation.point);
            let id = series.series_id.to_string();
            let mut relative = None;
            match (series.base, current) {
                (None, _) => {
                    flags.push(Flag::MissingBase {
                        series_id: id.clone(),
                        bucket: bucket.id.to_string(),
                    });
                    bucket_ok = false;
                }
                (Some(_), None) => {
                    flags.push(Flag::MissingData {
                        series_id: id.clone(),
                        bucket: bucket.id.to_string(),
                    });
                    bucket_ok = false;
                }
                (Some(base), Some(now)) if base.atoms == 0 || now.atoms == 0 => {
                    flags.push(Flag::NonPositivePrice {
                        series_id: id.clone(),
                    });
                    bucket_ok = false;
                }
                (Some(base), Some(now)) => {
                    match ratio(now.atoms, base.atoms).and_then(|r| {
                        weighted
                            .checked_add(r.checked_mul(u128::from(series.coefficient_bps))?)
                            .map(|sum| (r, sum))
                    }) {
                        Some((r, sum)) => {
                            weighted = sum;
                            relative = Some(r);
                        }
                        None => {
                            flags.push(Flag::Overflow {
                                series_id: id.clone(),
                            });
                            bucket_ok = false;
                        }
                    }
                    if now.observation_id == base.observation_id {
                        flags.push(Flag::NoNewObservation {
                            series_id: id.clone(),
                        });
                    }
                }
            }
            let age_days = current.and_then(|now| Some(as_of? - days(now.reference_period_end)?));
            if let (Some(age), Some(limit)) = (age_days, options.stale_after_days) {
                if age > i64::from(limit) {
                    flags.push(Flag::Stale {
                        series_id: id.clone(),
                        age_days: age,
                    });
                }
            }
            series_reports.push(SeriesReport {
                series_id: id,
                coefficient_bps: series.coefficient_bps,
                relative: relative.map(|r| decimal(r, SCALE)),
                base_atoms: series.base.map(|point| point.atoms),
                current_atoms: current.map(|point| point.atoms),
                base_observation: series.base.map(|point| point.observation_id.to_string()),
                current_observation: current.map(|point| point.observation_id.to_string()),
                base_evidence: series
                    .base
                    .map(|point| point.evidence_reference.to_string()),
                current_evidence: current.map(|point| point.evidence_reference.to_string()),
                age_days,
            });
        }
        // C[c] in SCALE units.
        let relative = bucket_ok.then(|| weighted / 10_000);
        let weight = u128::from(bucket.weight_bps);
        total = match (total, relative) {
            (Some(total), Some(c)) => c.checked_mul(weight).and_then(|wc| total.checked_add(wc)),
            _ => None,
        };
        categories.push(CategoryReport {
            bucket: bucket.id.to_string(),
            weight_bps: bucket.weight_bps,
            relative: relative.map(|c| decimal(c, SCALE)),
            index: relative.map(|c| decimal(c * 100, SCALE)),
            contribution: relative.map(|c| decimal(c * weight, SCALE * 100)),
            change: relative.map(|c| signed_decimal(c, weight)),
            series: series_reports,
        });
    }
    let blocked = flags.iter().any(Flag::blocks_index);
    // I = 100 * total / 10,000.
    let index = total
        .filter(|_| !blocked)
        .map(|total| decimal(total, SCALE * 100));
    IndexReport {
        product: product.symbol(),
        index,
        categories,
        flags,
    }
}

/// The largest relative the fixed-point reports hold: a billion times the base. It bounds
/// the arithmetic range so no report can overflow; it is not a price rule.
const MAX_RELATIVE: u128 = SCALE * 1_000_000_000;

/// `numerator / denominator` in SCALE units, truncated at the 18th place.
fn ratio(numerator: u64, denominator: u64) -> Option<u128> {
    let (n, d) = (u128::from(numerator), u128::from(denominator));
    (n / d)
        .checked_mul(SCALE)?
        .checked_add((n % d).checked_mul(SCALE)? / d)
        .filter(|r| *r <= MAX_RELATIVE)
}

/// `value / unit`, rounded half up to [`OUTPUT_DECIMALS`] places.
fn decimal(value: u128, unit: u128) -> String {
    let places = 10u128.pow(OUTPUT_DECIMALS);
    let scaled = value * places;
    let mut rounded = scaled / unit;
    if (scaled % unit) * 2 >= unit {
        rounded += 1;
    }
    format!(
        "{}.{:0width$}",
        rounded / places,
        rounded % places,
        width = OUTPUT_DECIMALS as usize
    )
}

/// `100 * w * (C - 1)` in index points, with its sign: `c` in SCALE units, `weight` in bps.
fn signed_decimal(c: u128, weight: u128) -> String {
    let (magnitude, sign) = if c >= SCALE {
        (c - SCALE, "")
    } else {
        (SCALE - c, "-")
    };
    let text = decimal(magnitude * weight, SCALE * 100);
    if sign == "-" && text.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
        format!("-{text}")
    } else {
        text
    }
}
