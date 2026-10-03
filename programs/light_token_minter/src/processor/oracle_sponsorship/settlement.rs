use super::*;

/// Existing escrow settlement must run first. Repeated calls only sweep later
/// challenge proceeds to the immutable funder; counters/exposure settle once.
pub(super) fn reconcile(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let source = load_valid_oracle_source(program, a[1].key, &a[2])?;
    let mut agreement = load_agreement(program, &a[3])?;
    let mut agent = load_agent(program, &a[4])?;
    if agreement.source != *a[2].key
        || agreement.reserve != *a[4].key
        || source.proposer != *a[3].key
        || source.listing_bond_locked != 0
    {
        return invalid();
    }
    let outcome = match source.status {
        OracleSourceStatus::Active | OracleSourceStatus::Inactive => 1,
        OracleSourceStatus::Rejected => 2,
        OracleSourceStatus::TimedOut | OracleSourceStatus::Merged => 3,
        _ => return invalid(),
    };
    let mut exposure = load_exposure(program, &a[7], a[4].key, &agreement.funder)?;
    if agreement.outcome == 0 {
        agent.pending = sub(agent.pending, 1)?;
        if outcome == 1 {
            agent.accepted = add(agent.accepted, 1)?;
        }
        if outcome == 2 {
            agent.rejected = add(agent.rejected, 1)?;
        }
        if agreement.funding == 1
            && matches!(
                source.status,
                OracleSourceStatus::Rejected | OracleSourceStatus::Merged
            )
        {
            agent.slashed = add(agent.slashed, agreement.bond)?;
        }
        exposure.outstanding = sub(exposure.outstanding, agreement.bond)?;
        agreement.outcome = outcome;
    }
    let cash = load_canonical_user_collateral(program, &a[5], a[3].key)?.available_balance;
    move_cash(program, &a[5], a[3].key, &a[6], &agreement.funder, cash)?;
    store_state(&a[3], &agreement)?;
    store_state(&a[4], &agent)?;
    store_state(&a[7], &exposure)
}

/// Claims only SourceProposer, using the original funded sleeve and canonical
/// receipt. A PDA cannot sign the old claim or withdrawal path independently.
pub(super) fn claim(program: &Pubkey, a: &[AccountInfo]) -> ProgramResult {
    let mut agreement = load_agreement(program, &a[17])?;
    let mut agent = load_agent(program, &a[18])?;
    if agreement.source != *a[14].key || agreement.reserve != *a[18].key || agreement.reward_settled
    {
        return invalid();
    }
    ensure_collateral(program, &a[0], &a[20], &a[12], &agent.authority)?;
    // Check every recipient even on a zero reward; absent state is not evidence
    // of zero entitlement. The normal finalized schedule/custody checks apply.
    load_canonical_user_collateral(program, &a[19], &agreement.funder)?;
    load_canonical_user_collateral(program, &a[21], a[18].key)?;
    let amount = super::super::oracle_usdc_rewards::process_claim_oracle_usdc_reward_for(
        program,
        &a[..17],
        ClaimOracleUsdcRewardParams {
            kind: OracleUsdcRewardKind::SourceProposer,
        },
        a[17].key,
        true,
    )?;
    let (sponsor, cash, reserve) =
        split_bounty(amount, agreement.researcher_bps, agreement.retention_bps)
            .ok_or(VaultError::InvalidOracleState)?;
    move_cash(
        program,
        &a[9],
        a[17].key,
        &a[19],
        &agreement.funder,
        sponsor,
    )?;
    move_cash(program, &a[9], a[17].key, &a[20], &agent.authority, cash)?;
    move_cash(program, &a[9], a[17].key, &a[21], a[18].key, reserve)?;
    agent.earned = add(agent.earned, add(cash, reserve)?)?;
    agreement.reward_settled = true;
    agreement.reward_paid = amount;
    store_state(&a[17], &agreement)?;
    store_state(&a[18], &agent)
}
