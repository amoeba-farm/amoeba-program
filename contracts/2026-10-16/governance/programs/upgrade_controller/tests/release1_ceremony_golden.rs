use std::{collections::BTreeSet, str::FromStr};

use borsh::BorshSerialize;
use serde::Deserialize;
use solana_program::pubkey::Pubkey;
use upgrade_controller::{
    pda::{derive_bootstrap_activation_pda, derive_target_handoff_pda},
    release1_ceremony_digest::{
        compute_bootstrap_activation_deployment_plan_digest_v1,
        compute_bootstrap_activation_proposal_digest_v1,
        compute_bootstrap_activation_receipt_digest_v1,
        compute_bootstrap_activation_receipt_plan_digest_v1, compute_capacity_policy_digest_v1,
        compute_controller_immutability_receipt_digest_v1, compute_controller_release_digest_v1,
        compute_current_deployment_digest_v1, compute_programdata_observation_digest_v1,
        compute_target_handoff_proposal_digest_v1, compute_target_handoff_receipt_digest_v1,
        validate_bootstrap_activation_proposal_digest_v1,
        validate_bootstrap_activation_receipt_digest_v1, validate_capacity_policy_digest_v1,
        validate_controller_immutability_receipt_digest_v1, validate_controller_release_digest_v1,
        validate_current_deployment_digest_v1, validate_programdata_observation_digest_v1,
        validate_target_handoff_proposal_digest_v1, validate_target_handoff_receipt_digest_v1,
    },
    release1_ceremony_state::{
        BootstrapActivationProposalV1, BootstrapActivationReceiptV1,
        ControllerImmutabilityReceiptV1, ControllerReleaseCommitmentV1, CurrentDeploymentStateV1,
        ProgramDataCapacityPolicyV1, ProgramDataObservationV1, TargetAuthorityHandoffProposalV1,
        TargetAuthorityHandoffReceiptV1, BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
        BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN,
        CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
        CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN,
        CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
        CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN, CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
        CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN, PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
        PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN, PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN,
        TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN,
        TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
        TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN,
    },
};

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/release1_ceremony_accounts_v1.json"
));

#[derive(Debug, Deserialize)]
struct CeremonyFixture {
    fixture_version: u8,
    activation_plan_digests: ActivationPlanDigests,
    proposal_pdas: ProposalPdas,
    entries: Vec<CeremonyEntry>,
}

#[derive(Debug, Deserialize)]
struct ActivationPlanDigests {
    deployment_plan_digest_hex: String,
    receipt_plan_digest_hex: String,
}

#[derive(Debug, Deserialize)]
struct ProposalPdas {
    controller_program: String,
    target_program: String,
    council_version: u64,
    target_authority_handoff_proposal: PdaVector,
    bootstrap_activation_proposal: PdaVector,
}

#[derive(Debug, Deserialize)]
struct PdaVector {
    address: String,
    bump: u8,
}

#[derive(Debug, Deserialize)]
struct CeremonyEntry {
    account_type: String,
    length: usize,
    discriminator_ascii: String,
    reserved_length: usize,
    digest_hex: String,
    data_base64: String,
}

fn decode_base64(value: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for byte in value.bytes() {
        if byte == b'=' {
            break;
        }
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("invalid base64 byte {byte}"),
        };
        accumulator = (accumulator << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
            accumulator &= (1u32 << bits) - 1;
        }
    }
    output
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[usize::from(byte >> 4)] as char);
        result.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    result
}

macro_rules! verify_account {
    ($entry:expr, $bytes:expr, $ty:ty, $field:ident, $compute:path, $validate:path) => {{
        let value = <$ty>::from_bytes_strict($bytes).expect("strict Rust decoding must succeed");
        assert_eq!(
            value
                .try_to_vec()
                .expect("Borsh encoding must succeed")
                .as_slice(),
            $bytes.as_slice(),
            "{} Rust bytes drifted",
            $entry.account_type
        );
        let digest = $compute(&value).expect("Rust digest must succeed");
        assert_eq!(
            digest, value.$field,
            "{} stored Rust digest drifted",
            $entry.account_type
        );
        assert_eq!(
            hex(&digest),
            $entry.digest_hex,
            "{} Rust/fixture digest drifted",
            $entry.account_type
        );
        $validate(&value).expect("stored digest must validate");
    }};
}

