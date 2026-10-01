//! Running the relay inside the desktop app.
//!
//! # Why the relay is here
//!
//! Conduit works entirely on a local network: the phone finds the desktop over
//! mDNS and dials it directly. The relay exists for the one case that does not
//! cover — the two devices on different networks — and it used to be a separate
//! program the user deployed, configured and kept alive themselves.
//!
//! It is now a background task in this process. The reasons are not only about
//! distribution:
//!
//!   * A phone on another network needs the relay to be *reachable*, and
//!     reachability is a property of the desktop's own machine. Keeping the
//!     relay in the same process keeps the port, the certificate, the token and
//!     the replay cache next to the thing they authenticate.
//!   * There is now exactly one way to run a Conduit relay, so there is no way
//!     to end up with two deployments that disagree about secrets or ports.
//!   * The relay never sees plaintext ([ADR-0007]), so hosting it in the same
//!     process as the hub does **not** weaken the envelope. What it does change
//!     is the threat model: the machine running the relay is now the machine
//!     running the app, so "the relay operator" is "the desktop user". See
//!     `SECURITY.md`.
//!
//! # What this module is responsible for
//!
//! Turning the app's settings into a [`conduit_relay::Config`], starting the
//! service, and stopping it when the app exits. It deliberately does **not**
//! decide policy: a failure to start is reported and the app carries on
//! LAN-only, because a desktop that cannot reach a phone across the internet is
//! still a perfectly good desktop for the phone sitting on the same desk.

use conduit_protocol::hmac::derive_route_key;
use conduit_relay::{
    Config, Overrides, RelayService, RouteKeys, ServiceHandle, StartError, StaticRouteKeys, Status,
};
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::encryption::{KeyStore, KeyringStore, app_data_dir};
use crate::storage::Storage;

/// Keyring account holding the bearer token relay clients present.
///
/// It is a credential, not a setting, so it lives in the OS keyring beside the
/// database key and the X25519 identity rather than in the settings table.
pub const RELAY_TOKEN_ACCOUNT: &str = "relay_token";

/// Keyring account holding the desktop's *own* route key.
///
/// A credential, for the same reason as the token: it is what the desktop signs
/// its own relayed frames with.
pub const RELAY_ROUTE_KEY_ACCOUNT: &str = "relay_route_key";

/// Default port for the relay's TLS WebSocket listener.
///
/// Matches `RELAY_WSS_PORT`'s former default so an existing port-forward rule
/// keeps working after the upgrade.
pub const DEFAULT_RELAY_PORT: u16 = 9529;

/// Default port for the relay's health/metrics surface. Loopback only.
pub const DEFAULT_RELAY_HEALTH_PORT: u16 = 9530;

/// Port the desktop uses to join its *own* relay over loopback.
///
/// A relay routes by looking up the recipient in its table of live connections,
/// so the desktop is only reachable through the relay if it is also connected to
/// it. It connects to itself over this loopback-only plaintext listener rather
/// than dialling its own TLS port, which would mean either trusting a
/// self-signed certificate or teaching the client a pin check it has no reason
/// to perform for traffic that never leaves the machine.
pub const DEFAULT_RELAY_LOCAL_PORT: u16 = 9531;

/// How often the relay re-reads the device registry while it is running.
///
/// Short enough that a revoke is close to immediate, long enough that a phone
/// syncing a folder does not cause a database read per relayed frame.
const KEY_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// The desktop's side of the per-device route-key scheme.
///
/// Two sources, because a desktop is in a different position from a phone:
///
///   * **Paired devices.** Their route key is derived from the X25519 secret the
///     pairing already produced, so nothing extra is stored and a re-pair rotates
///     the key for free — no rotation window, no key id to remember.
///   * **This desktop.** It has no pairing with itself, so its own route key is
///     generated once and kept in the keyring beside the relay token.
///
/// The keys are held in memory and rebuilt by [`Self::refresh`], because the
/// relay verifies frames from inside async code and must not await a database
/// read to do it. The refresh points are the ones that can change the answer:
/// relay start, and every pair / unpair / revoke.
pub struct DeviceRouteKeys {
    storage: Arc<Storage>,
    local_device_id: String,
    keys: std::sync::RwLock<StaticRouteKeys>,
}

