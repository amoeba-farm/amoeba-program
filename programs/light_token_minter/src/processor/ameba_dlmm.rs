//! Native in-program additive-grid DLMM.

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::{clock::Clock, Sysvar},
};

use super::*;
use crate::{
    ameba_dlmm_instruction::{
        decode_exact, AddAmoebaDlmmLiquidityV1Params, AmoebaDlmmDecode, AmoebaDlmmInstructionTag,
        AmoebaDlmmSwapDirection as WireSwapDirection, CollectAmoebaDlmmProtocolFeesV1Params,
        InitializeAmoebaDlmmBinPageV1Params, InitializeAmoebaDlmmPoolV1Params,
        InitializeAmoebaDlmmPositionV1Params, RemoveAmoebaDlmmLiquidityV1Params,
        SetAmoebaDlmmPoolStatusV1Params, SwapAmoebaDlmmExactInV1Params,
    },
    ameba_dlmm_math::{
        bin_to_page, calculate_share_deposit, calculate_share_withdrawal, page_first_bin,
        price_from_bin, quote_exact_in, refresh_local_liquidity_bits, AmoebaDlmmBinLiquidity,
        AmoebaDlmmMathError, AmoebaDlmmSwapDirection,
    },
    ameba_dlmm_state::{
        derive_ameba_dlmm_authority_pda, derive_ameba_dlmm_bin_page_pda,
        derive_ameba_dlmm_pool_pda, derive_ameba_dlmm_position_pda,
        derive_ameba_dlmm_share_page_pda, derive_ameba_dlmm_vault_pda, AmoebaDlmmBinPageV1,
        AmoebaDlmmLightState, AmoebaDlmmPoolStatus, AmoebaDlmmPoolV1, AmoebaDlmmPositionV1,
        AmoebaDlmmSharePageV1, AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR, AMOEBA_DLMM_EMPTY_BIN_ID,
        AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR, AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR,
        AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR,
    },
    constants::{
        AMOEBA_DLMM_AUTHORITY_PDA_SEED, AMOEBA_DLMM_BINS_PER_PAGE, AMOEBA_DLMM_BIN_PAGE_PDA_SEED,
        AMOEBA_DLMM_POOL_PDA_SEED, AMOEBA_DLMM_POSITION_PDA_SEED, AMOEBA_DLMM_SHARE_PAGE_PDA_SEED,
        AMOEBA_DLMM_VAULT_PDA_SEED, MAX_AMOEBA_DLMM_BINS_PER_SWAP, MAX_AMOEBA_DLMM_BIN_COUNT,
        MAX_AMOEBA_DLMM_PAGE_COUNT, MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP,
        MAX_AMOEBA_DLMM_SWAP_FEE_BPS,
    },
    light_token_instruction::create_token_account_rent_free,
};

use super::ameba_dlmm_light::{initialize_compression_info, register_initialized_pdas};

