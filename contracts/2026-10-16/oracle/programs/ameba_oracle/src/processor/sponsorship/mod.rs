//! Permissioned underwriting; permissionless settlement to immutable recipients.
use super::*;
use crate::instruction::OracleSponsoredActionV1 as Action;
use crate::sponsorship::*;
mod custody;
mod settlement;
use custody::*;

/// The exact account count of each action.
fn account_count(action: &Action) -> usize {
    match action {
        Action::Activate { .. } => 22,
        Action::Reconcile => 8,
        Action::ClaimBounty => 22,
        Action::WithdrawReserve { .. } => 4,
    }
}

pub(in crate::processor) fn process(
    program: &Pubkey,
    a: &[AccountInfo],
    action: Action,
) -> ProgramResult {
    if a.len() != account_count(&action) {
        return Err(OracleError::InvalidAccountList.into());
    }
    if !a[0].is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    match action {
        Action::Activate { terms, source } => activate(program, a, terms, source),
        Action::Reconcile => settlement::reconcile(program, a),
        Action::ClaimBounty => settlement::claim(program, a),
        Action::WithdrawReserve { amount } => {
            let agent = load_agent(program, &a[1])?;
            if a[0].key != &agent.authority || amount == 0 {
                return invalid();
            }
            move_cash(program, &a[2], a[1].key, &a[3], &agent.authority, amount)
        }
    }
}

fn invalid<T>() -> Result<T, ProgramError> {
    Err(OracleError::InvalidOracleState.into())
}
fn add(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_add(b)
        .ok_or(OracleError::ArithmeticOverflow.into())
}
fn sub(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_sub(b)
        .ok_or(OracleError::InvalidOracleState.into())
}

#[inline(never)]
fn activate(
    program: &Pubkey,
    a: &[AccountInfo],
    terms: SponsorTerms,
    source: ProposeOracleSourceV3Params,
) -> ProgramResult {
    if !terms.valid()
        || !a[18].is_signer
        || source.source_id != terms.candidate_hash
        || source.listing_bond == 0
        || source.listing_bond > terms.maximum_bond
    {
        return invalid();
    }
    let (agent_key, agent_bump) = reserve_address(program, &terms.agent_id, &terms.researcher);
    let (agreement_key, agreement_bump) = agreement_address(program, a[5].key);
    if a[16].key != &agreement_key || a[17].key != &agent_key {
        return invalid();
    }
    if terms.funding == 1 && a[18].key != &terms.researcher {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let funder = if terms.funding == 1 {
        agent_key
    } else {
        *a[18].key
    };
    let (exposure_key, exposure_bump) = exposure_address(program, &agent_key, &funder);
    if a[21].key != &exposure_key {
        return invalid();
    }
    validate_create_only_program_account_target(program, &a[16])?;

    let mut agent = if a[17].owner == program {
        load_agent(program, &a[17])?
    } else {
        create_program_account(
            &a[0],
            &a[17],
            &a[9],
            program,
            AgentReserve::LEN,
            &[
                RESERVE_SEED,
                &terms.agent_id,
                terms.researcher.as_ref(),
                &[agent_bump],
            ],
        )?;
        AgentReserve {
            discriminator: AgentReserve::MAGIC,
            bump: agent_bump,
            agent_id: terms.agent_id,
            authority: terms.researcher,
            ..Default::default()
        }
    };
    let mut exposure = if a[21].owner == program {
        load_exposure(program, &a[21], &agent_key, &funder)?
    } else {
        create_program_account(
            &a[0],
            &a[21],
            &a[9],
            program,
            SponsorExposure::LEN,
            &[
                EXPOSURE_SEED,
                agent_key.as_ref(),
                funder.as_ref(),
                &[exposure_bump],
            ],
        )?;
        SponsorExposure {
            discriminator: SponsorExposure::MAGIC,
            bump: exposure_bump,
            reserve: agent_key,
            funder,
            outstanding: 0,
        }
    };
    exposure.outstanding = add(exposure.outstanding, source.listing_bond)?;
    if exposure.outstanding > terms.maximum_outstanding {
        return invalid();
    }
    agent.pending = add(agent.pending, 1)?;
    create_program_account(
        &a[0],
        &a[16],
        &a[9],
        program,
        SourceSponsorship::LEN,
        &[AGREEMENT_SEED, a[5].key.as_ref(), &[agreement_bump]],
    )?;
    ensure_collateral(program, &a[0], &a[8], &a[9], &agreement_key)?;
    ensure_collateral(program, &a[0], &a[20], &a[9], &agent_key)?;
    move_cash(
        program,
        &a[19],
        &funder,
        &a[8],
        &agreement_key,
        source.listing_bond,
    )?;
    let agreement = SourceSponsorship {
        discriminator: SourceSponsorship::MAGIC,
        bump: agreement_bump,
        source: *a[5].key,
        reserve: agent_key,
        funder,
        candidate_hash: terms.candidate_hash,
        bond: source.listing_bond,
        researcher_bps: terms.researcher_bps,
        retention_bps: terms.retention_bps,
        month: *a[2].key,
        bucket_id: source.bucket_id,
        source_type_hash: source.source_type_hash,
        canonical_locator_hash: source.canonical_locator_hash,
        source_definition_hash: source.source_definition_hash,
        funding: terms.funding,
        ..Default::default()
    };
    // Uses ordinary import, evidence, membership, window, bond and source rules.
    super::usdc::process_propose_oracle_source_v3_for(program, &a[..16], source, &agreement_key)?;
    store_state(&a[16], &agreement)?;
    store_state(&a[17], &agent)?;
    store_state(&a[21], &exposure)
}
