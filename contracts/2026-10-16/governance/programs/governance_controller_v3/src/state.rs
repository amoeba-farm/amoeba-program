use crate::artifact::{BITMAP_BYTES, MAX_ARTIFACT_BYTES, MAX_CHUNKS};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hashv, program_error::ProgramError, pubkey::Pubkey};

pub const DOMAIN: &[u8] = b"ameba-governance-v3";
#[cfg(feature = "localnet-governance")]
pub const DEVNET_GENESIS: &str = ameba_governance_interface::LOCALNET_GENESIS;
#[cfg(not(feature = "localnet-governance"))]
pub const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
pub const WEEK_SLOTS: u64 = ameba_governance_interface::MIN_REVIEW_SLOTS;
pub const MIN_DELAY_SLOTS: u64 = ameba_governance_interface::MIN_DELAY_SLOTS;
pub const EXECUTION_MARGIN_SLOTS: u64 = ameba_governance_interface::EXECUTION_MARGIN_SLOTS;
pub const CONFIG_LEN: usize = 384;
pub const PROPOSAL_LEN: usize = 400;
pub const ACTION_LEN: usize = 192;
/// Bytes of `Proposal::verified_chunks` (chunks 0..96) kept at their original offset.
pub const LOW_BITMAP_BYTES: usize = 12;
pub const MAX_DATA: usize = 16_384;
pub const THRESHOLD: u32 = 3;
pub const PENDING: u8 = 0;
pub const EXECUTED: u8 = 1;
pub const CANCELLED: u8 = 2;
pub const EXPIRED: u8 = 3;
pub const UPGRADE_CONTROLLER: u8 = 0;
pub const SET_TIMING: u8 = 1;
pub const ROTATE_COUNCIL: u8 = 2;
pub const UPGRADE_TARGET: u8 = 3;
pub const SET_TARGET_GATE: u8 = 4;
pub const INITIALIZE_SPREAD_LIGHT_CONFIG: u8 = 5;
pub const PUBLISH_VERIFICATION: u8 = 6;
/// Release number of this controller binary. Every controller release must
/// change it: a self-upgrade proposal binds the release it was created under
/// and can only install while that release is running, so installing
/// any newer controller retires all older self-upgrade proposals. Binaries
/// before this field had release 0 (the eight action bytes were zero).
pub const CONTROLLER_RELEASE: u64 = 2;
pub const DEVNET_SPREAD: Pubkey =
    solana_program::pubkey!("2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    InvalidAccount = 3000,
    InvalidTiming,
    InvalidCouncil,
    Unauthorized,
    StaleProposal,
    DuplicateApproval,
    WrongState,
    InvalidArtifact,
    InvalidEnvelope,
    Overflow,
}
impl From<Error> for ProgramError {
    fn from(value: Error) -> Self {
        Self::Custom(value as u32)
    }
}
pub type Result<T> = std::result::Result<T, ProgramError>;
pub fn require(value: bool, error: Error) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(error.into())
    }
}
pub fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Overflow.into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct Timing {
    pub review_slots: u64,
    pub delay_slots: u64,
    pub expiry_slots: u64,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            review_slots: WEEK_SLOTS,
            delay_slots: MIN_DELAY_SLOTS,
            expiry_slots: if cfg!(feature = "localnet-governance") {
                4500
            } else {
                2_592_000
            },
        }
    }
}
impl Timing {
    pub fn validate(&self) -> Result<()> {
        require(
            self.review_slots >= WEEK_SLOTS && self.delay_slots >= MIN_DELAY_SLOTS,
            Error::InvalidTiming,
        )?;
        let minimum = add(
            self.review_slots.max(self.delay_slots),
            EXECUTION_MARGIN_SLOTS,
        )?;
        require(self.expiry_slots > minimum, Error::InvalidTiming)
    }
    pub fn boundaries(&self, creation: u64) -> Result<(u64, u64, u64)> {
        self.validate()?;
        require(creation > 0, Error::InvalidTiming)?;
        Ok((
            add(creation, self.review_slots)?,
            add(creation, self.delay_slots)?,
            add(creation, self.expiry_slots)?,
        ))
    }
}

