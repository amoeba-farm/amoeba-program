//! Shared exact Spread V3 gate ABI and signed instruction envelope.
use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};
use solana_sdk_ids::bpf_loader_upgradeable;
pub const PRODUCTION_CONTROLLER_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx");
pub const TEST_CONTROLLER_PROGRAM_ID: Pubkey = Pubkey::new_from_array([71; 32]);
include!(concat!(env!("OUT_DIR"), "/localnet_identity.rs"));
#[cfg(feature = "localnet-governance")]
pub const PINNED_CONTROLLER_PROGRAM_ID: Pubkey = LOCALNET_CONTROLLER_PROGRAM_ID;
#[cfg(not(feature = "localnet-governance"))]
pub const PINNED_CONTROLLER_PROGRAM_ID: Pubkey = PRODUCTION_CONTROLLER_PROGRAM_ID;
pub const GOVERNANCE_TAIL_MAGIC: [u8; 4] = *b"AGV1";
pub const GOVERNANCE_TAIL_VERSION_V1: u8 = 1;
pub const GOVERNANCE_TAIL_LEN: usize = 16;
pub const PROTOCOL_GATE_DISCRIMINATOR: [u8; 8] = *b"AGVGAT01";
pub const PROTOCOL_GATE_VERSION_V1: u8 = 1;
pub const PROTOCOL_GATE_LEN: usize = 192;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GateError {
    InvalidGovernanceTail = 3100,
    InvalidGovernanceGateData,
    MissingGovernanceTail,
    MissingGovernanceGate,
    InvalidGovernanceGatePda,
    DuplicateGovernanceGate,
    InvalidGovernanceGatePrivileges,
    InvalidGovernanceGateOwner,
    GovernanceGateFrozen,
    GovernanceGateEpochMismatch,
}
impl From<GateError> for ProgramError {
    fn from(e: GateError) -> Self {
        Self::Custom(e as u32)
    }
}
pub const LOCALNET_RELEASE_MARKER: &str = "AMEBA_LOCALNET_GOVERNANCE_DO_NOT_RELEASE";
#[inline(never)]
pub fn emit_profile_marker() {
    #[cfg(feature = "localnet-governance")]
    solana_program::log::sol_log(LOCALNET_RELEASE_MARKER);
}
pub const LOCALNET_GENESIS: &str = "A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX";
#[cfg(feature = "localnet-governance")]
pub const MIN_REVIEW_SLOTS: u64 = 750;
#[cfg(not(feature = "localnet-governance"))]
pub const MIN_REVIEW_SLOTS: u64 = 1_512_000;
pub const MIN_DELAY_SLOTS: u64 = 150;
#[cfg(feature = "localnet-governance")]
pub const EXECUTION_MARGIN_SLOTS: u64 = 150;
#[cfg(not(feature = "localnet-governance"))]
pub const EXECUTION_MARGIN_SLOTS: u64 = 9_000;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GovernanceInstructionTailV1 {
    pub magic: [u8; 4],
    pub version: u8,
    pub reserved: [u8; 3],
    pub expected_epoch: u64,
}

