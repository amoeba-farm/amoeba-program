use ameba_oracle::state::{
    OracleEscrowDisposition, OracleOpeningClaim, OracleOpeningClaimChallenge,
    OracleSourceChallenge, OracleSupportPosition, OracleUpdateChallenge, OracleUpdateClaimV2,
};

#[test]
fn every_current_oracle_escrow_record_starts_unsettled() {
    for disposition in [
        OracleSupportPosition::default().escrow_disposition,
        OracleSourceChallenge::default().escrow_disposition,
        OracleOpeningClaim::default().escrow_disposition,
        OracleOpeningClaimChallenge::default().escrow_disposition,
        OracleUpdateClaimV2::default().claim.escrow_disposition,
        OracleUpdateChallenge::default().escrow_disposition,
    ] {
        assert_eq!(disposition, OracleEscrowDisposition::Unsettled);
    }
}