pub fn validate_seats(seats: &[Pubkey; 5]) -> Result<()> {
    for (i, key) in seats.iter().enumerate() {
        require(
            *key != Pubkey::default() && !seats[..i].contains(key),
            Error::InvalidCouncil,
        )?;
    }
    Ok(())
}
pub fn config_pda(program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[DOMAIN, b"council"], program)
}
pub fn authority_pda(program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[DOMAIN, b"authority"], program)
}
pub fn target_authority_pda(program: &Pubkey, target: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[DOMAIN, b"target-authority", target.as_ref()], program)
}
pub fn gate_pda(program: &Pubkey, target: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[DOMAIN, b"gate", target.as_ref()], program)
}
pub fn proposal_pda(program: &Pubkey, id: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[DOMAIN, b"proposal", &id.to_le_bytes()], program)
}
pub fn buffer_authority_pda(program: &Pubkey, proposal: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[DOMAIN, b"buffer", proposal.as_ref()], program)
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct Config {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub controller: Pubkey,
    pub programdata: Pubkey,
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub seats: [Pubkey; 5],
    pub council_epoch: u64,
    pub timing_version: u64,
    pub timing: Timing,
    pub next_id: u64,
    pub reserved: [u8; 37],
}
impl Config {
    pub fn validate(&self, program: &Pubkey, key: &Pubkey) -> Result<()> {
        require(
            self.discriminator == *b"AG3CFG01"
                && self.version == 1
                && self.initialized
                && self.reserved == [0; 37]
                && self.controller == *program
                && config_pda(program) == (*key, self.bump)
                && authority_pda(program).0 == self.authority
                && self.programdata
                    == Pubkey::find_program_address(
                        &[program.as_ref()],
                        &solana_sdk_ids::bpf_loader_upgradeable::ID,
                    )
                    .0
                && self.treasury != Pubkey::default()
                && ![*key, self.programdata, self.controller, self.authority]
                    .contains(&self.treasury)
                && self.council_epoch > 0
                && self.timing_version > 0
                && self.next_id > 0,
            Error::InvalidAccount,
        )?;
        validate_seats(&self.seats)?;
        require(!self.seats.contains(&self.authority), Error::InvalidCouncil)?;
        self.timing.validate()
    }
}

