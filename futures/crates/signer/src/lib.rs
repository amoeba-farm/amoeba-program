//! The keys Amoeba's off-chain code signs with, and where they live: one
//! implementation for the futures API (`api/`: the sponsor) and the futures
//! ops tools (`futures/ops`: the maker, keeper, treasury, council seats and
//! oracle roles).
//!
//! A role's key is a [`SignerSpec`]:
//!
//! * a Solana keypair file ([`FileSigner`]): the dev chain's keys;
//! * a Google Cloud KMS `EC_SIGN_ED25519` key version ([`kms::KmsSigner`]),
//!   which must sign as an address the deployment names: production's keys.
//!
//! [`role_spec`] reads one role's configuration: `FUTURES_<ROLE>_SIGNER`, else
//! the deployment manifest's `keys.<role>`, holding
//! `kms:projects/…/cryptoKeyVersions/N` or a keypair file's path (`file:`
//! optional; a relative path in the manifest is the manifest folder's). A KMS
//! key signs as `FUTURES_<ROLE>_PUBKEY`, else the manifest's
//! `accounts.<role>`; its public key is fetched from KMS and compared before
//! the first signature. Every signature, from either kind of key, is verified
//! against the role's public key before it is used ([`RoleSigner`] for async
//! code, [`blocking::BlockingSigner`] for synchronous tools).
//!
//! Key material is never printed: a file's path and a KMS resource name are
//! the only things logged.
//!
//! Moved here from the futures API (`api/api/src/signer*`, merged at
//! b7e189c4), so the API and the ops tools share one implementation.
pub mod blocking;
pub mod google;
pub mod kms;

use anyhow::{anyhow, bail, Result};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::Signer as _;
use solana_transaction::Transaction;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// The genesis hash of Solana mainnet-beta (Agave's
/// `ClusterType::MainnetBeta`, `solana-cluster-type`; a test checks it).
pub const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

/// Where a role's key is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerSpec {
    /// A Solana keypair file (the 64-byte JSON array `solana-keygen` writes).
    File(PathBuf),
    /// A Cloud KMS CryptoKeyVersion, which must sign as `pubkey`.
    Kms { key_version: String, pubkey: Pubkey },
}

impl SignerSpec {
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }

    /// For logs: the file's path or the KMS key version (never key material).
    pub fn describe(&self) -> String {
        match self {
            Self::File(path) => format!("keypair file {}", path.display()),
            Self::Kms { key_version, .. } => format!("Cloud KMS {key_version}"),
        }
    }
}

/// `FUTURES_<ROLE>_<WHAT>` for a manifest role name: `sponsor` and `SIGNER`
/// give `FUTURES_SPONSOR_SIGNER`, `oracleAuthority` and `PUBKEY` give
/// `FUTURES_ORACLE_AUTHORITY_PUBKEY`, `councilSeat1` gives
/// `FUTURES_COUNCIL_SEAT_1_…`.
pub fn env_name(role: &str, what: &str) -> String {
    let mut out = String::from("FUTURES_");
    let mut previous: Option<char> = None;
    for c in role.chars() {
        let boundary = match previous {
            Some(p) => {
                (c.is_ascii_uppercase() && !p.is_ascii_uppercase())
                    || (c.is_ascii_digit() && !p.is_ascii_digit())
            }
            None => false,
        };
        if boundary || c == '-' || c == '_' {
            out.push('_');
        }
        if c != '-' && c != '_' {
            out.push(c.to_ascii_uppercase());
        }
        previous = Some(c);
    }
    out.push('_');
    out.push_str(what);
    out
}

