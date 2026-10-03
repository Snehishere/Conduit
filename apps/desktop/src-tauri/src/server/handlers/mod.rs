pub mod audio;
pub mod auto_rules;
pub mod files;
pub mod notifications;
pub mod pairing;
pub mod remote_input;
pub mod screen_mirror;

use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

use crate::audio::AudioStream;
use crate::encryption::EncryptionManager;
use crate::file_transfer::FileTransferEngine;
use crate::security::{PerTypeRateLimiter, RateLimiter, TokenStore};
use crate::storage::Storage;
use crate::sync::SyncEngine;

pub type ClientSender = broadcast::Sender<String>;
pub type Clients = Arc<RwLock<HashMap<String, ClientSender>>>;
pub type WsToDeviceId = Arc<RwLock<HashMap<String, String>>>;

/// Stable device id granted to the desktop's own webview once it has proved
/// possession of the per-launch local capability. It has no shared secret (the
/// webview has no key pair), so it is *not* in `SyncEngine`; see
/// [`crate::security::local_capability`].
pub const LOCAL_DESKTOP_ID: &str = "local_desktop";

#[derive(Clone)]
pub struct WsContext {
    pub clients: Clients,
    pub ws_to_device_id: WsToDeviceId,
    pub sync_engine: Arc<RwLock<SyncEngine>>,
    pub encryption: Arc<EncryptionManager>,
    pub file_engine: Arc<FileTransferEngine>,
    pub token_store: Arc<TokenStore>,
    pub rate_limiter: Arc<RateLimiter>,
    pub per_type_limiter: Arc<PerTypeRateLimiter>,
    pub storage: Arc<Storage>,
    pub automation_engine: Arc<RwLock<crate::automation::AutomationEngine>>,
    pub audio_stream: Arc<AudioStream>,
    /// This desktop's own device id, which is also its relay route-key id.
    pub device_id: Arc<String>,
    /// How a device id becomes the key it signs relayed frames with. Shared with
    /// the relay host, so the two can never disagree about who may route.
    pub route_keys: Arc<crate::relay::DeviceRouteKeys>,
    pub relay_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<String>>>>,
}

// ---------------------------------------------------------------------------
// Pairing registry
//
// `ws_to_device_id` is the *single* source of truth for "is this connection
// trusted?". It is written in exactly three places — `mark_paired` below,
// `disconnect_client` (revocation), and the disconnect cleanup — and read by
// the auth gate, the broadcast filter, the per-socket encryption decision and
// `send_to`. A second `paired: HashSet` would be a second source of truth that
// could drift from the one the auth gate already reads, which is precisely how
// the "unpaired peer gets everything in cleartext" bug happened in the first
// place.
//
// A connection with no entry is **unpaired**: it may `ping`, `pong`, `pair`,
// `discover` and receive unicast replies to its own requests, and nothing else.
// ---------------------------------------------------------------------------

/// Register `client_id` as the trusted local desktop (capability-proved).
pub async fn mark_local_desktop(ctx: &WsContext, client_id: &str) {
    ctx.ws_to_device_id
        .write()
        .await
        .insert(client_id.to_string(), LOCAL_DESKTOP_ID.to_string());
    info!("Local desktop capability accepted for connection {client_id}");
}

/// Register `client_id` as a paired device.
pub async fn mark_paired(ctx: &WsContext, client_id: &str, device_id: &str) {
    ctx.ws_to_device_id
        .write()
        .await
        .insert(client_id.to_string(), device_id.to_string());
}

