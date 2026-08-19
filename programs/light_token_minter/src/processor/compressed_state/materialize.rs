use super::*;

pub(in crate::processor) fn validate_leaf_target(
    target: &AccountInfo,
    leaf: &CompressedAmebaStateLeaf,
) -> ProgramResult {
    if !leaf.has_canonical_envelope() || leaf.canonical_pda != *target.key {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    validate_compact_state_data(leaf.domain, &leaf.data)
}

pub(in crate::processor) fn materialize_leaf<'a>(
    program_id: &Pubkey,
    rent_payer: &AccountInfo<'a>,
    core_accounts: &[AccountInfo<'a>],
    target: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    leaf: &CompressedAmebaStateLeaf,
) -> ProgramResult {
    let full_data = match leaf.domain {
        CompressedStateDomain::OracleSkuCoverageRecord => {
            let mut compact = FixedCursor::new(&leaf.data);
            let sku_id = compact.bytes();
            let month_info = core_accounts
                .iter()
                .find(|candidate| {
                    derive_oracle_sku_coverage_record_pda(program_id, candidate.key, &sku_id).0
                        == *target.key
                })
                .ok_or(VaultError::InvalidOracleSkuCoverageRecord)?;
            let (_, bump) =
                derive_oracle_sku_coverage_record_pda(program_id, month_info.key, &sku_id);
            let mut data = vec![0; OracleSkuCoverageRecord::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            OracleSkuCoverageRecord::ACCOUNT_DISCRIMINATOR.write(&mut output);
            OracleSkuCoverageRecord::ACCOUNT_VERSION.write(&mut output);
            month_info.key.write(&mut output);
            output.raw(&leaf.data);
            debug_assert!(output.offset <= OracleSkuCoverageRecord::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleSkuCoverageRecord::LEN,
                &[
                    ORACLE_SKU_COVERAGE_RECORD_PDA_SEED,
                    month_info.key.as_ref(),
                    &sku_id,
                    &bump_seed,
                ],
            )?;
            data
        }
        CompressedStateDomain::OracleUsdcSkuPool => {
            let mut compact = FixedCursor::new(&leaf.data);
            let bucket_id = compact.bytes();
            let (schedule, month) = core_accounts
                .iter()
                .find_map(|candidate| {
                    let (schedule, _) =
                        derive_oracle_usdc_reward_schedule_pda(program_id, candidate.key);
                    let sku = derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &bucket_id).0;
                    (sku == *target.key).then_some((schedule, *candidate.key))
                })
                .ok_or(VaultError::InvalidOracleUsdcSkuPool)?;
            let (_, bump) = derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &bucket_id);
            let mut data = vec![0; OracleUsdcSkuPool::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            OracleUsdcSkuPool::ACCOUNT_DISCRIMINATOR.write(&mut output);
            OracleUsdcSkuPool::ACCOUNT_VERSION.write(&mut output);
            schedule.write(&mut output);
            month.write(&mut output);
            output.raw(&leaf.data);
            debug_assert!(output.offset <= OracleUsdcSkuPool::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleUsdcSkuPool::LEN,
                &[
                    ORACLE_USDC_SKU_POOL_PDA_SEED,
                    schedule.as_ref(),
                    &bucket_id,
                    &bump_seed,
                ],
            )?;
            data
        }
        CompressedStateDomain::OracleUsdcSourceReward => {
            let mut compact = FixedCursor::new(&leaf.data);
            let source_key = compact.pubkey();
            let source_info = core_accounts
                .iter()
                .find(|candidate| *candidate.key == source_key)
                .ok_or(VaultError::InvalidOracleUsdcSourceReward)?;
            let source = Box::new(super::super::load_self_valid_oracle_source(
                program_id,
                source_info,
            )?);
            let (schedule, _) = derive_oracle_usdc_reward_schedule_pda(program_id, &source.month);
            let (expected, bump) =
                derive_oracle_usdc_source_reward_pda(program_id, &schedule, source_info.key);
            if expected != *target.key {
                return Err(VaultError::InvalidOracleUsdcSourceReward.into());
            }
            let sku_pool =
                derive_oracle_usdc_sku_pool_pda(program_id, &schedule, &source.bucket_id).0;
            let mut data = vec![0; OracleUsdcSourceReward::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            OracleUsdcSourceReward::ACCOUNT_DISCRIMINATOR.write(&mut output);
            OracleUsdcSourceReward::ACCOUNT_VERSION.write(&mut output);
            source.month.write(&mut output);
            schedule.write(&mut output);
            sku_pool.write(&mut output);
            output.raw(&leaf.data[..32]);
            source.source_id.write(&mut output);
            source.proposer.write(&mut output);
            output.raw(&leaf.data[32..]);
            debug_assert!(output.offset <= OracleUsdcSourceReward::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleUsdcSourceReward::LEN,
                &[
                    ORACLE_USDC_SOURCE_REWARD_PDA_SEED,
                    schedule.as_ref(),
                    source_info.key.as_ref(),
                    &bump_seed,
                ],
            )?;
            data
        }
        CompressedStateDomain::OracleSupportPosition => {
            let mut compact = FixedCursor::new(&leaf.data);
            let source = compact.pubkey();
            let supporter = compact.pubkey();
            let (month, bump) = core_accounts
                .iter()
                .find_map(|candidate| {
                    let (expected, bump) = super::super::derive_oracle_support_pda(
                        program_id,
                        candidate.key,
                        &source,
                        &supporter,
                    );
                    (expected == *target.key).then_some((*candidate.key, bump))
                })
                .ok_or(VaultError::InvalidOracleState)?;
            let mut data = vec![0; OracleSupportPosition::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            month.write(&mut output);
            output.raw(&leaf.data[32..64]);
            output.raw(&leaf.data[..32]);
            output.raw(&leaf.data[75..]);
            output.raw(&leaf.data[64..75]);
            debug_assert!(output.offset <= OracleSupportPosition::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleSupportPosition::LEN,
                &[
                    ORACLE_SUPPORT_POSITION_PDA_SEED,
                    month.as_ref(),
                    source.as_ref(),
                    supporter.as_ref(),
                    &bump_seed,
                ],
            )?;
            data
        }
        CompressedStateDomain::OracleSourceState => {
            let mut compact = FixedCursor::new(&leaf.data);
            let source_id = compact.bytes();
            let month_info = core_accounts
                .iter()
                .find(|candidate| {
                    super::super::derive_oracle_source_pda(program_id, candidate.key, &source_id).0
                        == *target.key
                })
                .ok_or(VaultError::InvalidOracleSourceAccount)?;
            let (_, bump) =
                super::super::derive_oracle_source_pda(program_id, month_info.key, &source_id);
            let mut data = vec![0; OracleSourceState::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            month_info.key.write(&mut output);
            output.raw(&leaf.data[..64]);
            output.offset += 96;
            output.raw(&leaf.data[64..]);
            debug_assert!(output.offset <= OracleSourceState::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleSourceState::LEN,
                &[
                    ORACLE_SOURCE_PDA_SEED,
                    month_info.key.as_ref(),
                    &source_id,
                    &bump_seed,
                ],
            )?;
            data
        }
        CompressedStateDomain::OracleSourceDescriptor => {
            super::super::load_self_valid_oracle_source(program_id, target)?;
            let mut data = target.try_borrow_data()?.to_vec();
            data[98..194].copy_from_slice(&leaf.data);
            data
        }
        CompressedStateDomain::OracleUsdcRewardRegistration => {
            let mut compact = FixedCursor::new(&leaf.data);
            compact.pubkey();
            let subject = compact.pubkey();
            let mut identity = None;
            for candidate in core_accounts {
                let Ok(schedule) = load_self_valid_reward_schedule(program_id, candidate) else {
                    continue;
                };
                let (expected, bump) = derive_oracle_usdc_reward_registration_pda(
                    program_id,
                    candidate.key,
                    OracleUsdcRewardKind::Update,
                    &subject,
                );
                if expected == *target.key {
                    identity = Some((candidate, schedule, bump));
                    break;
                }
            }
            let (schedule_info, schedule, bump) =
                identity.ok_or(VaultError::InvalidOracleUsdcRewardRegistration)?;
            let mut data = vec![0; OracleUsdcRewardRegistration::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            OracleUsdcRewardRegistration::ACCOUNT_DISCRIMINATOR.write(&mut output);
            OracleUsdcRewardRegistration::ACCOUNT_VERSION.write(&mut output);
            schedule.month.write(&mut output);
            schedule_info.key.write(&mut output);
            output.raw(&leaf.data[..32]);
            OracleUsdcRewardKind::Update.write(&mut output);
            output.raw(&leaf.data[32..]);
            debug_assert!(output.offset <= OracleUsdcRewardRegistration::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            let kind_seed = [OracleUsdcRewardKind::Update as u8];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleUsdcRewardRegistration::LEN,
                &[
                    ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED,
                    ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED,
                    schedule_info.key.as_ref(),
                    &kind_seed,
                    subject.as_ref(),
                    &bump_seed,
                ],
            )?;
            data
        }
        // Paid receipts are create-only nullifiers. No current transition may consume or rewrite
        // an existing receipt, so materializing one would enlarge the replay surface.
        CompressedStateDomain::OracleUsdcRewardReceipt => {
            return Err(VaultError::InvalidOracleUsdcRewardReceipt.into());
        }
        CompressedStateDomain::OracleSambaWinningVote => {
            let dispute_info = core_accounts.get(1).ok_or(VaultError::InvalidAccountList)?;
            let pot_info = core_accounts.get(2).ok_or(VaultError::InvalidAccountList)?;
            let vote_info = core_accounts.get(5).ok_or(VaultError::InvalidAccountList)?;
            let dispute =
                super::super::oracle_samba_pot::load_dispute_v3(program_id, dispute_info)?;
            let pot = super::super::oracle_samba_pot::load_pot(
                program_id,
                dispute_info,
                &dispute,
                pot_info,
            )?;
            let vote = super::super::oracle_samba_pot::load_vote_v3(
                program_id,
                dispute_info,
                &dispute,
                pot_info,
                vote_info,
            )?;
            let (expected, bump) =
                derive_oracle_samba_winning_vote_pda(program_id, pot_info.key, vote_info.key);
            if expected != *target.key || pot.dispute != *dispute_info.key {
                return Err(VaultError::InvalidOracleEmergencyDispute.into());
            }
            let mut data = vec![0; OracleSambaWinningVote::LEN];
            let mut output = FixedWriter::new(&mut data);
            true.write(&mut output);
            bump.write(&mut output);
            OracleSambaWinningVote::ACCOUNT_DISCRIMINATOR.write(&mut output);
            OracleSambaWinningVote::ACCOUNT_VERSION.write(&mut output);
            pot_info.key.write(&mut output);
            dispute_info.key.write(&mut output);
            vote_info.key.write(&mut output);
            vote.voter.write(&mut output);
            vote.voting_power.write(&mut output);
            output.raw(&leaf.data);
            debug_assert!(output.offset <= OracleSambaWinningVote::LEN);
            drop(output);
            validate_typed_state_data(program_id, target.key, leaf.domain, &data, None)?;
            let bump_seed = [bump];
            super::super::create_program_account(
                rent_payer,
                target,
                system_program_info,
                program_id,
                OracleSambaWinningVote::LEN,
                &[
                    ORACLE_SAMBA_WINNING_VOTE_PDA_SEED,
                    pot_info.key.as_ref(),
                    vote_info.key.as_ref(),
                    &bump_seed,
                ],
            )?;
            data
        }
        CompressedStateDomain::OracleSambaVoteSettlementReceipt => {
            return Err(VaultError::InvalidOracleEmergencyDispute.into());
        }
    };
    let mut data = target.try_borrow_mut_data()?;
    if data.len() != full_data.len() {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    data.copy_from_slice(&full_data);
    Ok(())
}

pub(in crate::processor) fn load_self_valid_reward_schedule(
    program_id: &Pubkey,
    schedule_info: &AccountInfo,
) -> Result<OracleUsdcRewardSchedule, ProgramError> {
    let schedule: OracleUsdcRewardSchedule = super::super::load_exact_zero_padded_state(
        schedule_info,
        program_id,
        OracleUsdcRewardSchedule::LEN,
        VaultError::InvalidOracleUsdcRewardSchedule,
    )?;
    let (expected, bump) = derive_oracle_usdc_reward_schedule_pda(program_id, &schedule.month);
    if *schedule_info.key != expected
        || !schedule.is_initialized
        || schedule.bump != bump
        || schedule.account_discriminator != OracleUsdcRewardSchedule::ACCOUNT_DISCRIMINATOR
        || schedule.account_version != OracleUsdcRewardSchedule::ACCOUNT_VERSION
        || crate::pubkey_is_default(&schedule.month)
        || crate::pubkey_is_default(&schedule.authority)
        || schedule.reward_vault != derive_oracle_usdc_reward_vault_pda(program_id).0
    {
        return Err(VaultError::InvalidOracleUsdcRewardSchedule.into());
    }
    Ok(schedule)
}

pub(in crate::processor) fn capture_leaf(
    program_id: &Pubkey,
    target: &AccountInfo,
    domain: CompressedStateDomain,
    revision: u64,
) -> Result<CompressedAmebaStateLeaf, solana_program::program_error::ProgramError> {
    if target.owner != program_id || target.executable {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    let full_data = target.try_borrow_data()?;
    let mut data = Vec::new();
    validate_typed_state_data(program_id, target.key, domain, &full_data, Some(&mut data))?;
    Ok(CompressedAmebaStateLeaf {
        schema_version: CompressedAmebaStateLeaf::CURRENT_SCHEMA_VERSION,
        domain,
        canonical_pda: *target.key,
        revision,
        data,
    })
}

#[inline(never)]
pub(in crate::processor) fn capture_fixed_slices(
    data: &[u8],
    encoded_len: usize,
    ranges: &[(u16, u16)],
) -> Vec<u8> {
    let mut encoded = Vec::new();
    for &(start, end) in ranges {
        encoded.extend_from_slice(&data[usize::from(start)..usize::from(end)]);
    }
    debug_assert_eq!(encoded.len(), encoded_len);
    encoded
}

/// The compact leaves are strict projections of fixed-layout typed accounts. Copying their
/// canonical byte ranges is byte-identical to decoding a full account, rebuilding a compact
/// value, and serializing it again, while avoiding ten runtime compact encoders.
pub(in crate::processor) fn capture_compact_state_data(
    domain: CompressedStateDomain,
    data: &[u8],
) -> Vec<u8> {
    match domain {
        CompressedStateDomain::OracleSkuCoverageRecord => {
            capture_fixed_slices(data, 44, &[(38, 82)])
        }
        CompressedStateDomain::OracleUsdcSkuPool => capture_fixed_slices(data, 156, &[(70, 226)]),
        CompressedStateDomain::OracleUsdcSourceReward => {
            capture_fixed_slices(data, 112, &[(102, 134), (198, 278)])
        }
        CompressedStateDomain::OracleSupportPosition => {
            capture_fixed_slices(data, 107, &[(66, 98), (34, 66), (130, 141), (98, 130)])
        }
        CompressedStateDomain::OracleUsdcRewardRegistration => {
            capture_fixed_slices(data, 105, &[(70, 102), (103, 176)])
        }
        CompressedStateDomain::OracleUsdcRewardReceipt => {
            capture_fixed_slices(data, 81, &[(102, 135), (70, 102), (135, 151)])
        }
        CompressedStateDomain::OracleSambaWinningVote => {
            capture_fixed_slices(data, 16, &[(142, 158)])
        }
        CompressedStateDomain::OracleSambaVoteSettlementReceipt => {
            capture_fixed_slices(data, 17, &[(166, 183)])
        }
        CompressedStateDomain::OracleSourceState => {
            capture_fixed_slices(data, 205, &[(34, 98), (194, 335)])
        }
        CompressedStateDomain::OracleSourceDescriptor => {
            capture_fixed_slices(data, 96, &[(98, 194)])
        }
    }
}
