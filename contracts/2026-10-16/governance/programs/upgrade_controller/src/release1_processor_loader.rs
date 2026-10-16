//! Closed Release 1 Loader-v3 extension, upgrade, and deployed-byte verification.
//!
//! Every CPI in this module is constructed by the pinned Loader-v3 interface.
//! There is no caller-selected program, instruction data, or account vector.

use solana_loader_v3_interface::instruction::{extend_program_checked, upgrade};
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
    artifact_merkle::verify_artifact_chunk_proof,
    instruction::{
        EnvelopeExpectationV1, ExecuteUpgradeV1, ExtendTargetV1, FinalizeProgramDataVerificationV1,
        ProgramDataChunkPhaseV1, ProposalExpectationV2, VerifyProgramDataChunkV1,
        MAX_ENVELOPE_COMPUTE_UNIT_LIMIT_V1, MAX_ENVELOPE_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS_V1,
    },
    pda::{
        derive_authority_pda, derive_buffer_check_pda, derive_checkpoint_pda,
        derive_controller_config_pda, derive_gate_pda, derive_programdata_check_pda,
        derive_proposal_pda, derive_upgradeable_programdata_address, AUTHORITY_SEED,
        PROGRAMDATA_CHECK_SEED, UPGRADEABLE_LOADER_ID, UPGRADE_SEED_DOMAIN_V1,
    },
    policy::validate_policy_against_config,
    release1_account_io::{
        create_fixed_pda_account, encode_fixed_account, load_fixed_controller_account,
        require_distinct_accounts, validate_exact_privileges,
    },
    release1_digest::{validate_proposal_digest_v2, validate_state_checkpoint_digest_v1},
    release1_loader_accounts::{
        loader_account_data_hash, parse_upgradeable_buffer, validate_program_programdata_linkage,
        LOADER_BUFFER_METADATA_LEN,
    },
    release1_state::{
        BufferVerificationStatusV1, BufferVerificationV1, ProgramDataVerificationStatusV1,
        ProgramDataVerificationV1, ProposalStateV2, StateCheckpointPhaseV1, StateCheckpointV1,
        UpgradeProposalV2, MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
        PROGRAMDATA_VERIFICATION_V1_DISCRIMINATOR, PROGRAMDATA_VERIFICATION_V1_RESERVED_LEN,
        RELEASE1_ACCOUNT_VERSION_V1, VERIFICATION_BITMAP_BYTES_V1,
    },
    state::{
        CheckpointPhaseV1, ControllerConfigV1, GateStatusV1, GovernancePolicyV1, ProposalClassV1,
        ProtocolGateV1,
    },
    GovernanceError,
};

#[cfg(test)]
mod tests;

mod account_context;
mod buffer;
mod execution;
mod loader_contract;
mod programdata;
mod rollback;
mod timing;
mod verification;

use account_context::{
    commit_one_fixed_account, commit_three_fixed_accounts, commit_two_fixed_accounts,
    load_accepted_prestate, load_config, load_gate, load_policy, load_proposal,
    validate_frozen_expectation,
};
use buffer::{load_buffer_verification, validate_live_sealed_buffer};
pub use execution::{process_execute_upgrade_v1, process_extend_target_v1};
use loader_contract::{
    validate_canonical_envelope, validate_extend_cpi_shape, validate_program_and_sysvar_ids,
    validate_target_account_keys, validate_upgrade_cpi_shape,
};
use programdata::{
    expected_unextended_raw_programdata_hash, require_raw_programdata_hash,
    validate_zero_appended_extension,
};
use rollback::validate_reciprocal_counterpart;
#[cfg(test)]
use rollback::{
    require_primary_execution_rollback_runway, validate_reciprocal_checkpoint_commitments,
};
#[cfg(test)]
use timing::validate_post_execution_frozen_slot;
use timing::{
    current_frozen_slot, current_post_execution_frozen_slot, require_extension_execution_runway,
    verify_exact_proposal_timing,
};
use verification::chunk_count_allow_empty;
#[cfg(test)]
use verification::{exact_region_chunk, mark_bitmap_bit};
pub use verification::{
    process_finalize_programdata_verification_v1, process_verify_programdata_chunk_v1,
};
