use solana_program::program_error::ProgramError;
use thiserror::Error;

pub type GovernanceResult<T> = Result<T, GovernanceError>;

/// Explicit predeployment V1 error codes. Numeric assignments remain stable
/// even when Phase 2 corrects keypair-oriented source terminology.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[repr(u32)]
pub enum GovernanceError {
    #[error("invalid account discriminator")]
    InvalidDiscriminator = 10_000,
    #[error("unsupported account version")]
    UnsupportedVersion = 10_001,
    #[error("account is not initialized")]
    Uninitialized = 10_002,
    #[error("reserved bytes must be zero")]
    NonzeroReserved = 10_003,
    #[error("a required public key is the default key")]
    DefaultPubkey = 10_004,
    #[error("optional public key encoding is not canonical")]
    InvalidOptionalPubkey = 10_005,
    #[error("council shape is invalid")]
    InvalidCouncilComposition = 10_006,
    #[error("council contains a duplicate seat authority")]
    DuplicateSeatAuthority = 10_007,
    #[error("seat authority must carry Solana signer privilege")]
    MissingSeatAuthoritySignature = 10_008,
    #[error("seat authority must be read-only")]
    WritableSeatAuthority = 10_009,
    #[error("council seat is inactive or outside its term")]
    InactiveCouncilSeat = 10_010,
    #[error("council activation or deactivation range is invalid")]
    InvalidCouncilActivation = 10_011,
    #[error("council threshold policy is invalid")]
    InvalidCouncilThreshold = 10_012,
    #[error("council set hash does not match its canonical fields")]
    CouncilHashMismatch = 10_013,
    #[error("governance policy is invalid")]
    InvalidPolicy = 10_014,
    #[error("governance policy hash does not match its canonical fields")]
    PolicyHashMismatch = 10_015,
    #[error("proposal references a stale council version")]
    StaleCouncilVersion = 10_016,
    #[error("proposal council hash does not match the supplied council")]
    ProposalCouncilHashMismatch = 10_017,
    #[error("approval bitset contains an unknown seat")]
    InvalidApprovalBitset = 10_018,
    #[error("approval count does not match the approval bitset")]
    ApprovalCountMismatch = 10_019,
    #[error("authority does not occupy a council seat")]
    UnknownSeatAuthority = 10_020,
    #[error("council seat has already approved")]
    DuplicateApproval = 10_021,
    #[error("council quorum is not satisfied")]
    QuorumNotSatisfied = 10_022,
    #[error("seat authority must not be executable")]
    ExecutableSeatAuthority = 10_023,
    #[error("proposal state transition is not permitted")]
    InvalidStateTransition = 10_024,
    #[error("proposal timing commitments are invalid")]
    InvalidProposalTiming = 10_025,
    #[error("proposal epoch commitments are invalid")]
    InvalidProposalEpoch = 10_026,
    #[error("proposal capacity arithmetic is invalid")]
    InvalidCapacityPlan = 10_027,
    #[error("proposal static fields are internally inconsistent")]
    InvalidProposalCommitment = 10_028,
    #[error("proposal digest does not match its canonical commitments")]
    ProposalDigestMismatch = 10_029,
    #[error("arithmetic overflow")]
    ArithmeticOverflow = 10_030,
    #[error("controller configuration is internally inconsistent")]
    InvalidControllerConfig = 10_031,
    #[error("protocol gate state is internally inconsistent")]
    InvalidGateState = 10_032,
    #[error("proposal class is scaffolded but has no safe execution path")]
    UnsupportedProposalClass = 10_033,
    #[error("instruction requires exactly five accounts")]
    InvalidAccountCount = 10_034,
    #[error("controller account privileges are invalid")]
    InvalidAccountPrivileges = 10_035,
    #[error("controller account has the wrong owner")]
    IncorrectAccountOwner = 10_036,
    #[error("controller account has the wrong fixed length")]
    InvalidAccountSize = 10_037,
    #[error("account address or stored bump is not canonical")]
    InvalidPda = 10_038,
    #[error("cross-account governance identities are inconsistent")]
    CrossAccountMismatch = 10_039,
    #[error("the selected governance policy is not active")]
    InactivePolicy = 10_040,
    #[error("Release 1 account fields are internally inconsistent")]
    InvalidRelease1Account = 10_041,
    #[error("Release 1 verification bitmap is not canonical")]
    InvalidRelease1Bitmap = 10_042,
    #[error("artifact Merkle parameters are invalid")]
    InvalidMerkleParameters = 10_043,
    #[error("artifact Merkle proof is invalid")]
    InvalidMerkleProof = 10_044,
    #[error("Release 1 digest does not match its canonical commitments")]
    Release1DigestMismatch = 10_045,
    #[error("ProgramData observation fields are not canonical")]
    InvalidProgramDataObservation = 10_046,
    #[error("ProgramData observation is stale")]
    StaleProgramDataObservation = 10_047,
    #[error("ProgramData observation is not mechanically complete")]
    IncompleteProgramDataObservation = 10_048,
    #[error("ProgramData capacity policy does not match the canonical policy")]
    CapacityPolicyMismatch = 10_049,
    #[error("controller immutability has not been proven")]
    ControllerNotImmutable = 10_050,
    #[error("Loader authority transition is not the exact admitted transition")]
    InvalidAuthorityTransition = 10_051,
    #[error("ceremony account is already terminal or finalized")]
    CeremonyAlreadyFinalized = 10_052,
}