#[test]
fn nine_ceremony_accounts_match_cross_language_golden_bytes_and_digests() {
    let fixture: CeremonyFixture = serde_json::from_str(FIXTURE).expect("fixture must be JSON");
    assert_eq!(fixture.fixture_version, 1);
    assert_eq!(fixture.entries.len(), 9);

    let controller = Pubkey::from_str(&fixture.proposal_pdas.controller_program)
        .expect("controller program must be base58");
    let target = Pubkey::from_str(&fixture.proposal_pdas.target_program)
        .expect("target program must be base58");
    let handoff_pda =
        derive_target_handoff_pda(&controller, &target, fixture.proposal_pdas.council_version);
    let activation_pda = derive_bootstrap_activation_pda(
        &controller,
        &target,
        fixture.proposal_pdas.council_version,
    );
    assert_eq!(
        handoff_pda,
        (
            Pubkey::from_str(
                &fixture
                    .proposal_pdas
                    .target_authority_handoff_proposal
                    .address
            )
            .expect("handoff PDA must be base58"),
            fixture.proposal_pdas.target_authority_handoff_proposal.bump,
        ),
        "council-versioned handoff PDA drifted"
    );
    assert_eq!(
        activation_pda,
        (
            Pubkey::from_str(&fixture.proposal_pdas.bootstrap_activation_proposal.address)
                .expect("activation PDA must be base58"),
            fixture.proposal_pdas.bootstrap_activation_proposal.bump,
        ),
        "council-versioned activation PDA drifted"
    );

    let expected = [
        (
            "ProgramDataCapacityPolicyV1",
            ProgramDataCapacityPolicyV1::LEN,
            PROGRAMDATA_CAPACITY_POLICY_V1_DISCRIMINATOR,
            PROGRAMDATA_CAPACITY_POLICY_V1_RESERVED_LEN,
        ),
        (
            "ControllerReleaseCommitmentV1",
            ControllerReleaseCommitmentV1::LEN,
            CONTROLLER_RELEASE_COMMITMENT_V1_DISCRIMINATOR,
            CONTROLLER_RELEASE_COMMITMENT_V1_RESERVED_LEN,
        ),
        (
            "ProgramDataObservationV1",
            ProgramDataObservationV1::LEN,
            PROGRAMDATA_OBSERVATION_V1_DISCRIMINATOR,
            PROGRAMDATA_OBSERVATION_V1_RESERVED_LEN,
        ),
        (
            "CurrentDeploymentStateV1",
            CurrentDeploymentStateV1::LEN,
            CURRENT_DEPLOYMENT_STATE_V1_DISCRIMINATOR,
            CURRENT_DEPLOYMENT_STATE_V1_RESERVED_LEN,
        ),
        (
            "ControllerImmutabilityReceiptV1",
            ControllerImmutabilityReceiptV1::LEN,
            CONTROLLER_IMMUTABILITY_RECEIPT_V1_DISCRIMINATOR,
            CONTROLLER_IMMUTABILITY_RECEIPT_V1_RESERVED_LEN,
        ),
        (
            "TargetAuthorityHandoffProposalV1",
            TargetAuthorityHandoffProposalV1::LEN,
            TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_DISCRIMINATOR,
            TARGET_AUTHORITY_HANDOFF_PROPOSAL_V1_RESERVED_LEN,
        ),
        (
            "TargetAuthorityHandoffReceiptV1",
            TargetAuthorityHandoffReceiptV1::LEN,
            TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_DISCRIMINATOR,
            TARGET_AUTHORITY_HANDOFF_RECEIPT_V1_RESERVED_LEN,
        ),
        (
            "BootstrapActivationProposalV1",
            BootstrapActivationProposalV1::LEN,
            BOOTSTRAP_ACTIVATION_PROPOSAL_V1_DISCRIMINATOR,
            BOOTSTRAP_ACTIVATION_PROPOSAL_V1_RESERVED_LEN,
        ),
        (
            "BootstrapActivationReceiptV1",
            BootstrapActivationReceiptV1::LEN,
            BOOTSTRAP_ACTIVATION_RECEIPT_V1_DISCRIMINATOR,
            BOOTSTRAP_ACTIVATION_RECEIPT_V1_RESERVED_LEN,
        ),
    ];
    assert_eq!(
        fixture
            .entries
            .iter()
            .map(|entry| entry.account_type.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        expected.len(),
        "fixture account types must be unique"
    );

    for (entry, (name, length, discriminator, reserved_length)) in
        fixture.entries.iter().zip(expected)
    {
        let bytes = decode_base64(&entry.data_base64);
        assert_eq!(entry.account_type, name);
        assert_eq!(entry.length, length);
        assert_eq!(entry.discriminator_ascii.as_bytes(), discriminator);
        assert_eq!(entry.reserved_length, reserved_length);
        assert_eq!(bytes.len(), length, "{name} fixture length drifted");
        assert_eq!(&bytes[..8], discriminator, "{name} discriminator drifted");
        assert!(
            bytes[length - reserved_length..]
                .iter()
                .all(|byte| *byte == 0),
            "{name} reserved bytes must all be zero"
        );

        match name {
            "ProgramDataCapacityPolicyV1" => verify_account!(
                entry,
                &bytes,
                ProgramDataCapacityPolicyV1,
                policy_digest,
                compute_capacity_policy_digest_v1,
                validate_capacity_policy_digest_v1
            ),
            "ControllerReleaseCommitmentV1" => verify_account!(
                entry,
                &bytes,
                ControllerReleaseCommitmentV1,
                release_digest,
                compute_controller_release_digest_v1,
                validate_controller_release_digest_v1
            ),
            "ProgramDataObservationV1" => verify_account!(
                entry,
                &bytes,
                ProgramDataObservationV1,
                observation_digest,
                compute_programdata_observation_digest_v1,
                validate_programdata_observation_digest_v1
            ),
            "CurrentDeploymentStateV1" => verify_account!(
                entry,
                &bytes,
                CurrentDeploymentStateV1,
                deployment_digest,
                compute_current_deployment_digest_v1,
                validate_current_deployment_digest_v1
            ),
            "ControllerImmutabilityReceiptV1" => verify_account!(
                entry,
                &bytes,
                ControllerImmutabilityReceiptV1,
                receipt_digest,
                compute_controller_immutability_receipt_digest_v1,
                validate_controller_immutability_receipt_digest_v1
            ),
            "TargetAuthorityHandoffProposalV1" => {
                let value = TargetAuthorityHandoffProposalV1::from_bytes_strict(&bytes)
                    .expect("strict Rust decoding must succeed");
                assert_eq!(value.bump, handoff_pda.1);
                verify_account!(
                    entry,
                    &bytes,
                    TargetAuthorityHandoffProposalV1,
                    proposal_digest,
                    compute_target_handoff_proposal_digest_v1,
                    validate_target_handoff_proposal_digest_v1
                );
            }
            "TargetAuthorityHandoffReceiptV1" => {
                let value = TargetAuthorityHandoffReceiptV1::from_bytes_strict(&bytes)
                    .expect("strict Rust decoding must succeed");
                assert_eq!(value.proposal, handoff_pda.0);
                verify_account!(
                    entry,
                    &bytes,
                    TargetAuthorityHandoffReceiptV1,
                    receipt_digest,
                    compute_target_handoff_receipt_digest_v1,
                    validate_target_handoff_receipt_digest_v1
                );
            }
            "BootstrapActivationProposalV1" => {
                let value = BootstrapActivationProposalV1::from_bytes_strict(&bytes)
                    .expect("strict Rust decoding must succeed");
                assert_eq!(value.bump, activation_pda.1);
                verify_account!(
                    entry,
                    &bytes,
                    BootstrapActivationProposalV1,
                    proposal_digest,
                    compute_bootstrap_activation_proposal_digest_v1,
                    validate_bootstrap_activation_proposal_digest_v1
                );
            }
            "BootstrapActivationReceiptV1" => {
                let value = BootstrapActivationReceiptV1::from_bytes_strict(&bytes)
                    .expect("strict Rust decoding must succeed");
                assert_eq!(value.proposal, activation_pda.0);
                assert_eq!(
                    hex(&compute_bootstrap_activation_receipt_plan_digest_v1(&value)
                        .expect("activation receipt plan digest")),
                    fixture.activation_plan_digests.receipt_plan_digest_hex
                );
                verify_account!(
                    entry,
                    &bytes,
                    BootstrapActivationReceiptV1,
                    receipt_digest,
                    compute_bootstrap_activation_receipt_digest_v1,
                    validate_bootstrap_activation_receipt_digest_v1
                );
            }
            unexpected => panic!("unexpected ceremony fixture account {unexpected}"),
        }
    }

    let deployment_entry = fixture
        .entries
        .iter()
        .find(|entry| entry.account_type == "CurrentDeploymentStateV1")
        .expect("deployment fixture entry");
    let deployment =
        CurrentDeploymentStateV1::from_bytes_strict(&decode_base64(&deployment_entry.data_base64))
            .expect("strict deployment decoding");
    assert_eq!(
        hex(
            &compute_bootstrap_activation_deployment_plan_digest_v1(&deployment)
                .expect("activation deployment plan digest")
        ),
        fixture.activation_plan_digests.deployment_plan_digest_hex
    );
}
