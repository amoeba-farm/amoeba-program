//! Tag 181: one oracle month per Forming period, anchored on the period, with its coverage
//! manifest and the product's maturity ladder rung.

mod common;

use ameba_oracle::{
    error::OracleError,
    instruction::{self, InitializeOracleMonthV5Params},
    state::{
        derive_oracle_maturity_ladder_registry_pda, derive_oracle_sku_coverage_manifest_pda,
        OracleEconomicParams, OracleMaturityLadderRegistry, OracleMonthState, OraclePhase,
        OracleSettlementStatus, OracleSkuCoverageManifest,
    },
};
use ameba_oracle_interface::{find_period_address, Period, PeriodStatus};
use common::*;
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey, signature::Signer};

const BASE: u64 = 100_000_000;

struct Product {
    id: [u8; 32],
    root: [u8; 32],
    count: u16,
}

async fn ready_harness() -> (Harness, Product, Product) {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    h.configure_economics(0).await.unwrap();
    let ramx = product("RAMX");
    let nandx = product("NANDX");
    let ramx_ids = synthetic_sku_ids(20);
    let nandx_ids = synthetic_sku_ids(5);
    let ramx_root = h.register_product(ramx, &ramx_ids).await;
    let nandx_root = h.register_product(nandx, &nandx_ids).await;
    (
        h,
        Product {
            id: ramx,
            root: ramx_root,
            count: 20,
        },
        Product {
            id: nandx,
            root: nandx_root,
            count: 5,
        },
    )
}

fn month_params(product: &Product, listing: u64) -> InitializeOracleMonthV5Params {
    InitializeOracleMonthV5Params {
        scramble_start_ts: listing - PROGRAM_PRE_LISTING_SECONDS,
        listing_ts: listing,
        settlement_base_oracle_atomic: BASE,
        required_sku_root: product.root,
        required_sku_count: product.count,
    }
}

impl Harness {
    async fn create_rung(&mut self, product_id: [u8; 32], planned: u64, expiry: u64) {
        let oracle = self.oracle_authority.insecure_clone();
        self.create_period(&oracle, product_id, planned, expiry, 6)
            .await
            .unwrap();
    }

    async fn initialize_month(
        &mut self,
        product_id: &[u8; 32],
        expiry: u64,
        params: InitializeOracleMonthV5Params,
    ) -> Result<(), solana_program_test::BanksClientError> {
        let ix = instruction::initialize_oracle_month_v5(
            &program(),
            &self.oracle_authority.pubkey(),
            &self.ctx.payer.pubkey(),
            product_id,
            expiry,
            params,
        );
        let oracle = self.oracle_authority.insecure_clone();
        self.send(ix, &[&oracle]).await
    }

    fn month_key(&self, product_id: &[u8; 32], expiry: u64) -> Pubkey {
        let period = find_period_address(&program(), product_id, expiry).0;
        instruction::find_oracle_month_address(&program(), &period, expiry)
    }

    /// Overwrite a period with an explicit, layout-valid state.
    async fn install_period(&mut self, period: Period) {
        let (key, bump) = find_period_address(&program(), &period.product_id, period.expiry_ts);
        let period = Period { bump, ..period };
        period.validate().unwrap();
        let rent = self.ctx.banks_client.get_rent().await.unwrap();
        let mut account = solana_sdk::account::AccountSharedData::new(
            rent.minimum_balance(Period::LEN),
            Period::LEN,
            &program(),
        );
        account.set_data_from_slice(&period.encode());
        self.set_account_preserving_capitalization(key, account)
            .await;
    }
}

