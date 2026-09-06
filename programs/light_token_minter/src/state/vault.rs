use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultConfig {
    pub is_initialized: bool,
    pub bump: u8,
    pub admin: Pubkey,
    pub oracle_authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub vault_token_account: Pubkey,
    pub paused: bool,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
}

impl VaultConfig {
    pub const LEN: usize = 1 + 1 + 32 + 32 + 32 + 32 + 1 + 3 + 1;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"VCF";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementSignerRegistry {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub current_set: Pubkey,
    pub current_version: u64,
    pub pending_set: Pubkey,
    pub pending_version: u64,
    pub recovery_authority: Pubkey,
    /// Monotonic proposal generation. Every proposal consumes the next value so cancellation
    /// invalidates the prior public Ed25519 attestations.
    pub proposal_nonce: u64,
}

#[allow(clippy::derivable_impls)]
impl Default for SettlementSignerRegistry {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            current_set: Pubkey::default(),
            current_version: 0,
            pending_set: Pubkey::default(),
            pending_version: 0,
            recovery_authority: Pubkey::default(),
            proposal_nonce: 0,
        }
    }
}

impl SettlementSignerRegistry {
    pub const LEN: usize = 128;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"SRG";
    pub const ACCOUNT_VERSION: u8 = 1;

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementSignerSet {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub registry: Pubkey,
    pub version: u64,
    pub threshold: u8,
    pub signer_count: u8,
    pub signers: [Pubkey; MAX_SETTLEMENT_SIGNER_COUNT],
    pub set_hash: [u8; 32],
    pub rotation_delay_slots: u64,
    pub proposed_slot: u64,
    pub activate_after_slot: u64,
    pub emergency: bool,
    pub proposer: Pubkey,
}

impl Default for SettlementSignerSet {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            account_discriminator: [0; 3],
            account_version: 0,
            registry: Pubkey::default(),
            version: 0,
            threshold: 0,
            signer_count: 0,
            signers: [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT],
            set_hash: [0; 32],
            rotation_delay_slots: 0,
            proposed_slot: 0,
            activate_after_slot: 0,
            emergency: false,
            proposer: Pubkey::default(),
        }
    }
}

impl SettlementSignerSet {
    pub const LEN: usize = 416;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"SSS";
    pub const ACCOUNT_VERSION: u8 = 1;
    pub const HASH_DOMAIN: &'static [u8] = b"ameba_settlement_signer_set_v1";

    pub fn has_current_layout(&self) -> bool {
        self.account_discriminator == Self::ACCOUNT_DISCRIMINATOR
            && self.account_version == Self::ACCOUNT_VERSION
    }

    pub fn compute_set_hash(&self) -> [u8; 32] {
        let mut bytes = Vec::with_capacity(
            Self::HASH_DOMAIN.len() + 32 + 8 + 2 + (MAX_SETTLEMENT_SIGNER_COUNT * 32),
        );
        bytes.extend_from_slice(Self::HASH_DOMAIN);
        bytes.extend_from_slice(self.registry.as_ref());
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.push(self.threshold);
        bytes.push(self.signer_count);
        for signer in self.signers.iter().take(usize::from(self.signer_count)) {
            bytes.extend_from_slice(signer.as_ref());
        }
        hashv(&[bytes.as_slice()]).to_bytes()
    }
}

pub fn derive_settlement_signer_registry_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            SETTLEMENT_SIGNER_REGISTRY_PDA_SEED,
        ],
        program_id,
    )
}

pub fn derive_settlement_signer_set_pda(program_id: &Pubkey, version: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            SETTLEMENT_SIGNER_SET_PDA_SEED,
            &version.to_le_bytes(),
        ],
        program_id,
    )
}
