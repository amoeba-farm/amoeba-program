//! Google Cloud KMS signing, ported from the old Spread backend's custody
//! (`api/crates/signer/src/kms.rs`, removed in 49801a67): one pinned
//! `EC_SIGN_ED25519` CryptoKeyVersion whose public key must be the
//! configured Solana address; `asymmetricSign` over the exact message bytes,
//! which returns a raw 64-byte Ed25519 signature; and every signature
//! verified locally before use.
//!
//! At start, `getPublicKey` reads the version's public key, which must be
//! exact Ed25519 SPKI of the configured address. Then each signature is one
//! `asymmetricSign` request (plus a token refresh about hourly). The key
//! needs `cloudkms.cryptoKeyVersions.viewPublicKey` and `…useToSign`, both in
//! `roles/cloudkms.signerVerifier`. A disabled or destroyed version fails at
//! the `getPublicKey`.
use super::google::{json_answer, pem_body, Credentials};
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use std::time::Duration;

/// Cloud KMS's REST API.
pub const KMS_API: &str = "https://cloudkms.googleapis.com/v1";
/// The DER prefix of an Ed25519 SubjectPublicKeyInfo (RFC 8410); the 32-byte
/// key follows it.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
/// The whole of one request to Google, so a request that never completes
/// fails instead of holding a submit open. The value is the old backend's
/// (`REQUEST_TIMEOUT` in its kms.rs); nothing is retried.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

fn id_part(text: &str) -> bool {
    let bytes = text.as_bytes();
    (6..=30).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[bytes.len() - 1] != b'-'
}

fn resource_part(text: &str) -> bool {
    (1..=63).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One exact CryptoKeyVersion resource name:
/// `projects/<p>/locations/<l>/keyRings/<r>/cryptoKeys/<k>/cryptoKeyVersions/<n>`
/// (the old backend's `valid_key_version`).
pub fn valid_key_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('/').collect();
    let ["projects", project, "locations", location, "keyRings", ring, "cryptoKeys", key, "cryptoKeyVersions", version] =
        parts.as_slice()
    else {
        return false;
    };
    id_part(project)
        && !location.is_empty()
        && location
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && resource_part(ring)
        && resource_part(key)
        && version
            .bytes()
            .next()
            .is_some_and(|b| (b'1'..=b'9').contains(&b))
        && version.bytes().all(|b| b.is_ascii_digit())
}

/// The Ed25519 key in a KMS public-key PEM (exact SPKI).
pub fn ed25519_from_pem(pem: &str) -> Option<Pubkey> {
    let der = pem_body(pem, "PUBLIC KEY")?;
    if der.len() != 44 || der[..12] != ED25519_SPKI_PREFIX {
        return None;
    }
    Some(Pubkey::new_from_array(der[12..].try_into().ok()?))
}

/// Canonical base64 of exactly 64 bytes.
fn decode_signature(text: &str) -> Option<[u8; 64]> {
    let bytes = B64.decode(text).ok()?;
    let signature: [u8; 64] = bytes.as_slice().try_into().ok()?;
    (B64.encode(signature) == text).then_some(signature)
}

/// One Cloud KMS Ed25519 key version, checked to be the configured address's
/// key (the API's sponsor, an ops tool's role).
pub struct KmsSigner {
    http: reqwest::Client,
    api: String,
    credentials: Credentials,
    key_version: String,
    pubkey: Pubkey,
}

