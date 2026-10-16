//! Tag 130 `CfmLaunchV1` wire (`crate::cfm`, `processor/cfm_launch.rs`).

use super::*;

/// Begin: three current council seats approve the plan; creates the receipt, the month in
/// Opening with its finalized coverage, the recipe and active-weight manifests and the reward
/// schedule.
pub const CFM_LAUNCH_BEGIN: u8 = 0;
/// Append: one registry row becomes an Active source with its printed opening, its SKU pool and
/// its bucket median. Permissionless; rows go in registry order, and the last one moves the month
/// to Game and the period Live.
pub const CFM_LAUNCH_APPEND: u8 = 1;

/// Payload of tag 130 (35 bytes): the step, the product code, the row (append only, else 0) and
/// the plan digest the council approved.
#[derive(BorshSerialize, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CfmLaunchV1Params {
    pub operation: u8,
    pub product: u8,
    pub row: u8,
    pub plan_hash: [u8; 32],
}

crate::fixed_codec::fixed_instruction_deserialize!(CfmLaunchV1Params, 35, {
    operation: u8,
    product: u8,
    row: u8,
    plan_hash: [u8; 32],
});

#[cfg(not(target_os = "solana"))]
mod builders {
    use super::*;
    use crate::cfm::{find_receipt_address, CfmProduct};
    use crate::state::{
        derive_oracle_active_weight_manifest_pda, derive_oracle_bucket_median_pda,
        derive_oracle_recipe_source_index_pda, derive_oracle_recipe_weight_manifest_pda,
        derive_oracle_sku_coverage_manifest_pda, derive_oracle_usdc_reward_schedule_pda,
        derive_oracle_usdc_sku_pool_pda, find_config_address,
    };
    use solana_program::instruction::{AccountMeta, Instruction};
    use solana_sdk_ids::system_program;

    /// The accounts every step derives from the product and expiry.
    #[derive(Clone, Copy, Debug)]
    pub struct CfmLaunchAccounts {
        pub period: Pubkey,
        pub month: Pubkey,
        pub coverage: Pubkey,
        pub receipt: Pubkey,
        pub recipe: Pubkey,
        pub active: Pubkey,
        pub schedule: Pubkey,
        pub recipe_index: Pubkey,
    }

    impl CfmLaunchAccounts {
        pub fn derive(program_id: &Pubkey, product: &CfmProduct, expiry_ts: u64) -> Self {
            let period = ameba_oracle_interface::find_period_address(
                program_id,
                &product.product_id,
                expiry_ts,
            )
            .0;
            let month = super::super::find_oracle_month_address(program_id, &period, expiry_ts);
            Self {
                period,
                month,
                coverage: derive_oracle_sku_coverage_manifest_pda(program_id, &month).0,
                receipt: find_receipt_address(program_id, &product.product_id).0,
                recipe: derive_oracle_recipe_weight_manifest_pda(program_id, &month).0,
                active: derive_oracle_active_weight_manifest_pda(program_id, &month).0,
                schedule: derive_oracle_usdc_reward_schedule_pda(program_id, &month).0,
                recipe_index: derive_oracle_recipe_source_index_pda(program_id, &month).0,
            }
        }