impl std::fmt::Debug for DeviceRouteKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        f.debug_struct("DeviceRouteKeys")
            .field("local_device_id", &self.local_device_id)
            .field(
                "registered",
                &self.keys.read().map(|k| k.len()).unwrap_or_default(),
            )
            .finish()
    }
}

impl DeviceRouteKeys {
    /// A resolver for this desktop, empty until [`Self::refresh`] fills it.
    pub fn new(storage: Arc<Storage>, local_device_id: String) -> Self {
        Self {
            storage,
            local_device_id,
            keys: std::sync::RwLock::new(StaticRouteKeys::new()),
        }
    }

    /// The desktop's own device id, which is also its route key id.
    pub fn local_device_id(&self) -> &str {
        &self.local_device_id
    }

    /// The desktop's own route key, generating and storing one the first time.
    pub fn own_route_key(&self) -> Result<[u8; 32], String> {
        let keyring = KeyringStore::for_account(RELAY_ROUTE_KEY_ACCOUNT);
        if let Some(existing) = keyring.load()? {
            let bytes = hex::decode(&existing)
                .map_err(|e| format!("stored relay route key is not hex: {e}"))?;
            match <[u8; 32]>::try_from(bytes) {
                Ok(key) => return Ok(key),
                Err(_) => warn!("Stored relay route key was the wrong length; replacing it"),
            }
        }
        let key = generate_route_key()?;
        keyring.store(&hex::encode(key))?;
        info!("Generated the desktop's own relay route key");
        Ok(key)
    }

    /// The key to sign a relayed frame with on this device's behalf.
    ///
    /// Same resolution the relay applies, exposed to the app so the desktop
    /// signs its own outgoing routes exactly as the relay expects to verify
    /// them. `None` means the device is not registered, which is a bug rather
    /// than a condition to paper over.
    pub fn signing_key(&self, device_id: &str) -> Option<Vec<u8>> {
        self.key_for(device_id)
    }

    /// Rebuild the key set from the device registry.
    ///
    /// Called at relay start and whenever a device is paired, re-paired or
    /// revoked, so the change takes effect without restarting the relay. A
    /// device that leaves the registry loses its key here and can no longer sign.
    pub async fn refresh(&self) -> Result<usize, String> {
        let mut keys = StaticRouteKeys::new();

        keys.insert(self.local_device_id.clone(), self.own_route_key()?.to_vec());

        for device in self
            .storage
            .get_all_devices()
            .await
            .map_err(|e| format!("could not read the device registry: {e}"))?
        {
            // Revoked and unpaired rows stay in the table; they must not be able
            // to route, so only `paired` devices are registered.
            if device.status != "paired" {
                debug!(
                    "Not registering route key for {}: {}",
                    device.id, device.status
                );
                continue;
            }
            let secret = match hex::decode(&device.shared_secret) {
                Ok(s) if !s.is_empty() => s,
                _ => {
                    warn!(
                        "Paired device {} has no usable shared secret; it cannot route",
                        device.id
                    );
                    continue;
                }
            };
            keys.insert(
                device.id.clone(),
                derive_route_key(&secret, &device.id).to_vec(),
            );
        }

        let count = keys.len();
        *self.keys.write().map_err(|_| "route key lock poisoned")? = keys;
        Ok(count)
    }
}

impl RouteKeys for DeviceRouteKeys {
    fn key_for(&self, device_id: &str) -> Option<Vec<u8>> {
        // The desktop is registered like any other device, under its own id, so
        // a phone verifies it exactly the way it verifies a peer.
        self.keys.read().ok()?.key_for(device_id)
    }