/// What configures one role's key, in the order it is read.
#[derive(Debug, Clone, Copy)]
pub struct RoleSource<'a> {
    /// The manifest's role name (`sponsor`, `treasury`, `oracleAuthority`).
    pub role: &'a str,
    /// `FUTURES_<ROLE>_SIGNER` ([`env_name`]).
    pub env_signer: Option<&'a str>,
    /// `FUTURES_<ROLE>_PUBKEY`.
    pub env_pubkey: Option<&'a str>,
    /// The manifest's `keys.<role>`.
    pub manifest_signer: Option<&'a str>,
    /// The manifest's `accounts.<role>`.
    pub manifest_pubkey: Option<&'a str>,
    /// The manifest's folder: a relative key path in the manifest is its.
    pub base: &'a Path,
}

fn non_empty(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty())
}

/// The role's signer: the environment's, else the manifest's; `None` when
/// neither configures one. A KMS key version signs as the environment's
/// address, else the manifest's; when both are set they must agree.
pub fn role_spec(source: &RoleSource) -> Result<Option<SignerSpec>> {
    let role = source.role;
    let env_signer = env_name(role, "SIGNER");
    let env_pubkey = env_name(role, "PUBKEY");
    let (text, from_manifest) = match non_empty(source.env_signer) {
        Some(text) => (text, false),
        None => match non_empty(source.manifest_signer) {
            Some(text) => (text, true),
            None => return Ok(None),
        },
    };
    let what = if from_manifest {
        format!("manifest keys.{role}")
    } else {
        env_signer.clone()
    };
    if let Some(version) = text.strip_prefix("kms:") {
        if !kms::valid_key_version(version) {
            bail!(
                "{what}: `kms:` must name one Cloud KMS key version, \
                 projects/<project>/locations/<location>/keyRings/<ring>/cryptoKeys/<key>/cryptoKeyVersions/<n>"
            );
        }
        let parse = |text: &str, from: &str| {
            Pubkey::from_str(text.trim()).map_err(|_| anyhow!("{from} is not a Solana address"))
        };
        let from_env = non_empty(source.env_pubkey)
            .map(|t| parse(t, &env_pubkey))
            .transpose()?;
        let from_manifest = non_empty(source.manifest_pubkey)
            .map(|t| parse(t, &format!("manifest accounts.{role}")))
            .transpose()?;
        let pubkey = match (from_env, from_manifest) {
            (Some(a), Some(b)) if a != b => {
                bail!("{env_pubkey} and the manifest's accounts.{role} differ")
            }
            (Some(k), _) | (None, Some(k)) => k,
            (None, None) => bail!(
                "a KMS {role} key needs the address it signs as: set {env_pubkey} \
                 or the manifest's accounts.{role}"
            ),
        };
        return Ok(Some(SignerSpec::Kms {
            key_version: version.to_owned(),
            pubkey,
        }));
    }
    let path = PathBuf::from(text.strip_prefix("file:").unwrap_or(text));
    if path.as_os_str().is_empty() {
        bail!("{what}: the keypair file path is empty");
    }
    Ok(Some(SignerSpec::File(
        if from_manifest && path.is_relative() {
            source.base.join(path)
        } else {
            path
        },
    )))
}

/// Whether the deployment is Solana mainnet: its `cluster` says so, or its
/// genesis hash is mainnet's.
pub fn is_mainnet(cluster: &str, genesis: Option<&str>) -> bool {
    matches!(
        cluster.trim().to_ascii_lowercase().as_str(),
        "mainnet-beta" | "mainnet"
    ) || genesis.map(str::trim) == Some(MAINNET_GENESIS)
}

