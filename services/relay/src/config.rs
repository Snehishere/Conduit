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
//! **never** used to verify a `relay_route`. It is key material only: the
//! `/health` token default is *derived* from it under a domain-separated
//! label, and the secret itself never leaves this module. Every secret the
//! resolver accepts is held to a minimum length floor — see
//! [`MIN_SECRET_LEN`]. Keeping the roles separate is the relay’s core
//! security property — see ADR-0004.

use std::path::PathBuf;

use log::info;

/// The effective configuration a relay runs with.
///
/// Built by [`Config::resolve`], which is the only constructor: there is no way
/// to build one field-by-field and end up with a half-configured relay.
///
/// Hand-written [`Debug`](std::fmt::Debug): see the impl below.
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
    ///
    /// `RELAY_HEALTH_TOKEN` (or [`Overrides::health_token`]) is honoured after
    /// trimming and the [`MIN_SECRET_LEN`] floor. When it is unset, this field
    /// is filled with a token *derived* from the master secret rather than a
    /// copy of it — see [`derive_health_token`]. So the no-token-configured
    /// case stays gated: `/health` always demands a bearer token, and the
    /// value it demands is not the master secret. `/healthz` remains the
    /// unauthenticated liveness probe. Holding this token never reveals the
    /// master secret.
    pub(crate) health_token: String,
    /// Optional bearer token guarding `GET /metrics`.
    ///
    /// `/metrics` is unauthenticated by default (a deliberate, tested decision
    /// so Prometheus can scrape without distributing the health secret). Setting
    /// `RELAY_METRICS_TOKEN` opts into gating it.
    pub metrics_token: Option<String>,
    /// Resolved HMAC master secret: `HMAC_SECRET` env var, else the contents of
    /// `HMAC_SECRET_FILE`, else a freshly generated + persisted 64-char hex
    /// secret (see [`load_or_create_hmac_secret_from`]). An all-whitespace
    /// `HMAC_SECRET` counts as unset, exactly like an empty one, so a
    /// half-exported variable can never become the secret.
    ///
    /// `hmac_secret` and `health_token` are **never equal**: when
    /// `RELAY_HEALTH_TOKEN` is unset or empty, `health_token` is derived from
    /// this secret with a domain-separated label (see
    /// [`derive_health_token`]), so holding the health token reveals nothing
    /// about the master secret. They used to share one value whenever the
    /// override was absent — one credential, two roles (W6.2) — and the
    /// derivation is what removed that.
    ///
    /// The master secret is **never** used to verify `relay_route` HMACs and
    /// is never exposed to clients or sent over the wire. Deriving the health
    /// token is its only job, which is also why an ephemeral secret would be a
    /// bug: it would silently mint a new health token on every restart.
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

/// How a secret appears in `Debug` output: its length, never its value.
///
/// See the hand-written [`Debug`](std::fmt::Debug) impl for [`Config`].
struct SecretLength(usize);

impl std::fmt::Debug for SecretLength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED {} bytes]", self.0)
    }
}

/// Hand-written so `{:?}` stays useful without being a credential dump.
///
/// The derived version printed `hmac_secret`, `relay_token`, `health_token`
/// and `metrics_token` verbatim — and `{:?}` reaches assert messages, log
/// lines and panic output, so any one of those leaked every secret the relay
/// holds (W6.22). The replacement keeps every non-secret field as-is (ports,
/// binds, paths, TLS settings) and reduces each secret to its length, which
/// is what you actually want to see when debugging a config: present or not,
/// and long enough to be plausible.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("ws_port", &self.ws_port)
            .field("ws_bind", &self.ws_bind)
            .field("wss_port", &self.wss_port)
            .field("health_port", &self.health_port)
            .field("health_bind", &self.health_bind)
            .field("health_token", &SecretLength(self.health_token.len()))
            .field(
                "metrics_token",
                &self.metrics_token.as_ref().map(|t| SecretLength(t.len())),
            )
            .field("hmac_secret", &SecretLength(self.hmac_secret.len()))
            .field("enable_plain_ws", &self.enable_plain_ws)
            .field("hmac_secret_file", &self.hmac_secret_file)
            .field("nonce_file", &self.nonce_file)
            .field("auth_timeout_secs", &self.auth_timeout_secs)
            .field("relay_token", &SecretLength(self.relay_token.len()))
            .field("tls", &self.tls)
            .field("relay_cert_pin", &self.relay_cert_pin)
            .finish()
    }
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

