//! RAMX and NANDX commodity buckets and submission routing, from
//! `governance/commodity_submission_spec.v2.json`, and BATX's battery-cell categories, from
//! `governance/batx_basket_spec.v1.json` (the user's BATX brief of 2026-10-20).
//!
//! The spec has three layers, which map onto the oracle as:
//!
//! | Spec layer | Oracle |
//! |---|---|
//! | commodity bucket | an on-chain SKU of the product's chip manifest (tag 190) |
//! | comparison series | one source; its definition object is a [`SeriesDefinition`] |
//! | observation | one opening or update print of that source, described by an [`Observation`] |
//!
//! Contract eligibility is the month's recipe: the sources frozen at day 7 are its committed
//! series set. Routing ([`route_series`], [`route_series_in_month`], [`route_observation`])
//! is deterministic and off chain: petri runs it before it sends, the API runs it on every
//! submission. The program needs no change.
//!
//! The user moved RAMX and NANDX from modules and SSDs to these chip buckets on 2026-10-20, for
//! every month expiring after 2026-11-01. NANDX's were fabricated-wafer assessments until
//! 2026-10-22, when the user made them packaged NAND chips like RAMX's (spec v2; same bucket
//! labels, so the same on-chain SKU list). BATX starts with the months
//! expiring 2026-12-01 ([`BATX_FIRST_EXPIRY`]); [`index`] is its brief's fixed-base index.
//!
//! Oracle v2's long-term agreement layer observes every bucket in three channels, Spot,
//! Contract and LTA: [`molds`] compiles `governance/channel_molds.v1.json`, each product's
//! components with their frozen sentences and hashes, mixes, evidence tables and listing terms
//! document (`oracle/docs/lta.md`).

mod buckets;
pub mod index;
pub mod molds;
mod routing;
mod series;
pub mod time;

pub use buckets::*;
pub use molds::{
    listing_mix_commitment, listing_mixes, listing_terms_document, listing_terms_sha256, molds,
    origin_id, Channel, ChannelMold, Component, MixBasis, Mold, MoldItem, Molds, Quarter,
    SettlementTarget,
};
pub use routing::*;
pub use series::*;

/// The first expiry on the chip buckets: months expiring after 2026-11-01 00:00 UTC (the
/// launch months, Oct 26, keep their 13/22 CFM categories).
pub const CHIP_BUCKETS_AFTER_EXPIRY: i64 = 1_793_491_200;

/// Whether a month with this expiry (unix seconds) is on the chip buckets.
pub fn uses_chip_buckets(expiry_ts: i64) -> bool {
    expiry_ts > CHIP_BUCKETS_AFTER_EXPIRY
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_batx;
#[cfg(test)]
mod tests_molds;
