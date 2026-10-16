//! Ameba oracle program. It forms and settles governed index periods; other
//! programs read the results through `ameba-oracle-interface`.

pub mod associated_token;
mod bounded_heap;
pub mod cfm;
pub mod constants;
pub mod error;
mod fixed_codec;
pub mod governance_gate;
pub mod governance_manifest;
pub mod instruction;
pub mod live_index;
pub(crate) mod oracle_rank;
pub mod processor;
pub mod sponsorship;
pub mod state;
mod system_instruction;
mod token_instruction;
mod token_state;

pub use ameba_oracle_interface as interface;

use solana_program::pubkey::Pubkey;

pub(crate) mod compact_error;
pub use compact_error::{ProgramError, ProgramResult};

// The production (mainnet) oracle address, reserved by the user and bound in source:
// ship/releases/2026-10-15-preflight.md, "Decisions" 4. Dev builds keep their own key below.
#[cfg(not(any(feature = "localnet-dev-authority", feature = "localnet-governance")))]
solana_program::declare_id!("45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS");

// The localnet dev profile deploys under a key the operator holds; its public key is supplied at
// build time and validated as base58 at compile time.
#[cfg(any(feature = "localnet-dev-authority", feature = "localnet-governance"))]
solana_program::declare_id!(env!(
    "AMEBA_ORACLE_PROGRAM_ID",
    "the localnet oracle profiles need AMEBA_ORACLE_PROGRAM_ID=<program public key> at build time"
));

#[cfg(all(
    feature = "localnet-dev-authority",
    any(
        feature = "governance-gate-v1",
        feature = "synthetic-governance-controller",
        feature = "localnet-governance"
    )
))]
compile_error!(
    "localnet-dev-authority is the ungated localnet dev profile; it cannot be combined with the governance gate"
);

#[cfg(all(
    feature = "localnet-governance",
    feature = "synthetic-governance-controller"
))]
compile_error!("localnet-governance uses the real controller; the synthetic fixture is a separate test profile");

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

/// Share the System Program identity comparison without constructing a temporary 32-byte key at
/// each call site.
#[inline(never)]
pub(crate) fn is_system_program(value: &Pubkey) -> bool {
    pubkey_eq(value, &solana_sdk_ids::system_program::ID)
}

#[inline(never)]
pub(crate) fn bytes32_is_zero(value: &[u8; 32]) -> bool {
    (word32(value, 0) | word32(value, 1) | word32(value, 2) | word32(value, 3)) == 0
}

/// Share byte-padding checks without a separate byte loop at each call site.
#[inline(never)]
pub(crate) fn bytes_are_zero(value: &[u8]) -> bool {
    let (words, tail) = value.as_chunks::<8>();
    words.iter().all(|word| u64::from_le_bytes(*word) == 0) && tail.iter().all(|byte| *byte == 0)
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

/// Expected validation failures return typed errors before reaching this handler.
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

#[cfg(test)]
mod fast_compare_tests {
    use super::*;

    #[test]
    fn variable_zero_check_visits_every_byte_at_unaligned_boundaries() {
        let mut bytes = vec![0u8; 528];
        for start in 0..8 {
            for len in 0..=520 {
                assert!(bytes_are_zero(&bytes[start..start + len]));
                for index in start..start + len {
                    bytes[index] = 0x80;
                    assert!(!bytes_are_zero(&bytes[start..start + len]));
                    bytes[index] = 0;
                }
            }
        }
    }
    use proptest::prelude::*;

    fn flip(mut bytes: [u8; 32], index: usize, mask: u8) -> [u8; 32] {
        bytes[index] ^= mask;
        bytes
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4096))]
        #[test]
        fn bytes32_eq_matches_array_equality(
            a in proptest::array::uniform32(any::<u8>()),
            b in proptest::array::uniform32(any::<u8>()),
            index in 0usize..32,
            mask in 1u8..=255,
        ) {
            prop_assert_eq!(bytes32_eq(&a, &b), a == b);
            prop_assert!(bytes32_eq(&a, &a));
            // Every single-byte difference, in every word, is detected.
            let changed = flip(a, index, mask);
            prop_assert!(!bytes32_eq(&a, &changed));
            prop_assert!(!bytes32_eq(&changed, &a));
            let (pa, pc) = (Pubkey::new_from_array(a), Pubkey::new_from_array(changed));
            prop_assert_eq!(pubkey_eq(&pa, &pc), pa == pc);
            prop_assert!(pubkey_eq(&pa, &pa));
            prop_assert_eq!(bytes32_is_zero(&a), a == [0; 32]);
            prop_assert_eq!(pubkey_is_default(&pa), pa == Pubkey::default());
            prop_assert_eq!(is_system_program(&pa), solana_sdk_ids::system_program::check_id(&pa));
            prop_assert!(!bytes32_is_zero(&flip([0; 32], index, mask)));
        }
    }

    #[test]
    fn zero_and_default_are_recognized() {
        assert!(bytes32_is_zero(&[0; 32]));
        assert!(pubkey_is_default(&Pubkey::default()));
        assert!(is_system_program(&solana_sdk_ids::system_program::ID));
        assert!(bytes32_eq(&[0; 32], &[0; 32]));
        assert!(!bytes32_eq(&[0; 32], &[0xff; 32]));
    }
}
