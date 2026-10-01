//! Configuration and the secrets that authenticate the relay.
//!
//! # Precedence
//!
//! [`Config::resolve`] takes an explicit override first, then the environment,
//! then the built-in default. The desktop passes its own settings as the
//! explicit override so the app's Settings screen is the source of truth; the
//! environment stays meaningful for headless and container use, and for
//! overriding a single value in a test.
//!
//! # Secrets
//!
//! The master HMAC secret is loaded from memory or from a file, and is
//! **never** used to verify a `relay_route`. It only seeds the domain-separated
//! signing key and backs the `/health` token default. Keeping those separate is
//! the relay’s core security property — see ADR-0004.

use std::path::PathBuf;

use log::info;

/// The effective configuration a relay runs with.
///
/// Built by [`Config::resolve`], which is the only constructor: there is no way
/// to build one field-by-field and end up with a half-configured relay.
#[derive(Debug)]
pub struct Config {
    /// Port for the plaintext WebSocket listener. Off unless
    /// [`Config::enable_plain_ws`] is set — it carries the bearer token in the
    /// clear.
    pub ws_port: u16,
    /// Address the plaintext WebSocket listener binds.
    ///
    /// Loopback by default, and that default is the point. A plaintext listener
    /// that is reachable from the network hands the bearer token to anyone who
    /// asks for it, so the only safe default is "not reachable from the
    /// network". Binding it to loopback is still useful — it is how a host that
    /// *is* the relay joins its own routing table without a certificate check.
    pub ws_bind: std::net::IpAddr,
    /// Port for the TLS WebSocket listener. The only port a client should dial.
    pub wss_port: u16,
    /// Port for the health/metrics/pin HTTP surface.
    pub health_port: u16,
    /// Address the health/metrics/pin HTTP surface binds.
    ///
    /// Loopback by default, and that default is the point. Those endpoints
    /// publish the peer count, the tracked-address count, the payload-size
    /// distribution and the certificate pin; none of that belongs on every
    /// interface of what is usually a laptop. A host that genuinely wants to
    /// scrape from elsewhere binds it explicitly.
    pub health_bind: std::net::IpAddr,
    /// Bearer token required for `GET /health` and `GET /`.
    /// Defaults to the UTF-8 HMAC secret unless `RELAY_HEALTH_TOKEN` is set.
    pub(crate) health_token: String,
    /// Optional bearer token guarding `GET /metrics`.
    ///
    /// `/metrics` is unauthenticated by default (a deliberate, tested decision
    /// so Prometheus can scrape without distributing the health secret). Setting
    /// `RELAY_METRICS_TOKEN` opts into gating it.
    pub metrics_token: Option<String>,
    /// Resolved HMAC master secret: `HMAC_SECRET` env var, else the contents of
    /// `HMAC_SECRET_FILE`, else a freshly generated + persisted 64-char hex
    /// secret (see [`load_or_create_hmac_secret`]).
    ///
    /// `hmac_secret` and `health_token` are **intentionally equal** whenever
    /// `RELAY_HEALTH_TOKEN` is unset/empty — that fallback is the whole reason
    /// both are populated from the same value in `from_env`. They diverge by
    /// design only when `RELAY_HEALTH_TOKEN` is explicitly set, in which case
    /// `health_token` is the override and this field is left untouched.
    ///
    /// The master secret is **never** used directly to verify `relay_route`
    /// HMACs and is never exposed to clients; it only (a) seeds the derived
    /// signing key when `RELAY_SIGNING_KEY` is unset and (b) backs the
    /// `/health` token default.
    pub(crate) hmac_secret: Vec<u8>,
    /// Serve the plaintext WebSocket listener. Off by default: it puts the
    /// bearer token on the wire in the clear.
    pub enable_plain_ws: bool,
    /// Where the HMAC secret was read from, or bootstrapped to.
    ///
    /// Public because a host has to be able to assert that it pinned this to a
    /// path it controls, rather than leaving it relative to the process working
    /// directory. See [`Overrides::hmac_secret_file`].
    pub hmac_secret_file: PathBuf,
    /// Where the replay cache is persisted. The file is written on a timer and
    /// again on shutdown, so a crash loses at most one interval of nonces.
    pub nonce_file: std::path::PathBuf,
    /// Seconds a connection may stay open without authenticating.
    /// Kept in Config so integration tests can shrink it (default 10).
    pub auth_timeout_secs: u64,
    /// Shared bearer credential presented as `relay_token` during `relay_auth`.
    /// Authentication only — never a signing key.
    pub relay_token: String,
    /// How TLS should be presented: where the certificate lives, and what names
    /// it must be valid for.
    ///
    /// Held here rather than re-read from the environment at load time so that a
    /// host embedding the relay controls it from its own settings, and so the
    /// certificate a relay presents is a function of its configuration rather
    /// than of process-global state.
    pub tls: crate::tls::TlsParams,
    /// SPKI pin the served certificate must present, as `sha256/<base64>`.
    ///
    /// `None` enforces nothing, which is right when the host *is* the server and
    /// is not authenticating anyone. When set, a mismatch refuses startup: a
    /// certificate change is either a renewal the operator knows about or
    /// something that should not be served, and only failing tells them apart.
    pub relay_cert_pin: Option<String>,
}

