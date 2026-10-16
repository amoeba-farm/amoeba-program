use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use serde_json::Value;
use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    gate_abi::{
        decode_protocol_gate_v1, encode_protocol_gate_v1, envelope_instruction_data_v1,
        governance_tail_offset, protocol_gate_offset, strip_governance_tail_v1,
        GovernanceInstructionTailV1, GOVERNANCE_TAIL_LEN,
    },
    pda::{derive_controller_config_pda, derive_gate_pda, derive_upgradeable_programdata_address},
    state::{GateStatusV1, ProtocolGateV1},
};

const FIXTURE_TEXT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/spread_gate_bridge_v1.json"
));

fn fixture() -> Value {
    serde_json::from_str(FIXTURE_TEXT).expect("valid frozen Spread gate bridge fixture")
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex length");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => panic!("invalid hex byte"),
            };
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect()
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
fn phase3_bridge_fixture_matches_rust_derivations_and_exact_bytes() {
    let fixture = fixture();
    assert_eq!(fixture["schemaVersion"].as_u64(), Some(1));
    assert!(fixture["warning"]
        .as_str()
        .unwrap()
        .contains("SYNTHETIC NON-PRODUCTION"));

    let controller = Pubkey::from_str(fixture["controllerProgram"].as_str().unwrap()).unwrap();
    let target = Pubkey::from_str(fixture["targetProgram"].as_str().unwrap()).unwrap();
    let expected_programdata = &fixture["targetProgramdata"];
    let expected_config = &fixture["pdas"]["controllerConfig"];
    let expected_gate = &fixture["pdas"]["protocolGate"];
    let programdata = derive_upgradeable_programdata_address(&target);
    let config = derive_controller_config_pda(&controller, &target);
    let gate_pda = derive_gate_pda(&controller, &target);

    assert_eq!(programdata.0.to_string(), expected_programdata["address"]);
    assert_eq!(u64::from(programdata.1), expected_programdata["bump"]);
    assert_eq!(config.0.to_string(), expected_config["address"]);
    assert_eq!(u64::from(config.1), expected_config["bump"]);
    assert_eq!(gate_pda.0.to_string(), expected_gate["address"]);
    assert_eq!(u64::from(gate_pda.1), expected_gate["bump"]);

    let gate_bytes = decode_hex(fixture["gateAccount"]["hex"].as_str().unwrap());
    assert_eq!(gate_bytes.len(), ProtocolGateV1::LEN);
    assert_eq!(fixture["gateAccount"]["length"].as_u64(), Some(192));
    let gate = decode_protocol_gate_v1(&gate_bytes).unwrap();
    let decoded = &fixture["gateAccount"]["decoded"];
    assert_eq!(gate.discriminator, *b"AGVGAT01");
    assert_eq!(gate.version, decoded["accountVersion"]);
    assert_eq!(u64::from(gate.bump), decoded["bump"]);
    assert_eq!(gate.initialized, decoded["initialized"]);
    assert_eq!(u64::from(gate.status as u8), decoded["status"]);
    assert_eq!(gate.status, GateStatusV1::Active);
    assert_eq!(
        gate.controller_config.to_string(),
        decoded["controllerConfig"]
    );
    assert_eq!(gate.target_program.to_string(), decoded["targetProgram"]);
    assert_eq!(
        gate.target_programdata.to_string(),
        decoded["targetProgramdata"]
    );
    assert_eq!(gate.epoch.to_string(), decoded["epoch"]);
    assert_eq!(gate.active_proposal.to_string(), decoded["activeProposal"]);
    assert_eq!(gate.freeze_slot.to_string(), decoded["freezeSlot"]);
    assert_eq!(
        u64::from(gate.freeze_reason_code),
        decoded["freezeReasonCode"]
    );
    assert_eq!(
        gate.last_completed_proposal.to_string(),
        decoded["lastCompletedProposal"]
    );
    assert_eq!(hex(&gate.reserved), decoded["reservedHex"]);
    assert_eq!(gate.try_to_vec().unwrap(), gate_bytes);
    assert_eq!(ProtocolGateV1::try_from_slice(&gate_bytes).unwrap(), gate);
    assert_eq!(
        encode_protocol_gate_v1(&gate).unwrap().as_slice(),
        gate_bytes
    );

    let tail_bytes = decode_hex(fixture["governanceTail"]["hex"].as_str().unwrap());
    assert_eq!(tail_bytes.len(), GOVERNANCE_TAIL_LEN);
    assert_eq!(fixture["governanceTail"]["length"].as_u64(), Some(16));
    assert_eq!(hex(&tail_bytes), "41475631010000002900000000000000");
    let tail = GovernanceInstructionTailV1::decode(&tail_bytes).unwrap();
    assert_eq!(tail, GovernanceInstructionTailV1::new(41));
    assert_eq!(tail.encode().unwrap().as_slice(), tail_bytes);

    let bridge = &fixture["instructionBridge"];
    let legacy = decode_hex(bridge["legacyInstruction"]["hex"].as_str().unwrap());
    let enveloped = decode_hex(bridge["envelopedInstruction"]["hex"].as_str().unwrap());
    let stripped_expected =
        decode_hex(bridge["strippedLegacyInstruction"]["hex"].as_str().unwrap());
    assert_eq!(
        envelope_instruction_data_v1(&legacy, &tail).unwrap(),
        enveloped
    );
    let (stripped, stripped_tail) = strip_governance_tail_v1(&enveloped).unwrap();
    assert_eq!(stripped, stripped_expected);
    assert_eq!(stripped, legacy);
    assert_eq!(stripped_tail, tail);
    assert_eq!(bridge["strippedMatchesLegacy"].as_bool(), Some(true));
}

