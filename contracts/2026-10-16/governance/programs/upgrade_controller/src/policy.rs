use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    state::{
        validate_reserved, ControllerConfigV1, GovernanceModeV1, GovernancePolicyV1,
        ProposalClassV1, VoteRequirementV1, ACCOUNT_VERSION_V1, GOVERNANCE_POLICY_DISCRIMINATOR,
    },
    GovernanceError, GovernanceResult,
};

pub const POLICY_HASH_DOMAIN_V1: &[u8] = b"AMOEBA_GOVERNANCE_POLICY_V1";
pub const POLICY_HASH_MATERIAL_LEN: usize = 96;

pub fn canonical_policy_hash_material(
    policy: &GovernancePolicyV1,
) -> [u8; POLICY_HASH_MATERIAL_LEN] {
    let mut out = [0u8; POLICY_HASH_MATERIAL_LEN];
    let mut offset = 0usize;
    put_pubkey(&mut out, &mut offset, &policy.controller_config);
    put_pubkey(&mut out, &mut offset, &policy.target_program);
    put_u64(&mut out, &mut offset, policy.version);
    put_u64(&mut out, &mut offset, policy.activation_slot);
    for value in [
        policy.council_size,
        policy.routine_threshold,
        policy.terminal_threshold,
        policy.governance_mode as u8,
        policy.policy_flags,
    ] {
        put_u8(&mut out, &mut offset, value);
    }
    for value in [
        policy.veto_quorum_bps,
        policy.affirmative_quorum_bps,
        policy.affirmative_approval_bps,
    ] {
        put_u16(&mut out, &mut offset, value);
    }
    for value in [
        policy.routine_requires_vote,
        policy.economic_requires_vote,
        policy.constitutional_requires_vote,
        policy.rotation_requires_vote,
        policy.immutability_requires_vote,
    ] {
        put_u8(&mut out, &mut offset, u8::from(value));
    }
    assert_eq!(offset, POLICY_HASH_MATERIAL_LEN);
    out
}

/// Bootstrap V1 has no token chamber. The class argument remains explicit so a
/// future reviewed policy version can add class-derived vote behavior without
/// accepting a caller-selected mode.
pub fn vote_requirement_for_class(
    policy: &GovernancePolicyV1,
    _class: ProposalClassV1,
) -> GovernanceResult<VoteRequirementV1> {
    validate_policy(policy)?;
    Ok(VoteRequirementV1::None)
}

pub fn compute_policy_hash(policy: &GovernancePolicyV1) -> [u8; 32] {
    let material = canonical_policy_hash_material(policy);
    hashv(&[POLICY_HASH_DOMAIN_V1, &material]).to_bytes()
}

pub fn validate_policy(policy: &GovernancePolicyV1) -> GovernanceResult<()> {
    if policy.discriminator != GOVERNANCE_POLICY_DISCRIMINATOR {
        return Err(GovernanceError::InvalidDiscriminator);
    }
    if policy.account_version != ACCOUNT_VERSION_V1 {
        return Err(GovernanceError::UnsupportedVersion);
    }
    if !policy.initialized {
        return Err(GovernanceError::Uninitialized);
    }
    validate_reserved(&policy.reserved)?;
    require_pubkey(&policy.controller_config)?;
    require_pubkey(&policy.target_program)?;
    if policy.version == 0
        || policy.council_size != 5
        || policy.routine_threshold != 3
        || policy.terminal_threshold != 4
        || policy.governance_mode != GovernanceModeV1::BootstrapCouncilOnly
        || policy.policy_flags != 0
        || policy.veto_quorum_bps != 0
        || policy.affirmative_quorum_bps != 0
        || policy.affirmative_approval_bps != 0
        || policy.routine_requires_vote
        || policy.economic_requires_vote
        || policy.constitutional_requires_vote
        || policy.rotation_requires_vote
        || policy.immutability_requires_vote
    {
        return Err(GovernanceError::InvalidPolicy);
    }
    if compute_policy_hash(policy) != policy.policy_hash {
        return Err(GovernanceError::PolicyHashMismatch);
    }
    Ok(())
}

/// Cross-validates the immutable bootstrap policy against the controller
/// configuration. Token governance cannot be activated through account data in
/// V1: both accounts must carry the canonical disabled representation.
pub fn validate_policy_against_config(
    policy: &GovernancePolicyV1,
    config: &ControllerConfigV1,
) -> GovernanceResult<()> {
    config.validate_static()?;
    validate_policy(policy)?;
    if policy.controller_config == Pubkey::default()
        || policy.target_program != config.target_program
        || policy.version != config.current_policy_version
        || config.token_governance_enabled
        || [
            config.vote_program,
            config.vote_programdata,
            config.vote_config,
            config.vote_mint,
        ]
        .iter()
        .any(|key| *key != Pubkey::default())
    {
        return Err(GovernanceError::InvalidPolicy);
    }
    Ok(())
}

fn require_pubkey(value: &Pubkey) -> GovernanceResult<()> {
    if *value == Pubkey::default() {
        Err(GovernanceError::DefaultPubkey)
    } else {
        Ok(())
    }
}

fn put_u8<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: u8) {
    out[*offset] = value;
    *offset += 1;
}

fn put_u16<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: u16) {
    out[*offset..*offset + 2].copy_from_slice(&value.to_le_bytes());
    *offset += 2;
}

fn put_u64<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: u64) {
    out[*offset..*offset + 8].copy_from_slice(&value.to_le_bytes());
    *offset += 8;
}

fn put_pubkey<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: &Pubkey) {
    out[*offset..*offset + 32].copy_from_slice(value.as_ref());
    *offset += 32;
}
