//! Closed Otter Verify metadata recipe. No caller-selected URL, CPI, or arguments.
use crate::state::{require, Error, Result, DEVNET_SPREAD};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{hash::hash, pubkey, pubkey::Pubkey};

pub const VERIFY_PROGRAM: Pubkey = pubkey!("verifycLy8mB96wd9wqq3WDXQwM4oU6r42Th37Db9fC");
pub const BUILD_IMAGE: &str = "solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68";
pub const MAINNET_TARGET_BUILD_IMAGE: &str = "us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/gate-final@sha256:7ad1e567f19831e658f5b892a7269d23d7f1826438e025b5d9d2e385240a4d67";
pub const MAINNET_FUTURES_BUILD_IMAGE: &str = "us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/amba-release@sha256:36bdcd0005a5a41f9eba76add58f16fccdc7a2b7b2fbace1be3aaba8cda7b40e";
pub const INITIALIZE: [u8; 8] = [175, 175, 109, 31, 13, 152, 155, 237];
pub const UPDATE: [u8; 8] = [219, 200, 88, 176, 158, 63, 253, 127];
pub const MAINNET_ORACLE: Pubkey = pubkey!("45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS");
pub const MAINNET_FUTURES: Pubkey = pubkey!("Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu");
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
        require(self.target == DEVNET_SPREAD, Error::InvalidAccount)?;
        self.params_for(controller, true)
    }
    /// Authenticate only the exact release-5 Futures recipe already stored on chain.
    pub(crate) fn previous_futures_params(&self, controller: &Pubkey) -> Result<Vec<u8>> {
        require(self.target == MAINNET_FUTURES, Error::InvalidAccount)?;
        self.params_for(controller, true)
    }
    fn params_for(&self, controller: &Pubkey, previous: bool) -> Result<Vec<u8>> {
        self.validate()?;
        let own = self.target == *controller;
        let spread = self.target == DEVNET_SPREAD;
        let oracle = self.target == MAINNET_ORACLE;
        let futures = self.target == MAINNET_FUTURES;
        require(own || spread || oracle || futures, Error::InvalidAccount)?;
        require(!previous || spread || futures, Error::InvalidAccount)?;
        let repo = repository(own);
        let mut args = Vec::new();
        if spread {
            args.extend([
                "--mount-path",
                "programs/light_token_minter",
                "--workspace-path",
                "programs/light_token_minter",
            ]);
        } else if oracle || futures {
            // The hosted verifier mounts the repository root, preserving sibling sources.
            args.extend([
                "--workspace-path",
                if oracle { "oracle" } else { "futures" },
            ]);
        }
        args.extend([
            "--library-name",
            if own {
                "governance_controller_v3"
            } else if spread {
                "light_token_minter"
            } else if oracle {
                "ameba_oracle"
            } else {
                "ameba_futures"
            },
            "--base-image",
            if futures && !previous {
                MAINNET_FUTURES_BUILD_IMAGE
            } else if oracle || futures {
                MAINNET_TARGET_BUILD_IMAGE
            } else {
                BUILD_IMAGE
            },
            "--arch",
            "v1",
        ]);
        if oracle || futures {
            args.extend(["--cargo-build-sbf-args", "--tools-version v1.53"]);
        }
        if spread {
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
        let version = if oracle || futures { "0.5.2" } else { VERSION };
        for value in [version, repo, &commit] {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_controller_and_spread_recipe_digests_are_unchanged() {
        let controller = Pubkey::new_from_array([31; 32]);
        for (target, expected) in [
            (
                controller,
                "e623ee1b4b62a0c9b03ec5b8c815a253975587ce60cde6ebad43f3da3488d167",
            ),
            (
                DEVNET_SPREAD,
                "c7a8fbcd45c237e911d6f4b7297b3b1317b8cb34823f5d910bb8cc35a0d2d9cc",
            ),
        ] {
            let policy = VerificationPolicy {
                target,
                deployed_slot: 123,
                commit: [0xab; 20],
            };
            let digest: String = hash(&policy.params(&controller).unwrap())
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(digest, expected);
        }
    }

    #[test]
    fn official_program_recipes_keep_the_complete_source_mount() {
        let controller = Pubkey::new_unique();
        for (target, workspace, library) in [
            (MAINNET_ORACLE, "oracle", "ameba_oracle"),
            (MAINNET_FUTURES, "futures", "ameba_futures"),
        ] {
            let policy = VerificationPolicy {
                target,
                deployed_slot: 123,
                commit: [0xab; 20],
            };
            let params = policy.params(&controller).unwrap();
            let decoded =
                <(String, String, String, Vec<String>, u64)>::try_from_slice(&params).unwrap();
            assert_eq!(decoded.0, "0.5.2");
            assert_eq!(decoded.1, "https://github.com/amoeba-farm/amoeba-program");
            assert_eq!(decoded.2, "ab".repeat(20));
            assert_eq!(
                decoded.3,
                [
                    "--workspace-path",
                    workspace,
                    "--library-name",
                    library,
                    "--base-image",
                    if target == MAINNET_FUTURES {
                        MAINNET_FUTURES_BUILD_IMAGE
                    } else {
                        MAINNET_TARGET_BUILD_IMAGE
                    },
                    "--arch",
                    "v1",
                    "--cargo-build-sbf-args",
                    "--tools-version v1.53"
                ]
            );
            assert_eq!(decoded.4, 123);
            assert!(params.len() <= 640);
            assert!(policy.previous_spread_params(&controller).is_err());
            if target == MAINNET_FUTURES {
                let previous = policy.previous_futures_params(&controller).unwrap();
                let decoded =
                    <(String, String, String, Vec<String>, u64)>::try_from_slice(&previous)
                        .unwrap();
                assert_eq!(decoded.3[5], MAINNET_TARGET_BUILD_IMAGE);
                assert_ne!(previous, params);
            } else {
                assert!(policy.previous_futures_params(&controller).is_err());
            }
            let metadata = record(
                &target,
                &target_authority_for_test(&controller, &target),
                &params,
                1,
            );
            assert!(matches_existing_record(&metadata, &metadata, false));
            let mut substituted = metadata.clone();
            substituted[80] ^= 1;
            assert!(!matches_existing_record(&substituted, &metadata, false));
        }
    }

    fn target_authority_for_test(controller: &Pubkey, target: &Pubkey) -> Pubkey {
        crate::state::target_authority_pda(controller, target).0
    }

    #[test]
    fn unknown_targets_and_zero_commits_still_fail() {
        let controller = Pubkey::new_unique();
        let mut policy = VerificationPolicy {
            target: Pubkey::new_unique(),
            deployed_slot: 123,
            commit: [0xab; 20],
        };
        assert!(policy.params(&controller).is_err());
        policy.target = MAINNET_ORACLE;
        policy.commit = [0; 20];
        assert!(policy.params(&controller).is_err());
    }
}