/// Resolve the HMAC secret: env var → secret file → generate + persist.
///
/// Fail-closed: if the secret cannot be read or persisted, startup aborts.
/// An ephemeral secret would silently invalidate every deployed token on
/// restart — exactly the bug this path exists to fix.
///
/// Takes the path so a host can pin it. The environment default is *relative* —
/// `./secrets/hmac_secret` — so a host that does not pin it writes the file into
/// whatever directory the process happened to be launched from, which for a
/// desktop app is a path chosen by the OS and not writable at all on a packaged
/// install. See [`Overrides::hmac_secret_file`].
pub(crate) fn load_or_create_hmac_secret_from(path: Option<PathBuf>) -> Result<Vec<u8>, String> {
    let path = path
        .or_else(|| std::env::var("HMAC_SECRET_FILE").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("./secrets/hmac_secret"));

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            // Tolerate a trailing newline from `echo`/secret managers.
            let secret = content.trim_end_matches(['\r', '\n']);
            if secret.is_empty() {
                return Err(format!("HMAC secret file {} is empty", path.display()));
            }
            info!("Loaded HMAC secret from {}", path.display());
            Ok(secret.as_bytes().to_vec())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Bootstrap: 32 random bytes as 64-char hex, persisted with 0600.
            let mut bytes = [0u8; 32];
            use rand::Rng;
            rand::thread_rng().fill(&mut bytes[..]);
            let secret = hex::encode(bytes);
            write_secret_file(&path, &secret)?;
            info!(
                "Generated new HMAC secret and persisted to {}",
                path.display()
            );
            Ok(secret.into_bytes())
        }
        Err(e) => Err(format!(
            "Failed to read HMAC secret file {}: {}",
            path.display(),
            e
        )),
    }
}

/// Write a secret file with owner-only permissions where the OS supports it.
#[cfg(unix)]
pub(crate) fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    // `.mode()` only applies at creation and is masked by umask — enforce 0600.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Failed to chmod {}: {}", path.display(), e))?;
    Ok(())
}

/// Non-unix: `Permissions::from_mode` is unavailable; rely on filesystem ACLs.
#[cfg(not(unix))]
pub(crate) fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

/// Settings the host already holds, layered over the environment.
///
/// The desktop fills this from its own settings database and passes it to
/// [`Config::resolve`]. A field left as `None` falls through to the
/// environment, then to the built-in default — so an operator can still pin one
/// value from the environment without restating the other eighteen.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    /// Bearer token clients present during `relay_auth`.
    pub relay_token: Option<String>,
    /// Port for the TLS WebSocket listener.
    pub wss_port: Option<u16>,
    /// Port for the plaintext WebSocket listener.
    pub ws_port: Option<u16>,
    /// Address for the plaintext WebSocket listener. Loopback by default.
    pub ws_bind: Option<std::net::IpAddr>,
    /// Port for the health/metrics HTTP surface.
    pub health_port: Option<u16>,
    /// Address for the health/metrics/pin surface. Loopback by default.
    pub health_bind: Option<std::net::IpAddr>,
    /// Where the replay cache is persisted.
    pub nonce_file: Option<PathBuf>,
    /// Bearer token guarding `/health` and `/`.
    pub health_token: Option<String>,
    /// Bearer token guarding `/metrics`. `None` leaves the endpoint open.
    pub metrics_token: Option<String>,
    /// Serve TLS on this port, or generate a self-signed certificate for it.
    pub tls_hostname: Option<String>,
    /// SPKI pin the served certificate must present, as `sha256/<base64>`.
    ///
    /// `None` — the default — means no pin is enforced, which is right for a
    /// relay hosting its own connections. Setting it makes a mismatch a refusal
    /// to start rather than a log line, so a certificate that changed without
    /// being intended to cannot be served.
    pub relay_cert_pin: Option<String>,
    /// Extra subject alternative names for a generated certificate.
    pub tls_extra_sans: Option<Vec<String>>,
    /// Directory holding (or receiving) `cert.pem` and `key.pem`.
    pub tls_cert_dir: Option<PathBuf>,
    /// Where the HMAC secret is read from, or bootstrapped to.
    ///
    /// A host that embeds the library should set this. The built-in default is
    /// relative to the process working directory, which is not a location any
    /// host controls — see [`load_or_create_hmac_secret_from`].
    pub hmac_secret_file: Option<PathBuf>,
    /// Enable the plaintext listener.
    pub enable_plain_ws: Option<bool>,
    /// Seconds a socket may stay open without authenticating.
    pub auth_timeout_secs: Option<u64>,
}