#[tokio::test]
async fn month_initializes_on_a_forming_period_with_coverage_and_ladder() {
    let (mut h, ramx, _) = ready_harness().await;
    h.create_rung(ramx.id, PLANNED, EXPIRY).await;
    h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
        .await
        .unwrap();

    let period_key = find_period_address(&program(), &ramx.id, EXPIRY).0;
    let month_key = h.month_key(&ramx.id, EXPIRY);
    let month: OracleMonthState = h.read_state(month_key).await;
    assert!(month.is_initialized);
    assert!(month.has_current_layout());
    assert_eq!(month.period, period_key);
    assert_eq!(month.authority, h.oracle_authority.pubkey());
    assert_eq!(
        month.scramble_start_ts,
        PLANNED - PROGRAM_PRE_LISTING_SECONDS
    );
    assert_eq!(month.listing_ts, PLANNED);
    assert_eq!(month.phase, OraclePhase::SourceSubmission);
    assert_eq!(month.settlement_base_oracle_atomic, BASE);
    assert_eq!(month.economics, OracleEconomicParams::default());
    assert_eq!(
        month.settlement_status,
        OracleSettlementStatus::FrozenPendingEvidence
    );
    assert_eq!(month.settlement_record, None);
    assert_eq!(
        month.schedule_version,
        OracleMonthState::SKU_COVERAGE_SCHEDULE_VERSION
    );
    let account = h.account(month_key).await.unwrap();
    assert_eq!(account.data.len(), OracleMonthState::LEN);
    // The month's period field sits at the established offset readers decode.
    assert_eq!(&account.data[6..38], period_key.as_ref());

    let coverage: OracleSkuCoverageManifest = h
        .read_state(derive_oracle_sku_coverage_manifest_pda(&program(), &month_key).0)
        .await;
    assert!(coverage.is_initialized);
    assert_eq!(coverage.month, month_key);
    assert_eq!(coverage.required_sku_root, ramx.root);
    assert_eq!(coverage.required_sku_count, ramx.count);
    assert_eq!(coverage.covered_sku_count, 0);
    assert_eq!(coverage.planned_listing_ts, PLANNED);
    assert!(!coverage.coverage_finalized);

    let ladder: OracleMaturityLadderRegistry = h
        .read_state(derive_oracle_maturity_ladder_registry_pda(&program(), &ramx.id).0)
        .await;
    assert_eq!(ladder.underlying_id, ramx.id);
    assert_eq!(ladder.planned_listing_ts, PLANNED);
    assert_eq!(ladder.planned_expiry_ts, EXPIRY);

    // The period itself is untouched: month formation does not list it.
    let period = h.period(ramx.id, EXPIRY).await;
    assert_eq!(period.status, PeriodStatus::Forming);
    assert_eq!(period.effective_listing_ts, 0);

    // Create-only: a retry neither succeeds nor rewrites the month.
    let before = h.account(month_key).await.unwrap();
    h.advance_slot().await;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
            .await,
        OracleError::AlreadyInitialized,
    );
    assert_eq!(h.account(month_key).await.unwrap(), before);
}

#[tokio::test]
async fn month_requires_a_forming_period() {
    let (mut h, ramx, _) = ready_harness().await;
    h.create_rung(ramx.id, PLANNED, EXPIRY).await;
    let forming = h.period(ramx.id, EXPIRY).await;
    for period in [
        Period {
            status: PeriodStatus::Live,
            effective_listing_ts: PLANNED,
            ..forming.clone()
        },
        Period {
            status: PeriodStatus::Failed,
            ..forming.clone()
        },
        Period {
            status: PeriodStatus::Failed,
            effective_listing_ts: PLANNED + 3_600,
            ..forming.clone()
        },
    ] {
        h.install_period(period.clone()).await;
        assert_oracle_error(
            h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
                .await,
            OracleError::InvalidOracleState,
        );
        h.advance_slot().await;
    }
    assert!(h.account(h.month_key(&ramx.id, EXPIRY)).await.is_none());
}

#[tokio::test]
async fn month_listing_must_equal_the_planned_listing() {
    let (mut h, ramx, _) = ready_harness().await;
    // Off-ladder planned listings cannot be created through tag 1; install one directly so the
    // month's own listing check is reached after the ladder accepts its parameters.
    h.install_period(Period::new_forming(0, ramx.id, PLANNED + 86_400, EXPIRY, 6))
        .await;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
            .await,
        OracleError::InvalidOracleState,
    );
    // A listing off the three-month rung fails the ladder first.
    h.install_period(Period::new_forming(0, ramx.id, PLANNED, EXPIRY, 6))
        .await;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, JAN_01_2027))
            .await,
        OracleError::InvalidOracleMaturityLadder,
    );
    // A scramble start that does not precede listing by the full calendar is rejected.
    let mut gapped = month_params(&ramx, PLANNED);
    gapped.scramble_start_ts -= 1;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, gapped).await,
        OracleError::InvalidOracleState,
    );
    h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
        .await
        .unwrap();
}

