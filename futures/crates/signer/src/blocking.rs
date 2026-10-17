//! A role's key as a [`solana_signer::Signer`], for synchronous tools (the
//! futures ops tools): it goes wherever a `Keypair` goes (`Transaction::
//! try_sign`, an Ed25519 attestation), whatever holds the key.
//!
//! * A keypair file signs in process.
//! * A Cloud KMS key version ([`KmsSigner`]) is checked once when it
//!   connects (its public key must be the configured address) and signs
//!   each message with one `asymmetricSign`, on a small runtime this module
//!   keeps for the whole process. Don't call it from inside another async
//!   runtime.
//! * An offline signer ([`BlockingSigner::offline`]) knows only the address:
//!   a dry run builds and prints transactions with it and never signs, so a
//!   review needs no key and no credentials.
//!
//! Every signature is verified against the role's public key before it is
//! returned.
use crate::{
    google::Credentials,
    kms::{self, KmsSigner},
    FileSigner, SignerSpec,
};
use anyhow::{anyhow, Result};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::{Signer, SignerError};
use std::sync::OnceLock;

/// The runtime every KMS request of the process runs on: one worker, so the
/// HTTP client's connections keep being served between signatures.
fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> =
        OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("ameba-signer")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow!("the signer's runtime could not start: {e}"))
}

enum Key {
    File(Keypair),
    Kms(KmsSigner),
    /// A dry run's: the address only.
    Offline,
}

/// One role's key (`treasury`, `keeper`, `councilSeat1`, ...).
pub struct BlockingSigner {
    role: String,
    describe: String,
    pubkey: Pubkey,
    key: Key,
}

impl std::fmt::Debug for BlockingSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} ({})", self.role, self.pubkey, self.describe)
    }
}

impl BlockingSigner {
    /// The key `spec` names, ready to sign. A KMS key authenticates with
    /// Application Default Credentials and must be an enabled Ed25519 key
    /// whose public key is the configured address.
    pub fn connect(role: &str, spec: &SignerSpec) -> Result<Self> {
        match spec {
            SignerSpec::File(path) => {
                let file = FileSigner::read(path).map_err(|e| anyhow!("{role}: {e:#}"))?;
                Ok(Self::from_keypair(
                    role,
                    file.keypair().insecure_clone(),
                    &spec.describe(),
                ))
            }
            SignerSpec::Kms {
                key_version,
                pubkey,
            } => {
                let credentials = Credentials::from_environment()?;
                Self::connect_kms(role, key_version, *pubkey, credentials, kms::KMS_API)
            }
        }
    }

    /// The KMS key version `key_version` at `api` (Google's [`kms::KMS_API`]
    /// except against a stub), reached with `credentials`; it must sign as
    /// `pubkey`.
    pub fn connect_kms(
        role: &str,
        key_version: &str,
        pubkey: Pubkey,
        credentials: Credentials,
        api: &str,
    ) -> Result<Self> {
        let signer = runtime()?
            .block_on(KmsSigner::connect(key_version, pubkey, credentials, api))
            .map_err(|e| anyhow!("{role}: {e:#}"))?;
        let describe = format!(
            "Cloud KMS {} ({})",
            signer.key_version(),
            signer.credentials()
        );
        Ok(Self {
            role: role.to_owned(),
            describe,
            pubkey,
            key: Key::Kms(signer),
        })
    }

    /// A keypair already in memory (a dev key, a fresh account's key).
    pub fn from_keypair(role: &str, keypair: Keypair, describe: &str) -> Self {
        Self {
            role: role.to_owned(),
            describe: describe.to_owned(),
            pubkey: keypair.pubkey(),
            key: Key::File(keypair),
        }
    }

    /// A dry run's signer for `pubkey`: it is never asked for a signature,
    /// and refuses if it is.
    pub fn offline(role: &str, pubkey: Pubkey, describe: &str) -> Self {
        Self {
            role: role.to_owned(),
            describe: describe.to_owned(),
            pubkey,
            key: Key::Offline,
        }
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    /// Where the key is (a file's path or a KMS key version; never key
    /// material).
    pub fn describe(&self) -> &str {
        &self.describe
    }

    pub fn is_offline(&self) -> bool {
        matches!(self.key, Key::Offline)
    }

    pub fn is_kms(&self) -> bool {
        matches!(self.key, Key::Kms(_))
    }

    /// The keypair of a key held in process (a file's), for the dev tools
    /// that hand it to another program (the Agave CLI).
    pub fn keypair(&self) -> Option<&Keypair> {
        match &self.key {
            Key::File(k) => Some(k),
            _ => None,
        }
    }
}

impl Signer for BlockingSigner {
    fn try_pubkey(&self) -> std::result::Result<Pubkey, SignerError> {
        Ok(self.pubkey)
    }

