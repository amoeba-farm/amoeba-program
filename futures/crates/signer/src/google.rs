//! Google access tokens for Cloud KMS, from Application Default Credentials
//! in Google's order:
//!
//! 1. the JSON file `GOOGLE_APPLICATION_CREDENTIALS` names;
//! 2. gcloud's `application_default_credentials.json` (`gcloud auth
//!    application-default login`), in `$CLOUDSDK_CONFIG` or
//!    `~/.config/gcloud`;
//! 3. the metadata server of the Google Cloud host the service runs on
//!    (Compute Engine, Cloud Run, GKE), as the old Spread backend's KMS
//!    custody did (`api/crates/signer/src/kms.rs`, removed in 49801a67).
//!    `GCE_METADATA_HOST` moves it, as in Google's own libraries.
//!
//! A credentials file holds a service account key (`service_account`, a
//! signed JWT exchanged for a token) or gcloud user credentials
//! (`authorized_user`, a refresh token). Other kinds (workload identity
//! federation, impersonation) are refused at start. Nothing here is ever
//! configured in the repository or the deployment manifest, and neither
//! credentials nor tokens are logged or put in an error.
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// The scope Cloud KMS accepts.
pub const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const METADATA_BASE: &str = "http://metadata.google.internal/computeMetadata/v1";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
/// A token is replaced this long before Google says it expires, so a
/// signature never goes out with one that lapses on the way.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
/// What a service account's signed JWT asks for (Google's maximum).
const ASSERTION_LIFETIME_SECS: u64 = 3600;

enum Source {
    /// The host's own service account.
    Metadata { base: String },
    /// gcloud user credentials.
    AuthorizedUser {
        client_id: String,
        client_secret: String,
        refresh_token: String,
        token_uri: String,
    },
    /// A service account key.
    ServiceAccount {
        client_email: String,
        key_id: Option<String>,
        key: Box<ring::signature::RsaKeyPair>,
        token_uri: String,
    },
}

struct Token {
    value: String,
    refresh_at: Instant,
}

/// One source of access tokens, cached until shortly before they expire.
pub struct Credentials {
    source: Source,
    token: Mutex<Option<Token>>,
}

impl std::fmt::Display for Credentials {
    /// Which credentials these are (never their contents).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.source {
            Source::Metadata { .. } => f.write_str("metadata server credentials"),
            Source::AuthorizedUser { .. } => f.write_str("gcloud user credentials"),
            Source::ServiceAccount { client_email, .. } => {
                write!(f, "service account {client_email}")
            }
        }
    }
}