/// Refuses a keypair-file key for `role` on mainnet. `allow` is the explicit
/// override where a role has one (`FUTURES_ALLOW_FILE_SIGNER_ON_MAINNET`, the
/// API's sponsor), `None` where it has none (the ops tools' treasury and
/// council seats). A key-handling protection, not a product limit:
/// production keys live in Cloud KMS (AGENTS.md "Shipping to production");
/// the API's KMS signer task (2026-10-16, preflight
/// `ship/releases/2026-10-15-preflight/api-sdk.md` D4) asked the sponsor to
/// refuse a file on `mainnet-beta` unless overridden, and the ops tools'
/// mainnet task (2026-10-16) that a mainnet config refuse file signers for
/// the treasury and the council.
pub fn refuse_file_signer_on_mainnet(
    role: &str,
    spec: &SignerSpec,
    cluster: &str,
    genesis: Option<&str>,
    allow: Option<bool>,
) -> Result<()> {
    if !spec.is_file() || !is_mainnet(cluster, genesis) || allow == Some(true) {
        return Ok(());
    }
    let signer = env_name(role, "SIGNER");
    match allow {
        Some(_) => bail!(
            "refusing to sign on mainnet with a keypair file ({}): set {signer}=kms:<key version>, \
             or FUTURES_ALLOW_FILE_SIGNER_ON_MAINNET=1 to override",
            spec.describe()
        ),
        None => bail!(
            "refusing to sign on mainnet as {role} with a keypair file ({}): its key must be a \
             Cloud KMS key version (the manifest's keys.{role}, or {signer}=kms:<key version>)",
            spec.describe()
        ),
    }
}

/// A keypair held in memory, read once from its file.
pub struct FileSigner {
    keypair: Keypair,
}

impl FileSigner {
    pub fn new(keypair: Keypair) -> Self {
        Self { keypair }
    }

    /// The keypair in `path`; an error names the path only.
    pub fn read(path: &Path) -> Result<Self> {
        solana_keypair::read_keypair_file(path)
            .map(Self::new)
            .map_err(|_| anyhow!("cannot read the keypair file {}", path.display()))
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }
}

/// One role's key for async code: a keypair file or a Cloud KMS key version.
#[allow(clippy::large_enum_variant)] // a handful per process
pub enum RoleSigner {
    File(FileSigner),
    Kms(kms::KmsSigner),
}

impl From<Keypair> for RoleSigner {
    fn from(keypair: Keypair) -> Self {
        Self::File(FileSigner::new(keypair))
    }
}

impl RoleSigner {
    /// The signer `spec` names. A KMS key authenticates with Application
    /// Default Credentials ([`google`]) and must be an enabled Ed25519 key
    /// whose public key is the configured address; anything else is an error.
    pub async fn connect(spec: &SignerSpec) -> Result<Self> {
        match spec {
            SignerSpec::File(path) => FileSigner::read(path).map(Self::File),
            SignerSpec::Kms {
                key_version,
                pubkey,
            } => {
                let credentials = google::Credentials::from_environment()?;
                kms::KmsSigner::connect(key_version, *pubkey, credentials, kms::KMS_API)
                    .await
                    .map(Self::Kms)
            }
        }
    }

    pub fn pubkey(&self) -> Pubkey {
        match self {
            Self::File(f) => f.pubkey(),
            Self::Kms(k) => k.pubkey(),
        }
    }

    /// For logs (never key material).
    pub fn describe(&self) -> String {
        match self {
            Self::File(_) => "keypair file".into(),
            Self::Kms(k) => format!("Cloud KMS {} ({})", k.key_version(), k.credentials()),
        }
    }

    /// The Ed25519 signature over exactly `message`, verified against the
    /// signer's public key.
    pub async fn sign_message(&self, message: &[u8]) -> Result<Signature> {
        let signature = match self {
            Self::File(f) => f.keypair.sign_message(message),
            Self::Kms(k) => k.sign(message).await?,
        };
        if !signature.verify(self.pubkey().as_ref(), message) {
            bail!(
                "the signature does not verify against the signer's public key {}",
                self.pubkey()
            );
        }
        Ok(signature)
    }

