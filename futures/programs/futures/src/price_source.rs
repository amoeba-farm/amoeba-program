//! The AMBA price from mainnet's own accounts, with no trusted poster
//! (`PoolPrice`; the user, 2026-10-18; `docs/futures.md`, "Staked AMBA
//! backs the maker", **Price**).
//!
//! - AMBA/SOL: the Meteora DAMM v2 (CP-AMM) pool `AMBA_SOL_POOL`, owned by
//!   `DAMM_V2_PROGRAM`, token A AMBA (Token-2022, 9 decimals), token B wrapped
//!   SOL (9 decimals). Its `sqrt_price` is a Q64.64 square root of token B
//!   atoms per token A atom. Layout: `programs/cp-amm/src/state/pool.rs` and
//!   `state/fee.rs` of github.com/MeteoraAg/damm-v2 (1,112 bytes, Anchor
//!   discriminator `sha256("account:Pool")[..8]`). The pool keeps no oracle:
//!   DAMM v2 has none, and this pool's dynamic fee (whose reference price
//!   would be the nearest thing) is off. So the spot price is the sample, and
//!   the average is the stake pool's own (`StakePool::post`, `twap_window`).
//! - SOL/USD: Pyth's sponsored SOL/USD feed on Solana, a `PriceUpdateV2`
//!   account of the Pyth Solana Receiver (`PYTH_RECEIVER`) at `SOL_USD_FEED`
//!   (feed id `SOL_USD_FEED_ID`, shard 0 of the Pyth push oracle). Layout:
//!   `pyth-solana-receiver-sdk` `price_update.rs` of github.com/pyth-network/
//!   pyth-crosschain (134 bytes allocated). The program uses its `ema_price`,
//!   Pyth's own exponentially weighted average (about an hour), and only an
//!   update with `VerificationLevel::Full` (every Wormhole guardian
//!   signature checked).
//!
//! A sample is refused unless the transaction holds nothing but this
//! instruction and compute-budget instructions, so a swap in the same
//! transaction can't move it, and the stake pool's average moves toward a
//! sample only over `twap_window`.

use crate::{state::StakePool, FuturesError};
use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, pubkey, pubkey::Pubkey,
    sysvar::instructions as ixs_sysvar,
};

/// Meteora DAMM v2 (CP-AMM).
pub const DAMM_V2_PROGRAM: Pubkey = pubkey!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
/// The Meteora AMBA/SOL pool (the user, 2026-10-18).
pub const AMBA_SOL_POOL: Pubkey = pubkey!("DxUB19khkvd3FehY8sovQ1FTTcEktPL4CM6yc49coWQH");
/// Mainnet AMBA (`ops::rewards::AMBA_MAINNET`).
pub const AMBA_MINT: Pubkey = pubkey!("BxKJN4PKAT2CFhVbsyKzVyCznu5a2n2oVh8Pb2wdDjam");
pub const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
/// The Pyth Solana Receiver, owner of every `PriceUpdateV2`.
pub const PYTH_RECEIVER: Pubkey = pubkey!("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");
/// Pyth's sponsored SOL/USD price feed account (shard 0).
pub const SOL_USD_FEED: Pubkey = pubkey!("7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE");
/// Pyth's SOL/USD feed id,
/// `ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d`.
pub const SOL_USD_FEED_ID: [u8; 32] = [
    239, 13, 139, 111, 218, 44, 235, 164, 29, 161, 93, 64, 149, 209, 218, 57, 42, 13, 47, 142, 208,
    198, 199, 188, 15, 76, 250, 200, 194, 128, 181, 109,
];
/// Wrapped SOL's decimals.
pub const SOL_DECIMALS: u32 = 9;
/// USDC's decimals (the stake pool's prices are in USDC atoms).
const USDC_DECIMALS: i64 = 6;
/// `PRICE_SCALE` = 10^6.
const PRICE_SCALE_DIGITS: i64 = 6;
const _: () = assert!(crate::state::PRICE_SCALE == 1_000_000);
/// The oldest Pyth update a sample takes, in seconds (a security check, not
/// a policy: a stalled feed must not price credit). The sponsored feed
/// updated every 50 to 210 seconds when sampled on 2026-10-18, and the EMA
/// it carries averages about an hour, so five minutes costs no accuracy.
pub const PYTH_MAX_AGE: u64 = 300;

