//! Fixed bridge ABI for the controller-owned protocol gate.
//!
//! Production and governed localnet admission use the shared controller wire verifier and
//! profile identity. The local adapter preserves oracle error codes and the synthetic test
//! verifier; only the explicit legacy localnet authority profile disables gate admission.
//!
//! The routing capability is intentionally outside the public crate API:
//! ```compile_fail
//! use ameba_oracle::governance_gate::GateValidated;
//! ```

#[cfg(feature = "synthetic-governance-controller")]
use crate::compact_error::CompactAccountInfo;
use crate::ProgramError;
#[cfg(not(feature = "localnet-dev-authority"))]
use solana_program::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;
use solana_sdk_ids::bpf_loader_upgradeable;

use crate::error::OracleError;

pub const GOVERNANCE_TAIL_MAGIC: [u8; 4] = *b"AGV1";
pub const GOVERNANCE_TAIL_VERSION_V1: u8 = 1;
pub const GOVERNANCE_TAIL_LEN: usize = 16;
pub const PROTOCOL_GATE_DISCRIMINATOR: [u8; 8] = *b"AGVGAT01";
pub const PROTOCOL_GATE_VERSION_V1: u8 = 1;
pub const PROTOCOL_GATE_LEN: usize = 192;

/// Seed domain of the V3 governance controller's PDAs: `[domain, "council"]` for its Config and
/// `[domain, "gate", target_program]` for each target's protocol gate.
const GOVERNANCE_SEED_DOMAIN_V3: &[u8] = b"ameba-governance-v3";
const COUNCIL_SEED: &[u8] = b"council";
const GATE_SEED: &[u8] = b"gate";

#[cfg(feature = "synthetic-governance-controller")]
pub const PINNED_CONTROLLER_PROGRAM_ID: Pubkey =
    solana_program::pubkey!("4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi");

#[cfg(not(feature = "synthetic-governance-controller"))]
pub const PINNED_CONTROLLER_PROGRAM_ID: Pubkey =
    ameba_governance_interface::PINNED_CONTROLLER_PROGRAM_ID;