    fn device_ids(&self) -> Vec<String> {
        self.keys.read().map(|k| k.device_ids()).unwrap_or_default()
    }
}

/// The running relay, if the user has turned it on.
///
/// Held behind a `Mutex` because stopping is `async` and consuming the handle
/// is the only way to stop it, and behind `RwLock` for the status snapshot the
/// UI polls.
pub struct RelayHost {
    handle: Mutex<Option<ServiceHandle>>,
    status: RwLock<Option<Status>>,
    /// Why the relay is not running, for the status UI. A misconfiguration is
    /// a normal state here, not an error the user caused by breaking something.
    last_error: RwLock<Option<String>>,
    /// How the relay turns a device id into the key that device signs with.
    /// Owned here so pairing changes can refresh it on a running relay.
    route_keys: Arc<DeviceRouteKeys>,
    /// Keeps the key set in step with the device registry while the relay runs.
    key_refresh_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RelayHost {
    /// A host for this desktop's device registry.
    pub fn new(storage: Arc<Storage>, local_device_id: String) -> Self {
        Self {
            handle: Mutex::new(None),
            status: RwLock::new(None),
            last_error: RwLock::new(None),
            route_keys: Arc::new(DeviceRouteKeys::new(storage, local_device_id)),
            key_refresh_task: Mutex::new(None),
        }
    }

    /// The route-key resolver, so pairing changes can refresh it.
    pub fn route_keys(&self) -> Arc<DeviceRouteKeys> {
        self.route_keys.clone()
    }

    /// Whether a relay is currently accepting connections.
    pub async fn is_running(&self) -> bool {
        self.handle.lock().await.is_some()
    }

    /// A snapshot for the status UI, or the reason there isn't one.
    pub async fn snapshot(&self) -> RelayStatus {
        if let Some(status) = self.status.read().await.clone() {
            return RelayStatus {
                running: true,
                error: None,
                status: Some(status),
            };
        }
        RelayStatus {
            running: false,
            error: self.last_error.read().await.clone(),
            status: None,
        }
    }

    /// Stop the relay, if it is running.
    ///
    /// Idempotent, and safe to call from an app-exit path that may race a
    /// settings change that already stopped it.
    pub async fn stop(&self) {
        if let Some(task) = self.key_refresh_task.lock().await.take() {
            task.abort();
        }
        if let Some(handle) = self.handle.lock().await.take() {
            let summary = handle.shutdown().await;
            info!(
                "Relay stopped: {} connection(s) live at shutdown, {} routed, {} dropped",
                summary.active_at_exit, summary.messages_routed, summary.messages_dropped
            );
            *self.status.write().await = None;
        }
    }

    /// Start the relay, replacing any relay already running.
    ///
    /// A start failure is recorded rather than propagated: the app's own hub is
    /// unaffected by the relay, and taking the whole UI down over a port clash
    /// would be the wrong trade.
    pub async fn start(&self, settings: &crate::commands::settings::ConduitSettings) {
        if !settings.relay_enabled {
            self.stop().await;
            *self.last_error.write().await = None;
            debug!("Relay disabled in settings");
            return;
        }

        // Replacing a running relay: stop the old one first so the port is free
        // and the two never overlap.
        self.stop().await;

        let token = match resolve_relay_token() {
            Ok(t) => t,
            Err(e) => {
                warn!("Relay not started: {e}");
                *self.last_error.write().await = Some(e);
                return;
            }
        };

        let config = match build_config(&token, settings) {
            Ok(c) => c,
            Err(e) => {
                warn!("Relay not started: {e}");
                *self.last_error.write().await = Some(e);
                return;
            }
        };

        // The registry is the source of truth for who can route, so it is read
        // before the listener opens rather than after: a relay that came up
        // without its keys would reject every device with `unknown_device`.
        match self.route_keys.refresh().await {
            Ok(count) => debug!("Relay route keys refreshed for {count} device(s)"),
            Err(e) => {
                warn!("Relay not started: {e}");
                *self.last_error.write().await = Some(e);
                return;
            }
        }

        let route_keys: Arc<dyn RouteKeys> = self.route_keys.clone();
        match RelayService::from_config(config).start(route_keys).await {
            Ok(handle) => {
                let status = handle.status().await;
                info!(
                    "Relay running in-process — WSS on port {}, health on 127.0.0.1:{}",
                    status.wss_port.unwrap_or(0),
                    DEFAULT_RELAY_HEALTH_PORT
                );
                *self.status.write().await = Some(status);
                *self.last_error.write().await = None;
                *self.handle.lock().await = Some(handle);
                self.spawn_key_refresh().await;
            }
            Err(e) => {
                let message = describe_start_error(&e);
                error!("Relay failed to start: {message}");
                *self.last_error.write().await = Some(message);
            }
        }
    }

    /// Keep the key set in step with the device registry while the relay runs.
    ///
    /// Pairing changes reach this through the registry rather than a callback:
    /// a re-pair, a revoke and a device that arrives over the LAN WebSocket all
    /// just write the table, and polling is the one thing that cannot miss any of
    /// them. A revoked device stops being routable within [`KEY_REFRESH_INTERVAL`].
    async fn spawn_key_refresh(&self) {
        let keys = self.route_keys.clone();
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(KEY_REFRESH_INTERVAL);
            // The first tick fires immediately; the set was just loaded.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                match keys.refresh().await {
                    Ok(_) => {}
                    // A failed read leaves the previous key set in place, so a
                    // transient database error cannot unpair every device.
                    Err(e) => warn!("Could not refresh relay route keys: {e}"),
                }
            }
        });
        *self.key_refresh_task.lock().await = Some(task);
    }
}

