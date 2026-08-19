pub mod ameba_dlmm_instruction;
pub mod ameba_dlmm_math;
pub mod ameba_dlmm_state;
pub mod associated_token;
pub mod compression;
pub mod constants;
pub mod error;
mod fixed_codec;
pub mod instruction;
mod light_token_instruction;
pub mod processor;
pub mod state;
mod system_instruction;
mod token_instruction;
mod token_state;
pub mod writer_sleeve_math;

use light_sdk::{derive_light_cpi_signer, CpiSigner};
use solana_program::{entrypoint::ProgramResult, pubkey::Pubkey};

solana_program::declare_id!("9ipkBCjEfeJDMF6AFrezRmDDHmbnmeyv45cfXNqAnWsH");
pub const LIGHT_CPI_SIGNER: CpiSigner =
    derive_light_cpi_signer!("9ipkBCjEfeJDMF6AFrezRmDDHmbnmeyv45cfXNqAnWsH");

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
    if accounts.is_empty()
        && instruction_data.len() == writer_sleeve_math::WRITER_MATH_BENCHMARK_DOMAIN.len() + 1
        && instruction_data.starts_with(writer_sleeve_math::WRITER_MATH_BENCHMARK_DOMAIN)
    {
        return writer_sleeve_math::process_sbf_benchmark(
            instruction_data[writer_sleeve_math::WRITER_MATH_BENCHMARK_DOMAIN.len()],
        );
    }
    processor::process_instruction(program_id, accounts, instruction_data)
}