/// Deliberately reachable in every synthetic bridge admission so release tooling can prove that
/// an artifact was not built with the test-only controller hidden through Cargo rustflags or
/// configuration. The warning is intentionally absent from every production-feature build.
#[cfg(feature = "synthetic-governance-controller")]
#[inline(never)]
fn emit_selected_controller_release_marker() {
    solana_program::msg!("AMEBA_ORACLE_SYNTHETIC_CONTROLLER_DO_NOT_RELEASE");
}

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
            return Err(OracleError::InvalidGovernanceTail.into());
        }
        let tail = Self {
            magic: bytes[0..4]
                .try_into()
                .map_err(|_| OracleError::InvalidGovernanceTail)?,
            version: bytes[4],
            reserved: bytes[5..8]
                .try_into()
                .map_err(|_| OracleError::InvalidGovernanceTail)?,
            expected_epoch: u64::from_le_bytes(
                bytes[8..16]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceTail)?,
            ),
        };
        if tail.magic != GOVERNANCE_TAIL_MAGIC
            || tail.version != GOVERNANCE_TAIL_VERSION_V1
            || tail.reserved != [0; 3]
        {
            return Err(OracleError::InvalidGovernanceTail.into());
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
            _ => Err(OracleError::InvalidGovernanceGateData.into()),
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
            return Err(OracleError::InvalidGovernanceGateData.into());
        }
        let status = GateStatusV1::decode(bytes[11])?;
        let gate = Self {
            bump: bytes[9],
            status,
            controller_config: Pubkey::new_from_array(
                bytes[12..44]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            target_program: Pubkey::new_from_array(
                bytes[44..76]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            target_programdata: Pubkey::new_from_array(
                bytes[76..108]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            epoch: u64::from_le_bytes(
                bytes[108..116]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            active_proposal: Pubkey::new_from_array(
                bytes[116..148]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            freeze_slot: u64::from_le_bytes(
                bytes[148..156]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            freeze_reason_code: u16::from_le_bytes(
                bytes[156..158]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
            last_completed_proposal: Pubkey::new_from_array(
                bytes[158..190]
                    .try_into()
                    .map_err(|_| OracleError::InvalidGovernanceGateData)?,
            ),
        };
        gate.validate_canonical_status()?;
        Ok(gate)
    }

    fn validate_canonical_status(&self) -> Result<(), ProgramError> {
        let proposal_is_default = crate::pubkey_is_default(&self.active_proposal);
        let freeze_is_clear = self.freeze_slot == 0 && self.freeze_reason_code == 0;
        let freeze_is_set = self.freeze_slot != 0 && self.freeze_reason_code != 0;
        let canonical = match self.status {
            GateStatusV1::Active => proposal_is_default && freeze_is_clear,
            GateStatusV1::FrozenForUpgrade => !proposal_is_default && freeze_is_set,
            GateStatusV1::EmergencyFrozen => proposal_is_default && freeze_is_set,
        };
        if !canonical {
            return Err(OracleError::InvalidGovernanceGateData.into());
        }
        Ok(())
    }
}

/// Authorization capability carried from the top-level envelope into every business router.
/// Its fields and constructors stay private to this module.
pub(crate) struct GateValidated {
    expected_epoch: u64,
    _private: (),
}

impl GateValidated {
    pub(crate) fn expected_epoch(&self) -> u64 {
        self.expected_epoch
    }
}

fn validated_capability(expected_epoch: u64) -> GateValidated {
    GateValidated {
        expected_epoch,
        _private: (),
    }
}

#[cfg(feature = "localnet-dev-authority")]
pub(crate) fn disabled_build_capability() -> GateValidated {
    validated_capability(0)
}

/// The V3 controller has one Config for every target it governs.
pub fn derive_controller_config_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    let _ = target_program;
    Pubkey::find_program_address(
        &[GOVERNANCE_SEED_DOMAIN_V3, COUNCIL_SEED],
        controller_program,
    )
}

pub fn derive_protocol_gate_pda(
    controller_program: &Pubkey,
    target_program: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            GOVERNANCE_SEED_DOMAIN_V3,
            GATE_SEED,
            target_program.as_ref(),
        ],
        controller_program,
    )
}

pub fn derive_target_programdata_pda(target_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[target_program.as_ref()], &bpf_loader_upgradeable::id()).0
}

#[cfg(feature = "synthetic-governance-controller")]
pub(crate) fn validate_top_level_envelope<'accounts, 'info, 'data>(
    program_id: &Pubkey,
    accounts: &'accounts [AccountInfo<'info>],
    instruction_data: &'data [u8],
) -> Result<(&'accounts [AccountInfo<'info>], &'data [u8], GateValidated), ProgramError> {
    emit_selected_controller_release_marker();
    if instruction_data.len() <= GOVERNANCE_TAIL_LEN {
        return Err(OracleError::MissingGovernanceTail.into());
    }
    let legacy_data_len = instruction_data.len() - GOVERNANCE_TAIL_LEN;
    let tail = GovernanceInstructionTailV1::decode_exact(&instruction_data[legacy_data_len..])?;

    let gate_info = accounts.last().ok_or(OracleError::MissingGovernanceGate)?;
    if program_id != &crate::id() {
        return Err(OracleError::InvalidGovernanceGateData.into());
    }
    let (expected_gate, expected_bump) =
        derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, program_id);
    if gate_info.key != &expected_gate {
        return Err(OracleError::InvalidGovernanceGatePda.into());
    }
    if accounts[..accounts.len() - 1]
        .iter()
        .any(|account| account.key == &expected_gate)
    {
        return Err(OracleError::DuplicateGovernanceGate.into());
    }
    if gate_info.is_signer || gate_info.is_writable || gate_info.executable {
        return Err(OracleError::InvalidGovernanceGatePrivileges.into());
    }
    if gate_info.owner != &PINNED_CONTROLLER_PROGRAM_ID {
        return Err(OracleError::InvalidGovernanceGateOwner.into());
    }

    let gate = {
        let data = gate_info
            .try_data()
            .map_err(|_| OracleError::InvalidGovernanceGateData)?;
        ProtocolGateV1::decode_exact(&data)?
    };
    let expected_controller_config =
        derive_controller_config_pda(&PINNED_CONTROLLER_PROGRAM_ID, program_id).0;
    if gate.bump != expected_bump {
        return Err(OracleError::InvalidGovernanceGatePda.into());
    }
    if gate.controller_config != expected_controller_config
        || gate.target_program != *program_id
        || gate.target_programdata != derive_target_programdata_pda(program_id)
    {
        return Err(OracleError::InvalidGovernanceGateData.into());
    }
    if gate.status != GateStatusV1::Active {
        return Err(OracleError::GovernanceGateFrozen.into());
    }
    if tail.expected_epoch != gate.epoch {
        return Err(OracleError::GovernanceGateEpochMismatch.into());
    }

    Ok((
        &accounts[..accounts.len() - 1],
        &instruction_data[..legacy_data_len],
        validated_capability(tail.expected_epoch),
    ))
}

