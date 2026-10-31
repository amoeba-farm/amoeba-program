//! Oracle v2, "heads on pikes" (`oracle/docs/heads.md`): the arithmetic every implementation
//! of the board shares, so the Solana program, the chain, the API and petri compute the same
//! numbers.
//!
//! Stakers delegate AMBA to a source; that delegation is its weight. A contract settles on
//! the weighted median of its sources' changes, per component, each source weighted by its
//! time-weighted delegation over the accumulation window, with no source or address cap (the
//! user, 2026-10-29). Lost source challenges strike and slash; a falling claim costs only its
//! bond; nothing slashed is ever distributed but the proposer's slash to the winners.
//!
//! The channel layer (`oracle/docs/lta.md`, [`channels`]) splits every basket item into three
//! components, Spot, Contract and LTA; thresholds count independent origins, a duplicate origin
//! carries no weight, and a contract settles on each item's fixed-mix reference: its three
//! channels weighted against each other by the item's mix (assumed or matched), each at its own
//! settlement, live or last valid, so the mix never shifts.
//!
//! Everything here is integer arithmetic with fixed rounding, so two implementations agree to
//! the atom. No dependencies: the oracle program links it on SBF.

pub mod channels;
pub mod liability;
pub mod median;
pub mod outcome;
pub mod params;
pub mod rewards;
pub mod settle;
pub mod thresholds;
pub mod time;

pub use channels::{Channel, MixBasis, SettlementTarget};
pub use median::weighted_median;
pub use params::Params;

/// Basis points in one.
pub const BPS: u64 = 10_000;
/// Parts per million in one: changes and the index move in ppm.
pub const PPM: i64 = 1_000_000;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_channels;