    /// Puts this signer's signature into `tx`, whose required signers must
    /// include it, over its unchanged message. The other signatures are left
    /// as they are.
    pub async fn sign_transaction(&self, tx: &mut Transaction) -> Result<()> {
        let key = self.pubkey();
        let signers = usize::from(tx.message.header.num_required_signatures);
        if tx.signatures.len() != signers || tx.message.account_keys.len() < signers {
            bail!("unexpected signature count");
        }
        let index = tx.message.account_keys[..signers]
            .iter()
            .position(|k| *k == key)
            .ok_or_else(|| anyhow!("{key} is not a signer of this transaction"))?;
        let message = tx.message_data();
        tx.signatures[index] = self.sign_message(&message).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str =
        "projects/amoeba-test1/locations/global/keyRings/ring/cryptoKeys/sponsor/cryptoKeyVersions/1";

    /// Whether every required signature of `tx` verifies over its message.
    pub(crate) fn all_signatures_verify(tx: &Transaction) -> bool {
        let message = tx.message_data();
        let signers = usize::from(tx.message.header.num_required_signatures);
        tx.signatures.len() == signers
            && tx
                .signatures
                .iter()
                .zip(&tx.message.account_keys)
                .all(|(s, k)| s.verify(k.as_ref(), &message))
    }

    #[test]
    fn mainnet_genesis_is_agaves() {
        let agave = solana_cluster_type::ClusterType::MainnetBeta
            .get_genesis_hash()
            .expect("mainnet-beta has a genesis hash");
        assert_eq!(agave.to_string(), MAINNET_GENESIS);
    }

    #[test]
    fn env_names_follow_the_role() {
        assert_eq!(env_name("sponsor", "SIGNER"), "FUTURES_SPONSOR_SIGNER");
        assert_eq!(
            env_name("oracleAuthority", "PUBKEY"),
            "FUTURES_ORACLE_AUTHORITY_PUBKEY"
        );
        assert_eq!(
            env_name("councilSeat1", "SIGNER"),
            "FUTURES_COUNCIL_SEAT_1_SIGNER"
        );
        assert_eq!(
            env_name("oracleSigner5", "PUBKEY"),
            "FUTURES_ORACLE_SIGNER_5_PUBKEY"
        );
        assert_eq!(env_name("maker", "SIGNER"), "FUTURES_MAKER_SIGNER");
    }

    fn source<'a>(
        role: &'a str,
        env: (Option<&'a str>, Option<&'a str>),
        manifest: (Option<&'a str>, Option<&'a str>),
    ) -> RoleSource<'a> {
        RoleSource {
            role,
            env_signer: env.0,
            env_pubkey: env.1,
            manifest_signer: manifest.0,
            manifest_pubkey: manifest.1,
            base: Path::new("/srv/deploy"),
        }
    }

    #[test]
    fn a_role_is_a_file_or_a_kms_key_version() {
        let address = Pubkey::new_unique().to_string();
        let other = Pubkey::new_unique().to_string();
        let kms = format!("kms:{VERSION}");
        // Nothing configured: no signer (the caller decides whether it needs one).
        assert_eq!(
            role_spec(&source("keeper", (None, None), (None, None))).unwrap(),
            None
        );
        assert_eq!(
            role_spec(&source("keeper", (Some(" "), None), (Some(""), None))).unwrap(),
            None
        );
        // The manifest's file, relative to the manifest; the environment's as given.
        assert_eq!(
            role_spec(&source(
                "keeper",
                (None, None),
                (Some("keys/keeper.json"), None)
            ))
            .unwrap(),
            Some(SignerSpec::File("/srv/deploy/keys/keeper.json".into()))
        );
        assert_eq!(
            role_spec(&source(
                "keeper",
                (Some("file:k.json"), None),
                (Some("keys/keeper.json"), None)
            ))
            .unwrap(),
            Some(SignerSpec::File("k.json".into()))
        );
        // A KMS key version and the address it signs as.
        let want = Some(SignerSpec::Kms {
            key_version: VERSION.into(),
            pubkey: address.parse().unwrap(),
        });
        assert_eq!(
            role_spec(&source(
                "treasury",
                (None, None),
                (Some(&kms), Some(&address))
            ))
            .unwrap(),
            want
        );
        assert_eq!(
            role_spec(&source(
                "treasury",
                (Some(&kms), Some(&address)),
                (None, None)
            ))
            .unwrap(),
            want
        );
        assert_eq!(
            role_spec(&source(
                "treasury",
                (None, Some(&address)),
                (Some(&kms), Some(&address))
            ))
            .unwrap(),
            want
        );
        // ... and refuses what it cannot pin.
        let no_address =
            role_spec(&source("treasury", (None, None), (Some(&kms), None))).unwrap_err();
        assert!(
            no_address.to_string().contains("FUTURES_TREASURY_PUBKEY"),
            "{no_address:#}"
        );
        let differ = role_spec(&source(
            "treasury",
            (None, Some(&other)),
            (Some(&kms), Some(&address)),
        ))
        .unwrap_err();
        assert!(differ.to_string().contains("differ"), "{differ:#}");
        assert!(role_spec(&source("treasury", (None, Some("x")), (Some(&kms), None))).is_err());
        for bad in [
            "kms:",
            "kms:projects/x/keys/1",
            "kms:projects/amoeba-test1/locations/global/keyRings/ring/cryptoKeys/sponsor",
            "file:",
        ] {
            assert!(
                role_spec(&source(
                    "treasury",
                    (Some(bad), None),
                    (None, Some(&address))
                ))
                .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn mainnet_refuses_a_file_where_the_role_needs_kms() {
        let file = SignerSpec::File("/srv/treasury.json".into());
        let kms = SignerSpec::Kms {
            key_version: VERSION.into(),
            pubkey: Pubkey::new_unique(),
        };
        for (cluster, genesis) in [
            ("mainnet-beta", None),
            ("Mainnet-Beta", None),
            ("mainnet", None),
            ("localnet", Some(MAINNET_GENESIS)),
        ] {
            // No override for this role.
            let e = refuse_file_signer_on_mainnet("treasury", &file, cluster, genesis, None)
                .unwrap_err();
            assert!(
                e.to_string().contains("refusing to sign on mainnet"),
                "{e:#}"
            );
            assert!(e.to_string().contains("FUTURES_TREASURY_SIGNER"), "{e:#}");
            // With one (the sponsor's), only when it is set.
            let e = refuse_file_signer_on_mainnet("sponsor", &file, cluster, genesis, Some(false))
                .unwrap_err();
            assert!(
                e.to_string()
                    .contains("FUTURES_ALLOW_FILE_SIGNER_ON_MAINNET"),
                "{e:#}"
            );
            refuse_file_signer_on_mainnet("sponsor", &file, cluster, genesis, Some(true)).unwrap();
            refuse_file_signer_on_mainnet("treasury", &kms, cluster, genesis, None).unwrap();
        }
        for cluster in ["localnet", "devnet", "testnet"] {
            refuse_file_signer_on_mainnet(
                "treasury",
                &file,
                cluster,
                Some("A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX"),
                None,
            )
            .expect("not mainnet");
        }
    }

    #[tokio::test]
    async fn a_file_signer_fills_only_its_slot() {
        let key = Keypair::new();
        let owner = Keypair::new();
        let signer = RoleSigner::from(key.insecure_clone());
        let ix = solana_program::instruction::Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[1],
            vec![solana_program::instruction::AccountMeta::new(
                owner.pubkey(),
                true,
            )],
        );
        let message = solana_message::Message::new(&[ix], Some(&key.pubkey()));
        let mut tx = Transaction::new_unsigned(message);
        tx.partial_sign(&[&owner], tx.message.recent_blockhash);
        signer.sign_transaction(&mut tx).await.unwrap();
        assert!(all_signatures_verify(&tx), "every signature verifies");
        let mut foreign =
            Transaction::new_unsigned(solana_message::Message::new(&[], Some(&owner.pubkey())));
        assert!(signer.sign_transaction(&mut foreign).await.is_err());
    }
}
