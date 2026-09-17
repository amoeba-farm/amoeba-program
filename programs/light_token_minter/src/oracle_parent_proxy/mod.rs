//! Prospective CFM parent-registry and arithmetic primitives.
//!
//! These are not transaction handlers. A caller must authenticate the governed
//! month policy, source accounts and accepted observations before using them.
//! No existing month/account is reinterpreted by adding this module.
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, program_error::ProgramError};

use crate::error::VaultError;

mod profile;
pub use profile::*;
pub mod policy;
pub mod september_bootstrap;

pub const REGISTRY_DOMAIN: &[u8] = b"amoeba-cfm-registry-v1";
pub const REGISTRY_HEADER_LEN: usize = 72;
pub const PARENT_ROW_LEN: usize = 107;
pub const MAX_PARENT_ROWS: usize = 22;

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
struct ParentRow {
    source_id: [u8; 32],
    locator_hash: [u8; 32],
    definition_hash: [u8; 32],
    weight_bps: u16,
    terminal_mask: u64,
    direct_assessment: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
struct RegistryWire {
    version: u8,
    product: u8,
    terminal_count: u16,
    terminal_root: [u8; 32],
    publisher: [u8; 32],
    rows: Vec<ParentRow>,
}

/// Private fields prevent callers from mutating a validated commitment.
#[derive(Clone, Debug)]
pub struct CfmRegistry {
    wire: RegistryWire,
    digest: [u8; 32],
}

fn invalid<T>() -> Result<T, ProgramError> {
    Err(VaultError::InvalidOracleState.into())
}

impl CfmRegistry {
    /// Check the fixed header/count/size before Borsh can allocate a vector.
    /// Accept only the complete pinned CFM registry, not caller-supplied roots.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProgramError> {
        if bytes.len() < REGISTRY_HEADER_LEN
            || bytes.len() > REGISTRY_HEADER_LEN + MAX_PARENT_ROWS * PARENT_ROW_LEN
        {
            return invalid();
        }
        let count = u32::from_le_bytes(
            bytes[68..72]
                .try_into()
                .map_err(|_| VaultError::InvalidOracleState)?,
        ) as usize;
        if count == 0
            || count > MAX_PARENT_ROWS
            || bytes.len() != REGISTRY_HEADER_LEN + count * PARENT_ROW_LEN
        {
            return invalid();
        }
        let wire =
            RegistryWire::try_from_slice(bytes).map_err(|_| VaultError::InvalidOracleState)?;
        let (expected_count, expected_terminals, expected_root, expected_digest) =
            match wire.product {
                0 => (13, 52, RAMX_TERMINAL_ROOT, RAMX_REGISTRY_HASH),
                1 => (22, 48, NANDX_TERMINAL_ROOT, NANDX_REGISTRY_HASH),
                _ => return invalid(),
            };
        if wire.version != 1
            || count != expected_count
            || wire.terminal_count != expected_terminals
            || wire.terminal_root != expected_root
            || wire.publisher != CFM_PUBLISHER
        {
            return invalid();
        }
        let mut terminal_mask = 0u64;
        let mut total_weight = 0u16;
        let mut direct_weight = 0u16;
        let mut direct_count = 0u16;
        let mut previous_source = [0; 32];
        let complete_mask = (1u64 << expected_terminals) - 1;
        for row in &wire.rows {
            if row.source_id <= previous_source
                || row.locator_hash == [0; 32]
                || row.definition_hash == [0; 32]
                || row.weight_bps == 0
                || row.weight_bps > 10_000
                || row.direct_assessment != (row.terminal_mask == 0)
                || row.terminal_mask & !complete_mask != 0
                || row.terminal_mask & terminal_mask != 0
                || row.terminal_mask.count_ones() > 5
            {
                return invalid();
            }
            previous_source = row.source_id;
            terminal_mask |= row.terminal_mask;
            total_weight = total_weight
                .checked_add(row.weight_bps)
                .ok_or(VaultError::ArithmeticOverflow)?;
            if row.direct_assessment {
                direct_weight = direct_weight
                    .checked_add(row.weight_bps)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                direct_count += 1;
            }
        }
        let (expected_direct_count, expected_direct_weight) =
            if wire.product == 0 { (0, 0) } else { (4, 2500) };
        if terminal_mask != complete_mask
            || total_weight != 10_000
            || direct_count != expected_direct_count
            || direct_weight != expected_direct_weight
        {
            return invalid();
        }
        let digest = hashv(&[REGISTRY_DOMAIN, bytes]).to_bytes();
        if digest != expected_digest {
            return invalid();
        }
        Ok(Self { wire, digest })
    }

    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub fn product(&self) -> u8 {
        self.wire.product
    }

    pub fn parent_count(&self) -> usize {
        self.wire.rows.len()
    }

