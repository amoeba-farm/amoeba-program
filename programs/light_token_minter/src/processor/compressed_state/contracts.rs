use super::*;

macro_rules! required_accesses {
    ($($item:expr),* $(,)?) => {
        RequiredAccesses::from_slice(&[$($item),*])
    };
}

#[inline(never)]
fn source_pair_walk(
    start: usize,
    end: usize,
    stride: usize,
    source_kind: RequiredAccessKind,
) -> Result<RequiredAccesses, ProgramError> {
    let mut accesses = RequiredAccesses::from_slice(&[]);
    let mut account_index = start;
    while account_index < end {
        let index = u8::try_from(account_index)
            .map_err(|_| ProgramError::from(VaultError::InvalidCompressionWitness))?;
        accesses.extend_from_slice(&[
            RequiredAccessSpec::new(index, CompressedStateDomain::OracleSourceState, source_kind),
            RequiredAccessSpec::new(
                index,
                CompressedStateDomain::OracleSourceDescriptor,
                RequiredAccessKind::ReadOnly,
            ),
        ]);
        account_index = account_index
            .checked_add(stride)
            .ok_or(VaultError::ArithmeticOverflow)?;
    }
    Ok(accesses)
}

pub(in crate::processor) fn validate_compressed_initializer_contract(
    params: &ExecuteCompressedStateParams,
) -> ProgramResult {
    let required = build_required_access_contract(
        &params.inner_instruction,
        usize::from(params.core_account_count),
    )?;
    if params.accesses.len() != required.len()
        || params
            .accesses
            .iter()
            .zip(required.as_slice())
            .any(|(actual, expected)| !access_matches(actual, *expected))
    {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    Ok(())
}

pub(in crate::processor) fn build_required_access_contract(
    inner_instruction: &[u8],
    core_account_count: usize,
) -> Result<RequiredAccesses, ProgramError> {
    let (tag_byte, payload) = inner_instruction
        .split_first()
        .ok_or(VaultError::InvalidInstructionData)?;
    let tag =
        VaultInstructionTag::from_byte(*tag_byte).ok_or(VaultError::InvalidInstructionData)?;
    let spec = RequiredAccessSpec::new;
    let sku = CompressedStateDomain::OracleUsdcSkuPool;
    let reward = CompressedStateDomain::OracleUsdcSourceReward;
    let coverage = CompressedStateDomain::OracleSkuCoverageRecord;
    let registration = CompressedStateDomain::OracleUsdcRewardRegistration;
    let receipt = CompressedStateDomain::OracleUsdcRewardReceipt;
    let winning_vote = CompressedStateDomain::OracleSambaWinningVote;
    let vote_receipt = CompressedStateDomain::OracleSambaVoteSettlementReceipt;
    let support = CompressedStateDomain::OracleSupportPosition;
    let source = CompressedStateDomain::OracleSourceState;
    let source_descriptor = CompressedStateDomain::OracleSourceDescriptor;
    let read = RequiredAccessKind::ReadOnly;
    let mutable = RequiredAccessKind::Mutable;
    let initialize = RequiredAccessKind::Initialize;
    let mutable_or_initialize = RequiredAccessKind::MutableOrInitialize;

    let source_pair = |account_index, kind| {
        required_accesses![
            spec(account_index, source, kind),
            spec(account_index, source_descriptor, read),
        ]
    };

    let required = match tag {
        VaultInstructionTag::OracleCarryForwardV1 => {
            super::super::oracle_carry::required_accesses(payload, core_account_count)?
        }
        VaultInstructionTag::AddOracleUsdcSkuBudget => required_accesses![spec(5, sku, initialize)],
        VaultInstructionTag::ProposeOracleSourceV3 => required_accesses![
            spec(4, sku, read),
            spec(5, source, initialize),
            spec(5, source_descriptor, initialize),
            spec(7, reward, initialize),
        ],
        VaultInstructionTag::SupportOracleSourceV3 => required_accesses![
            spec(4, sku, read),
            spec(5, source, mutable),
            spec(6, reward, mutable),
            spec(8, support, initialize),
            spec(11, coverage, mutable_or_initialize),
        ],
        VaultInstructionTag::ChallengeOracleSourceV2 => {
            let mut accesses = required_accesses![spec(3, sku, read), spec(4, source, read)];
            match core_account_count {
                10 => {}
                12 => accesses.push(spec(9, source, read)),
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
            accesses
        }
        VaultInstructionTag::SubmitOracleOpeningClaimV2 => {
            let mut accesses = required_accesses![spec(3, sku, read)];
            accesses.extend(source_pair(4, mutable));
            accesses
        }
        VaultInstructionTag::ChallengeOracleOpeningClaimV2 => {
            let mut accesses = required_accesses![spec(3, sku, read)];
            accesses.extend(source_pair(4, read));
            accesses
        }
        VaultInstructionTag::ResolveOracleOpeningClaimChallengeV2 => {
            let mut accesses = required_accesses![spec(3, sku, read)];
            accesses.extend(source_pair(4, mutable));
            accesses
        }
        VaultInstructionTag::FinalizeOracleOpeningClaimV2 => {
            require_empty_inner_payload(payload)?;
            source_pair(3, mutable)
        }
        VaultInstructionTag::CommitOracleUpdateClaimV3 => {
            required_accesses![spec(3, source, read), spec(5, sku, read)]
        }
        VaultInstructionTag::SettleExpiredOracleUpdateCommitmentV3 => {
            require_empty_inner_payload(payload)?;
            required_accesses![spec(3, source, read)]
        }
        VaultInstructionTag::ChallengeOracleUpdateClaimV2 => {
            required_accesses![spec(3, sku, read), spec(5, source, read)]
        }
        VaultInstructionTag::RevealOracleUpdateClaimV3 => source_pair(3, read),
        VaultInstructionTag::FinalizeOracleUpdateClaimV2 => {
            required_accesses![spec(4, source, mutable)]
        }
        VaultInstructionTag::CancelStaleOracleUpdateClaimV2 => {
            require_empty_inner_payload(payload)?;
            required_accesses![spec(3, source, read)]
        }
        VaultInstructionTag::TimeoutUnsupportedOracleSourceV2 => {
            require_empty_inner_payload(payload)?;
            required_accesses![spec(4, source, mutable)]
        }
        VaultInstructionTag::RegisterOracleUsdcRewardSource => {
            require_empty_inner_payload(payload)?;
            let mut accesses = required_accesses![spec(3, sku, mutable)];
            accesses.extend(source_pair(5, read));
            accesses.push(spec(6, reward, mutable));
            accesses
        }
        VaultInstructionTag::RegisterOracleUsdcRewardUpdate => {
            require_empty_inner_payload(payload)?;
            required_accesses![
                spec(3, sku, mutable),
                spec(4, source, read),
                spec(6, registration, initialize),
            ]
        }
        VaultInstructionTag::SettleOracleUsdcEscrow
        | VaultInstructionTag::SettleFailedOracleMonthEscrowV2 => {
            let kind = require_single_inner_payload_byte(payload)?;
            match kind {
                0 | 2 => required_accesses![spec(4, source, mutable), spec(5, reward, mutable),],
                1 => required_accesses![spec(4, source, read), spec(5, support, mutable)],
                3 | 4 => source_pair(3, read),
                5 => required_accesses![spec(3, source, read)],
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
        }
        VaultInstructionTag::ClaimOracleUsdcReward => {
            let kind = require_single_inner_payload_byte(payload)?;
            if kind > 3 {
                return Err(VaultError::InvalidInstructionData.into());
            }
            let mut accesses =
                required_accesses![spec(10, receipt, initialize), spec(13, sku, mutable)];
            if kind <= 2 {
                if kind == 2 {
                    accesses.extend(source_pair(14, read));
                } else {
                    accesses.push(spec(14, source, read));
                }
                accesses.push(spec(15, reward, read));
            } else {
                accesses.push(spec(14, registration, read));
            }
            if kind == 1 {
                let trailing_length = core_account_count
                    .checked_sub(13)
                    .ok_or(VaultError::InvalidCompressionWitness)?;
                if trailing_length < 4 || !(trailing_length - 4).is_multiple_of(2) {
                    return Err(VaultError::InvalidCompressionWitness.into());
                }
                accesses.push(spec(
                    u8::try_from(core_account_count - 1)
                        .map_err(|_| VaultError::InvalidCompressionWitness)?,
                    support,
                    read,
                ));
            }
            accesses
        }
        VaultInstructionTag::ResolveOracleSourceChallengeV2 => {
            let outcome = require_single_inner_payload_byte(payload)?;
            let outcome_account_count = match outcome {
                0 | 1 => 0usize,
                2 => 2,
                3 => 4,
                _ => return Err(VaultError::InvalidInstructionData.into()),
            };
            let fixed_count = 7usize
                .checked_add(outcome_account_count)
                .ok_or(VaultError::ArithmeticOverflow)?;
            let guard_count = core_account_count
                .checked_sub(fixed_count)
                .ok_or(VaultError::InvalidCompressionWitness)?;
            if guard_count != 1 && guard_count != 2 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            let mut accesses = required_accesses![spec(3, source, mutable)];
            if outcome == 3 {
                accesses.extend_from_slice(&[
                    spec(5, source, mutable),
                    spec(6, sku, read),
                    spec(7, reward, mutable),
                    spec(8, reward, mutable),
                ]);
            }
            let record_index = 6usize
                .checked_add(outcome_account_count)
                .and_then(|value| value.checked_add(guard_count))
                .ok_or(VaultError::ArithmeticOverflow)?;
            accesses.push(spec(
                u8::try_from(record_index).map_err(|_| VaultError::InvalidCompressionWitness)?,
                coverage,
                mutable,
            ));
            accesses
        }
        VaultInstructionTag::ExpireUnlistableOracleSourceV2 => {
            require_empty_inner_payload(payload)?;
            match core_account_count {
                5 => required_accesses![spec(4, source, mutable)],
                6 => required_accesses![spec(4, source, mutable), spec(5, coverage, mutable)],
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
        }
        VaultInstructionTag::ExpireOracleOpeningSource => {
            require_empty_inner_payload(payload)?;
            required_accesses![spec(3, source, mutable)]
        }
        VaultInstructionTag::AccumulateOracleRecipeBucketV2 => {
            if !(6..=9).contains(&core_account_count) {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            source_pair_walk(5, core_account_count, 1, mutable)?
        }
        VaultInstructionTag::AccumulateOracleActiveWeightGroup => {
            if !(10..=9 + crate::constants::MAX_ORACLE_ACTIVE_WEIGHT_SOURCES_PER_STEP)
                .contains(&core_account_count)
            {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            // The completed recipe/bucket source indexes are ordinary read-only tail accounts.
            let mut accesses = required_accesses![spec(5, sku, read)];
            accesses.extend(source_pair_walk(7, core_account_count - 2, 1, read)?);
            accesses
        }
        VaultInstructionTag::RecomputeOracleBucketMedianV1 => {
            if payload.len() != 33 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            if super::super::bucket_medians::recompute_core_account_count(payload[32])
                != Some(core_account_count)
            {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            required_accesses![spec(
                super::super::bucket_medians::RECOMPUTE_SOURCE_ACCOUNT_INDEX,
                source,
                read,
            )]
        }
        VaultInstructionTag::TryOpenOracleEmergencyDisputeV2 => {
            let kind = payload
                .first()
                .copied()
                .ok_or(VaultError::InvalidInstructionData)?;
            match kind {
                0 => match core_account_count {
                    15 => required_accesses![spec(13, source, read)],
                    17 => required_accesses![spec(13, source, read), spec(15, source, read)],
                    _ => return Err(VaultError::InvalidCompressionWitness.into()),
                },
                1 if core_account_count == 19 => required_accesses![spec(14, source, read)],
                2 if core_account_count == 15 => source_pair(14, read),
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
        }
        VaultInstructionTag::ResolveOracleEmergencyDisputeV2 => {
            require_empty_inner_payload(payload)?;
            match core_account_count {
                10 => source_pair(9, mutable),
                17 => required_accesses![spec(9, source, mutable)],
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
        }
        VaultInstructionTag::ResolveOracleEmergencyDisputeV4 => {
            require_empty_inner_payload(payload)?;
            match core_account_count {
                12 => required_accesses![spec(8, source, mutable), spec(11, coverage, mutable)],
                17 => required_accesses![
                    spec(8, source, mutable),
                    spec(10, source, mutable),
                    spec(12, sku, read),
                    spec(13, reward, mutable),
                    spec(14, reward, mutable),
                    spec(16, coverage, mutable),
                ],
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
        }
        VaultInstructionTag::AbortStaleOracleUpdateEmergencyDisputeV2 => {
            require_empty_inner_payload(payload)?;
            required_accesses![spec(3, source, read)]
        }
        VaultInstructionTag::RegisterOracleSambaWinningVote => {
            require_empty_inner_payload(payload)?;
            if core_account_count != 6 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            required_accesses![spec(4, winning_vote, initialize)]
        }
        VaultInstructionTag::SettleOracleSambaEmergencyVoteV2 => {
            require_empty_inner_payload(payload)?;
            match core_account_count {
                10 => required_accesses![spec(7, vote_receipt, initialize)],
                11 => required_accesses![
                    spec(7, vote_receipt, initialize),
                    spec(10, winning_vote, read),
                ],
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
        }
        _ => return Err(VaultError::InvalidCompressionWitness.into()),
    };
    Ok(required)
}

pub(in crate::processor) fn require_empty_inner_payload(payload: &[u8]) -> ProgramResult {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(VaultError::InvalidInstructionData.into())
    }
}

pub(in crate::processor) fn require_single_inner_payload_byte(
    payload: &[u8],
) -> Result<u8, ProgramError> {
    if let [value] = payload {
        Ok(*value)
    } else {
        Err(VaultError::InvalidInstructionData.into())
    }
}

pub(in crate::processor) fn access_matches(access: &CompressedStateAccess, required: u16) -> bool {
    let (account_index, domain, kind) = match access {
        CompressedStateAccess::ReadOnly {
            account_index,
            leaf,
            ..
        } => (*account_index, leaf.domain, 0u8),
        CompressedStateAccess::Mutable {
            account_index,
            leaf,
            ..
        } => (*account_index, leaf.domain, 1u8),
        CompressedStateAccess::Initialize {
            account_index,
            domain,
            ..
        } => (*account_index, *domain, 2u8),
    };
    let required_kind = (required >> 12) as u8;
    account_index == required as u8
        && domain as u8 == ((required >> 8) & 0xf) as u8
        && (kind == required_kind || (required_kind == 3 && kind != 0))
}
