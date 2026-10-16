use borsh::{BorshDeserialize, BorshSerialize};

use crate::state::{
    CheckpointPhaseV1, ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1,
    GovernanceModeV1, GovernancePolicyV1, ProposalClassV1, ProposalStateV1, ProtocolGateV1,
    UpgradeProposalV1, VoteRequirementV1,
};

use super::support::{controller_config, council, gate, policy, proposal};

fn assert_fixed_roundtrip<T>(value: &T, expected_len: usize)
where
    T: BorshSerialize + BorshDeserialize + PartialEq + std::fmt::Debug,
{
    let bytes = value.try_to_vec().expect("serialize");
    assert_eq!(bytes.len(), expected_len);
    assert_eq!(T::try_from_slice(&bytes).expect("deserialize"), *value);
    assert!(T::try_from_slice(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(T::try_from_slice(&trailing).is_err());
}

#[test]
fn account_layouts_have_exact_deterministic_lengths() {
    let council = council();
    assert_fixed_roundtrip(&controller_config(), ControllerConfigV1::LEN);
    assert_fixed_roundtrip(&policy(), GovernancePolicyV1::LEN);
    assert_fixed_roundtrip(&council.seats[0], CouncilSeatV1::LEN);
    assert_fixed_roundtrip(&council, GovernanceCouncilSetV1::LEN);
    assert_fixed_roundtrip(&gate(), ProtocolGateV1::LEN);
    assert_fixed_roundtrip(&proposal(), UpgradeProposalV1::LEN);
}

#[test]
fn config_and_gate_static_invariants_fail_closed() {
    let mut config = controller_config();
    assert_eq!(config.validate_static(), Ok(()));
    assert_eq!(config.council_review_slots(), config.vote_review_slots);
    config.token_governance_enabled = true;
    assert!(config.validate_static().is_err());

    for mutate in [
        |config: &mut ControllerConfigV1| config.vote_program = super::support::key(90),
        |config: &mut ControllerConfigV1| config.vote_programdata = super::support::key(91),
        |config: &mut ControllerConfigV1| config.vote_config = super::support::key(92),
        |config: &mut ControllerConfigV1| config.vote_mint = super::support::key(93),
        |config: &mut ControllerConfigV1| config.next_proposal_id = 0,
        |config: &mut ControllerConfigV1| config.target_nonce = 0,
        |config: &mut ControllerConfigV1| config.proposal_expiry_slots = config.major_delay_slots,
    ] {
        let mut config = controller_config();
        mutate(&mut config);
        assert!(config.validate_static().is_err());
    }

    let mut exact_boundary = controller_config();
    exact_boundary.proposal_expiry_slots = 1u64
        .checked_add(exact_boundary.vote_review_slots)
        .and_then(|slots| slots.checked_add(exact_boundary.major_delay_slots))
        .and_then(|slots| slots.checked_add(exact_boundary.vote_review_slots))
        .and_then(|slots| slots.checked_add(2))
        .unwrap();
    assert!(exact_boundary.validate_static().is_err());
    exact_boundary.proposal_expiry_slots += 1;
    assert_eq!(exact_boundary.validate_static(), Ok(()));

    let mut overflow = controller_config();
    overflow.vote_review_slots = u64::MAX;
    assert!(overflow.validate_static().is_err());

    let mut invalid_gate = gate();
    assert_eq!(invalid_gate.validate_static(), Ok(()));
    invalid_gate.freeze_slot = 1;
    assert!(invalid_gate.validate_static().is_err());

    let mut zero_epoch_gate = gate();
    zero_epoch_gate.epoch = 0;
    assert!(zero_epoch_gate.validate_static().is_err());

    let mut gate = gate();
    gate.status = GateStatusV1::EmergencyFrozen;
    gate.freeze_slot = 500;
    gate.freeze_reason_code = 7;
    assert_eq!(gate.validate_static(), Ok(()));
    gate.active_proposal = super::support::key(90);
    assert!(gate.validate_static().is_err());
}

#[test]
fn every_stable_enum_has_one_byte_wire_values_and_rejects_unknowns() {
    macro_rules! check {
        ($ty:ty, {$($variant:expr => $byte:expr),+ $(,)?}, $unknown:expr) => {{
            $(assert_eq!($variant.try_to_vec().unwrap(), vec![$byte]);)+
            assert!(<$ty>::try_from_slice(&[$unknown]).is_err());
        }};
    }
    check!(GovernanceModeV1, {
        GovernanceModeV1::BootstrapCouncilOnly => 0
    }, 1);
    check!(GateStatusV1, {
        GateStatusV1::Active => 0,
        GateStatusV1::FrozenForUpgrade => 1,
        GateStatusV1::EmergencyFrozen => 2
    }, 3);
    check!(CheckpointPhaseV1, {
        CheckpointPhaseV1::Prestate => 0,
        CheckpointPhaseV1::Poststate => 1
    }, 2);
    check!(ProposalClassV1, {
        ProposalClassV1::RoutineUpgrade => 0,
        ProposalClassV1::EmergencyRollback => 1,
        ProposalClassV1::EconomicChange => 2,
        ProposalClassV1::ConstitutionalChange => 3,
        ProposalClassV1::CouncilSetRotation => 4,
        ProposalClassV1::TargetImmutability => 5
    }, 6);
    check!(VoteRequirementV1, {
        VoteRequirementV1::None => 0,
        VoteRequirementV1::Veto => 1,
        VoteRequirementV1::Affirmative => 2
    }, 3);
    for byte in 0u8..=15 {
        assert_eq!(
            ProposalStateV1::try_from_slice(&[byte]).unwrap() as u8,
            byte
        );
    }
    assert!(ProposalStateV1::try_from_slice(&[16]).is_err());
}

#[test]
fn borsh_rejects_noncanonical_boolean_bytes() {
    let mut bytes = controller_config().try_to_vec().unwrap();
    // discriminator(8) + version(1) + bump(1)
    bytes[10] = 2;
    assert!(ControllerConfigV1::try_from_slice(&bytes).is_err());
}