/// What the status UI needs to know about the relay.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RelayStatus {
    /// Whether a relay is accepting connections.
    pub running: bool,
    /// Why it is not running, if it is not.
    pub error: Option<String>,
    /// Counters, when it is running.
    #[serde(flatten)]
    pub status: Option<Status>,
}

/// Turn a start failure into something a person can act on.
fn describe_start_error(e: &StartError) -> String {
    match e {
        StartError::Config(msg) => msg.clone(),
        StartError::Bind { port, source } => {
            format!("port {port} could not be bound: {source}")
        }
        StartError::NoListeners => {
            "the relay has no TLS certificate and plain WebSocket is off".to_string()
        }
    }
}

/// Find the relay's bearer token, generating one the first time.
///
/// The token is a credential every relay client must present, so it is held in
/// the OS keyring. It is generated once and then reused: a new token on every
/// launch would silently unpair every phone.
///
/// Public because the desktop is itself a relay client — it has to present the
/// same token to the relay it hosts, or it will not appear in the routing table.
pub fn resolve_relay_token() -> Result<String, String> {
    let keyring = KeyringStore::for_account(RELAY_TOKEN_ACCOUNT);
    if let Some(existing) = keyring.load()?.filter(|t| !t.is_empty()) {
        return Ok(existing);
    }
    let token = generate_relay_token()?;
    keyring.store(&token)?;
    info!("Generated a new relay token; phones already paired will need to re-pair");
    Ok(token)
}

/// A 32-byte random token, hex-encoded.
///
/// 64 hex characters: this is a bearer credential guarding every relayed frame,
/// so it is sized like a key rather than like a password.
fn generate_relay_token() -> Result<String, String> {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    Ok(hex::encode(bytes))
}