#[tokio::test]
async fn month_binds_the_period_identity_and_product_manifest() {
    let (mut h, ramx, nandx) = ready_harness().await;
    h.create_rung(ramx.id, PLANNED, EXPIRY).await;
    h.create_rung(nandx.id, PLANNED, EXPIRY).await;

    // A root or count that differs from the product's registered manifest is rejected.
    let mut wrong_root = month_params(&ramx, PLANNED);
    wrong_root.required_sku_root = nandx.root;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, wrong_root).await,
        OracleError::InvalidOracleSkuCoverageManifest,
    );
    let mut wrong_count = month_params(&ramx, PLANNED);
    wrong_count.required_sku_count = 21;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, wrong_count).await,
        OracleError::InvalidOracleSkuCoverageManifest,
    );
    let mut zero_base = month_params(&ramx, PLANNED);
    zero_base.settlement_base_oracle_atomic = 0;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, zero_base).await,
        OracleError::InvalidOracleSkuCoverageManifest,
    );

    // Another product's period cannot stand in for this product's manifest.
    let mut swapped = instruction::initialize_oracle_month_v5(
        &program(),
        &h.oracle_authority.pubkey(),
        &h.ctx.payer.pubkey(),
        &ramx.id,
        EXPIRY,
        month_params(&ramx, PLANNED),
    );
    swapped.accounts[2] =
        AccountMeta::new_readonly(find_period_address(&program(), &nandx.id, EXPIRY).0, false);
    let oracle = h.oracle_authority.insecure_clone();
    assert_oracle_error(
        h.send(swapped.clone(), &[&oracle]).await,
        OracleError::InvalidOracleSkuCoverageManifest,
    );
    // An account that is not one of this program's periods is not an anchor.
    swapped.accounts[2] = AccountMeta::new_readonly(Pubkey::new_unique(), false);
    assert_oracle_error(
        h.send(swapped, &[&oracle]).await,
        OracleError::InvalidPeriodAccount,
    );

    // The month PDA is bound to the period and its expiry.
    let mut wrong_month = instruction::initialize_oracle_month_v5(
        &program(),
        &h.oracle_authority.pubkey(),
        &h.ctx.payer.pubkey(),
        &ramx.id,
        EXPIRY,
        month_params(&ramx, PLANNED),
    );
    wrong_month.accounts[3] = AccountMeta::new(h.month_key(&nandx.id, EXPIRY), false);
    assert_oracle_error(
        h.send(wrong_month, &[&oracle]).await,
        OracleError::InvalidOracleMonthAccount,
    );

    // Only the current oracle authority initializes months; the admin cannot.
    let mut by_admin = instruction::initialize_oracle_month_v5(
        &program(),
        &h.admin.pubkey(),
        &h.ctx.payer.pubkey(),
        &ramx.id,
        EXPIRY,
        month_params(&ramx, PLANNED),
    );
    by_admin.accounts[0].is_signer = true;
    let admin = h.admin.insecure_clone();
    assert_oracle_error(h.send(by_admin, &[&admin]).await, OracleError::Unauthorized);

    h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
        .await
        .unwrap();
    h.initialize_month(&nandx.id, EXPIRY, month_params(&nandx, PLANNED))
        .await
        .unwrap();
}

#[tokio::test]
async fn month_requires_the_economics_template() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    let ramx = product("RAMX");
    let ids = synthetic_sku_ids(3);
    let root = h.register_product(ramx, &ids).await;
    h.create_rung(ramx, PLANNED, EXPIRY).await;
    let params = InitializeOracleMonthV5Params {
        scramble_start_ts: PLANNED - PROGRAM_PRE_LISTING_SECONDS,
        listing_ts: PLANNED,
        settlement_base_oracle_atomic: BASE,
        required_sku_root: root,
        required_sku_count: 3,
    };
    assert_oracle_error(
        h.initialize_month(&ramx, EXPIRY, params.clone()).await,
        OracleError::InvalidOracleState,
    );
    h.configure_economics(0).await.unwrap();
    h.initialize_month(&ramx, EXPIRY, params).await.unwrap();
}

#[tokio::test]
async fn month_initialization_preserves_the_post_submission_review_calendar() {
    let (mut h, ramx, _) = ready_harness().await;
    h.create_rung(ramx.id, PLANNED, EXPIRY).await;
    // Four rulebook days (challenge, resolution-freeze, opening) must fit before expiry.
    set_unix_timestamp(&h.ctx, EXPIRY - 4 * PROGRAM_DAY_SECONDS).await;
    h.advance_slot().await;
    assert_oracle_error(
        h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
            .await,
        OracleError::OracleSkuCoverageExtensionTooLate,
    );
    set_unix_timestamp(&h.ctx, EXPIRY - 4 * PROGRAM_DAY_SECONDS - 1).await;
    h.advance_slot().await;
    h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
        .await
        .unwrap();
}

