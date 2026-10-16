//! Localnet dev bootstrap wire (`localnet-dev-authority` only). Each instruction is its tag byte
//! followed by the Borsh payload below; the ordinary instruction enum never decodes these bytes.

use super::*;

/// Create a Game-phase month for a Forming period and write the period Live.
pub const DEV_BOOTSTRAP_MONTH_TAG: u8 = 240;
/// Create one Active source, with its first observation, checkpoint and locator link, under a
/// bootstrapped month.
pub const DEV_BOOTSTRAP_SOURCE_TAG: u8 = 241;
/// Settle a bootstrapped month at or after expiry and write the period Settled. No payload.
pub const DEV_SETTLE_MONTH_TAG: u8 = 242;
/// Create the dev council Config (epoch 1) or rotate its seats (next epoch). The council of the
/// dev profile reads it in place of the governance controller's Config.
pub const DEV_COUNCIL_CONFIG_TAG: u8 = 243;
/// Complete the formation of a bootstrapped month in place: its recipe, the reward schedule and
/// one SKU pool per bucket, so the ordinary indexing (200) and active-weight (121, 122, 124)
/// instructions can finish it and its Game updates and settlement run through the ordinary path.
pub const DEV_COMPLETE_FORMATION_TAG: u8 = 244;
/// Domain of the dev settlement digest:
/// `sha256(domain ‖ period ‖ month ‖ expiry_ts ‖ base ‖ Δ ‖ value ‖ settled_ts)`, integers
/// little-endian (Δ as `i64`).
pub const DEV_SETTLEMENT_DIGEST_DOMAIN: &[u8] = b"ameba-oracle-localnet-dev-settlement-v1";

/// Payload of [`DEV_BOOTSTRAP_MONTH_TAG`] (42 bytes).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct DevBootstrapMonthParams {
    /// Six-decimal base value; the dev chain uses 100_000_000 (100.000000).
    pub settlement_base_oracle_atomic: u64,
    /// Index-bound Merkle root of the month's bucket set, as tag 190 computes it.
    pub required_sku_root: [u8; 32],
    pub required_sku_count: u16,
}

/// Payload of [`DEV_COUNCIL_CONFIG_TAG`]: the five council seats, distinct and nonzero.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct DevCouncilConfigParams {
    pub seats: [Pubkey; 5],
}

/// The dev council Config: `[ameba-governance-v3, council]` under the program itself, the
/// address the council derives for its controller in this profile.
pub fn find_dev_council_config_address(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], program_id)
}

/// Payload of [`DEV_COMPLETE_FORMATION_TAG`]: one byte selects the step.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub enum DevCompleteFormationParams {
    /// 0: create the recipe manifest and the reward schedule for a month made by 240 and 241.
    Begin {
        expected_source_count: u16,
        expected_bucket_count: u16,
    },
    /// 1: collect up to four of one bucket's sources into the recipe, in ascending id order,
    /// creating the bucket's SKU pool on its first call.
    Bucket(DevFormationBucketParams),
    /// 2: freeze the recipe and fund the (zero-budget) schedule.
    Finalize,
}

/// One bucket step of [`DevCompleteFormationParams::Bucket`]. The SKU pool carries these bonds
/// and no reward budget.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Eq, PartialEq)]
pub struct DevFormationBucketParams {
    pub bucket_id: [u8; 32],
    /// Every source of the bucket carries this weight.
    pub bucket_weight_bps: u16,
    /// The bucket's last call: no further source of this bucket follows.
    pub finalize_collection: bool,
    pub proposer_reward_bps: u16,
    pub listing_bond: u64,
    pub support_bond: u64,
    pub opening_bond: u64,
    pub update_min_bond: u64,
    pub challenge_min_bond: u64,
    pub challenge_max_bond: u64,
    pub challenge_bond_bps: u16,
}

