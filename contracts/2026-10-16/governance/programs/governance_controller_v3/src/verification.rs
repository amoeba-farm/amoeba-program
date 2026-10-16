//! Closed Otter Verify metadata recipe. No caller-selected URL, CPI, or arguments.
use crate::state::{require, Error, Result, DEVNET_SPREAD};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hash, pubkey, pubkey::Pubkey};

pub const VERIFY_PROGRAM: Pubkey = pubkey!("verifycLy8mB96wd9wqq3WDXQwM4oU6r42Th37Db9fC");
pub const BUILD_IMAGE: &str = "solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68";
pub const INITIALIZE: [u8; 8] = [175, 175, 109, 31, 13, 152, 155, 237];
pub const UPDATE: [u8; 8] = [219, 200, 88, 176, 158, 63, 253, 127];
const VERSION: &str = "0.5.1";

fn repository(own: bool) -> &'static str {
    if own {
        "https://github.com/amoeba-farm/amoeba-governance"
    } else {
        "https://github.com/amoeba-farm/amoeba-program"
    }
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct VerificationPolicy {
    pub target: Pubkey,
    pub deployed_slot: u64,
    pub commit: [u8; 20],
}
impl VerificationPolicy {
    pub const LEN: usize = 60;
    pub fn validate(&self) -> Result<()> {
        require(
            self.target != Pubkey::default() && self.commit != [0; 20],
            Error::InvalidAccount,
        )
    }
    /// Construct the exact external ABI, allocating only fixed, trusted strings.
    pub fn params(&self, controller: &Pubkey) -> Result<Vec<u8>> {
        self.params_for(controller, false)
    }
    /// Only authenticates existing records from the previous closed Spread recipe.
    /// New publications always use `params` and cannot select this recipe.
    pub(crate) fn previous_spread_params(&self, controller: &Pubkey) -> Result<Vec<u8>> {
        require(self.target != *controller, Error::InvalidAccount)?;
        self.params_for(controller, true)
    }
    fn params_for(&self, controller: &Pubkey, previous: bool) -> Result<Vec<u8>> {
        self.validate()?;
        let own = self.target == *controller;
        require(own || self.target == DEVNET_SPREAD, Error::InvalidAccount)?;
        let repo = repository(own);
        let mut args = Vec::new();
        if !own {
            args.extend([
                "--mount-path",
                "programs/light_token_minter",
                "--workspace-path",
                "programs/light_token_minter",
            ]);
        }
        args.extend([
            "--library-name",
            if own {
                "governance_controller_v3"
            } else {
                "light_token_minter"
            },
            "--base-image",
            BUILD_IMAGE,
            "--arch",
            "v1",
        ]);
        if !own {
            if previous {
                args.extend([
                    "--",
                    "--no-default-features",
                    "--features",
                    "mainnet-v3,devnet-solo-backfill-2026",
                ]);
            } else {
                args.extend(["--", "--features", "mainnet-v3"]);
            }
        }
        let mut commit = String::with_capacity(40);
        for byte in self.commit {
            use std::fmt::Write;
            write!(&mut commit, "{byte:02x}").map_err(|_| Error::InvalidAccount)?;
        }
        let mut bytes = Vec::with_capacity(640);
        for value in [VERSION, repo, &commit] {
            string(&mut bytes, value);
        }
        bytes.extend_from_slice(&(args.len() as u32).to_le_bytes());
        for value in args {
            string(&mut bytes, value);
        }
        bytes.extend_from_slice(&self.deployed_slot.to_le_bytes());
        require(bytes.len() <= 640, Error::InvalidAccount)?;
        Ok(bytes)
    }
}

/// Every stored byte must match a closed recipe except the prior commit and slot.
/// The commit remains an exactly forty-character, nonzero lowercase hex value.
pub(crate) fn matches_existing_record(bytes: &[u8], expected: &[u8], own: bool) -> bool {
    let commit_start = 72 + 4 + VERSION.len() + 4 + repository(own).len() + 4;
    let commit_end = commit_start + 40;
    if bytes.len() != expected.len() || bytes.len() < commit_end + 9 {
        return false;
    }
    let commit = &bytes[commit_start..commit_end];
    let slot_start = bytes.len() - 9;
    bytes[..commit_start] == expected[..commit_start]
        && commit
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        && commit.iter().any(|b| *b != b'0')
        && bytes[commit_end..slot_start] == expected[commit_end..slot_start]
        && bytes[bytes.len() - 1] == expected[expected.len() - 1]
}
fn string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}
pub fn metadata_pda(authority: &Pubkey, target: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"otter_verify", authority.as_ref(), target.as_ref()],
        &VERIFY_PROGRAM,
    )
}
pub fn discriminator() -> [u8; 8] {
    hash(b"account:BuildParams").to_bytes()[..8]
        .try_into()
        .unwrap()
}
pub fn record(target: &Pubkey, authority: &Pubkey, params: &[u8], bump: u8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(73 + params.len());
    bytes.extend_from_slice(&discriminator());
    bytes.extend_from_slice(target.as_ref());
    bytes.extend_from_slice(authority.as_ref());
    bytes.extend_from_slice(params);
    bytes.push(bump);
    bytes
}
