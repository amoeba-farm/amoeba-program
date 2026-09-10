pub mod ameba_dlmm_instruction;
pub mod ameba_dlmm_math;
pub mod ameba_dlmm_state;
pub mod associated_token;
pub mod compression;
pub mod constants;
pub mod error;
mod fixed_codec;
pub mod governance_gate;
pub mod governance_manifest;
pub mod instruction;
mod light_token_instruction;
pub mod processor;
pub mod state;
pub mod scoped_settlement;
mod system_instruction;
mod token_instruction;
mod token_state;
pub mod writer_sleeve_math;
pub mod writer_dlmm_math;
pub mod writer_dlmm_instruction;
pub mod writer_dlmm_quote;

use light_sdk::{derive_light_cpi_signer, CpiSigner};
use solana_program::{entrypoint::ProgramResult, pubkey::Pubkey};

solana_program::declare_id!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");

#[cfg(all(
    feature = "governance-gate-v1",
    not(any(
        feature = "phase3-synthetic-governance-controller",
        feature = "reviewed-governance-controller",
        feature = "devnet-v3-governance-controller"
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
pub const LIGHT_CPI_SIGNER: CpiSigner =
    derive_light_cpi_signer!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");

#[inline(never)]
pub(crate) fn pubkey_is_default(value: &Pubkey) -> bool {
    value == &Pubkey::default()
}

#[inline(never)]
pub(crate) fn bytes32_is_zero(value: &[u8; 32]) -> bool {
    value == &[0; 32]
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
    match process_instruction(program_id, &accounts, instruction_data) {
        Ok(()) => solana_program::entrypoint::SUCCESS,
        Err(error) => error.into(),
    }
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::custom_heap_default!();

/// Expected validation failures return typed `ProgramError`s before reaching this handler. Avoid
/// linking full panic formatting into the SBF artifact for unreachable internal bug paths.
#[cfg(all(not(feature = "no-entrypoint"), target_os = "solana"))]
#[no_mangle]
fn custom_panic(_: &core::panic::PanicInfo<'_>) {}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    #[cfg(feature = "writer-math-benchmark")]
    if instruction_data.starts_with(writer_sleeve_math::WRITER_MATH_BENCHMARK_DOMAIN) {
        if accounts.is_empty()
            && instruction_data.len() == writer_sleeve_math::WRITER_MATH_BENCHMARK_DOMAIN.len() + 1
        {
            return writer_sleeve_math::process_sbf_benchmark(
                instruction_data[writer_sleeve_math::WRITER_MATH_BENCHMARK_DOMAIN.len()],
            );
        }
        // The benchmark domain begins with byte 97, which is a Devnet backfill tag under a
        // separate feature. A malformed/non-accountless benchmark must never fall through into
        // that mutating registry when both features are compiled together.
        return Err(error::VaultError::InvalidInstructionData.into());
    }
    processor::process_instruction(program_id, accounts, instruction_data)
}