        /// The source of a registry row, and the accounts its append creates.
        pub fn row(&self, program_id: &Pubkey, source_id: &[u8; 32]) -> CfmRowAccounts {
            let source =
                super::super::find_oracle_source_address(program_id, &self.month, source_id);
            CfmRowAccounts {
                source,
                observations: super::super::find_oracle_source_observations_address(
                    program_id, &source,
                ),
                sku_pool: derive_oracle_usdc_sku_pool_pda(program_id, &self.schedule, source_id).0,
                bucket: derive_oracle_bucket_median_pda(program_id, &self.month, source_id).0,
                journal: super::super::find_source_journal_address(program_id, &source),
                checkpoint: super::super::find_checkpoint_address(
                    program_id,
                    &source,
                    &self.receipt,
                ),
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub struct CfmRowAccounts {
        pub source: Pubkey,
        pub observations: Pubkey,
        pub sku_pool: Pubkey,
        pub bucket: Pubkey,
        pub journal: Pubkey,
        pub checkpoint: Pubkey,
    }

    fn launch(
        program_id: &Pubkey,
        params: CfmLaunchV1Params,
        accounts: Vec<AccountMeta>,
    ) -> Instruction {
        let mut data = vec![OracleInstructionTag::CfmLaunchV1 as u8];
        params
            .serialize(&mut data)
            .expect("instruction serialization is infallible");
        Instruction {
            program_id: *program_id,
            accounts,
            data,
        }
    }

    /// Step 0. Accounts: 0 [signer, writable] payer (the proposer); 1 [writable] Forming
    /// period; 2-7 [writable] the month, coverage, receipt, recipe manifest, active-weight
    /// manifest and reward schedule PDAs; 8 config; 9 economics config; 10 the sealed plan
    /// object; 11 the sealed registry object; 12 the council Config; 13-15 [signer] three
    /// distinct current council seats; 16 system program.
    #[allow(clippy::too_many_arguments)]
    pub fn cfm_launch_begin(
        program_id: &Pubkey,
        payer: &Pubkey,
        product: &CfmProduct,
        expiry_ts: u64,
        plan_hash: [u8; 32],
        plan_object: &Pubkey,
        registry_object: &Pubkey,
        council_config: &Pubkey,
        seats: &[Pubkey; 3],
    ) -> Instruction {
        let a = CfmLaunchAccounts::derive(program_id, product, expiry_ts);
        let mut accounts = vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(a.period, false),
            AccountMeta::new(a.month, false),
            AccountMeta::new(a.coverage, false),
            AccountMeta::new(a.receipt, false),
            AccountMeta::new(a.recipe, false),
            AccountMeta::new(a.active, false),
            AccountMeta::new(a.schedule, false),
            AccountMeta::new_readonly(find_config_address(program_id).0, false),
            AccountMeta::new_readonly(
                super::super::find_oracle_economics_config_address(program_id),
                false,
            ),
            AccountMeta::new_readonly(*plan_object, false),
            AccountMeta::new_readonly(*registry_object, false),
            AccountMeta::new_readonly(*council_config, false),
        ];
        accounts.extend(
            seats
                .iter()
                .map(|seat| AccountMeta::new_readonly(*seat, true)),
        );
        accounts.push(AccountMeta::new_readonly(system_program::id(), false));
        launch(
            program_id,
            CfmLaunchV1Params {
                operation: CFM_LAUNCH_BEGIN,
                product: product.code,
                row: 0,
                plan_hash,
            },
            accounts,
        )
    }

    /// Step 1. Accounts: 0 [signer, writable] payer; 1 [writable] period; 2 [writable] month;
    /// 3 [writable] receipt; 4 plan object; 5 registry object; 6 [writable] recipe manifest;
    /// 7 [writable] active-weight manifest; 8 [writable] reward schedule; 9-14 [writable] the
    /// row's source, observations, SKU pool, bucket median, source journal and checkpoint PDAs;
    /// 15 system program.
    #[allow(clippy::too_many_arguments)]
    pub fn cfm_launch_append(
        program_id: &Pubkey,
        payer: &Pubkey,
        product: &CfmProduct,
        expiry_ts: u64,
        plan_hash: [u8; 32],
        plan_object: &Pubkey,
        registry_object: &Pubkey,
        row: u8,
        source_id: &[u8; 32],
    ) -> Instruction {
        let a = CfmLaunchAccounts::derive(program_id, product, expiry_ts);
        let r = a.row(program_id, source_id);
        launch(
            program_id,
            CfmLaunchV1Params {
                operation: CFM_LAUNCH_APPEND,
                product: product.code,
                row,
                plan_hash,
            },
            vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(a.period, false),
                AccountMeta::new(a.month, false),
                AccountMeta::new(a.receipt, false),
                AccountMeta::new_readonly(*plan_object, false),
                AccountMeta::new_readonly(*registry_object, false),
                AccountMeta::new(a.recipe, false),
                AccountMeta::new(a.active, false),
                AccountMeta::new(a.schedule, false),
                AccountMeta::new(r.source, false),
                AccountMeta::new(r.observations, false),
                AccountMeta::new(r.sku_pool, false),
                AccountMeta::new(r.bucket, false),
                AccountMeta::new(r.journal, false),
                AccountMeta::new(r.checkpoint, false),
                AccountMeta::new_readonly(system_program::id(), false),
            ],
        )
    }
}

#[cfg(not(target_os = "solana"))]
pub use builders::*;