impl From<GovernanceError> for ProgramError {
    fn from(value: GovernanceError) -> Self {
        ProgramError::Custom(value as u32)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn custom_error_codes_are_unique_and_map_exactly() {
        let values = [
            GovernanceError::InvalidDiscriminator,
            GovernanceError::UnsupportedVersion,
            GovernanceError::Uninitialized,
            GovernanceError::NonzeroReserved,
            GovernanceError::DefaultPubkey,
            GovernanceError::InvalidOptionalPubkey,
            GovernanceError::InvalidCouncilComposition,
            GovernanceError::DuplicateSeatAuthority,
            GovernanceError::MissingSeatAuthoritySignature,
            GovernanceError::WritableSeatAuthority,
            GovernanceError::InactiveCouncilSeat,
            GovernanceError::InvalidCouncilActivation,
            GovernanceError::InvalidCouncilThreshold,
            GovernanceError::CouncilHashMismatch,
            GovernanceError::InvalidPolicy,
            GovernanceError::PolicyHashMismatch,
            GovernanceError::StaleCouncilVersion,
            GovernanceError::ProposalCouncilHashMismatch,
            GovernanceError::InvalidApprovalBitset,
            GovernanceError::ApprovalCountMismatch,
            GovernanceError::UnknownSeatAuthority,
            GovernanceError::DuplicateApproval,
            GovernanceError::QuorumNotSatisfied,
            GovernanceError::ExecutableSeatAuthority,
            GovernanceError::InvalidStateTransition,
            GovernanceError::InvalidProposalTiming,
            GovernanceError::InvalidProposalEpoch,
            GovernanceError::InvalidCapacityPlan,
            GovernanceError::InvalidProposalCommitment,
            GovernanceError::ProposalDigestMismatch,
            GovernanceError::ArithmeticOverflow,
            GovernanceError::InvalidControllerConfig,
            GovernanceError::InvalidGateState,
            GovernanceError::UnsupportedProposalClass,
            GovernanceError::InvalidAccountCount,
            GovernanceError::InvalidAccountPrivileges,
            GovernanceError::IncorrectAccountOwner,
            GovernanceError::InvalidAccountSize,
            GovernanceError::InvalidPda,
            GovernanceError::CrossAccountMismatch,
            GovernanceError::InactivePolicy,
            GovernanceError::InvalidRelease1Account,
            GovernanceError::InvalidRelease1Bitmap,
            GovernanceError::InvalidMerkleParameters,
            GovernanceError::InvalidMerkleProof,
            GovernanceError::Release1DigestMismatch,
            GovernanceError::InvalidProgramDataObservation,
            GovernanceError::StaleProgramDataObservation,
            GovernanceError::IncompleteProgramDataObservation,
            GovernanceError::CapacityPolicyMismatch,
            GovernanceError::ControllerNotImmutable,
            GovernanceError::InvalidAuthorityTransition,
            GovernanceError::CeremonyAlreadyFinalized,
        ];
        let codes = values.iter().map(|value| *value as u32).collect::<Vec<_>>();
        assert_eq!(codes.iter().collect::<BTreeSet<_>>().len(), codes.len());
        for value in values {
            assert_eq!(
                ProgramError::from(value),
                ProgramError::Custom(value as u32)
            );
        }
    }
}