/// `sha256("account:Pool")[..8]`.
pub const DAMM_POOL_DISCRIMINATOR: [u8; 8] = [0xf1, 0x9a, 0x6d, 0x04, 0x11, 0xb1, 0x6d, 0xbc];
pub const DAMM_POOL_LEN: usize = 1_112;
/// Byte offsets in a DAMM v2 `Pool` (8-byte discriminator, then
/// `PoolFeesStruct`, 160 bytes, then the fields below).
pub const DAMM_TOKEN_A_MINT: usize = 168;
pub const DAMM_TOKEN_B_MINT: usize = 200;
pub const DAMM_LIQUIDITY: usize = 360;
pub const DAMM_SQRT_PRICE: usize = 456;
pub const DAMM_ACTIVATION_POINT: usize = 472;
pub const DAMM_ACTIVATION_TYPE: usize = 480;
pub const DAMM_POOL_STATUS: usize = 481;

/// `sha256("account:PriceUpdateV2")[..8]`.
pub const PRICE_UPDATE_DISCRIMINATOR: [u8; 8] = [0x22, 0xf1, 0x23, 0x63, 0x9d, 0x7e, 0xf4, 0xcd];
pub const PRICE_UPDATE_LEN: usize = 134;
/// Byte offsets in a `PriceUpdateV2`: discriminator, `write_authority`
/// (32), then the Borsh `VerificationLevel` (tag 1 `Full`, one byte; tag 0
/// `Partial { num_signatures: u8 }`, two), then the `PriceFeedMessage`. The
/// message offsets below are for `Full`, the only level taken.
pub const PYTH_VERIFICATION: usize = 40;
pub const PYTH_FULL: u8 = 1;
pub const PYTH_FEED_ID: usize = 41;
pub const PYTH_EXPONENT: usize = 89;
pub const PYTH_PUBLISH_TIME: usize = 93;
pub const PYTH_EMA_PRICE: usize = 109;

/// What the program reads from the DAMM v2 pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DammPool {
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub activation_point: u64,
    /// 0: by slot, 1: by timestamp.
    pub activation_type: u8,
    /// 0: trading, 1: disabled.
    pub pool_status: u8,
}

/// What the program reads from a Pyth `PriceUpdateV2` (`Full` only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PythPrice {
    pub feed_id: [u8; 32],
    pub exponent: i32,
    pub publish_time: i64,
    pub ema_price: i64,
}

