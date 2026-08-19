use super::*;

#[inline(never)]
pub(in crate::processor) fn process_execute_compressed_state_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    mut params: ExecuteCompressedStateParams,
) -> ProgramResult {
    validate_session_shape(accounts, &params)?;
    validate_compressed_initializer_contract(&params)?;
    let core_count = usize::from(params.core_account_count);
    let (core_accounts, suffix) = accounts.split_at(core_count);
    let (system_program_info, light_accounts) =
        suffix.split_first().ok_or(VaultError::InvalidAccountList)?;
    let rent_payer = core_accounts
        .get(usize::from(params.rent_payer_index))
        .ok_or(VaultError::InvalidAccountList)?;

    // Validate the complete session before creating even a temporary view. Transaction rollback
    // is still authoritative, but fail-first ordering makes malformed proofs deterministic.
    for access in &mut params.accesses {
        let target = core_accounts
            .get(usize::from(access.account_index()))
            .ok_or(VaultError::InvalidAccountList)?;
        if target.key == rent_payer.key || !target.is_writable {
            return Err(VaultError::InvalidAccountList.into());
        }
        hydrate_existing_access(program_id, target.key, access);
        match access {
            CompressedStateAccess::ReadOnly { leaf, .. }
            | CompressedStateAccess::Mutable { leaf, .. } => {
                validate_leaf_target(target, leaf)?;
                super::super::validate_create_only_program_account_target(program_id, target)?;
            }
            CompressedStateAccess::Initialize { .. } => {
                super::super::validate_create_only_program_account_target(program_id, target)?;
            }
        }
    }

    for access in &params.accesses {
        match access {
            CompressedStateAccess::ReadOnly {
                account_index,
                leaf,
                ..
            }
            | CompressedStateAccess::Mutable {
                account_index,
                leaf,
                ..
            } => materialize_leaf(
                program_id,
                rent_payer,
                core_accounts,
                &core_accounts[usize::from(*account_index)],
                system_program_info,
                leaf,
            )?,
            CompressedStateAccess::Initialize { .. } => {}
        }
    }

    // This is the unchanged current transition function. Only its storage representation has
    // been adapted around it.
    super::super::process_compressed_inner_instruction(
        program_id,
        core_accounts,
        &params.inner_instruction,
    )?;

    capture_apply_and_close(
        program_id,
        core_accounts,
        rent_payer,
        light_accounts,
        &params,
    )
}

