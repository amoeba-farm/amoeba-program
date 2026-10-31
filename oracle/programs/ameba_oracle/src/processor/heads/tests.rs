//! Tag 210's routing: every selector decodes completely and reaches its handler before any
//! account is read; malformed payloads never do.

use super::*;
use crate::instruction::heads::tests::samples;

/// What every handler answers to a one-account list it never reads: it refuses the list before
/// touching it.
fn unread() -> ProgramError {
    OracleError::InvalidAccountList.into()
}

fn poisoned<'a>(key: &'a Pubkey, lamports: &'a mut u64, data: &'a mut [u8]) -> AccountInfo<'a> {
    AccountInfo::new(key, false, true, lamports, data, key, false, 0)
}

#[test]
fn every_selector_reaches_its_handler_without_reading_an_account() {
    let key = Pubkey::new_unique();
    let mut lamports = 1;
    let mut data = [0u8; 4];
    let account = poisoned(&key, &mut lamports, &mut data);
    // A held mutable borrow: any handler that touched the account would fail differently.
    let guard = account.try_borrow_mut_data().unwrap();
    for (instruction, _) in samples() {
        let payload = &instruction.data().unwrap()[1..];
        // A build without the dev migration (the heads-only production one) refuses selector 30
        // as data it does not take, before any account too.
        let expected = if cfg!(not(any(
            feature = "heads-migrate",
            feature = "localnet-dev-authority"
        ))) && matches!(
            instruction,
            crate::instruction::heads::HeadsInstruction::MigrateHead { .. }
        ) {
            OracleError::InvalidInstructionData.into()
        } else {
            unread()
        };
        assert_eq!(
            process(&crate::id(), core::slice::from_ref(&account), payload),
            Err(expected),
            "{instruction:?}"
        );
    }
    drop(guard);
}

#[test]
fn malformed_payloads_reject_as_invalid_instruction_data() {
    let invalid = Err(ProgramError::from(OracleError::InvalidInstructionData));
    assert_eq!(process(&crate::id(), &[], &[]), invalid);
    for (instruction, _) in samples() {
        let data = instruction.data().unwrap();
        assert_eq!(
            process(&crate::id(), &[], &data[1..data.len() - 1]),
            invalid
        );
        let mut trailing = data[1..].to_vec();
        trailing.push(0);
        assert_eq!(process(&crate::id(), &[], &trailing), invalid);
    }
    for selector in crate::instruction::heads::selector::COUNT..=u8::MAX {
        assert_eq!(process(&crate::id(), &[], &[selector]), invalid);
    }
}

/// The whole top-level path of the ungated dev profile: classification, then tag 210's routing.
#[cfg(feature = "localnet-dev-authority")]
#[test]
fn the_dev_profile_routes_tag_210_to_the_family() {
    for (instruction, _) in samples() {
        assert_eq!(
            crate::processor::process_instruction_compact(
                &crate::id(),
                &[],
                &instruction.data().unwrap()
            ),
            Err(unread()),
            "{instruction:?}"
        );
    }
    assert_eq!(
        crate::processor::process_instruction_compact(&crate::id(), &[], &[210]),
        Err(OracleError::InvalidInstructionData.into())
    );
}