#[cfg(all(
    not(feature = "localnet-dev-authority"),
    not(feature = "synthetic-governance-controller")
))]
pub(crate) fn validate_top_level_envelope<'accounts, 'info, 'data>(
    program_id: &Pubkey,
    accounts: &'accounts [AccountInfo<'info>],
    instruction_data: &'data [u8],
) -> Result<(&'accounts [AccountInfo<'info>], &'data [u8], GateValidated), ProgramError> {
    if program_id != &crate::id() {
        return Err(OracleError::InvalidGovernanceGateData.into());
    }
    let (accounts, data, gate) = ameba_governance_interface::validate_top_level_envelope(
        program_id,
        accounts,
        instruction_data,
    )
    .map_err(|error| {
        use ameba_governance_interface::GateError as G;
        let code = match error {
            solana_program::program_error::ProgramError::Custom(code) => code,
            _ => return ProgramError::from(OracleError::InvalidGovernanceGateData),
        };
        let mapped = match code {
            x if x == G::InvalidGovernanceTail as u32 => OracleError::InvalidGovernanceTail,
            x if x == G::MissingGovernanceTail as u32 => OracleError::MissingGovernanceTail,
            x if x == G::MissingGovernanceGate as u32 => OracleError::MissingGovernanceGate,
            x if x == G::InvalidGovernanceGatePda as u32 => OracleError::InvalidGovernanceGatePda,
            x if x == G::DuplicateGovernanceGate as u32 => OracleError::DuplicateGovernanceGate,
            x if x == G::InvalidGovernanceGatePrivileges as u32 => {
                OracleError::InvalidGovernanceGatePrivileges
            }
            x if x == G::InvalidGovernanceGateOwner as u32 => {
                OracleError::InvalidGovernanceGateOwner
            }
            x if x == G::GovernanceGateFrozen as u32 => OracleError::GovernanceGateFrozen,
            x if x == G::GovernanceGateEpochMismatch as u32 => {
                OracleError::GovernanceGateEpochMismatch
            }
            _ => OracleError::InvalidGovernanceGateData,
        };
        ProgramError::from(mapped)
    })?;
    Ok((accounts, data, validated_capability(gate.expected_epoch())))
}