    /// Immutable, bounded parent walk in the registry's authenticated order.
    pub fn parent_at(
        &self,
        index: usize,
    ) -> Result<([u8; 32], [u8; 32], [u8; 32], u16), ProgramError> {
        self.wire
            .rows
            .get(index)
            .map(|r| (r.source_id, r.locator_hash, r.definition_hash, r.weight_bps))
            .ok_or_else(|| VaultError::InvalidOracleState.into())
    }

    /// Separate economic-row Merkle root. Never replaces the terminal-product
    /// root committed by this registry and the governed product manifest.
    pub fn economic_root(&self) -> [u8; 32] {
        let width = self.wire.rows.len().next_power_of_two();
        let mut nodes: Vec<[u8; 32]> = (0..width)
            .map(|i| {
                let index = (i as u16).to_le_bytes();
                if let Some(row) = self.wire.rows.get(i) {
                    hashv(&[b"amoeba-oracle-sku-leaf-v1", &index, &row.source_id]).to_bytes()
                } else {
                    hashv(&[b"amoeba-oracle-sku-empty-v1", &index]).to_bytes()
                }
            })
            .collect();
        while nodes.len() > 1 {
            nodes = nodes
                .chunks_exact(2)
                .map(|pair| hashv(&[b"amoeba-oracle-sku-node-v1", &pair[0], &pair[1]]).to_bytes())
                .collect();
        }
        nodes[0]
    }

    pub fn parent_weight(&self, source_id: &[u8; 32]) -> Result<u16, ProgramError> {
        self.wire
            .rows
            .iter()
            .find(|row| row.source_id == *source_id)
            .map(|row| row.weight_bps)
            .ok_or_else(|| VaultError::InvalidOracleState.into())
    }

    /// A child resolves to the parent's single source identity. No child source
    /// or independent observation/reward is created by this association.
    pub fn parent_for_terminal(&self, terminal_index: u16) -> Result<[u8; 32], ProgramError> {
        if terminal_index >= self.wire.terminal_count {
            return invalid();
        }
        self.wire
            .rows
            .iter()
            .find(|row| row.terminal_mask & (1u64 << terminal_index) != 0)
            .map(|row| row.source_id)
            .ok_or_else(|| VaultError::InvalidOracleState.into())
    }

    pub fn definition_for_parent(
        &self,
        source_id: &[u8; 32],
    ) -> Result<([u8; 32], [u8; 32]), ProgramError> {
        self.wire
            .rows
            .iter()
            .find(|row| row.source_id == *source_id)
            .map(|row| (row.locator_hash, row.definition_hash))
            .ok_or_else(|| VaultError::InvalidOracleState.into())
    }

    /// Pure aggregation of a complete authenticated parent walk. `values` must
    /// come from accepted, current source/history state validated by a handler;
    /// supplying values to this helper alone does NOT authenticate evidence.
    /// Aliases are deliberately absent from this interface.
    pub fn aggregate(&self, values: &[ParentValue]) -> Result<ParentAggregate, ProgramError> {
        if values.len() != self.wire.rows.len() {
            return invalid();
        }
        let mut delta_bps = 0i64;
        let mut revision_digest = hashv(&[b"amoeba-cfm-parent-walk-v1", &self.digest]).to_bytes();
        for (row, value) in self.wire.rows.iter().zip(values) {
            if row.source_id != value.source_id
                || row.definition_hash != value.definition_hash
                || value.revision == 0
                || value.accepted_history_hash == [0; 32]
            {
                return invalid();
            }
            let parent_delta = crate::processor::oracle_core::source_delta_bps(
                value.opening_state,
                value.temporal_state,
            )?;
            let contribution =
                crate::processor::bucket_index_contribution_bps(row.weight_bps, parent_delta)?;
            delta_bps = delta_bps
                .checked_add(contribution)
                .ok_or(VaultError::ArithmeticOverflow)?;
            revision_digest = hashv(&[
                b"amoeba-cfm-parent-walk-step-v1",
                &revision_digest,
                &row.source_id,
                &value.revision.to_le_bytes(),
                &value.opening_state.to_le_bytes(),
                &value.temporal_state.to_le_bytes(),
                &value.accepted_history_hash,
            ])
            .to_bytes();
        }
        Ok(ParentAggregate {
            delta_bps,
            revision_digest,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ParentValue {
    pub source_id: [u8; 32],
    pub definition_hash: [u8; 32],
    pub opening_state: u64,
    /// Verified source-local temporal median, not an arbitrary live-page value.
    pub temporal_state: u64,
    pub revision: u64,
    pub accepted_history_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParentAggregate {
    pub delta_bps: i64,
    pub revision_digest: [u8; 32],
}

/// Preserve checked exposure accounting, but do not impose an oracle/publisher
/// exposure budget. Writer collateral and solvency remain independent checks.
pub fn check_publisher_exposure(
    _legacy_cap: u64,
    current_exposure: u64,
    additional_exposure: u64,
) -> Result<(), ProgramError> {
    if additional_exposure == 0 {
        return invalid();
    }
    current_exposure
        .checked_add(additional_exposure)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(())
}