impl GovernanceInstructionTailV1 {
    pub fn decode_exact(bytes: &[u8]) -> Result<Self, ProgramError> {
        if bytes.len() != GOVERNANCE_TAIL_LEN {
            return Err(GateError::InvalidGovernanceTail.into());
        }
        let tail = Self {
            magic: bytes[0..4]
                .try_into()
                .map_err(|_| GateError::InvalidGovernanceTail)?,
            version: bytes[4],
            reserved: bytes[5..8]
                .try_into()
                .map_err(|_| GateError::InvalidGovernanceTail)?,
            expected_epoch: u64::from_le_bytes(
                bytes[8..16]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceTail)?,
            ),
        };
        if tail.magic != GOVERNANCE_TAIL_MAGIC
            || tail.version != GOVERNANCE_TAIL_VERSION_V1
            || tail.reserved != [0; 3]
        {
            return Err(GateError::InvalidGovernanceTail.into());
        }
        Ok(tail)
    }

    pub fn encode(self) -> [u8; GOVERNANCE_TAIL_LEN] {
        let mut bytes = [0u8; GOVERNANCE_TAIL_LEN];
        bytes[0..4].copy_from_slice(&self.magic);
        bytes[4] = self.version;
        bytes[5..8].copy_from_slice(&self.reserved);
        bytes[8..16].copy_from_slice(&self.expected_epoch.to_le_bytes());
        bytes
    }

    pub fn for_epoch(expected_epoch: u64) -> Self {
        Self {
            magic: GOVERNANCE_TAIL_MAGIC,
            version: GOVERNANCE_TAIL_VERSION_V1,
            reserved: [0; 3],
            expected_epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GateStatusV1 {
    Active = 0,
    FrozenForUpgrade = 1,
    EmergencyFrozen = 2,
}

impl GateStatusV1 {
    fn decode(value: u8) -> Result<Self, ProgramError> {
        match value {
            0 => Ok(Self::Active),
            1 => Ok(Self::FrozenForUpgrade),
            2 => Ok(Self::EmergencyFrozen),
            _ => Err(GateError::InvalidGovernanceGateData.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolGateV1 {
    pub bump: u8,
    pub status: GateStatusV1,
    pub controller_config: Pubkey,
    pub target_program: Pubkey,
    pub target_programdata: Pubkey,
    pub epoch: u64,
    pub active_proposal: Pubkey,
    pub freeze_slot: u64,
    pub freeze_reason_code: u16,
    pub last_completed_proposal: Pubkey,
}

impl ProtocolGateV1 {
    pub fn decode_exact(bytes: &[u8]) -> Result<Self, ProgramError> {
        if bytes.len() != PROTOCOL_GATE_LEN
            || bytes[0..8] != PROTOCOL_GATE_DISCRIMINATOR
            || bytes[8] != PROTOCOL_GATE_VERSION_V1
            || bytes[10] != 1
            || bytes[190..192] != [0, 0]
        {
            return Err(GateError::InvalidGovernanceGateData.into());
        }
        let status = GateStatusV1::decode(bytes[11])?;
        let gate = Self {
            bump: bytes[9],
            status,
            controller_config: Pubkey::new_from_array(
                bytes[12..44]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            target_program: Pubkey::new_from_array(
                bytes[44..76]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            target_programdata: Pubkey::new_from_array(
                bytes[76..108]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            epoch: u64::from_le_bytes(
                bytes[108..116]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            active_proposal: Pubkey::new_from_array(
                bytes[116..148]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            freeze_slot: u64::from_le_bytes(
                bytes[148..156]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            freeze_reason_code: u16::from_le_bytes(
                bytes[156..158]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
            last_completed_proposal: Pubkey::new_from_array(
                bytes[158..190]
                    .try_into()
                    .map_err(|_| GateError::InvalidGovernanceGateData)?,
            ),
        };
        if gate.epoch == 0
            || gate.controller_config == Pubkey::default()
            || gate.target_program == Pubkey::default()
            || gate.target_programdata == Pubkey::default()
        {
            return Err(GateError::InvalidGovernanceGateData.into());
        }
        gate.validate_canonical_status()?;
        Ok(gate)
    }

    fn validate_canonical_status(&self) -> Result<(), ProgramError> {
        let proposal_is_default = self.active_proposal == Pubkey::default();
        let freeze_is_clear = self.freeze_slot == 0 && self.freeze_reason_code == 0;
        let freeze_is_set = self.freeze_slot != 0 && self.freeze_reason_code != 0;
        let canonical = match self.status {
            GateStatusV1::Active => proposal_is_default && freeze_is_clear,
            GateStatusV1::FrozenForUpgrade => !proposal_is_default && freeze_is_set,
            GateStatusV1::EmergencyFrozen => proposal_is_default && freeze_is_set,
        };
        if !canonical {
            return Err(GateError::InvalidGovernanceGateData.into());
        }
        Ok(())
    }
}

pub struct GateValidated {
    expected_epoch: u64,
}
impl GateValidated {
    pub fn expected_epoch(&self) -> u64 {
        self.expected_epoch
    }
}
pub fn derive_controller_config_pda(
    controller_program: &Pubkey,
    _target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], controller_program)
}
pub fn derive_protocol_gate_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"ameba-governance-v3", b"gate", target_program.as_ref()],
        controller_program,
    )
}
pub fn derive_target_programdata_pda(target_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[target_program.as_ref()], &bpf_loader_upgradeable::id()).0
}
pub fn validate_top_level_envelope<'accounts, 'info, 'data>(
    program_id: &Pubkey,
    accounts: &'accounts [AccountInfo<'info>],
    instruction_data: &'data [u8],
) -> Result<(&'accounts [AccountInfo<'info>], &'data [u8], GateValidated), ProgramError> {
    emit_profile_marker();
    if instruction_data.len() <= GOVERNANCE_TAIL_LEN {
        return Err(GateError::MissingGovernanceTail.into());
    }
    let legacy_data_len = instruction_data.len() - GOVERNANCE_TAIL_LEN;
    let tail = GovernanceInstructionTailV1::decode_exact(&instruction_data[legacy_data_len..])?;

    let gate_info = accounts.last().ok_or(GateError::MissingGovernanceGate)?;

    let (expected_gate, _expected_bump) =
        derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, program_id);

    if gate_info.key != &expected_gate {
        return Err(GateError::InvalidGovernanceGatePda.into());
    }
    if accounts[..accounts.len() - 1]
        .iter()
        .any(|account| account.key == &expected_gate)
    {
        return Err(GateError::DuplicateGovernanceGate.into());
    }
    if gate_info.is_signer || gate_info.is_writable || gate_info.executable {
        return Err(GateError::InvalidGovernanceGatePrivileges.into());
    }
    if gate_info.owner != &PINNED_CONTROLLER_PROGRAM_ID {
        return Err(GateError::InvalidGovernanceGateOwner.into());
    }

    validate_gate(program_id, gate_info, tail.expected_epoch)?;

    Ok((
        &accounts[..accounts.len() - 1],
        &instruction_data[..legacy_data_len],
        GateValidated {
            expected_epoch: tail.expected_epoch,
        },
    ))
}

/// Validate a separate CPI target gate against an epoch signed in the outer instruction.
pub fn validate_gate(
    program_id: &Pubkey,
    gate_info: &AccountInfo<'_>,
    expected_epoch: u64,
) -> Result<ProtocolGateV1, ProgramError> {
    if gate_info.is_signer || gate_info.is_writable || gate_info.executable {
        return Err(GateError::InvalidGovernanceGatePrivileges.into());
    }
    let data = gate_info.try_borrow_data()?;
    let gate = read_gate(
        &PINNED_CONTROLLER_PROGRAM_ID,
        program_id,
        gate_info.key,
        gate_info.owner,
        &data,
    )?;
    if gate.status != GateStatusV1::Active {
        return Err(GateError::GovernanceGateFrozen.into());
    }
    if expected_epoch != gate.epoch {
        return Err(GateError::GovernanceGateEpochMismatch.into());
    }

    Ok(gate)
}

/// Decode chain data using an explicitly selected deployment identity; frozen states remain visible.
pub fn read_gate(
    controller: &Pubkey,
    target: &Pubkey,
    key: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<ProtocolGateV1, ProgramError> {
    let (expected, bump) = derive_protocol_gate_pda(controller, target);
    if *key != expected {
        return Err(GateError::InvalidGovernanceGatePda.into());
    }
    if owner != controller {
        return Err(GateError::InvalidGovernanceGateOwner.into());
    }
    let gate = ProtocolGateV1::decode_exact(data)?;
    if gate.bump != bump {
        return Err(GateError::InvalidGovernanceGatePda.into());
    }
    if gate.controller_config != derive_controller_config_pda(controller, target).0
        || gate.target_program != *target
        || gate.target_programdata != derive_target_programdata_pda(target)
    {
        return Err(GateError::InvalidGovernanceGateData.into());
    }
    Ok(gate)
}

/// Canonical Spread V3 council consent for a target's first initialization.
pub struct CouncilAuthorization {
    pub seats: [Pubkey; 5],
    pub council_epoch: u64,
}
pub fn validate_council_authorization(
    config: &AccountInfo<'_>,
    approvals: &[AccountInfo<'_>],
) -> Result<CouncilAuthorization, ProgramError> {
    let controller = PINNED_CONTROLLER_PROGRAM_ID;
    let (config_key, bump) = derive_controller_config_pda(&controller, &Pubkey::default());
    if config.key != &config_key
        || config.owner != &controller
        || config.is_signer
        || config.is_writable
        || config.executable
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let data = config.try_borrow_data()?;
    if data.len() != 384
        || &data[..8] != b"AG3CFG01"
        || data[8] != 1
        || data[9] != bump
        || data[10] != 1
        || data[347..].iter().any(|b| *b != 0)
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let key_at =
        |offset: usize| Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap());
    let u64_at = |offset: usize| u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
    let pd = derive_target_programdata_pda(&controller);
    let authority =
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"authority"], &controller).0;
    let treasury = key_at(107);
    if key_at(11) != controller
        || key_at(43) != pd
        || key_at(75) != authority
        || treasury == Pubkey::default()
        || [controller, pd, config_key, authority].contains(&treasury)
        || u64_at(299) == 0
        || u64_at(307) == 0
        || u64_at(339) == 0
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let review = u64_at(315);
    let delay = u64_at(323);
    let expiry = u64_at(331);
    if review < MIN_REVIEW_SLOTS
        || delay < MIN_DELAY_SLOTS
        || review
            .max(delay)
            .checked_add(EXECUTION_MARGIN_SLOTS)
            .is_none_or(|minimum| expiry <= minimum)
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let seats = std::array::from_fn(|i| key_at(139 + i * 32));
    for (i, seat) in seats.iter().enumerate() {
        if *seat == Pubkey::default() || *seat == authority || seats[..i].contains(seat) {
            return Err(ProgramError::InvalidAccountData);
        }
    }
    if approvals.len() != 3 {
        return Err(ProgramError::MissingRequiredSignature);
    }
    for (i, approval) in approvals.iter().enumerate() {
        if !approval.is_signer
            || approval.executable
            || !seats.contains(approval.key)
            || approvals[..i].iter().any(|other| other.key == approval.key)
        {
            return Err(ProgramError::MissingRequiredSignature);
        }
    }
    Ok(CouncilAuthorization {
        seats,
        council_epoch: u64_at(299),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_test_controller() -> Pubkey {
        PINNED_CONTROLLER_PROGRAM_ID
    }
    fn gate_test_target() -> Pubkey {
        Pubkey::new_from_array([91; 32])
    }
    fn gate_bytes(status: GateStatusV1) -> [u8; PROTOCOL_GATE_LEN] {
        let controller = gate_test_controller();
        let target = gate_test_target();
        let (gate, bump) = derive_protocol_gate_pda(&controller, &target);
        let _ = gate;
        let config = derive_controller_config_pda(&controller, &target).0;
        let mut bytes = [0u8; PROTOCOL_GATE_LEN];
        bytes[0..8].copy_from_slice(&PROTOCOL_GATE_DISCRIMINATOR);
        bytes[8] = PROTOCOL_GATE_VERSION_V1;
        bytes[9] = bump;
        bytes[10] = 1;
        bytes[11] = status as u8;
        bytes[12..44].copy_from_slice(config.as_ref());
        bytes[44..76].copy_from_slice(target.as_ref());
        bytes[76..108].copy_from_slice(derive_target_programdata_pda(&target).as_ref());
        bytes[108..116].copy_from_slice(&41u64.to_le_bytes());
        match status {
            GateStatusV1::Active => {}
            GateStatusV1::FrozenForUpgrade => {
                bytes[116..148].copy_from_slice(Pubkey::new_unique().as_ref());
                bytes[148..156].copy_from_slice(&10u64.to_le_bytes());
                bytes[156..158].copy_from_slice(&7u16.to_le_bytes());
            }
            GateStatusV1::EmergencyFrozen => {
                bytes[148..156].copy_from_slice(&10u64.to_le_bytes());
                bytes[156..158].copy_from_slice(&7u16.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn epoch_41_tail_is_the_exact_cross_repository_vector() {
        let encoded = GovernanceInstructionTailV1::for_epoch(41).encode();
        assert_eq!(encoded, *b"AGV1\x01\0\0\0)\0\0\0\0\0\0\0");
        assert_eq!(
            GovernanceInstructionTailV1::decode_exact(&encoded).unwrap(),
            GovernanceInstructionTailV1::for_epoch(41)
        );
    }

    #[test]
    fn tail_codec_rejects_every_noncanonical_shape() {
        for len in 0..GOVERNANCE_TAIL_LEN {
            assert!(
                GovernanceInstructionTailV1::decode_exact(&[0u8; GOVERNANCE_TAIL_LEN][..len])
                    .is_err()
            );
        }
        assert!(
            GovernanceInstructionTailV1::decode_exact(&[0u8; GOVERNANCE_TAIL_LEN + 1]).is_err()
        );
        for index in 0..8 {
            let mut bytes = GovernanceInstructionTailV1::for_epoch(u64::MAX).encode();
            bytes[index] ^= 0xff;
            assert!(GovernanceInstructionTailV1::decode_exact(&bytes).is_err());
        }
        for epoch in [0, u64::MAX] {
            let bytes = GovernanceInstructionTailV1::for_epoch(epoch).encode();
            assert_eq!(
                GovernanceInstructionTailV1::decode_exact(&bytes)
                    .unwrap()
                    .expected_epoch,
                epoch
            );
        }
    }

    #[test]
    fn gate_decoder_accepts_all_canonical_statuses_and_rejects_bad_shapes() {
        for status in [
            GateStatusV1::Active,
            GateStatusV1::FrozenForUpgrade,
            GateStatusV1::EmergencyFrozen,
        ] {
            assert_eq!(
                ProtocolGateV1::decode_exact(&gate_bytes(status))
                    .unwrap()
                    .status,
                status
            );
        }
        let active = gate_bytes(GateStatusV1::Active);
        for range in [12..44, 44..76, 76..108, 108..116] {
            let mut bytes = active;
            bytes[range].fill(0);
            assert_eq!(
                ProtocolGateV1::decode_exact(&bytes),
                Err(GateError::InvalidGovernanceGateData.into())
            );
        }
        assert!(ProtocolGateV1::decode_exact(&active[..PROTOCOL_GATE_LEN - 1]).is_err());
        let mut longer = active.to_vec();
        longer.push(0);
        assert!(ProtocolGateV1::decode_exact(&longer).is_err());
        for index in [0usize, 8, 10, 11, 190, 191] {
            let mut bytes = active;
            bytes[index] ^= 0xff;
            assert!(
                ProtocolGateV1::decode_exact(&bytes).is_err(),
                "index {index}"
            );
        }
    }

    #[test]
    fn gate_decoder_rejects_noncanonical_status_fields() {
        let mut active = gate_bytes(GateStatusV1::Active);
        active[148..156].copy_from_slice(&1u64.to_le_bytes());
        assert!(ProtocolGateV1::decode_exact(&active).is_err());

        let mut upgrade = gate_bytes(GateStatusV1::FrozenForUpgrade);
        upgrade[116..148].fill(0);
        assert!(ProtocolGateV1::decode_exact(&upgrade).is_err());

        let mut emergency = gate_bytes(GateStatusV1::EmergencyFrozen);
        emergency[116..148].copy_from_slice(Pubkey::new_unique().as_ref());
        assert!(ProtocolGateV1::decode_exact(&emergency).is_err());
    }

    struct AdmissionFixture {
        key: Pubkey,
        owner: Pubkey,
        lamports: u64,
        data: [u8; PROTOCOL_GATE_LEN],
        is_signer: bool,
        is_writable: bool,
        executable: bool,
        instruction_data: Vec<u8>,
    }

    impl AdmissionFixture {
        fn active(epoch: u64) -> Self {
            let target = gate_test_target();
            let key = derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &target).0;
            let mut data = gate_bytes(GateStatusV1::Active);
            data[108..116].copy_from_slice(&epoch.to_le_bytes());
            let mut instruction_data = vec![2];
            instruction_data.extend_from_slice(&[0xaa, 0xbb]);
            instruction_data
                .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(epoch).encode());
            Self {
                key,
                owner: PINNED_CONTROLLER_PROGRAM_ID,
                lamports: 1,
                data,
                is_signer: false,
                is_writable: false,
                executable: false,
                instruction_data,
            }
        }

        fn validate(&mut self) -> Result<(Vec<u8>, u64), ProgramError> {
            self.validate_for_target(&gate_test_target())
        }

        fn validate_for_target(&mut self, target: &Pubkey) -> Result<(Vec<u8>, u64), ProgramError> {
            let account = AccountInfo::new(
                &self.key,
                self.is_signer,
                self.is_writable,
                &mut self.lamports,
                &mut self.data,
                &self.owner,
                self.executable,
                0,
            );
            let accounts = [account];
            let (legacy_accounts, legacy_data, capability) =
                validate_top_level_envelope(target, &accounts, &self.instruction_data)?;
            assert!(legacy_accounts.is_empty());
            Ok((legacy_data.to_vec(), capability.expected_epoch()))
        }
    }

    #[test]
    fn admission_strips_only_the_absolute_tail_and_gate() {
        let mut fixture = AdmissionFixture::active(41);
        assert_eq!(fixture.validate().unwrap(), (vec![2, 0xaa, 0xbb], 41));
    }

    #[test]
    fn admission_rejects_missing_or_malformed_tail_before_gate_access() {
        let mut fixture = AdmissionFixture::active(41);
        fixture.instruction_data.truncate(3);
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                GateError::MissingGovernanceTail as u32
            ))
        );

        let mut fixture = AdmissionFixture::active(41);
        let tail_start = fixture.instruction_data.len() - GOVERNANCE_TAIL_LEN;
        fixture.instruction_data[tail_start] ^= 1;
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                GateError::InvalidGovernanceTail as u32
            ))
        );
    }

    #[test]
    fn admission_rejects_privilege_owner_pda_data_status_and_epoch_failures() {
        for mutate in [
            |fixture: &mut AdmissionFixture| fixture.is_signer = true,
            |fixture: &mut AdmissionFixture| fixture.is_writable = true,
            |fixture: &mut AdmissionFixture| fixture.executable = true,
        ] {
            let mut fixture = AdmissionFixture::active(41);
            mutate(&mut fixture);
            assert_eq!(
                fixture.validate(),
                Err(ProgramError::Custom(
                    GateError::InvalidGovernanceGatePrivileges as u32
                ))
            );
        }

        let mut fixture = AdmissionFixture::active(41);
        fixture.owner = Pubkey::new_unique();
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                GateError::InvalidGovernanceGateOwner as u32
            ))
        );

        let mut fixture = AdmissionFixture::active(41);
        fixture.key = Pubkey::new_unique();
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                GateError::InvalidGovernanceGatePda as u32
            ))
        );

        for index in [0usize, 8, 10, 12, 44, 76, 190] {
            let mut fixture = AdmissionFixture::active(41);
            fixture.data[index] ^= 1;
            assert_eq!(
                fixture.validate(),
                Err(ProgramError::Custom(
                    GateError::InvalidGovernanceGateData as u32
                )),
                "gate byte {index}"
            );
        }

        let mut fixture = AdmissionFixture::active(41);
        fixture.data[9] ^= 1;
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                GateError::InvalidGovernanceGatePda as u32
            ))
        );

        for status in [
            GateStatusV1::FrozenForUpgrade,
            GateStatusV1::EmergencyFrozen,
        ] {
            let mut fixture = AdmissionFixture::active(41);
            fixture.data = gate_bytes(status);
            assert_eq!(
                fixture.validate(),
                Err(ProgramError::Custom(GateError::GovernanceGateFrozen as u32))
            );
        }

        for signed_epoch in [40u64, 42] {
            let mut fixture = AdmissionFixture::active(41);
            let tail_start = fixture.instruction_data.len() - GOVERNANCE_TAIL_LEN;
            fixture.instruction_data[tail_start..]
                .copy_from_slice(&GovernanceInstructionTailV1::for_epoch(signed_epoch).encode());
            assert_eq!(
                fixture.validate(),
                Err(ProgramError::Custom(
                    GateError::GovernanceGateEpochMismatch as u32
                ))
            );
        }
    }

    #[test]
    fn active_admission_ignores_nondefault_last_completed_proposal() {
        let mut fixture = AdmissionFixture::active(41);
        let last_completed_proposal = Pubkey::new_unique();
        fixture.data[158..190].copy_from_slice(last_completed_proposal.as_ref());

        let decoded = ProtocolGateV1::decode_exact(&fixture.data).unwrap();
        assert_eq!(decoded.status, GateStatusV1::Active);
        assert_eq!(decoded.last_completed_proposal, last_completed_proposal);
        assert_eq!(fixture.validate().unwrap(), (vec![2, 0xaa, 0xbb], 41));
    }

    #[test]
    fn admission_requires_one_final_gate_and_rejects_duplicates() {
        let mut fixture = AdmissionFixture::active(41);
        let instruction_data = fixture.instruction_data.clone();
        assert_eq!(
            validate_top_level_envelope(&gate_test_target(), &[], &instruction_data).map(|_| ()),
            Err(ProgramError::Custom(
                GateError::MissingGovernanceGate as u32
            ))
        );

        let mut duplicate_lamports = 1;
        let mut duplicate_data = fixture.data;
        let duplicate = AccountInfo::new(
            &fixture.key,
            false,
            false,
            &mut duplicate_lamports,
            &mut duplicate_data,
            &fixture.owner,
            false,
            0,
        );
        let final_gate = AccountInfo::new(
            &fixture.key,
            false,
            false,
            &mut fixture.lamports,
            &mut fixture.data,
            &fixture.owner,
            false,
            0,
        );
        let accounts = [duplicate, final_gate];
        assert_eq!(
            validate_top_level_envelope(&gate_test_target(), &accounts, &instruction_data)
                .map(|_| ()),
            Err(ProgramError::Custom(
                GateError::DuplicateGovernanceGate as u32
            ))
        );
    }

    #[test]
    fn envelope_strip_preserves_representative_handler_inputs() {
        let representative_tags = [2, 3, 97];
        for tag in representative_tags {
            let mut fixture = AdmissionFixture::active(41);
            let legacy_data = vec![tag, 0x10, 0x20, 0x30];
            fixture.instruction_data = legacy_data.clone();
            fixture
                .instruction_data
                .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(41).encode());

            let legacy_key = Pubkey::new_unique();
            let legacy_owner = Pubkey::new_unique();
            let mut legacy_lamports = 9;
            let mut legacy_account_data = [7u8; 3];
            let legacy_account = AccountInfo::new(
                &legacy_key,
                true,
                true,
                &mut legacy_lamports,
                &mut legacy_account_data,
                &legacy_owner,
                false,
                3,
            );
            let gate_account = AccountInfo::new(
                &fixture.key,
                false,
                false,
                &mut fixture.lamports,
                &mut fixture.data,
                &fixture.owner,
                false,
                0,
            );
            let accounts = [legacy_account, gate_account];
            let (stripped_accounts, stripped_data, capability) = validate_top_level_envelope(
                &gate_test_target(),
                &accounts,
                &fixture.instruction_data,
            )
            .expect("representative envelope validates");
            assert_eq!(stripped_data, legacy_data, "tag {tag}");
            assert_eq!(stripped_accounts.len(), 1, "tag {tag}");
            assert_eq!(stripped_accounts[0].key, &legacy_key, "tag {tag}");
            assert!(stripped_accounts[0].is_signer, "tag {tag}");
            assert!(stripped_accounts[0].is_writable, "tag {tag}");
            assert_eq!(stripped_accounts[0].rent_epoch, 3, "tag {tag}");
            assert_eq!(capability.expected_epoch(), 41, "tag {tag}");
        }
    }
}