impl Config {
    /// Resolve the effective configuration.
    ///
    /// Precedence, highest first: `overrides`, then the environment, then the
    /// built-in default.
    ///
    /// Fails only on the things that make a relay meaningless: no bearer token,
    /// and an unreadable or unwritable master secret. Everything else falls
    /// back, because a relay that starts with a default port and no
    /// certificate is still better than one that refuses to run.
    ///
    /// The error is a `String` rather than a typed error because it is surfaced
    /// to a log and to a status field, never matched on.
    pub fn resolve(overrides: Option<Overrides>) -> Result<Self, String> {
        let o = overrides.unwrap_or_default();

        let relay_token = o
            .relay_token
            .filter(|t| !t.is_empty())
            .or_else(|| std::env::var("RELAY_TOKEN").ok())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                "no relay token: set one in Settings, or set RELAY_TOKEN (fail-closed)".to_string()
            })?;

        let ws_port = o
            .ws_port
            .or_else(|| env_port("RELAY_WS_PORT"))
            .unwrap_or(9528);
        let wss_port = o
            .wss_port
            .or_else(|| env_port("RELAY_WSS_PORT"))
            .unwrap_or(9529);
        let health_port = o
            .health_port
            .or_else(|| env_port("RELAY_HEALTH_PORT"))
            .unwrap_or(9530);

        // Loopback unless the host says otherwise. See `Config::health_bind`.
        let health_bind = o.health_bind.unwrap_or_else(|| {
            std::env::var("RELAY_HEALTH_BIND")
                .ok()
                .and_then(|a| a.parse().ok())
                .unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]))
        });

        // Loopback unless the host says otherwise, for the same reason and
        // because the plaintext listener is the one that carries the bearer
        // token in the clear. See `Config::ws_bind`.
        let ws_bind = o.ws_bind.unwrap_or_else(|| {
            std::env::var("RELAY_WS_BIND")
                .ok()
                .and_then(|a| a.parse().ok())
                .unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]))
        });

        let hmac_secret_file = o
            .hmac_secret_file
            .clone()
            .or_else(|| std::env::var("HMAC_SECRET_FILE").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("./secrets/hmac_secret"));
        let hmac_secret: Vec<u8> = match std::env::var("HMAC_SECRET") {
            Ok(s) if !s.is_empty() => s.into_bytes(),
            _ => load_or_create_hmac_secret_from(Some(hmac_secret_file.clone()))?,
        };

        let health_token = o
            .health_token
            .or_else(|| std::env::var("RELAY_HEALTH_TOKEN").ok())
            .filter(|t| !t.is_empty())
            .unwrap_or_default();

        let metrics_token = o
            .metrics_token
            .or_else(|| std::env::var("RELAY_METRICS_TOKEN").ok())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());

        let nonce_file = o
            .nonce_file
            .or_else(|| std::env::var("RELAY_NONCE_FILE").ok().map(PathBuf::from))
            .unwrap_or_else(default_nonce_file);

        let enable_plain_ws = o
            .enable_plain_ws
            .or_else(|| {
                std::env::var("RELAY_ENABLE_PLAIN_WS")
                    .ok()
                    .map(|v| v == "true" || v == "1")
            })
            .unwrap_or(false);

        let auth_timeout_secs = o
            .auth_timeout_secs
            .or_else(|| std::env::var("RELAY_AUTH_TIMEOUT_SECS").ok()?.parse().ok())
            .unwrap_or(10);

        // Subject alternative names and the certificate directory come from the
        // host when it knows them, and from the environment otherwise. A relay
        // that is only reachable at `localhost` but is told to say so will fail
        // hostname verification for every real client, which is the most common
        // way a self-hosted relay appears to work and then does not.
        let tls = match (o.tls_hostname, o.tls_extra_sans, o.tls_cert_dir) {
            (Some(hostname), extra, cert_dir) => crate::tls::TlsParams {
                hostname: hostname.trim().to_string(),
                extra_sans: extra.unwrap_or_default(),
                cert_dir: cert_dir.unwrap_or_else(|| {
                    std::env::var("RELAY_CERT_DIR")
                        .ok()
                        .filter(|d| !d.is_empty())
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("./certs"))
                }),
            },
            (None, None, None) => crate::tls::TlsParams::from_env(),
            // A host that supplied only *some* of them still gets the
            // environment's value for the rest.
            (hostname, extra, cert_dir) => {
                let base = crate::tls::TlsParams::from_env();
                crate::tls::TlsParams {
                    hostname: hostname
                        .map(|h| h.trim().to_string())
                        .filter(|h| !h.is_empty())
                        .unwrap_or(base.hostname),
                    extra_sans: extra.unwrap_or(base.extra_sans),
                    cert_dir: cert_dir.unwrap_or(base.cert_dir),
                }
            }
        };

        let config = Config {
            ws_port,
            ws_bind,
            wss_port,
            health_port,
            health_bind,
            health_token,
            metrics_token,
            hmac_secret,
            hmac_secret_file,
            enable_plain_ws,
            nonce_file,
            auth_timeout_secs,
            relay_token,
            tls,
            relay_cert_pin: o
                .relay_cert_pin
                .or_else(|| std::env::var("RELAY_CERT_PIN").ok())
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty()),
        };
        Ok(config.with_health_token_fallback())
    }
}