/// The shared secret for `device_id`, from whichever authority still has it.
///
/// **The one answer to "does this device id name a paired device, and what do I
/// encrypt for it".** Three places used to ask this separately and disagreed,
/// because they read two different maps that mean different things:
///
///   * [`crate::sync::SyncEngine`] is a **liveness** map. `handle_client` removes
///     a device from it the instant its LAN socket closes, so a phone that
///     paired on the LAN and then moved networks is *absent from it* while
///     still paired, still registered with the relay, and still sending.
///   * `ctx.route_keys` is the **pairing registry**, refreshed from the device
///     table and refreshed again every [`KEY_REFRESH_INTERVAL`]. It outlives
///     the socket, which is exactly the property a relay peer needs.
///
/// Order matters and is not arbitrary: `SyncEngine` is consulted first so that
/// every device with a live socket keeps byte-identical behaviour to before,
/// including the case where a re-pair has written a fresh secret there and the
/// relay's copy is up to 5 s behind. `route_keys` is the fallback, not a
/// replacement.
///
/// `None` for this desktop's own id: its route key is self-generated and has no
/// pairing secret behind it. Callers that must not address it subtract it
/// separately — see [`crate::server::WsServer::fan_out_to_relay`].
pub async fn peer_secret(ctx: &WsContext, device_id: &str) -> Option<String> {
    if let Some(client) = ctx.sync_engine.read().await.get_client(device_id) {
        return Some(client.shared_secret.clone());
    }
    ctx.route_keys.secret_for(device_id)
}

/// Whether `client_id` holds *any* identity, i.e. an entry exists in the
/// pairing registry.
///
/// This is the **broadcast eligibility** predicate, and it is deliberately the
/// weaker of the two: the desktop's own webview is `local_desktop`, has no key
/// pair and therefore no shared secret, and it must still receive broadcasts
/// (that is what renders the notification list). Unpaired connections have no
/// entry, so they are excluded.
pub async fn has_identity(ctx: &WsContext, client_id: &str) -> bool {
    ctx.ws_to_device_id.read().await.contains_key(client_id)
}

/// Whether `client_id` may send a message that the auth gate protects.
///
/// Stronger than [`has_identity`]: besides an entry, a device must have the
/// shared secret that pairing derived. The extra requirement is redundant for
/// the two writers of the registry — `mark_paired` always runs next to
/// `SyncEngine::add_client`, and `mark_local_desktop` is the explicit
/// no-secret case — but it means an entry on its own is never sufficient, so a
/// half-finished pairing is treated as untrusted instead of trusted.
///
/// A relayed message arrives already attributed to a device that has no socket
/// here, so `client_id` is a **device id** and there is no registry entry for
/// it. That case resolves through [`peer_secret`]: the relay authenticated the
/// sender during `relay_auth` and re-checked the route signature against that
/// same id, so the device registry is the authority, and the absence of a LAN
/// socket is not evidence of anything.
///
/// Resolved in two steps rather than under one guard, so `ws_to_device_id` is
/// read and released before `SyncEngine` is taken. Two readers taking two locks
/// in opposite orders cannot deadlock whatever else is queued — that was never
/// the hazard here — but it does mean the two cannot be held simultaneously,
/// which is worth preserving now that this function is on the auth gate for
/// every protected frame.
pub async fn is_trusted_peer(ctx: &WsContext, client_id: &str) -> bool {
    // A relayed sender, or a device id. Tried first because a LAN connection id
    // is never also a device id.
    if let Some(secret) = peer_secret(ctx, client_id).await {
        return !secret.is_empty();
    }

    let Some(stable_id) = paired_device_id(ctx, client_id).await else {
        return false;
    };
    stable_id == LOCAL_DESKTOP_ID
        || peer_secret(ctx, &stable_id)
            .await
            .is_some_and(|s| !s.is_empty())
}

/// The stable device id bound to `client_id`, if any.
pub async fn paired_device_id(ctx: &WsContext, client_id: &str) -> Option<String> {
    ctx.ws_to_device_id.read().await.get(client_id).cloned()
}

