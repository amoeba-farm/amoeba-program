use solana_program::pubkey::Pubkey;

use crate::{
    council::compute_council_set_hash,
    digest::compute_proposal_digest,
    policy::compute_policy_hash,
    state::{
        ControllerConfigV1, CouncilSeatV1, GateStatusV1, GovernanceCouncilSetV1, GovernanceModeV1,
        GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProposalStateV1, ProtocolGateV1,
        UpgradeProposalV1, VoteRequirementV1, ACCOUNT_VERSION_V1, CONTROLLER_CONFIG_DISCRIMINATOR,
        CONTROLLER_CONFIG_RESERVED_LEN, COUNCIL_SEAT_RESERVED_LEN,
        GOVERNANCE_COUNCIL_DISCRIMINATOR, GOVERNANCE_COUNCIL_RESERVED_LEN,
        GOVERNANCE_POLICY_DISCRIMINATOR, GOVERNANCE_POLICY_RESERVED_LEN,
        PROTOCOL_GATE_DISCRIMINATOR, PROTOCOL_GATE_RESERVED_LEN, UPGRADE_PROPOSAL_DISCRIMINATOR,
        UPGRADE_PROPOSAL_RESERVED_LEN,
    },
};

pub fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}

pub fn policy() -> GovernancePolicyV1 {
    let mut value = GovernancePolicyV1 {
        discriminator: GOVERNANCE_POLICY_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: 201,
        initialized: true,
        controller_config: key(2),
        version: 7,
        target_program: key(3),
        activation_slot: 0x0102_0304_0506_0708,
        council_size: 5,
        routine_threshold: 3,
        terminal_threshold: 4,
        governance_mode: GovernanceModeV1::BootstrapCouncilOnly,
        policy_flags: 0,
        veto_quorum_bps: 0,
        affirmative_quorum_bps: 0,
        affirmative_approval_bps: 0,
        routine_requires_vote: false,
        economic_requires_vote: false,
        constitutional_requires_vote: false,
        rotation_requires_vote: false,
        immutability_requires_vote: false,
        policy_hash: [0; 32],
        reserved: [0; GOVERNANCE_POLICY_RESERVED_LEN],
    };
    value.policy_hash = compute_policy_hash(&value);
    value
}

pub fn council() -> GovernanceCouncilSetV1 {
    let seat = |authority| CouncilSeatV1 {
        seat_authority: key(authority),
        term_start_slot: 10,
        term_end_slot: 10_000,
        active: true,
        reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
    };
    let mut value = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: 202,
        initialized: true,
        controller_config: key(2),
        version: 11,
        target_program: key(3),
        activation_slot: 100,
        deactivation_slot: 0,
        seats: [seat(20), seat(21), seat(22), seat(23), seat(24)],
        routine_threshold: 3,
        terminal_threshold: 4,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    value.set_hash = compute_council_set_hash(&value);
    value
}

pub fn controller_config() -> ControllerConfigV1 {
    ControllerConfigV1 {
        discriminator: CONTROLLER_CONFIG_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump: 203,
        initialized: true,
        cluster_domain: [1; 32],
        target_program: key(3),
        target_programdata: key(4),
        upgradeable_loader: key(5),
        authority_pda: key(6),
        gate_pda: key(7),
        canonical_spill_treasury: key(8),
        current_council_version: 11,
        current_policy_version: 7,
        next_proposal_id: 43,
        target_nonce: 99,
        guardian: key(9),
        vote_program: Pubkey::default(),
        vote_programdata: Pubkey::default(),
        vote_config: Pubkey::default(),
        vote_mint: Pubkey::default(),
        token_governance_enabled: false,
        routine_delay_slots: 100,
        major_delay_slots: 200,
        rollback_delay_slots: 50,
        terminal_delay_slots: 400,
        vote_review_slots: 300,
        proposal_expiry_slots: 2_000,
        policy_flags: 0,
        reserved: [0; CONTROLLER_CONFIG_RESERVED_LEN],
    }
}

pub fn gate() -> ProtocolGateV1 {
    ProtocolGateV1 {
        discriminator: PROTOCOL_GATE_DISCRIMINATOR,
        version: ACCOUNT_VERSION_V1,
        bump: 204,
        initialized: true,
        status: GateStatusV1::Active,
        controller_config: key(2),
        target_program: key(3),
        target_programdata: key(4),
        epoch: 41,
        active_proposal: Pubkey::default(),
        freeze_slot: 0,
        freeze_reason_code: 0,
        last_completed_proposal: key(14),
        reserved: [0; PROTOCOL_GATE_RESERVED_LEN],
    }
}

pub fn proposal() -> UpgradeProposalV1 {
    let mut value = UpgradeProposalV1 {
        discriminator: UPGRADE_PROPOSAL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: 205,
        initialized: true,
        proposal_id: 0x0102_0304_0506_0708,
        target_nonce: 0x1112_1314_1516_1718,
        proposal_class: ProposalClassV1::RoutineUpgrade,
        state: ProposalStateV1::Draft,
        cluster_domain: [1; 32],
        controller_program: key(1),
        controller_config: key(2),
        protocol_gate: key(7),
        policy_version: 7,
        policy_hash: policy().policy_hash,
        council_version: 11,
        council_hash: council().set_hash,
        creation_gate_epoch: 41,
        freeze_gate_epoch: 42,
        target_program: key(3),
        target_programdata: key(4),
        upgradeable_loader: key(5),
        authority_pda: key(6),
        canonical_spill_treasury: key(8),
        buffer_pubkey: key(15),
        buffer_loader_owner: key(5),
        buffer_authority: key(6),
        artifact_length: 1_142_664,
        artifact_sha256: [33; 32],
        source_commit_hash: [34; 32],
        source_tree_hash: [35; 32],
        build_input_inventory_hash: [36; 32],
        reproducible_build_receipt_hash: [37; 32],
        package_receipt_hash: [38; 32],
        release_intent_hash: [39; 32],
        current_deployed_payload_hash: [40; 32],
        current_raw_programdata_hash: [41; 32],
        deployed_slot: 487_702_729,
        current_capacity: 1_241_776,
        extension_delta: 0,
        expected_post_capacity: 1_241_776,
        prestate_checkpoint: key(16),
        required_poststate_checkpoint: key(17),
        rollback_proposal: OptionalPubkeyV1::none(),
        rollback_buffer: OptionalPubkeyV1::none(),
        rollback_artifact_hash: [0; 32],
        vote_requirement: VoteRequirementV1::None,
        vote_program: Pubkey::default(),
        vote_result_pda: Pubkey::default(),
        review_start_slot: 0x2122_2324_2526_2728,
        review_end_slot: 0x3132_3334_3536_3738,
        not_before_slot: 0x4142_4344_4546_4748,
        expiry_slot: 0x5152_5354_5556_5758,
        council_approval_bitset: 0,
        council_approval_count: 0,
        poststate_approval_bitset: 0,
        poststate_approval_count: 0,
        unfreeze_approval_bitset: 0,
        unfreeze_approval_count: 0,
        proposal_digest: [0; 32],
        cancellation_reason_code: 0,
        terminal_reason_code: 0,
        reserved: [0; UPGRADE_PROPOSAL_RESERVED_LEN],
    };
    value.proposal_digest = compute_proposal_digest(&value).expect("canonical proposal");
    value
}
