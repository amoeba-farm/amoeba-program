//! Tag 116: publish a month's settlement once its signers attest to it. The month becomes
//! Settled and the period Live -> Settled with the attested value.

use super::*;
use crate::instruction::SettlementLeaf;

/// Accounts: 0 [signer, writable] publisher and payer of the record; 1 config;
/// 2 [writable] period; 3 [writable] month; 4 [writable] settlement record PDA; 5 recipe-weight
/// manifest; 6 active-weight manifest; 7 settlement-source manifest; 8 signer registry;
/// 9 current signer set; 10 instructions sysvar; 11 system program. The threshold of Ed25519
/// attestations precedes this instruction in the transaction.
#[inline(never)]
pub(super) fn process_upsert_settlement(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    mut settlement: Box<SettlementLeaf>,
) -> ProgramResult {
    if accounts.len() != 12 {
        return Err(OracleError::InvalidAccountList.into());
    }
    let authority_info = &accounts[0];
    let config_info = &accounts[1];
    let period_info = &accounts[2];
    let oracle_month_info = &accounts[3];
    let settlement_record_info = &accounts[4];
    let recipe_manifest_info = &accounts[5];
    let active_manifest_info = &accounts[6];
    let source_manifest_info = &accounts[7];
    let signer_registry_info = &accounts[8];
    let signer_set_info = &accounts[9];
    let instructions_sysvar_info = &accounts[10];
    let system_program_info = &accounts[11];

    if !authority_info.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !crate::is_system_program(system_program_info.key) {
        return Err(OracleError::InvalidSystemProgram.into());
    }
    if *instructions_sysvar_info.key != instructions::id() {
        return Err(OracleError::InvalidInstructionsSysvar.into());
    }

    let config = Box::new(load_canonical_vault_config(program_id, config_info)?);
    if config.paused {
        return Err(OracleError::ContractPaused.into());
    }
    let (anchor, oracle_month) =
        load_valid_period_and_oracle_month(program_id, period_info, oracle_month_info)?;
    let mut oracle_month = Box::new(oracle_month);
    let recipe_manifest = Box::new(load_valid_oracle_recipe_weight_manifest(
        program_id,
        oracle_month_info.key,
        recipe_manifest_info,
    )?);
    let source_manifest = Box::new(load_valid_oracle_settlement_source_manifest(
        program_id,
        oracle_month_info.key,
        source_manifest_info,
    )?);
    let active_manifest = Box::new(load_valid_oracle_active_weight_manifest(
        program_id,
        oracle_month_info.key,
        active_manifest_info,
    )?);
    validate_canonical_settlement_provenance(
        oracle_month_info.key,
        &oracle_month,
        &recipe_manifest,
        &active_manifest,
        &source_manifest,
        &settlement,
    )?;
    validate_settlement_period_binding(&anchor, &settlement)?;
    ensure_settlement_finalization_ready(&anchor)?;
    let registry = Box::new(load_canonical_settlement_signer_registry(
        program_id,
        signer_registry_info,
    )?);
    let signer_set = Box::new(load_canonical_settlement_signer_set(
        program_id,
        signer_registry_info.key,
        signer_set_info,
    )?);
    if registry.recovery_authority == config.admin
        || registry.recovery_authority == config.oracle_authority
        || registry.current_set != *signer_set_info.key
    {
        return Err(OracleError::SettlementSignerGovernanceRequired.into());
    }
    settlement_signer_set_excludes_governance(
        &signer_set,
        &[
            config.admin,
            config.oracle_authority,
            registry.recovery_authority,
        ],
    )?;
    let _signer_bitmap = verify_settlement_oracle_attestations(
        program_id,
        &settlement,
        signer_registry_info,
        signer_set_info,
        instructions_sysvar_info,
    )?;
    let (submitted_slot, now) = current_slot_and_unix_timestamp()?;
    if now < settlement.settlement_ts {
        return Err(OracleError::PeriodNotExpired.into());
    }
    validate_settlement_leaf(&settlement, &oracle_month)?;
    let signed_leaf_commitment = settlement_signed_leaf_commitment(&settlement)?;

    let (existing_record, record_bump) = load_or_create_settlement_record_v2(
        program_id,
        authority_info,
        period_info,
        oracle_month_info,
        settlement_record_info,
        system_program_info,
    )?;
    let existing_record = existing_record.map(Box::new);

    if let Some(record) = existing_record.as_ref() {
        // A replay finds the record, the month and the period settled exactly as submitted.
        if !settlement_record_v2_matches_submission(
            record,
            period_info.key,
            oracle_month_info.key,
            &settlement,
            &signed_leaf_commitment,
            authority_info.key,
        ) || oracle_month.phase != OraclePhase::Settled
            || oracle_month.settlement_record != Some(*settlement_record_info.key)
            || anchor.period.status != PeriodStatus::Settled
            || anchor.period.settlement_value != record.settlement_price_atomic
            || anchor.period.settlement_digest != record.signed_leaf_commitment
        {
            return Err(OracleError::SettlementRecordImmutable.into());
        }
        return Ok(());
    }

    if oracle_month.phase != OraclePhase::Game {
        return Err(OracleError::SettlementRecordImmutable.into());
    }
    ensure_oracle_month_ready_for_settlement(&oracle_month)?;
    let settlement_value = index_delta_settlement_value(
        oracle_month.settlement_base_oracle_atomic,
        oracle_month.index_delta_bps,
    )?;
    settlement.submitted_by = *authority_info.key;
    settlement.submitted_slot = submitted_slot;
    let settlement_record = settlement_record_v2_from_leaf(
        record_bump,
        *period_info.key,
        *oracle_month_info.key,
        &settlement,
        signed_leaf_commitment,
        *authority_info.key,
    );
    store_state(settlement_record_info, &settlement_record)?;
    write_period_settled(
        program_id,
        period_info,
        &anchor,
        settlement_value,
        now,
        signed_leaf_commitment,
    )?;
    oracle_month.phase = OraclePhase::Settled;
    oracle_month.settlement_record = Some(*settlement_record_info.key);
    oracle_month.last_updated_slot = submitted_slot;
    store_oracle_month_state(oracle_month_info, oracle_month.as_ref())
}
