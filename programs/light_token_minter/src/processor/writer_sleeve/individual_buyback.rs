//! Atomic, portfolio-funded acquisitions from canonical individual asks.
//! The buyer never receives spendable cash before matching longs are locked.
use super::*;
use crate::individual_writer::{IndividualBuybackLegV1, IndividualBuybackV1, MAX_BUYBACK_LEGS};

const COMMON: usize = 25;
const LEG: usize = 7;

fn checked<T>(value: Option<T>) -> Result<T, ProgramError> {
    value.ok_or_else(|| VaultError::ArithmeticOverflow.into())
}

/// Reuse the exact issuer and listing checks of an ordinary primary fill.
/// The recipient is explicitly supplied to issue_to, with no external delegate.
pub(super) fn fill_accounts<'a>(a: &[AccountInfo<'a>], leg: usize) -> [AccountInfo<'a>; 32] {
    let n = COMMON + leg * LEG;
    [
        0,
        1,
        2,
        3,
        4,
        n,
        6,
        7,
        8,
        9,
        10,
        11,
        12,
        13,
        14,
        15,
        n + 2,
        16,
        n + 3,
        13,
        n + 4,
        n + 5,
        n + 6,
        17,
        18,
        19,
        20,
        21,
        22,
        23,
        24,
        n + 1,
    ]
    .map(|i| a[i].clone())
}

