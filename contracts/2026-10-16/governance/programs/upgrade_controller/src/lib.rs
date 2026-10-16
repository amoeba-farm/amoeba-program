//! Release 1 verification kernel for the independent Amoeba upgrade
//! controller.
//!
//! Initialization, proposal, emergency-freeze, checkpoint, council rotation,
//! sealed-buffer custody, typed Loader-v3 execution, deployed-byte
//! verification, rollback, and separate governed unfreeze are executable
//! through closed account contracts. The historical V1 approval kernel and
//! reserved/unknown instruction tags remain rejected by the dispatcher.

pub mod artifact_merkle;
pub mod authorization;
pub mod council;
pub mod digest;
#[cfg(not(feature = "no-entrypoint"))]
pub mod entrypoint;
pub mod error;
pub mod gate_abi;
pub mod instruction;
pub mod pda;
pub mod policy;
pub mod processor;
pub mod programdata_observation_merkle;
pub mod proposal;
pub mod release1_account_io;
pub mod release1_authority_instruction;
pub mod release1_ceremony_digest;
pub mod release1_ceremony_instruction;
pub mod release1_ceremony_state;
pub mod release1_digest;
pub mod release1_governance_v2;
pub mod release1_governance_v2_authority_processor;
pub mod release1_governance_v2_processor;
pub mod release1_loader_accounts;
// The reference model is deliberately host-only. It is an executable oracle
// for unit/property tests, not part of the controller's on-chain verification
// kernel. Keeping it out of the SBF target also prevents its large, pure model
// action enum from consuming a runtime stack frame.
#[cfg(not(target_os = "solana"))]
pub mod release1_model;
pub mod release1_processor_authority;
pub mod release1_processor_buffer;
pub mod release1_processor_checkpoint;
pub mod release1_processor_initialize;
pub mod release1_processor_initialize_v2;
pub mod release1_processor_loader;
pub mod release1_processor_observation;
pub mod release1_processor_proposal;
pub mod release1_processor_terminal;
pub mod release1_processor_v3_custody;
pub mod release1_processor_v3_lifecycle;
pub mod release1_state;
pub mod release1_v3_custody_instruction;
pub mod release1_v3_digest;
pub mod release1_v3_instruction;
pub mod release1_v3_state;
pub mod state;

pub use error::{GovernanceError, GovernanceResult};

#[cfg(test)]
mod tests;
