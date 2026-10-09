//! One price-ordered choice across the existing option-market sources. A chunk
//! retains the ordinary router's LP/FIFO/preissued-writer tie order; shared
//! primary issuance or retirement competes at its actual eligible price.
use crate::{
    ameba_dlmm_math::{AmoebaDlmmBinLiquidity, AmoebaDlmmMathError, AmoebaDlmmSwapDirection},
    dlmm_order_state::DlmmOrder,
    state::WriterDlmmBinV1,
    writer_dlmm_quote::{
        quote_validated_sources_prepared, validate_route_sources, PublicOrderRouteLimits,
        WriterDlmmRouteConfig, WriterDlmmRouteQuote, WriterDlmmSwapPolicy,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AtomicOptionChunk {
    OrdinaryAndPreissued(WriterDlmmRouteQuote),
    SharedPrimary(WriterDlmmRouteQuote),
}

impl AtomicOptionChunk {
    pub fn route(&self) -> &WriterDlmmRouteQuote {
        match self {
            Self::OrdinaryAndPreissued(route) | Self::SharedPrimary(route) => route,
        }
    }
}

/// Quote one nonempty chunk, stopping normal sources at the next primary price.
/// The caller applies its canonical reserve,
/// order, writer-book and cash changes before calling again for the remaining
/// exact quantity. Aggregate slippage and whole-action buyback budgets belong
/// to that caller; this helper never assumes future credits or retirements.
#[allow(clippy::too_many_arguments)]
pub fn quote_next_atomic_option_chunk(
    config: WriterDlmmRouteConfig,
    ordinary: &[AmoebaDlmmBinLiquidity],
    writer: &[WriterDlmmBinV1],
    policy: Option<&WriterDlmmSwapPolicy>,
    orders: &[DlmmOrder],
    primary: &[WriterDlmmBinV1],
    primary_policy: Option<&WriterDlmmSwapPolicy>,
    limits: PublicOrderRouteLimits,
) -> Result<Option<AtomicOptionChunk>, AmoebaDlmmMathError> {
    quote_next_atomic_option_chunk_prepared(
        config,
        ordinary,
        writer,
        policy,
        orders,
        primary,
        primary_policy,
        limits,
        None,
    )
}

/// Reuse a reserve projection authenticated against the current writer book.
/// The caller refreshes it after each committed chunk before continuing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quote_next_atomic_option_chunk_prepared(
    config: WriterDlmmRouteConfig,
    ordinary: &[AmoebaDlmmBinLiquidity],
    writer: &[WriterDlmmBinV1],
    policy: Option<&WriterDlmmSwapPolicy>,
    orders: &[DlmmOrder],
    primary: &[WriterDlmmBinV1],
    primary_policy: Option<&WriterDlmmSwapPolicy>,
    limits: PublicOrderRouteLimits,
    prepared: Option<&crate::writer_sleeve_math::PreparedWriterReserve>,
) -> Result<Option<AtomicOptionChunk>, AmoebaDlmmMathError> {
    let ascending = config.direction == AmoebaDlmmSwapDirection::QuoteForOption;
    let limits = PublicOrderRouteLimits {
        allow_partial: true,
        ..limits
    };
    let config = WriterDlmmRouteConfig {
        minimum_amount_out: 0,
        ..config
    };
    validate_route_sources(config, ordinary, writer, policy, orders, limits)?;
    validate_route_sources(config, &[], primary, primary_policy, &[], limits)?;
    // Preserve amount validation even when every source prefix is empty.
    crate::ameba_dlmm_math::calculate_fees(config.amount_in)?;
    let within_limit = |bin: u16| {
        if ascending {
            bin <= config.limit_bin_id
        } else {
            bin >= config.limit_bin_id
        }
    };
    let reaches_unloaded = |bin: u16| {
        config.unloaded_ordinary_boundary.is_some_and(|boundary| {
            if ascending {
                bin >= boundary
            } else {
                bin <= boundary
            }
        })
    };
    let mut prices: Vec<u16> = ordinary
        .iter()
        .map(|b| b.bin_id)
        .chain(writer.iter().map(|b| b.bin_id))
        .chain(orders.iter().map(|o| o.limit_bin))
        .chain(primary.iter().map(|b| b.bin_id))
        .filter(|bin| within_limit(*bin))
        .collect();
    prices.sort_unstable();
    prices.dedup();
    if !ascending {
        prices.reverse();
    }
    for (price_index, &price) in prices.iter().enumerate() {
        // A cached later source cannot hide a potentially better missing page.
        if reaches_unloaded(price) {
            return Err(AmoebaDlmmMathError::InvalidRoute);
        }
        let at_price = WriterDlmmRouteConfig {
            limit_bin_id: price,
            maximum_bins: 1,
            // The complete boundary is checked above. This one-price slice
            // contains everything supplied at this price and cannot cross it.
            unloaded_ordinary_boundary: None,
            ..config
        };
        // Ordinary LP, FIFO and preissued writer liquidity already share one
        // canonical multi-price router. Batch its prefix through the next
        // primary price, including the tie that ordinary sources win. Never
        // cross an unauthenticated LP page while forming that prefix.
        let mut normal_end = price;
        for &candidate in &prices[price_index..] {
            if reaches_unloaded(candidate) {
                break;
            }
            normal_end = candidate;
            // A bid-only primary row cannot interrupt an ordinary buy prefix,
            // nor an ask-only row an ordinary sell prefix. The canonical quote
            // already ignores that opposite-side capacity. Preserve its full
            // financial admission at every direction-relevant primary price.
            if price_slice(primary, candidate, |b| b.bin_id)
                .iter()
                .any(|row| {
                    if ascending {
                        row.option_atoms != 0
                    } else {
                        row.quote_atoms != 0
                    }
                })
            {
                break;
            }
        }
        let ordinary_at = price_range(ordinary, price, normal_end, |b| b.bin_id);
        let writer_at = price_range(writer, price, normal_end, |b| b.bin_id);
        let orders_at = price_range(orders, price, normal_end, |o| o.limit_bin);
        if !ordinary_at.is_empty() || !writer_at.is_empty() || !orders_at.is_empty() {
            let route = quote_validated_sources_prepared(
                WriterDlmmRouteConfig {
                    limit_bin_id: normal_end,
                    maximum_bins: config.maximum_bins,
                    ..at_price
                },
                ordinary_at,
                writer_at,
                policy,
                orders_at,
                limits,
                false,
                prepared.filter(|_| policy.is_some()),
            )?;
            if route.quote.amount_in > 0 && route.quote.amount_out > 0 {
                return Ok(Some(AtomicOptionChunk::OrdinaryAndPreissued(route)));
            }
        }
        let primary_at = price_slice(primary, price, |b| b.bin_id);
        if !primary_at.is_empty() {
            let policy = primary_policy.ok_or(AmoebaDlmmMathError::InvalidRoute)?;
            let route = quote_validated_sources_prepared(
                at_price,
                &[],
                primary_at,
                Some(policy),
                &[],
                limits,
                true,
                prepared,
            )?;
            if route.quote.amount_in > 0 && route.quote.amount_out > 0 {
                return Ok(Some(AtomicOptionChunk::SharedPrimary(route)));
            }
        }
    }
    if config.unloaded_ordinary_boundary.is_some_and(within_limit) {
        return Err(AmoebaDlmmMathError::InvalidRoute);
    }
    Ok(None)
}

fn price_slice<T>(rows: &[T], price: u16, bin: impl Fn(&T) -> u16) -> &[T] {
    price_range(rows, price, price, bin)
}

// Complete source ordering was validated before slicing. Both direction-sorted
// LP/FIFO rows and ascending writer rows have a contiguous inclusive range.
fn price_range<T>(rows: &[T], first: u16, last: u16, bin: impl Fn(&T) -> u16) -> &[T] {
    let lower = first.min(last);
    let upper = first.max(last);
    let within = |row: &T| (lower..=upper).contains(&bin(row));
    let Some(start) = rows.iter().position(&within) else {
        return &[];
    };
    let end = start + rows[start..].iter().take_while(|row| within(row)).count();
    &rows[start..end]
}