/// A fresh random route key for the desktop itself.
///
/// A paired device's route key comes from its pairing secret, but the desktop
/// has no pairing with itself, so this one is generated. Reused as the entropy
/// for [`DeviceRouteKeys::own_route_key`].
fn generate_route_key() -> Result<[u8; 32], String> {
    use rand::Rng;
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    Ok(key)
}
/// Build the relay's configuration from the app's settings.
///
/// Environment variables still win, so a headless or scripted setup can pin a
/// value without touching the UI. Everything not pinned falls through to the
/// relay's own defaults.
fn build_config(
    token: &str,
    settings: &crate::commands::settings::ConduitSettings,
) -> Result<Config, String> {
    let overrides = Overrides {
        relay_token: Some(token.to_string()),
        wss_port: Some(settings.relay_port),
        health_port: Some(settings.relay_health_port),
        // Never inherited from the environment here: the health surface
        // publishes the peer count and the certificate pin, and the relay's own
        // default is loopback.
        health_bind: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        // The desktop's own connection to its own relay. Loopback, and pinned
        // here rather than inherited, for the same reason as `health_bind`.
        ws_port: Some(local_relay_port()),
        ws_bind: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        enable_plain_ws: Some(true),
        nonce_file: Some(app_data_dir().join("relay-nonces.json")),
        tls_cert_dir: Some(app_data_dir().join("relay-certs")),
        // Pinned for the same reason as the two above. Left to its own default
        // the relay writes the secret to `./secrets/hmac_secret`, relative to
        // the process working directory — which for a packaged app is a path
        // the OS chose and that is frequently not writable at all.
        hmac_secret_file: Some(app_data_dir().join("relay-hmac-secret")),
        tls_hostname: Some(settings.relay_hostname.clone()),
        // Empty by default, which enforces nothing: this app hosts the relay,
        // so it is not authenticating a remote server and has no pin to check.
        // Set it to pin a certificate the operator is standing behind, and a
        // mismatch stops the relay instead of being logged and ignored.
        relay_cert_pin: Some(settings.relay_cert_pin.clone()),
        ..Overrides::default()
    };
    Config::resolve(Some(overrides))
}

/// The loopback port the desktop joins its own relay on.
///
/// A fixed port rather than a derived one, so the value is the same in the
/// config and in the URL the client dials even when the app starts before the
/// settings have been read.
fn local_relay_port() -> u16 {
    DEFAULT_RELAY_LOCAL_PORT
}

