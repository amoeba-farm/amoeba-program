use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

fn instruction_data(discriminant: u32, fields: &[&[u8]]) -> Vec<u8> {
    let payload_len = fields.iter().map(|field| field.len()).sum::<usize>();
    let mut data = Vec::with_capacity(4 + payload_len);
    data.extend_from_slice(&discriminant.to_le_bytes());
    for field in fields {
        data.extend_from_slice(field);
    }
    data
}

/// Build the canonical bincode `SystemInstruction::CreateAccount` payload
/// without linking the general bincode/serde constructor path on chain.
pub fn create_account(
    from: &Pubkey,
    to: &Pubkey,
    lamports: u64,
    space: u64,
    owner: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: system_program::id(),
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, true)],
        data: instruction_data(
            0,
            &[
                &lamports.to_le_bytes(),
                &space.to_le_bytes(),
                owner.as_ref(),
            ],
        ),
    }
}

/// Build the canonical bincode `SystemInstruction::Assign` payload.
pub fn assign(account: &Pubkey, owner: &Pubkey) -> Instruction {
    Instruction {
        program_id: system_program::id(),
        accounts: vec![AccountMeta::new(*account, true)],
        data: instruction_data(1, &[owner.as_ref()]),
    }
}

/// Build the canonical bincode `SystemInstruction::Transfer` payload.
pub fn transfer(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    Instruction {
        program_id: system_program::id(),
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
        data: instruction_data(2, &[&lamports.to_le_bytes()]),
    }
}

/// Build the canonical bincode `SystemInstruction::Allocate` payload.
pub fn allocate(account: &Pubkey, space: u64) -> Instruction {
    Instruction {
        program_id: system_program::id(),
        accounts: vec![AccountMeta::new(*account, true)],
        data: instruction_data(8, &[&space.to_le_bytes()]),
    }
}