/// Fixed action bytes. Unused bytes must be zero; no arbitrary instruction payload.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct Action {
    pub kind: u8,
    pub data: [u8; ACTION_LEN],
}
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct Upgrade {
    pub buffer: Pubkey,
    pub artifact_length: u64,
    pub artifact_sha256: [u8; 32],
    pub merkle_root: [u8; 32],
    pub deployed_slot: u64,
    pub capacity: u64,
    pub source_commitment: [u8; 32],
    pub build_commitment: [u8; 32],
    /// `CONTROLLER_RELEASE` at creation (formerly eight zero bytes).
    pub controller_release: u64,
}
/// Target upgrades commit the artifact through its length-bound Merkle root.
/// The build commitment additionally binds the ordinary file SHA-256 receipt.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct TargetUpgrade {
    pub buffer: Pubkey,
    pub artifact_length: u64,
    pub merkle_root: [u8; 32],
    pub deployed_slot: u64,
    pub capacity: u64,
    pub source_commitment: [u8; 32],
    pub build_commitment: [u8; 32],
    pub target: Pubkey,
    pub gate_epoch: u64,
}
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct TargetGatePolicy {
    pub target: Pubkey,
    pub expected_epoch: u64,
    pub active: bool,
}
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct LightRentPolicy {
    pub base_rent: u16,
    pub compression_cost: u16,
    pub lamports_per_byte_per_epoch: u8,
    pub max_funded_epochs: u8,
    pub max_top_up: u16,
}
/// Only the current Spread tag-216 create-once interface. All other bytes are derived.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct SpreadLightConfigPolicy {
    pub target: Pubkey,
    pub expected_epoch: u64,
    pub deployed_slot: u64,
    pub compression_authority: Pubkey,
    pub rent: LightRentPolicy,
    pub write_top_up: u32,
    pub address_tree: Pubkey,
}
impl SpreadLightConfigPolicy {
    pub const LEN: usize = 124;
    pub fn validate(&self) -> Result<()> {
        require(
            self.target == DEVNET_SPREAD
                && self.expected_epoch > 0
                && self.compression_authority != Pubkey::default()
                && self.address_tree != Pubkey::default(),
            Error::InvalidAccount,
        )
    }
}
pub struct Artifact {
    pub buffer: Pubkey,
    pub artifact_length: u64,
    pub merkle_root: [u8; 32],
    pub deployed_slot: u64,
    pub capacity: u64,
    pub source_commitment: [u8; 32],
    pub build_commitment: [u8; 32],
}
impl Action {
    pub fn verification_policy(&self) -> Result<crate::verification::VerificationPolicy> {
        use crate::verification::VerificationPolicy;
        require(
            self.kind == PUBLISH_VERIFICATION
                && self.data[VerificationPolicy::LEN..].iter().all(|b| *b == 0),
            Error::InvalidAccount,
        )?;
        let value = VerificationPolicy::try_from_slice(&self.data[..VerificationPolicy::LEN])
            .map_err(|_| Error::InvalidAccount)?;
        value.validate()?;
        Ok(value)
    }
    pub fn light_config_policy(&self) -> Result<SpreadLightConfigPolicy> {
        require(
            self.kind == INITIALIZE_SPREAD_LIGHT_CONFIG
                && self.data[SpreadLightConfigPolicy::LEN..]
                    .iter()
                    .all(|b| *b == 0),
            Error::InvalidAccount,
        )?;
        let value =
            SpreadLightConfigPolicy::try_from_slice(&self.data[..SpreadLightConfigPolicy::LEN])
                .map_err(|_| Error::InvalidAccount)?;
        value.validate()?;
        Ok(value)
    }
    pub fn is_upgrade(&self) -> bool {
        matches!(self.kind, UPGRADE_CONTROLLER | UPGRADE_TARGET)
    }
    pub fn target_upgrade(&self) -> Result<TargetUpgrade> {
        require(self.kind == UPGRADE_TARGET, Error::InvalidArtifact)?;
        TargetUpgrade::try_from_slice(&self.data).map_err(|_| Error::InvalidArtifact.into())
    }
    pub fn gate_policy(&self) -> Result<TargetGatePolicy> {
        require(
            self.kind == SET_TARGET_GATE && self.data[41..].iter().all(|x| *x == 0),
            Error::InvalidAccount,
        )?;
        TargetGatePolicy::try_from_slice(&self.data[..41]).map_err(|_| Error::InvalidAccount.into())
    }
    pub fn artifact(&self) -> Result<Artifact> {
        if self.kind == UPGRADE_CONTROLLER {
            let u = self.upgrade()?;
            require(u.artifact_sha256 != [0; 32], Error::InvalidArtifact)?;
            Ok(Artifact {
                buffer: u.buffer,
                artifact_length: u.artifact_length,
                merkle_root: u.merkle_root,
                deployed_slot: u.deployed_slot,
                capacity: u.capacity,
                source_commitment: u.source_commitment,
                build_commitment: u.build_commitment,
            })
        } else {
            let u = self.target_upgrade()?;
            require(
                u.target != Pubkey::default() && u.gate_epoch > 0,
                Error::InvalidAccount,
            )?;
            Ok(Artifact {
                buffer: u.buffer,
                artifact_length: u.artifact_length,
                merkle_root: u.merkle_root,
                deployed_slot: u.deployed_slot,
                capacity: u.capacity,
                source_commitment: u.source_commitment,
                build_commitment: u.build_commitment,
            })
        }
    }
    pub fn upgrade(&self) -> Result<Upgrade> {
        require(self.kind == UPGRADE_CONTROLLER, Error::InvalidArtifact)?;
        Upgrade::try_from_slice(&self.data).map_err(|_| Error::InvalidArtifact.into())
    }
    pub fn timing(&self) -> Result<Timing> {
        require(
            self.kind == SET_TIMING && self.data[24..].iter().all(|x| *x == 0),
            Error::InvalidTiming,
        )?;
        Timing::try_from_slice(&self.data[..24]).map_err(|_| Error::InvalidTiming.into())
    }
    pub fn seats(&self) -> Result<[Pubkey; 5]> {
        require(
            self.kind == ROTATE_COUNCIL && self.data[160..] == [0; 32],
            Error::InvalidCouncil,
        )?;
        <[Pubkey; 5]>::try_from_slice(&self.data[..160]).map_err(|_| Error::InvalidCouncil.into())
    }
    pub fn validate(&self) -> Result<()> {
        match self.kind {
            UPGRADE_CONTROLLER | UPGRADE_TARGET => {
                let u = self.artifact()?;
                require(
                    u.buffer != Pubkey::default()
                        && u.artifact_length > 0
                        && u.artifact_length <= MAX_ARTIFACT_BYTES
                        && u.capacity > 0
                        && u.merkle_root != [0; 32]
                        && u.source_commitment != [0; 32]
                        && u.build_commitment != [0; 32],
                    Error::InvalidArtifact,
                )
            }
            SET_TIMING => self.timing()?.validate(),
            ROTATE_COUNCIL => validate_seats(&self.seats()?),
            SET_TARGET_GATE => {
                let policy = self.gate_policy()?;
                require(
                    policy.target != Pubkey::default() && policy.expected_epoch > 0,
                    Error::InvalidAccount,
                )
            }
            INITIALIZE_SPREAD_LIGHT_CONFIG => self.light_config_policy()?.validate(),
            PUBLISH_VERIFICATION => self.verification_policy()?.validate(),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct Proposal {
    pub discriminator: [u8; 8],
    pub version: u8,
    pub bump: u8,
    pub initialized: bool,
    pub config: Pubkey,
    pub id: u64,
    pub council_epoch: u64,
    pub timing_version: u64,
    pub timing: Timing,
    pub created: u64,
    pub review_end: u64,
    pub not_before: u64,
    pub expires: u64,
    pub action: Action,
    pub digest: [u8; 32],
    pub approvals: u8,
    pub approval_count: u8,
    pub cancellations: u8,
    pub cancellation_count: u8,
    pub state: u8,
    /// Chunk bits 0..96, unchanged from the 1.5 MiB layout.
    pub verified_chunks: [u8; 12],
    pub verified_count: u32,
    pub extension_slot: u64,
    pub extended_capacity: u64,
    pub executed_slot: u64,
    /// Chunk bits 96..128. These were the first four of seven reserved
    /// bytes, which every earlier proposal holds as zero, so the 400-byte
    /// layout and every existing field offset are unchanged.
    pub verified_chunks_high: [u8; 4],
    pub reserved: [u8; 3],
}
impl Proposal {
    /// The logical 128-bit chunk bitmap: low twelve bytes, then high four.
    pub fn verified_bitmap(&self) -> [u8; BITMAP_BYTES] {
        let mut bitmap = [0; BITMAP_BYTES];
        bitmap[..LOW_BITMAP_BYTES].copy_from_slice(&self.verified_chunks);
        bitmap[LOW_BITMAP_BYTES..].copy_from_slice(&self.verified_chunks_high);
        bitmap
    }
    /// Marks chunk `index` once; a repeated chunk is rejected.
    pub fn mark_chunk_verified(&mut self, index: u32) -> Result<()> {
        let index = index as usize;
        require(index < MAX_CHUNKS, Error::InvalidArtifact)?;
        let byte = match self.verified_chunks.get_mut(index / 8) {
            Some(byte) => byte,
            None => &mut self.verified_chunks_high[index / 8 - LOW_BITMAP_BYTES],
        };
        let bit = 1 << (index % 8);
        require(*byte & bit == 0, Error::DuplicateApproval)?;
        *byte |= bit;
        self.verified_count += 1;
        Ok(())
    }
    pub fn compute_digest(&self, program: &Pubkey) -> Result<[u8; 32]> {
        let timing = self.timing.try_to_vec().map_err(|_| Error::InvalidTiming)?;
        let action = self
            .action
            .try_to_vec()
            .map_err(|_| Error::InvalidArtifact)?;
        Ok(hashv(&[
            b"AMOEBA_GOVERNANCE_PROPOSAL_V3",
            DEVNET_GENESIS.as_bytes(),
            program.as_ref(),
            self.config.as_ref(),
            &self.id.to_le_bytes(),
            &self.council_epoch.to_le_bytes(),
            &self.timing_version.to_le_bytes(),
            &timing,
            &self.created.to_le_bytes(),
            &self.review_end.to_le_bytes(),
            &self.not_before.to_le_bytes(),
            &self.expires.to_le_bytes(),
            &action,
        ])
        .to_bytes())
    }
    pub fn validate(&self, program: &Pubkey, key: &Pubkey) -> Result<()> {
        require(
            self.discriminator == *b"AG3PRP01"
                && self.version == 1
                && self.initialized
                && self.config == config_pda(program).0
                && self.id > 0
                && self.council_epoch > 0
                && self.timing_version > 0
                && self.reserved == [0; 3]
                && proposal_pda(program, self.id) == (*key, self.bump)
                && self.approvals < 32
                && self.cancellations < 32
                && self.approvals.count_ones() == u32::from(self.approval_count)
                && self.cancellations.count_ones() == u32::from(self.cancellation_count)
                && self.verified_count
                    == self
                        .verified_bitmap()
                        .iter()
                        .map(|x| x.count_ones())
                        .sum::<u32>()
                && self.state <= EXPIRED,
            Error::InvalidAccount,
        )?;
        require(
            (self.review_end, self.not_before, self.expires)
                == self.timing.boundaries(self.created)?
                && self.digest == self.compute_digest(program)?,
            Error::InvalidTiming,
        )?;
        self.action.validate()?;
        if self.action.is_upgrade() {
            let action = self.action.artifact()?;
            require(
                (self.extension_slot == 0 && self.extended_capacity == 0)
                    || (self.extension_slot >= self.not_before
                        && self.extension_slot < self.expires
                        && self.extended_capacity > action.capacity
                        && self.extended_capacity <= action.artifact_length),
                Error::InvalidArtifact,
            )?;
        } else {
            require(
                self.extension_slot == 0 && self.extended_capacity == 0 && self.verified_count == 0,
                Error::InvalidArtifact,
            )?;
        }
        require(
            (self.state == EXECUTED) == (self.executed_slot != 0)
                && (self.state != EXECUTED
                    || (self.approval_count >= 3
                        && self.executed_slot >= self.not_before
                        && self.executed_slot < self.expires))
                && (self.state == CANCELLED) == (self.cancellation_count >= 3),
            Error::WrongState,
        )
    }
    pub fn current(&self, config: &Config, slot: u64) -> Result<()> {
        require(
            self.state == PENDING && slot < self.expires,
            Error::WrongState,
        )?;
        require(
            self.council_epoch == config.council_epoch,
            Error::StaleProposal,
        )
    }
    pub fn approve(&mut self, index: usize, slot: u64, cancel: bool) -> Result<()> {
        require(
            index < 5
                && self.state == PENDING
                && slot >= self.created
                && slot < self.expires
                && (cancel || slot <= self.review_end),
            Error::InvalidTiming,
        )?;
        let (bits, count) = if cancel {
            (&mut self.cancellations, &mut self.cancellation_count)
        } else {
            (&mut self.approvals, &mut self.approval_count)
        };
        require(*bits & (1 << index) == 0, Error::DuplicateApproval)?;
        *bits |= 1 << index;
        *count = bits.count_ones() as u8;
        if cancel && u32::from(*count) >= THRESHOLD {
            self.state = CANCELLED;
        }
        Ok(())
    }
    pub fn executable(&self, config: &Config, slot: u64) -> Result<()> {
        self.current(config, slot)?;
        require(
            u32::from(self.approval_count) >= THRESHOLD && slot >= self.not_before,
            Error::Unauthorized,
        )
    }
}