#[test]
fn fixture_self_hash_covers_placeholder_form_of_exact_file_bytes() {
    let fixture = fixture();
    let expected = fixture["fixtureSha256"].as_str().unwrap();
    assert_eq!(expected.len(), 64);
    let needle = format!("\"fixtureSha256\": \"{expected}\"");
    assert_eq!(FIXTURE_TEXT.matches(&needle).count(), 1);
    let placeholder = format!("\"fixtureSha256\": \"{}\"", "0".repeat(64));
    let material = FIXTURE_TEXT.replacen(&needle, &placeholder, 1);
    assert_eq!(hex(hashv(&[material.as_bytes()]).as_ref()), expected);
}

#[test]
fn phase3_offsets_and_rejection_matrix_are_fail_closed() {
    assert_eq!(protocol_gate_offset::DISCRIMINATOR, 0);
    assert_eq!(protocol_gate_offset::VERSION, 8);
    assert_eq!(protocol_gate_offset::BUMP, 9);
    assert_eq!(protocol_gate_offset::INITIALIZED, 10);
    assert_eq!(protocol_gate_offset::STATUS, 11);
    assert_eq!(protocol_gate_offset::CONTROLLER_CONFIG, 12);
    assert_eq!(protocol_gate_offset::TARGET_PROGRAM, 44);
    assert_eq!(protocol_gate_offset::TARGET_PROGRAMDATA, 76);
    assert_eq!(protocol_gate_offset::EPOCH, 108);
    assert_eq!(protocol_gate_offset::ACTIVE_PROPOSAL, 116);
    assert_eq!(protocol_gate_offset::FREEZE_SLOT, 148);
    assert_eq!(protocol_gate_offset::FREEZE_REASON_CODE, 156);
    assert_eq!(protocol_gate_offset::LAST_COMPLETED_PROPOSAL, 158);
    assert_eq!(protocol_gate_offset::RESERVED, 190);
    assert_eq!(governance_tail_offset::MAGIC, 0);
    assert_eq!(governance_tail_offset::VERSION, 4);
    assert_eq!(governance_tail_offset::RESERVED, 5);
    assert_eq!(governance_tail_offset::EXPECTED_EPOCH, 8);

    let fixture = fixture();
    let canonical = decode_hex(fixture["gateAccount"]["hex"].as_str().unwrap());
    assert!(decode_protocol_gate_v1(&canonical[..191]).is_err());
    let mut trailing = canonical.clone();
    trailing.push(0);
    assert!(decode_protocol_gate_v1(&trailing).is_err());

    for (offset, value) in [
        (protocol_gate_offset::DISCRIMINATOR, b'X'),
        (protocol_gate_offset::VERSION, 2),
        (protocol_gate_offset::INITIALIZED, 0),
        (protocol_gate_offset::INITIALIZED, 2),
        (protocol_gate_offset::STATUS, 3),
        (protocol_gate_offset::RESERVED, 1),
    ] {
        let mut mutated = canonical.clone();
        mutated[offset] = value;
        assert!(
            decode_protocol_gate_v1(&mutated).is_err(),
            "gate mutation at offset {offset} unexpectedly passed"
        );
    }

    for offset in [
        protocol_gate_offset::CONTROLLER_CONFIG,
        protocol_gate_offset::TARGET_PROGRAM,
        protocol_gate_offset::TARGET_PROGRAMDATA,
    ] {
        let mut mutated = canonical.clone();
        mutated[offset..offset + 32].fill(0);
        assert!(decode_protocol_gate_v1(&mutated).is_err());
    }

    let tail = GovernanceInstructionTailV1::new(41).encode().unwrap();
    assert!(GovernanceInstructionTailV1::decode(&tail[..15]).is_err());
    let mut trailing_tail = tail.to_vec();
    trailing_tail.push(0);
    assert!(GovernanceInstructionTailV1::decode(&trailing_tail).is_err());
    for (offset, value) in [
        (governance_tail_offset::MAGIC, b'X'),
        (governance_tail_offset::VERSION, 2),
        (governance_tail_offset::RESERVED, 1),
    ] {
        let mut mutated = tail;
        mutated[offset] = value;
        assert!(GovernanceInstructionTailV1::decode(&mutated).is_err());
    }

    let embedded_magic = b"prefixAGV1but-not-a-final-tail";
    assert!(strip_governance_tail_v1(embedded_magic).is_err());
}

#[test]
fn all_gate_status_shapes_roundtrip_without_variable_width_encoding() {
    let fixture = fixture();
    let bytes = decode_hex(fixture["gateAccount"]["hex"].as_str().unwrap());
    let active = decode_protocol_gate_v1(&bytes).unwrap();

    let mut upgrade_frozen = active.clone();
    upgrade_frozen.status = GateStatusV1::FrozenForUpgrade;
    upgrade_frozen.active_proposal = Pubkey::new_from_array([9; 32]);
    upgrade_frozen.freeze_slot = 500;
    upgrade_frozen.freeze_reason_code = 7;

    let mut emergency_frozen = active.clone();
    emergency_frozen.status = GateStatusV1::EmergencyFrozen;
    emergency_frozen.freeze_slot = 501;
    emergency_frozen.freeze_reason_code = 8;

    for value in [active, upgrade_frozen, emergency_frozen] {
        let encoded = encode_protocol_gate_v1(&value).unwrap();
        assert_eq!(encoded.len(), ProtocolGateV1::LEN);
        assert_eq!(decode_protocol_gate_v1(&encoded).unwrap(), value);
    }
}