fn u128_at(b: &[u8], o: usize) -> u128 {
    u128::from_le_bytes(b[o..o + 16].try_into().unwrap())
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn i64_at(b: &[u8], o: usize) -> i64 {
    i64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn key_at(b: &[u8], o: usize) -> Pubkey {
    Pubkey::new_from_array(b[o..o + 32].try_into().unwrap())
}

/// A DAMM v2 `Pool`'s data, or `None` if it is not one (length or
/// discriminator).
pub fn read_damm_pool(b: &[u8]) -> Option<DammPool> {
    if b.len() != DAMM_POOL_LEN || b[..8] != DAMM_POOL_DISCRIMINATOR {
        return None;
    }
    Some(DammPool {
        token_a_mint: key_at(b, DAMM_TOKEN_A_MINT),
        token_b_mint: key_at(b, DAMM_TOKEN_B_MINT),
        liquidity: u128_at(b, DAMM_LIQUIDITY),
        sqrt_price: u128_at(b, DAMM_SQRT_PRICE),
        activation_point: u64_at(b, DAMM_ACTIVATION_POINT),
        activation_type: b[DAMM_ACTIVATION_TYPE],
        pool_status: b[DAMM_POOL_STATUS],
    })
}

/// A fully verified `PriceUpdateV2`'s data, or `None` if it is not one or
/// was only partly verified.
pub fn read_pyth(b: &[u8]) -> Option<PythPrice> {
    if b.len() != PRICE_UPDATE_LEN
        || b[..8] != PRICE_UPDATE_DISCRIMINATOR
        || b[PYTH_VERIFICATION] != PYTH_FULL
    {
        return None;
    }
    Some(PythPrice {
        feed_id: b[PYTH_FEED_ID..PYTH_FEED_ID + 32].try_into().unwrap(),
        exponent: i32::from_le_bytes(b[PYTH_EXPONENT..PYTH_EXPONENT + 4].try_into().unwrap()),
        publish_time: i64_at(b, PYTH_PUBLISH_TIME),
        ema_price: i64_at(b, PYTH_EMA_PRICE),
    })
}

/// `a * b / d` rounded down, through a 256-bit product; `None` if `d` is 0
/// or the quotient overflows.
pub fn mul_div(a: u128, b: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    const M: u128 = u64::MAX as u128;
    let (a1, a0) = (a >> 64, a & M);
    let (b1, b0) = (b >> 64, b & M);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 & M) + (p10 & M);
    let lo = (p00 & M) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    if hi >= d {
        return None;
    }
    // Long division of (hi, lo) by d; the remainder stays below d.
    let (mut r, mut q) = (hi, 0u128);
    for i in (0..128).rev() {
        let carry = r >> 127;
        r = (r << 1) | ((lo >> i) & 1);
        q <<= 1;
        if carry == 1 || r >= d {
            r = r.wrapping_sub(d);
            q |= 1;
        }
    }
    Some(q)
}

/// The AMBA price in the stake pool's units (USDC atoms per whole AMBA,
/// times `PRICE_SCALE` = 10^6; USDC has 6 decimals) from the pool's
/// `sqrt_price` (Q64.64, square root of lamports per AMBA atom) and SOL/USD
/// `sol_usd * 10^exponent`. Rounded down; `None` on overflow or a zero
/// result.
///
/// price = sqrt_price^2 / 2^128 [lamports per atom] * 10^amba_decimals
/// / 10^9 [SOL per AMBA] * sol_usd * 10^exponent [USD] * 10^6 [USDC atoms]
/// * 10^6 [PRICE_SCALE] = sqrt_price^2 * sol_usd * 10^(amba_decimals + 3
/// + exponent) / 2^128.
pub fn amba_price(sqrt_price: u128, amba_decimals: u8, sol_usd: i64, exponent: i32) -> Option<u64> {
    let sol_usd = u128::try_from(sol_usd).ok().filter(|&p| p > 0)?;
    let e = i64::from(amba_decimals) - i64::from(SOL_DECIMALS)
        + USDC_DECIMALS
        + PRICE_SCALE_DIGITS
        + i64::from(exponent);
    // 10^19 is the largest power of ten in a u64, so `sol_usd * 10^e` fits a
    // u128; beyond it the price is not representable anyway.
    if !(-19..=19).contains(&e) {
        return None;
    }
    let pow = 10u128.pow(e.unsigned_abs() as u32);
    let q64 = 1u128 << 64;
    // Lamports per atom, Q64.64.
    let per_atom = mul_div(sqrt_price, sqrt_price, q64)?;
    let v = if e >= 0 {
        mul_div(per_atom, sol_usd * pow, q64)?
    } else {
        mul_div(per_atom, sol_usd, q64)? / pow
    };
    u64::try_from(v).ok().filter(|&p| p > 0)
}

fn bad() -> ProgramError {
    FuturesError::InvalidAccount.into()
}

/// Refuses unless the transaction holds only this instruction (at the top
/// level, not through another program) and compute-budget instructions.
pub fn need_alone(program_id: &Pubkey, ixs: &AccountInfo) -> Result<(), ProgramError> {
    let current = ixs_sysvar::load_current_index_checked(ixs)?;
    let count = {
        let data = ixs.try_borrow_data()?;
        u16::from_le_bytes(data.get(..2).ok_or_else(bad)?.try_into().unwrap())
    };
    for i in 0..count {
        let ix = ixs_sysvar::load_instruction_at_checked(usize::from(i), ixs)?;
        let fine = if i == current {
            ix.program_id == *program_id
        } else {
            ix.program_id == solana_sdk_ids::compute_budget::ID
        };
        if !fine {
            return Err(FuturesError::InvalidInstruction.into());
        }
    }
    Ok(())
}

/// Reads and checks both sources and returns the AMBA price for `p`
/// (`amba_price`). `now` and `slot` are the clock's.
pub fn sample(
    p: &StakePool,
    amm: &AccountInfo,
    feed: &AccountInfo,
    now: u64,
    slot: u64,
) -> Result<u64, ProgramError> {
    if *amm.key != AMBA_SOL_POOL || *amm.owner != DAMM_V2_PROGRAM {
        return Err(bad());
    }
    if *feed.key != SOL_USD_FEED || *feed.owner != PYTH_RECEIVER {
        return Err(bad());
    }
    let pool = read_damm_pool(&amm.try_borrow_data()?).ok_or_else(bad)?;
    if pool.token_a_mint != AMBA_MINT || p.amba_mint != AMBA_MINT || pool.token_b_mint != WSOL_MINT
    {
        return Err(bad());
    }
    // A pool that can't trade (disabled, not yet active or empty) has no
    // price that arbitrage keeps honest.
    let active = match pool.activation_type {
        0 => slot >= pool.activation_point,
        1 => now >= pool.activation_point,
        _ => false,
    };
    if pool.pool_status != 0 || !active || pool.liquidity == 0 {
        return Err(FuturesError::InvalidPrice.into());
    }
    // Owner, address and `Full` verification: only the receiver writes it,
    // and only after checking every guardian signature.
    let pyth = read_pyth(&feed.try_borrow_data()?).ok_or(FuturesError::InvalidPrice)?;
    if pyth.feed_id != SOL_USD_FEED_ID {
        return Err(bad());
    }
    // Pythnet's clock may run a few seconds ahead of the cluster's: an
    // update from the future is fresh.
    let age = i128::from(now) - i128::from(pyth.publish_time);
    if age > i128::from(PYTH_MAX_AGE) {
        return Err(FuturesError::InvalidPrice.into());
    }
    amba_price(
        pool.sqrt_price,
        p.amba_decimals,
        pyth.ema_price,
        pyth.exponent,
    )
    .ok_or_else(|| FuturesError::InvalidPrice.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PRICE_SCALE;

    #[test]
    fn mul_div_matches_u128_and_wide_products() {
        assert_eq!(mul_div(6, 7, 4), Some(10));
        assert_eq!(mul_div(0, u128::MAX, 1), Some(0));
        assert_eq!(mul_div(u128::MAX, u128::MAX, u128::MAX), Some(u128::MAX));
        assert_eq!(mul_div(u128::MAX, 2, 4), Some(u128::MAX / 2));
        // 2^127 * 2^64 / 2^65 = 2^126.
        assert_eq!(mul_div(1 << 127, 1 << 64, 1 << 65), Some(1 << 126));
        assert_eq!(mul_div(u128::MAX, 2, 1), None);
        assert_eq!(mul_div(1, 1, 0), None);
        for (a, b, d) in [
            (123_456_789u128, 987_654_321u128, 1_000_003u128),
            (1 << 70, 3, 7),
        ] {
            assert_eq!(mul_div(a, b, d), Some(a * b / d));
        }
    }

    #[test]
    fn one_sol_an_amba_at_a_hundred_dollars() {
        // sqrt_price 2^64: one lamport per atom, so one SOL per AMBA.
        let p = amba_price(1 << 64, 9, 100_00000000, -8).unwrap();
        assert_eq!(u128::from(p), 100 * 1_000_000 * PRICE_SCALE);
        // A quarter: sqrt 2^63.
        let p = amba_price(1 << 63, 9, 100_00000000, -8).unwrap();
        assert_eq!(u128::from(p), 25 * 1_000_000 * PRICE_SCALE);
        // A small Pyth exponent takes the division path.
        let p = amba_price(1 << 64, 9, 100 * 10i64.pow(16), -16).unwrap();
        assert_eq!(u128::from(p), 100 * 1_000_000 * PRICE_SCALE);
    }

    #[test]
    fn the_mainnet_snapshot_prices_amba_near_dexscreener() {
        // DxUB19kh... sqrt_price and Pyth SOL/USD's EMA, 2026-10-18 (the
        // integration fixtures): 1.7744e-6 SOL an AMBA at $103.706.
        let p = amba_price(24_572_015_119_900_627, 9, 10_370_606_330, -8).unwrap();
        let sp = 24_572_015_119_900_627f64 / 2f64.powi(64);
        let want = sp * sp * 103.706_063_30 * 1e12;
        assert!(
            (p as f64 - want).abs() <= 1.0 + want * 1e-12,
            "{p} vs {want}"
        );
        // DexScreener showed $0.0001819-0.000184.
        assert!((180_000_000..=186_000_000).contains(&p), "{p}");
    }

    #[test]
    fn unrepresentable_or_zero_prices_are_refused() {
        assert_eq!(amba_price(1 << 64, 9, 0, -8), None);
        assert_eq!(amba_price(1 << 64, 9, -5, -8), None);
        assert_eq!(amba_price(0, 9, 100, -8), None);
        assert_eq!(amba_price(1 << 64, 9, 100, -40), None);
        assert_eq!(amba_price(u128::MAX, 9, i64::MAX, 0), None);
        // A price below one unit rounds to zero and is refused.
        assert_eq!(amba_price(1 << 20, 9, 1, -8), None);
    }

    #[test]
    fn layouts_reject_other_accounts() {
        let mut b = vec![0u8; DAMM_POOL_LEN];
        assert_eq!(read_damm_pool(&b), None);
        b[..8].copy_from_slice(&DAMM_POOL_DISCRIMINATOR);
        b[DAMM_SQRT_PRICE..DAMM_SQRT_PRICE + 16].copy_from_slice(&7u128.to_le_bytes());
        assert_eq!(read_damm_pool(&b).unwrap().sqrt_price, 7);
        assert_eq!(read_damm_pool(&b[..DAMM_POOL_LEN - 1]), None);
        let mut u = vec![0u8; PRICE_UPDATE_LEN];
        u[..8].copy_from_slice(&PRICE_UPDATE_DISCRIMINATOR);
        assert_eq!(read_pyth(&u), None, "partial verification");
        u[PYTH_VERIFICATION] = PYTH_FULL;
        u[PYTH_EMA_PRICE..PYTH_EMA_PRICE + 8].copy_from_slice(&5i64.to_le_bytes());
        assert_eq!(read_pyth(&u).unwrap().ema_price, 5);
    }
}