/// Minimum length, in bytes after trimming, for every secret the resolver
/// accepts: `RELAY_TOKEN`, `HMAC_SECRET`, and an explicitly set
/// `RELAY_HEALTH_TOKEN` / `RELAY_METRICS_TOKEN`.
///
/// Why a floor at all (W6.21): these are bearer credentials, and a
/// one-character token — or `RELAY_TOKEN="   "` — is guessed on the first
/// request. Eight bytes is where a hand-picked value stops being
/// indistinguishable from a placeholder (`"t"`, `"tok"`, `"   "`), while
/// still leaving operators free to choose their own secret; it is a floor,
/// not a target, and the values this relay generates are 64 hex characters
/// (256 bits). Length is only a proxy for entropy — nothing here can measure
/// the generator — so the check trims surrounding whitespace first: padding
/// is the one inflation a naive `len >= 8` would count as strong.
pub(crate) const MIN_SECRET_LEN: usize = 8;

/// Trim surrounding whitespace and enforce [`MIN_SECRET_LEN`], returning the
/// trimmed value.
///
/// The refused value never appears in the error — only its length — so the
/// message that aborts startup cannot leak the secret it is rejecting.
/// Callers treat a value that is empty *after* trimming as unset before it
/// reaches this, which keeps the two failure modes distinct: "no token
/// configured" versus "token too weak".
fn secret_with_minimum_length(secret: String, what: &str) -> Result<String, String> {
    let trimmed = secret.trim().to_string();
    let len = trimmed.len();
    if len < MIN_SECRET_LEN {
        Err(format!(
            "{what} is too short: {len} bytes after trimming, at least {MIN_SECRET_LEN} required (fail-closed)"
        ))
    } else {
        Ok(trimmed)
    }
}

impl Config {
    /// Resolve the effective configuration.
    ///
    /// Precedence, highest first: `overrides`, then the environment, then the
    /// built-in default.
    ///
    /// Fails only on things a relay cannot safely ignore: no bearer token, a
    /// secret below the length floor, an unreadable or unwritable master
    /// secret, and a port variable that does not name a port. Everything else
    /// falls back — unset ports take their defaults, binds stay on loopback,
    /// the certificate is generated on first run — because a relay that starts
    /// with defaults is still better than one that refuses to run. A *set but
    /// wrong* value never falls back, though: guessing the port the operator
    /// mistyped hands them a listener nobody asked for.
    ///
    /// The error is a `String` rather than a typed error because it is surfaced
    /// to a log and to a status field, never matched on.
    pub fn resolve(overrides: Option<Overrides>) -> Result<Self, String> {
        let o = overrides.unwrap_or_default();

        // Presence first — the common misconfiguration gets the message that
        // says how to fix it — then the floor, so a present-but-guessable
        // token is refused just as loudly as a missing one (W6.21).
        let relay_token = o
            .relay_token
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("RELAY_TOKEN").ok())
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                "no relay token: set one in Settings, or set RELAY_TOKEN (fail-closed)".to_string()
            })?;
        let relay_token = secret_with_minimum_length(relay_token, "relay token")?;

        // The environment is consulted only when the host pinned no override,
        // and a variable that is set but does not name a port is a hard error
        // rather than a silent default. See `env_port`.
        let ws_port = match o.ws_port {
            Some(port) => port,
            None => env_port("RELAY_WS_PORT")?.unwrap_or(9528),
        };
        let wss_port = match o.wss_port {
            Some(port) => port,
            None => env_port("RELAY_WSS_PORT")?.unwrap_or(9529),
        };
        let health_port = match o.health_port {
            Some(port) => port,
            None => env_port("RELAY_HEALTH_PORT")?.unwrap_or(9530),
        };

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
        let hmac_secret = match std::env::var("HMAC_SECRET") {
            // Whitespace-only counts as unset, like an empty value: a
            // half-exported variable must not silently become the secret.
            Ok(s) if !s.trim().is_empty() => s,
            _ => {
                let from_file = load_or_create_hmac_secret_from(Some(hmac_secret_file.clone()))?;
                // The loader only ever produces text — an env string, hex it
                // generated, or `read_to_string` — so this cannot fail in
                // practice; it exists so a corrupt file is refused, not
                // lossily re-encoded.
                String::from_utf8(from_file)
                    .map_err(|_| "HMAC secret is not valid UTF-8 (fail-closed)".to_string())?
            }
        };
        let hmac_secret = secret_with_minimum_length(hmac_secret, "HMAC secret")?.into_bytes();

        // Empty after this means "not configured": `resolve` finishes by
        // deriving a purpose-bound token instead of copying the secret (W6.2).
        let health_token = o
            .health_token
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("RELAY_HEALTH_TOKEN").ok())
            .filter(|t| !t.trim().is_empty())
            .map(|t| secret_with_minimum_length(t, "health token"))
            .transpose()?
            .unwrap_or_default();

        // `None` leaves `/metrics` open — the deliberate default Prometheus
        // scrapes without a credential. A token that *is* set must clear the
        // floor like every other secret.
        let metrics_token = o
            .metrics_token
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("RELAY_METRICS_TOKEN").ok())
            .filter(|t| !t.trim().is_empty())
            .map(|t| secret_with_minimum_length(t, "metrics token"))
            .transpose()?;

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
                hostname: usable_hostname(&hostname),
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
                        .as_deref()
                        .map(usable_hostname)
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
        Ok(config.with_derived_health_token())
    }
}