impl Credentials {
    /// Application Default Credentials, from this process's environment.
    pub fn from_environment() -> Result<Self> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let well_known = var("CLOUDSDK_CONFIG")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config/gcloud")))
            .map(|dir| dir.join("application_default_credentials.json"));
        Self::from_sources(
            var("GOOGLE_APPLICATION_CREDENTIALS").map(PathBuf::from),
            well_known,
            var("GCE_METADATA_HOST").and_then(|h| h.into_string().ok()),
        )
    }

    /// ADC's order over its three sources: `explicit` (which must be
    /// readable), else `well_known` when it exists, else the metadata server
    /// (at `metadata_host` when given).
    pub fn from_sources(
        explicit: Option<PathBuf>,
        well_known: Option<PathBuf>,
        metadata_host: Option<String>,
    ) -> Result<Self> {
        let read = |path: &Path| -> Result<Self> {
            let text = std::fs::read_to_string(path).map_err(|e| {
                anyhow!(
                    "cannot read the Google credentials file {}: {e}",
                    path.display()
                )
            })?;
            Self::from_json(&text)
                .with_context(|| format!("Google credentials file {}", path.display()))
        };
        if let Some(path) = explicit {
            return read(&path);
        }
        if let Some(path) = well_known.filter(|p| p.is_file()) {
            return read(&path);
        }
        Ok(Self::metadata(&match metadata_host {
            Some(host) => format!("http://{host}/computeMetadata/v1"),
            None => METADATA_BASE.to_owned(),
        }))
    }

    /// The metadata server at `base` (`…/computeMetadata/v1`).
    pub fn metadata(base: &str) -> Self {
        Self::new(Source::Metadata {
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    /// A credentials file's contents. Errors name the field, never a value.
    pub fn from_json(text: &str) -> Result<Self> {
        let v: Value = serde_json::from_str(text).map_err(|_| anyhow!("not JSON"))?;
        let field = |name: &str| -> Result<String> {
            v[name]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("`{name}` is missing"))
        };
        let token_uri = v["token_uri"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(TOKEN_URI)
            .to_owned();
        let source = match v["type"].as_str() {
            Some("service_account") => {
                let der = pem_body(&field("private_key")?, "PRIVATE KEY")
                    .ok_or_else(|| anyhow!("`private_key` is not a PKCS #8 PEM key"))?;
                let key = ring::signature::RsaKeyPair::from_pkcs8(&der)
                    .map_err(|_| anyhow!("`private_key` is not an RSA key ring accepts"))?;
                Source::ServiceAccount {
                    client_email: field("client_email")?,
                    key_id: v["private_key_id"].as_str().map(str::to_owned),
                    key: Box::new(key),
                    token_uri,
                }
            }
            Some("authorized_user") => Source::AuthorizedUser {
                client_id: field("client_id")?,
                client_secret: field("client_secret")?,
                refresh_token: field("refresh_token")?,
                token_uri,
            },
            Some(other) => bail!(
                "credentials of type `{other}` are not supported: run on a Google Cloud host (the metadata \
                 server), or use a service account key or gcloud user credentials"
            ),
            None => bail!("`type` is missing"),
        };
        Ok(Self::new(source))
    }

    fn new(source: Source) -> Self {
        Self {
            source,
            token: Mutex::new(None),
        }
    }

    /// A current access token for the cloud-platform scope: the cached one,
    /// or a new one (one request; none is retried).
    pub async fn token(&self, http: &reqwest::Client) -> Result<String> {
        let mut cached = self.token.lock().await;
        if let Some(t) = cached.as_ref().filter(|t| Instant::now() < t.refresh_at) {
            return Ok(t.value.clone());
        }
        let answer = match &self.source {
            Source::Metadata { base } => {
                json_answer(
                    http.get(format!("{base}/instance/service-accounts/default/token"))
                        .header("Metadata-Flavor", "Google"),
                    "the metadata server's token (no GOOGLE_APPLICATION_CREDENTIALS or gcloud credentials \
                     were found, so this must run on a Google Cloud host)",
                )
                .await?
            }
            Source::AuthorizedUser { client_id, client_secret, refresh_token, token_uri } => {
                json_answer(
                    http.post(token_uri).form(&[
                        ("grant_type", "refresh_token"),
                        ("client_id", client_id.as_str()),
                        ("client_secret", client_secret.as_str()),
                        ("refresh_token", refresh_token.as_str()),
                    ]),
                    "the gcloud user token",
                )
                .await?
            }
            Source::ServiceAccount { client_email, key_id, key, token_uri } => {
                let assertion = assertion(client_email, key_id.as_deref(), key, token_uri)?;
                json_answer(
                    http.post(token_uri).form(&[
                        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                        ("assertion", assertion.as_str()),
                    ]),
                    "the service account token",
                )
                .await?
            }
        };
        let value = answer["access_token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| anyhow!("{self}: the token response has no access_token"))?
            .to_owned();
        let lifetime = Duration::from_secs(answer["expires_in"].as_u64().unwrap_or(0));
        *cached = Some(Token {
            value: value.clone(),
            refresh_at: Instant::now() + lifetime.saturating_sub(EXPIRY_MARGIN),
        });
        Ok(value)
    }
}

/// A service account's signed JWT (RS256) asking `token_uri` for a
/// cloud-platform token.
fn assertion(
    client_email: &str,
    key_id: Option<&str>,
    key: &ring::signature::RsaKeyPair,
    token_uri: &str,
) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut header = json!({ "alg": "RS256", "typ": "JWT" });
    if let Some(id) = key_id {
        header["kid"] = json!(id);
    }
    let claims = json!({
        "iss": client_email,
        "scope": CLOUD_PLATFORM_SCOPE,
        "aud": token_uri,
        "iat": now,
        "exp": now + ASSERTION_LIFETIME_SECS,
    });
    let unsigned = format!(
        "{}.{}",
        B64URL.encode(header.to_string()),
        B64URL.encode(claims.to_string())
    );
    let mut signature = vec![0u8; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        unsigned.as_bytes(),
        &mut signature,
    )
    .map_err(|_| anyhow!("signing the service account's JWT failed"))?;
    Ok(format!("{unsigned}.{}", B64URL.encode(signature)))
}

/// The DER inside a PEM block labelled `label`.
pub(crate) fn pem_body(pem: &str, label: &str) -> Option<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let inner = pem.trim().strip_prefix(&begin)?.strip_suffix(&end)?;
    let body: String = inner.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD.decode(body).ok()
}

/// The JSON answer to `request`; a failure says what was asked, the HTTP
/// status and Google's own error message, never a credential.
pub(crate) async fn json_answer(request: reqwest::RequestBuilder, label: &str) -> Result<Value> {
    let response = request
        .send()
        .await
        .map_err(|e| anyhow!("{label}: request failed: {}", e.without_url()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| anyhow!("{label}: reading the answer failed: {}", e.without_url()))?;
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = match &value["error"] {
            Value::Object(e) => e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            Value::String(code) => {
                let description = value["error_description"].as_str().unwrap_or_default();
                format!("{code} {description}")
            }
            _ => String::new(),
        };
        let detail: String = detail
            .trim()
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(300)
            .collect();
        bail!(
            "{label}: HTTP {}{}{detail}",
            status.as_u16(),
            if detail.is_empty() { "" } else { ": " }
        );
    }
    if value.is_null() {
        bail!("{label}: the answer is not JSON");
    }
    Ok(value)
}
