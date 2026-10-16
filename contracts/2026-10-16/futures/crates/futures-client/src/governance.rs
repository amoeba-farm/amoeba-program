//! Bind instructions to the controller's current gate epochs before signing.
//! Read these accounts with the market snapshot. A freeze or intervening
//! upgrade invalidates the prepared transaction; rebuild it from a fresh read.
pub use ameba_governance_interface::{
    derive_protocol_gate_pda as gate_address, read_gate, GateError, GateStatusV1,
    GovernanceInstructionTailV1, ProtocolGateV1, GOVERNANCE_TAIL_LEN,
    LOCALNET_CONTROLLER_PROGRAM_ID, PRODUCTION_CONTROLLER_PROGRAM_ID,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};

#[derive(Clone, Copy, Debug)]
pub struct GateBinding {
    pub controller: Pubkey,
    pub target: Pubkey,
    pub address: Pubkey,
    pub epoch: u64,
}
impl GateBinding {
    /// Rejects substituted accounts, malformed state and frozen targets.
    pub fn from_account(
        controller: Pubkey,
        target: Pubkey,
        address: Pubkey,
        owner: Pubkey,
        data: &[u8],
    ) -> Result<Self, ProgramError> {
        let gate = read_gate(&controller, &target, &address, &owner, data)?;
        if gate.status != GateStatusV1::Active {
            return Err(GateError::GovernanceGateFrozen.into());
        }
        Ok(Self {
            controller,
            target,
            address,
            epoch: gate.epoch,
        })
    }
    /// Adds the gate account and signed epoch to a raw instruction.
    pub fn bind(&self, mut ix: Instruction) -> Result<Instruction, ProgramError> {
        if ix.program_id != self.target {
            return Err(ProgramError::IncorrectProgramId);
        }
        if ix.accounts.iter().any(|a| a.pubkey == self.address) {
            return Err(ProgramError::InvalidArgument);
        }
        ix.accounts
            .push(AccountMeta::new_readonly(self.address, false));
        ix.data
            .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(self.epoch).encode());
        Ok(ix)
    }
}

/// Authorize a new futures market (its registry) after the program's upgrade
/// authority has moved to the controller. Add this before binding the gate.
pub fn with_council_authorization(
    mut ix: Instruction,
    controller: &Pubkey,
    seats: &[Pubkey; 3],
) -> Result<Instruction, ProgramError> {
    if seats[0] == seats[1] || seats[0] == seats[2] || seats[1] == seats[2] {
        return Err(ProgramError::InvalidArgument);
    }
    let config = Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], controller).0;
    ix.accounts.push(AccountMeta::new_readonly(config, false));
    for seat in seats {
        ix.accounts.push(AccountMeta::new_readonly(*seat, true));
    }
    Ok(ix)
}

/// Shared verifier error codes, distinct from each target program's own errors.
pub fn error_name(code: u32) -> Option<&'static str> {
    const NAMES: [&str; 10] = [
        "InvalidGovernanceTail",
        "InvalidGovernanceGateData",
        "MissingGovernanceTail",
        "MissingGovernanceGate",
        "InvalidGovernanceGatePda",
        "DuplicateGovernanceGate",
        "InvalidGovernanceGatePrivileges",
        "InvalidGovernanceGateOwner",
        "GovernanceGateFrozen",
        "GovernanceGateEpochMismatch",
    ];
    NAMES.get(code.checked_sub(3100)? as usize).copied()
}
