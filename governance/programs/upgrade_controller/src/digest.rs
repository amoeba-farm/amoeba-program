use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    state::{OptionalPubkeyV1, ProposalClassV1, UpgradeProposalV1},
    GovernanceResult,
};

pub const PROPOSAL_DIGEST_DOMAIN_V1: &[u8] = b"AMOEBA_UPGRADE_PROPOSAL_V1";
pub const PROPOSAL_DIGEST_MATERIAL_LEN: usize = 1084;
pub const PROPOSAL_DIGEST_PREIMAGE_LEN: usize =
    PROPOSAL_DIGEST_DOMAIN_V1.len() + PROPOSAL_DIGEST_MATERIAL_LEN;

/// Returns the exact fixed-width bytes covered by council approvals and votes.
/// Mutable lifecycle state, approval accumulators, reason codes, bump/header,
/// stored digest, and reserved bytes are intentionally excluded.
pub fn canonical_proposal_digest_material(
    proposal: &UpgradeProposalV1,
) -> GovernanceResult<[u8; PROPOSAL_DIGEST_MATERIAL_LEN]> {
    proposal.rollback_proposal.validate()?;
    proposal.rollback_buffer.validate()?;

    let mut out = [0u8; PROPOSAL_DIGEST_MATERIAL_LEN];
    let mut offset = 0usize;
    put_bytes(&mut out, &mut offset, &proposal.cluster_domain);
    put_pubkey(&mut out, &mut offset, &proposal.controller_program);
    put_pubkey(&mut out, &mut offset, &proposal.controller_config);
    put_pubkey(&mut out, &mut offset, &proposal.protocol_gate);
    put_u64(&mut out, &mut offset, proposal.policy_version);
    put_bytes(&mut out, &mut offset, &proposal.policy_hash);
    put_pubkey(&mut out, &mut offset, &proposal.target_program);
    put_pubkey(&mut out, &mut offset, &proposal.target_programdata);
    put_pubkey(&mut out, &mut offset, &proposal.upgradeable_loader);
    put_pubkey(&mut out, &mut offset, &proposal.authority_pda);
    put_pubkey(&mut out, &mut offset, &proposal.canonical_spill_treasury);
    put_u64(&mut out, &mut offset, proposal.proposal_id);
    put_u64(&mut out, &mut offset, proposal.target_nonce);
    put_proposal_class(&mut out, &mut offset, proposal.proposal_class);
    put_u64(&mut out, &mut offset, proposal.council_version);
    put_bytes(&mut out, &mut offset, &proposal.council_hash);
    put_u64(&mut out, &mut offset, proposal.creation_gate_epoch);
    put_u64(&mut out, &mut offset, proposal.freeze_gate_epoch);
    put_pubkey(&mut out, &mut offset, &proposal.buffer_pubkey);
    put_pubkey(&mut out, &mut offset, &proposal.buffer_loader_owner);
    put_pubkey(&mut out, &mut offset, &proposal.buffer_authority);
    put_u64(&mut out, &mut offset, proposal.artifact_length);
    put_bytes(&mut out, &mut offset, &proposal.artifact_sha256);
    put_bytes(&mut out, &mut offset, &proposal.source_commit_hash);
    put_bytes(&mut out, &mut offset, &proposal.source_tree_hash);
    put_bytes(&mut out, &mut offset, &proposal.build_input_inventory_hash);
    put_bytes(
        &mut out,
        &mut offset,
        &proposal.reproducible_build_receipt_hash,
    );
    put_bytes(&mut out, &mut offset, &proposal.package_receipt_hash);
    put_bytes(&mut out, &mut offset, &proposal.release_intent_hash);
    put_bytes(
        &mut out,
        &mut offset,
        &proposal.current_deployed_payload_hash,
    );
    put_bytes(
        &mut out,
        &mut offset,
        &proposal.current_raw_programdata_hash,
    );
    put_u64(&mut out, &mut offset, proposal.deployed_slot);
    put_u64(&mut out, &mut offset, proposal.current_capacity);
    put_u64(&mut out, &mut offset, proposal.extension_delta);
    put_u64(&mut out, &mut offset, proposal.expected_post_capacity);
    put_pubkey(&mut out, &mut offset, &proposal.prestate_checkpoint);
    put_pubkey(
        &mut out,
        &mut offset,
        &proposal.required_poststate_checkpoint,
    );
    put_optional_pubkey(&mut out, &mut offset, &proposal.rollback_proposal);
    put_optional_pubkey(&mut out, &mut offset, &proposal.rollback_buffer);
    put_bytes(&mut out, &mut offset, &proposal.rollback_artifact_hash);
    put_pubkey(&mut out, &mut offset, &proposal.vote_program);
    put_pubkey(&mut out, &mut offset, &proposal.vote_result_pda);
    put_u8(&mut out, &mut offset, proposal.vote_requirement as u8);
    put_u64(&mut out, &mut offset, proposal.review_start_slot);
    put_u64(&mut out, &mut offset, proposal.review_end_slot);
    put_u64(&mut out, &mut offset, proposal.not_before_slot);
    put_u64(&mut out, &mut offset, proposal.expiry_slot);
    assert_eq!(offset, PROPOSAL_DIGEST_MATERIAL_LEN);
    Ok(out)
}

pub fn compute_proposal_digest(proposal: &UpgradeProposalV1) -> GovernanceResult<[u8; 32]> {
    let material = canonical_proposal_digest_material(proposal)?;
    Ok(hashv(&[PROPOSAL_DIGEST_DOMAIN_V1, &material]).to_bytes())
}

fn put_proposal_class<const N: usize>(
    out: &mut [u8; N],
    offset: &mut usize,
    value: ProposalClassV1,
) {
    put_u8(out, offset, value as u8);
}

fn put_optional_pubkey<const N: usize>(
    out: &mut [u8; N],
    offset: &mut usize,
    value: &OptionalPubkeyV1,
) {
    put_u8(out, offset, u8::from(value.present));
    put_pubkey(out, offset, &value.value);
}

fn put_u8<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: u8) {
    out[*offset] = value;
    *offset += 1;
}

fn put_u64<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: u64) {
    out[*offset..*offset + 8].copy_from_slice(&value.to_le_bytes());
    *offset += 8;
}

fn put_pubkey<const N: usize>(out: &mut [u8; N], offset: &mut usize, value: &Pubkey) {
    put_bytes(out, offset, &value.to_bytes());
}

fn put_bytes<const N: usize, const M: usize>(
    out: &mut [u8; N],
    offset: &mut usize,
    value: &[u8; M],
) {
    out[*offset..*offset + M].copy_from_slice(value);
    *offset += M;
}