impl KmsSigner {
    /// The key version `key_version` at `api` (Google's [`KMS_API`] except
    /// against a stub), reached with `credentials`. Its public key must be
    /// `expected`, its algorithm `EC_SIGN_ED25519` and its protection level
    /// software or HSM.
    pub async fn connect(
        key_version: &str,
        expected: Pubkey,
        credentials: Credentials,
        api: &str,
    ) -> Result<Self> {
        if !valid_key_version(key_version) {
            bail!("the KMS key version must be one exact Cloud KMS CryptoKeyVersion resource");
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| anyhow!("the KMS HTTP client could not start: {e}"))?;
        let signer = Self {
            http,
            api: api.trim_end_matches('/').to_owned(),
            credentials,
            key_version: key_version.to_owned(),
            pubkey: expected,
        };
        signer.check_public_key().await?;
        Ok(signer)
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    pub fn key_version(&self) -> &str {
        &self.key_version
    }

    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    async fn check_public_key(&self) -> Result<()> {
        let token = self.credentials.token(&self.http).await?;
        let answer = json_answer(
            self.http
                .get(format!("{}/{}/publicKey", self.api, self.key_version))
                .bearer_auth(&token),
            "KMS getPublicKey",
        )
        .await?;
        if answer["name"] != self.key_version.as_str() {
            bail!("KMS getPublicKey answered for another key version");
        }
        if answer["algorithm"] != "EC_SIGN_ED25519" {
            bail!(
                "KMS key {} is not an Ed25519 signing key (algorithm {})",
                self.key_version,
                answer["algorithm"].as_str().unwrap_or("unknown")
            );
        }
        if !matches!(answer["protectionLevel"].as_str(), Some("SOFTWARE" | "HSM")) {
            bail!(
                "KMS key {} is neither a software nor an HSM key",
                self.key_version
            );
        }
        let key = answer["pem"]
            .as_str()
            .and_then(ed25519_from_pem)
            .ok_or_else(|| {
                anyhow!(
                    "KMS key {}'s public key is not exact Ed25519 SPKI",
                    self.key_version
                )
            })?;
        if key != self.pubkey {
            bail!(
                "KMS key {} is {key}, not the configured address {}: refusing to sign with it",
                self.key_version,
                self.pubkey
            );
        }
        Ok(())
    }

    /// The key's Ed25519 signature over exactly `message`, verified against
    /// the configured address.
    pub async fn sign(&self, message: &[u8]) -> Result<Signature> {
        let token = self.credentials.token(&self.http).await?;
        let answer: Value = json_answer(
            self.http
                .post(format!("{}/{}:asymmetricSign", self.api, self.key_version))
                .bearer_auth(&token)
                .json(&json!({ "data": B64.encode(message) })),
            "KMS asymmetricSign",
        )
        .await?;
        if answer["name"] != self.key_version.as_str() {
            bail!("KMS signed with another key version");
        }
        let signature = answer["signature"]
            .as_str()
            .and_then(decode_signature)
            .map(Signature::from)
            .ok_or_else(|| {
                anyhow!("KMS returned something other than one 64-byte Ed25519 signature")
            })?;
        if !signature.verify(self.pubkey.as_ref(), message) {
            bail!(
                "KMS returned a signature that does not verify against the configured address {}",
                self.pubkey
            );
        }
        Ok(signature)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Against a local stub of the metadata server, Google's token endpoint
    //! and the KMS API, holding throwaway keys. Nothing here reaches Google.
    use super::*;
    use crate::{google::Credentials, RoleSigner};
    use axum::{
        body::Bytes,
        extract::State,
        http::{HeaderMap, Method, StatusCode, Uri},
        response::{IntoResponse, Response},
        Json, Router,
    };
    use solana_keypair::Keypair;
    use solana_signer::Signer;
    use solana_transaction::Transaction;
    use std::sync::{Arc, Mutex};

    pub(crate) const VERSION: &str =
        "projects/amoeba-test1/locations/global/keyRings/ring/cryptoKeys/sponsor/cryptoKeyVersions/1";
    const TOKEN: &str = "stub-token";

    /// How the stub misbehaves, if at all.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Fault {
        None,
        /// `getPublicKey` names another key (not the sponsor).
        OtherPublicKey,
        /// `asymmetricSign` signs with another key.
        ForeignSignature,
        /// The key is a P-256 key.
        WrongAlgorithm,
        /// The caller may not use the key.
        Denied,
    }

    pub(crate) struct Stub {
        pub(crate) key: Keypair,
        other: Keypair,
        fault: Fault,
        /// Every request, as `METHOD path`.
        pub(crate) seen: Mutex<Vec<String>>,
        /// The token requests' form bodies.
        forms: Mutex<Vec<String>>,
        /// A service account key's public half (PKCS #1 DER), to check its JWTs.
        rsa_public: Option<Vec<u8>>,
    }

    fn pem(key: &Pubkey) -> String {
        let mut der = ED25519_SPKI_PREFIX.to_vec();
        der.extend_from_slice(key.as_ref());
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            B64.encode(der)
        )
    }

