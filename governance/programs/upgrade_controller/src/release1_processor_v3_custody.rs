//! Capacity-safe V3 custody, Loader-v3, and deployed-byte processors.
//!
//! The only CPIs in this module are the four concrete checked Loader-v3
//! operations selected below.  No instruction accepts arbitrary CPI bytes,
//! program IDs, or account vectors.  Capacity-sensitive transitions bind a
//! finalized scalable observation and re-read the live Loader graph before any
//! mutation.

use solana_loader_v3_interface::instruction::{
    close, extend_program_checked, set_buffer_authority_checked, upgrade,
};
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
        artifact_chunk_node_hash, verify_artifact_chunk_proof, MAX_ARTIFACT_PROOF_DEPTH_V1,
        MAX_PADDED_ARTIFACT_CHUNKS_V1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_capacity_policy_pda,
        derive_checkpoint_pda, derive_controller_config_pda, derive_current_deployment_state_pda,
        derive_gate_pda, derive_policy_pda, derive_programdata_check_pda,
        derive_programdata_failure_observation_pda, derive_programdata_observation_pda,
        derive_proposal_pda, derive_upgradeable_programdata_address, AUTHORITY_SEED,
        BUFFER_CHECK_SEED, PROGRAMDATA_CHECK_SEED, PROGRAMDATA_FAILURE_OBSERVATION_SEED,
        UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        load_upgrade_proposal_v3, require_distinct_accounts, validate_exact_privileges,
    },
    release1_authority_instruction::{
        CeremonyEnvelopeV1, MAX_CEREMONY_COMPUTE_UNIT_LIMIT_V1,
        MAX_CEREMONY_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    release1_ceremony_digest::{
        compute_current_deployment_digest_v1, compute_programdata_observation_subject_digest_v1,
        validate_capacity_policy_digest_v1, validate_current_deployment_digest_v1,
        validate_programdata_observation_digest_v1,
    },
    release1_ceremony_state::{
        CurrentDeploymentStateV1, ProgramDataCapacityPolicyV1, ProgramDataObservationPurposeV1,
        ProgramDataObservationStatusV1, ProgramDataObservationV1,
    },
    release1_loader_accounts::{
        parse_upgradeable_buffer, parse_upgradeable_program, parse_upgradeable_programdata,
        validate_buffer_account, validate_program_programdata_linkage, LOADER_BUFFER_METADATA_LEN,
        LOADER_PROGRAMDATA_METADATA_LEN, LOADER_PROGRAM_ACCOUNT_LEN,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, ProposalStateV2, StateCheckpointPhaseV1,
        BUFFER_VERIFICATION_V1_DISCRIMINATOR, BUFFER_VERIFICATION_V1_RESERVED_LEN,
        NO_FAILING_CHUNK_INDEX_V1, RELEASE1_ACCOUNT_VERSION_V1, VERIFICATION_BITMAP_BYTES_V1,
    },
    release1_v3_custody_instruction::{
        ActivateRollbackV2, AdoptBufferV2, CloseAbandonedBufferV2, ExecuteUpgradeV2,
        ExtendTargetV2, FinalizeBufferVerificationV2, VerifyBufferChunkV2,
    },
    release1_v3_digest::{
        compute_programdata_failure_observation_digest_v2,
        compute_programdata_verification_digest_v2,
        validate_programdata_failure_observation_digest_v2,
        validate_programdata_verification_digest_v2, validate_state_checkpoint_digest_v2,
        validate_upgrade_proposal_digest_v3,
    },
    release1_v3_instruction::{
        BindProgramDataVerificationV2, FinalizeProgramDataVerificationV2,
        ObserveProgramDataFailureV2, ProgramDataFailureWitnessV2, ProposalGuardV3,
    },
    release1_v3_state::{
        ProgramDataFailureObservationV2, ProgramDataMismatchClassV2,
        ProgramDataVerificationStatusV2, ProgramDataVerificationV2, StateCheckpointV2,
        UpgradeProposalV3, CAPACITY_SAFE_ACCOUNT_VERSION_V2,
        PROGRAMDATA_FAILURE_OBSERVATION_V2_DIGEST_DOMAIN_ID,
        PROGRAMDATA_FAILURE_OBSERVATION_V2_DISCRIMINATOR,
        PROGRAMDATA_FAILURE_OBSERVATION_V2_RESERVED_LEN,
        PROGRAMDATA_VERIFICATION_V2_DIGEST_DOMAIN_ID, PROGRAMDATA_VERIFICATION_V2_DISCRIMINATOR,
        PROGRAMDATA_VERIFICATION_V2_RESERVED_LEN,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, GovernancePolicyV1, OptionalPubkeyV1,
        ProposalClassV1, ProtocolGateV1,
    },
    GovernanceError,
};