/// Payload of [`DEV_BOOTSTRAP_SOURCE_TAG`].
#[derive(BorshSerialize, Clone, Debug, Eq, PartialEq)]
pub struct DevBootstrapSourceParams {
    pub source_id: [u8; 32],
    pub bucket_id: [u8; 32],
    pub source_type_hash: [u8; 32],
    /// `sha256("locator" || url)`; the sealed kind-1 evidence object must commit to it.
    pub canonical_locator_hash: [u8; 32],
    pub source_definition_hash: [u8; 32],
    pub bucket_weight_bps: u16,
    /// Baseline and current state of the source (one observation at the month's listing).
    pub opening_state: u64,
    /// Position of `bucket_id` in the month's bucket set and its Merkle siblings.
    pub sku_index: u16,
    pub sku_proof: Vec<[u8; 32]>,
}

impl BorshDeserialize for DevBootstrapSourceParams {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        Ok(Self {
            source_id: <[u8; 32]>::deserialize_reader(reader)?,
            bucket_id: <[u8; 32]>::deserialize_reader(reader)?,
            source_type_hash: <[u8; 32]>::deserialize_reader(reader)?,
            canonical_locator_hash: <[u8; 32]>::deserialize_reader(reader)?,
            source_definition_hash: <[u8; 32]>::deserialize_reader(reader)?,
            bucket_weight_bps: u16::deserialize_reader(reader)?,
            opening_state: u64::deserialize_reader(reader)?,
            sku_index: u16::deserialize_reader(reader)?,
            sku_proof: deserialize_bounded_vec(
                reader,
                MAX_ORACLE_SKU_MERKLE_PROOF_DEPTH,
                "sku_proof",
            )?,
        })
    }
}

#[cfg(not(target_os = "solana"))]
mod builders {
    use super::*;
    use crate::state::{
        derive_oracle_sku_coverage_manifest_pda, derive_settlement_v2_pda, find_config_address,
    };
    use solana_program::instruction::{AccountMeta, Instruction};
    use solana_sdk_ids::system_program;

    fn dev_instruction<T: BorshSerialize>(
        program_id: &Pubkey,
        tag: u8,
        params: &T,
        accounts: Vec<AccountMeta>,
    ) -> Instruction {
        let mut data = vec![tag];
        params
            .serialize(&mut data)
            .expect("instruction serialization is infallible");
        Instruction {
            program_id: *program_id,
            accounts,
            data,
        }
    }