/// The name a generated certificate has to be valid for, or the default when
/// the host supplied nothing usable.
///
/// An empty hostname is not a name, and it is not harmless either.
/// [`crate::tls::TlsParams::subject_alt_names`] puts this value at the head of
/// the SAN list unconditionally, so an empty one produced a zero-length
/// `dNSName` *and* displaced `localhost` — leaving a certificate valid for
/// nothing at all, which every real client refuses. This arm of the resolver
/// did not filter it, so "leave it empty to mean localhost" only worked if
/// every host remembered to spell that out; the desktop, whose `relay_hostname`
/// setting defaults to empty, did not.
///
/// So an empty or whitespace-only hostname resolves to the same default the
/// environment path and [`crate::tls::TlsParams::default`] use. Fail-safe in
/// the direction that matters: the alternative is a certificate no client can
/// verify, which fails closed at the handshake instead of quietly serving
/// something unusable.
fn usable_hostname(hostname: &str) -> String {
    let trimmed = hostname.trim();
    if trimmed.is_empty() {
        crate::tls::TlsParams::default().hostname
    } else {
        trimmed.to_string()
    }
}

/// Read a port from the environment: `Ok(None)` when unset or empty.
///
/// A set-but-unparseable value is a **hard error**, not a silent fallback to
/// the default. `RELAY_WSS_PORT=95x9` used to quietly bind 9529 — a relay on
/// a port the operator, the firewall and the health checks all disagree
/// about, with nothing in the logs to say so. Refusing to start matches how
/// this resolver already treats a wrong value it can see: a missing relay
/// token is a hard error for the same reason (fail-closed), and a warning
/// would still leave the relay bound to a port nobody chose. An unset or
/// empty variable is not a typo: it yields `None` and the caller's default
/// applies.
pub(crate) fn env_port(name: &str) -> Result<Option<u16>, String> {
    let raw = match std::env::var(name) {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse().map(Some).map_err(|e| {
        format!(
            "{name}={raw:?} is not a valid port ({e}); fix it or unset {name} to take the default (fail-closed)"
        )
    })
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

/// Domain-separation label for the default `/health` bearer token.
///
/// Distinct from [`crate::hmac::ROUTE_KEY_LABEL`] and from every other label
/// [`crate::hmac::derive_key`] is called with, so the health token and any
/// key derived from the same master secret are unrelated values.
pub(crate) const HEALTH_TOKEN_LABEL: &str = "conduit-relay/v1/health-token";

/// Derive the default `/health` bearer token from the master secret.
///
/// Purpose-bound rather than shared (W6.2): `derive_key` is HMAC-SHA256 under
/// a domain-separated label, so the result is 64 hex characters that are not
/// the master secret, cannot be inverted to recover it, and are stable across
/// restarts because the secret is. The endpoint therefore stays gated when
/// `RELAY_HEALTH_TOKEN` is unset — nobody can *guess* the derived value — but
/// only an operator who sets `RELAY_HEALTH_TOKEN` (or `/healthz`, which needs
/// no token) can actually use `/health`. Requiring `RELAY_HEALTH_TOKEN`
/// outright was the other option and was rejected: the desktop sets no health
/// token, so a required one would refuse to start the very app the relay
/// serves.
pub(crate) fn derive_health_token(hmac_secret: &[u8]) -> String {
    hex::encode(crate::hmac::derive_key(hmac_secret, HEALTH_TOKEN_LABEL))
}

impl Config {
    /// Fill an empty `health_token` with a token derived from `hmac_secret`.
    ///
    /// `resolve` leaves `health_token` empty when `RELAY_HEALTH_TOKEN` is
    /// unset or empty; this fills the default in one place. The fill is a
    /// derivation, not a copy (W6.2): the old fallback put the master secret
    /// itself into `health_token`, so anyone holding the health token — a
    /// scraper, a log line, a `{:?}` — held the secret the relay derives from.
    pub(crate) fn with_derived_health_token(mut self) -> Self {
        if self.health_token.is_empty() {
            self.health_token = derive_health_token(&self.hmac_secret);
        }
        self
    }

    /// The bearer token actually guarding `GET /health` and `GET /`.
    ///
    /// Read path for the health endpoint. A `Config` built by [`Config::resolve`]
    /// carries the token in `health_token` already; the derivation here covers
    /// a `Config` assembled field-by-field with an empty token, so no read path
    /// can ever fall back to the master secret the way W6.2 described.
    pub(crate) fn effective_health_token(&self) -> std::borrow::Cow<'_, str> {
        if self.health_token.is_empty() {
            std::borrow::Cow::Owned(derive_health_token(&self.hmac_secret))
        } else {
            std::borrow::Cow::Borrowed(&self.health_token)
        }
    }
}

/// Constant-time bearer token check for the health endpoint.
///
/// An empty `expected` token is refused before any comparison: an
/// unconfigured token and a client sending `Bearer ` are both zero-length, so
/// without this guard the length check passes and the constant-time compare
/// agrees — forgetting to configure a token would *open* the endpoint instead
/// of closing it (W6.19).
pub(crate) fn bearer_token_authorized(header_value: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
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

// ---------------------------------------------------------------
//  Regression tests for the security-audit items owned by this file:
//  W6.2 (health token ≠ master secret), W6.19 (empty expected token),
//  W6.21 (secret length floor, loud port failures), W6.22 (redacted
//  Debug). They live here rather than in `suite.rs` so they can reach
//  `pub(crate)` items without widening anything's visibility.
// ---------------------------------------------------------------

#[cfg(test)]
// `std::env::set_var`/`remove_var` are `unsafe` in edition 2024, and these
// tests exist to prove the resolver's environment handling — same exception
// the crate-level `deny(unsafe_code)` makes for `suite.rs`.
#[allow(unsafe_code)]
mod config_tests {
    use super::*;

    /// Every environment variable [`Config::resolve`] reads.
    ///
    /// Cleared before each test and restored after, so a value exported in
    /// the developer's shell can neither hide a bug nor fail an assertion.
    const ENV_KEYS: &[&str] = &[
        "RELAY_TOKEN",
        "HMAC_SECRET",
        "HMAC_SECRET_FILE",
        "RELAY_HEALTH_TOKEN",
        "RELAY_METRICS_TOKEN",
        "RELAY_WS_PORT",
        "RELAY_WSS_PORT",
        "RELAY_HEALTH_PORT",
        "RELAY_WS_BIND",
        "RELAY_HEALTH_BIND",
        "RELAY_ENABLE_PLAIN_WS",
        "RELAY_AUTH_TIMEOUT_SECS",
        "RELAY_NONCE_FILE",
        "RELAY_TLS_HOSTNAME",
        "RELAY_CERT_DIR",
        "RELAY_CERT_PIN",
    ];

    /// Clear the resolver's environment, run `f`, then restore what was there.
    ///
    /// Must be paired with `#[serial_test::serial]`: the environment is
    /// process-wide, and `suite.rs`'s config tests share it.
    fn with_env_snapshot<F: FnOnce()>(f: F) {
        let saved: Vec<(&'static str, Option<String>)> = ENV_KEYS
            .iter()
            .map(|&k| (k, std::env::var(k).ok()))
            .collect();
        for &k in ENV_KEYS {
            unsafe { std::env::remove_var(k) };
        }
        f();
        for (k, v) in saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    /// Fixtures that clear [`MIN_SECRET_LEN`] wherever they are accepted.
    const STRONG_TOKEN: &str = "fixture-relay-token-01";
    const STRONG_SECRET: &str = "fixture-master-secret-0123456789";

    // ---------------------------------------------------------------
    //  W6.2 — the /health token must not be the master secret
    // ---------------------------------------------------------------

    #[test]
    #[serial_test::serial]
    fn health_token_is_derived_not_copied_from_the_master_secret() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
            }
            let cfg = Config::resolve(None).expect("strong token and secret resolve");

            assert_ne!(
                cfg.health_token, STRONG_SECRET,
                "W6.2: the /health token must not be the master secret"
            );
            assert_eq!(
                cfg.health_token,
                derive_health_token(STRONG_SECRET.as_bytes()),
                "unset RELAY_HEALTH_TOKEN must yield the derived default, in one place"
            );
            let effective: &str = &cfg.effective_health_token();
            assert_eq!(effective, cfg.health_token);
            assert_eq!(
                cfg.health_token.len(),
                64,
                "the derived token is 32 bytes hex-encoded"
            );
            assert!(cfg.health_token.chars().all(|c| c.is_ascii_hexdigit()));

            // Domain separation: the same secret under a different label must
            // give a different value, or the label is decoration.
            assert_ne!(
                cfg.health_token,
                hex::encode(crate::hmac::derive_key(
                    STRONG_SECRET.as_bytes(),
                    crate::hmac::ROUTE_KEY_LABEL
                )),
                "the health token must be purpose-bound, not any old derived key"
            );

            // Deterministic: a restart must not invalidate a deployed token.
            let again = Config::resolve(None).expect("second resolve");
            assert_eq!(again.health_token, cfg.health_token);
        });
    }

    #[test]
    #[serial_test::serial]
    fn explicit_relay_health_token_overrides_the_derived_default() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
                std::env::set_var("RELAY_HEALTH_TOKEN", "explicit-health-token");
            }
            let cfg = Config::resolve(None).expect("resolve");
            assert_eq!(cfg.health_token, "explicit-health-token");
            let effective: &str = &cfg.effective_health_token();
            assert_eq!(effective, "explicit-health-token");
        });
    }

    #[test]
    fn effective_health_token_never_exposes_a_hand_built_configs_secret() {
        // `resolve` always populates `health_token`, but the read path is what
        // the endpoint trusts; a field-by-field `Config` with an empty token
        // must derive, not leak `hmac_secret` the way W6.2 described.
        let secret = b"hand-built-master-secret!".to_vec();
        let cfg = Config {
            ws_port: 9528,
            ws_bind: "127.0.0.1".parse().unwrap(),
            wss_port: 9529,
            health_port: 9530,
            health_bind: "127.0.0.1".parse().unwrap(),
            health_token: String::new(),
            metrics_token: None,
            hmac_secret: secret.clone(),
            hmac_secret_file: std::path::PathBuf::from("./secrets/hmac_secret"),
            enable_plain_ws: false,
            nonce_file: std::path::PathBuf::from("./data/nonces.json"),
            auth_timeout_secs: 10,
            relay_token: "hand-built-relay-token".to_string(),
            tls: crate::tls::TlsParams::default(),
            relay_cert_pin: None,
        };

        let effective: &str = &cfg.effective_health_token();
        assert_ne!(
            effective,
            String::from_utf8_lossy(&secret),
            "W6.2: the read path must not hand out the master secret"
        );
        assert_eq!(effective, derive_health_token(&secret));
    }

    // ---------------------------------------------------------------
    //  W6.19 — an empty expected token must deny
    // ---------------------------------------------------------------

    #[test]
    fn bearer_token_authorized_denies_an_empty_expected_token() {
        assert!(
            !bearer_token_authorized("Bearer ", ""),
            "W6.19: zero-length expected against zero-length provided must deny, \
             not pass the length check and compare equal"
        );
        assert!(!bearer_token_authorized("", ""));
        assert!(!bearer_token_authorized("Bearer x", ""));

        // A configured token still behaves.
        assert!(bearer_token_authorized(
            "Bearer configured-token",
            "configured-token"
        ));
        assert!(!bearer_token_authorized(
            "Bearer wrong-token",
            "configured-token"
        ));
    }

    // ---------------------------------------------------------------
    //  W6.21 — minimum length floor, and loud port failures
    // ---------------------------------------------------------------

    #[test]
    #[serial_test::serial]
    fn weak_relay_tokens_are_refused() {
        with_env_snapshot(|| {
            unsafe { std::env::set_var("HMAC_SECRET", STRONG_SECRET) };

            unsafe { std::env::set_var("RELAY_TOKEN", "t") };
            let err = Config::resolve(None).expect_err("a one-character token must be refused");
            assert!(
                err.contains("relay token is too short"),
                "the refusal must name the weak credential: {err}"
            );

            unsafe { std::env::set_var("RELAY_TOKEN", "1234567") };
            let err = Config::resolve(None).expect_err("seven bytes is below the floor");
            assert!(
                err.contains("relay token is too short"),
                "the refusal must name the weak credential: {err}"
            );
            assert!(
                !err.contains("1234567"),
                "the refused value must not appear in the error: {err}"
            );

            unsafe { std::env::set_var("RELAY_TOKEN", "   ") };
            let err = Config::resolve(None).expect_err("whitespace is not a token");
            assert!(
                err.contains("no relay token"),
                "whitespace-only counts as unset, so the fail-closed message applies: {err}"
            );

            unsafe { std::env::set_var("RELAY_TOKEN", "12345678") };
            Config::resolve(None).expect("exactly the floor is accepted");
            unsafe { std::env::set_var("RELAY_TOKEN", STRONG_TOKEN) };
            Config::resolve(None).expect("a comfortable token is accepted");
        });
    }

    #[test]
    #[serial_test::serial]
    fn weak_hmac_secret_is_refused() {
        with_env_snapshot(|| {
            unsafe { std::env::set_var("RELAY_TOKEN", STRONG_TOKEN) };

            unsafe { std::env::set_var("HMAC_SECRET", "x") };
            let err = Config::resolve(None).expect_err("a one-byte master secret must be refused");
            assert!(
                err.contains("HMAC secret is too short"),
                "the refusal must name the weak credential: {err}"
            );

            unsafe { std::env::set_var("HMAC_SECRET", "1234567") };
            let err = Config::resolve(None)
                .expect_err("seven bytes is below the floor for the master secret");
            assert!(
                !err.contains("1234567"),
                "the refused value must not appear in the error: {err}"
            );

            unsafe { std::env::set_var("HMAC_SECRET", "12345678") };
            Config::resolve(None).expect("exactly the floor is accepted");
        });
    }

    #[test]
    #[serial_test::serial]
    fn whitespace_only_hmac_secret_env_is_treated_as_unset() {
        with_env_snapshot(|| {
            let dir = std::env::temp_dir().join(format!("relay-cfg-hmac-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("temp dir");
            let file = dir.join("hmac_secret");
            let file_secret = "file-supplied-secret-0123456789";
            std::fs::write(&file, file_secret).expect("fixture secret file");

            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", "   ");
                std::env::set_var(
                    "HMAC_SECRET_FILE",
                    file.to_str().expect("temp path is UTF-8"),
                );
            }

            let cfg = Config::resolve(None)
                .expect("whitespace-only falls through to the file, which supplies the secret");
            assert_eq!(
                cfg.hmac_secret,
                file_secret.as_bytes(),
                "an all-whitespace HMAC_SECRET must never become the master secret"
            );

            std::fs::remove_file(&file).ok();
            std::fs::remove_dir(&dir).ok();
        });
    }

    #[test]
    #[serial_test::serial]
    fn weak_explicit_health_and_metrics_tokens_are_refused() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
                std::env::set_var("RELAY_HEALTH_TOKEN", "short");
            }
            let err = Config::resolve(None)
                .expect_err("an explicit health token below the floor must be refused");
            assert!(
                err.contains("health token is too short"),
                "the refusal must name the weak credential: {err}"
            );

            unsafe {
                std::env::remove_var("RELAY_HEALTH_TOKEN");
                std::env::set_var("RELAY_METRICS_TOKEN", "short");
            }
            let err = Config::resolve(None)
                .expect_err("an explicit metrics token below the floor must be refused");
            assert!(
                err.contains("metrics token is too short"),
                "the refusal must name the weak credential: {err}"
            );

            unsafe {
                std::env::set_var("RELAY_METRICS_TOKEN", "strong-metrics-token");
            }
            let cfg = Config::resolve(None).expect("a strong metrics token resolves");
            assert_eq!(cfg.metrics_token.as_deref(), Some("strong-metrics-token"));

            // Unset stays the documented default: `/metrics` is open.
            unsafe { std::env::remove_var("RELAY_METRICS_TOKEN") };
            let cfg = Config::resolve(None).expect("resolve");
            assert!(cfg.metrics_token.is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn unparseable_port_is_an_error_not_a_silent_default() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
                std::env::set_var("RELAY_WSS_PORT", "95x9");
            }

            let err = Config::resolve(None)
                .expect_err("95x9 names no port; the relay must refuse to guess");
            assert!(
                err.contains("RELAY_WSS_PORT"),
                "the error must name the variable the operator got wrong: {err}"
            );
            assert!(
                !err.contains("9529"),
                "nothing may suggest it silently landed on the default: {err}"
            );

            unsafe { std::env::set_var("RELAY_WSS_PORT", "70000") };
            let err =
                Config::resolve(None).expect_err("70000 is outside u16 and must be refused too");
            assert!(err.contains("RELAY_WSS_PORT"), "got: {err}");

            unsafe { std::env::set_var("RELAY_WSS_PORT", "8443") };
            let cfg = Config::resolve(None).expect("a valid port resolves");
            assert_eq!(cfg.wss_port, 8443);
            assert_eq!(
                cfg.ws_port, 9528,
                "unset ports still take their defaults alongside a set one"
            );
            assert_eq!(cfg.health_port, 9530);
        });
    }

    #[test]
    #[serial_test::serial]
    fn empty_port_variable_takes_the_default() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
                std::env::set_var("RELAY_WS_PORT", "");
            }
            // An empty variable is "unset with extra keystrokes", not a typo:
            // the default applies. Only a *set* value that parses wrong fails.
            let cfg = Config::resolve(None).expect("empty means default");
            assert_eq!(cfg.ws_port, 9528);
        });
    }

    // ---------------------------------------------------------------
    //  W6.22 — Debug output must not carry credentials
    // ---------------------------------------------------------------

    #[test]
    fn debug_output_redacts_secrets_and_keeps_the_rest() {
        let cfg = Config {
            ws_port: 9528,
            ws_bind: "127.0.0.1".parse().unwrap(),
            wss_port: 9529,
            health_port: 9530,
            health_bind: "127.0.0.1".parse().unwrap(),
            health_token: "debug-health-token-value".to_string(),
            metrics_token: Some("debug-metrics-token".to_string()),
            hmac_secret: b"debug-master-secret-bytes!".to_vec(),
            hmac_secret_file: std::path::PathBuf::from("./secrets/hmac_secret"),
            enable_plain_ws: false,
            nonce_file: std::path::PathBuf::from("./data/nonces.json"),
            auth_timeout_secs: 10,
            relay_token: "debug-relay-token-value".to_string(),
            tls: crate::tls::TlsParams::default(),
            relay_cert_pin: Some("sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()),
        };

        let rendered = format!("{cfg:?}");
        for secret in [
            "debug-master-secret-bytes!",
            "debug-health-token-value",
            "debug-metrics-token",
            "debug-relay-token-value",
        ] {
            assert!(
                !rendered.contains(secret),
                "W6.22: Debug leaked {secret:?}: {rendered}"
            );
        }
        assert!(
            rendered.contains("[REDACTED"),
            "secrets must still show as present, with their length: {rendered}"
        );
        for field in [
            "ws_port",
            "wss_port",
            "health_port",
            "relay_token",
            "health_token",
            "metrics_token",
            "hmac_secret",
        ] {
            assert!(
                rendered.contains(field),
                "field {field} must stay visible in Debug output: {rendered}"
            );
        }
        assert!(
            rendered.contains("9528"),
            "non-secret values must stay readable: {rendered}"
        );
    }

    // ---------------------------------------------------------------
    //  An empty TLS hostname must not become a zero-length SAN
    // ---------------------------------------------------------------

    /// The bug this closes: the `(Some(hostname), …)` arm took the value
    /// verbatim, and `TlsParams::subject_alt_names` puts it at the head of the
    /// SAN list unconditionally. A host that passes `Some(String::new())` —
    /// which is exactly what the desktop did with its default `relay_hostname`
    /// — therefore got `["", "127.0.0.1", "::1"]`: a zero-length `dNSName` and
    /// no `localhost`, so the generated certificate verified against nothing.
    #[test]
    #[serial_test::serial]
    fn an_empty_tls_hostname_still_yields_a_certificate_valid_for_localhost() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
            }

            for empty in ["", "   ", "\t"] {
                let cfg = Config::resolve(Some(Overrides {
                    tls_hostname: Some(empty.to_string()),
                    tls_cert_dir: Some(PathBuf::from("./certs")),
                    ..Overrides::default()
                }))
                .expect("an empty hostname is not a config error; it means the default");

                assert_eq!(
                    cfg.tls.hostname, "localhost",
                    "{empty:?} must resolve to the documented default, not to itself"
                );
                let sans = cfg.tls.subject_alt_names();
                assert!(
                    sans.contains(&"localhost".to_string()),
                    "the certificate must be valid for localhost: {sans:?}"
                );
                assert!(
                    !sans.iter().any(|s| s.is_empty()),
                    "a zero-length dNSName is what broke certificate verification: {sans:?}"
                );
            }
        });
    }

    /// The counterpart: a hostname an operator *did* set still reaches the
    /// certificate. Filtering the empty case must not become filtering
    /// everything.
    #[test]
    #[serial_test::serial]
    fn a_configured_tls_hostname_reaches_the_certificate_unchanged() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
                // A stray environment value must not shadow the host's own.
                std::env::set_var("RELAY_TLS_HOSTNAME", "environment.example.com");
            }

            let cfg = Config::resolve(Some(Overrides {
                tls_hostname: Some(" relay.example.com ".to_string()),
                tls_cert_dir: Some(PathBuf::from("./certs")),
                ..Overrides::default()
            }))
            .expect("resolve");

            assert_eq!(
                cfg.tls.hostname, "relay.example.com",
                "the host's own setting must win over the environment, trimmed"
            );
            assert!(
                cfg.tls
                    .subject_alt_names()
                    .contains(&"relay.example.com".to_string()),
                "the name the relay is reached by must be in its certificate"
            );
            assert!(
                !cfg.tls
                    .subject_alt_names()
                    .contains(&"environment.example.com".to_string()),
                "the environment is a fallback for values the host did not pin, not an override"
            );
        });
    }

    /// The same guarantee has to hold on the fallback path: with no
    /// `tls_hostname` at all the resolver mixes the environment in with the
    /// host's other TLS fields, and a whitespace-only `RELAY_TLS_HOSTNAME`
    /// must not slip through that arm either.
    #[test]
    #[serial_test::serial]
    fn an_empty_environment_hostname_cannot_produce_an_empty_san_either() {
        with_env_snapshot(|| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", STRONG_TOKEN);
                std::env::set_var("HMAC_SECRET", STRONG_SECRET);
                std::env::set_var("RELAY_TLS_HOSTNAME", "   ");
            }

            let cfg = Config::resolve(Some(Overrides {
                // `tls_cert_dir` is set, so this takes the partial arm.
                tls_cert_dir: Some(PathBuf::from("./certs")),
                ..Overrides::default()
            }))
            .expect("resolve");

            assert_eq!(cfg.tls.hostname, "localhost");
            assert!(!cfg.tls.subject_alt_names().iter().any(String::is_empty));
        });
    }
}