/// Fan a message out to every **paired** peer except the sender, on whichever
/// transport reaches each one.
///
/// Unpaired connections are skipped. This is the fix for the plaintext-leak
/// vulnerability: every accepted socket used to be in `ctx.clients`, so a peer
/// that had merely opened a connection received notification bodies, clipboard
/// content, SMS bodies, base64 file chunks, screen-mirror frames and audio —
/// and, having no `ws_to_device_id` entry, the per-socket encryption path could
/// not even find a shared secret for it, so the **raw plaintext** was written
/// to its socket.
///
/// # The relay arm
///
/// The loop below can only reach `ctx.clients`, and the relay socket is not in
/// it — `relay_tx` is an `mpsc::Sender`, not a connection. So a phone reachable
/// only through the relay received nothing from any of the seventeen handlers
/// that call this: clipboard, notifications, SMS, calls, discovery/remove and
/// every file-transfer control frame. It failed silently, because iterating an
/// empty set is indistinguishable from success. That is what
/// [`crate::server::WsServer::fan_out_to_relay`] is for, and it targets
/// exactly the paired devices with **no** socket here, so a device that is on
/// the LAN as well is served by this loop and never served twice.
///
/// # Lock order, which is why the relay arm runs first
///
/// The relay arm seals per recipient, which reads `SyncEngine`. This loop holds
/// `clients` and `ws_to_device_id` readers, and `is_trusted_peer` takes
/// `SyncEngine` then `ws_to_device_id` — the opposite order. Calling the arm
/// while either guard is live would invert it, and tokio's `RwLock` is
/// write-preferring, so a single queued writer would wedge both. Running it
/// first, before either guard is taken, makes the two orders independent rather
/// than nested.
pub async fn broadcast_to_others(ctx: &WsContext, sender_id: &str, message: &str) {
    // `sender_id` is a connection id on the LAN and a **device id** when the
    // sender is relayed, and the relay arm excludes by device. Resolved through
    // the registry so one comparison answers "is this the sender" on both
    // transports. For a relayed sender the registry has no entry and the id is
    // already a device id, so it stands — which is the case that matters: the
    // raw string would have excluded nothing and handed the phone its own
    // notification back.
    let sender_device = paired_device_id(ctx, sender_id)
        .await
        .unwrap_or_else(|| sender_id.to_string());
    crate::server::WsServer::fan_out_to_relay(ctx, Some(&sender_device), message).await;

    let clients = ctx.clients.read().await;
    let identities = ctx.ws_to_device_id.read().await;
    for (id, client_tx) in clients.iter() {
        if id != sender_id && identities.contains_key(id) {
            let _ = client_tx.send(message.to_string());
        }
    }
}

