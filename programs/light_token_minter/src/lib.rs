pub mod ameba_dlmm_instruction;
pub mod ameba_dlmm_math;
pub mod ameba_dlmm_state;
pub mod associated_token;
pub mod business_generation;
pub mod compressed_custody;
pub mod compressed_option_settlement;
pub mod compressed_swap_plan;
pub mod compression;
pub mod constants;
pub mod dlmm_order_math;
pub mod dlmm_order_state;
pub mod error;
mod fixed_codec;
pub mod governance_gate;
pub mod governance_manifest;
pub mod individual_writer;
pub mod instruction;
mod light_token_instruction;
mod local_direct_address;
pub(crate) mod observation_wire;
pub mod oracle_parent_proxy;
pub mod oracle_rank;
pub mod oracle_sponsorship;
pub mod processor;
pub mod regular_compressed_transfer;
pub mod scoped_settlement;
pub mod state;
mod system_instruction;
mod token_instruction;
mod token_state;
pub mod writer_dlmm_instruction;
pub mod writer_dlmm_math;
pub mod writer_dlmm_quote;
pub mod writer_participation_math;
pub mod writer_participation_state;
pub mod writer_portfolio;
pub mod writer_settlement_handoff;
pub mod writer_sleeve_math;

#[cfg(not(feature = "mainnet-v3"))]
use light_sdk::derive_light_cpi_signer;
use light_sdk::CpiSigner;
use solana_program::pubkey::Pubkey;

pub(crate) mod compact_error;
pub use compact_error::{ProgramError, ProgramResult};

#[cfg(not(feature = "mainnet-v3"))]
solana_program::declare_id!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");
#[cfg(feature = "mainnet-v3")]
include!(env!("AMEBA_MAINNET_PROFILE_RS"));
#[cfg(all(
    feature = "mainnet-v3",
    any(
        feature = "devnet-solo-backfill-2026",
        feature = "devnet-v3-governance-controller",
        feature = "phase3-synthetic-governance-controller",
        feature = "reviewed-governance-controller",
        feature = "local-ceremony-governance-controller",
        feature = "test-sbf"
    )
))]
compile_error!("mainnet-v3 excludes other controllers and test timing capabilities");

#[cfg(all(
    feature = "governance-gate-v1",
    not(any(
        feature = "phase3-synthetic-governance-controller",
        feature = "reviewed-governance-controller",
        feature = "devnet-v3-governance-controller",
        feature = "mainnet-v3"
    ))
))]
compile_error!(
    "governance-gate-v1 requires exactly one reviewed pinned controller identity; use the explicit synthetic feature only for local tests or generate a reviewed ceremony identity"
);
#[cfg(all(
    feature = "phase3-synthetic-governance-controller",
    feature = "reviewed-governance-controller"
))]
compile_error!("synthetic and reviewed governance controller identities are mutually exclusive");
#[cfg(all(
    feature = "devnet-v3-governance-controller",
    any(
        feature = "reviewed-governance-controller",
        feature = "phase3-synthetic-governance-controller"
    )
))]
compile_error!("Devnet V3 has one exclusive controller identity");
#[cfg(all(
    feature = "phase3-synthetic-governance-controller",
    not(feature = "governance-gate-v1")
))]
compile_error!(
    "phase3-synthetic-governance-controller is test-only and cannot be compiled without governance-gate-v1"
);
#[cfg(all(
    feature = "reviewed-governance-controller",
    not(feature = "governance-gate-v1")
))]
compile_error!("reviewed-governance-controller cannot be compiled without governance-gate-v1");
#[cfg(all(
    feature = "local-ceremony-governance-controller",
    not(feature = "reviewed-governance-controller")
))]
compile_error!(
    "local-ceremony-governance-controller cannot be compiled without reviewed-governance-controller"
);
#[cfg(not(feature = "mainnet-v3"))]
pub const LIGHT_CPI_SIGNER: CpiSigner =
    derive_light_cpi_signer!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");

/// One little-endian word of a 32-byte value. `index` is always below four.
#[inline(always)]
fn word32(value: &[u8; 32], index: usize) -> u64 {
    debug_assert!(index < 4);
    // SAFETY: `index < 4`, so the eight bytes read lie inside the 32-byte array; the read is
    // unaligned-safe.
    unsafe { core::ptr::read_unaligned(value.as_ptr().add(index * 8).cast::<u64>()) }
}

/// Byte equality of two 32-byte values (pubkeys, hashes). Equivalent to `a == b`, but compares
/// four words in place instead of calling `memcmp`, which costs a syscall on SBF.
/// The first word is checked alone because distinct keys (the common case in duplicate-account
/// scans) almost always differ there.
#[inline(never)]
pub(crate) fn bytes32_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    word32(a, 0) == word32(b, 0)
        && ((word32(a, 1) ^ word32(b, 1))
            | (word32(a, 2) ^ word32(b, 2))
            | (word32(a, 3) ^ word32(b, 3)))
            == 0
}

/// `a == b` for pubkeys; see [`bytes32_eq`].
#[inline(always)]
pub(crate) fn pubkey_eq(a: &Pubkey, b: &Pubkey) -> bool {
    bytes32_eq(a.as_array(), b.as_array())
}

#[inline(never)]
pub(crate) fn pubkey_is_default(value: &Pubkey) -> bool {
    bytes32_is_zero(value.as_array())
}

#[inline(never)]
pub(crate) fn bytes32_is_zero(value: &[u8; 32]) -> bool {
    (word32(value, 0) | word32(value, 1) | word32(value, 2) | word32(value, 3)) == 0
}

#[cfg(not(feature = "no-entrypoint"))]
#[no_mangle]
/// Solana SBF entrypoint generated against the loader's serialized input ABI.
///
/// # Safety
///
/// `input` must point to a loader-provided, valid serialized instruction context for the complete
/// duration of this call. The loader owns and validates that allocation before invoking the
/// program.
pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
    let (program_id, accounts, instruction_data) =
        unsafe { solana_program::entrypoint::deserialize(input) };
    match processor::process_instruction_compact(program_id, &accounts, instruction_data) {
        Ok(()) => solana_program::entrypoint::SUCCESS,
        // The compact error already holds the exact code Solana's `ProgramError` converts into.
        Err(error) => error.code(),
    }
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::custom_heap_default!();

/// Expected validation failures return typed `ProgramError`s before reaching this handler. Avoid
/// linking full panic formatting into the SBF artifact for unreachable internal bug paths.
#[cfg(all(not(feature = "no-entrypoint"), target_os = "solana"))]
#[no_mangle]
fn custom_panic(_: &core::panic::PanicInfo<'_>) {}

/// Public processor entry with Solana's error type, for native harnesses and host callers.
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    instruction_data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    processor::process_instruction(program_id, accounts, instruction_data)
}
