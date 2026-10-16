//! Release 1 rollback activation, abandoned-buffer close, and governed unfreeze.
//!
//! This module is deliberately a closed verification kernel. It never accepts
//! caller-selected CPI bytes or accounts, derives every Loader-v3 instruction
//! from the pinned interface, and prepares every controller-owned account byte
//! before the first mutation.

use solana_loader_v3_interface::instruction::close;
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    hash::hashv,
    instruction::Instruction,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::{instructions, Sysvar},
};
use solana_sdk_ids::{compute_budget, system_program, sysvar as sysvar_ids};
use solana_system_interface::instruction as system_instruction;

use crate::{
    artifact_merkle::{
        artifact_chunk_count, artifact_chunk_empty_hash, artifact_chunk_leaf_hash,
        artifact_chunk_node_hash, MAX_ARTIFACT_PROOF_DEPTH_V1, MAX_PADDED_ARTIFACT_CHUNKS_V1,
    },
    authorization::validate_seat_authority,
    council::{
        record_seat_approval, validate_council_guardian_separation, validate_council_set,
        VALID_APPROVAL_MASK,
    },
    instruction::{
        ActivateRollbackV1, ApproveUnfreezeV1, CloseAbandonedBufferV1, EnvelopeExpectationV1,
        ExecuteUnfreezeV1, FixedMerkleProofV1, ObserveProgramDataFailureV1, ProposalExpectationV2,
        UnfreezeExpectationV1, MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1,
        MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_council_pda, derive_gate_pda, derive_policy_pda,
        derive_programdata_check_pda, derive_programdata_failure_observation_pda,
        derive_proposal_pda, derive_upgradeable_programdata_address, AUTHORITY_SEED,
        PROGRAMDATA_FAILURE_OBSERVATION_SEED, UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, store_fixed_controller_account, validate_exact_privileges,
    },
    release1_digest::{
        compute_programdata_failure_observation_digest_v1,
        validate_programdata_failure_observation_digest_v1, validate_proposal_digest_v2,
        validate_state_checkpoint_digest_v1,
    },
    release1_loader_accounts::{
        loader_account_data_hash, parse_upgradeable_program, parse_upgradeable_programdata,
        validate_buffer_account, validate_program_programdata_linkage, LOADER_BUFFER_METADATA_LEN,
        LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, ProgramDataFailureObservationV1,
        ProgramDataMismatchClassV1, ProgramDataVerificationStatusV1, ProgramDataVerificationV1,
        ProposalStateV2, StateCheckpointPhaseV1, StateCheckpointV1, UpgradeProposalV2,
        MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_FAILURE_OBSERVATION_V1_DISCRIMINATOR,
        PROGRAMDATA_FAILURE_OBSERVATION_V1_RESERVED_LEN, PROPOSAL_COMPLETED_TERMINAL_REASON_V1,
        PROPOSAL_RETIRED_ROLLBACK_TERMINAL_REASON_V1,
        PROPOSAL_SUPERSEDED_BY_ROLLBACK_TERMINAL_REASON_V1, RELEASE1_ACCOUNT_VERSION_V1,
        RELEASE1_APPROVAL_THRESHOLD, VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, GovernanceCouncilSetV1,
        GovernancePolicyV1, OptionalPubkeyV1, ProposalClassV1, ProtocolGateV1,
    },
    GovernanceError, GovernanceResult,
};

/// A distinct nonzero reason for the continuously frozen handoff from a failed
/// primary proposal to its precommitted rollback.
pub const ROLLBACK_ACTIVATION_FREEZE_REASON_V1: u16 = 3;

/// Zero-tail chunks are deliberately outside the artifact Merkle tree. Their
/// relative index and exact final-partial length are committed under this
/// separate domain.
const PROGRAMDATA_ZERO_TAIL_CHUNK_DOMAIN_V1: &[u8] = b"AMOEBA_PROGRAMDATA_ZERO_TAIL_CHUNK_V1";
const MAX_PROGRAMDATA_ZERO_TAIL_CHUNK_BYTES_V1: usize = 16 * 1024;
const PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1: usize = 1024;
const MAX_PROGRAMDATA_ZERO_HASH_BLOCKS_V1: usize =
    MAX_PROGRAMDATA_ZERO_TAIL_CHUNK_BYTES_V1 / PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1;
static PROGRAMDATA_ZERO_HASH_BLOCK_V1: [u8; PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1] =
    [0; PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1];

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeProgramDataObservationV1 {
    program_owner: Pubkey,
    program_executable: bool,
    program_data_length: u64,
    program_header_present: bool,
    linked_programdata: OptionalPubkeyV1,
    programdata_owner: Pubkey,
    programdata_executable: bool,
    programdata_data_length: u64,
    programdata_header_present: bool,
    programdata_slot: u64,
    raw_hash_complete: bool,
    raw_programdata_hash: [u8; 32],
    capacity: u64,
    authority: OptionalPubkeyV1,
}

#[cfg(test)]
mod tests;

mod account_context;
mod buffer;
mod failure;
mod loader_contract;
mod proofs;
mod rollback;
mod timing;
mod unfreeze;

use account_context::{
    check_proposal_expectation_with_policy, check_proposal_expectation_without_live_council,
    commit_one_fixed_account, commit_three_fixed_accounts, commit_two_fixed_accounts, load_config,
    load_current_council, load_gate, load_policy, load_proposal, require_policy_active,
    require_policy_and_council_active, validate_active_primary_binding,
    validate_frozen_proposal_binding,
};
use buffer::load_buffer_verification_without_live_buffer;
pub use buffer::process_close_abandoned_buffer_v1;
#[cfg(test)]
use buffer::{validate_abandoned_buffer_close_state, validate_close_identities};
pub use failure::process_observe_programdata_failure_v1;
use failure::{
    capture_runtime_observation, linked_value, load_failure_observation,
    load_programdata_verification_for_failure, require_observation_unchanged,
};
use loader_contract::{validate_canonical_envelope, validate_close_cpi_shape};
use proofs::{
    exact_region_chunk, programdata_zero_tail_chunk_hash, programdata_zero_tail_zero_hash,
    require_verified_prefix, validate_expected_leaf_proof,
};
pub use rollback::process_activate_rollback_v1;
#[cfg(test)]
use rollback::{
    require_recoverable_rollback_failure_observation, validate_reciprocal_rollback,
    validate_rejected_checkpoint_finalization, validate_rollback_activation_timing,
};
use timing::verify_exact_proposal_timing;
use unfreeze::validate_live_verified_programdata;
#[cfg(test)]
use unfreeze::{prepare_unfreeze_accumulator, terminalize_unfreeze_pair};
pub use unfreeze::{process_approve_unfreeze_v1, process_execute_unfreeze_v1};