/// Unicast a message to exactly one connection.
///
/// Deliberately **not** filtered on pairing: this is how a device that has not
/// paired yet receives the replies to its own `pairing/request` and
/// `discovery/announce` frames. Only ever used for request/response, never for
/// fan-out.
pub async fn send_to_client(ctx: &WsContext, client_id: &str, message: &str) -> bool {
    let clients = ctx.clients.read().await;
    if let Some(tx) = clients.get(client_id) {
        let _ = tx.send(message.to_string());
        return true;
    }
    false
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use rusqlite::Connection;
    use tokio::sync::broadcast;

    /// Create a `WsContext` with in-memory storage for unit tests.
    pub fn create_test_ctx() -> WsContext {
        // In-memory SQLite with production schema
        let mut db = Connection::open_in_memory().expect("failed to open in-memory db");
        db.execute_batch("PRAGMA journal_mode=WAL;")
            .expect("failed to set WAL");
        let migrations = rusqlite_migration::Migrations::new(vec![rusqlite_migration::M::up(
            include_str!("../../migrations/001_initial.sql"),
        )]);
        migrations
            .to_latest(&mut db)
            .expect("failed to run migrations");
        let storage = Arc::new(Storage::from_connection(db));

        let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
        let ws_to_device_id: WsToDeviceId = Arc::new(RwLock::new(HashMap::new()));
        let sync_engine = Arc::new(RwLock::new(SyncEngine::new()));
        let encryption = Arc::new(EncryptionManager::new_random());
        let file_engine = Arc::new(FileTransferEngine::new());
        // Must match the production TokenStore TTL (60s) so tests exercise real expiry behavior.
        let token_store = Arc::new(TokenStore::new(std::time::Duration::from_secs(60)));
        let rate_limiter = Arc::new(RateLimiter::new(crate::security::RateLimitConfig::default()));
        let per_type_limiter = Arc::new(PerTypeRateLimiter::new());
        let automation_engine = Arc::new(RwLock::new(crate::automation::AutomationEngine::new()));
        let audio_stream = Arc::new(AudioStream::new());
        let device_id = Arc::new("test-device".to_string());
        let route_keys = Arc::new(crate::relay::DeviceRouteKeys::new(
            storage.clone(),
            device_id.as_str().to_string(),
        ));

        WsContext {
            clients,
            ws_to_device_id,
            sync_engine,
            encryption,
            file_engine,
            token_store,
            rate_limiter,
            per_type_limiter,
            storage,
            automation_engine,
            audio_stream,
            device_id,
            route_keys,
            relay_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Insert a connected client that has an **identity in the pairing registry
    /// but no derived shared secret** — the state a connection is in between
    /// `mark_paired` and `SyncEngine::add_client`.
    ///
    /// Two consequences, both of which are deliberate and both of which are
    /// pinned by tests:
    ///
    ///  * it **is** broadcast-eligible (`has_identity` is true), so handler
    ///    tests that assert "the message reached the other clients" keep
    ///    working;
    ///  * it is **not** authorised to send protected messages
    ///    (`is_trusted_peer` is false), which is what the
    ///    `*_unauthenticated_rejected_by_dispatcher` tests in the handler
    ///    modules rely on.
    ///
    /// Tests that want a *fully* paired client — a real one, past the gate and
    /// with a secret — should use [`add_test_paired_client`]. Tests that want no
    /// identity at all should use [`add_test_unpaired_client`].
    pub async fn add_test_client(ctx: &WsContext, device_id: &str) -> broadcast::Sender<String> {
        add_test_client_mapped(ctx, device_id, device_id).await
    }

    /// Insert a connected client with a stable device-id mapping.
    pub async fn add_test_client_mapped(
        ctx: &WsContext,
        ws_client_id: &str,
        device_id: &str,
    ) -> broadcast::Sender<String> {
        let (tx, _) = broadcast::channel(16);
        ctx.clients
            .write()
            .await
            .insert(ws_client_id.to_string(), tx.clone());
        ctx.ws_to_device_id
            .write()
            .await
            .insert(ws_client_id.to_string(), device_id.to_string());
        tx
    }

    /// Insert a connected client that has **not** completed pairing.
    ///
    /// This is the state every real connection used to be in without a token,
    /// and it is what the broadcast filter must exclude.
    pub async fn add_test_unpaired_client(
        ctx: &WsContext,
        ws_client_id: &str,
    ) -> broadcast::Sender<String> {
        let (tx, _) = broadcast::channel(16);
        ctx.clients
            .write()
            .await
            .insert(ws_client_id.to_string(), tx.clone());
        tx
    }

    /// Insert a **fully paired** client: registry entry *and* the shared secret
    /// pairing derived.
    ///
    /// The auth gate requires both, so a test that needs to get past it must
    /// register both. [`add_test_client_mapped`] registers only the entry, which
    /// is enough for broadcast *eligibility* but not to send protected
    /// messages.
    pub async fn add_test_paired_client(
        ctx: &WsContext,
        ws_client_id: &str,
        device_id: &str,
    ) -> broadcast::Sender<String> {
        let tx = add_test_client_mapped(ctx, ws_client_id, device_id).await;
        ctx.sync_engine
            .write()
            .await
            .add_client(crate::sync::ConnectedClient {
                device_id: device_id.to_string(),
                device_name: device_id.to_string(),
                device_type: "phone".to_string(),
                shared_secret: "00".repeat(32),
                last_heartbeat: chrono::Utc::now().timestamp(),
                battery_level: None,
            });
        tx
    }

    /// Make this desktop one that is hosting **and connected to** a relay, with
    /// `devices` paired in the device registry, and hand back the receiving end
    /// of the relay's egress queue.
    ///
    /// Every part of this is load-bearing, and the reason a relay-only peer is
    /// hard to model in a test is that each piece of it used to be faked
    /// separately:
    ///
    ///   * the registry row, because [`crate::server::WsServer::fan_out_to_relay`]
    ///     enumerates the route keys, which are refreshed from the device table —
    ///     and `create_test_ctx` never refreshes them, so `routable_device_ids()`
    ///     is empty and the relay arm is dead code in every other test here;
    ///   * this desktop's **own** id in that set, because it is what makes the
    ///     self-exclusion test meaningful rather than vacuous: `relay_route_to`
    ///     gates on `is_trusted_peer`, and the desktop has no pairing secret of
    ///     its own, so without a seeded one the gate would mask a missing filter;
    ///   * the channel, because without a connected relay the fan-out returns 0
    ///     on its first line.
    ///
    /// The registry row and the route key are deliberately *not* the same thing,
    /// and both are written: the row is what [`crate::handlers::is_trusted_peer`]
    /// eventually consults and what a test asserting on storage sees, while the
    /// key is what the fan-out enumerates. Seeding the keys directly rather than
    /// calling `refresh()` keeps the fixture off the OS keyring.
    ///
    /// The returned receiver is what a test asserts *on*: it is the relay's
    /// routing table from the desktop's point of view, so what lands in it is
    /// exactly what a phone would have received.
    ///
    /// `devices` is `(device_id, shared_secret_hex)`. A device listed here has a
    /// registry row but **no socket** — that is the relay-only state, and it is
    /// not the state `add_test_paired_client` builds (that one also registers a
    /// connection and a `ws_to_device_id` entry). The fixture asserts that
    /// absence itself, so it cannot quietly grow a socket later and turn every
    /// test here into a tautology.
    pub async fn add_test_relay(
        ctx: &WsContext,
        devices: &[(&str, &str)],
    ) -> tokio::sync::mpsc::Receiver<String> {
        for (id, secret) in devices {
            ctx.storage
                .save_device(&crate::storage::StoredDevice {
                    id: (*id).to_string(),
                    name: (*id).to_string(),
                    device_type: "mobile".to_string(),
                    os: "android".to_string(),
                    public_key: "00".repeat(32),
                    shared_secret: (*secret).to_string(),
                    paired_at: 0,
                    last_seen: 0,
                    battery: None,
                    signal: None,
                    status: "paired".to_string(),
                })
                .await
                .expect("save paired device for the relay fixture");
            let bytes = hex::decode(secret).expect("the fixture's secret must be hex");
            ctx.route_keys.register_for_tests(
                id,
                conduit_protocol::hmac::derive_route_key(&bytes, id).to_vec(),
                secret,
            );
        }
        // This desktop registers itself under its own id in production. Seeded
        // here with a secret it does not really have, so that a fan-out which
        // failed to exclude it would be caught by `relay_route_to`'s trust gate
        // rather than passing because the gate happened to refuse it.
        let own = ctx.device_id.as_str();
        ctx.route_keys
            .register_for_tests(own, vec![0x5a; 32], &"5a".repeat(32));

        // The fixture's whole purpose is a peer with *no* socket here. Assert it
        // rather than trusting it: if this ever stops holding, every test using
        // the fixture silently becomes a LAN test.
        for (id, _) in devices {
            assert!(
                !ctx.clients.read().await.contains_key(*id),
                "{id} must have no socket for this fixture to model a relay-only peer"
            );
            assert!(
                ctx.ws_to_device_id.read().await.get(*id).is_none(),
                "{id} must have no pairing-registry entry either"
            );
        }

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        *ctx.relay_tx.write().await = Some(tx);
        rx
    }

    /// The single frame sitting in a relay queue, parsed.
    ///
    /// Asserts there is **exactly one**, because "served twice" is the failure
    /// mode a fan-out fix introduces most easily and a fan-out that sends twice
    /// is worse than one that sends never: duplicate notifications, duplicate
    /// clipboard rows, and duplicated file chunks corrupting a transfer.
    #[track_caller]
    pub fn only_relay_frame(rx: &mut tokio::sync::mpsc::Receiver<String>) -> serde_json::Value {
        let frame = rx.try_recv().expect("the relay queue must carry the frame");
        assert!(
            rx.try_recv().is_err(),
            "exactly one frame may be routed per recipient; a second one means \
             the peer is served twice"
        );
        serde_json::from_str(&frame).expect("a relayed frame is JSON")
    }

    /// Nothing at all was routed.
    #[track_caller]
    pub fn assert_relay_silent(rx: &mut tokio::sync::mpsc::Receiver<String>) {
        assert!(
            rx.try_recv().is_err(),
            "nothing may be routed through the relay in this case"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{
        add_test_client, add_test_client_mapped, add_test_unpaired_client, create_test_ctx,
    };

    // NOTE: `broadcast_to_others` filters on the pairing registry, not on an
    // "auth concept at this layer" — pairing *is* the concept at this layer.

    // ── broadcast_to_others ───────────────────────────────────────────────────

    /// The three clients the old fan-out reached: sender + two peers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_to_others_happy_reaches_all_paired_peers_except_sender() {
        let ctx = create_test_ctx();
        let sender_tx = add_test_client_mapped(&ctx, "sender", "dev_sender").await;
        let peer1_tx = add_test_client_mapped(&ctx, "peer1", "dev_1").await;
        let peer2_tx = add_test_client_mapped(&ctx, "peer2", "dev_2").await;
        let mut sender_rx = sender_tx.subscribe();
        let mut peer1_rx = peer1_tx.subscribe();
        let mut peer2_rx = peer2_tx.subscribe();

        broadcast_to_others(&ctx, "sender", "hello").await;

        let p1 = tokio::time::timeout(std::time::Duration::from_millis(500), peer1_rx.recv()).await;
        let p2 = tokio::time::timeout(std::time::Duration::from_millis(500), peer2_rx.recv()).await;
        assert!(
            p1.is_ok() && p2.is_ok(),
            "both paired peers must receive the message"
        );
        assert_eq!(p1.unwrap().unwrap(), "hello");
        assert_eq!(p2.unwrap().unwrap(), "hello");

        let s = tokio::time::timeout(std::time::Duration::from_millis(200), sender_rx.recv()).await;
        assert!(s.is_err(), "sender must NOT receive its own broadcast");
    }

    /// V2: a connection that has merely opened a socket must receive nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_to_others_unpaired_peer_receives_nothing() {
        let ctx = create_test_ctx();
        let sender_tx = add_test_client_mapped(&ctx, "sender", "dev_sender").await;
        let paired_tx = add_test_client_mapped(&ctx, "paired", "dev_1").await;
        let unpaired_tx = add_test_unpaired_client(&ctx, "stranger").await;
        let _sender_rx = sender_tx.subscribe();
        let mut paired_rx = paired_tx.subscribe();
        let mut stranger_rx = unpaired_tx.subscribe();

        broadcast_to_others(&ctx, "sender", r#"{"secret":"clipboard"}"#).await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(300), stranger_rx.recv()).await;
        assert!(
            leaked.is_err(),
            "an unpaired connection must not receive broadcasts at all, plaintext or not"
        );

        let got =
            tokio::time::timeout(std::time::Duration::from_millis(500), paired_rx.recv()).await;
        assert!(
            got.is_ok(),
            "a paired connection must still receive everything"
        );
        assert_eq!(got.unwrap().unwrap(), r#"{"secret":"clipboard"}"#);
    }

    /// Revocation must take effect immediately, not at the next reconnect.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_to_others_edge_revoked_peer_is_cut_off() {
        let ctx = create_test_ctx();
        let sender_tx = add_test_client_mapped(&ctx, "sender", "dev_sender").await;
        let revoked_tx = add_test_client_mapped(&ctx, "revoked", "dev_x").await;
        let _sender_rx = sender_tx.subscribe();
        let mut revoked_rx = revoked_tx.subscribe();

        ctx.ws_to_device_id.write().await.remove("revoked");

        broadcast_to_others(&ctx, "sender", "x").await;
        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(300), revoked_rx.recv()).await;
        assert!(leaked.is_err(), "a revoked peer must stop receiving");
    }

    /// The desktop's own webview is paired as `local_desktop` and must keep
    /// receiving broadcasts — that is what renders the notification list.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_to_others_local_desktop_is_a_recipient() {
        let ctx = create_test_ctx();
        let phone = add_test_client_mapped(&ctx, "phone", "dev_phone").await;
        let _phone_rx = phone.subscribe();
        let webview = add_test_unpaired_client(&ctx, "webview").await;
        mark_local_desktop(&ctx, "webview").await;
        let mut webview_rx = webview.subscribe();

        broadcast_to_others(&ctx, "phone", "n").await;
        let got =
            tokio::time::timeout(std::time::Duration::from_millis(500), webview_rx.recv()).await;
        assert!(
            got.is_ok(),
            "the trusted local webview must still be served"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_to_others_edge_no_clients_is_noop() {
        let ctx = create_test_ctx();
        // No clients registered → must not panic.
        broadcast_to_others(&ctx, "ghost", "anyone there?").await;
    }

    // ── send_to_client ────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_to_client_valid_known_client_returns_true_and_delivers() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "known").await;
        let mut rx = tx.subscribe();

        let delivered = send_to_client(&ctx, "known", "ping").await;
        assert!(delivered, "known client must return true");

        let got = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(got.is_ok(), "message must be delivered");
        assert_eq!(got.unwrap().unwrap(), "ping");
    }

    /// A pairing reply must reach a peer that is not paired *yet*.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_to_client_reaches_an_unpaired_peer_for_its_own_request() {
        let ctx = create_test_ctx();
        let tx = add_test_unpaired_client(&ctx, "pairing_in_progress").await;
        let mut rx = tx.subscribe();

        assert!(send_to_client(&ctx, "pairing_in_progress", "accept").await);
        let got = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(got.is_ok(), "unicast replies must not require pairing");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_to_client_invalid_unknown_client_returns_false() {
        let ctx = create_test_ctx();
        let delivered = send_to_client(&ctx, "nope", "ping").await;
        assert!(!delivered, "unknown client must return false, not panic");
    }

    // ── pairing registry ──────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn has_identity_reflects_the_registry() {
        let ctx = create_test_ctx();
        add_test_unpaired_client(&ctx, "u").await;
        assert!(!has_identity(&ctx, "u").await);
        assert!(paired_device_id(&ctx, "u").await.is_none());
        assert!(!is_trusted_peer(&ctx, "u").await);

        mark_paired(&ctx, "u", "dev_1").await;
        assert!(has_identity(&ctx, "u").await);
        assert_eq!(paired_device_id(&ctx, "u").await.as_deref(), Some("dev_1"));
    }

    /// An identity entry without the shared secret is not enough to be trusted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_trusted_peer_requires_the_shared_secret() {
        let ctx = create_test_ctx();
        add_test_client_mapped(&ctx, "ws", "dev_1").await;
        assert!(
            has_identity(&ctx, "ws").await,
            "an identity alone makes the client broadcast-eligible"
        );
        assert!(
            !is_trusted_peer(&ctx, "ws").await,
            "but without a derived secret it may not send protected messages"
        );

        ctx.sync_engine
            .write()
            .await
            .add_client(crate::sync::ConnectedClient {
                device_id: "dev_1".to_string(),
                device_name: "Peer".to_string(),
                device_type: "phone".to_string(),
                shared_secret: "00".repeat(32),
                last_heartbeat: 0,
                battery_level: None,
            });
        assert!(is_trusted_peer(&ctx, "ws").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn is_trusted_peer_accepts_the_local_desktop_without_a_secret() {
        let ctx = create_test_ctx();
        add_test_unpaired_client(&ctx, "webview").await;
        mark_local_desktop(&ctx, "webview").await;
        assert!(is_trusted_peer(&ctx, "webview").await);
    }
}
