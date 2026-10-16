use ameba_oracle::state::{
    derive_oracle_source_challenge_guard_pda, derive_oracle_update_challenge_guard_pda,
    OracleSourceChallengeGuard, OracleUpdateChallengeGuard,
};
use borsh::BorshSerialize;
use solana_program::pubkey::Pubkey;
#[test]
fn challenge_guards_have_typed_fixed_layouts_and_disjoint_canonical_addresses() {
    assert_eq!(OracleSourceChallengeGuard::LEN, 224);
    assert_eq!(OracleSourceChallengeGuard::ACCOUNT_DISCRIMINATOR, *b"OCG");
    assert_eq!(OracleSourceChallengeGuard::ACCOUNT_VERSION, 1);
    assert_eq!(
        OracleSourceChallengeGuard::default()
            .try_to_vec()
            .expect("serialize source challenge guard")
            .len(),
        206
    );

    assert_eq!(OracleUpdateChallengeGuard::LEN, 224);
    assert_eq!(OracleUpdateChallengeGuard::ACCOUNT_DISCRIMINATOR, *b"OUG");
    assert_eq!(OracleUpdateChallengeGuard::ACCOUNT_VERSION, 1);
    assert_eq!(
        OracleUpdateChallengeGuard::default()
            .try_to_vec()
            .expect("serialize update challenge guard")
            .len(),
        222
    );

    let program_id = Pubkey::new_unique();
    let month = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let claim = Pubkey::new_unique();
    let source_guard = derive_oracle_source_challenge_guard_pda(&program_id, &month, &source).0;
    let update_guard = derive_oracle_update_challenge_guard_pda(&program_id, &month, &claim).0;
    assert_ne!(source_guard, source);
    assert_ne!(update_guard, claim);
    assert_ne!(source_guard, update_guard);
    assert_ne!(
        source_guard,
        derive_oracle_source_challenge_guard_pda(&program_id, &month, &claim).0
    );
}
