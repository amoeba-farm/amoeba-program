use std::str::FromStr;

use borsh::BorshSerialize;
use serde_json::Value;
use solana_program::pubkey::Pubkey;

use crate::{
    council::{canonical_council_hash_material, compute_council_set_hash},
    digest::{
        canonical_proposal_digest_material, compute_proposal_digest, PROPOSAL_DIGEST_DOMAIN_V1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_council_pda, derive_gate_pda, derive_policy_pda,
        derive_proposal_pda,
    },
    policy::{canonical_policy_hash_material, compute_policy_hash},
    state::{
        CheckpointPhaseV1, ControllerConfigV1, CouncilSeatV1, GovernanceCouncilSetV1,
        GovernancePolicyV1, ProtocolGateV1, UpgradeProposalV1,
    },
};

use super::support::{council, policy, proposal};

fn fixture() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/upgrade_governance_v1.json"
    )))
    .expect("valid frozen fixture")
}

fn assert_pda(actual: (Pubkey, u8), expected: &Value) {
    assert_eq!(actual.0.to_string(), expected["address"].as_str().unwrap());
    assert_eq!(u64::from(actual.1), expected["bump"].as_u64().unwrap());
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[test]
fn all_phase2_pdas_match_frozen_addresses_and_bumps() {
    let fixture = fixture();
    assert_eq!(fixture["schemaVersion"].as_u64(), Some(2));
    let inputs = &fixture["pdaInputs"];
    let expected = &fixture["pdas"];
    let controller = Pubkey::from_str(inputs["controllerProgram"].as_str().unwrap()).unwrap();
    let target = Pubkey::from_str(inputs["targetProgram"].as_str().unwrap()).unwrap();
    let policy_version = inputs["policyVersion"].as_str().unwrap().parse().unwrap();
    let council_version = inputs["councilVersion"].as_str().unwrap().parse().unwrap();
    let proposal_id = inputs["proposalId"].as_str().unwrap().parse().unwrap();
    let proposal_pda = derive_proposal_pda(&controller, &target, proposal_id).0;

    assert_pda(
        derive_controller_config_pda(&controller, &target),
        &expected["controllerConfig"],
    );
    assert_pda(
        derive_authority_pda(&controller, &target),
        &expected["authority"],
    );
    assert_pda(derive_gate_pda(&controller, &target), &expected["gate"]);
    assert_pda(
        derive_policy_pda(&controller, &target, policy_version),
        &expected["policy"],
    );
    assert_pda(
        derive_council_pda(&controller, &target, council_version),
        &expected["council"],
    );
    assert_pda(
        derive_proposal_pda(&controller, &target, proposal_id),
        &expected["proposal"],
    );
    assert_pda(
        derive_checkpoint_pda(&controller, &proposal_pda, CheckpointPhaseV1::Prestate),
        &expected["prestateCheckpoint"],
    );
    assert_pda(
        derive_checkpoint_pda(&controller, &proposal_pda, CheckpointPhaseV1::Poststate),
        &expected["poststateCheckpoint"],
    );
    assert_pda(
        derive_buffer_check_pda(&controller, &proposal_pda),
        &expected["bufferCheck"],
    );
}

#[test]
fn fixed_layouts_and_consensus_hashes_match_frozen_bytes() {
    let fixture = fixture();
    let policy = policy();
    let council = council();
    let seat_bytes = council.seats[0].try_to_vec().unwrap();
    let policy_bytes = policy.try_to_vec().unwrap();
    let council_bytes = council.try_to_vec().unwrap();
    let policy_material = canonical_policy_hash_material(&policy);
    let council_material = canonical_council_hash_material(&council);

    assert_eq!(
        ControllerConfigV1::LEN,
        fixture["accountLengths"]["controllerConfig"]
            .as_u64()
            .unwrap() as usize
    );
    assert_eq!(
        GovernancePolicyV1::LEN,
        fixture["accountLengths"]["governancePolicy"]
            .as_u64()
            .unwrap() as usize
    );
    assert_eq!(
        CouncilSeatV1::LEN,
        fixture["accountLengths"]["councilSeat"].as_u64().unwrap() as usize
    );
    assert_eq!(
        GovernanceCouncilSetV1::LEN,
        fixture["accountLengths"]["governanceCouncilSet"]
            .as_u64()
            .unwrap() as usize
    );
    assert_eq!(
        ProtocolGateV1::LEN,
        fixture["accountLengths"]["protocolGate"].as_u64().unwrap() as usize
    );
    assert_eq!(
        UpgradeProposalV1::LEN,
        fixture["accountLengths"]["upgradeProposal"]
            .as_u64()
            .unwrap() as usize
    );
    assert_eq!(seat_bytes.len(), CouncilSeatV1::LEN);
    assert_eq!(
        hex(&seat_bytes),
        fixture["council"]["firstSeatHex"].as_str().unwrap()
    );
    assert_eq!(policy_bytes.len(), GovernancePolicyV1::LEN);
    assert_eq!(council_bytes.len(), GovernanceCouncilSetV1::LEN);

    assert_eq!(
        policy_material.len() as u64,
        fixture["policy"]["materialLength"].as_u64().unwrap()
    );
    assert_eq!(
        hex(&policy_material),
        fixture["policy"]["materialHex"].as_str().unwrap()
    );
    assert_eq!(
        hex(&compute_policy_hash(&policy)),
        fixture["policy"]["sha256Hex"].as_str().unwrap()
    );
    assert_eq!(
        hex(&policy_bytes),
        fixture["policy"]["accountHex"].as_str().unwrap()
    );

    assert_eq!(
        council_material.len() as u64,
        fixture["council"]["materialLength"].as_u64().unwrap()
    );
    assert_eq!(
        hex(&council_material),
        fixture["council"]["materialHex"].as_str().unwrap()
    );
    assert_eq!(
        hex(&compute_council_set_hash(&council)),
        fixture["council"]["sha256Hex"].as_str().unwrap()
    );
    assert_eq!(
        hex(&council_bytes),
        fixture["council"]["accountHex"].as_str().unwrap()
    );
}

#[test]
fn proposal_material_preimage_and_digest_match_frozen_bytes() {
    let fixture = fixture();
    let expected = &fixture["proposalDigest"];
    let proposal = proposal();
    let material = canonical_proposal_digest_material(&proposal).unwrap();
    let mut preimage = Vec::with_capacity(PROPOSAL_DIGEST_DOMAIN_V1.len() + material.len());
    preimage.extend_from_slice(PROPOSAL_DIGEST_DOMAIN_V1);
    preimage.extend_from_slice(&material);

    assert_eq!(
        hex(&proposal.policy_hash),
        fixture["proposalInputs"]["policyHashHex"].as_str().unwrap()
    );
    assert_eq!(
        hex(&proposal.council_hash),
        fixture["proposalInputs"]["councilHashHex"]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        u64::from(proposal.vote_requirement as u8),
        fixture["proposalInputs"]["voteRequirement"]
            .as_u64()
            .unwrap()
    );
    assert_eq!(
        proposal.vote_program.to_string(),
        fixture["proposalInputs"]["voteProgram"].as_str().unwrap()
    );
    assert_eq!(
        proposal.vote_result_pda.to_string(),
        fixture["proposalInputs"]["voteResultPda"].as_str().unwrap()
    );
    assert_eq!(
        PROPOSAL_DIGEST_DOMAIN_V1,
        expected["domainAscii"].as_str().unwrap().as_bytes()
    );
    assert_eq!(
        material.len() as u64,
        expected["materialLength"].as_u64().unwrap()
    );
    assert_eq!(
        preimage.len() as u64,
        expected["preimageLength"].as_u64().unwrap()
    );
    assert_eq!(hex(&material), expected["materialHex"].as_str().unwrap());
    assert_eq!(hex(&preimage), expected["preimageHex"].as_str().unwrap());
    assert_eq!(
        hex(&compute_proposal_digest(&proposal).unwrap()),
        expected["sha256Hex"].as_str().unwrap()
    );
}
