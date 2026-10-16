use ameba_oracle::state::{
    OracleOpeningClaim, OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OraclePhase,
    OracleSourceStatus,
};

#[test]
fn opening_claim_starts_pending_without_activating_the_source() {
    let claim = OracleOpeningClaim::default();
    assert_eq!(claim.status, OracleOpeningClaimStatus::Empty);
    assert_eq!(borsh::to_vec(&OraclePhase::Opening).unwrap(), vec![5]);
    assert_eq!(
        borsh::to_vec(&OracleSourceStatus::OpeningPending).unwrap(),
        vec![4]
    );
    assert_eq!(borsh::to_vec(&OracleSourceStatus::Active).unwrap(), vec![5]);
}

#[test]
fn current_opening_accounts_fit_their_exact_allocations() {
    let claim = OracleOpeningClaim {
        archive_url_hash: [0x41; 32],
        ..OracleOpeningClaim::default()
    };
    let challenge = OracleOpeningClaimChallenge {
        archive_url_hash: [0x42; 32],
        ..OracleOpeningClaimChallenge::default()
    };

    assert!(borsh::to_vec(&claim).unwrap().len() <= OracleOpeningClaim::LEN);
    assert!(borsh::to_vec(&challenge).unwrap().len() <= OracleOpeningClaimChallenge::LEN);
    assert_eq!(OracleOpeningClaim::LEN, 144);
    assert_eq!(OracleOpeningClaimChallenge::LEN, 384);
    assert_eq!(OracleOpeningClaimChallenge::ACCOUNT_DISCRIMINATOR, *b"OCH");
    assert_eq!(OracleOpeningClaimChallenge::ACCOUNT_VERSION, 4);
}
