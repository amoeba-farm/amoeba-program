//! Game update helpers: a sealed archive-URL print, commit, reveal and finalize.
use super::{cohort::*, *};
use ameba_oracle::{
    instruction::{
        self, CommitOracleUpdateClaimV2Params, FinalizeOracleUpdateClaimV2Params,
        RevealOracleUpdateClaimV3Params, UpdateFinalization,
    },
    processor::oracle_update_claim_v2_commitment_hash,
    state::{OracleSourceState, OracleUpdateClaimOutcome},
};

pub const SALT: [u8; 32] = [0x5a; 32];

/// One update print: its archive URL is sealed by `payer`, its evidence commits to the source.
pub struct Print {
    pub state: u64,
    pub source_time: u64,
    pub url: String,
    pub evidence: [u8; 32],
}

pub struct Claim {
    pub key: Pubkey,
    pub claim_id: [u8; 32],
    pub print: Print,
    pub prior: u64,
}

impl Harness {
    pub async fn print(
        &mut self,
        cohort: &Cohort,
        payer: &Keypair,
        source: &Pubkey,
        source_id: &[u8; 32],
        state: u64,
        source_time: u64,
    ) -> Print {
        let url = archive_url(source_id, source_time);
        self.print_evidence(cohort, payer, source, source_id, state, source_time, url)
            .await
    }

    /// One update print whose archive evidence is `url`, a Wayback URL (sealed as kind 3) or an
    /// evidence record (kind 4); its evidence hash commits to the text.
    #[allow(clippy::too_many_arguments)]
    pub async fn print_evidence(
        &mut self,
        cohort: &Cohort,
        payer: &Keypair,
        source: &Pubkey,
        source_id: &[u8; 32],
        state: u64,
        source_time: u64,
        url: String,
    ) -> Print {
        self.upload_evidence(
            payer,
            instruction::archive_evidence_kind(&url),
            url.as_bytes(),
        )
        .await;
        let evidence = instruction::update_evidence_hash(
            &cohort.month,
            source,
            source_id,
            state,
            source_time,
            &locator_hash(&locator(source_id)),
            &definition_hash(&definition(source_id)),
            &url,
        );
        Print {
            state,
            source_time,
            url,
            evidence,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn commit(
        &mut self,
        cohort: &Cohort,
        claimant: &Keypair,
        source: &Pubkey,
        source_id: &[u8; 32],
        bucket: usize,
        claim_id: [u8; 32],
        print: Print,
    ) -> Result<Claim, solana_program_test::BanksClientError> {
        let prior: OracleSourceState = self.read_state(*source).await;
        let commit_hash = oracle_update_claim_v2_commitment_hash(
            &program(),
            &cohort.month,
            source,
            source_id,
            &claimant.pubkey(),
            &claim_id,
            prior.current_state,
            print.state,
            print.source_time,
            &print.evidence,
            &instruction::archive_url_hash(&print.url),
            &SALT,
        );
        self.send(
            instruction::commit_oracle_update_claim_v3(
                &program(),
                &claimant.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
                &cohort.sku_ids[bucket],
                CommitOracleUpdateClaimV2Params {
                    claim_id,
                    commit_hash,
                    stake: UPDATE_MIN_BOND,
                },
            ),
            &[claimant],
        )
        .await?;
        Ok(Claim {
            key: instruction::find_oracle_update_claim_address(
                &program(),
                &cohort.month,
                source,
                &claimant.pubkey(),
                &claim_id,
            ),
            claim_id,
            print,
            prior: prior.current_state,
        })
    }

    pub async fn reveal(
        &mut self,
        cohort: &Cohort,
        claimant: &Keypair,
        source: &Pubkey,
        claim: &Claim,
    ) -> Result<(), solana_program_test::BanksClientError> {
        self.send(
            instruction::reveal_oracle_update_claim_v3(
                &program(),
                &claimant.pubkey(),
                &claimant.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
                RevealOracleUpdateClaimV3Params {
                    claim_id: claim.claim_id,
                    prior_state: claim.prior,
                    new_state: claim.print.state,
                    source_time: claim.print.source_time,
                    evidence_hash: claim.print.evidence,
                    archive_url: String::new(),
                    secret_salt: SALT,
                },
                &claim.print.url,
            ),
            &[claimant],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finalize(
        &mut self,
        cohort: &Cohort,
        source: &Pubkey,
        bucket: usize,
        claim: &Pubkey,
        outcome: OracleUpdateClaimOutcome,
        current_step: u64,
        shape: UpdateFinalization,
    ) -> Result<(), solana_program_test::BanksClientError> {
        let oracle = self.oracle_authority.insecure_clone();
        self.send(
            instruction::finalize_oracle_update_claim_v2(
                &program(),
                &oracle.pubkey(),
                &cohort.period,
                &cohort.month,
                source,
                &cohort.sku_ids[bucket],
                claim,
                FinalizeOracleUpdateClaimV2Params {
                    outcome,
                    current_step,
                },
                shape,
            ),
            &[&oracle],
        )
        .await
    }
}