const ROLLBACK_ACTIVATION_FREEZE_REASON_V1: u16 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeProgramDataGraphV2 {
    program_owner: Pubkey,
    program_executable: bool,
    program_data_length: u64,
    program_header_present: bool,
    linked_programdata: OptionalPubkeyV1,
    programdata_owner: Pubkey,
    programdata_executable: bool,
    programdata_data_length: u64,
    programdata_header_present: bool,
    deployed_slot: u64,
    actual_capacity: u64,
    authority: OptionalPubkeyV1,
}

const PROGRAMDATA_ZERO_TAIL_CHUNK_DOMAIN_V1: &[u8] = b"AMOEBA_PROGRAMDATA_ZERO_TAIL_CHUNK_V1";
const PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1: usize = 1_024;
const PROGRAMDATA_ZERO_HASH_BLOCK_V1: [u8; PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1] =
    [0; PROGRAMDATA_ZERO_HASH_BLOCK_BYTES_V1];
const MAX_PROGRAMDATA_ZERO_HASH_BLOCKS_V1: usize = 16;

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
mod tests;

mod account_context;
mod buffer;
mod execution;
mod failure;
mod loader_contract;
mod observations;
mod proofs;
mod rollback;
mod verification;

use account_context::{
    commit_one_fixed_account, commit_two_fixed_accounts, current_frozen_slot,
    current_preexpiry_slot, load_accepted_prestate_at_epoch, load_capacity_policy, load_config,
    load_current_deployment, load_gate, load_policy, load_proposal, require_deployment_guard,
    require_program_ids, validate_proposal_guard,
};
use buffer::{
    load_buffer_verification, load_buffer_verification_without_live_buffer, validate_sealed_buffer,
};
pub use buffer::{
    process_adopt_buffer_v2, process_close_abandoned_buffer_v2,
    process_finalize_buffer_verification_v2, process_verify_buffer_chunk_v2,
};
pub use execution::{process_execute_upgrade_v2, process_extend_target_v2};
pub use failure::process_observe_programdata_failure_witness_v2;
use failure::validate_rollback_activation_mismatch_class;
#[cfg(test)]
use failure::{
    validate_failure_observation_binding, validate_recordable_programdata_mismatch_class,
};
use loader_contract::{
    validate_canonical_envelope, validate_close_cpi_shape, validate_extend_cpi_shape,
    validate_set_buffer_authority_checked_cpi_shape, validate_target_keys,
    validate_upgrade_cpi_shape, validate_zero_appended_extension,
};
use observations::{
    capture_runtime_graph, load_fresh_observation, load_observation_any_status,
    require_observation_guard, runtime_matches_failure_observation, runtime_matches_observation,
};
use proofs::{
    exact_region_chunk, programdata_zero_tail_chunk_hash, programdata_zero_tail_zero_hash,
    validate_expected_leaf_proof,
};
pub use rollback::process_activate_rollback_v2;
use rollback::{
    is_loader_executable_rollback_failure, load_and_revalidate_rollback_failure,
    require_rollback_failure_guard, validate_counterpart,
};
use verification::load_programdata_verification;
pub use verification::{
    process_bind_programdata_verification_v2, process_finalize_programdata_verification_v2,
};