    /// Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 [writable] Forming
    /// period; 3 [writable] month PDA; 4 [writable] coverage manifest PDA; 5 config;
    /// 6 economics config; 7 system program.
    pub fn dev_bootstrap_month(
        program_id: &Pubkey,
        oracle_authority: &Pubkey,
        payer: &Pubkey,
        product_id: &[u8; 32],
        expiry_ts: u64,
        params: DevBootstrapMonthParams,
    ) -> Instruction {
        let period =
            ameba_oracle_interface::find_period_address(program_id, product_id, expiry_ts).0;
        let month = super::super::find_oracle_month_address(program_id, &period, expiry_ts);
        dev_instruction(
            program_id,
            DEV_BOOTSTRAP_MONTH_TAG,
            &params,
            vec![
                AccountMeta::new_readonly(*oracle_authority, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new(period, false),
                AccountMeta::new(month, false),
                AccountMeta::new(
                    derive_oracle_sku_coverage_manifest_pda(program_id, &month).0,
                    false,
                ),
                AccountMeta::new_readonly(find_config_address(program_id).0, false),
                AccountMeta::new_readonly(
                    super::super::find_oracle_economics_config_address(program_id),
                    false,
                ),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
        )
    }

    /// Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 period; 3 [writable]
    /// month; 4 config; 5 coverage manifest; 6 [writable] source PDA; 7 [writable] observations
    /// PDA; 8 sealed kind-1 locator object written by `object_payer` through tag 30 selector 14;
    /// 9 [writable] locator link; 10 [writable] source journal; 11 [writable] checkpoint;
    /// 12 system program.
    #[allow(clippy::too_many_arguments)]
    pub fn dev_bootstrap_source(
        program_id: &Pubkey,
        oracle_authority: &Pubkey,
        payer: &Pubkey,
        object_payer: &Pubkey,
        period: &Pubkey,
        month: &Pubkey,
        params: DevBootstrapSourceParams,
    ) -> Instruction {
        let source = super::super::find_oracle_source_address(program_id, month, &params.source_id);
        let locator = params.canonical_locator_hash;
        dev_instruction(
            program_id,
            DEV_BOOTSTRAP_SOURCE_TAG,
            &params,
            vec![
                AccountMeta::new_readonly(*oracle_authority, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(*period, false),
                AccountMeta::new(*month, false),
                AccountMeta::new_readonly(find_config_address(program_id).0, false),
                AccountMeta::new_readonly(
                    derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                    false,
                ),
                AccountMeta::new(source, false),
                AccountMeta::new(
                    super::super::find_oracle_source_observations_address(program_id, &source),
                    false,
                ),
                AccountMeta::new_readonly(
                    super::super::find_evidence_object_address(
                        program_id,
                        object_payer,
                        1,
                        &locator,
                    ),
                    false,
                ),
                AccountMeta::new(
                    super::super::find_evidence_link_address(program_id, &source, 0, &locator),
                    false,
                ),
                AccountMeta::new(
                    super::super::find_source_journal_address(program_id, &source),
                    false,
                ),
                AccountMeta::new(
                    super::super::find_checkpoint_address(program_id, &source, &source),
                    false,
                ),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
        )
    }

    /// Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 [writable] Live
    /// period; 3 [writable] bootstrapped month; 4 [writable] settlement record PDA; 5 config;
    /// 6 system program.
    pub fn dev_settle_month(
        program_id: &Pubkey,
        oracle_authority: &Pubkey,
        payer: &Pubkey,
        product_id: &[u8; 32],
        expiry_ts: u64,
    ) -> Instruction {
        let period =
            ameba_oracle_interface::find_period_address(program_id, product_id, expiry_ts).0;
        let month = super::super::find_oracle_month_address(program_id, &period, expiry_ts);
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new_readonly(*oracle_authority, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new(period, false),
                AccountMeta::new(month, false),
                AccountMeta::new(
                    derive_settlement_v2_pda(program_id, &period, &month).0,
                    false,
                ),
                AccountMeta::new_readonly(find_config_address(program_id).0, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data: vec![DEV_SETTLE_MONTH_TAG],
        }
    }
}

#[cfg(not(target_os = "solana"))]
pub use builders::*;

#[cfg(not(target_os = "solana"))]
mod council_builder {
    use super::*;
    use crate::state::find_config_address;
    use solana_program::instruction::{AccountMeta, Instruction};
    use solana_sdk_ids::system_program;

    /// Accounts: 0 [signer] admin; 1 [signer, writable] payer; 2 config; 3 [writable] dev
    /// council Config; 4 system program.
    pub fn dev_council_config(
        program_id: &Pubkey,
        admin: &Pubkey,
        payer: &Pubkey,
        seats: [Pubkey; 5],
    ) -> Instruction {
        let mut data = vec![DEV_COUNCIL_CONFIG_TAG];
        DevCouncilConfigParams { seats }
            .serialize(&mut data)
            .expect("instruction serialization is infallible");
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new_readonly(*admin, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(find_config_address(program_id).0, false),
                AccountMeta::new(find_dev_council_config_address(program_id).0, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            data,
        }
    }
}

#[cfg(not(target_os = "solana"))]
pub use council_builder::*;

#[cfg(not(target_os = "solana"))]
mod formation_builders {
    use super::*;
    use crate::state::{
        derive_oracle_recipe_weight_manifest_pda, derive_oracle_sku_coverage_manifest_pda,
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_sku_pool_pda,
        find_config_address,
    };
    use solana_program::instruction::{AccountMeta, Instruction};
    use solana_sdk_ids::system_program;

    fn formation(
        program_id: &Pubkey,
        params: &DevCompleteFormationParams,
        accounts: Vec<AccountMeta>,
    ) -> Instruction {
        let mut data = vec![DEV_COMPLETE_FORMATION_TAG];
        params
            .serialize(&mut data)
            .expect("instruction serialization is infallible");
        Instruction {
            program_id: *program_id,
            accounts,
            data,
        }
    }

    /// Step 0. Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 config;
    /// 3 period; 4 [writable] month; 5 coverage manifest; 6 [writable] recipe manifest PDA;
    /// 7 [writable] reward schedule PDA; 8 system program.
    pub fn dev_begin_formation(
        program_id: &Pubkey,
        oracle_authority: &Pubkey,
        payer: &Pubkey,
        period: &Pubkey,
        month: &Pubkey,
        expected_source_count: u16,
        expected_bucket_count: u16,
    ) -> Instruction {
        formation(
            program_id,
            &DevCompleteFormationParams::Begin {
                expected_source_count,
                expected_bucket_count,
            },
            vec![
                AccountMeta::new_readonly(*oracle_authority, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(find_config_address(program_id).0, false),
                AccountMeta::new_readonly(*period, false),
                AccountMeta::new(*month, false),
                AccountMeta::new_readonly(
                    derive_oracle_sku_coverage_manifest_pda(program_id, month).0,
                    false,
                ),
                AccountMeta::new(
                    derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
                    false,
                ),
                AccountMeta::new(
                    derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                    false,
                ),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
        )
    }

    /// Step 1. Accounts: 0 [signer] oracle authority; 1 [signer, writable] payer; 2 config;
    /// 3 period; 4 month; 5 [writable] recipe manifest; 6 [writable] reward schedule;
    /// 7 [writable] the bucket's SKU pool PDA; 8 system program; then one to four of the
    /// bucket's sources in ascending id order.
    #[allow(clippy::too_many_arguments)]
    pub fn dev_formation_bucket(
        program_id: &Pubkey,
        oracle_authority: &Pubkey,
        payer: &Pubkey,
        period: &Pubkey,
        month: &Pubkey,
        params: DevFormationBucketParams,
        sources: &[Pubkey],
    ) -> Instruction {
        let schedule = derive_oracle_usdc_reward_schedule_pda(program_id, month).0;
        let mut accounts = vec![
            AccountMeta::new_readonly(*oracle_authority, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(*period, false),
            AccountMeta::new_readonly(*month, false),
            AccountMeta::new(
                derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
                false,
            ),
            AccountMeta::new(schedule, false),
            AccountMeta::new(
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &params.bucket_id).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::ID, false),
        ];
        accounts.extend(
            sources
                .iter()
                .map(|source| AccountMeta::new_readonly(*source, false)),
        );
        formation(
            program_id,
            &DevCompleteFormationParams::Bucket(params),
            accounts,
        )
    }

    /// Step 2. Accounts: 0 [signer] oracle authority; 1 config; 2 period; 3 [writable] month;
    /// 4 [writable] recipe manifest; 5 [writable] reward schedule.
    pub fn dev_finalize_formation(
        program_id: &Pubkey,
        oracle_authority: &Pubkey,
        period: &Pubkey,
        month: &Pubkey,
    ) -> Instruction {
        formation(
            program_id,
            &DevCompleteFormationParams::Finalize,
            vec![
                AccountMeta::new_readonly(*oracle_authority, true),
                AccountMeta::new_readonly(find_config_address(program_id).0, false),
                AccountMeta::new_readonly(*period, false),
                AccountMeta::new(*month, false),
                AccountMeta::new(
                    derive_oracle_recipe_weight_manifest_pda(program_id, month).0,
                    false,
                ),
                AccountMeta::new(
                    derive_oracle_usdc_reward_schedule_pda(program_id, month).0,
                    false,
                ),
            ],
        )
    }
}

#[cfg(not(target_os = "solana"))]
pub use formation_builders::*;