    fn try_sign_message(&self, message: &[u8]) -> std::result::Result<Signature, SignerError> {
        let signature = match &self.key {
            Key::File(keypair) => keypair.sign_message(message),
            Key::Kms(kms) => runtime()
                .and_then(|rt| rt.block_on(kms.sign(message)))
                .map_err(|e| {
                    SignerError::Custom(format!("{} ({}): {e:#}", self.role, self.describe))
                })?,
            Key::Offline => {
                return Err(SignerError::Custom(format!(
                    "{} ({}) does not sign: this is a dry run",
                    self.role, self.describe
                )))
            }
        };
        if !signature.verify(self.pubkey.as_ref(), message) {
            return Err(SignerError::Custom(format!(
                "{}'s signature does not verify against its address {}",
                self.role, self.pubkey
            )));
        }
        Ok(signature)
    }

    fn is_interactive(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kms::tests::{metadata, serve, Fault, Stub, VERSION};
    use std::sync::Arc;

    /// The KMS stub on its own runtime, in a thread of its own (a blocking
    /// signer may not run inside another runtime).
    fn start(fault: Fault) -> (String, Arc<Stub>) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                tx.send(serve(fault, None).await).unwrap();
                std::future::pending::<()>().await;
            });
        });
        rx.recv().unwrap()
    }

    fn connect(base: &str, pubkey: Pubkey) -> Result<BlockingSigner> {
        BlockingSigner::connect_kms(
            "treasury",
            VERSION,
            pubkey,
            metadata(base),
            &format!("{base}/kms"),
        )
    }

    #[test]
    fn a_kms_key_signs_transactions_like_a_keypair() {
        let (base, stub) = start(Fault::None);
        let treasury =
            connect(&base, stub.key.pubkey()).expect("the key is the configured address's");
        assert!(treasury.is_kms() && !treasury.is_offline());
        assert_eq!(treasury.pubkey(), stub.key.pubkey());
        assert!(
            treasury.describe().starts_with("Cloud KMS projects/"),
            "{treasury:?}"
        );
        let signature = treasury.try_sign_message(b"message").unwrap();
        assert_eq!(
            signature,
            stub.key.sign_message(b"message"),
            "Ed25519 is deterministic"
        );
        // Beside a keypair in one transaction.
        let other = Keypair::new();
        let ix = solana_program::instruction::Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[3],
            vec![solana_program::instruction::AccountMeta::new(
                other.pubkey(),
                true,
            )],
        );
        let mut tx =
            solana_transaction::Transaction::new_with_payer(&[ix], Some(&treasury.pubkey()));
        let signers: Vec<&dyn Signer> = vec![&treasury, &other];
        tx.try_sign(&signers, solana_program::hash::Hash::default())
            .unwrap();
        assert!(crate::tests::all_signatures_verify(&tx));
    }

    #[test]
    fn a_signature_that_does_not_verify_is_refused() {
        let (base, stub) = start(Fault::ForeignSignature);
        let treasury = connect(&base, stub.key.pubkey()).expect("the key itself is right");
        let e = treasury.try_sign_message(b"message").unwrap_err();
        assert!(e.to_string().contains("does not verify"), "{e}");
        // A key that is not the configured address never connects.
        let (base, _) = start(Fault::None);
        let e = connect(&base, Pubkey::new_unique()).unwrap_err();
        assert!(e.to_string().contains("refusing to sign with it"), "{e:#}");
    }

    #[test]
    fn an_offline_signer_never_signs_and_never_calls_kms() {
        let (_, stub) = start(Fault::None);
        let spec = SignerSpec::Kms {
            key_version: VERSION.into(),
            pubkey: stub.key.pubkey(),
        };
        let dry = BlockingSigner::offline("treasury", stub.key.pubkey(), &spec.describe());
        assert!(dry.is_offline());
        assert_eq!(dry.pubkey(), stub.key.pubkey());
        let e = dry.try_sign_message(b"message").unwrap_err();
        assert!(e.to_string().contains("dry run"), "{e}");
        assert!(
            stub.seen.lock().unwrap().is_empty(),
            "no request reached KMS"
        );
    }

    #[test]
    fn a_file_signer_reads_its_keypair() {
        let dir = std::env::temp_dir().join(format!("ameba-signer-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keeper.json");
        let key = Keypair::new();
        solana_keypair::write_keypair_file(&key, &path).unwrap();
        let keeper = BlockingSigner::connect("keeper", &SignerSpec::File(path.clone())).unwrap();
        assert_eq!(keeper.pubkey(), key.pubkey());
        assert_eq!(
            keeper.try_sign_message(b"m").unwrap(),
            key.sign_message(b"m")
        );
        assert!(keeper.keypair().is_some());
        assert!(!format!("{keeper:?}").contains(&key.to_base58_string()));
        let missing =
            BlockingSigner::connect("keeper", &SignerSpec::File(dir.join("missing.json")))
                .unwrap_err();
        assert!(missing.to_string().contains("missing.json"), "{missing:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