#[inline(never)]
pub(in crate::processor) fn capture_apply_and_close<'a>(
    program_id: &Pubkey,
    core_accounts: &[AccountInfo<'a>],
    rent_payer: &AccountInfo<'a>,
    light_accounts: &[AccountInfo<'a>],
    params: &ExecuteCompressedStateParams,
) -> ProgramResult {
    let mut read_only_captures = CaptureVec::new();
    let mut mutable_captures = CaptureVec::new();
    let mut close_captures = CaptureVec::new();
    let mut initialize_captures = CaptureVec::new();
    let mut retained_classic_mask = 0u8;
    for (access_position, access) in params.accesses.iter().enumerate() {
        let target = &core_accounts[usize::from(access.account_index())];
        match access {
            CompressedStateAccess::ReadOnly { meta, leaf, .. } => {
                let captured = capture_leaf(program_id, target, leaf.domain, leaf.revision)?;
                if captured.data != leaf.data {
                    return Err(VaultError::InvalidCompressionWitness.into());
                }
                read_only_captures.push(CompressedStateLeafReadOnly { meta, leaf });
            }
            CompressedStateAccess::Mutable { meta, leaf, .. } => {
                let revision = leaf
                    .revision
                    .checked_add(1)
                    .ok_or(VaultError::ArithmeticOverflow)?;
                let captured = capture_leaf(program_id, target, leaf.domain, revision)?;
                if retain_classic_merged_reward(target, leaf.domain)? {
                    close_captures.push(CompressedStateLeafClose {
                        meta,
                        old_leaf: leaf,
                    });
                    retained_classic_mask |= 1u8 << access_position;
                } else {
                    mutable_captures.push(CompressedStateLeafUpdate {
                        meta,
                        old_leaf: leaf,
                        new_leaf: captured,
                    });
                }
            }
            CompressedStateAccess::Initialize { domain, output, .. } => {
                initialize_captures.push(CompressedStateLeafCreate {
                    output,
                    leaf: capture_leaf(program_id, target, *domain, 0)?,
                });
            }
        }
    }

    apply_compressed_state_leaf_mutations(
        program_id,
        rent_payer,
        light_accounts,
        &params.proof,
        read_only_captures.as_slice(),
        mutable_captures.as_slice(),
        close_captures.as_slice(),
        initialize_captures.as_slice(),
    )?;

    let mut last_closed_index = None;
    for (access_position, access) in params.accesses.iter().enumerate() {
        if retained_classic_mask & (1u8 << access_position) != 0 {
            continue;
        }
        if last_closed_index == Some(access.account_index()) {
            continue;
        }
        let target = &core_accounts[usize::from(access.account_index())];
        super::super::close_program_account(program_id, target, rent_payer)?;
        last_closed_index = Some(access.account_index());
    }
    Ok(())
}

pub(in crate::processor) fn hydrate_existing_access(
    program_id: &Pubkey,
    canonical_pda: &Pubkey,
    access: &mut CompressedStateAccess,
) {
    match access {
        CompressedStateAccess::ReadOnly { meta, leaf, .. } => {
            leaf.canonical_pda = *canonical_pda;
            meta.address = derive_compressed_state_leaf_address(
                program_id,
                &crate::constants::LIGHT_DEFAULT_ADDRESS_TREE_V2,
                leaf.domain,
                canonical_pda,
            )
            .0;
        }
        CompressedStateAccess::Mutable { meta, leaf, .. } => {
            leaf.canonical_pda = *canonical_pda;
            meta.address = derive_compressed_state_leaf_address(
                program_id,
                &crate::constants::LIGHT_DEFAULT_ADDRESS_TREE_V2,
                leaf.domain,
                canonical_pda,
            )
            .0;
        }
        CompressedStateAccess::Initialize { .. } => {}
    }
}

pub(in crate::processor) fn retain_classic_merged_reward(
    target: &AccountInfo,
    domain: CompressedStateDomain,
) -> Result<bool, ProgramError> {
    if domain != CompressedStateDomain::OracleUsdcSourceReward {
        return Ok(false);
    }
    let data = target.try_borrow_data()?;
    Ok(!fixed_bytes32_is_zero(&data, 244))
}

pub(in crate::processor) fn validate_session_shape(
    accounts: &[AccountInfo],
    params: &ExecuteCompressedStateParams,
) -> ProgramResult {
    let core_count = usize::from(params.core_account_count);
    if core_count == 0
        || accounts.len() <= core_count
        || params.accesses.is_empty()
        || params.accesses.len() > MAX_COMPRESSED_STATE_SESSION_RECORDS
        || params.inner_instruction.is_empty()
        || params.inner_instruction.len() > MAX_COMPRESSED_INNER_INSTRUCTION_BYTES
        || usize::from(params.rent_payer_index) >= core_count
    {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let inner_tag = VaultInstructionTag::from_byte(params.inner_instruction[0])
        .ok_or(VaultError::InvalidInstructionData)?;
    if inner_tag == VaultInstructionTag::ExecuteCompressedStateV1 {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let payer = &accounts[usize::from(params.rent_payer_index)];
    if !payer.is_signer || !payer.is_writable || *accounts[core_count].key != system_program::id() {
        return Err(VaultError::InvalidAccountList.into());
    }
    let mut previous_access: Option<&CompressedStateAccess> = None;
    for access in &params.accesses {
        let index = usize::from(access.account_index());
        if index >= core_count
            || previous_access.is_some_and(|previous| index < usize::from(previous.account_index()))
        {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        if previous_access.is_some_and(|previous| {
            previous.account_index() == access.account_index()
                && !is_source_descriptor_pair(previous, access)
        }) {
            return Err(VaultError::InvalidCompressionWitness.into());
        }
        previous_access = Some(access);
    }
    Ok(())
}

pub(in crate::processor) fn is_source_descriptor_pair(
    source: &CompressedStateAccess,
    descriptor: &CompressedStateAccess,
) -> bool {
    source.domain() == CompressedStateDomain::OracleSourceState
        && descriptor.domain() == CompressedStateDomain::OracleSourceDescriptor
        && matches!(
            (source, descriptor),
            (
                CompressedStateAccess::ReadOnly { .. },
                CompressedStateAccess::ReadOnly { .. }
            ) | (
                CompressedStateAccess::Mutable { .. },
                CompressedStateAccess::ReadOnly { .. }
            ) | (
                CompressedStateAccess::Initialize { .. },
                CompressedStateAccess::Initialize { .. }
            )
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::processor) enum RequiredAccessKind {
    ReadOnly,
    Mutable,
    Initialize,
    MutableOrInitialize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::processor) struct RequiredAccessSpec {
    pub(in crate::processor) account_index: u8,
    pub(in crate::processor) domain: CompressedStateDomain,
    pub(in crate::processor) kind: RequiredAccessKind,
}

impl RequiredAccessSpec {
    pub(in crate::processor) const fn new(
        account_index: u8,
        domain: CompressedStateDomain,
        kind: RequiredAccessKind,
    ) -> Self {
        Self {
            account_index,
            domain,
            kind,
        }
    }

    pub(in crate::processor) const fn packed(self) -> u16 {
        (self.account_index as u16) | ((self.domain as u16) << 8) | ((self.kind as u16) << 12)
    }
}

pub(in crate::processor) const MAX_REQUIRED_ACCESSES: usize = 8;

#[derive(Clone, Copy)]
pub(in crate::processor) struct RequiredAccesses {
    items: [u16; MAX_REQUIRED_ACCESSES],
    len: u8,
}

impl RequiredAccesses {
    #[inline(never)]
    pub(in crate::processor) fn from_slice(items: &[RequiredAccessSpec]) -> Self {
        let mut required = Self {
            items: [0; MAX_REQUIRED_ACCESSES],
            len: items.len() as u8,
        };
        for (target, item) in required.items.iter_mut().zip(items) {
            *target = item.packed();
        }
        required
    }

    #[inline(never)]
    pub(in crate::processor) fn push(&mut self, item: RequiredAccessSpec) {
        let index = usize::from(self.len);
        self.items[index] = item.packed();
        self.len += 1;
    }

    #[inline(never)]
    pub(in crate::processor) fn extend(&mut self, other: Self) {
        for index in 0..other.len() {
            let target = usize::from(self.len);
            self.items[target] = other.items[index];
            self.len += 1;
        }
    }

    #[inline(never)]
    pub(in crate::processor) fn extend_from_slice(&mut self, items: &[RequiredAccessSpec]) {
        for item in items {
            self.push(*item);
        }
    }

    #[inline(always)]
    pub(in crate::processor) fn as_slice(&self) -> &[u16] {
        &self.items[..usize::from(self.len)]
    }

    #[inline(always)]
    pub(in crate::processor) fn len(&self) -> usize {
        usize::from(self.len)
    }
}