/// Read a port from the environment, ignoring an unparseable value.
///
/// A malformed port is a typo, not a decision: falling back to the default and
/// continuing is friendlier than refusing to start, and the value it lands on
/// is written in the startup log.
pub(crate) fn env_port(name: &str) -> Option<u16> {
    std::env::var(name).ok()?.parse().ok()
}

/// The replay cache's default location.
///
/// The relay used to hardcode `./data/nonces.json`, which meant the cache landed
/// wherever the process happened to be started. A host that embeds the relay
/// needs it under the app's data directory, so the default follows the same
/// platform convention as the rest of Conduit's state.
pub fn default_nonce_file() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    base.join("conduit").join("relay-nonces.json")
}

impl Config {
    /// Fill an empty `health_token` from the stored `hmac_secret`.
    ///
    /// `from_env` leaves `health_token` empty when `RELAY_HEALTH_TOKEN` is
    /// unset or empty; this restores the documented default so the effective
    /// token is always derived from the persisted secret in one place.
    pub(crate) fn with_health_token_fallback(mut self) -> Self {
        if self.health_token.is_empty() {
            self.health_token = String::from_utf8_lossy(&self.hmac_secret).to_string();
        }
        self
    }

    /// The bearer token actually guarding `GET /health` and `GET /`.
    ///
    /// Read path for the health endpoint: falls back to the HMAC secret for
    /// any `Config` built without an explicit `RELAY_HEALTH_TOKEN`.
    pub(crate) fn effective_health_token(&self) -> &str {
        if self.health_token.is_empty() {
            std::str::from_utf8(&self.hmac_secret).unwrap_or_default()
        } else {
            &self.health_token
        }
    }
}

/// Constant-time bearer token check for the health endpoint.
pub(crate) fn bearer_token_authorized(header_value: &str, expected: &str) -> bool {
    let Some(provided) = header_value.strip_prefix("Bearer ") else {
        return false;
    };
    let expected_b = expected.as_bytes();
    let provided_b = provided.as_bytes();
    if expected_b.len() != provided_b.len() {
        return false;
    }
    bool::from(subtle::ConstantTimeEq::ct_eq(expected_b, provided_b))
}