#[cfg(test)]
mod profiles {
    use super::*;
    #[test]
    fn production_and_localnet_are_separate() {
        assert_ne!(
            PRODUCTION_CONTROLLER_PROGRAM_ID,
            LOCALNET_CONTROLLER_PROGRAM_ID
        );
        assert_ne!(
            derive_protocol_gate_pda(
                &PRODUCTION_CONTROLLER_PROGRAM_ID,
                &Pubkey::new_from_array([91; 32])
            )
            .0,
            derive_protocol_gate_pda(
                &LOCALNET_CONTROLLER_PROGRAM_ID,
                &Pubkey::new_from_array([91; 32])
            )
            .0
        );
        if cfg!(feature = "localnet-governance") {
            assert_eq!(
                (MIN_REVIEW_SLOTS, MIN_DELAY_SLOTS, EXECUTION_MARGIN_SLOTS),
                (750, 150, 150)
            );
        } else {
            assert_eq!(
                (MIN_REVIEW_SLOTS, MIN_DELAY_SLOTS, EXECUTION_MARGIN_SLOTS),
                (1_512_000, 150, 9_000)
            );
        }
    }
}

#[cfg(test)]
mod council_tests {
    use super::*;
    #[test]
    fn quorum_bootstrap_rejects_forged_configs_and_nonmembers() {
        let controller = PINNED_CONTROLLER_PROGRAM_ID;
        let (config_key, bump) = derive_controller_config_pda(&controller, &Pubkey::default());
        let authority =
            Pubkey::find_program_address(&[b"ameba-governance-v3", b"authority"], &controller).0;
        let seats: [Pubkey; 5] = std::array::from_fn(|_| Pubkey::new_unique());
        let mut canonical = [0u8; 384];
        canonical[..8].copy_from_slice(b"AG3CFG01");
        canonical[8] = 1;
        canonical[9] = bump;
        canonical[10] = 1;
        canonical[11..43].copy_from_slice(controller.as_ref());
        canonical[43..75].copy_from_slice(derive_target_programdata_pda(&controller).as_ref());
        canonical[75..107].copy_from_slice(authority.as_ref());
        canonical[107..139].copy_from_slice(Pubkey::new_unique().as_ref());
        for (i, seat) in seats.iter().enumerate() {
            canonical[139 + i * 32..171 + i * 32].copy_from_slice(seat.as_ref());
        }
        for (offset, value) in [
            (299, 1u64),
            (307, 1),
            (315, MIN_REVIEW_SLOTS),
            (323, MIN_DELAY_SLOTS),
            (
                331,
                MIN_REVIEW_SLOTS.max(MIN_DELAY_SLOTS) + EXECUTION_MARGIN_SLOTS + 1,
            ),
            (339, 1),
        ] {
            canonical[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        let try_case =
            |mut bytes: [u8; 384], approval_keys: [Pubkey; 3], signed: bool, writable: bool| {
                let mut config_lamports = 1;
                let mut approval_lamports = [1, 1, 1];
                let mut empty1 = [];
                let mut empty2 = [];
                let mut empty3 = [];
                let config = AccountInfo::new(
                    &config_key,
                    false,
                    false,
                    &mut config_lamports,
                    &mut bytes,
                    &controller,
                    false,
                    0,
                );
                let [l1, l2, l3] = &mut approval_lamports;
                let approvals = [
                    AccountInfo::new(
                        &approval_keys[0],
                        signed,
                        writable,
                        l1,
                        &mut empty1,
                        &controller,
                        false,
                        0,
                    ),
                    AccountInfo::new(
                        &approval_keys[1],
                        signed,
                        false,
                        l2,
                        &mut empty2,
                        &controller,
                        false,
                        0,
                    ),
                    AccountInfo::new(
                        &approval_keys[2],
                        signed,
                        false,
                        l3,
                        &mut empty3,
                        &controller,
                        false,
                        0,
                    ),
                ];
                validate_council_authorization(&config, &approvals).map(|c| c.council_epoch)
            };
        assert_eq!(
            try_case(canonical, [seats[0], seats[1], seats[2]], true, true).unwrap(),
            1
        );
        for index in [0usize, 8, 9, 10, 11, 43, 75, 347, 383] {
            let mut bytes = canonical;
            bytes[index] ^= 255;
            assert!(
                try_case(bytes, [seats[0], seats[1], seats[2]], true, false).is_err(),
                "config offset {index}"
            );
        }
        assert!(try_case(canonical, [seats[0], seats[0], seats[2]], true, false).is_err());
        assert!(try_case(
            canonical,
            [Pubkey::new_unique(), seats[1], seats[2]],
            true,
            false
        )
        .is_err());
        assert!(try_case(canonical, [seats[0], seats[1], seats[2]], false, false).is_err());
        let mut bytes = canonical;
        bytes[315..323].copy_from_slice(&(MIN_REVIEW_SLOTS - 1).to_le_bytes());
        assert!(try_case(bytes, [seats[0], seats[1], seats[2]], true, false).is_err());
    }
}
