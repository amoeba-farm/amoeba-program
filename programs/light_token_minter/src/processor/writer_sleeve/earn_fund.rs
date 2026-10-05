//! Earn Fund (`ManageEarnFundV1`, tag 12), fund layout version 4.
//!
//! One shared USDC fund contributes into any number of writer sleeves as
//! their pooled writer and collects back at settlement. Users only hold fund
//! shares. The share ledger lives in `crate::earn_fund_math`; this module
//! authenticates accounts, moves tokens with exact before/after balances and
//! stores state.
//!
//! Each sleeve the fund holds has one `EarnFundSlotV1`; the fund keeps their
//! O(1) aggregate (`SlotBook`), maintained by Allocate, Collect and the
//! permissionless slot valuation crank, so Deposit, Withdraw and Roll read a
//! fixed set of accounts however many slots are open. With nothing open every
//! price is exact (lean). While invested, the aggregate prices both sides
//! when it is usable: instant share exits and roll fills at the lower
//! (ask-side) NAV, immediate deposit conversions and roll conversions at the
//! upper (bid-side) NAV; otherwise exits queue, deposits stay pending and the
//! roll waits. Pending deposits always return at par. Layouts are in
//! `docs/earn-fund/EARN_FUND_V2.md`.
use super::participation::{
    claim_commit, claim_create_custody, claim_prepare, contribute_core, ClaimAccounts,
    ContributeAccounts,
};
use super::*;
use crate::buyback_mark_math::{BuybackParams, ExitBucket};
use crate::earn_fund_math::{
    mul_div_floor, FundLedger, FundMathError, FundParams, PositionLedger, QueueState, RollOutcome,
    BPS_DENOMINATOR, MAX_COMPLETED_BATCHES,
};
use crate::earn_fund_state::*;

pub(super) mod accounts;
mod buyback;
use accounts::*;