#[tokio::test]
async fn maturity_ladder_admits_one_rung_and_advances_only_after_its_listing() {
    let (mut h, ramx, nandx) = ready_harness().await;
    h.create_rung(ramx.id, PLANNED, EXPIRY).await;
    h.create_rung(ramx.id, JAN_01_2027, APR_01_2027).await;
    h.create_rung(nandx.id, JAN_01_2027, APR_01_2027).await;
    h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
        .await
        .unwrap();

    // The next RAMX rung cannot be staged before the current planned listing arrives.
    assert_oracle_error(
        h.initialize_month(&ramx.id, APR_01_2027, month_params(&ramx, JAN_01_2027))
            .await,
        OracleError::InvalidOracleMaturityLadder,
    );
    // Each product keeps its own ladder.
    h.initialize_month(&nandx.id, APR_01_2027, month_params(&nandx, JAN_01_2027))
        .await
        .unwrap();

    set_unix_timestamp(&h.ctx, PLANNED).await;
    h.advance_slot().await;
    h.initialize_month(&ramx.id, APR_01_2027, month_params(&ramx, JAN_01_2027))
        .await
        .unwrap();
    let ladder: OracleMaturityLadderRegistry = h
        .read_state(derive_oracle_maturity_ladder_registry_pda(&program(), &ramx.id).0)
        .await;
    assert_eq!(ladder.planned_listing_ts, JAN_01_2027);
    assert_eq!(ladder.planned_expiry_ts, APR_01_2027);
}

#[tokio::test]
async fn economics_template_is_admin_governed_and_versioned() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    h.split_oracle_authority().await;
    assert_oracle_error(
        h.configure_economics(1).await,
        OracleError::InvalidOracleState,
    );
    h.configure_economics(0).await.unwrap();
    let key = instruction::find_oracle_economics_config_address(&program());
    let first: ameba_oracle::state::OracleEconomicsConfig = h.read_state(key).await;
    assert_eq!(first.config_version, 1);
    h.advance_slot().await;
    assert_oracle_error(
        h.configure_economics(0).await,
        OracleError::InvalidOracleState,
    );
    h.configure_economics(1).await.unwrap();
    let second: ameba_oracle::state::OracleEconomicsConfig = h.read_state(key).await;
    assert_eq!(second.config_version, 2);

    let oracle = h.oracle_authority.insecure_clone();
    let by_oracle = instruction::configure_oracle_economics_template_v2(
        &program(),
        &oracle.pubkey(),
        instruction::ConfigureOracleEconomicsTemplateV2Params {
            expected_config_version: 2,
            economics: OracleEconomicParams::default(),
        },
    );
    assert_oracle_error(
        h.send(by_oracle, &[&oracle]).await,
        OracleError::Unauthorized,
    );
    let invalid = instruction::configure_oracle_economics_template_v2(
        &program(),
        &h.admin.pubkey(),
        instruction::ConfigureOracleEconomicsTemplateV2Params {
            expected_config_version: 2,
            economics: OracleEconomicParams {
                emergency_supermajority_bps: 10_001,
                ..OracleEconomicParams::default()
            },
        },
    );
    let admin = h.admin.insecure_clone();
    assert_oracle_error(
        h.send(invalid, &[&admin]).await,
        OracleError::InvalidOracleState,
    );
}

#[tokio::test]
async fn shortening_a_template_only_changes_new_month_snapshots() {
    let (mut h, ramx, nandx) = ready_harness().await;
    let admin = h.admin.insecure_clone();
    let template = |version, economics| {
        instruction::configure_oracle_economics_template_v2(
            &program(),
            &admin.pubkey(),
            instruction::ConfigureOracleEconomicsTemplateV2Params {
                expected_config_version: version,
                economics,
            },
        )
    };
    h.send(template(1, OracleEconomicParams::production()), &[&admin])
        .await
        .unwrap();
    h.create_rung(ramx.id, PLANNED, EXPIRY).await;
    h.initialize_month(&ramx.id, EXPIRY, month_params(&ramx, PLANNED))
        .await
        .unwrap();
    let old_key = h.month_key(&ramx.id, EXPIRY);
    let before = h.account(old_key).await.unwrap().data;
    let old: OracleMonthState = h.read_state(old_key).await;
    assert_eq!(old.economics, OracleEconomicParams::production());
    h.send(
        template(2, OracleEconomicParams::governed_localnet()),
        &[&admin],
    )
    .await
    .unwrap();
    h.create_rung(nandx.id, PLANNED, EXPIRY).await;
    h.initialize_month(&nandx.id, EXPIRY, month_params(&nandx, PLANNED))
        .await
        .unwrap();
    let new_key = h.month_key(&nandx.id, EXPIRY);
    let new: OracleMonthState = h.read_state(new_key).await;
    assert_eq!(new.economics, OracleEconomicParams::governed_localnet());
    assert_eq!(h.account(old_key).await.unwrap().data, before);
}
