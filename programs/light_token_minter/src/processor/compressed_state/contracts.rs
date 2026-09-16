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
    if params.inner_instruction.first().copied()
        == Some(VaultInstructionTag::RecomputeOracleBucketMedianV1 as u8)
    {
        return validate_recompute_access_contract(params);
    }
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

/// Recompute has a state-bound optional observations witness. The source leaf's authenticated
/// status selects the contract: active sources require the observations proof, while inactive
/// sources require the canonical absent observations account and therefore omit that proof. This
/// is selected from the proof-bound source bytes rather than from caller omission.
#[inline(never)]
fn validate_recompute_access_contract(params: &ExecuteCompressedStateParams) -> ProgramResult {
    if params.inner_instruction.len() != 34 {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    let mode = params.inner_instruction[33];
    if super::super::bucket_medians::recompute_core_account_count(mode)
        != Some(usize::from(params.core_account_count))
    {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    let source_spec = RequiredAccessSpec::new(
        super::super::bucket_medians::RECOMPUTE_SOURCE_ACCOUNT_INDEX,
        CompressedStateDomain::OracleSourceState,
        RequiredAccessKind::ReadOnly,
    );
    let source = match params.accesses.iter().find(|access| {
        access.account_index() == source_spec.account_index
            && access.domain() == source_spec.domain
            && matches!(access, CompressedStateAccess::ReadOnly { .. })
    }) {
        Some(CompressedStateAccess::ReadOnly { leaf, .. }) => leaf,
        _ => return Err(VaultError::InvalidCompressionWitness.into()),
    };
    if source.data.len() != CompactOracleSourceState::REQUIRED_DATA_LEN {
        return Err(VaultError::InvalidCompressionWitness.into());
    }
    let expected = match source.data[130] {
        value
            if value == OracleSourceStatus::Active as u8
                && u32::from_le_bytes(
                    source.data[172..176]
                        .try_into()
                        .map_err(|_| VaultError::InvalidCompressionWitness)?,
                ) <= crate::constants::MAX_ORACLE_SOURCE_OBSERVATIONS as u32 =>
        {
            [
                source_spec.packed(),
                RequiredAccessSpec::new(
                    super::super::bucket_medians::RECOMPUTE_SOURCE_ACCOUNT_INDEX + 1,
                    CompressedStateDomain::OracleSourceObservations,
                    RequiredAccessKind::ReadOnly,
                )
                .packed(),
            ]
        }
        value
            if value == OracleSourceStatus::Inactive as u8
                || value == OracleSourceStatus::Active as u8 =>
        {
            [source_spec.packed(), 0]
        }
        _ => return Err(VaultError::InvalidCompressionWitness.into()),
    };
    let expected_len = if expected[1] == 0 { 1 } else { 2 };
    if params.accesses.len() != expected_len
        || params
            .accesses
            .iter()
            .zip(expected[..expected_len].iter())
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
    #[cfg(feature = "devnet-solo-backfill-2026")]
    if super::super::devnet_solo_backfill_2026::is_compressed_state_transport_tag(*tag_byte) {
        return build_required_backfill_access_contract(*tag_byte, payload, core_account_count);
    }
    let tag =
        VaultInstructionTag::from_byte(*tag_byte).ok_or(VaultError::InvalidInstructionData)?;
    let spec = RequiredAccessSpec::new;
    let sku = CompressedStateDomain::OracleUsdcSkuPool;
    let reward = CompressedStateDomain::OracleUsdcSourceReward;
    let coverage = CompressedStateDomain::OracleSkuCoverageRecord;
    let registration = CompressedStateDomain::OracleUsdcRewardRegistration;
    let receipt = CompressedStateDomain::OracleUsdcRewardReceipt;
    let support = CompressedStateDomain::OracleSupportPosition;
    let source = CompressedStateDomain::OracleSourceState;
    let source_descriptor = CompressedStateDomain::OracleSourceDescriptor;
    let observations = CompressedStateDomain::OracleSourceObservations;
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
            spec(6, observations, initialize),
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
            if core_account_count != 10 {
                return Err(VaultError::InvalidAccountList.into());
            }
            let mut accesses = source_pair(3, mutable);
            accesses.push(spec(4, observations, mutable));
            accesses.push(spec(
                7,
                CompressedStateDomain::OracleCarryJournal,
                mutable_or_initialize,
            ));
            accesses.push(spec(
                8,
                CompressedStateDomain::OracleCarryCheckpoint,
                initialize,
            ));
            accesses
        }
        VaultInstructionTag::CommitOracleUpdateClaimV3 => {
            required_accesses![spec(3, source, read), spec(5, sku, read)]
        }
        VaultInstructionTag::SettleExpiredOracleUpdateCommitmentV3 => {
            require_empty_inner_payload(payload)?;
            required_accesses![spec(3, source, read)]
        }
        VaultInstructionTag::ChallengeOracleUpdateClaimV2 => {
            let mut accesses = required_accesses![spec(3, sku, read)];
            accesses.extend(source_pair(5, read));
            accesses
        }
        VaultInstructionTag::RevealOracleUpdateClaimV3 => source_pair(3, read),
        VaultInstructionTag::FinalizeOracleUpdateClaimV2 => {
            if payload.len() != 9 {
                return Err(VaultError::InvalidInstructionData.into());
            }
            let mut accesses =
                required_accesses![spec(4, source, mutable), spec(5, observations, mutable),];
            match (payload[0], core_account_count) {
                (0, 13) => {
                    accesses.push(spec(
                        10,
                        CompressedStateDomain::OracleCarryJournal,
                        mutable_or_initialize,
                    ));
                    accesses.push(spec(
                        11,
                        CompressedStateDomain::OracleCarryCheckpoint,
                        initialize,
                    ));
                }
                (0 | 1, 14) => {
                    accesses.push(spec(
                        11,
                        CompressedStateDomain::OracleCarryJournal,
                        mutable_or_initialize,
                    ));
                    accesses.push(spec(
                        12,
                        CompressedStateDomain::OracleCarryCheckpoint,
                        initialize,
                    ));
                }
                (2, 13) | (1, 11) => {}
                _ => return Err(VaultError::InvalidCompressionWitness.into()),
            }
            accesses
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
                2 => 0,
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
            if core_account_count != 12 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            // The completed recipe/bucket source indexes are ordinary read-only tail accounts.
            let mut accesses = required_accesses![spec(5, sku, read)];
            accesses.extend(source_pair_walk(7, core_account_count - 4, 1, read)?);
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
            required_accesses![
                spec(
                    super::super::bucket_medians::RECOMPUTE_SOURCE_ACCOUNT_INDEX,
                    source,
                    read,
                ),
                spec(
                    super::super::bucket_medians::RECOMPUTE_SOURCE_ACCOUNT_INDEX + 1,
                    observations,
                    read,
                )
            ]
        }

        _ => return Err(VaultError::InvalidCompressionWitness.into()),
    };
    Ok(required)
}