/// A nested instruction must never carry its own governance tail. The first nested-instruction
/// transport uses this check.
#[allow(dead_code)]
pub(crate) fn ends_with_valid_governance_tail(instruction_data: &[u8]) -> bool {
    instruction_data.len() > GOVERNANCE_TAIL_LEN
        && GovernanceInstructionTailV1::decode_exact(
            &instruction_data[instruction_data.len() - GOVERNANCE_TAIL_LEN..],
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_test_controller() -> Pubkey {
        #[cfg(not(feature = "localnet-dev-authority"))]
        {
            PINNED_CONTROLLER_PROGRAM_ID
        }
        #[cfg(feature = "localnet-dev-authority")]
        {
            solana_program::pubkey!("4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi")
        }
    }

    fn gate_test_target() -> Pubkey {
        crate::id()
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

    #[cfg(not(feature = "localnet-dev-authority"))]
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

    #[cfg(not(feature = "localnet-dev-authority"))]
    impl AdmissionFixture {
        fn active(epoch: u64) -> Self {
            let target = gate_test_target();
            let key = derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &target).0;
            let mut data = gate_bytes(GateStatusV1::Active);
            data[108..116].copy_from_slice(&epoch.to_le_bytes());
            let mut instruction_data =
                vec![crate::instruction::OracleInstructionTag::UpdateConfig as u8];
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
    #[cfg(not(feature = "localnet-dev-authority"))]
    fn admission_strips_only_the_absolute_tail_and_gate() {
        let mut fixture = AdmissionFixture::active(41);
        assert_eq!(fixture.validate().unwrap(), (vec![2, 0xaa, 0xbb], 41));
    }

    #[test]
    #[cfg(not(feature = "localnet-dev-authority"))]
    fn admission_rejects_missing_or_malformed_tail_before_gate_access() {
        let mut fixture = AdmissionFixture::active(41);
        fixture.instruction_data.truncate(3);
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                OracleError::MissingGovernanceTail as u32
            ))
        );

        let mut fixture = AdmissionFixture::active(41);
        let tail_start = fixture.instruction_data.len() - GOVERNANCE_TAIL_LEN;
        fixture.instruction_data[tail_start] ^= 1;
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                OracleError::InvalidGovernanceTail as u32
            ))
        );
    }

    #[test]
    #[cfg(not(feature = "localnet-dev-authority"))]
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
                    OracleError::InvalidGovernanceGatePrivileges as u32
                ))
            );
        }

        let mut fixture = AdmissionFixture::active(41);
        fixture.owner = Pubkey::new_unique();
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                OracleError::InvalidGovernanceGateOwner as u32
            ))
        );

        let mut fixture = AdmissionFixture::active(41);
        fixture.key = Pubkey::new_unique();
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                OracleError::InvalidGovernanceGatePda as u32
            ))
        );

        for index in [0usize, 8, 10, 12, 44, 76, 190] {
            let mut fixture = AdmissionFixture::active(41);
            fixture.data[index] ^= 1;
            assert_eq!(
                fixture.validate(),
                Err(ProgramError::Custom(
                    OracleError::InvalidGovernanceGateData as u32
                )),
                "gate byte {index}"
            );
        }

        let mut fixture = AdmissionFixture::active(41);
        fixture.data[9] ^= 1;
        assert_eq!(
            fixture.validate(),
            Err(ProgramError::Custom(
                OracleError::InvalidGovernanceGatePda as u32
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
                Err(ProgramError::Custom(
                    OracleError::GovernanceGateFrozen as u32
                ))
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
                    OracleError::GovernanceGateEpochMismatch as u32
                ))
            );
        }
    }

    #[test]
    #[cfg(not(feature = "localnet-dev-authority"))]
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
    #[cfg(not(feature = "localnet-dev-authority"))]
    fn admission_requires_one_final_gate_and_rejects_duplicates() {
        let mut fixture = AdmissionFixture::active(41);
        let instruction_data = fixture.instruction_data.clone();
        assert_eq!(
            validate_top_level_envelope(&crate::id(), &[], &instruction_data).map(|_| ()),
            Err(ProgramError::Custom(
                OracleError::MissingGovernanceGate as u32
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
            validate_top_level_envelope(&crate::id(), &accounts, &instruction_data).map(|_| ()),
            Err(ProgramError::Custom(
                OracleError::DuplicateGovernanceGate as u32
            ))
        );
    }

    #[test]
    #[cfg(not(feature = "localnet-dev-authority"))]
    fn envelope_strip_preserves_representative_handler_inputs() {
        let representative_tags = [
            crate::instruction::OracleInstructionTag::UpdateConfig as u8,
            crate::instruction::OracleInstructionTag::CreatePeriod as u8,
            crate::instruction::OracleInstructionTag::InitializeOracleMonthV5 as u8,
            crate::instruction::OracleInstructionTag::ConfigureOracleProductSkuManifest as u8,
        ];
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
            let (stripped_accounts, stripped_data, capability) =
                validate_top_level_envelope(&crate::id(), &accounts, &fixture.instruction_data)
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
