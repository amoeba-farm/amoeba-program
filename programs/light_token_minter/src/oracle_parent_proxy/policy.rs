//! Exact companion-account codec for the prospective September oracle-only mode.
//! Creation/dispatch integration is intentionally not exposed by this codec.
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

use super::{invalid, CfmRegistry, CFM_PUBLISHER};
use crate::{constants::CURRENT_STATE_NAMESPACE_SEED, error::VaultError};

pub const CFM_MONTH_POLICY_SEED: &[u8] = b"g3-cfm-month-policy-v1";
pub const SEPTEMBER_EXPIRY_TS: u64 = 1_790_812_800;

#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
struct PolicyWire {
    discriminator: [u8; 3],
    version: u8,
    initialized: bool,
    bump: u8,
    market: Pubkey,
    month: Pubkey,
    registry_hash: [u8; 32],
    publisher: [u8; 32],
    expiry_ts: u64,
    /// Reserved legacy wire field. Zero is canonical, not an issuance limit.
    publisher_exposure_cap: u64,
}

/// Validated immutable policy; no public mutation or permissive fallback.
#[derive(Clone, Debug)]
pub struct CfmMonthPolicy(PolicyWire);

impl CfmMonthPolicy {
    pub const LEN: usize = 150;

    /// Called only after the governed handler validates both current role
    /// signers, empty paused market and an absent canonical month.
    pub(crate) fn initial_bytes(
        program: &Pubkey,
        market: &Pubkey,
        month: &Pubkey,
        product: u8,
        underlying: &[u8; 32],
        expiry: u64,
    ) -> Result<Vec<u8>, ProgramError> {
        let (label, registry_hash): (&[u8], [u8; 32]) = match product {
            0 => (b"ram-standardized-baskets", super::RAMX_REGISTRY_HASH),
            1 => (b"nand-standardized-baskets", super::NANDX_REGISTRY_HASH),
            _ => return invalid(),
        };
        let mut expected_underlying = [0; 32];
        expected_underlying[..label.len()].copy_from_slice(label);
        if *underlying != expected_underlying
            || expiry != SEPTEMBER_EXPIRY_TS
            || *market == Pubkey::default()
            || *month == Pubkey::default()
        {
            return invalid();
        }
        let (_, bump) = Self::address(program, month);
        PolicyWire {
            discriminator: *b"CFP",
            version: 1,
            initialized: true,
            bump,
            market: *market,
            month: *month,
            registry_hash,
            publisher: CFM_PUBLISHER,
            expiry_ts: expiry,
            publisher_exposure_cap: 0,
        }
        .try_to_vec()
        .map_err(|_| VaultError::InvalidOracleState.into())
    }

    pub fn address(program: &Pubkey, month: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                CURRENT_STATE_NAMESPACE_SEED,
                CFM_MONTH_POLICY_SEED,
                month.as_ref(),
            ],
            program,
        )
    }

    /// `market` and `month` identities must come from the ordinary canonical
    /// account loaders; this function authenticates the companion itself.
    pub fn load(
        program: &Pubkey,
        info: &AccountInfo,
        market: &Pubkey,
        month: &Pubkey,
        market_expiry: u64,
        registry: &CfmRegistry,
    ) -> Result<Self, ProgramError> {
        let (address, bump) = Self::address(program, month);
        if info.owner != program
            || info.key != &address
            || info.executable
            || info.is_signer
            || info.data_len() != Self::LEN
            || *market == Pubkey::default()
            || *month == Pubkey::default()
        {
            return invalid();
        }
        let data = info.try_borrow_data()?;
        let policy =
            PolicyWire::try_from_slice(&data).map_err(|_| VaultError::InvalidOracleState)?;
        if policy.discriminator != *b"CFP"
            || policy.version != 1
            || !policy.initialized
            || policy.bump != bump
            || policy.market != *market
            || policy.month != *month
            || policy.registry_hash != registry.digest()
            || policy.publisher != CFM_PUBLISHER
            || policy.expiry_ts != SEPTEMBER_EXPIRY_TS
            || market_expiry != policy.expiry_ts
            || policy.publisher_exposure_cap != 0
        {
            return invalid();
        }
        Ok(Self(policy))
    }

    pub fn registry_hash(&self) -> [u8; 32] {
        self.0.registry_hash
    }

    pub fn publisher_exposure_cap(&self) -> u64 {
        u64::MAX
    }
}