const EVENT_POOL_INITIALIZED: [u8; 8] = *b"ADPIEV1\0";
const EVENT_PAGE_INITIALIZED: [u8; 8] = *b"ADBIEV1\0";
const EVENT_POSITION_INITIALIZED: [u8; 8] = *b"ADPOEV1\0";
const EVENT_LIQUIDITY_ADDED: [u8; 8] = *b"ADLAEV1\0";
const EVENT_LIQUIDITY_REMOVED: [u8; 8] = *b"ADLREV1\0";
const EVENT_SWAP_EXECUTED: [u8; 8] = *b"ADSWPV1\0";
const EVENT_STATUS_CHANGED: [u8; 8] = *b"ADSTEV1\0";
const EVENT_FEES_COLLECTED: [u8; 8] = *b"ADFEEV1\0";
const EVENT_POOL_SETTLED: [u8; 8] = *b"ADSEEV1\0";
const EVENT_POOL_CLOSED: [u8; 8] = *b"ADCLEV1\0";

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct PoolInitializedEvent {
    pool: Pubkey,
    market: Pubkey,
    liquidity_manager: Pubkey,
    maximum_bin_id: u16,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct PageInitializedEvent {
    pool: Pubkey,
    page_index: u16,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct PositionInitializedEvent {
    pool: Pubkey,
    position: Pubkey,
    owner: Pubkey,
    position_nonce: u64,
    lower_bin_id: u16,
    bin_count: u8,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct LiquidityEvent {
    pool: Pubkey,
    position: Pubkey,
    owner: Pubkey,
    option_amount: u64,
    quote_amount: u64,
    shares: u128,
    entry_count: u8,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct SwapEvent {
    pool: Pubkey,
    trader: Pubkey,
    direction: u8,
    amount_in: u64,
    amount_out: u64,
    total_fee: u64,
    protocol_fee: u64,
    first_bin: u16,
    last_bin: u16,
    bins_crossed: u8,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct StatusEvent {
    pool: Pubkey,
    old_status: AmoebaDlmmPoolStatus,
    new_status: AmoebaDlmmPoolStatus,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct FeesEvent {
    pool: Pubkey,
    option_amount: u64,
    quote_amount: u64,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct SettledEvent {
    pool: Pubkey,
    settlement: Pubkey,
    settlement_price_atomic: u64,
    slot: u64,
}

#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct ClosedEvent {
    pool: Pubkey,
    page_index: Option<u16>,
    fully_closed: bool,
    slot: u64,
}

enum AmoebaDlmmEvent {
    PoolInitialized(PoolInitializedEvent),
    PageInitialized(PageInitializedEvent),
    PositionInitialized(PositionInitializedEvent),
    Liquidity(LiquidityEvent),
    Swap(SwapEvent),
    Status(StatusEvent),
    Fees(FeesEvent),
    Settled(SettledEvent),
    Closed(ClosedEvent),
}

struct EventWriter {
    bytes: [u8; 137],
    len: usize,
}

impl EventWriter {
    #[inline(always)]
    fn new() -> Self {
        Self {
            bytes: [0; 137],
            len: 0,
        }
    }

    #[inline(always)]
    fn fixed<const LENGTH: usize>(&mut self, value: &[u8; LENGTH]) {
        // SAFETY: 137 bytes is the exact maximum of the fixed event layouts below.
        unsafe {
            std::ptr::copy_nonoverlapping(
                value.as_ptr(),
                self.bytes.as_mut_ptr().add(self.len),
                LENGTH,
            );
        }
        self.len += LENGTH;
    }

    #[inline(always)]
    fn pubkey(&mut self, value: &Pubkey) {
        self.fixed(&value.to_bytes());
    }

    #[inline(always)]
    fn u8(&mut self, value: u8) {
        self.fixed(&[value]);
    }

    #[inline(always)]
    fn u16(&mut self, value: u16) {
        self.fixed(&value.to_le_bytes());
    }

    #[inline(always)]
    fn u64(&mut self, value: u64) {
        self.fixed(&value.to_le_bytes());
    }

    #[inline(always)]
    fn u128(&mut self, value: u128) {
        self.fixed(&value.to_le_bytes());
    }

    #[inline(always)]
    fn optional_u16(&mut self, value: Option<u16>) {
        match value {
            None => self.u8(0),
            Some(value) => {
                self.u8(1);
                self.u16(value);
            }
        }
    }
}

fn encode_event(event: &AmoebaDlmmEvent) -> EventWriter {
    let mut writer = EventWriter::new();
    match event {
        AmoebaDlmmEvent::PoolInitialized(value) => {
            writer.pubkey(&value.pool);
            writer.pubkey(&value.market);
            writer.pubkey(&value.liquidity_manager);
            writer.u16(value.maximum_bin_id);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::PageInitialized(value) => {
            writer.pubkey(&value.pool);
            writer.u16(value.page_index);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::PositionInitialized(value) => {
            writer.pubkey(&value.pool);
            writer.pubkey(&value.position);
            writer.pubkey(&value.owner);
            writer.u64(value.position_nonce);
            writer.u16(value.lower_bin_id);
            writer.u8(value.bin_count);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::Liquidity(value) => {
            writer.pubkey(&value.pool);
            writer.pubkey(&value.position);
            writer.pubkey(&value.owner);
            writer.u64(value.option_amount);
            writer.u64(value.quote_amount);
            writer.u128(value.shares);
            writer.u8(value.entry_count);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::Swap(value) => {
            writer.pubkey(&value.pool);
            writer.pubkey(&value.trader);
            writer.u8(value.direction);
            writer.u64(value.amount_in);
            writer.u64(value.amount_out);
            writer.u64(value.total_fee);
            writer.u64(value.protocol_fee);
            writer.u16(value.first_bin);
            writer.u16(value.last_bin);
            writer.u8(value.bins_crossed);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::Status(value) => {
            writer.pubkey(&value.pool);
            writer.u8(value.old_status as u8);
            writer.u8(value.new_status as u8);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::Fees(value) => {
            writer.pubkey(&value.pool);
            writer.u64(value.option_amount);
            writer.u64(value.quote_amount);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::Settled(value) => {
            writer.pubkey(&value.pool);
            writer.pubkey(&value.settlement);
            writer.u64(value.settlement_price_atomic);
            writer.u64(value.slot);
        }
        AmoebaDlmmEvent::Closed(value) => {
            writer.pubkey(&value.pool);
            writer.optional_u16(value.page_index);
            writer.u8(u8::from(value.fully_closed));
            writer.u64(value.slot);
        }
    }
    writer
}

fn emit_event(discriminator: &[u8; 8], event: AmoebaDlmmEvent) -> ProgramResult {
    let encoded = encode_event(&event);
    solana_program::log::sol_log_data(&[discriminator, &encoded.bytes[..encoded.len]]);
    Ok(())
}

fn math_error(error: AmoebaDlmmMathError) -> ProgramError {
    match error {
        AmoebaDlmmMathError::ArithmeticOverflow | AmoebaDlmmMathError::DivisionByZero => {
            VaultError::ArithmeticOverflow.into()
        }
        AmoebaDlmmMathError::InsufficientLiquidity => {
            VaultError::InsufficientAmoebaDlmmLiquidity.into()
        }
        AmoebaDlmmMathError::MinimumOutputNotMet => VaultError::AmoebaDlmmSlippageExceeded.into(),
        AmoebaDlmmMathError::PriceLimitExceeded => VaultError::AmoebaDlmmPriceLimitExceeded.into(),
        AmoebaDlmmMathError::TooManyBins => VaultError::AmoebaDlmmRouteTooLarge.into(),
        AmoebaDlmmMathError::InvalidBin => VaultError::InvalidAmoebaDlmmGrid.into(),
        AmoebaDlmmMathError::InvalidFee => VaultError::InvalidAmoebaDlmmFees.into(),
        AmoebaDlmmMathError::InvalidRoute => VaultError::InvalidAmoebaDlmmRoute.into(),
        AmoebaDlmmMathError::InvalidAmount | AmoebaDlmmMathError::InvalidLiquidity => {
            VaultError::InvalidAmoebaDlmmLiquidity.into()
        }
    }
}

fn decode<T: AmoebaDlmmDecode>(payload: &[u8]) -> Result<T, ProgramError> {
    decode_exact(payload).map_err(|_| VaultError::InvalidInstructionData.into())
}

fn expect_empty(payload: &[u8]) -> ProgramResult {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(VaultError::InvalidInstructionData.into())
    }
}

#[inline(always)]
fn process_initialize_bin_page_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: InitializeAmoebaDlmmBinPageV1Params = decode(payload)?;
    process_initialize_bin_page(program_id, accounts, params)
}

#[inline(always)]
fn process_initialize_position_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: InitializeAmoebaDlmmPositionV1Params = decode(payload)?;
    process_initialize_position(program_id, accounts, params)
}

#[inline(always)]
fn process_liquidity_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
    add: bool,
) -> ProgramResult {
    let change = if add {
        LiquidityChange::Add(decode::<AddAmoebaDlmmLiquidityV1Params>(payload)?)
    } else {
        LiquidityChange::Remove(decode::<RemoveAmoebaDlmmLiquidityV1Params>(payload)?)
    };
    process_liquidity_change(program_id, accounts, change)
}

#[inline(always)]
fn process_collect_protocol_fees_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: CollectAmoebaDlmmProtocolFeesV1Params = decode(payload)?;
    process_collect_protocol_fees(program_id, accounts, params)
}

#[inline(always)]
fn process_close_pool_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty(payload)?;
    process_close_pool(program_id, accounts)
}

#[inline(always)]
fn process_initialize_collective_pool_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: InitializeAmoebaDlmmPoolV1Params = decode(payload)?;
    process_initialize_collective_pool(program_id, accounts, params)
}

#[inline(always)]
fn process_set_collective_pool_status_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: SetAmoebaDlmmPoolStatusV1Params = decode(payload)?;
    process_set_collective_pool_status(program_id, accounts, params)
}

#[inline(always)]
fn process_collective_swap_exact_in_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params: SwapAmoebaDlmmExactInV1Params = decode(payload)?;
    process_collective_swap_exact_in(program_id, accounts, params)
}

#[inline(always)]
fn process_settle_collective_pool_payload(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    expect_empty(payload)?;
    process_settle_collective_pool(program_id, accounts)
}

#[inline(always)]
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    tag: AmoebaDlmmInstructionTag,
    payload: &[u8],
) -> ProgramResult {
    match tag {
        AmoebaDlmmInstructionTag::InitializeBinPageV1 => {
            process_initialize_bin_page_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::InitializePositionV1 => {
            process_initialize_position_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::AddLiquidityV1 => {
            process_liquidity_payload(program_id, accounts, payload, true)
        }
        AmoebaDlmmInstructionTag::RemoveLiquidityV1 => {
            process_liquidity_payload(program_id, accounts, payload, false)
        }
        AmoebaDlmmInstructionTag::CollectProtocolFeesV1 => {
            process_collect_protocol_fees_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::ClosePoolV1 => {
            process_close_pool_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::InitializeCollectiveDlmmPoolV1 => {
            process_initialize_collective_pool_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::SetCollectiveDlmmPoolStatusV1 => {
            process_set_collective_pool_status_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::SwapCollectiveDlmmExactInV1 => {
            process_collective_swap_exact_in_payload(program_id, accounts, payload)
        }
        AmoebaDlmmInstructionTag::SettleCollectiveDlmmPoolV1 => {
            process_settle_collective_pool_payload(program_id, accounts, payload)
        }
        _ => Err(VaultError::InvalidInstructionData.into()),
    }
}

mod accounts;
mod collective;
mod initialization;
mod lifecycle;
mod liquidity;
mod swap;

use accounts::*;
use collective::*;
use initialization::*;
use lifecycle::*;
use liquidity::*;
use swap::*;