#[cfg(feature = "devnet-solo-backfill-2026")]
#[inline(never)]
fn build_required_backfill_access_contract(
    tag: u8,
    payload: &[u8],
    core_account_count: usize,
) -> Result<RequiredAccesses, ProgramError> {
    let spec = RequiredAccessSpec::new;
    let source = CompressedStateDomain::OracleSourceState;
    let descriptor = CompressedStateDomain::OracleSourceDescriptor;
    let observations = CompressedStateDomain::OracleSourceObservations;
    let initialize = RequiredAccessKind::Initialize;
    let required = match tag {
        crate::governance_manifest::DEVNET_SOLO_BACKFILL_CREATE_SUPPORTED_SOURCE_TAG => {
            if payload.len() != 228 || core_account_count != 15 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            required_accesses![
                spec(8, source, initialize),
                spec(8, descriptor, initialize),
                spec(9, observations, initialize),
            ]
        }
        crate::governance_manifest::DEVNET_SOLO_BACKFILL_FINALIZE_OPENING_TAG => {
            if payload.len() != 36 || core_account_count != 13 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            required_accesses![
                spec(6, source, RequiredAccessKind::Mutable),
                spec(7, observations, RequiredAccessKind::Mutable),
                spec(
                    10,
                    CompressedStateDomain::OracleCarryJournal,
                    RequiredAccessKind::MutableOrInitialize,
                ),
                spec(
                    11,
                    CompressedStateDomain::OracleCarryCheckpoint,
                    RequiredAccessKind::Initialize,
                ),
            ]
        }
        crate::governance_manifest::DEVNET_SOLO_BACKFILL_ACCUMULATE_ACTIVE_WEIGHT_TAG => {
            if payload.len() != 36 || core_account_count != 12 {
                return Err(VaultError::InvalidCompressionWitness.into());
            }
            required_accesses![
                spec(7, source, RequiredAccessKind::ReadOnly),
                spec(8, observations, RequiredAccessKind::ReadOnly),
            ]
        }
        _ => return Err(VaultError::InvalidInstructionData.into()),
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