/// Decode the payload straight into the handler requests with the fixed
/// cursor readers of `earn_fund_state::wire` (the same bytes the Borsh codec
/// defines; `EarnFundActionV1::decode_wire` is the tested reference), then
/// dispatch. Any byte string the codec rejects fails here with
/// `InvalidInstructionData` before an account is read.
pub(in crate::processor) fn process(
    program: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    use crate::earn_fund_state::wire;
    let mut cursor = crate::fixed_codec::CheckedCursor::new(payload);
    let c = &mut cursor;
    let selector = c.u8();
    macro_rules! decoded {
        ($value:expr) => {{
            let value = $value;
            cursor
                .finish_exact()
                .map_err(|_| VaultError::InvalidInstructionData)?;
            value
        }};
    }
    match selector {
        1 | 2 => {
            let params = decoded!(wire::params(c));
            if selector == 1 {
                initialize(program, accounts, &params)
            } else {
                configure(program, accounts, &params)
            }
        }
        3 => {
            let request = decoded!(DepositRequest {
                amount_atoms: c.u64(),
                mode: wire::mode(c),
                min_shares: c.u64(),
                source: wire::source(c),
                position: wire::witness(c),
                proof: wire::proof(c),
            });
            deposit(program, accounts, request)
        }
        4 => {
            let request = decoded!(WithdrawRequest {
                pending_atoms: c.u64(),
                shares: c.u64(),
                min_instant_atoms: c.u64(),
                mode: wire::mode(c),
                position: wire::witness(c),
                proof: wire::proof(c),
            });
            withdraw(program, accounts, request)
        }
        5 => {
            let (position, proof) = decoded!((wire::witness(c), wire::proof(c)));
            complete_withdrawal(program, accounts, position, proof)
        }
        6 => {
            let amount = decoded!(c.u64());
            allocate(program, accounts, amount)
        }
        7 => {
            let cash = decoded!(CashWitness {
                amount: c.u64(),
                leaf_index: c.u32(),
                root_index: c.u16(),
                prove_by_index: c.boolean(),
                proof: wire::proof(c),
            });
            collect(program, accounts, cash)
        }
        8 => {
            decoded!(());
            roll(program, accounts)
        }
        9 => {
            let op = decoded!(wire::op(c));
            buyback::process(program, accounts, op)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

fn ledger_of(fund: &EarnFundV1) -> Result<FundLedger, ProgramError> {
    fund.ledger()
        .ok_or_else(|| VaultError::EarnFundAccounting.into())
}

fn clock_now() -> Result<u64, ProgramError> {
    u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| VaultError::EarnFundNotReady.into())
}

fn buyback_of(fund: &EarnFundV1) -> BuybackParams {
    BuybackParams::decode(&fund.buyback_params).unwrap_or(BuybackParams::INVALID)
}

/// The open slots' `(lower, upper)` value when they may price an entry or an
/// exit now: `(0, 0)` with nothing open (the exact NAV); with open slots, the
/// slot aggregate when it is usable (every live slot priced, none stale,
/// every value within `max_head_age`), the buy-back mode enabled and the
/// vault (`config`, read only then) unpaused; otherwise `None`.
fn open_values(
    program: &Pubkey,
    fund: &EarnFundV1,
    config: &AccountInfo,
    now: u64,
) -> Result<Option<(u64, u64)>, ProgramError> {
    if fund.book.slots == 0 {
        return Ok(Some((0, 0)));
    }
    let params = buyback_of(fund);
    Ok(
        if params.enabled && !load_canonical_vault_config(program, config)?.paused {
            fund.book.usable(now, u64::from(params.max_head_age_secs))
        } else {
            None
        },
    )
}

/// admin, config, fund, fund vault, USDC mint, system, SPL Token.
fn initialize(program: &Pubkey, a: &[AccountInfo], params: &FundParams) -> ProgramResult {
    if a.len() != 7
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[2].is_writable
        || !a[3].is_writable
        || *a[5].key != system_program::id()
        || *a[6].key != spl_token_program_id()
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    if config.admin != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    if !params.is_valid() {
        return Err(VaultError::EarnFundInvalidParams.into());
    }
    let (fund_key, bump) = derive_earn_fund(program);
    let (vault_key, vault_bump) = derive_earn_fund_vault(program, &fund_key);
    if *a[2].key != fund_key || *a[3].key != vault_key || config.usdc_mint != *a[4].key {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    validate_collateral_mint_account(&a[4], a[6].key)?;
    validate_create_only_program_account_target(program, &a[2])?;
    validate_create_only_program_account_target(program, &a[3])?;
    create_program_account(
        &a[0],
        &a[2],
        &a[5],
        program,
        EarnFundV1::LEN,
        &[EARN_FUND_SEED, &[bump]],
    )?;
    create_classic_token_pda(
        program,
        &a[0],
        &a[3],
        &a[4],
        &fund_key,
        &a[6],
        &a[5],
        &[EARN_FUND_VAULT_SEED, fund_key.as_ref(), &[vault_bump]],
    )?;
    validate_vault_token_account(&a[3], a[4].key, &fund_key)?;
    let now = clock_now()?;
    // The new account is all zero: decode it as the empty fund and fill it in
    // (cheaper on SBF than building the whole default struct inline).
    let mut fund = Box::new(load_exact_zero_padded_state::<EarnFundV1>(
        &a[2],
        program,
        EarnFundV1::LEN,
        VaultError::EarnFundInvalidAccount,
    )?);
    fund.initialized = true;
    fund.bump = bump;
    fund.discriminator = EARN_FUND_DISCRIMINATOR;
    fund.version = EARN_FUND_VERSION;
    fund.vault_config = *a[1].key;
    fund.usdc_mint = *a[4].key;
    fund.usdc_vault = vault_key;
    fund.usdc_vault_bump = vault_bump;
    fund.set_ledger(&FundLedger::new(now));
    fund.set_params(params);
    store_state(&a[2], fund.as_ref())
}

/// admin, config, fund. Parameters only; no bucket, share or slot changes.
fn configure(program: &Pubkey, a: &[AccountInfo], params: &FundParams) -> ProgramResult {
    if a.len() != 3 || !a[0].is_signer || !a[2].is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let config = load_canonical_vault_config(program, &a[1])?;
    let mut fund = load_fund(program, &a[2])?;
    if config.admin != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    if fund.vault_config != *a[1].key {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    if !params.is_valid() {
        return Err(VaultError::EarnFundInvalidParams.into());
    }
    fund.set_params(params);
    store_state(&a[2], fund.as_ref())
}

/// The five cash-rail accounts every position instruction carries: USDC
/// mint, SPL Token, Light Token, Light Token CPI authority, SPL interface.
const RAIL_ACCOUNTS: usize = 5;

struct DepositRequest {
    amount_atoms: u64,
    mode: EarnFundDepositMode,
    min_shares: u64,
    source: EarnFundDepositSource,
    position: EarnFundPositionWitness,
    proof: Option<[u8; 128]>,
}

const DEPOSIT_FIXED_ACCOUNTS: usize = 7 + RAIL_ACCOUNTS;

/// payer (transaction and Light fees; may differ from the owner), owner,
/// fund, fund vault, owner classic USDC (or system for a compressed source
/// or a pure conversion), pending record (or system), queue record (or
/// system), the five rail accounts; then — unless the mode is pending-only —
/// the vault config; then the Light block (position and compressed cash). Gas
/// sponsorship lives outside the program: no USDC ever moves to the payer.
///
/// The deposit joins the position's pending money (par, refundable). Unless
/// the mode is pending-only, the position's whole pending money then converts
/// to shares now at the upper NAV (`convert_now`); when it cannot,
/// ConvertElsePending leaves it pending for the next roll and ConvertOnly
/// fails. `amount = 0` (classic source, no account) only converts.
fn deposit(program: &Pubkey, a: &[AccountInfo], request: DepositRequest) -> ProgramResult {
    let amount = request.amount_atoms;
    let converts = request.mode != EarnFundDepositMode::QueueOnly;
    let light = DEPOSIT_FIXED_ACCOUNTS + usize::from(converts);
    let classic = request.source == EarnFundDepositSource::Classic;
    if a.len() < light
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_signer
        || !a[2].is_writable
        || ((!classic || amount == 0) && *a[4].key != system_program::id())
        || (amount == 0 && !classic)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if amount == 0 && !converts {
        return Err(VaultError::EarnFundInvalidAmount.into());
    }
    let mut fund = load_fund(program, &a[2])?;
    if fund.paused {
        return Err(VaultError::EarnFundPaused.into());
    }
    let rails = Rails::new(
        &a[0],
        &a[7..DEPOSIT_FIXED_ACCOUNTS],
        &a[light..],
        &fund.usdc_mint,
    )?;
    fund_vault_balance(&a[2], &fund, &a[3])?;
    let mut ledger = ledger_of(&fund)?;
    let mut session = open_position(program, a[2].key, a[1].key, &request.position, true)?;
    settle_position(
        program,
        a[2].key,
        &mut ledger,
        &mut session.position,
        &a[5],
        &a[6],
    )?;
    let mut owed = session.position.ledger();
    if amount != 0 {
        ledger.deposit(amount).map_err(fund_error)?;
        owed.add_pending(amount, ledger.epoch).map_err(fund_error)?;
    }
    if converts {
        if owed.pending_atoms == 0 {
            return Err(VaultError::EarnFundInvalidAmount.into());
        }
        if !convert_now(
            program,
            &a[DEPOSIT_FIXED_ACCOUNTS],
            &mut fund,
            &mut ledger,
            &mut owed,
            request.min_shares,
        )? && (request.mode == EarnFundDepositMode::InstantOnly || amount == 0)
        {
            return Err(VaultError::EarnFundInstantUnavailable.into());
        }
    }
    session.position.set_ledger(&owed);
    match request.source {
        EarnFundDepositSource::Classic => {
            if amount != 0 {
                usdc_account(&a[4], &fund.usdc_mint, Some(a[1].key))?;
                pay_from_owner(rails.spl_token, &a[4], rails.mint, &a[3], &a[1], amount)?;
            }
        }
        EarnFundDepositSource::Compressed {
            amount: leaf_amount,
            leaf_index,
            root_index,
            prove_by_index,
            tree_index,
            queue_index,
            proof,
        } => deposit_compressed(
            &rails,
            &a[3],
            &a[1],
            CashInput {
                amount: leaf_amount,
                leaf_index,
                root_index,
                prove_by_index,
                tree_index,
                queue_index,
                output_queue_index: session.output_queue_index(),
                proof,
            },
            amount,
        )?,
    }
    fund.set_ledger(&ledger);
    store_state(&a[2], fund.as_ref())?;
    require_backed(&fund, &a[3])?;
    commit_position(&a[0], rails.light, request.proof, session)
}

/// Convert the position's whole pending money to shares now at the upper
/// NAV `(free cash + Σ upper) / S` (exact with nothing open; par with no
/// shares and nothing open), when the open slots may price an entry
/// (`open_values`), at least `min_shares` result and — while invested — the
/// entry bucket (`entry_cap_bps` of the cap base per 24h) admits the atoms.
/// Otherwise `false` and nothing changes.
fn convert_now(
    program: &Pubkey,
    config: &AccountInfo,
    fund: &mut EarnFundV1,
    ledger: &mut FundLedger,
    owed: &mut PositionLedger,
    min_shares: u64,
) -> Result<bool, ProgramError> {
    let now = clock_now()?;
    let atoms = owed.pending_atoms;
    let Some(shares) = open_values(program, fund, config, now)?
        .and_then(|(_, upper)| ledger.entry_shares(atoms, upper).ok())
        .filter(|shares| *shares >= min_shares)
    else {
        return Ok(false);
    };
    if fund.book.slots != 0 {
        // Entries while invested also fill the entry bucket.
        let cap = mul_div_floor(
            ledger.cap_base_atoms,
            u64::from(buyback_of(fund).entry_cap_bps),
            BPS_DENOMINATOR,
        )
        .map_err(fund_error)?;
        let [updated_ts, level] = fund.entry_window;
        let Some(bucket) = ExitBucket { updated_ts, level }.admit(cap, now, atoms) else {
            return Ok(false);
        };
        fund.entry_window = [bucket.updated_ts, bucket.level];
    }
    ledger.apply_conversion(atoms, shares).map_err(fund_error)?;
    owed.take_pending(atoms).map_err(fund_error)?;
    owed.shares = owed
        .shares
        .checked_add(shares)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(true)
}

struct WithdrawRequest {
    pending_atoms: u64,
    shares: u64,
    min_instant_atoms: u64,
    mode: EarnFundWithdrawMode,
    position: EarnFundPositionWitness,
    proof: Option<[u8; 128]>,
}

const WITHDRAW_FIXED_ACCOUNTS: usize = 6 + RAIL_ACCOUNTS;

/// payer, owner, fund, fund vault, pending record (or system), queue record
/// (or system), the five rail accounts; then — when redeeming shares in a
/// mode that quotes — the vault config; then the Light block. All proceeds
/// are paid to the owner as compressed USDC; the payer only pays transaction
/// and Light fees. The account list never grows with the open slots.
///
/// Share pricing switches per withdrawal: with nothing invested, the exact
/// NAV (lean); while invested, the lower (ask-side) marked NAV `free cash +
/// Σ lower` when the open slots may price (`open_values`) and the marked-exit
/// bucket admits it; otherwise the shares queue for the next roll
/// (InstantOnly fails). The last share never exits instantly while a slot is
/// open.
fn withdraw(program: &Pubkey, a: &[AccountInfo], request: WithdrawRequest) -> ProgramResult {
    let quotes = request.shares != 0 && request.mode != EarnFundWithdrawMode::QueueOnly;
    let light = WITHDRAW_FIXED_ACCOUNTS + usize::from(quotes);
    if a.len() < light
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_signer
        || !a[2].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if request.pending_atoms == 0 && request.shares == 0 {
        return Err(VaultError::EarnFundInvalidAmount.into());
    }
    let mut fund = load_fund(program, &a[2])?;
    let rails = Rails::new(
        &a[0],
        &a[6..WITHDRAW_FIXED_ACCOUNTS],
        &a[light..],
        &fund.usdc_mint,
    )?;
    fund_vault_balance(&a[2], &fund, &a[3])?;
    let mut ledger = ledger_of(&fund)?;
    let mut session = open_position(program, a[2].key, a[1].key, &request.position, false)?;
    settle_position(
        program,
        a[2].key,
        &mut ledger,
        &mut session.position,
        &a[4],
        &a[5],
    )?;
    let mut owed = session.position.ledger();
    let mut proceeds = 0u64;
    if request.pending_atoms != 0 {
        owed.take_pending(request.pending_atoms)
            .map_err(fund_error)?;
        ledger
            .refund_pending(request.pending_atoms)
            .map_err(fund_error)?;
        proceeds = request.pending_atoms;
    }
    if request.shares != 0 {
        if request.shares > owed.shares {
            return Err(VaultError::EarnFundInvalidAmount.into());
        }
        let invested = fund.book.slots != 0;
        let mut marked_bucket = None;
        let instant = if !quotes {
            Err(VaultError::EarnFundInstantUnavailable)
        } else if fund.paused {
            Err(VaultError::EarnFundPaused)
        } else {
            let now = clock_now()?;
            // Lean: nothing invested, the exact NAV. Buy-back: the slot
            // aggregate's lower value, when usable. Otherwise no instant
            // price. A vault-wide pause stops marked exits at once.
            open_values(program, &fund, &a[WITHDRAW_FIXED_ACCOUNTS], now)?
                .ok_or(FundMathError::Invested)
                .and_then(|(lower, _)| ledger.nav_price(lower))
                .and_then(|mark| {
                    ledger.quote_instant(request.shares, mark, fund.instant_daily_cap_bps, now)
                })
                .and_then(|quote| {
                    if invested {
                        // Marked exits also fill the buy-back bucket.
                        let cap = mul_div_floor(
                            fund.cap_base_atoms,
                            u64::from(buyback_of(&fund).buyback_cap_bps),
                            BPS_DENOMINATOR,
                        )?;
                        marked_bucket = ExitBucket {
                            updated_ts: fund.marked_window_updated_ts,
                            level: fund.marked_window_level_atoms,
                        }
                        .admit(cap, now, quote.payout);
                        if marked_bucket.is_none() {
                            return Err(FundMathError::InstantCapExceeded);
                        }
                    }
                    Ok(quote)
                })
                .map_err(|error| match error {
                    FundMathError::Overflow => VaultError::ArithmeticOverflow,
                    _ => VaultError::EarnFundInstantUnavailable,
                })
                .and_then(|quote| {
                    if quote.payout < request.min_instant_atoms {
                        Err(VaultError::EarnFundInstantUnavailable)
                    } else {
                        Ok(quote)
                    }
                })
        };
        match instant {
            Ok(quote) => {
                owed.take_shares(request.shares).map_err(fund_error)?;
                ledger.apply_instant(quote).map_err(fund_error)?;
                if let Some(bucket) = marked_bucket {
                    fund.marked_window_updated_ts = bucket.updated_ts;
                    fund.marked_window_level_atoms = bucket.level;
                }
                proceeds = proceeds
                    .checked_add(quote.payout)
                    .ok_or(VaultError::ArithmeticOverflow)?;
            }
            Err(VaultError::ArithmeticOverflow) => {
                return Err(VaultError::ArithmeticOverflow.into())
            }
            Err(_) if request.mode != EarnFundWithdrawMode::InstantOnly => {
                owed.queue(request.shares, &mut ledger)
                    .map_err(fund_error)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    owed.record_paid(proceeds).map_err(fund_error)?;
    let queue = session.output_queue_index();
    pay_compressed(&rails, &a[3], &a[2], fund.bump, &a[1], queue, proceeds)?;
    session.position.set_ledger(&owed);
    fund.set_ledger(&ledger);
    store_state(&a[2], fund.as_ref())?;
    require_backed(&fund, &a[3])?;
    commit_position(&a[0], rails.light, request.proof, session)
}

const COMPLETE_FIXED_ACCOUNTS: usize = 6 + RAIL_ACCOUNTS;

/// Permissionless: payer, owner wallet, fund, fund vault, pending record (or
/// system), queue record (or system), the five rail accounts, Light block.
/// Settles the owner's position, then pays its claimable atoms to the owner
/// as compressed USDC.
fn complete_withdrawal(
    program: &Pubkey,
    a: &[AccountInfo],
    position: EarnFundPositionWitness,
    proof: Option<[u8; 128]>,
) -> ProgramResult {
    if a.len() < COMPLETE_FIXED_ACCOUNTS || !a[2].is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut fund = load_fund(program, &a[2])?;
    let rails = Rails::new(
        &a[0],
        &a[6..COMPLETE_FIXED_ACCOUNTS],
        &a[COMPLETE_FIXED_ACCOUNTS..],
        &fund.usdc_mint,
    )?;
    fund_vault_balance(&a[2], &fund, &a[3])?;
    let mut ledger = ledger_of(&fund)?;
    let mut session = open_position(program, a[2].key, a[1].key, &position, false)?;
    let before = session.position.ledger();
    settle_position(
        program,
        a[2].key,
        &mut ledger,
        &mut session.position,
        &a[4],
        &a[5],
    )?;
    let mut owed = session.position.ledger();
    if owed.claimable_atoms != 0 {
        let atoms = owed.complete(&mut ledger).map_err(fund_error)?;
        let queue = session.output_queue_index();
        pay_compressed(&rails, &a[3], &a[2], fund.bump, &a[1], queue, atoms)?;
    } else if owed == before {
        return Err(VaultError::EarnFundInvalidAmount.into());
    }
    session.position.set_ledger(&owed);
    fund.set_ledger(&ledger);
    store_state(&a[2], fund.as_ref())?;
    require_backed(&fund, &a[3])?;
    commit_position(&a[0], rails.light, proof, session)
}

/// allocator (signer), payer (receipt and slot rent), config, fund, fund
/// vault, sleeve, group, book, policy snapshot, DLMM policy, sleeve USDC
/// vault, WriterCash sidecar (or system), USDC mint, new receipt (nonce
/// `fund.book.next_lot_id`), the sleeve's fund slot (created on first entry),
/// system, SPL Token.
///
/// Any program-owned writer sleeve of any market (Spread's own loaders),
/// Funding or Active before expiry and inside the tenor window, regardless
/// of existing pooled ownership or exposure; at least `min_allocation`;
/// the minimum cash buffer stays live. A
/// top-up must extend the slot's contiguous capital-seconds range and leaves
/// the slot unpriced until it is valued again.
fn allocate(program: &Pubkey, a: &[AccountInfo], amount: u64) -> ProgramResult {
    if a.len() != 17
        || !a[0].is_signer
        || !a[1].is_signer
        || !a[1].is_writable
        || !a[3].is_writable
        || !a[14].is_writable
        || (*a[11].key != system_program::id() && !a[11].is_writable)
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut fund = load_fund(program, &a[3])?;
    if fund.allocator != *a[0].key {
        return Err(VaultError::Unauthorized.into());
    }
    if fund.paused {
        return Err(VaultError::EarnFundPaused.into());
    }
    if fund.vault_config != *a[2].key || fund.usdc_mint != *a[12].key {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    fund_vault_balance(&a[3], &fund, &a[4])?;
    let params = fund.params();
    let mut ledger = ledger_of(&fund)?;
    ledger.allocate(&params, amount).map_err(fund_error)?;
    let now = clock_now()?;
    // The sleeve's slot: the existing one, or a new one on first entry.
    let existing = a[14].owner == program;
    let mut slot = if existing {
        load_slot(program, &a[14], a[5].key)?
    } else {
        EarnFundSlotV1::default()
    };
    let bump = [fund.bump];
    let seeds = fund_signer_seeds(&bump);
    let lot = contribute_core(
        program,
        &ContributeAccounts {
            payer: &a[1],
            owner: &a[3],
            config: &a[2],
            sleeve: &a[5],
            group: &a[6],
            book: &a[7],
            snapshot: &a[8],
            policy: &a[9],
            writer_usdc: &a[10],
            cash_custody: &a[11],
            source_usdc: &a[4],
            mint: &a[12],
            receipt: &a[13],
            system: &a[15],
            token: &a[16],
        },
        fund.book.next_lot_id,
        amount,
        &params,
        &[&seeds],
    )?;
    let end = lot
        .interval()
        .weight()
        .ok()
        .and_then(|weight| lot.weight_offset.checked_add(weight))
        .ok_or(VaultError::EarnFundAccounting)?;
    if !params.tenor_admissible(now, lot.expiry_ts)
        || lot.principal != amount
        // A top-up continues the slot's range (every pooled entry since the
        // fund's first is the fund's own: user Contribute is retired).
        || (existing && lot.weight_offset != slot.end())
    {
        return Err(VaultError::EarnFundInvalidAllocation.into());
    }
    if existing {
        // The sleeve changed: the recorded value no longer applies.
        fund.book.invalidate(&mut slot.mark);
    } else {
        let (key, slot_bump) = derive_earn_fund_slot(program, a[5].key);
        if *a[14].key != key {
            return Err(VaultError::EarnFundInvalidAccount.into());
        }
        validate_create_only_program_account_target(program, &a[14])?;
        create_program_account(
            &a[1],
            &a[14],
            &a[15],
            program,
            EarnFundSlotV1::LEN,
            &[EARN_FUND_SLOT_SEED, a[5].key.as_ref(), &[slot_bump]],
        )?;
        slot = EarnFundSlotV1::new(slot_bump, *a[5].key);
        slot.set_range(lot.weight_offset, lot.weight_offset);
        slot.mark = fund.book.open(now);
    }
    slot.principal = slot
        .principal
        .checked_add(amount)
        .ok_or(VaultError::ArithmeticOverflow)?;
    slot.set_range(slot.start(), end);
    fund.book.next_lot_id = fund
        .book
        .next_lot_id
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    fund.set_ledger(&ledger);
    if fund.ledger() != Some(ledger) {
        return Err(VaultError::EarnFundAccounting.into());
    }
    slot.write(&mut a[14].try_borrow_mut_data()?);
    store_state(&a[3], fund.as_ref())?;
    require_backed(&fund, &a[4])
}

struct CashWitness {
    amount: u64,
    leaf_index: u32,
    root_index: u16,
    prove_by_index: bool,
    proof: Option<[u8; 128]>,
}

/// payer, fund, fund vault, sleeve, receipt, sleeve hot USDC vault, USDC mint,
/// config, WriterCash sidecar, SPL interface, Light Token, CPI authority, SPL
/// Token, system, Light System, registered program, compression authority,
/// compression program, output queue, cash input tree, cash input queue,
/// receipt rent recipient, the sleeve's fund slot. Zero cash uses system
/// sentinels at 19/20.
///
/// Only a receipt the fund created, and only the slot's oldest open lot (so
/// the slot's range stays contiguous). The rest of the slot is then exact
/// (settled): its claim value is recorded as final. The last lot closes the
/// slot and returns its rent to the receipt's rent payer.
fn collect(program: &Pubkey, a: &[AccountInfo], cash: CashWitness) -> ProgramResult {
    use crate::compressed_custody::{self as custody, CustodyKind};
    use crate::regular_compressed_transfer::{self as transfer, InputLeaf, OutputLeaf};
    use solana_program::instruction::AccountMeta;

    if a.len() != 23
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_writable
        || !a[2].is_writable
        || !a[3].is_writable
        || !a[4].is_writable
        || !a[5].is_writable
        || !a[8].is_writable
        || !a[9].is_writable
        || !a[18].is_writable
        || !a[21].is_writable
        || !a[22].is_writable
        || *a[10].key != light_token_program_id()
        || *a[11].key != cpi_authority()
        || *a[12].key != spl_token_program_id()
        || *a[13].key != system_program::id()
        || *a[14].key != Pubkey::new_from_array(light_sdk::constants::LIGHT_SYSTEM_PROGRAM_ID)
        || *a[15].key != Pubkey::new_from_array(light_sdk::constants::REGISTERED_PROGRAM_PDA)
        || *a[16].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_AUTHORITY_PDA)
        || *a[17].key
            != Pubkey::new_from_array(light_sdk::constants::ACCOUNT_COMPRESSION_PROGRAM_ID)
        || (cash.amount == 0
            && (cash.leaf_index != 0
                || cash.root_index != 0
                || cash.prove_by_index
                || cash.proof.is_some()
                || a[19].key != &system_program::id()
                || a[20].key != &system_program::id()))
        || (cash.amount != 0
            && (!a[19].is_writable
                || !a[20].is_writable
                || (cash.prove_by_index && cash.root_index != 0)
                || (!cash.prove_by_index && cash.proof.is_none())))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut fund = load_fund(program, &a[1])?;
    let mut slot = load_slot(program, &a[22], a[3].key)?;
    if fund.usdc_mint != *a[6].key {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    let vault_before = fund_vault_balance(&a[1], &fund, &a[2])?;
    let mut ledger = ledger_of(&fund)?;
    let accounts = ClaimAccounts {
        payer: &a[0],
        owner: a[1].key,
        sleeve: &a[3],
        receipt: &a[4],
        hot_vault: &a[5],
        mint: &a[6],
        config: &a[7],
        cash_custody: &a[8],
        spl_interface: &a[9],
        token_program: &a[12],
        system_program: &a[13],
    };
    let mut plan = claim_prepare(program, &accounts, cash.amount)?;
    // The receipt is the fund's own (created by Allocate: a receipt merely
    // transferred to the fund PDA is not part of its book) and the slot's
    // oldest open lot.
    let end = plan
        .lot
        .interval()
        .weight()
        .ok()
        .and_then(|weight| plan.lot.weight_offset.checked_add(weight))
        .ok_or(VaultError::EarnFundAccounting)?;
    if plan.lot.creator != *a[1].key
        || plan.lot.weight_offset != slot.start()
        || end > slot.end()
        || plan.lot.principal > slot.principal
    {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    if *a[21].key != plan.lot.rent_payer {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    let payout = plan.payout;
    let compressed_draw = payout.min(cash.amount);
    let hot_draw = payout - compressed_draw;
    if plan.hot_before < hot_draw {
        return Err(ProgramError::InsufficientFunds);
    }
    if cash.amount != 0 && compressed_draw == 0 {
        // A zero payout never needs to consume a WriterCash leaf.
        return Err(VaultError::EarnFundInvalidAmount.into());
    }
    claim_create_custody(program, &accounts, &plan)?;
    if compressed_draw != 0 {
        // Light's fixed seven metas, then output queue, input tree/queue,
        // USDC mint, canonical WriterCash signer, fund vault, SPL interface
        // pool and the SPL Token program for the pool transfer.
        let indices = [14, 0, 11, 15, 16, 17, 13, 18, 19, 20, 6, 8, 2, 9, 12];
        let metas = indices
            .iter()
            .enumerate()
            .map(|(i, &n)| AccountMeta {
                pubkey: *a[n].key,
                is_writable: matches!(i, 1 | 7 | 8 | 9 | 11 | 12 | 13),
                is_signer: i == 1 || i == 11,
            })
            .collect();
        let (_, interface_bump) = light_token_instruction::get_spl_interface_pda_and_bump(a[6].key);
        let change = cash.amount - compressed_draw;
        let outputs = [OutputLeaf {
            owner: 4,
            amount: change,
            has_delegate: false,
            delegate: 0,
            mint: 3,
        }];
        let ix = transfer::decompress_to_spl_instruction(
            *a[10].key,
            metas,
            0,
            cash.proof,
            &[InputLeaf {
                owner: 4,
                amount: cash.amount,
                has_delegate: false,
                delegate: 0,
                mint: 3,
                tree: 1,
                queue: 2,
                leaf_index: cash.leaf_index,
                prove_by_index: cash.prove_by_index,
                root_index: cash.root_index,
            }],
            transfer::SplDecompression {
                amount: compressed_draw,
                mint: 3,
                recipient: 5,
                pool_account_index: 6,
                pool_index: 0,
                bump: interface_bump,
                decimals: MarketMintAccounting::CANONICAL_DECIMALS,
            },
            &outputs[..usize::from(change != 0)],
        )?;
        let mut infos: Vec<_> = indices.iter().map(|&i| a[i].clone()).collect();
        infos.push(a[10].clone());
        let bump = [plan.cash_custody.bump];
        let kind = [CustodyKind::WriterCash as u8];
        let seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            custody::COMPRESSED_CUSTODY_SEED,
            &kind,
            a[5].key.as_ref(),
            &bump,
        ];
        invoke_signed(&ix, &infos, &[seeds])?;
    }
    if hot_draw != 0 {
        let bump = [plan.sleeve.bump];
        let sleeve_seeds = writer_sleeve_signer_seeds(&plan.sleeve.settlement_group, &bump);
        invoke_token_transfer_checked(
            &a[12],
            &a[5],
            &a[6],
            &a[2],
            &a[3],
            hot_draw,
            MarketMintAccounting::CANONICAL_DECIMALS,
            &[&sleeve_seeds],
        )?;
    }
    if validate_token_account(&a[2])?
        .amount
        .checked_sub(vault_before)
        != Some(payout)
    {
        return Err(VaultError::EarnFundAccounting.into());
    }
    let principal = plan.lot.principal;
    claim_commit(&accounts, &mut plan, compressed_draw, hot_draw)?;
    // The fund can never sign a receipt Close; return the keeper's rent here.
    close_program_account(program, &a[4], &a[21])?;
    ledger.collect(principal, payout).map_err(fund_error)?;
    let now = clock_now()?;
    slot.principal -= principal;
    slot.set_range(end, slot.end());
    if slot.principal == 0 {
        if end != slot.end() {
            return Err(VaultError::EarnFundAccounting.into());
        }
        fund.book.close(&slot.mark, now).map_err(fund_error)?;
        close_program_account(program, &a[22], &a[21])?;
    } else {
        // The remaining lots share the settled sleeve's exact claim inputs.
        let value = crate::writer_participation_math::range_payout(
            slot.principal,
            end,
            slot.end(),
            plan.sleeve.settlement_principal_atoms,
            plan.sleeve.capital_seconds,
            plan.sleeve.writer_residual_initial_atoms,
        )
        .map_err(|_| VaultError::EarnFundAccounting)?;
        fund.book
            .record(&mut slot.mark, value, value, now, true, now)
            .map_err(fund_error)?;
        slot.write(&mut a[22].try_borrow_mut_data()?);
    }
    fund.set_ledger(&ledger);
    if fund.ledger() != Some(ledger) {
        return Err(VaultError::EarnFundAccounting.into());
    }
    store_state(&a[1], fund.as_ref())?;
    require_backed(&fund, &a[2])
}

/// payer (epoch record rent), fund, new epoch record, system, then — while
/// any slot is open — the vault config. Permissionless once the epoch is
/// long enough. With nothing open the roll is exact (free cash per share).
/// With open slots it runs only while they may price (`open_values`): pending
/// deposits convert at the upper NAV and queued shares fill at the lower NAV
/// from free cash (pro rata when short, never the last share), both prices
/// taken before any conversion; otherwise it waits (`EarnFundNotReady`).
fn roll(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    if a.len() < 4
        || a.len() > 5
        || !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_writable
        || !a[2].is_writable
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut fund = load_fund(program, &a[1])?;
    if a.len() != 4 + usize::from(fund.book.slots != 0) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut ledger = ledger_of(&fund)?;
    let now = clock_now()?;
    // The config is the last account (read only while a slot is open).
    let (lower, upper) =
        open_values(program, &fund, &a[a.len() - 1], now)?.ok_or(VaultError::EarnFundNotReady)?;
    let outcome: RollOutcome = ledger
        .roll(&fund.params(), now, lower, upper)
        .map_err(fund_error)?;
    debug_assert!(usize::from(outcome.completed_count) <= MAX_COMPLETED_BATCHES);
    create_fund_epoch(program, &a[0], &a[2], &a[3], a[1].key, &outcome, now)?;
    fund.set_ledger(&ledger);
    store_state(&a[1], fund.as_ref())
}
