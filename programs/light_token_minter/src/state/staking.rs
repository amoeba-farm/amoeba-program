use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclePlayerLedger {
    pub is_initialized: bool,
    pub bump: u8,
    pub owner: Pubkey,
    pub major_tokens: u64,
    pub locked_major_tokens: u64,
    pub last_updated_slot: u64,
    pub last_balance_change_slot: u64,
}

impl Default for OraclePlayerLedger {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            owner: Pubkey::default(),
            major_tokens: 0,
            locked_major_tokens: 0,
            last_updated_slot: 0,
            last_balance_change_slot: 0,
        }
    }
}

impl OraclePlayerLedger {
    pub const LEN: usize = 128;
}

/// Singleton accounting state for the transferable sAMBA liquid-staking share mint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleStakingPool {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub major_token_config: Pubkey,
    pub samba_mint: Pubkey,
    pub samba_vote_vault: Pubkey,
    /// AMBA backing the currently circulating sAMBA supply, including funded rewards.
    pub active_amba_backing: u64,
    /// Canonical mirror of the sAMBA SPL mint supply.
    pub samba_supply: u64,
    /// AMBA already reserved by burned sAMBA and waiting out the unbonding period.
    pub pending_unstake_amba: u64,
    /// Saturating lifetime diagnostic; never used to price shares or prove current backing.
    pub total_rewards_funded: u64,
    /// Number of unresolved emergency checkpoints that freeze sAMBA mint/burn supply changes.
    pub governance_lock_count: u64,
    pub last_updated_slot: u64,
    /// Backing abandoned when every outstanding sAMBA share is burned outside this program.
    /// It is excluded from future exchange-rate generations and remains protocol custody.
    pub orphaned_amba_backing: u64,
}

impl OracleStakingPool {
    pub const LEN: usize = 160;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSP";
    pub const ACCOUNT_VERSION: u8 = 1;
}

/// Canonical accounting record for the PDA-owned creator-fee AMBA intake ATA.
///
/// The token account is deliberately separate from the main vault. Anyone may transfer AMBA into
/// the intake ATA, while the program is the only authority able to sweep it into reward custody.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleRewardFunnel {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub major_token_config: Pubkey,
    pub amba_mint: Pubkey,
    pub funnel_token_account: Pubkey,
    /// Lifetime flow counters use u128 so recirculating a finite token supply cannot eventually
    /// overflow and disable an otherwise valid future sweep.
    pub total_swept: u128,
    pub total_game_funded: u128,
    pub total_scramble_funded: u128,
    pub total_challenge_funded: u128,
    pub total_staking_funded: u128,
    pub total_reserve_funded: u128,
    pub last_updated_slot: u64,
}

impl OracleRewardFunnel {
    pub const LEN: usize = 208;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"ORF";
    pub const ACCOUNT_VERSION: u8 = 1;
}

/// Canonical owner-scoped AMBA reservation created when sAMBA is burned for unstaking.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleUnstakeRequest {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub owner: Pubkey,
    pub pending_amba: u64,
    pub claimable_at_ts: u64,
    pub last_updated_slot: u64,
}

impl OracleUnstakeRequest {
    pub const LEN: usize = 96;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OUR";
    pub const ACCOUNT_VERSION: u8 = 1;
}

/// Canonical owner-scoped AMBA principal waiting for delayed sAMBA activation.
///
/// Queued AMBA remains in canonical physical custody but is excluded from active staking backing
/// and reward eligibility. After `activate_after_ts`, activation mints shares at the then-current
/// exchange rate, which prevents a just-in-time queue from capturing earlier funnel rewards.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleStakeActivation {
    pub is_initialized: bool,
    pub bump: u8,
    pub account_discriminator: [u8; 3],
    pub account_version: u8,
    pub owner: Pubkey,
    pub queued_amba: u64,
    pub activate_after_ts: u64,
    pub last_updated_slot: u64,
}

impl OracleStakeActivation {
    pub const LEN: usize = 96;
    pub const ACCOUNT_DISCRIMINATOR: [u8; 3] = *b"OSQ";
    pub const ACCOUNT_VERSION: u8 = 1;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleMajorTokenConfig {
    pub is_initialized: bool,
    pub bump: u8,
    pub mint: Pubkey,
    pub vault_token_account: Pubkey,
}

impl Default for OracleMajorTokenConfig {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            mint: Pubkey::default(),
            vault_token_account: Pubkey::default(),
        }
    }
}

impl OracleMajorTokenConfig {
    pub const LEN: usize = 1 + 1 + 32 + 32;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleTreasuryState {
    pub is_initialized: bool,
    pub bump: u8,
    pub major_tokens: u64,
    pub game_pool: u64,
    pub scramble_pool: u64,
    pub challenge_pool: u64,
    pub reserve_pool: u64,
    pub last_balance_change_slot: u64,
}

impl Default for OracleTreasuryState {
    fn default() -> Self {
        Self {
            is_initialized: false,
            bump: 0,
            major_tokens: 0,
            game_pool: 0,
            scramble_pool: 0,
            challenge_pool: 0,
            reserve_pool: 0,
            last_balance_change_slot: 0,
        }
    }
}

impl OracleTreasuryState {
    pub const LEN: usize = 96;
}