fn validate_accounts(a: &[AccountInfo], wire: &IndividualBuybackV1) -> ProgramResult {
    let count = usize::from(wire.leg_count);
    if count == 0
        || count > MAX_BUYBACK_LEGS
        || a.len() != COMMON + LEG * count
        || wire.maximum_payment == 0
        || wire.deadline_ts == 0
        || wire.legs[count..]
            .iter()
            .any(|leg| *leg != IndividualBuybackLegV1::default())
    {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let writable = |i: usize| [0, 4, 5, 6, 7, 9, 15, 24].contains(&i) || i >= COMMON;
    // Repeated maker portfolios or mints are legitimate across different legs.
    // AccountInfo privileges are the transaction-wide union for an address.
    for (i, info) in a.iter().enumerate() {
        let expected_writable = writable(i)
            || a.iter()
                .enumerate()
                .any(|(j, other)| crate::pubkey_eq(other.key, info.key) && writable(j));
        if info.is_signer != crate::pubkey_eq(info.key, a[0].key)
            || info.is_writable != expected_writable
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if i < COMMON
            && a[..i]
                .iter()
                .any(|other| crate::pubkey_eq(other.key, info.key))
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    for i in 0..count {
        let n = COMMON + i * LEG;
        if wire.legs[i].quantity == 0
            || (0..i).any(|j| crate::pubkey_eq(a[COMMON + j * LEG].key, a[n].key))
            || a[n + 1].key == a[5].key
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    super::buyback_issuance::validate_repeated_series_roles(a, wire)?;
    validate_writer_compression_accounts(&a[10], &a[11], &a[12], &a[13], &a[14], &a[15])
}

#[inline(never)]
pub(super) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    wire: IndividualBuybackV1,
) -> ProgramResult {
    validate_accounts(a, &wire)?;
    let now = current_unix_timestamp()?;
    if now > wire.deadline_ts {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    let WriterBookContext {
        sleeve,
        group,
        mut book,
    } = load_writer_book_context(program, &a[2], &a[3], &a[4])?;
    if !book.frozen
        || sleeve.vault_config != *a[1].key
        || group.sleeve != *a[2].key
        || sleeve.settlement_mint != *a[8].key
        || config.usdc_mint != *a[8].key
        || sleeve.status != WriterSleeveStatus::Active
        || group.status != WriterSettlementGroupStatus::Active
        || book.individual.funded
        || now >= group.expiry_ts
    {
        return Err(VaultError::InvalidWriterLifecycle.into());
    }
    validate_collateral_mint_account(&a[8], a[12].key)?;
    validate_spl_interface_account(a[8].key, &a[9])?;
    let buyer = individual::load_portfolio(program, &a[5], a[4].key, a[0].key, group.expiry_ts)?;
    let cash_before = load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount;
    if cash_before < book.individual.cash_obligations
        || buyer.cash_atoms > book.individual.cash_obligations
    {
        return Err(VaultError::WriterSolvencyViolation.into());
    }
    let series = writer_book_math_series(&book)?;
    let mut quantities = [0u64; 20];
    let mut payment = 0u64;
    let count = usize::from(wire.leg_count);
    // The public eight-ask bound also bounds temporary memory. Keep this small
    // validation buffer on the stack so it does not consume the CPI heap budget.
    let mut fills: [Option<(crate::individual_writer::IndividualWriterPosition, u64)>;
        MAX_BUYBACK_LEGS] = std::array::from_fn(|_| None);
    for i in 0..count {
        let view = fill_accounts(a, i);
        let leg = wire.legs[i];
        let mut ask = individual::load_position(program, &view, &book, &group)?;
        let index = usize::from(leg.series_index);
        if ask.owner == *a[0].key
            || ask.series_index != leg.series_index
            || index >= usize::from(book.series_count)
            || !book.records[index].active
        {
            return Err(VaultError::InvalidWriterSleeve.into());
        }
        // Repeated roles were authenticated before this read-only phase. No
        // market, oracle, or common context changes before all asks validate.
        if quantities[index] == 0 {
            individual::live_market(program, &view, &config, &sleeve, &group, &book, index)?;
        }
        let price = ask
            .fill(leg.quantity, wire.maximum_payment)
            .ok_or(VaultError::InvalidInstructionData)?;
        payment = checked(payment.checked_add(price))?;
        quantities[index] = checked(quantities[index].checked_add(leg.quantity))?;
        fills[i] = Some((ask, price));
    }
    // No partial execution: even a multi-leg straddle is funded against its
    // final combined risk before any seller or custody state is committed.
    let (mut buyer, refund) = buyer
        .bought_back(
            &series,
            &quantities,
            payment,
            wire.maximum_payment,
            wire.minimum_refund,
        )
        .map_err(writer_math_error)?;
    for (i, fill) in fills.into_iter().take(count).enumerate() {
        let (ask, price) = fill.ok_or(VaultError::InvalidInstructionData)?;
        let view = fill_accounts(a, i);
        let index = usize::from(wire.legs[i].series_index);
        // Reload after earlier legs: multiple asks may share this seller.
        let mut seller =
            individual::load_portfolio(program, &view[31], a[4].key, &ask.owner, group.expiry_ts)?;
        seller = seller
            .filled(index, wire.legs[i].quantity, price)
            .map_err(writer_math_error)?;
        if !seller.funding_registered {
            seller.funding_registered = true;
            book.individual.pending_portfolio_funding =
                checked(book.individual.pending_portfolio_funding.checked_add(1))?;
        }
        store_state(&view[31], &seller)?;
        store_state(&view[5], &ask)?;
    }
    super::buyback_issuance::issue(program, a, &wire, &sleeve, &group, &mut book, &quantities)?;
    if !buyer.funding_registered {
        buyer.funding_registered = true;
        book.individual.pending_portfolio_funding =
            checked(book.individual.pending_portfolio_funding.checked_add(1))?;
    }
    // The payment stays in the same canonical book cash account: seller cash
    // increases by exactly what buyer cash spends. Only the refund leaves it.
    book.individual.cash_obligations =
        checked(book.individual.cash_obligations.checked_sub(refund))?;
    if refund > 0 {
        let actor_before = load_or_create_light_associated_token_account(
            &a[0], &a[0], &a[8], &a[7], &a[10], &a[14], &a[15], &a[13],
        )?
        .amount;
        let bump = [book.bump];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::constants::WRITER_SERIES_BOOK_PDA_SEED,
            a[2].key.as_ref(),
            &bump,
        ];
        individual::transfer(a, refund, &a[6], &a[7], &a[4], &[seeds])?;
        if actor_before.checked_add(refund)
            != Some(load_canonical_light_token_account(&a[7], a[0].key, a[8].key)?.amount)
        {
            return Err(VaultError::WriterSupplyMismatch.into());
        }
    }
    let cash_after = load_canonical_light_token_account(&a[6], a[4].key, a[8].key)?.amount;
    if cash_before.checked_sub(refund) != Some(cash_after)
        || cash_after < book.individual.cash_obligations
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    store_state(&a[5], &buyer)?;
    individual::persist_book(a, &mut book)
}