    async fn answer(
        State(stub): State<Arc<Stub>>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let path = uri.path().to_owned();
        stub.seen.lock().unwrap().push(format!("{method} {path}"));
        let bearer = headers
            .get("authorization")
            .is_some_and(|v| v == &format!("Bearer {TOKEN}"));
        let metadata = headers
            .get("metadata-flavor")
            .is_some_and(|v| v == "Google");
        let token = || {
            Json(json!({ "access_token": TOKEN, "expires_in": 3599, "token_type": "Bearer" }))
                .into_response()
        };
        let refuse = |status: StatusCode, message: &str| {
            (status, Json(json!({ "error": { "code": status.as_u16(), "message": message, "status": "DENIED" } })))
                .into_response()
        };
        let version_path = format!("/kms/{VERSION}");
        match path.as_str() {
            "/metadata/computeMetadata/v1/instance/service-accounts/default/token" if metadata => {
                token()
            }
            "/token" if method == Method::POST => {
                let form = String::from_utf8_lossy(&body).into_owned();
                stub.forms.lock().unwrap().push(form.clone());
                let fields: std::collections::HashMap<String, String> =
                    form_urlencoded::parse(form.as_bytes())
                        .into_owned()
                        .collect();
                let good = match fields.get("grant_type").map(String::as_str) {
                    Some("refresh_token") => fields
                        .get("refresh_token")
                        .is_some_and(|t| t == "stub-refresh"),
                    Some("urn:ietf:params:oauth:grant-type:jwt-bearer") => fields
                        .get("assertion")
                        .is_some_and(|jwt| jwt_verifies(&stub, jwt)),
                    _ => false,
                };
                if good {
                    token()
                } else {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "invalid_grant", "error_description": "bad" })),
                    )
                        .into_response()
                }
            }
            _ if !bearer => refuse(StatusCode::UNAUTHORIZED, "no token"),
            _ if stub.fault == Fault::Denied => refuse(
                StatusCode::FORBIDDEN,
                "Permission 'cloudkms.cryptoKeyVersions.useToSign' denied",
            ),
            p if p == format!("{version_path}/publicKey") && method == Method::GET => {
                let key = if stub.fault == Fault::OtherPublicKey {
                    stub.other.pubkey()
                } else {
                    stub.key.pubkey()
                };
                let algorithm = if stub.fault == Fault::WrongAlgorithm {
                    "EC_SIGN_P256_SHA256"
                } else {
                    "EC_SIGN_ED25519"
                };
                Json(json!({ "name": VERSION, "algorithm": algorithm, "protectionLevel": "HSM", "pem": pem(&key) }))
                    .into_response()
            }
            p if p == format!("{version_path}:asymmetricSign") && method == Method::POST => {
                let request: Value = serde_json::from_slice(&body).unwrap_or_default();
                let Some(data) = request["data"].as_str().and_then(|d| B64.decode(d).ok()) else {
                    return refuse(StatusCode::BAD_REQUEST, "no data");
                };
                let key = if stub.fault == Fault::ForeignSignature {
                    &stub.other
                } else {
                    &stub.key
                };
                let signature = key.sign_message(&data);
                Json(json!({ "name": VERSION, "signature": B64.encode(signature.as_ref()) }))
                    .into_response()
            }
            _ => refuse(StatusCode::NOT_FOUND, "unknown"),
        }
    }

    /// Whether `jwt` is the service account's RS256 assertion for the stub.
    fn jwt_verifies(stub: &Stub, jwt: &str) -> bool {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let Some(public) = &stub.rsa_public else {
            return false;
        };
        let Some((unsigned, signature)) = jwt.rsplit_once('.') else {
            return false;
        };
        let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else {
            return false;
        };
        let verified = ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            public,
        )
        .verify(unsigned.as_bytes(), &signature)
        .is_ok();
        let claims: Value = unsigned
            .split('.')
            .nth(1)
            .and_then(|c| URL_SAFE_NO_PAD.decode(c).ok())
            .and_then(|c| serde_json::from_slice(&c).ok())
            .unwrap_or_default();
        verified
            && claims["iss"] == "ameba-api@amoeba-test1.iam.gserviceaccount.com"
            && claims["scope"] == crate::google::CLOUD_PLATFORM_SCOPE
            && claims["aud"]
                .as_str()
                .is_some_and(|a| a.ends_with("/token"))
    }

    /// The stub on a local port: its base URL and its state.
    pub(crate) async fn serve(fault: Fault, rsa_public: Option<Vec<u8>>) -> (String, Arc<Stub>) {
        let stub = Arc::new(Stub {
            key: Keypair::new(),
            other: Keypair::new(),
            fault,
            seen: Mutex::default(),
            forms: Mutex::default(),
            rsa_public,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().fallback(answer).with_state(stub.clone());
        tokio::spawn(async move { axum::serve(listener, router).await });
        (base, stub)
    }

    pub(crate) fn metadata(base: &str) -> Credentials {
        Credentials::metadata(&format!("{base}/metadata/computeMetadata/v1"))
    }

    async fn connect(base: &str, expected: Pubkey) -> Result<KmsSigner> {
        KmsSigner::connect(VERSION, expected, metadata(base), &format!("{base}/kms")).await
    }

    #[tokio::test]
    async fn kms_signs_the_exact_message_and_the_signature_verifies() {
        let (base, stub) = serve(Fault::None, None).await;
        let sponsor = stub.key.pubkey();
        let kms = connect(&base, sponsor)
            .await
            .expect("the key is the sponsor's");
        let signature = kms.sign(b"message").await.unwrap();
        assert!(signature.verify(sponsor.as_ref(), b"message"));
        assert_eq!(
            signature,
            stub.key.sign_message(b"message"),
            "Ed25519 is deterministic"
        );
        // One token for everything; one request per signature after the start.
        let seen = stub.seen.lock().unwrap().clone();
        assert_eq!(
            seen.iter().filter(|r| r.ends_with("/token")).count(),
            1,
            "{seen:?}"
        );
        assert_eq!(
            seen.iter()
                .filter(|r| r.ends_with(":asymmetricSign"))
                .count(),
            1
        );

        // As the sponsor of a sponsored transaction: only its slot is filled,
        // over the unchanged message, and the whole transaction verifies.
        let signer = RoleSigner::Kms(kms);
        let owner = Keypair::new();
        let ix = solana_program::instruction::Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[7],
            vec![solana_program::instruction::AccountMeta::new(
                owner.pubkey(),
                true,
            )],
        );
        let mut tx = Transaction::new_unsigned(solana_message::Message::new(&[ix], Some(&sponsor)));
        let before = tx.message_data();
        tx.partial_sign(&[&owner], tx.message.recent_blockhash);
        signer.sign_transaction(&mut tx).await.unwrap();
        assert_eq!(tx.message_data(), before);
        assert!(
            crate::tests::all_signatures_verify(&tx),
            "every signature verifies"
        );
        assert_eq!(signer.pubkey(), sponsor);
    }

    #[tokio::test]
    async fn kms_refuses_a_key_that_is_not_the_sponsor() {
        // The configured address is not the key's.
        let (base, _) = serve(Fault::None, None).await;
        let error = connect(&base, Pubkey::new_unique())
            .await
            .err()
            .expect("refused");
        assert!(
            error.to_string().contains("refusing to sign with it"),
            "{error:#}"
        );
        // KMS names another public key.
        let (base, stub) = serve(Fault::OtherPublicKey, None).await;
        let error = connect(&base, stub.key.pubkey())
            .await
            .err()
            .expect("refused");
        assert!(
            error.to_string().contains("refusing to sign with it"),
            "{error:#}"
        );
        // The key is not Ed25519.
        let (base, stub) = serve(Fault::WrongAlgorithm, None).await;
        let error = connect(&base, stub.key.pubkey())
            .await
            .err()
            .expect("refused");
        assert!(
            error.to_string().contains("not an Ed25519 signing key"),
            "{error:#}"
        );
        // The service account may not use the key: Google's reason is kept.
        let (base, stub) = serve(Fault::Denied, None).await;
        let error = connect(&base, stub.key.pubkey())
            .await
            .err()
            .expect("refused");
        assert!(
            error.to_string().contains("HTTP 403") && error.to_string().contains("useToSign"),
            "{error:#}"
        );
        // No signature request was ever made for a refused key.
        assert!(!stub
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.ends_with(":asymmetricSign")));
    }

    #[tokio::test]
    async fn kms_refuses_a_signature_that_does_not_verify() {
        let (base, stub) = serve(Fault::ForeignSignature, None).await;
        let kms = connect(&base, stub.key.pubkey())
            .await
            .expect("the key itself is right");
        let error = kms.sign(b"message").await.expect_err("refused");
        assert!(error.to_string().contains("does not verify"), "{error:#}");
        // The sponsor signer never puts it into a transaction either.
        let signer = RoleSigner::Kms(kms);
        let mut tx =
            Transaction::new_unsigned(solana_message::Message::new(&[], Some(&stub.key.pubkey())));
        assert!(signer.sign_transaction(&mut tx).await.is_err());
        assert_eq!(tx.signatures[0], Signature::default());
    }

    #[tokio::test]
    async fn application_default_credentials_files() {
        // gcloud user credentials: a refresh token.
        let (base, stub) = serve(Fault::None, None).await;
        let user = Credentials::from_json(
            &json!({ "type": "authorized_user", "client_id": "id", "client_secret": "secret",
                     "refresh_token": "stub-refresh", "token_uri": format!("{base}/token") })
            .to_string(),
        )
        .unwrap();
        assert_eq!(user.to_string(), "gcloud user credentials");
        let kms = KmsSigner::connect(VERSION, stub.key.pubkey(), user, &format!("{base}/kms"))
            .await
            .unwrap();
        assert!(kms
            .sign(b"m")
            .await
            .unwrap()
            .verify(stub.key.pubkey().as_ref(), b"m"));

        // A service account key: an RS256 JWT, which the stub checks.
        use rsa::pkcs1::EncodeRsaPublicKey;
        use rsa::pkcs8::EncodePrivateKey;
        let private = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let pkcs8 = private.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let public = private
            .to_public_key()
            .to_pkcs1_der()
            .unwrap()
            .as_bytes()
            .to_vec();
        let (base, stub) = serve(Fault::None, Some(public)).await;
        let account = Credentials::from_json(
            &json!({ "type": "service_account", "client_email": "ameba-api@amoeba-test1.iam.gserviceaccount.com",
                     "private_key_id": "k1", "private_key": pkcs8.as_str(), "token_uri": format!("{base}/token") })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            account.to_string(),
            "service account ameba-api@amoeba-test1.iam.gserviceaccount.com"
        );
        let kms = KmsSigner::connect(VERSION, stub.key.pubkey(), account, &format!("{base}/kms"))
            .await
            .unwrap();
        assert!(kms
            .sign(b"m")
            .await
            .unwrap()
            .verify(stub.key.pubkey().as_ref(), b"m"));
        assert!(stub.forms.lock().unwrap()[0].contains("jwt-bearer"));

        // Other kinds are refused, and errors never carry a credential.
        for bad in [
            json!({ "type": "external_account", "audience": "x" }),
            json!({ "type": "authorized_user", "client_id": "id", "client_secret": "s3cr3t" }),
            json!({ "type": "service_account", "client_email": "a@b", "private_key": "s3cr3t" }),
            json!({ "private_key": "s3cr3t" }),
        ] {
            let error = Credentials::from_json(&bad.to_string())
                .err()
                .expect("refused");
            assert!(!format!("{error:#}").contains("s3cr3t"), "{error:#}");
        }
        let error = Credentials::from_json("{ \"type\": \"authorized_user\", s3cr3t")
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("s3cr3t"));
    }

    #[test]
    fn adc_looks_at_the_explicit_file_then_gclouds_then_the_metadata_server() {
        let dir = std::env::temp_dir().join(format!("adc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let user = dir.join("user.json");
        std::fs::write(
            &user,
            json!({ "type": "authorized_user", "client_id": "i", "client_secret": "s", "refresh_token": "r" })
                .to_string(),
        )
        .unwrap();
        let missing = dir.join("missing.json");
        let pick = |explicit: Option<&std::path::Path>, well_known: Option<&std::path::Path>| {
            Credentials::from_sources(
                explicit.map(Into::into),
                well_known.map(Into::into),
                Some("127.0.0.1:9".into()),
            )
            .map(|c| c.to_string())
        };
        assert_eq!(pick(Some(&user), None).unwrap(), "gcloud user credentials");
        assert!(
            pick(Some(&missing), Some(&user)).is_err(),
            "an explicit file must be readable"
        );
        assert_eq!(pick(None, Some(&user)).unwrap(), "gcloud user credentials");
        assert_eq!(
            pick(None, Some(&missing)).unwrap(),
            "metadata server credentials"
        );
        assert_eq!(pick(None, None).unwrap(), "metadata server credentials");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_versions_and_public_keys_are_exact() {
        assert!(valid_key_version(VERSION));
        assert!(valid_key_version(
            "projects/amoeba-hm0q2k/locations/us-central1/keyRings/futures/cryptoKeys/sponsor/cryptoKeyVersions/12"
        ));
        for bad in [
            "",
            "projects/x/keys/1",
            "projects/amoeba-test1/locations/global/keyRings/ring/cryptoKeys/sponsor",
            "projects/amoeba-test1/locations/global/keyRings/ring/cryptoKeys/sponsor/cryptoKeyVersions/01",
            "projects/amoeba-test1/locations/global/keyRings/ring/cryptoKeys/sponsor/cryptoKeyVersions/1/x",
            "projects/A/locations/global/keyRings/ring/cryptoKeys/sponsor/cryptoKeyVersions/1",
        ] {
            assert!(!valid_key_version(bad), "{bad}");
        }
        let key = Pubkey::new_unique();
        assert_eq!(ed25519_from_pem(&pem(&key)), Some(key));
        assert_eq!(
            ed25519_from_pem("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----"),
            None
        );
        assert_eq!(decode_signature(&B64.encode([1u8; 64])), Some([1u8; 64]));
        assert_eq!(decode_signature(&B64.encode([1u8; 63])), None);
    }
}
