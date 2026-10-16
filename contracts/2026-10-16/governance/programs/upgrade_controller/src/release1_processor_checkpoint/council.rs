use super::*;

pub(super) fn require_active_seat_authority(
    council: &GovernanceCouncilSetV1,
    authority: &Pubkey,
    slot: u64,
) -> GovernanceResult<()> {
    let seat = council
        .seats
        .iter()
        .find(|seat| seat.seat_authority == *authority)
        .ok_or(GovernanceError::UnknownSeatAuthority)?;
    if !council.active_at(slot) || !seat.term_covers(slot) {
        return Err(GovernanceError::InactiveCouncilSeat);
    }
    Ok(())
}

pub(super) fn validate_mask_members_active(
    council: &GovernanceCouncilSetV1,
    bitset: u8,
    count: u8,
    slot: u64,
) -> GovernanceResult<()> {
    if bitset & !VALID_APPROVAL_MASK != 0 {
        return Err(GovernanceError::InvalidApprovalBitset);
    }
    if bitset.count_ones() as u8 != count {
        return Err(GovernanceError::ApprovalCountMismatch);
    }
    if !council.active_at(slot) {
        return Err(GovernanceError::InactiveCouncilSeat);
    }
    for (index, seat) in council.seats.iter().enumerate() {
        if bitset & (1 << index) != 0 && !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat);
        }
    }
    Ok(())
}

pub(super) fn validate_candidate_seats_at_slot(
    candidate: &GovernanceCouncilSetV1,
    slot: u64,
) -> GovernanceResult<()> {
    if !candidate.active_at(slot) {
        return Err(GovernanceError::InvalidCouncilActivation);
    }
    for seat in &candidate.seats {
        if !seat.term_covers(slot) {
            return Err(GovernanceError::InactiveCouncilSeat);
        }
    }
    Ok(())
}

fn create_council_pda_account<'a>(
    program_id: &Pubkey,
    payer: &AccountInfo<'a>,
    council_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    council: &GovernanceCouncilSetV1,
) -> ProgramResult {
    let version = council.version.to_le_bytes();
    let bump = [council.bump];
    let seeds: &[&[u8]] = &[
        UPGRADE_SEED_DOMAIN_V1,
        COUNCIL_SEED,
        council.target_program.as_ref(),
        &version,
        &bump,
    ];
    let encoded = encode_fixed_account(council, GovernanceCouncilSetV1::LEN)?;
    create_fixed_pda_account(
        program_id,
        payer,
        council_info,
        system_program_info,
        &Rent::get()?,
        GovernanceCouncilSetV1::LEN,
        seeds,
    )?;
    let mut data = council_info.try_borrow_mut_data()?;
    data.copy_from_slice(&encoded);
    Ok(())
}

pub fn process_create_candidate_council_set_v1(
    program_id: &Pubkey,
    accounts: &[AccountInfo<'_>],
    instruction: CreateCandidateCouncilSetV1,
) -> ProgramResult {
    exact_account_count(accounts, 13)?;
    let payer = &accounts[0];
    let creator = &accounts[1];
    let config_info = &accounts[2];
    let policy_info = &accounts[3];
    let current_council_info = &accounts[4];
    let gate_info = &accounts[5];
    let candidate_info = &accounts[6];
    let candidate_authorities = &accounts[7..12];
    let system_program_info = &accounts[12];

    validate_candidate_creation_authority_contract(
        payer,
        creator,
        config_info,
        policy_info,
        current_council_info,
        gate_info,
        candidate_info,
        candidate_authorities,
        system_program_info,
    )?;

    validate_signer_writable(payer)?;
    validate_signer_readonly(creator)?;
    for readonly in [config_info, policy_info, current_council_info, gate_info] {
        validate_readonly(readonly)?;
    }
    validate_writable(candidate_info)?;
    for authority in candidate_authorities {
        if *authority.key == Pubkey::default() {
            return Err(GovernanceError::DefaultPubkey.into());
        }
    }
    validate_system_program(system_program_info)?;
    require_absent_system_account(candidate_info)?;

    let slot = current_slot()?;
    let config = load_config(program_id, config_info)?;
    let policy = load_policy(program_id, policy_info, config_info.key, &config, slot)?;
    let current_council = load_current_council(
        program_id,
        current_council_info,
        config_info.key,
        &config,
        &policy,
        slot,
    )?;
    let gate = load_gate(program_id, gate_info, config_info.key, &config)?;
    require_active_seat_authority(&current_council, creator.key, slot)?;
    if instruction.expected_current_council_version != current_council.version
        || instruction.expected_current_council_hash != current_council.set_hash
        || instruction.expected_target_nonce != config.target_nonce
        || instruction.expected_gate_status != gate.status
        || instruction.expected_gate_epoch != gate.epoch
        || instruction.candidate_council_version <= current_council.version
        || instruction.candidate_council_version == u64::MAX
    {
        return Err(GovernanceError::StaleCouncilVersion.into());
    }
    let minimum_activation = slot
        .checked_add(config.major_delay_slots)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    if instruction.activation_slot < minimum_activation {
        return Err(GovernanceError::InvalidProposalTiming.into());
    }
    let (candidate_pda, candidate_bump) = derive_council_pda(
        program_id,
        &config.target_program,
        instruction.candidate_council_version,
    );
    if *candidate_info.key != candidate_pda {
        return Err(GovernanceError::InvalidPda.into());
    }
    let mut seats = [CouncilSeatV1::default(); 5];
    for (index, (authority, term)) in candidate_authorities
        .iter()
        .zip(instruction.seat_terms.iter())
        .enumerate()
    {
        if term.term_start_slot >= term.term_end_slot
            || term.term_start_slot > instruction.activation_slot
            || instruction.activation_slot >= term.term_end_slot
        {
            return Err(GovernanceError::InactiveCouncilSeat.into());
        }
        seats[index] = CouncilSeatV1 {
            seat_authority: *authority.key,
            term_start_slot: term.term_start_slot,
            term_end_slot: term.term_end_slot,
            active: true,
            reserved: [0; COUNCIL_SEAT_RESERVED_LEN],
        };
    }
    let mut candidate = GovernanceCouncilSetV1 {
        discriminator: GOVERNANCE_COUNCIL_DISCRIMINATOR,
        account_version: ACCOUNT_VERSION_V1,
        bump: candidate_bump,
        initialized: true,
        controller_config: *config_info.key,
        version: instruction.candidate_council_version,
        target_program: config.target_program,
        activation_slot: instruction.activation_slot,
        deactivation_slot: 0,
        seats,
        routine_threshold: policy.routine_threshold,
        terminal_threshold: policy.terminal_threshold,
        policy_flags: 0,
        set_hash: [0; 32],
        reserved: [0; GOVERNANCE_COUNCIL_RESERVED_LEN],
    };
    candidate.set_hash = compute_council_set_hash(&candidate);
    if candidate.set_hash != instruction.expected_candidate_council_hash {
        return Err(GovernanceError::CouncilHashMismatch.into());
    }
    validate_council_set(&candidate, &policy)?;
    validate_council_guardian_separation(&candidate, &config.guardian)?;
    create_council_pda_account(
        program_id,
        payer,
        candidate_info,
        system_program_info,
        &candidate,
    )
}