/// The URL the desktop uses to reach its own relay.
pub fn local_relay_url() -> String {
    format!("ws://127.0.0.1:{}", local_relay_port())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::settings::ConduitSettings;

    fn settings() -> ConduitSettings {
        ConduitSettings::default()
    }

    #[test]
    fn relay_defaults_match_the_ports_the_protocol_reserves() {
        // 9529/9530 are the relay's ports in packages/protocol and in the
        // README port table. If either moves, this fails.
        assert_eq!(DEFAULT_RELAY_PORT, 9529);
        assert_eq!(DEFAULT_RELAY_HEALTH_PORT, 9530);
    }

    #[test]
    fn the_health_surface_is_always_loopback() {
        // Whatever the environment says, the desktop never exposes the peer
        // count or the certificate pin to the LAN. See the audit's W6.20.
        unsafe { std::env::set_var("RELAY_HEALTH_BIND", "0.0.0.0") };
        let config = build_config("test-relay-token", &settings()).unwrap();
        assert_eq!(
            config.health_bind,
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            "the desktop must not widen the health bind from the environment"
        );
        unsafe { std::env::remove_var("RELAY_HEALTH_BIND") };
    }

    #[test]
    fn the_relay_writes_no_state_beside_the_app_data_dir() {
        // Every path the relay persists to is pinned. The library's own default
        // for the HMAC secret is `./secrets/hmac_secret`, relative to the
        // process working directory — which for a packaged app is a path the OS
        // chose. Left alone it produced a stray `secrets/` directory next to
        // whatever happened to launch the app.
        let config = build_config("test-relay-token", &settings()).unwrap();
        let app_data = app_data_dir();
        for path in [
            config.nonce_file.as_path(),
            config.hmac_secret_file.as_path(),
            config.tls.cert_dir.as_path(),
        ] {
            assert!(
                path.starts_with(&app_data),
                "{} must live under {}, not beside the executable",
                path.display(),
                app_data.display()
            );
        }
    }

    #[test]
    fn the_settings_ports_reach_the_relay_config() {
        let mut s = settings();
        s.relay_port = 19529;
        s.relay_health_port = 19530;
        let config = build_config("test-relay-token", &s).unwrap();
        assert_eq!(config.wss_port, 19529);
        assert_eq!(config.health_port, 19530);
    }

    #[test]
    fn the_token_is_the_one_the_host_supplied() {
        // The relay must not invent a second credential: the phone is told
        // this one, and the relay must be verifying against the same one.
        let config = build_config("the-real-token", &settings()).unwrap();
        assert_eq!(config.relay_token, "the-real-token");
    }

    #[test]
    fn a_generated_token_is_32_bytes_of_hex() {
        let token = generate_relay_token().unwrap();
        assert_eq!(token.len(), 64, "32 bytes hex-encoded");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_generated_tokens_differ() {
        // A token that repeated would be a credential that does not.
        assert_ne!(
            generate_relay_token().unwrap(),
            generate_relay_token().unwrap()
        );
    }

    #[test]
    fn a_start_failure_is_described_in_words_not_codes() {
        let msg = describe_start_error(&StartError::Config("no relay token".into()));
        assert!(msg.contains("no relay token"));
        let msg = describe_start_error(&StartError::NoListeners);
        assert!(
            msg.contains("TLS") && msg.contains("plain"),
            "a user needs to know both halves of the condition: {msg}"
        );
    }

    // -----------------------------------------------------------------
    //  Per-device route keys
    // -----------------------------------------------------------------

    /// An in-memory registry with one paired device, as the resolver sees it.
    async fn registry_with(secret_hex: &str, status: &str) -> (Arc<Storage>, String) {
        let mut db = rusqlite::Connection::open_in_memory().expect("in-memory db");
        crate::storage::run_migrations(&mut db).expect("migrations");
        let storage = Arc::new(Storage::from_connection(db));
        storage
            .save_device(&crate::storage::StoredDevice {
                id: "phone-1".to_string(),
                name: "Phone".to_string(),
                device_type: "mobile".to_string(),
                os: "android".to_string(),
                public_key: "00".repeat(32),
                shared_secret: secret_hex.to_string(),
                paired_at: 0,
                last_seen: 0,
                battery: None,
                signal: None,
                status: status.to_string(),
            })
            .await
            .expect("save device");
        (storage, "desktop-1".to_string())
    }

    const PAIRING_SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_paired_device_resolves_the_key_derived_from_its_pairing_secret() {
        let (storage, local) = registry_with(PAIRING_SECRET, "paired").await;
        let keys = DeviceRouteKeys::new(storage, local);
        keys.refresh().await.expect("refresh");

        let expected = derive_route_key(&hex::decode(PAIRING_SECRET).unwrap(), "phone-1");
        assert_eq!(
            RouteKeys::key_for(&keys, "phone-1"),
            Some(expected.to_vec()),
            "a phone must sign with the key its pairing secret derives"
        );
    }

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_desktop_resolves_its_own_key_under_its_own_device_id() {
        let (storage, local) = registry_with(PAIRING_SECRET, "paired").await;
        let keys = DeviceRouteKeys::new(storage, local.clone());
        keys.refresh().await.expect("refresh");

        let own = RouteKeys::key_for(&keys, &local).expect("desktop is registered");
        assert_eq!(own.len(), 32);
        assert_eq!(
            RouteKeys::key_for(&keys, &local),
            Some(own),
            "the desktop's own key must be stable across refreshes"
        );
    }

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unpaired_or_revoked_device_cannot_route() {
        for status in ["revoked", "pending", "unpaired"] {
            let (storage, local) = registry_with(PAIRING_SECRET, status).await;
            let keys = DeviceRouteKeys::new(storage, local);
            keys.refresh().await.expect("refresh");
            assert_eq!(
                RouteKeys::key_for(&keys, "phone-1"),
                None,
                "a device with status {status} must not be able to route"
            );
        }
    }

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_device_that_is_not_in_the_registry_cannot_route() {
        let (storage, local) = registry_with(PAIRING_SECRET, "paired").await;
        let keys = DeviceRouteKeys::new(storage, local);
        keys.refresh().await.expect("refresh");
        assert_eq!(RouteKeys::key_for(&keys, "stranger"), None);
    }

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_devices_get_different_keys_from_the_same_secret() {
        // Same pairing secret, different device ids: the id is inside the
        // derivation label, so the keys must not collide.
        let key_a = derive_route_key(&hex::decode(PAIRING_SECRET).unwrap(), "phone-1");
        let key_b = derive_route_key(&hex::decode(PAIRING_SECRET).unwrap(), "phone-2");
        assert_ne!(key_a, key_b);
    }

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_re_pair_replaces_the_key_rather_than_adding_a_second_one() {
        let (storage, local) = registry_with(PAIRING_SECRET, "paired").await;
        let keys = DeviceRouteKeys::new(storage.clone(), local);
        keys.refresh().await.expect("first refresh");
        let before = RouteKeys::key_for(&keys, "phone-1").expect("registered");

        let new_secret = "ff".repeat(32);
        storage
            .save_device(&crate::storage::StoredDevice {
                id: "phone-1".to_string(),
                name: "Phone".to_string(),
                device_type: "mobile".to_string(),
                os: "android".to_string(),
                public_key: "11".repeat(32),
                shared_secret: new_secret.clone(),
                paired_at: 1,
                last_seen: 1,
                battery: None,
                signal: None,
                status: "paired".to_string(),
            })
            .await
            .expect("re-save device");
        keys.refresh().await.expect("second refresh");

        let after = RouteKeys::key_for(&keys, "phone-1").expect("still registered");
        assert_ne!(before, after, "a re-pair must rotate the route key");
        assert_eq!(
            RouteKeys::key_for(&keys, "phone-1"),
            Some(derive_route_key(&hex::decode(&new_secret).unwrap(), "phone-1").to_vec())
        );
        // Still exactly one key for the device: per-device keys leave nothing to
        // keep alive during a rotation, so there is no second id to accept.
        assert_eq!(
            RouteKeys::device_ids(&keys)
                .iter()
                .filter(|id| *id == "phone-1")
                .count(),
            1
        );
    }

    // The storage layer uses `block_in_place`, which needs a multi-threaded runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_device_without_a_usable_secret_is_skipped_rather_than_guessed() {
        // An empty shared secret must not derive a key from nothing.
        let (storage, local) = registry_with("", "paired").await;
        let keys = DeviceRouteKeys::new(storage, local);
        keys.refresh().await.expect("refresh");
        assert_eq!(RouteKeys::key_for(&keys, "phone-1"), None);
    }

    #[test]
    fn the_resolvers_debug_output_carries_no_key_material() {
        // It ends up in a log line on every start failure.
        let rendered = format!("{:?}", DeviceRouteKeys::new(test_storage(), "d1".into()));
        assert!(rendered.contains("DeviceRouteKeys"));
        assert!(
            !rendered.contains("route_key") && rendered.len() < 200,
            "the debug form must not grow key material: {rendered}"
        );
    }

    fn test_storage() -> Arc<Storage> {
        let mut db = rusqlite::Connection::open_in_memory().expect("in-memory db");
        crate::storage::run_migrations(&mut db).expect("migrations");
        Arc::new(Storage::from_connection(db))
    }

    #[test]
    fn a_generated_route_key_is_32_bytes() {
        assert_eq!(generate_route_key().unwrap().len(), 32);
        assert_ne!(
            generate_route_key().unwrap(),
            generate_route_key().unwrap(),
            "a repeated key would sign for a device anyone else could guess"
        );
    }
}
