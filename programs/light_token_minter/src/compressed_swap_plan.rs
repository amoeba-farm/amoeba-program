//! Physical custody split for a quoted DLMM compressed-input swap.
//! Route pricing and obligations remain in the existing DLMM state.
use crate::ProgramError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    QuoteForOption,
    OptionForQuote,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plan {
    pub user_change: u64,
    pub input_to_pool: u64,
    pub input_to_writer_cash: u64,
    pub input_to_retirement: u64,
    pub output_from_hot: u64,
    pub output_from_compressed_pool: u64,
    pub pool_option_after: u64,
    pub pool_quote_after: u64,
    pub writer_quote_after: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LpWithdrawal {
    pub compressed_input: u64,
    pub hot_compression: u64,
    pub wallet_output: u64,
    pub compressed_change: u64,
    pub hot_after: u64,
}

/// Spend the existing Pool leaf whole and return its unused value to the same
/// custody PDA. Only the part of the withdrawal that exceeds compressed
/// inventory is compressed from the canonical hot pool vault.
pub fn plan_lp_withdrawal(
    hot_before: u64,
    compressed_before: u64,
    obligation_before: u64,
    withdrawal: u64,
) -> Result<LpWithdrawal, ProgramError> {
    let physical_before = hot_before
        .checked_add(compressed_before)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if physical_before < obligation_before || withdrawal > obligation_before {
        return Err(ProgramError::InsufficientFunds);
    }
    let hot_compression = withdrawal.saturating_sub(compressed_before);
    let compressed_change = compressed_before.saturating_sub(withdrawal);
    let hot_after = hot_before
        .checked_sub(hot_compression)
        .ok_or(ProgramError::InsufficientFunds)?;
    let obligation_after = obligation_before - withdrawal;
    if hot_after
        .checked_add(compressed_change)
        .filter(|physical| *physical >= obligation_after)
        .is_none()
    {
        return Err(ProgramError::InsufficientFunds);
    }
    Ok(LpWithdrawal {
        compressed_input: if withdrawal == 0 {
            0
        } else {
            compressed_before
        },
        hot_compression,
        wallet_output: withdrawal,
        compressed_change,
        hot_after,
    })
}

/// Preserve program-owned compressed surplus while moving exactly the change
/// in authenticated book obligations. The old sidecar must back old rights.
pub fn book_sidecar_target(
    old_sidecar: u64,
    old_obligations: u64,
    new_obligations: u64,
) -> Result<u64, ProgramError> {
    let surplus = old_sidecar
        .checked_sub(old_obligations)
        .ok_or(ProgramError::InsufficientFunds)?;
    new_obligations
        .checked_add(surplus)
        .ok_or(ProgramError::ArithmeticOverflow)
}

/// Solve the aggregate regular leaf left in Pool custody for one mint. Hot
/// inputs are compressed inside the same Transfer2; hot outputs are handled by
/// their separate authorized compression CPI and excluded here.
#[allow(clippy::too_many_arguments)]
pub fn pool_sidecar_target(
    user_compressed_input: u64,
    user_hot_compression: u64,
    hot_pool_compression: u64,
    book_before: u64,
    book_after: u64,
    pool_before: u64,
    writer_before: u64,
    writer_after: u64,
    user_change: u64,
    sponsor_fee: u64,
    direct_compressed_output: u64,
    retirement: u64,
) -> Result<u64, ProgramError> {
    let inputs = [
        user_compressed_input,
        user_hot_compression,
        hot_pool_compression,
        book_before,
        pool_before,
        writer_before,
    ]
    .into_iter()
    .try_fold(0u128, |sum, amount| sum.checked_add(u128::from(amount)))
    .ok_or(ProgramError::ArithmeticOverflow)?;
    let others = [
        book_after,
        writer_after,
        user_change,
        sponsor_fee,
        direct_compressed_output,
        retirement,
    ]
    .into_iter()
    .try_fold(0u128, |sum, amount| sum.checked_add(u128::from(amount)))
    .ok_or(ProgramError::ArithmeticOverflow)?;
    u64::try_from(
        inputs
            .checked_sub(others)
            .ok_or(ProgramError::InsufficientFunds)?,
    )
    .map_err(|_| ProgramError::ArithmeticOverflow)
}

#[allow(clippy::too_many_arguments)]
pub fn plan(
    direction: Direction,
    user_leaf_amount: u64,
    amount_in: u64,
    amount_out: u64,
    writer_premium: u64,
    writer_retirement: u64,
    hot_output_available: u64,
    pool_option_before: u64,
    pool_quote_before: u64,
    writer_quote_before: u64,
) -> Result<Plan, ProgramError> {
    if amount_in == 0
        || amount_out == 0
        || amount_in > user_leaf_amount
        || (direction == Direction::QuoteForOption && writer_retirement != 0)
        || (direction == Direction::OptionForQuote && writer_premium != 0)
    {
        return Err(ProgramError::InvalidInstructionData);
    }
    let writer_input = match direction {
        Direction::QuoteForOption => writer_premium,
        Direction::OptionForQuote => writer_retirement,
    };
    let input_to_pool = amount_in
        .checked_sub(writer_input)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let output_from_hot = amount_out.min(hot_output_available);
    let output_from_compressed_pool = amount_out - output_from_hot;
    let (pool_option_after, pool_quote_after) = match direction {
        Direction::QuoteForOption => (
            pool_option_before
                .checked_sub(output_from_compressed_pool)
                .ok_or(ProgramError::InsufficientFunds)?,
            pool_quote_before
                .checked_add(input_to_pool)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        ),
        Direction::OptionForQuote => (
            pool_option_before
                .checked_add(input_to_pool)
                .ok_or(ProgramError::ArithmeticOverflow)?,
            pool_quote_before
                .checked_sub(output_from_compressed_pool)
                .ok_or(ProgramError::InsufficientFunds)?,
        ),
    };
    let writer_quote_after = writer_quote_before
        .checked_add(writer_premium)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    Ok(Plan {
        user_change: user_leaf_amount - amount_in,
        input_to_pool,
        input_to_writer_cash: writer_premium,
        input_to_retirement: writer_retirement,
        output_from_hot,
        output_from_compressed_pool,
        pool_option_after,
        pool_quote_after,
        writer_quote_after,
    })
}
