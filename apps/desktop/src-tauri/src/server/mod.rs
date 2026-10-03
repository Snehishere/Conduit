use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, broadcast};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_async_with_config;
use tungstenite::protocol::Message;
use tungstenite::protocol::WebSocketConfig;

use conduit_protocol::types::*;

use crate::audio::AudioStream;
use crate::automation::{self, TriggerContext, TriggerEvent};
use crate::encryption::EncryptionManager;
use crate::file_transfer::FileTransferEngine;
use crate::security::{self, PerTypeRateLimiter, RateLimiter, TokenStore};
use crate::storage::Storage;
use crate::sync::{ConnectedClient, SyncEngine};

pub mod handlers;

use handlers::{ClientSender, Clients, WsContext, WsToDeviceId, broadcast_to_others};

/// Read limits for every accepted WebSocket connection.
///
/// `accept_async` uses tungstenite's defaults (64 MiB message, 16 MiB frame,
/// unbounded write buffer). Those are generous to the point of being a DoS
/// lever: a peer could assemble a 64 MiB message in memory without ever
/// tripping a check, and a client that stops reading could make the server
/// buffer without bound. Pin the limits to the protocol's own budget so a frame
/// larger than the protocol permits is refused while it is being read.
fn ws_read_limits() -> WebSocketConfig {
    // `WebSocketConfig` is `#[non_exhaustive]`, so it must be built by
    // mutating the default rather than with a struct expression.
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(security::MAX_MESSAGE_SIZE);
    config.max_frame_size = Some(security::MAX_FRAME_SIZE);
    // A slow/non-reading client must not be able to grow server memory.
    config.max_write_buffer_size = security::MAX_SEND_BUFFER_BYTES;
    config
}

/// Whether an already-serialised frame declares itself to be of `ty`.
///
/// Used by the outbound path, which must decide *without re-parsing into a
/// handler type* whether a frame still needs encrypting. A structural check is
/// enough and, unlike `contains("\"type\":\"pairing\"")`, it is not fooled by a
/// `"type":"pairing"` substring appearing inside an encrypted payload's data.
fn message_type_is(msg_text: &str, ty: &str) -> bool {
    serde_json::from_str::<Value>(msg_text)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .is_some_and(|found| found == ty)
}

/// A non-sensitive, clearly-marked placeholder for a frame we refused to send
/// in the clear.
fn drop_message(_msg_text: &str) -> String {
    serde_json::to_string(&ErrorMessage {
        msg_type: "error".into(),
        code: "delivery_failed".into(),
        message: "The message could not be encrypted for this device.".into(),
        server_version: Some(PROTOCOL_VERSION),
    })
    .expect("ErrorMessage serializes")
}

pub struct WsServer {
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    ctx: WsContext,
    app_handle: Option<tauri::AppHandle>,
}

impl WsServer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        addr: String,
        sync_engine: Arc<RwLock<SyncEngine>>,
        encryption: Arc<EncryptionManager>,
        file_engine: Arc<FileTransferEngine>,
        token_store: Arc<TokenStore>,
        storage: Arc<Storage>,
        automation_engine: Arc<RwLock<automation::AutomationEngine>>,
        audio_stream: Arc<AudioStream>,
        device_id: Arc<String>,
        route_keys: Arc<crate::relay::DeviceRouteKeys>,
    ) -> Self {
        let addr: SocketAddr = match addr.parse() {
            Ok(a) => a,
            Err(e) => {
                error!(
                    "Invalid WS address, falling back to {}: {}",
                    crate::WS_BIND_ADDR,
                    e
                );
                crate::WS_BIND_ADDR.parse().unwrap()
            }
        };
        let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
        let ws_to_device_id: WsToDeviceId = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown_tx, _) = broadcast::channel(1);
        let rate_limiter = Arc::new(RateLimiter::new(security::RateLimitConfig::default()));
        let per_type_limiter = Arc::new(PerTypeRateLimiter::new());

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(
                    "Failed to bind WS server on {} (another instance running?): {}. Server will start without listening.",
                    addr, e
                );
                let ctx = WsContext {
                    clients: clients.clone(),
                    ws_to_device_id: ws_to_device_id.clone(),
                    sync_engine,
                    encryption,
                    file_engine,
                    token_store,
                    rate_limiter,
                    per_type_limiter,
                    storage,
                    automation_engine,
                    audio_stream,
                    device_id: device_id.clone(),
                    route_keys: route_keys.clone(),
                    relay_tx: Arc::new(RwLock::new(None)),
                };
                return WsServer {
                    shutdown_tx: Some(shutdown_tx),
                    ctx,
                    app_handle: None,
                };
            }
        };
        info!("WebSocket server listening on {}", addr);

        let ctx = WsContext {
            clients: clients.clone(),
            ws_to_device_id: ws_to_device_id.clone(),
            sync_engine,
            encryption,
            file_engine,
            token_store,
            rate_limiter,
            per_type_limiter,
            storage,
            automation_engine,
            audio_stream,
            device_id: device_id.clone(),
            route_keys: route_keys.clone(),
            relay_tx: Arc::new(RwLock::new(None)),
        };

        let ctx_clone = ctx.clone();
        let shutdown_rx = shutdown_tx.subscribe();

        if let Ok(devices) = ctx.storage.get_all_devices().await {
            let mut engine = ctx.sync_engine.write().await;
            for device in devices {
                if device.status == "paired" {
                    let connected_client = ConnectedClient {
                        device_id: device.id,
                        device_name: device.name,
                        device_type: device.device_type,
                        shared_secret: device.shared_secret,
                        last_heartbeat: device.last_seen,
                        battery_level: device.battery,
                    };
                    engine.add_client(connected_client);
                }
            }
        }

        tokio::spawn(Self::accept_loop(listener, ctx_clone.clone(), shutdown_rx));

        // Start WSS (WebSocket Secure) listener on a second port for LAN TLS connections
        let wss_addr: SocketAddr = match format!("0.0.0.0:{}", crate::WSS_PORT).parse() {
            Ok(a) => a,
            Err(e) => {
                error!("Invalid WSS address: {}", e);
                return WsServer {
                    shutdown_tx: Some(shutdown_tx),
                    ctx,
                    app_handle: None,
                };
            }
        };

        match TcpListener::bind(&wss_addr).await {
            Ok(wss_listener) => {
                info!("WSS server listening on {}", wss_addr);
                match crate::tls::get_wss_acceptor() {
                    Ok(tls_acceptor) => {
                        let wss_shutdown_rx = shutdown_tx.subscribe();
                        tokio::spawn(Self::wss_accept_loop(
                            wss_listener,
                            tls_acceptor,
                            ctx_clone,
                            wss_shutdown_rx,
                        ));
                    }
                    Err(e) => {
                        error!(
                            "Failed to create WSS TLS acceptor: {}. WSS will not be available.",
                            e
                        );
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Failed to bind WSS on {} (port may be in use): {}. WSS will not be available.",
                    wss_addr, e
                );
            }
        }

        WsServer {
            shutdown_tx: Some(shutdown_tx),
            ctx,
            app_handle: None,
        }
    }

    pub fn set_app_handle(&mut self, app_handle: tauri::AppHandle) {
        self.app_handle = Some(app_handle);
    }

    async fn accept_loop(
        listener: TcpListener,
        ctx: WsContext,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, peer)) => {
                            info!("New connection from {}", peer);
                            let ctx = ctx.clone();
                            let client_id = uuid::Uuid::new_v4().to_string();
                            // SECURITY: a loopback peer is NOT trusted. It used
                            // to be registered as `local_desktop` right here,
                            // which made "can reach 127.0.0.1:9527" equivalent
                            // to "is paired" for every message type. The
                            // capability check now happens in `handle_client`.
                            //
                            // This listener is plaintext. It is not a substitute
                            // for the TLS listener: the `encrypted` envelope is
                            // hop encryption terminated by this process, not an
                            // end-to-end channel, so 9527 carries plaintext for
                            // every peer and every type. A LAN client that wants
                            // TLS dials WSS_PORT (9531). See ADR-0007.
                            tokio::spawn(Self::handle_client(stream, ctx, client_id, peer));
                        }
                        Err(e) => error!("Accept error: {}", e),
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Server shutting down");
                    break;
                }
            }
        }
    }

    async fn wss_accept_loop(
        listener: TcpListener,
        tls_acceptor: TlsAcceptor,
        ctx: WsContext,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((tcp_stream, peer)) => {
                            info!("New WSS connection from {}", peer);
                            let acceptor = tls_acceptor.clone();
                            let ctx = ctx.clone();
                            let client_id = uuid::Uuid::new_v4().to_string();
                            // Same as `accept_loop`: no implicit trust for
                            // loopback peers, on the TLS listener either.
                            tokio::spawn(async move {
                                match acceptor.accept(tcp_stream).await {
                                    Ok(tls_stream) => {
                                        Self::handle_client(tls_stream, ctx, client_id, peer).await;
                                    }
                                    Err(e) => {
                                        error!("WSS TLS handshake error from {}: {}", peer, e);
                                    }
                                }
                            });
                        }
                        Err(e) => error!("WSS accept error: {}", e),
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("WSS server shutting down");
                    break;
                }
            }
        }
    }

    /// Encrypt one outbound frame for the peer behind `client_id`, if that peer
    /// has a secret.
    ///
    /// Thin wrapper over [`Self::seal_for_device`]: it resolves the connection
    /// to a stable device id and applies the same rules. Both egresses go
    /// through the *same* function so the LAN socket writer and the relay route
    /// cannot drift apart — that drift is what left every directed frame
    /// travelling to an off-LAN phone in the clear while the LAN copy of the
    /// identical frame was sealed.
    async fn seal_for_peer(ctx: &WsContext, client_id: &str, msg_text: &str) -> String {
        match handlers::paired_device_id(ctx, client_id).await {
            Some(stable_id) => Self::seal_for_device(ctx, &stable_id, msg_text).await,
            // Unpaired: should be unreachable for anything but a reply to this
            // peer's own request, but never leak a fan-out if it happens.
            None => msg_text.to_string(),
        }
    }

    /// Encrypt one outbound frame for a **stable device id**, if that device
    /// has a secret.
    ///
    /// **Exactly one encryption layer, applied here and only here.** Two rules
    /// make that hold:
    ///
    /// 1. A frame that is *already* an `encrypted` envelope is passed through
    ///    untouched. `commands::pairing::send_encrypted_message` builds such an
    ///    envelope before calling `send_to`; re-encrypting it produced
    ///    `encrypted(encrypted(payload))`, which the mobile client peels exactly
    ///    one layer from and then dispatches the inner envelope to a handler it
    ///    has not registered — a silent drop.
    /// 2. A `pairing` frame is never wrapped, because it is what *establishes*
    ///    the secret; encrypting it would make it undecryptable at the peer.
    ///
    /// A peer with no shared secret — the desktop's own webview
    /// (`local_desktop`), which has no key pair, or a device id that is not
    /// paired at all — receives plaintext by necessity. That is sound
    /// because reaching the first state requires the per-launch capability, and
    /// because broadcasts are filtered on pairing, so a stranger never gets
    /// here.
    ///
    /// # Why the secret comes from `peer_secret` and not from `SyncEngine`
    ///
    /// It used to read `SyncEngine` directly, which is a **liveness** map:
    /// `handle_client` drops a device from it the moment its LAN socket closes.
    /// A phone that paired on the LAN and then moved networks is therefore
    /// absent from `SyncEngine` while still paired — so every frame routed to
    /// it over the relay fell through the line below and went out **in
    /// cleartext**. That was already true of directed `file/chunk` frames,
    /// base64 payload and all, and it would have become true of clipboard and
    /// SMS bodies the moment fan-out learned to reach that device.
    ///
    /// `peer_secret` falls back to the pairing registry, which outlives the
    /// socket, so the egress can seal for a device it cannot see a socket for.
    ///
    /// Keyed by device id rather than connection id because one caller has no
    /// connection: `send_to`'s relay egress addresses a device that is by
    /// definition not on this LAN.
    async fn seal_for_device(ctx: &WsContext, stable_id: &str, msg_text: &str) -> String {
        if message_type_is(msg_text, "pairing") || message_type_is(msg_text, "encrypted") {
            return msg_text.to_string();
        }
        let Some(shared_secret) = handlers::peer_secret(ctx, stable_id).await else {
            return msg_text.to_string();
        };
        match ctx.encryption.encrypt(&shared_secret, msg_text) {
            Ok((nonce, ciphertext)) => {
                let data_hex = hex::encode(ciphertext);
                let hmac_hex = ctx
                    .encryption
                    .generate_hmac(&shared_secret, &data_hex)
                    .unwrap_or_default();
                let envelope = EncryptedEnvelope {
                    msg_type: "encrypted".into(),
                    nonce: hex::encode(nonce),
                    hmac: hmac_hex,
                    data: data_hex,
                    source_device: Some(stable_id.to_string()),
                    protocol_version: Some(PROTOCOL_VERSION),
                };
                serde_json::to_string(&envelope).expect("EncryptedEnvelope serializes")
            }
            Err(e) => {
                // Fail closed: never fall back to plaintext for a peer we do
                // have a secret for.
                error!(
                    "Failed to encrypt outbound message for {}: {}",
                    stable_id, e
                );
                drop_message(msg_text)
            }
        }
    }

    /// Charge one relayed **binary** frame to the transport budget, and report
    /// whether it is within it.
    ///
    /// A v2 relay frame names its **recipient**, not its sender, so there is no
    /// device id to charge a per-sender budget against — the tag is verified
    /// against every paired device's key in turn by
    /// `unwrap_relay_binary_frame` precisely because the frame does not say who
    /// sent it. That is a property of the format, not a gap to paper over, so
    /// this charges the only subject the frame does name.
    ///
    /// It is not the same subject as the text arm on the same socket, which
    /// charges the *sender* because a text delivery does name one. So a socket
    /// carrying binary traffic has a transport budget and a text-only one does
    /// not — which is worth stating, because it used to charge neither.
    ///
    /// Split out of `spawn_relay_client`'s read loop rather than inlined there so
    /// the accounting is reachable by a test. The loop itself needs a live
    /// WebSocket, and "the relay transport is charged" is exactly the kind of
    /// invariant that is untestable and therefore untrue.
    async fn admit_relay_binary(ctx: &WsContext, bytes: &[u8]) -> bool {
        if !ctx
            .rate_limiter
            .check(Self::RELAY_CLIENT_ID, "", bytes.len() as u64)
            .await
        {
            warn!(
                "Transport budget exceeded on the relay socket — dropping \
                 {}-byte relayed binary frame",
                bytes.len()
            );
            return false;
        }
        true
    }

    async fn handle_client<S>(stream: S, ctx: WsContext, client_id: String, peer: SocketAddr)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        // Explicit limits: tungstenite's defaults allow a 64 MiB message and
        // an unbounded write buffer.
        let ws_stream = match accept_async_with_config(stream, Some(ws_read_limits())).await {
            Ok(ws) => ws,
            Err(e) => {
                error!("WebSocket handshake error from {}: {}", peer, e);
                return;
            }
        };

        // SECURITY: a connection is unpaired from the instant it is accepted.
        // Nothing about it grants an identity — not "it came from loopback",
        // not "it has a socket". The desktop's own webview proves possession of
        // the per-launch capability with a `pairing`/`local_auth` frame (see
        // `handle_local_auth`); until it does it is on exactly the same footing
        // as any other unauthenticated peer, which is the whole point. The
        // unpaired window is harmless: broadcasts are filtered on identity, so
        // nothing reaches the socket before the frame lands.
        if peer.ip().is_loopback() {
            debug!(
                "Loopback connection {client_id} stays unpaired until it presents the local capability"
            );
        }

        let (mut write, mut read) = ws_stream.split();
        let (tx, mut rx) = broadcast::channel(256);

        {
            let mut clients_lock = ctx.clients.write().await;
            clients_lock.insert(client_id.clone(), tx.clone());
        }

        info!("Client registered: {}", client_id);

        let ctx_clone = ctx.clone();
        let my_id = client_id.clone();
        let last_pong_time = Arc::new(tokio::sync::Mutex::new(Instant::now()));
        let last_pong_reader = last_pong_time.clone();
        let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(25));
        let broadcast_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = ping_interval.tick() => {
                        // Check if client has exceeded idle timeout (60s without pong/message)
                        {
                            let last = last_pong_reader.lock().await;
                            if last.elapsed() > std::time::Duration::from_secs(60) {
                                warn!("Client {} idle timeout (no pong in 60s), closing", my_id);
                                break;
                            }
                        }
                        let ping = serde_json::to_string(&Ping::new()).expect("Ping serializes");
                        if let Err(e) = write.send(Message::Text(ping.into())).await {
                            error!("Failed to send ping to {}: {}", my_id, e);
                            break;
                        }
                    }
                    msg_res = rx.recv() => {
                        match msg_res {
                            Ok(msg_text) => {
                                let payload = Self::seal_for_peer(&ctx_clone, &my_id, &msg_text).await;
                                if let Err(e) = write.send(Message::Text(payload.into())).await {
                                    error!("Send error to {}: {}", my_id, e);
                                    break;
                                }
                            }
                            Err(_) => {
                                break;
                            }
                        }
                    }
                }
            }
        });

        loop {
            match tokio::time::timeout(tokio::time::Duration::from_secs(60), read.next()).await {
                Ok(Some(Ok(msg))) => {
                    // Update last activity time on any received message
                    {
                        let mut last = last_pong_time.lock().await;
                        *last = Instant::now();
                    }
                    Self::admit_frame(&ctx, &client_id, msg).await;
                }
                Ok(Some(Err(e))) => {
                    warn!("WebSocket read error for {}: {}", client_id, e);
                    break;
                }
                Ok(None) => {
                    break;
                }
                Err(_) => {
                    warn!("Client {} timed out (no pong/message in 60s)", client_id);
                    break;
                }
            }
        }

        ctx.clients.write().await.remove(&client_id);
        let stable_id = ctx
            .ws_to_device_id
            .write()
            .await
            .remove(&client_id)
            .unwrap_or_else(|| client_id.clone());
        ctx.rate_limiter.remove_client(&client_id).await;
        ctx.per_type_limiter.remove_client(&client_id).await;
        ctx.sync_engine.write().await.remove_client(&stable_id);
        let disc_ctx = TriggerContext {
            event: TriggerEvent::DeviceDisconnect,
            source_device_id: stable_id.clone(),
            battery_level: None,
            wifi_ssid: None,
            app_package: None,
        };
        Self::evaluate_device_triggers(&disc_ctx, &ctx).await;
        broadcast_task.abort();
        info!("Client disconnected: {} (stable: {})", client_id, stable_id);
    }

    pub(crate) async fn evaluate_device_triggers(event_ctx: &TriggerContext, ctx: &WsContext) {
        let allowlist = security::current_command_allowlist();
        let engine = ctx.automation_engine.read().await;
        for rule in engine.get_rules() {
            if !rule.enabled {
                continue;
            }
            if engine.evaluate_trigger(&rule.trigger, event_ctx).await {
                if !automation::is_desktop_executable(&rule.action) {
                    continue;
                }
                // SECURITY: the allowlist is passed here, not `None`. Every
                // production call site used to pass `None`, which made
                // `CommandAllowlist` (and its ~40 tests) dead code while a
                // remotely-supplied `RunShellCommand` rule executed unchecked.
                let log = automation::execute_action_with_allowlist(&rule.action, Some(&allowlist));
                let _ = ctx
                    .storage
                    .log_automation_execution(
                        &rule.id,
                        &automation::trigger_tag(&rule.trigger),
                        log.timestamp,
                        log.success,
                        log.message.as_deref(),
                    )
                    .await;
                info!("Automation device trigger fired for rule: {}", rule.name);
            }
        }
    }

    /// Resolve the shared secret to open an `encrypted` envelope with, or `None`
    /// if this socket is not a paired peer.
    ///
    /// Order of trust, highest first:
    ///
    /// 1. **The connection's own identity** (`ws_to_device_id`). The hub wrote this
    ///    at pairing time, from a one-time token, so it cannot be spoofed.
    /// 2. **The peer's claimed `source_device`.** Consulted *only* when the socket
    ///    has no entry of its own — which is an already-paired peer reconnecting
    ///    over a fresh socket, before it re-pairs.
    ///
    /// The claim is deliberately never allowed to override the connection
    /// identity. A mapped socket that names a different device is reaching for
    /// someone else's shared secret; it would fail the HMAC anyway, and letting the
    /// claim win would mean one socket has as many identities as it cares to
    /// claim. One socket, one identity.
    ///
    /// Step 2 is what rescues a peer that has not been told its own id. The peer
    /// cannot derive one — the desktop picks it — and it is the only value stamped
    /// into `source_device`. When a desktop predates assigned ids, or the field is
    /// simply wrong, this is the only path that still works; without it the frame
    /// is dropped here and the peer sees nothing but a silent failure.
    async fn resolve_sender_secret(
        ctx: &WsContext,
        client_id: &str,
        claimed_id: Option<&str>,
    ) -> Option<String> {
        // A relayed message arrives already attributed: `client_id` is the
        // device the relay authenticated, not a connection id. It is resolved
        // directly, and it is not overridable by the envelope's own
        // `source_device` claim — a device that is not paired yields no secret
        // at all.
        //
        // This tier comes first because a relayed id is a stronger statement
        // than anything the socket itself can say: it is what the relay
        // established during `relay_auth` and re-checked against the signature.
        //
        // `peer_secret` rather than a bare `SyncEngine` lookup, which is the
        // fourth read of a *liveness* map as if it were a trust registry. It is
        // why a relay-only phone could receive a frame this desktop correctly
        // routed to it and then fail to decrypt it here — silently, one line
        // after the relay had authenticated the sender.
        if let Some(secret) = handlers::peer_secret(ctx, client_id)
            .await
            .filter(|s| !s.is_empty())
        {
            return Some(secret);
        }

        // Otherwise this is a LAN socket, and its identity is whatever the hub
        // wrote at pairing time. Read and released *before* any registry
        // lookup: holding a `sync_engine` guard across this read is the inverse
        // of the order the fan-out uses, and harmless only while every holder is
        // a reader.
        let connection_id = ctx.ws_to_device_id.read().await.get(client_id).cloned();
        let by_connection = match connection_id {
            Some(id) => handlers::peer_secret(ctx, &id).await,
            None => None,
        };
        if let Some(secret) = by_connection {
            return Some(secret);
        }

        // The claim is consulted only when the socket's own identity did not
        // resolve. A *live* connection identity always wins, so one socket has
        // exactly one identity and cannot borrow another device's secret by
        // naming it. A *dangling* one — the hub still has a mapping but the
        // device row is gone, e.g. the database was reset — falls through to
        // the claim, because refusing there would strand a peer that has since
        // legitimately re-paired.
        //
        // Note this is a secret *lookup*, not an authentication decision: the
        // caller still has to produce a valid HMAC over the ciphertext with
        // whatever secret comes back. Resolving to a device you are not does
        // not let you read its traffic.
        match claimed_id {
            Some(id) => handlers::peer_secret(ctx, id).await,
            None => None,
        }
    }

    /// The client id the outbound relay connection is dispatched under.
    ///
    /// It is deliberately not a device id and deliberately not in
    /// `ws_to_device_id`: it names the *transport*, not a peer. A relayed message
    /// carries its own authenticated sender in a `relay_delivery` envelope, and
    /// that sender — not this id — is what the rest of the hub sees.
    const RELAY_CLIENT_ID: &str = "relay_server";

    /// Unwrap a `relay_delivery` envelope into `(sender, payload)`.
    ///
    /// Returns `None` for anything that is not a well-formed envelope, and for
    /// any sender this desktop has not paired. The `from_device_id` is checked
    /// against the pairing registry rather than trusted: the relay stamped it,
    /// but a field that arrives over a socket is attacker-influenced, and the
    /// cost of being wrong is impersonating a paired device.
    async fn unwrap_relay_delivery(text: &str, ctx: &WsContext) -> Option<(String, Value)> {
        let msg: Value = serde_json::from_str(text).ok()?;
        if msg.get("type").and_then(|v| v.as_str()) != Some("relay_delivery") {
            return None;
        }

        let from = msg
            .get("from_device_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())?;

        // The relay only routes to a device it authenticated, and only a paired
        // device has a route key at all, so this is nearly always redundant. It
        // is still the difference between "the relay said so" and "the desktop
        // verified it", and an unpaired sender must never reach a handler.
        //
        // Resolved through `peer_secret`, which falls back to the pairing
        // registry. It used to read `SyncEngine`, which meant the refusal below
        // fired for the one device this whole mechanism exists for: a phone that
        // paired on the LAN and then moved networks had been dropped from
        // `SyncEngine` by its own disconnect, so every frame the desktop
        // correctly routed to it was thrown away on arrival. The egress fix and
        // this have to move together — an egress that delivers into a socket
        // whose inbound is refused is unobservable.
        let trusted = handlers::peer_secret(ctx, from)
            .await
            .is_some_and(|secret| !secret.is_empty());
        if !trusted {
            warn!("Refusing a relay_delivery from unpaired device {from}");
            return None;
        }

        let payload = msg.get("payload")?.clone();
        Some((from.to_string(), payload))
    }

    /// Charge one inbound frame to the connection's transport budget, and handle
    /// it if it is within it.
    ///
    /// **This is the only place the global (`RateLimiter`) budget is charged for a
    /// frame arriving on a LAN socket**, because it is the only place a LAN socket
    /// is read. `spawn_relay_client`'s read loop calls `handle_message` directly
    /// and so never reaches this function; a relayed *text* frame is charged
    /// against the **device** that sent it, in `handle_message`'s relay arm, or
    /// against the transport when no device can be named for it. Those are the two
    /// arms of one `match` and the first returns, so a given frame is charged
    /// exactly once either way — the accounting is per *subject*, not per hop.
    ///
    /// It used to be charged here *and* again inside [`Self::dispatch`], against
    /// the same `client_id`-keyed bucket, so every inbound text frame cost two
    /// counts and the real ceiling was half of `max_messages`. With the old
    /// default of 100 per 10 s that is ~5 msg/s — and screen mirroring (15–30
    /// frames/s), audio (10–50/s) and a 64 KiB-chunked file push (~47/s) all
    /// died inside it.
    ///
    /// # Why the reader, and not `dispatch`
    ///
    /// Not merely because it was there first. By the time a frame reaches this
    /// function its bytes are already in the server's memory and about to be
    /// JSON-parsed, HMAC-verified and decrypted — that work is what the limiter
    /// exists to bound, and it happens before *anything* can reject the frame.
    /// The charge therefore sits at the boundary where the cost starts.
    ///
    /// Keeping it here is also what makes **rejected frames cost something**.
    /// `dispatch` is only reached by frames that parsed, passed
    /// `validate_message`, the auth gate, the type allowlist and `settings_gate`;
    /// a peer flooding with unparsable JSON, a forged HMAC or an unauthenticated
    /// `file/chunk` never gets there. A limiter charged only in `dispatch` would
    /// hand exactly that peer an unlimited supply of free parse attempts.
    /// Charged at the reader, one frame costs one count whatever becomes of it
    /// afterwards — which is exactly what `security::RateLimiter`'s tests assert
    /// about its bucket.
    ///
    /// Bytes are charged here too, and only here: a frame's length is known
    /// before parsing and never changes, and the count is what `dispatch` was
    /// double-charging.
    ///
    /// Note that `check` charges before it decides, so a frame refused here has
    /// still been billed. That is deliberate — see [`security::RateLimiter`].
    ///
    /// Returns `false` if the frame was dropped by the budget. `Ping`/`Close`
    /// carry no payload and are answered by tungstenite, so they are neither
    /// charged nor refused here.
    async fn admit_frame(ctx: &WsContext, client_id: &str, msg: Message) -> bool {
        match msg {
            Message::Text(text) => {
                let text = text.as_str();
                let len = text.len();
                if !ctx.rate_limiter.check(client_id, "", len as u64).await {
                    warn!(
                        "Transport budget exceeded by {} ({} bytes in window) — dropping frame",
                        client_id,
                        ctx.rate_limiter.window_bytes(client_id).await
                    );
                    return false;
                }
                let preview: String = text.chars().take(120).collect();
                info!("Received from {}: {}...", client_id, preview);
                Self::handle_message(text, client_id, ctx).await;
                true
            }
            Message::Binary(bytes) => {
                let len = bytes.len();
                if !ctx.rate_limiter.check(client_id, "", len as u64).await {
                    warn!(
                        "Transport budget exceeded by {} — dropping {len}-byte binary frame",
                        client_id
                    );
                    return false;
                }
                info!("Received binary frame from {}: {} bytes", client_id, len);
                handlers::files::handle_binary_message(bytes.to_vec(), client_id, ctx).await;
                true
            }
            Message::Pong(_) => {
                info!("Received pong from {}", client_id);
                true
            }
            _ => true,
        }
    }

    async fn handle_message(text: &str, client_id: &str, ctx: &WsContext) {
        // A relayed message is not this socket's traffic: it is another
        // device's, carried here. Unwrap it and re-enter as the sender the
        // relay authenticated, so the auth gate and the handlers see a real
        // paired identity instead of a transport id that is in no registry.
        //
        // Without this the whole feature is dead on arrival: `relay_server` is
        // never a trusted peer, so every relayed message was refused with
        // `not_authenticated` and the phone's notification simply vanished.
        if client_id == Self::RELAY_CLIENT_ID {
            match Self::unwrap_relay_delivery(text, ctx).await {
                Some((sender, payload)) => {
                    // The transport budget, charged against the **device** that
                    // sent this frame rather than the socket that carried it.
                    //
                    // Every other inbound frame is charged in `admit_frame`,
                    // ahead of any work, because the subject of the budget there
                    // is the socket and a socket is known before a byte is
                    // looked at. Here the subject has to be the device, and the
                    // device is named inside the frame — so this is the first
                    // point at which it is both known and checked against the
                    // pairing registry (`unwrap_relay_delivery` refuses an
                    // unpaired sender outright).
                    //
                    // # Why this is a charge and not a split
                    //
                    // The audit recorded this as "every phone behind the relay
                    // shares one bucket keyed on `"relay_server"`, so one file
                    // transfer throttles every other phone". The mechanism was
                    // wrong and the defect was worse. `admit_frame` has exactly
                    // one caller — `handle_client`, the LAN reader — and this
                    // read loop calls `handle_message` directly. So the
                    // `"relay_server"` bucket was **never created**: relayed
                    // frames were charged no count *and no bytes* by the
                    // transport limiter, by any phone, at all. The commit
                    // message's claim that "the relay socket was charged once in
                    // `admit_frame`" described an accounting the code did not
                    // have.
                    //
                    // What is left is the per-type limiter, which was already
                    // per-device because this same re-entry dispatches under the
                    // sender. So one phone *could* not throttle another — it
                    // could also not be throttled at all. `max_bytes`, the field
                    // `security::RateLimitConfig` documents as "the budget that
                    // bounds CPU", did not exist on this path.
                    //
                    // Charging per sender is therefore strictly *more*
                    // accounting than before, not a repartition of an existing
                    // charge. The transport still keeps its own budget below, for
                    // frames no device can be named for.
                    if !ctx.rate_limiter.check(&sender, "", text.len() as u64).await {
                        warn!(
                            "Transport budget exceeded by relayed device {} ({} bytes in \
                             window) — dropping frame",
                            sender,
                            ctx.rate_limiter.window_bytes(&sender).await
                        );
                        return;
                    }
                    let forwarded = payload.to_string();
                    // Not a recursive call: an unwrapped delivery is dispatched
                    // once, as the sender, and a second envelope inside it is
                    // refused like any other unexpected type. A loop here would
                    // also be an unbounded nesting primitive.
                    return Self::dispatch(&forwarded, &sender, ctx).await;
                }
                None => {
                    // Not a delivery, or a sender we do not trust. `ping`/`pong`
                    // still have to work on this socket, so fall through to the
                    // normal dispatcher, which will refuse anything else.
                    //
                    // No device can be named, so this frame is charged to the
                    // transport instead. That keeps "a frame this desktop refuses
                    // still costs something" true on the relay path too — which
                    // matters most precisely here, because these are the frames
                    // that never reach a handler at all. The relay applies its
                    // own per-connection limit to this socket before a byte
                    // arrives, so the parse this charge follows is bounded.
                    let _ = ctx
                        .rate_limiter
                        .check(Self::RELAY_CLIENT_ID, "", text.len() as u64)
                        .await;
                }
            }
        }

        Self::dispatch(text, client_id, ctx).await
    }

    /// The body of [`Self::handle_message`], once the transport has been peeled
    /// off: everything below this point is about the *message*, not about which
    /// socket carried it.
    async fn dispatch(text: &str, client_id: &str, ctx: &WsContext) {
        let msg: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to parse message from {}: {}", client_id, e);
                return;
            }
        };

        if let Some(version) = msg.get("protocol_version").and_then(|v| v.as_u64())
            && version > u64::from(PROTOCOL_VERSION)
        {
            warn!(
                "Unsupported protocol version {} from {}",
                version, client_id
            );
            let err_resp = ErrorMessage {
                msg_type: "error".into(),
                code: "unsupported_protocol_version".into(),
                message: format!(
                    "Server supports protocol_version {}, got {}",
                    PROTOCOL_VERSION, version
                ),
                server_version: Some(PROTOCOL_VERSION),
            };
            let err_resp = serde_json::to_string(&err_resp).expect("ErrorMessage serializes");
            answer(ctx, client_id, &err_resp).await;
            return;
        }

        let raw_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Deliberately **not** charged to the global limiter here. This frame was
        // already charged its one count and its bytes by the reader
        // (`Self::admit_frame`); charging again against the same `client_id`
        // bucket halved the real ceiling and killed every stream in the app.
        //
        // The reason that site is the right one is not only that it is first:
        // everything below can refuse this frame — the JSON parse, the protocol
        // version, the HMAC, `validate_message`, the auth gate, the type
        // allowlist, `settings_gate` — and a peer that only ever sends refused
        // frames would never reach a limiter placed here at all. Refused frames
        // are charged at the reader instead, so the budget cannot be evaded by
        // making the server reject your traffic.
        //
        // A relayed frame's *sender* is charged nothing here, and that is also
        // deliberate: the transport budget is charged in exactly one place per
        // frame, and on the relay that place is `handle_message`'s relay arm —
        // against the sender's device id, once the delivery has been unwrapped
        // and the sender checked against the pairing registry. Inventing a
        // second transport charge here would double-bill the same frame against
        // the same device, which is the defect `admit_frame`'s doc describes.

        let processed_msg = if raw_type == "encrypted" {
            let claimed_id = msg.get("source_device").and_then(|v| v.as_str());

            let Some(shared_secret) = Self::resolve_sender_secret(ctx, client_id, claimed_id).await
            else {
                warn!(
                    "Received encrypted message but no shared secret found for {} (claimed {})",
                    client_id,
                    claimed_id.unwrap_or("<none>")
                );
                return;
            };

            let nonce_hex = msg.get("nonce").and_then(|v| v.as_str()).unwrap_or("");
            let hmac_hex = msg.get("hmac").and_then(|v| v.as_str()).unwrap_or("");
            let data_hex = msg.get("data").and_then(|v| v.as_str()).unwrap_or("");

            if !ctx
                .encryption
                .verify_hmac(&shared_secret, data_hex, hmac_hex)
            {
                warn!("HMAC verification failed for message from {}", client_id);
                return;
            }

            let nonce = match hex::decode(nonce_hex) {
                Ok(n) => n,
                Err(_) => return,
            };
            let data = match hex::decode(data_hex) {
                Ok(d) => d,
                Err(_) => return,
            };

            match ctx.encryption.decrypt(&shared_secret, &nonce, &data) {
                Ok(decrypted) => match serde_json::from_str(&decrypted) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("Failed to parse decrypted message: {}", e);
                        return;
                    }
                },
                Err(e) => {
                    warn!("Decryption failed: {}", e);
                    return;
                }
            }
        } else {
            msg
        };

        if let Err(e) = security::validate_message(&processed_msg) {
            warn!("Invalid message from {}: {}", client_id, e);
            // The peer used to get silence, which is indistinguishable from a
            // dropped connection. Answer with a protocol error carrying a
            // stable code so the sender can log *why*.
            send_error(ctx, client_id, "invalid_message", &e.to_string()).await;
            return;
        }

        let msg_type = processed_msg
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let action = processed_msg
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Require pairing authentication for every message type except the
        // handshake itself.
        //
        // `pairing` is exempt because it *is* the authentication step
        // (`request`/`accept` carry a one-time token, `local_auth` carries the
        // per-launch capability), and `ping`/`pong` must work before pairing so
        // a client can probe reachability. `discovery` is **not** exempt: it
        // answers with the desktop's own identity, name, OS, version and ports,
        // which is reconnaissance, and its `remove` action mutates peer state.
        // It used to be exempt, and because `discovery` was also missing from
        // `VALID_TYPES` the exemption was moot — the packet died in
        // `validate_message` anyway. Both are fixed together here.
        let requires_auth = !matches!(
            (msg_type, action),
            ("pairing", _) | ("ping", _) | ("pong", _)
        );
        if requires_auth {
            let is_paired = handlers::is_trusted_peer(ctx, client_id).await;
            if !is_paired {
                warn!(
                    "Rejected unauthenticated {} message from {}",
                    msg_type, client_id
                );
                send_error(
                    ctx,
                    client_id,
                    "not_authenticated",
                    "Device must complete pairing before sending this message type",
                )
                .await;
                return;
            }
        }

        // Per-message-type rate limiting — prevents high-throughput types
        // (e.g. file/chunk) from starving low-volume types (e.g. pairing).
        //
        // The action matters, and not only for accounting: a control action (a
        // stream's own `stop`, a transfer's `complete`, a pairing's `accept`) is
        // charged to a bucket the stream's own frames cannot exhaust, so a peer
        // can always stop what it started. See `security::is_control_action`.
        if !ctx
            .per_type_limiter
            .check_type_action_limit(client_id, msg_type, action)
            .await
        {
            warn!(
                "Per-type rate limited: {} (type: {}, action: {})",
                client_id, msg_type, action
            );
            let err_resp = ErrorMessage {
                msg_type: "error".into(),
                code: "rate_limited".into(),
                message: format!("Rate limit exceeded for message type '{}'", msg_type),
                server_version: None,
            };
            let err_resp = serde_json::to_string(&err_resp).expect("ErrorMessage serializes");
            // `answer`, not a bare `clients.get(client_id)`: a relayed sender's
            // id is a device id, so the bare lookup missed and the phone was
            // throttled into silence with nothing to log on its side. A sender
            // with no socket here has the relay as its only route back.
            if !answer(ctx, client_id, &err_resp).await {
                warn!(
                    "Rate-limited {} (type: {}) could not be answered: no socket on \
                     this desktop and no route to it through the relay",
                    client_id, msg_type
                );
            }
            return;
        }

        // User-facing settings gates, evaluated once for the whole message before
        // any handler runs. `Some((code, detail))` means the message must be
        // dropped: it is logged and answered with an `error` frame.
        if let Some((code, detail)) = settings_gate(ctx, &processed_msg, msg_type, action).await {
            reject_due_to_setting(ctx, client_id, code, detail).await;
            return;
        }

        match (msg_type, action) {
            ("pairing", "request") => {
                handlers::pairing::handle_pairing_request(processed_msg, client_id, ctx).await;
            }
            ("pairing", "accept") => {
                handlers::pairing::handle_pairing_accept(processed_msg, client_id, ctx).await;
            }
            ("pairing", "local_auth") => {
                handlers::pairing::handle_local_auth(processed_msg, client_id, ctx).await;
            }
            ("notification", "post") => {
                handlers::notifications::handle_notification_post(processed_msg, client_id, ctx)
                    .await;
            }
            ("notification", "dismiss") => {
                handlers::notifications::handle_notification_dismiss(processed_msg, client_id, ctx)
                    .await;
            }
            ("notification", "reply") => {
                handlers::notifications::handle_notification_reply(processed_msg, client_id, ctx)
                    .await;
            }
            ("clipboard", "sync") => {
                // Persist the *received* content before relaying it. This arm
                // used to be a bare fan-out, and `clipboard_history` therefore
                // only ever got a row from the `sync_clipboard` Tauri command —
                // which the webview never calls, because `useClipboard` posts a
                // raw `clipboard/sync` frame instead. The history table was
                // consequently always empty: the UI showed "Your clipboard is
                // empty" permanently, the Clear button could not render, and
                // the dock badge never left 0.
                persist_inbound_clipboard(ctx, client_id, &processed_msg).await;
                broadcast_to_others(ctx, client_id, &processed_msg.to_string()).await;
            }
            ("file", "request") => {
                handlers::files::handle_file_request(processed_msg, client_id, ctx).await;
            }
            ("file", "accept") => {
                handlers::files::handle_file_accept(processed_msg, client_id, ctx).await;
            }
            ("file", "chunk") => {
                handlers::files::handle_file_chunk(processed_msg, client_id, ctx).await;
            }
            ("file", "progress") => {
                handlers::files::handle_file_progress(processed_msg, client_id, ctx).await;
            }
            ("file", "complete") => {
                handlers::files::handle_file_complete(processed_msg, client_id, ctx).await;
            }
            ("file", "cancel") => {
                handlers::files::handle_file_cancel(processed_msg, client_id, ctx).await;
            }
            ("file", "resume") => {
                handlers::files::handle_file_resume(processed_msg, client_id, ctx).await;
            }
            ("sms", _) => {
                broadcast_to_others(ctx, client_id, &processed_msg.to_string()).await;
            }
            ("call", _) => {
                broadcast_to_others(ctx, client_id, &processed_msg.to_string()).await;
            }
            ("audio", _) => {
                handlers::audio::handle_audio(processed_msg, client_id, ctx).await;
            }
            ("status", _) => {
                handle_status_update(processed_msg, client_id, ctx).await;
            }
            // ── Automation ────────────────────────────────────────────────────
            //
            // The `is_paired` re-checks that used to be duplicated into five
            // arms are gone: the single auth gate above has already refused an
            // unpaired client, and duplicating it per arm is how the earlier
            // loopback auto-pairing slipped past. What is *new* here is the
            // shell-command gate, which `validate_rule` does not perform.
            ("automation", "rule") | ("automation", "") => {
                if let Err(reason) =
                    shell_rule_gate(&processed_msg, &security::current_command_allowlist())
                {
                    warn!("Automation rule rejected ({}): {}", client_id, reason);
                    send_error(
                        ctx,
                        client_id,
                        "command_not_allowed",
                        &format!(
                            "{reason}. Add it to Settings → Advanced → Allowed Commands first."
                        ),
                    )
                    .await;
                    return;
                }
                handlers::auto_rules::handle_automation_rule(processed_msg, ctx).await;
            }
            ("automation", "delete") => {
                handlers::auto_rules::handle_automation_delete(processed_msg, ctx).await;
            }
            ("automation", "sync") => {
                // `sync` persists an array of rules, so it needs the same gate.
                if let Err(reason) =
                    shell_rule_gate(&processed_msg, &security::current_command_allowlist())
                {
                    warn!("Automation sync rejected ({}): {}", client_id, reason);
                    send_error(ctx, client_id, "command_not_allowed", &reason).await;
                    return;
                }
                handlers::auto_rules::handle_automation_sync(processed_msg, ctx).await;
            }
            ("automation", "triggered") => {
                // SECURITY: `handle_automation_triggered` calls
                // `execute_action(&rule.action)` — i.e. `None`, no allowlist —
                // so the *execution* side of the shell gate has to be enforced
                // here, at the only dispatch point that leads to it. Without
                // this, a rule persisted by an older build (or before the
                // allowlist existed) would still execute its command the moment
                // a peer asked it to, even with an empty allowlist.
                if let Err(reason) =
                    triggered_rule_gate(&processed_msg, ctx, &security::current_command_allowlist())
                        .await
                {
                    warn!("Automation trigger rejected ({}): {}", client_id, reason);
                    send_error(ctx, client_id, "command_not_allowed", &reason).await;
                    return;
                }
                handlers::auto_rules::handle_automation_triggered(processed_msg, ctx).await;
            }
            ("automation", other) => {
                warn!("Automation: unknown action '{}' from {}", other, client_id);
                send_error(
                    ctx,
                    client_id,
                    "unknown_action",
                    &format!("Unknown automation action '{other}'"),
                )
                .await;
            }

            ("notification", "mark_read") => {
                handlers::notifications::handle_notification_mark_read(
                    processed_msg,
                    client_id,
                    ctx,
                )
                .await;
            }
            ("screen_mirror", _) => {
                handlers::screen_mirror::handle_screen_mirror(processed_msg, client_id, ctx).await;
            }
            ("remote_input", _) => {
                handlers::remote_input::handle_remote_input(processed_msg, client_id, ctx).await;
            }
            ("discovery", "announce") => {
                handle_discovery_announce(processed_msg, client_id, ctx).await;
            }
            ("discovery", "remove") => {
                handle_discovery_remove(processed_msg, client_id, ctx).await;
            }
            ("pong", _) => {}
            ("ping", _) => {
                let pong = serde_json::to_string(&Pong::new()).expect("Pong serializes");
                // Also how a relay-only device gets its `pong`: `handle_message`
                // dispatches its delivery under the device id, which is in no
                // connection map.
                answer(ctx, client_id, &pong).await;
            }
            // ── Relay control plane ───────────────────────────────────────────
            //
            // Only reachable on the outbound relay connection. These used to be
            // rejected by `validate_msg_type` before dispatch, so a `relay_auth`
            // the relay refused was indistinguishable from a dropped socket, and
            // the reconnect loop just backed off and tried again.
            ("relay_auth_ok", _) => {
                info!("Relay accepted this desktop's relay_auth");
            }
            ("relay_auth_rejected", _) => {
                let reason = processed_msg
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unspecified");
                error!(
                    "Relay refused this desktop's relay_auth ({reason}). The stored relay token \
                     is wrong or was rotated; messages to phones off the LAN will not be routed."
                );
            }
            ("error", _) => {
                let code = processed_msg
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let detail = processed_msg
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("no detail");
                warn!("Relay refused a message from this desktop: {code} ({detail})");
            }
            // `tv` and `watch` are accepted by `validate_msg_type` but have no
            // handler anywhere in the workspace — dead protocol surface that
            // silently swallows frames. Report rather than delete: the mobile
            // app is not mine to change. Answering with an explicit error is
            // strictly better than the current `warn!`-and-drop.
            (other_type, other_action) => {
                warn!(
                    "Unknown message type: {} action: {}",
                    other_type, other_action
                );
                send_error(
                    ctx,
                    client_id,
                    "unsupported_message",
                    &format!(
                        "Message type '{other_type}' is accepted by the protocol \
                         but has no handler on the desktop hub"
                    ),
                )
                .await;
            }
        }
    }

    /// Fan a message out to every **paired** connection, and to every paired
    /// device that has no socket here at all.
    ///
    /// Unpaired connections are skipped. Every accepted socket used to be in
    /// `ctx.clients` unconditionally, and because the per-socket encryption
    /// only fires when `ws_to_device_id` yields a peer with a shared secret, an
    /// unpaired socket was handed the **raw plaintext** of every broadcast.
    ///
    /// The relay arm is [`Self::fan_out_to_relay`] and it runs *first*, outside
    /// both guards, because it awaits and this loop holds `clients` and
    /// `ws_to_device_id` readers for its whole duration. This is the path every
    /// desktop-originated frame takes — `commands::notifications`'s
    /// `forward_to_peers_checked` reaches it for a dismissal, a reply and a
    /// clipboard sync — so without the arm the desktop's own notifications and
    /// clipboard reached the LAN and nothing else.
    ///
    /// `exclude` is `None`: nothing here has a sender, because a broadcast
    /// originates on the desktop itself rather than from a peer.
    pub(crate) async fn broadcast(&self, message: String) {
        Self::fan_out_to_relay(&self.ctx, None, &message).await;

        let ids: Vec<String> = self.ctx.clients.read().await.keys().cloned().collect();
        for id in ids {
            if !handlers::has_identity(&self.ctx, &id).await {
                debug!("Skipping broadcast to unpaired connection {}", id);
                continue;
            }
            let clients = self.ctx.clients.read().await;
            if let Some(tx) = clients.get(&id)
                && let Err(e) = tx.send(message.clone())
            {
                warn!("Failed to broadcast to {}: {}", id, e);
            }
        }
    }

    /// Send to the device with this stable id, whichever connection carries it.
    ///
    /// `ctx.clients` is keyed by a **per-connection UUID**, not by device id, so
    /// a direct `clients.get(device_id)` only ever succeeded when the two
    /// happened to be equal — i.e. essentially never, and only via the relay.
    /// This resolves `device_id -> ws_client_id` through the pairing registry
    /// first (the same fallback `disconnect_client` already used), and falls
    /// back to the relay.
    pub async fn send_to(&self, device_id: &str, message: String) -> bool {
        let clients = self.ctx.clients.read().await;
        // 1. The id is already a connection id.
        if let Some(client_tx) = clients.get(device_id) {
            if let Err(e) = client_tx.send(message.clone()) {
                warn!("Failed to send to {}: {}", device_id, e);
                return false;
            }
            return true;
        }
        // 2. The id is a stable device id: resolve its connection.
        let ws_id = {
            let ws_map = self.ctx.ws_to_device_id.read().await;
            ws_map
                .iter()
                .find(|(_, v)| v.as_str() == device_id)
                .map(|(k, _)| k.clone())
        };
        if let Some(ws_id) = ws_id
            && let Some(client_tx) = clients.get(&ws_id)
        {
            if let Err(e) = client_tx.send(message.clone()) {
                warn!(
                    "Failed to send to device {} (ws {}): {}",
                    device_id, ws_id, e
                );
                return false;
            }
            return true;
        }
        drop(clients);

        // 3. Not connected directly — try the relay.
        //
        // The sender is cloned out and the guard dropped before any await, for
        // the reason given below the clone.
        let Some(relay_tx) = self.ctx.relay_tx.read().await.clone() else {
            return false;
        };
        // Signing uses this desktop's *own* route key, never the relay
        // token. The token authenticates the connection; the key proves
        // which device is speaking. Conflating the two let anyone who had
        // seen the token route as anyone, and left `from_device_id`
        // unsigned, so the relay could not tell a spoof from the truth.
        let from_device_id = self.ctx.device_id.as_str().to_string();
        let Some(route_key) = self.ctx.route_keys.signing_key(&from_device_id) else {
            error!(
                "Cannot route through the relay: no route key is registered for this \
                 desktop ({from_device_id}). The device registry has not been read yet."
            );
            return false;
        };

        // Sealed for the recipient exactly as the LAN writer seals it.
        let Some(route) =
            Self::build_sealed_route(&self.ctx, device_id, &message, &route_key).await
        else {
            return false;
        };
        if let Err(e) = relay_tx.send(route).await {
            warn!("Failed to route message through relay: {}", e);
            return false;
        }
        true
    }

    /// Thin wrapper over [`Self::build_sealed_route`], which needs the context
    /// and nothing else, so the pre-existing egress tests can drive it through a
    /// `WsServer` without restating the context argument at each call site.
    #[cfg(test)]
    async fn sealed_relay_route(
        &self,
        to_device_id: &str,
        message: &str,
        route_key: &[u8],
    ) -> String {
        Self::build_sealed_route(&self.ctx, to_device_id, message, route_key)
            .await
            .expect("a relay route always serialises")
    }

    /// Largest plaintext frame [`WsServer::fan_out_to_relay`] will route.
    ///
    /// Well under the relay's 1 MiB read ceiling, so that hex-encoding the payload
    /// — which doubles it — plus the route and envelope overhead still fits.
    /// Nothing that legitimately broadcasts comes close: the largest is a
    /// `file/progress` at about 100 bytes.
    const MAX_RELAY_BROADCAST_BYTES: usize = 256 * 1024;

    /// Build the signed `relay_route` that carries one directed frame to
    /// `to_device_id`, sealed exactly as the LAN socket writer would have.
    ///
    /// `None` on any failure, so no caller can route a half-built frame.
    ///
    /// # Why the sealing has to happen here
    ///
    /// The LAN egress seals in the *per-connection writer*
    /// ([`Self::seal_for_peer`], called from `handle_client`), and the relay
    /// socket's writer sends whatever it is handed verbatim. So before this
    /// existed, `send_to`'s two paths behaved differently: a frame destined for
    /// a device on the LAN arrived wrapped in an `encrypted` envelope, and the
    /// *same* frame destined for the same device through the relay arrived as
    /// raw JSON — `commands::file`'s `file/request` and `file/chunk` (base64
    /// payload included) and `commands::pairing`'s pre-built envelope.
    ///
    /// Scope: this seals *exactly* what the LAN path seals and no more, because
    /// it calls the LAN path's own sealer. An `encrypted` frame still passes
    /// through unwrapped, and a `pairing` frame still goes in the clear so a peer
    /// can derive the secret it is about to be handed.
    async fn build_sealed_route(
        ctx: &WsContext,
        to_device_id: &str,
        message: &str,
        route_key: &[u8],
    ) -> Option<String> {
        let sealed = Self::seal_for_device(ctx, to_device_id, message).await;
        // Fails **closed**, unlike the `unwrap_or(Value::Null)` this replaces. A
        // null payload is not a harmless default: the relay does not inspect
        // payloads, so it would forward the literal string `null`, count it as
        // `messages_routed`, and the recipient — whose envelope parser requires
        // a map — would drop it with no error on either side.
        let payload: serde_json::Value = match serde_json::from_str(&sealed) {
            Ok(value) => value,
            Err(e) => {
                error!("Refusing to route to {to_device_id}: the payload is not JSON ({e})");
                return None;
            }
        };
        let from_device_id = ctx.device_id.as_str();
        let route = RelayRoute::signed_with(
            from_device_id,
            route_key,
            from_device_id,
            to_device_id,
            payload,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            uuid::Uuid::new_v4().to_string(),
        );
        match serde_json::to_string(&route) {
            Ok(wire) => Some(wire),
            Err(e) => {
                error!("Could not serialise a relay route for {to_device_id}: {e}");
                None
            }
        }
    }

    /// Route one frame to `device_id` through the relay, sealed for it.
    ///
    /// `false` is not an error: it means the relay is not connected, this
    /// desktop has no route key yet, or `device_id` does not name a paired
    /// device.
    ///
    /// That last one is a **gate, not a courtesy**, and it is the same predicate
    /// the auth gate uses, so the two cannot disagree about who a device is.
    /// Without it `send_error` — reachable from `not_authenticated` — would mint
    /// a signed, encrypted route for *every stranger's junk*, turning "you are
    /// not paired" into a cheap way to make this desktop sign and encrypt on
    /// demand. It is also what keeps an unpaired peer out of the fan-out this
    /// is the inner half of.
    ///
    /// The relay's own connection table is not visible from here, so whether
    /// `device_id` is actually connected to the relay is not answerable: a route
    /// to an absent device is queued and then dropped and counted by the relay
    /// as `messages_dropped_not_found`.
    ///
    /// `try_send`, never `send().await`. The channel is bounded at 1024 and the
    /// writer behind it can stall, and this is reached from broadcast paths that
    /// hold `ctx.clients` and `ws_to_device_id` readers — parking on an await
    /// there converts one slow relay into a hub-wide stall of registration,
    /// pairing and every LAN broadcast, because tokio's `RwLock` is
    /// write-preferring. Dropping is also the contract this path already has: the
    /// LAN half of a fan-out uses `broadcast::Sender::send`, which returns `Err`
    /// on a lagging subscriber and does not block. An off-LAN peer that misses
    /// one notification re-syncs; the alternative is delivering to nobody.
    async fn relay_route_to(ctx: &WsContext, device_id: &str, message: &str) -> bool {
        if !handlers::is_trusted_peer(ctx, device_id).await {
            debug!("Not routing to {device_id} through the relay: not a paired device");
            return false;
        }
        let Some(relay_tx) = ctx.relay_tx.read().await.clone() else {
            return false;
        };
        let from_device_id = ctx.device_id.as_str();
        // Never the relay token: the token authenticates the connection, the
        // route key proves which device is speaking.
        let Some(route_key) = ctx.route_keys.signing_key(from_device_id) else {
            error!(
                "Cannot route through the relay: no route key is registered for this \
                 desktop ({from_device_id})."
            );
            return false;
        };
        let Some(route) = Self::build_sealed_route(ctx, device_id, message, &route_key).await
        else {
            return false;
        };
        match relay_tx.try_send(route) {
            Ok(()) => true,
            Err(e) => {
                warn!("Relay queue full, dropping a frame for {device_id}: {e}");
                false
            }
        }
    }

    /// Fan `message` out to every paired device with **no socket on this
    /// desktop**, and report how many routes were queued.
    ///
    /// # What the relay is, as a destination
    ///
    /// `relay_tx` is an `mpsc::Sender<String>`, not a `ctx.clients` entry: the
    /// relay socket is outbound-only and is never in the connection map, which is
    /// why every fan-out in this app used to skip it entirely. A phone reachable
    /// only through the relay therefore received no clipboard, no notification,
    /// no SMS, no call and no discovery update — silently, because iterating an
    /// empty set looks exactly like success.
    ///
    /// # Who the recipients are
    ///
    /// `route_keys.routable_device_ids()`, minus three sets, and each exclusion
    /// is load-bearing:
    ///
    ///   * **This desktop.** It registers itself under its own id
    ///     (`DeviceRouteKeys::refresh`), so an unfiltered enumeration has the
    ///     desktop route a frame to itself — which the relay delivers straight
    ///     back, `unwrap_relay_delivery` accepts, `dispatch` re-enters, and
    ///     *that* fans out again. Multiplicative, not linear, until the
    ///     1024-slot queue fills and then hub-wide.
    ///   * **Devices with a live socket here.** Those are served by the LAN loop
    ///     that called us. Without this a phone that is on the LAN *and* joined
    ///     to the relay — exactly what happens during a network transition —
    ///     gets every notification twice, and file fan-out twice, which is worse
    ///     than not at all.
    ///   * **The sender**, passed as a **device id** because that is what the
    ///     relay addresses. `sender_id` is overloaded in this hub: a
    ///     per-connection UUID from the accept loop, or a relay-authenticated
    ///     device id once `handle_message` has unwrapped the delivery. Comparing
    ///     the raw string against device ids would exclude nothing for a relayed
    ///     sender and hand a phone its own notification back.
    ///
    /// # Why the recipient set is the registry and not `SyncEngine`
    ///
    /// Because `SyncEngine` is where a relay-only phone *is not*. `handle_client`
    /// removes a device from it when its LAN socket closes, so the flagship
    /// scenario — pair on the LAN, walk out of range — would produce an empty
    /// recipient set and a fix that does nothing.
    ///
    /// Locks: every guard is released before any `.await` that can block.
    /// `relay_route_to` seals (which reads `SyncEngine`) and then `try_send`s,
    /// and neither happens while `ws_to_device_id` or `SyncEngine` is held.
    pub(crate) async fn fan_out_to_relay(
        ctx: &WsContext,
        exclude_device: Option<&str>,
        message: &str,
    ) -> usize {
        // Size ceiling, and it is a *security* control rather than a courtesy.
        //
        // Sealing hex-encodes the payload, so a frame of `n` plaintext bytes
        // leaves as roughly `2n` plus the envelope. The relay's read limit is
        // 1 MiB, so anything over about half that crosses it — and crossing it
        // does not drop one frame, it **closes the connection**, taking every
        // relay-only peer offline for a whole backoff interval.
        //
        // Reachable because `validate_message` closes no field set for `sms`,
        // `call`, `clipboard` or the re-broadcast `file` actions: a paired peer
        // sends `{"type":"sms","action":"new","body":"x","pad":"<9 MB>"}` and it
        // is accepted. Before this arm existed that frame cost only memory on a
        // local `broadcast::Sender`; routing it turned a validation gap into an
        // availability bug. Closing `validate_message` is the better fix and is
        // tracked as W3.31; this is the part that has to hold regardless, since
        // the relay leg is a shared resource and one oversized frame spends it
        // all.
        //
        // Checked once per broadcast, not per recipient.
        if message.len() > Self::MAX_RELAY_BROADCAST_BYTES {
            warn!(
                "Refusing to fan out a {}-byte frame through the relay: sealed it would \
                 exceed the relay's frame ceiling and cost the whole connection. It was \
                 still delivered on the LAN.",
                message.len()
            );
            return 0;
        }
        // Checked up front so a desktop with no relay pays nothing per target and
        // a misconfiguration logs once rather than once per device.
        if ctx.relay_tx.read().await.is_none() {
            return 0;
        }
        let targets: Vec<String> = {
            let connected: Vec<String> =
                ctx.ws_to_device_id.read().await.values().cloned().collect();
            let own = ctx.device_id.as_str().to_string();
            ctx.route_keys
                .routable_device_ids()
                .into_iter()
                .filter(|d| *d != own)
                .filter(|d| Some(d.as_str()) != exclude_device)
                .filter(|d| !connected.contains(d))
                .collect()
        };
        let mut sent = 0usize;
        for device_id in targets {
            sent += usize::from(Self::relay_route_to(ctx, &device_id, message).await);
        }
        sent
    }

    /// Dial a relay and keep the connection up.
    ///
    /// `relay_token` is passed in rather than read from the environment: the
    /// host keeps the token in the OS keyring, so a client that reached for
    /// `RELAY_TOKEN` here would silently authenticate with an empty string.
    pub fn spawn_relay_client(&self, relay_url: String, server_id: String, relay_token: String) {
        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            let mut first_failure = true;
            let mut delay = std::time::Duration::from_secs(1);
            let max_delay = std::time::Duration::from_secs(60);
            loop {
                if first_failure {
                    info!("Attempting to connect to Relay Server at {}", relay_url);
                } else {
                    debug!("Attempting to connect to Relay Server at {}", relay_url);
                }
                match tokio_tungstenite::connect_async(&relay_url).await {
                    Ok((ws_stream, _)) => {
                        first_failure = true;
                        delay = std::time::Duration::from_secs(1); // Reset backoff on success
                        info!("Connected to Relay Server!");
                        let (mut write, mut read) = ws_stream.split();

                        let auth_msg = RelayAuth::new(server_id.as_str(), relay_token.clone());
                        let auth_msg =
                            serde_json::to_string(&auth_msg).expect("RelayAuth serializes");
                        if let Err(e) = write.send(Message::Text(auth_msg.into())).await {
                            error!("Failed to send auth to relay: {}", e);
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            continue;
                        }

                        let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
                        *ctx.relay_tx.write().await = Some(tx);

                        let mut ping_interval =
                            tokio::time::interval(tokio::time::Duration::from_secs(25));

                        let tx_task = tokio::spawn(async move {
                            let ping =
                                serde_json::to_string(&Ping::new()).expect("Ping serializes");
                            loop {
                                tokio::select! {
                                    _ = ping_interval.tick() => {
                                        let _ = write.send(Message::Text(ping.clone().into())).await;
                                    }
                                    msg_opt = rx.recv() => {
                                        if let Some(msg) = msg_opt {
                                            if let Err(e) = write.send(Message::Text(msg.into())).await {
                                                error!("Relay write failed: {}", e);
                                                break;
                                            }
                                        } else {
                                            break;
                                        }
                                    }
                                }
                            }
                        });

                        while let Some(Ok(msg)) = read.next().await {
                            match msg {
                                Message::Text(text) => {
                                    Self::handle_message(&text, Self::RELAY_CLIENT_ID, &ctx).await;
                                }
                                Message::Binary(bytes) => {
                                    if !Self::admit_relay_binary(&ctx, &bytes).await {
                                        continue;
                                    }
                                    handlers::files::handle_binary_message(
                                        bytes.to_vec(),
                                        "relay_server",
                                        &ctx,
                                    )
                                    .await;
                                }
                                _ => {}
                            }
                        }

                        tx_task.abort();
                        *ctx.relay_tx.write().await = None;
                        warn!("Disconnected from Relay Server. Reconnecting in 5s...");
                    }
                    Err(e) => {
                        if first_failure {
                            warn!(
                                "Failed to connect to relay ({}). Retrying in background...",
                                e
                            );
                            first_failure = false;
                        } else {
                            debug!("Failed to connect to relay ({}). Retrying...", e);
                        }
                    }
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
            }
        });
    }

    pub fn shutdown(&self) {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(());
        }
    }

    pub async fn disconnect_client(&self, device_id: &str) {
        let revoke_msg = PairingRevoke {
            msg_type: "pairing".into(),
            action: "revoke".into(),
            protocol_version: Some(PROTOCOL_VERSION),
            reason: Some("Device revoked by desktop".into()),
        };
        let revoke_msg = serde_json::to_string(&revoke_msg).expect("PairingRevoke serializes");
        {
            let clients_lock = self.ctx.clients.read().await;
            if let Some(tx) = clients_lock.get(device_id) {
                let _ = tx.send(revoke_msg.clone());
                info!("Sent revoke message to device {}", device_id);
            } else {
                let ws_map = self.ctx.ws_to_device_id.read().await;
                let ws_id = ws_map
                    .iter()
                    .find(|(_, v)| v.as_str() == device_id)
                    .map(|(k, _)| k.clone());
                drop(ws_map);
                if let Some(ws_id) = ws_id
                    && let Some(tx) = clients_lock.get(&ws_id)
                {
                    let _ = tx.send(revoke_msg);
                    info!(
                        "Sent revoke message to device {} (ws: {})",
                        device_id, ws_id
                    );
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.ctx.clients.write().await.remove(device_id);
        self.ctx.sync_engine.write().await.remove_client(device_id);
        // Now, not at the next 5 s registry refresh: since `peer_secret` falls back
        // to that registry, a device left in it stays both routable *and* trusted
        // after a revoke — so `pairing/accept`'s copy would keep delivering
        // clipboard and notification bodies to a phone the user just unpaired.
        self.ctx.route_keys.forget(device_id);
        self.ctx.rate_limiter.remove_client(device_id).await;
        self.ctx.per_type_limiter.remove_client(device_id).await;
        self.ctx
            .ws_to_device_id
            .write()
            .await
            .retain(|_, v| v.as_str() != device_id);
        info!("Device {} disconnected (revoked)", device_id);
    }
}

/// How far ahead of this desktop's own clock a peer's `timestamp` may be before
/// it is treated as unusable.
///
/// `save_clipboard` prunes by `ORDER BY timestamp DESC`, so the column is load
/// bearing for retention, not just display. A phone whose clock is hours fast
/// would otherwise pin its own entries at the top of every device's history for
/// as long as they stayed pinned — so a future timestamp is refused in favour of
/// the arrival time, which is monotonic by construction.
const CLIPBOARD_CLOCK_SKEW_TOLERANCE_SECS: i64 = 300;

/// Write one inbound `clipboard/sync` into `clipboard_history`, attributed to
/// the device that sent it.
///
/// Returns whether a row was written. This is the inbound counterpart of
/// `commands::notifications::sync_clipboard`, and it exists because of a wiring
/// gap: that command had exactly one caller in the whole repository and nothing
/// in the webview ever invoked it, so the only path into the table was dead code.
/// The live path is `useClipboard`'s raw `clipboard/sync` frame, which arrives
/// here — and used to leave without being stored.
///
/// # Attribution
///
/// `source_device` is the **connection's registered identity**, never the
/// frame's own `source_device` field. A socket's identity was written by the
/// hub at pairing time from a one-time token, so it cannot be forged; a field
/// inside the payload can. This is the same rule `resolve_sender_secret` and
/// `seal_for_peer` already apply, and it is why a relayed frame — whose
/// `client_id` is the device the relay authenticated, with no pairing-registry
/// entry at all — falls back to using that id directly.
///
/// The one substitution is [`LOCAL_DESKTOP_ID`], which is a *transport* alias
/// for the desktop's own webview and not a device: the row records
/// `ctx.device_id`, the id the rest of the app and the wire both use, which is
/// also what the sender put in the frame.
///
/// # `pinned`
///
/// Left at the column default (`0`). A pinned row survives
/// `clear_clipboard_history`, so letting a remote peer pin rows would give any
/// paired phone a way to make content on this desktop undeletable from the UI.
/// Pinning stays a local, deliberate act (`toggle_clipboard_pin`).
async fn persist_inbound_clipboard(ctx: &WsContext, client_id: &str, msg: &Value) -> bool {
    // `validate_clipboard_message` treats `content` as optional, so a frame can
    // legitimately reach here with nothing to store. Relaying it is still right;
    // storing an empty row is not.
    let Some(content) = msg.get("content").and_then(Value::as_str) else {
        return false;
    };
    if content.is_empty() {
        return false;
    }
    let mime = msg
        .get("mime")
        .and_then(Value::as_str)
        .unwrap_or("text/plain");

    let stable_id = handlers::paired_device_id(ctx, client_id)
        .await
        // A relayed sender has no LAN socket, so there is no entry to resolve
        // and the id the relay vouched for is already the stable one.
        .unwrap_or_else(|| client_id.to_string());
    let source_device = if stable_id == handlers::LOCAL_DESKTOP_ID {
        ctx.device_id.as_str().to_string()
    } else {
        stable_id
    };

    // The sender's clock is used when it is usable, so a shared clipboard sorts
    // the same way on every device; arrival time is the fallback. See
    // `CLIPBOARD_CLOCK_SKEW_TOLERANCE_SECS` for why a future stamp is refused.
    let now = chrono::Utc::now().timestamp();
    let timestamp = msg
        .get("timestamp")
        .and_then(Value::as_i64)
        .filter(|t| *t > 0 && *t <= now + CLIPBOARD_CLOCK_SKEW_TOLERANCE_SECS)
        .unwrap_or(now);

    match ctx
        .storage
        .save_clipboard(content, mime, &source_device, timestamp)
        .await
    {
        Ok(()) => true,
        Err(e) => {
            warn!(
                "Failed to persist clipboard entry from {}: {}",
                client_id, e
            );
            false
        }
    }
}

// ─── Settings enforcement at the dispatch boundary ───────────────────────────
//
// `sync_notifications`, `sync_clipboard`, `sync_files`, `notifications_enabled`,
// `notification_apps` and `auto_accept_files` (in `handlers::files`) are all
// enforced here, at the single point where an inbound message is translated into
// a handler call. The settings table is a key/value store, so each gate costs one
// indexed single-row read — cheap next to the SQLite write and encryption a
// handled message performs, and it removes the need to thread `Storage` into
// every handler. A rejected message is logged *and* answered with a protocol
// `error` frame carrying a stable `code`, so the sending device learns why.

/// Read a boolean setting, falling back to `default` when the row is missing.
///
/// Only the exact string `"true"` enables a gate, matching the convention
/// `Storage::get_settings` already uses for every boolean row.
async fn setting_enabled(ctx: &WsContext, key: &str, default: bool) -> bool {
    match ctx.storage.get_setting(key).await {
        Some(raw) => raw == "true",
        None => default,
    }
}

/// Read a `Vec<String>` setting stored as a JSON array.
///
/// A *missing* row falls back to the documented default (so a fresh install
/// behaves exactly like `Storage::get_settings` reports), while a *corrupt* row
/// yields an empty list — an allowlist fails closed.
async fn setting_string_list(ctx: &WsContext, key: &str, default: Vec<String>) -> Vec<String> {
    match ctx.storage.get_setting(key).await {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        None => default,
    }
}

/// Whether an inbound notification's `app` is on the user's allowlist.
///
/// Matching is case-insensitive and tolerates Android package names, so a
/// `notification_apps` entry of `WhatsApp` matches a reported `app` of either
/// `WhatsApp` or `com.whatsapp`. An app the sender did not name, or an empty
/// allowlist, is never allowed.
fn app_is_allowed(allowlist: &[String], app: &str) -> bool {
    let app = app.trim();
    if app.is_empty() {
        return false;
    }
    let tail = app.rsplit('.').next().unwrap_or(app);
    allowlist.iter().any(|entry| {
        let entry = entry.trim();
        entry.eq_ignore_ascii_case(app) || entry.eq_ignore_ascii_case(tail)
    })
}

/// The single settings gate in front of the dispatch `match`.
///
/// Evaluated after authentication and rate limiting, so a rejected message is
/// still accounted against the sender's budget and an unauthenticated peer can
/// never probe the user's settings. `None` means "dispatch normally".
///
/// | message type | setting | code when blocked |
/// |---|---|---|
/// | `notification`/`post` | `sync_notifications` | `notification_sync_disabled` |
/// | `notification`/`post` | `notifications_enabled` | `notifications_disabled` |
/// | `notification`/`post` | `notification_apps` | `notification_app_not_allowed` |
/// | `clipboard`/`sync` | `sync_clipboard` | `clipboard_sync_disabled` |
/// | `file`/any | `sync_files` | `file_sync_disabled` |
///
/// Gate matching on `msg_type` rather than on the individual `match` arms means
/// a new `file` action cannot accidentally escape `sync_files`.
async fn settings_gate(
    ctx: &WsContext,
    msg: &Value,
    msg_type: &str,
    action: &str,
) -> Option<(&'static str, String)> {
    match (msg_type, action) {
        ("notification", "post") => match notification_post_allowed(ctx, msg).await {
            Ok(()) => None,
            Err("notification_sync_disabled") => Some((
                "notification_sync_disabled",
                "Notification syncing is disabled in Settings".to_string(),
            )),
            Err("notifications_disabled") => Some((
                "notifications_disabled",
                "Notifications are disabled in Settings".to_string(),
            )),
            Err(_) => {
                let app = msg.get("app").and_then(|v| v.as_str()).unwrap_or("");
                Some((
                    "notification_app_not_allowed",
                    format!(
                        "App '{}' is not in the notification_apps allowlist",
                        if app.is_empty() { "<unnamed>" } else { app }
                    ),
                ))
            }
        },
        ("clipboard", "sync") => {
            if clipboard_sync_allowed(ctx).await.is_ok() {
                None
            } else {
                Some((
                    "clipboard_sync_disabled",
                    "Clipboard syncing is disabled in Settings".to_string(),
                ))
            }
        }
        ("file", _) => {
            if file_transfer_allowed(ctx).await.is_ok() {
                None
            } else {
                Some((
                    "file_sync_disabled",
                    "File transfers are disabled in Settings".to_string(),
                ))
            }
        }
        _ => None,
    }
}

/// Enforce `sync_clipboard`: with it off, clipboard content is never relayed to
/// other devices.
///
/// `Err(code)` means the message must not be dispatched.
async fn clipboard_sync_allowed(ctx: &WsContext) -> Result<(), &'static str> {
    if setting_enabled(ctx, "sync_clipboard", true).await {
        Ok(())
    } else {
        Err("clipboard_sync_disabled")
    }
}

/// Enforce `sync_files`: with it off, no file-transfer message of any kind
/// (`request`, `accept`, `chunk`, `progress`, `complete`, `cancel`, `resume`) is
/// dispatched.
async fn file_transfer_allowed(ctx: &WsContext) -> Result<(), &'static str> {
    if setting_enabled(ctx, "sync_files", true).await {
        Ok(())
    } else {
        Err("file_sync_disabled")
    }
}

/// Enforce the three settings that gate mirrored notifications, in order:
///  1. `sync_notifications` – master switch for mirroring notifications;
///  2. `notifications_enabled` – receive notifications at all;
///  3. `notification_apps` – per-app allowlist (an empty list mirrors nothing,
///     which is exactly what an emptied Settings list means).
async fn notification_post_allowed(ctx: &WsContext, msg: &Value) -> Result<(), &'static str> {
    if !setting_enabled(ctx, "sync_notifications", true).await {
        return Err("notification_sync_disabled");
    }
    if !setting_enabled(ctx, "notifications_enabled", true).await {
        return Err("notifications_disabled");
    }
    let allowlist = setting_string_list(
        ctx,
        "notification_apps",
        crate::commands::default_notification_apps(),
    )
    .await;
    let app = msg.get("app").and_then(|v| v.as_str()).unwrap_or("");
    if !app_is_allowed(&allowlist, app) {
        return Err("notification_app_not_allowed");
    }
    Ok(())
}

/// The socket to answer `id` on, if this desktop has one.
///
/// `ctx.clients` is keyed by **connection** id, so a direct lookup answers a LAN
/// socket and nothing else. A relayed sender's `client_id` is the *device* id the
/// relay authenticated (`handle_message` re-enters `dispatch` under it), and a
/// device id is never a connection id — so every refusal aimed at a relayed peer
/// (`rate_limited`, `invalid_message`, `not_authenticated`, a settings gate) was
/// looked up, missed, and dropped on the floor: the peer was refused and never
/// told why, which is indistinguishable from a dropped socket.
///
/// The second step is the same reverse walk of the pairing registry that
/// `WsServer::send_to` already does to reach a device on the LAN, so a
/// "how a device id is addressed" answer cannot exist in two places. Locks are
/// taken in the same order as everywhere else (`clients` then
/// `ws_to_device_id`).
///
/// `None` means "no socket here", which is **not** the same as "undeliverable":
/// a peer reachable only through the relay has no socket and is still reachable.
/// [`answer`] is what turns this answer into a delivery.
///
/// Deliberately still answers *only* sockets. This is the question "which
/// socket is this id on?", and a caller that wants the relay has to say so —
/// `answer` does, and is the only thing that does.
async fn answer_channel(ctx: &WsContext, id: &str) -> Option<ClientSender> {
    let clients = ctx.clients.read().await;
    if let Some(tx) = clients.get(id) {
        return Some(tx.clone());
    }
    let connection = {
        let registry = ctx.ws_to_device_id.read().await;
        registry
            .iter()
            .find(|(_, device_id)| device_id.as_str() == id)
            .map(|(connection_id, _)| connection_id.clone())
    }?;
    clients.get(&connection).cloned()
}

/// Get `message` to whoever `id` names, by whichever route exists.
///
/// A socket if there is one, and otherwise a signed, sealed route through the
/// relay. Reports whether it was handed to something.
///
/// This is the answer [`answer_channel`] deliberately could not give, and the
/// gap it left was that **every refusal aimed at a relay-only peer was looked
/// up, missed, and dropped**: `rate_limited`, `invalid_message`,
/// `not_authenticated`, `unsupported_protocol_version`, a settings gate. The
/// peer was refused and never told why, which from the far end is
/// indistinguishable from a dropped socket — and, since fan-out also skipped
/// those peers, the phone just went quiet.
///
/// Nothing about the wire format had to change for this. `relay_route`'s
/// payload is an arbitrary JSON message and the relay forwards it without
/// inspecting it, so an `error` is already a legal thing to route; the relay
/// wraps it in `relay_delivery` exactly as it does any other payload.
///
/// A LAN socket still wins, so a device that has both keeps its existing
/// behaviour and never pays for a round trip it does not need, and the two
/// paths seal identically so a peer cannot tell which one delivered a frame.
///
/// `relay_route_to` gates on the same trust predicate as the auth gate, so this
/// cannot be used to make the hub sign and encrypt for an unpaired id — see
/// its doc comment.
async fn answer(ctx: &WsContext, id: &str, message: &str) -> bool {
    if let Some(tx) = answer_channel(ctx, id).await {
        let _ = tx.send(message.to_string());
        return true;
    }
    crate::server::WsServer::relay_route_to(ctx, id, message).await
}

/// Deliver an already-serialised refusal to whoever `id` names.
///
/// The same egress [`send_error`] uses, for callers that have built their own
/// `ErrorMessage` with a domain-specific message — `handlers::files` declines a
/// transfer with a `file_accept_disabled` code and the filename in the detail,
/// which `send_error`'s signature has no room for.
pub(crate) async fn send_error_to(ctx: &WsContext, client_id: &str, serialized: &str) -> bool {
    answer(ctx, client_id, serialized).await
}

/// Tell the sender *why* its message was dropped, and say so in the log.
///
/// A silent drop leaves the peer waiting for an ack that never arrives; an
/// `error` frame with a stable `code` lets both sides record a meaningful
/// reason. Unicast, so it also reaches a peer that is not paired yet — via
/// [`answer`], which finds that peer's LAN socket if it has one and otherwise
/// routes the refusal back through the relay.
async fn send_error(ctx: &WsContext, client_id: &str, code: &str, detail: &str) {
    let err_resp = ErrorMessage {
        msg_type: "error".into(),
        code: code.to_string(),
        message: detail.to_string(),
        server_version: Some(PROTOCOL_VERSION),
    };
    let err_resp = serde_json::to_string(&err_resp).expect("ErrorMessage serializes");
    if !answer(ctx, client_id, &err_resp).await {
        warn!(
            "Cannot answer {code} to {client_id}: no socket on this desktop and no route to it through the relay"
        );
    }
}

/// `settings_gate` rejection: log it and answer the sender.
async fn reject_due_to_setting(
    ctx: &WsContext,
    client_id: &str,
    code: &'static str,
    detail: String,
) {
    warn!(
        "Message from {} rejected: {} ({}). Enable it in Settings to allow this.",
        client_id, detail, code
    );
    send_error(ctx, client_id, code, &detail).await;
}

/// Reject a remotely-supplied automation rule (or sync batch) whose action
/// would run a shell command the user has not allowlisted.
///
/// `automation::validate_rule` checks the id, name and trigger but **never
/// inspects `ActionType::RunShellCommand`**, so a paired-but-remote peer could
/// persist an arbitrary command string that later fired on a trigger (or on the
/// next launch, via the time-trigger timer). `automation.rs` is not mine to
/// change, so the equivalent check is enforced here at the dispatch boundary,
/// which is the last point every automation write passes through.
///
/// Enforcing it on *write* as well as on *execute* is deliberate: a rule that is
/// persisted while blocked becomes a landmine the moment the user later
/// allowlists the command. Better to refuse it and say why.
fn shell_rule_gate(msg: &Value, allowlist: &automation::CommandAllowlist) -> Result<(), String> {
    let packets: Vec<Value> = match msg.get("rules").and_then(|v| v.as_array()) {
        Some(arr) => arr.clone(),
        None => vec![msg.clone()],
    };
    for packet in &packets {
        let Some(action) = extract_action(packet) else {
            continue;
        };
        if let automation::ActionType::RunShellCommand { command } = &action
            && !allowlist.is_allowed(command)
        {
            return Err(format!(
                "Command '{}' is not in the shell command allowlist",
                command
            ));
        }
    }
    Ok(())
}

/// Pull the `ActionType` out of an automation packet.
///
/// `automation::parse_rule_packet` accepts the action under `rule_action`,
/// `action` or `action_config` and only if it is a JSON *object* — note the
/// distinction from the protocol's top-level string `action` field, which holds
/// `"rule"`/`"sync"`. Mirrored here so the gate cannot be walked around by
/// choosing a different spelling of the same field.
fn extract_action(packet: &Value) -> Option<automation::ActionType> {
    let src = match packet.get("rule") {
        Some(r) if r.is_object() => r,
        _ => packet,
    };
    let action_val = src
        .get("rule_action")
        .filter(|v| v.is_object())
        .or_else(|| src.get("action").filter(|v| v.is_object()))
        .or_else(|| src.get("action_config").filter(|v| v.is_object()))?;
    serde_json::from_value(action_val.clone()).ok()
}

/// Refuse to *execute* an existing rule whose shell command the allowlist
/// rejects, for the `automation/triggered` path.
///
/// This complements [`shell_rule_gate`], which guards rule **writes**. The
/// handler on the other side (`handlers::auto_rules::handle_automation_triggered`)
/// calls `execute_action` with no allowlist, so a rule that is already in the
/// engine — persisted by a build from before the allowlist existed, or written
/// to the database by any other means — would otherwise still run its command.
/// The check is on the rule the engine actually holds, not on anything the
/// sender supplies.
async fn triggered_rule_gate(
    msg: &Value,
    ctx: &WsContext,
    allowlist: &automation::CommandAllowlist,
) -> Result<(), String> {
    let rule_id = msg
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| msg.get("rule_id").and_then(|v| v.as_str()).unwrap_or(""));
    if rule_id.is_empty() {
        return Err("automation/triggered requires a rule id".to_string());
    }
    let action = {
        let engine = ctx.automation_engine.read().await;
        match engine.get_rule(rule_id) {
            Some(rule) => rule.action.clone(),
            // Unknown rules are the handler's business (it logs and returns);
            // there is nothing to execute, so nothing to gate.
            None => return Ok(()),
        }
    };
    if let automation::ActionType::RunShellCommand { command } = &action
        && !allowlist.is_allowed(command)
    {
        return Err(format!(
            "Rule '{rule_id}' runs '{}', which is not in the shell command allowlist",
            command
        ));
    }
    Ok(())
}

async fn handle_status_update(msg: Value, client_id: &str, ctx: &WsContext) {
    // `SyncEngine` is keyed by stable device id, but this handler used to look
    // up the *connection* id, so it never matched and every battery level and
    // heartbeat update was silently dropped.
    let stable_id = handlers::paired_device_id(ctx, client_id)
        .await
        .unwrap_or_else(|| client_id.to_string());
    let battery = msg.get("battery").and_then(|v| v.as_i64());

    {
        let mut engine = ctx.sync_engine.write().await;
        let updated_id = engine.apply_status(&stable_id, chrono::Utc::now().timestamp(), battery);
        if let (Some(id), Some(b)) = (updated_id.as_deref(), battery) {
            let _ = ctx.storage.update_battery(id, b as i32).await;
        }
    }

    if let Some(b) = battery {
        let context = TriggerContext {
            event: TriggerEvent::BatteryUpdate,
            source_device_id: stable_id.clone(),
            battery_level: Some(b as i32),
            wifi_ssid: None,
            app_package: None,
        };
        let allowlist = security::current_command_allowlist();
        let engine = ctx.automation_engine.read().await;
        for rule in engine.get_rules() {
            if rule.enabled && engine.evaluate_trigger(&rule.trigger, &context).await {
                if !automation::is_desktop_executable(&rule.action) {
                    continue;
                }
                // SECURITY: was `execute_action` (no allowlist).
                let log = automation::execute_action_with_allowlist(&rule.action, Some(&allowlist));
                let _ = ctx
                    .storage
                    .log_automation_execution(
                        &rule.id,
                        "battery_level",
                        log.timestamp,
                        log.success,
                        log.message.as_deref(),
                    )
                    .await;
            }
        }
    }
}

async fn handle_discovery_announce(msg: Value, client_id: &str, ctx: &WsContext) {
    let device_name = msg
        .get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let device_type = msg
        .get("device_type")
        .and_then(|v| v.as_str())
        .unwrap_or("phone");
    let device_id = msg
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or(client_id);
    let os = msg.get("os").and_then(|v| v.as_str()).unwrap_or("unknown");
    let version = msg
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    info!(
        "Discovery announce from {} ({}, {}, os={}, ver={})",
        device_id, device_name, device_type, os, version
    );

    let server_id = ctx
        .encryption
        .public_key_hex()
        .chars()
        .take(16)
        .collect::<String>();
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let response = DiscoveryAnnounce {
        msg_type: "discovery".into(),
        action: "announce".into(),
        protocol_version: Some(PROTOCOL_VERSION),
        device_id: server_id,
        device_name: hostname,
        device_type: "desktop".into(),
        os: std::env::consts::OS.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        battery: Some(100),
        ws_port: Some(crate::WS_PORT),
        wss_port: Some(crate::WSS_PORT),
        apns_token: None,
    };
    let response = serde_json::to_string(&response).expect("DiscoveryAnnounce serializes");

    let clients_lock = ctx.clients.read().await;
    if let Some(tx) = clients_lock.get(client_id) {
        let _ = tx.send(response);
    }
}

/// Handle `discovery/remove` — a peer announcing that a device has gone away.
///
/// The protocol type `DiscoveryRemove` exists and the mobile app sends it, but
/// there was **no dispatch arm at all**, so the frame died in the `_` catch-all
/// with only a `warn!`. The desktop's own presence bookkeeping is just a log
/// line, but the frame is also relayed to every paired peer so their device
/// lists can drop the id.
async fn handle_discovery_remove(msg: Value, client_id: &str, ctx: &WsContext) {
    let device_id = msg.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    if device_id.is_empty() {
        warn!("Discovery remove from {}: missing device_id", client_id);
        send_error(
            ctx,
            client_id,
            "invalid_message",
            "discovery/remove requires a device_id",
        )
        .await;
        return;
    }
    info!("Discovery remove: device {device_id} is no longer available");

    let remove = DiscoveryRemove {
        msg_type: "discovery".into(),
        action: "remove".into(),
        device_id: device_id.to_string(),
    };
    let frame = serde_json::to_string(&remove).expect("DiscoveryRemove serializes");
    // Only paired peers are told; an unpaired socket must learn nothing.
    broadcast_to_others(ctx, client_id, &frame).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers as th;
    use crate::server::handlers::test_helpers::{
        add_test_paired_client, add_test_unpaired_client, create_test_ctx,
    };

    /// A paired `("notification", "post")` from an app on the default allowlist
    /// must be persisted and relayed. This is the "settings at their defaults do
    /// not break anything" baseline for the gates below.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_valid_defaults_allow_allowed_app() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "notification",
            "action": "post",
            "id": "n_gate_ok",
            "device_id": "dev_phone",
            "app": "Slack",
            "title": "Deploy finished",
            "body": "all good",
            "timestamp": 1_700_000_000
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        let stored = ctx.storage.get_notifications(10).await.unwrap();
        assert_eq!(stored.len(), 1, "an allowlisted app must still be mirrored");
        assert_eq!(stored[0].id, "n_gate_ok");
        // Nothing rejected: no error frame for the sender.
        let err = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(err.is_err(), "sender must not receive an error frame");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_invalid_sync_notifications_disabled_blocks_post() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("sync_notifications", "false")
            .await
            .unwrap();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "notification", "action": "post", "id": "n_off", "app": "Slack"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        assert!(ctx.storage.get_notifications(10).await.unwrap().is_empty());
        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("sender must be told why")
            .unwrap();
        assert!(
            resp.contains("notification_sync_disabled"),
            "expected notification_sync_disabled, got: {resp}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_invalid_notifications_disabled_blocks_post() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("notifications_enabled", "false")
            .await
            .unwrap();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "notification", "action": "post", "id": "n_off2", "app": "Slack"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        assert!(ctx.storage.get_notifications(10).await.unwrap().is_empty());
        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("sender must be told why")
            .unwrap();
        assert!(
            resp.contains("notifications_disabled"),
            "expected notifications_disabled, got: {resp}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_invalid_app_outside_allowlist_blocks_post() {
        let ctx = create_test_ctx();
        // Persist the allowlist exactly as the Settings UI would.
        ctx.storage
            .save_setting("notification_apps", r#"["Slack"]"#)
            .await
            .unwrap();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "notification", "action": "post", "id": "n_other", "app": "com.secretbank.app"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        assert!(
            ctx.storage.get_notifications(10).await.unwrap().is_empty(),
            "apps outside notification_apps must not be mirrored"
        );
        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("sender must be told why")
            .unwrap();
        assert!(
            resp.contains("notification_app_not_allowed"),
            "expected notification_app_not_allowed, got: {resp}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_edge_empty_allowlist_mirrors_nothing() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("notification_apps", "[]")
            .await
            .unwrap();
        let _tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        let text = serde_json::to_string(&serde_json::json!({
            "type": "notification", "action": "post", "id": "n_empty", "app": "Slack"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        assert!(
            ctx.storage.get_notifications(10).await.unwrap().is_empty(),
            "an empty allowlist mirrors nothing — fail closed"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_valid_clipboard_sync_disabled_blocks_relay() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("sync_clipboard", "false")
            .await
            .unwrap();
        let sender = add_test_paired_client(&ctx, "ws_a", "dev_a").await;
        let peer = add_test_paired_client(&ctx, "ws_b", "dev_b").await;
        let mut peer_rx = peer.subscribe();
        let mut sender_rx = sender.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync", "content": "secret"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_a", &ctx).await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(200), peer_rx.recv()).await;
        assert!(leaked.is_err(), "clipboard must not reach other devices");
        let err = tokio::time::timeout(std::time::Duration::from_millis(500), sender_rx.recv())
            .await
            .expect("sender must be told why")
            .unwrap();
        assert!(
            err.contains("clipboard_sync_disabled"),
            "expected clipboard_sync_disabled, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_valid_sync_files_disabled_blocks_every_file_action() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("sync_files", "false")
            .await
            .unwrap();
        let sender = add_test_paired_client(&ctx, "ws_a", "dev_a").await;
        let peer = add_test_paired_client(&ctx, "ws_b", "dev_b").await;
        let mut peer_rx = peer.subscribe();
        let mut sender_rx = sender.subscribe();

        for action in [
            "request",
            "accept",
            "chunk",
            "progress",
            "complete",
            "cancel",
            "resume",
            "some_future_action",
        ] {
            let text = serde_json::to_string(&serde_json::json!({
                "type": "file", "action": action, "id": "t_blocked", "name": "x.bin"
            }))
            .unwrap();
            WsServer::handle_message(&text, "ws_a", &ctx).await;
        }

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(200), peer_rx.recv()).await;
        assert!(leaked.is_err(), "no file message may be relayed");
        let err = tokio::time::timeout(std::time::Duration::from_millis(500), sender_rx.recv())
            .await
            .expect("sender must be told why")
            .unwrap();
        assert!(
            err.contains("file_sync_disabled"),
            "expected file_sync_disabled, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_valid_sync_files_enabled_relays_file_request() {
        let ctx = create_test_ctx();
        let sender = add_test_paired_client(&ctx, "ws_a", "dev_a").await;
        let peer = add_test_paired_client(&ctx, "ws_b", "dev_b").await;
        let mut peer_rx = peer.subscribe();
        let _sender_rx = sender.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "file", "action": "request", "id": "t_ok", "name": "x.bin", "size": 8
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_a", &ctx).await;

        let relayed = tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv())
            .await
            .expect("file request must be relayed when sync_files is on")
            .unwrap();
        assert!(relayed.contains("t_ok"), "unexpected relay: {relayed}");
    }

    /// Unauthenticated peers must be rejected by the auth gate *before* the
    /// settings gate, so a stranger cannot infer the user's settings.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_invalid_unauthenticated_rejected_with_auth_error_not_settings() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("sync_clipboard", "false")
            .await
            .unwrap();
        let tx = add_test_paired_client(&ctx, "ws_stranger", "dev_stranger").await;
        // Remove the device-id mapping so the client counts as unpaired.
        ctx.ws_to_device_id.write().await.remove("ws_stranger");
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync", "content": "probe"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_stranger", &ctx).await;

        let err = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("stranger must be rejected")
            .unwrap();
        assert!(
            err.contains("not_authenticated"),
            "auth gate must win over the settings gate, got: {err}"
        );
    }

    // ── app_is_allowed unit tests ─────────────────────────────────────────────

    #[test]
    fn app_is_allowed_valid_exact_and_case_insensitive_match() {
        let list = vec!["Slack".to_string(), "whatsapp".to_string()];
        assert!(app_is_allowed(&list, "Slack"));
        assert!(app_is_allowed(&list, "slack"));
        assert!(app_is_allowed(&list, "  Slack  "));
    }

    #[test]
    fn app_is_allowed_valid_android_package_matches_listed_app() {
        let list = vec!["WhatsApp".to_string()];
        assert!(app_is_allowed(&list, "com.whatsapp"));
        assert!(
            !app_is_allowed(&list, "com.facebook.orca"),
            "an unlisted package must stay blocked"
        );
    }

    #[test]
    fn app_is_allowed_edge_empty_list_and_empty_app_are_denied() {
        assert!(!app_is_allowed(&[], "Slack"));
        assert!(!app_is_allowed(&["Slack".to_string()], ""));
        assert!(!app_is_allowed(&["Slack".to_string()], "   "));
    }

    // ── settings_gate unit tests ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settings_gate_edge_unrelated_message_types_are_never_gated() {
        let ctx = create_test_ctx();
        for key in [
            "sync_files",
            "sync_clipboard",
            "sync_notifications",
            "notifications_enabled",
        ] {
            ctx.storage.save_setting(key, "false").await.unwrap();
        }
        ctx.storage
            .save_setting("notification_apps", "[]")
            .await
            .unwrap();

        for (msg_type, action) in [
            ("ping", ""),
            ("status", ""),
            ("sms", "new"),
            ("call", "ring"),
            ("automation", "sync"),
            ("discovery", "announce"),
            ("screen_mirror", "start"),
        ] {
            assert!(
                settings_gate(&ctx, &serde_json::json!({}), msg_type, action)
                    .await
                    .is_none(),
                "({msg_type}, {action}) must not be affected by the sync settings"
            );
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // V1a — unauthenticated loopback client cannot reach automation/rule
    //
    // This is the exploit the whole change exists to close. It is written as a
    // three-step chain exactly as an attacker would run it:
    //
    //   1. open a socket (what used to be auto-paired just for being on
    //      loopback),
    //   2. persist a `RunShellCommand` automation rule,
    //   3. fire the rule.
    //
    // Steps 2 and 3 must both be refused, and no shell command may run.
    // ════════════════════════════════════════════════════════════════════════

    /// An automation rule carrying a shell command, in the exact wire shape the
    /// frontend's `useAutomation` hook sends.
    fn shell_rule_packet(id: &str, command: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "type": "automation",
            "action": "rule",
            "rule": {
                "id": id,
                "name": "pwn",
                "enabled": true,
                "trigger": { "type": "time", "time": "03:00" },
                "action": { "type": "run_shell_command", "command": command }
            }
        }))
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_unpaired_loopback_client_cannot_reach_automation_rule() {
        let ctx = create_test_ctx();
        // Step 1: a socket with no pairing and no local capability. Before the
        // fix, `accept_loop` had already inserted this connection into
        // `ws_to_device_id` as `local_desktop` by the time it got here.
        let tx = add_test_unpaired_client(&ctx, "ws_localhost_attacker").await;
        let mut rx = tx.subscribe();

        // Step 2: persist the rule.
        WsServer::handle_message(
            &shell_rule_packet("rce_1", "calc.exe"),
            "ws_localhost_attacker",
            &ctx,
        )
        .await;

        // Step 3: try to fire it directly, in case step 2 somehow landed.
        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "automation", "action": "triggered",
                "id": "rce_1", "trigger_type": "time"
            }))
            .unwrap(),
            "ws_localhost_attacker",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the peer must be told why")
            .unwrap();
        assert!(
            resp.contains("not_authenticated"),
            "expected not_authenticated, got: {resp}"
        );

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty(),
            "an unpaired client must never persist an automation rule"
        );
        assert!(
            ctx.automation_engine
                .read()
                .await
                .get_rule("rce_1")
                .is_none(),
            "an unpaired client must never load a rule into the engine"
        );
        assert!(
            ctx.storage
                .get_automation_logs(10)
                .await
                .unwrap()
                .is_empty(),
            "no rule may have executed"
        );
    }

    /// The same packet from a *paired* peer is accepted, so the fix is an
    /// authorisation change and not a blanket ban.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_paired_client_may_still_create_a_non_shell_rule() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let _rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "automation", "action": "rule",
                "rule": {
                    "id": "ok_1", "name": "notify",
                    "enabled": true,
                    "trigger": { "type": "time", "time": "03:00" },
                    "action": { "type": "send_notification", "title": "t", "body": "b" }
                }
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        assert_eq!(
            ctx.storage.get_all_automation_rules().await.unwrap().len(),
            1
        );
    }

    /// Deny-by-default: with the shipped default (an empty `allowed_commands`),
    /// even a *paired* peer cannot persist a shell rule, and cannot fire one
    /// that is already in the engine.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_paired_client_cannot_persist_a_shell_rule_by_default() {
        security::set_command_allowlist(Vec::new());
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        WsServer::handle_message(&shell_rule_packet("rce_2", "calc.exe"), "ws_phone", &ctx).await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the peer must be told why")
            .unwrap();
        assert!(
            resp.contains("command_not_allowed"),
            "expected command_not_allowed, got: {resp}"
        );
        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty(),
            "a shell rule outside the allowlist must never be persisted"
        );
    }

    /// The gate is not a bypassable by renaming the action field:
    /// `parse_rule_packet` accepts `rule_action`, `action` or `action_config`,
    /// so all three must be checked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_shell_gate_covers_every_action_field_alias() {
        security::set_command_allowlist(Vec::new());
        let allowlist = security::current_command_allowlist();
        for alias in ["rule_action", "action", "action_config"] {
            let msg = serde_json::json!({
                "type": "automation",
                "action": "rule",
                "rule": {
                    "id": "alias",
                    "name": "x",
                    "enabled": true,
                    "trigger": { "type": "time", "time": "01:00" },
                    alias: { "type": "run_shell_command", "command": "calc.exe" }
                }
            });
            assert!(
                shell_rule_gate(&msg, &allowlist).is_err(),
                "the gate must inspect the '{alias}' spelling"
            );
        }
    }

    /// And an allowlisted command is accepted by the gate (so the feature works
    /// once the user opts in).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_shell_gate_allows_an_explicitly_allowlisted_command() {
        security::set_command_allowlist(vec!["notepad".to_string()]);
        let allowlist = security::current_command_allowlist();
        let allowed = serde_json::json!({
            "type": "automation", "action": "rule",
            "rule": {
                "id": "ok", "name": "x", "enabled": true,
                "trigger": { "type": "time", "time": "01:00" },
                "action": { "type": "run_shell_command", "command": "notepad notes.txt" }
            }
        });
        assert!(shell_rule_gate(&allowed, &allowlist).is_ok());
        // Restore the deny-by-default the other tests assume.
        security::set_command_allowlist(Vec::new());
    }

    /// A rule already in the engine (e.g. persisted by an older build) must not
    /// execute a blocked command either — the allowlist is enforced at
    /// execution, not only at write time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_triggered_path_refuses_a_legacy_blocked_rule() {
        security::set_command_allowlist(Vec::new());
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        // As if an older build (or a direct database write) had persisted it.
        ctx.automation_engine
            .write()
            .await
            .add_rule(automation::AutomationRule {
                id: "legacy_shell".into(),
                name: "legacy".into(),
                enabled: true,
                trigger: automation::TriggerType::Time {
                    time: "03:00".into(),
                },
                action: automation::ActionType::RunShellCommand {
                    command: "this-command-does-not-exist-xyz".into(),
                },
                trusted_source_only: false,
            });

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "automation", "action": "triggered",
                "id": "legacy_shell", "trigger_type": "time"
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the peer must be told why")
            .unwrap();
        assert!(
            resp.contains("command_not_allowed"),
            "a legacy blocked rule must not be executable, got: {resp}"
        );
        assert!(
            ctx.storage
                .get_automation_logs(10)
                .await
                .unwrap()
                .is_empty(),
            "nothing may have been executed or logged"
        );
    }

    /// And the gate does not block a rule that is not a shell command, nor an
    /// unknown rule id.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_triggered_gate_allows_non_shell_and_unknown_rules() {
        security::set_command_allowlist(Vec::new());
        let ctx = create_test_ctx();
        ctx.automation_engine
            .write()
            .await
            .add_rule(automation::AutomationRule {
                id: "notify_rule".into(),
                name: "notify".into(),
                enabled: true,
                trigger: automation::TriggerType::Time {
                    time: "03:00".into(),
                },
                action: automation::ActionType::SendNotification {
                    title: "t".into(),
                    body: "b".into(),
                },
                trusted_source_only: false,
            });

        let allowlist = security::current_command_allowlist();
        assert!(
            triggered_rule_gate(&serde_json::json!({"id": "notify_rule"}), &ctx, &allowlist)
                .await
                .is_ok()
        );
        assert!(
            triggered_rule_gate(
                &serde_json::json!({"id": "does_not_exist"}),
                &ctx,
                &allowlist
            )
            .await
            .is_ok()
        );
        assert!(
            triggered_rule_gate(&serde_json::json!({}), &ctx, &allowlist)
                .await
                .is_err(),
            "a trigger with no rule id names nothing to execute"
        );
    }

    /// `automation/sync` persists an array of rules and must be gated too.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_shell_gate_covers_automation_sync_batches() {
        security::set_command_allowlist(Vec::new());
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let _rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "automation", "action": "sync",
                "rules": [ serde_json::from_str::<serde_json::Value>(
                    &shell_rule_packet("bulk", "calc.exe")).unwrap() ]
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty(),
            "a sync batch containing a blocked shell rule must be refused wholesale"
        );
    }

    /// A rule already in the engine (e.g. persisted by an older build) must not
    /// execute a blocked command either — the allowlist is enforced at
    /// execution, not only at write time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rce_chain_execution_path_enforces_the_allowlist() {
        security::set_command_allowlist(Vec::new());
        let ctx = create_test_ctx();
        ctx.automation_engine
            .write()
            .await
            .add_rule(automation::AutomationRule {
                id: "legacy".into(),
                name: "legacy".into(),
                enabled: true,
                trigger: automation::TriggerType::BatteryLevel {
                    below: 20,
                    device_id: None,
                },
                action: automation::ActionType::RunShellCommand {
                    command: "this-command-does-not-exist-xyz".into(),
                },
                trusted_source_only: false,
            });

        let event = TriggerContext {
            event: TriggerEvent::BatteryUpdate,
            source_device_id: "dev".into(),
            battery_level: Some(5),
            wifi_ssid: None,
            app_package: None,
        };
        WsServer::evaluate_device_triggers(&event, &ctx).await;

        let logs = ctx.storage.get_automation_logs(10).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert!(
            !logs[0].success,
            "a blocked command must be logged as a failure, not run"
        );
        let message = logs[0].message.as_deref().unwrap_or_default();
        assert!(
            message.contains("allowlist"),
            "the log must say the allowlist blocked it, got: {message}"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // V2 — broadcasts never reach an unpaired connection
    // ════════════════════════════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v2_broadcast_skips_unpaired_and_keeps_paired() {
        let ctx = create_test_ctx();
        let paired = add_test_paired_client(&ctx, "ws_paired", "dev_paired").await;
        let unpaired = add_test_unpaired_client(&ctx, "ws_eavesdropper").await;
        let mut paired_rx = paired.subscribe();
        let mut unpaired_rx = unpaired.subscribe();

        // The exact content that used to leak: a notification body relayed to
        // every connection, and — because the eavesdropper had no identity and
        // therefore no shared secret — in cleartext.
        broadcast_to_others(
            &ctx,
            "ws_other",
            r#"{"type":"notification","action":"post","body":"my 2FA code is 481920"}"#,
        )
        .await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(300), unpaired_rx.recv()).await;
        assert!(
            leaked.is_err(),
            "an unpaired connection must receive nothing at all"
        );

        let got =
            tokio::time::timeout(std::time::Duration::from_millis(500), paired_rx.recv()).await;
        assert!(got.is_ok(), "a paired connection must still receive it");
        assert!(got.unwrap().unwrap().contains("481920"));
    }

    /// `WsServer::broadcast` is the *fan-out to everyone* entry point (as
    /// opposed to `broadcast_to_others`, which excludes the sender) and had no
    /// caller at all, so its filter was never exercised.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v2_server_broadcast_skips_unpaired_and_keeps_paired() {
        let ctx = create_test_ctx();
        let paired = add_test_paired_client(&ctx, "ws_paired", "dev_paired").await;
        let unpaired = add_test_unpaired_client(&ctx, "ws_eavesdropper").await;
        let mut paired_rx = paired.subscribe();
        let mut unpaired_rx = unpaired.subscribe();

        let server = server_over(ctx);
        server
            .broadcast(r#"{"body":"2FA 481920"}"#.to_string())
            .await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(300), unpaired_rx.recv()).await;
        assert!(
            leaked.is_err(),
            "an unpaired connection must receive nothing"
        );

        let got =
            tokio::time::timeout(std::time::Duration::from_millis(500), paired_rx.recv()).await;
        assert!(got.is_ok(), "a paired connection must still receive it");
        assert!(got.unwrap().unwrap().contains("481920"));
    }

    // ════════════════════════════════════════════════════════════════════════
    // V3 — `send_to` resolves a device id, and encryption is applied once
    // ════════════════════════════════════════════════════════════════════════

    /// Build a `WsServer` around an existing context (the constructor binds a
    /// real port, which a unit test must not do).
    fn server_over(ctx: WsContext) -> WsServer {
        let (_tx, _rx) = broadcast::channel::<()>(1);
        WsServer {
            shutdown_tx: Some(_tx),
            ctx,
            app_handle: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v3_send_to_resolves_a_stable_device_id_to_its_connection() {
        let ctx = create_test_ctx();
        // The connection id (`ws-1`) and the stable device id (`dev_1`) differ —
        // they always do, because the connection id is a fresh UUID per socket.
        let tx = add_test_paired_client(&ctx, "ws-1", "dev_1").await;
        let mut rx = tx.subscribe();

        let server = server_over(ctx);
        let delivered = server.send_to("dev_1", "hello".to_string()).await;
        assert!(
            delivered,
            "send_to must resolve a device id through the pairing registry"
        );
        let got = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the message must reach the device's connection")
            .unwrap();
        assert_eq!(got, "hello");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v3_send_to_still_accepts_a_raw_connection_id() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws-2", "dev_2").await;
        let mut rx = tx.subscribe();

        let server = server_over(ctx);
        assert!(server.send_to("ws-2", "x".to_string()).await);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
                .await
                .is_ok()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v3_send_to_unknown_device_with_no_relay_returns_false() {
        let ctx = create_test_ctx();
        let server = server_over(ctx);
        assert!(
            !server.send_to("no_such_device", "x".to_string()).await,
            "an undeliverable message must be reported, not silently dropped"
        );
    }

    // ── resolve_sender_secret ────────────────────────────────────────────────

    /// Two paired devices with distinct shared secrets, each behind its own
    /// socket. Returns the two secrets so a test can assert *which* one was
    /// resolved — asserting on the device id would not distinguish them, since
    /// the id is the lookup key rather than the answer.
    async fn ctx_with_two_paired() -> (WsContext, String, String) {
        let ctx = create_test_ctx();
        let secret_1 = "a1".repeat(32);
        let secret_2 = "b2".repeat(32);
        for ((device, ws), secret) in [("dev_1", "ws-1"), ("dev_2", "ws-2")]
            .into_iter()
            .zip([secret_1.clone(), secret_2.clone()])
        {
            ctx.sync_engine.write().await.add_client(ConnectedClient {
                device_id: device.to_string(),
                device_name: device.to_string(),
                device_type: "phone".to_string(),
                shared_secret: secret,
                last_heartbeat: 0,
                battery_level: None,
            });
            ctx.ws_to_device_id
                .write()
                .await
                .insert(ws.to_string(), device.to_string());
        }
        (ctx, secret_1, secret_2)
    }

    /// The regression this whole change exists for.
    ///
    /// A peer whose socket is mapped is decrypted with the secret filed under
    /// its *own* identity, no matter what it claims. Before the change, a peer
    /// that had not been told its assigned id stamped a placeholder
    /// (`"mobile"`) into `source_device`, the lookup missed, and the envelope
    /// was dropped with a server-side `warn!` the peer could not see — so the
    /// phone's entire encrypted outbound stream was silently dead while the UI
    /// read "Connected".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_mapped_socket_decrypts_as_itself_even_when_it_claims_a_placeholder() {
        let (ctx, secret_1, _) = ctx_with_two_paired().await;
        assert_eq!(
            WsServer::resolve_sender_secret(&ctx, "ws-1", Some("mobile")).await,
            Some(secret_1),
            "a placeholder claim must not cost the peer its own secret"
        );
    }

    /// One socket, one identity. `ws-1` is paired as `dev_1` and must not be
    /// able to borrow `dev_2`'s secret by naming it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_claim_cannot_override_the_connection_identity() {
        let (ctx, secret_1, _) = ctx_with_two_paired().await;
        assert_eq!(
            WsServer::resolve_sender_secret(&ctx, "ws-1", Some("dev_2")).await,
            Some(secret_1),
            "the connection identity wins; a claim must not borrow another device"
        );
    }

    /// An unmapped socket may still be resolved by its claim, which is how a
    /// paired peer that reconnected over a fresh socket keeps working before it
    /// re-pairs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unmapped_socket_falls_back_to_the_claim() {
        let (ctx, _, secret_2) = ctx_with_two_paired().await;
        assert_eq!(
            WsServer::resolve_sender_secret(&ctx, "ws-new", Some("dev_2")).await,
            Some(secret_2),
            "a socket with no entry of its own may present a claim"
        );
    }

    /// A socket with no pairing entry at all resolves to nothing when it
    /// offers nothing usable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unmapped_socket_with_no_usable_claim_resolves_to_nothing() {
        let (ctx, _, _) = ctx_with_two_paired().await;
        assert!(
            WsServer::resolve_sender_secret(&ctx, "ws-stranger", None)
                .await
                .is_none()
        );
        assert!(
            WsServer::resolve_sender_secret(&ctx, "ws-stranger", Some("nonsense"))
                .await
                .is_none()
        );
    }

    /// An unmapped socket presenting a *real* device id does resolve — and that
    /// is deliberate, not a hole.
    ///
    /// A paired phone that reconnects after a desktop restart lands on a fresh
    /// socket with no `ws_to_device_id` entry. If that were refused, every peer
    /// would have to re-pair from a QR code after every hub restart. Device ids
    /// are not secret anyway: they ride along in `discovery/announce`.
    ///
    /// So this is a secret *lookup*, not an authentication decision. The caller
    /// still has to produce a valid HMAC over the ciphertext with whatever
    /// comes back, and an attacker who is not in possession of a device's
    /// shared secret cannot forge one. Naming a device you are not buys you
    /// nothing — which is exactly what the test above pins.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reconnecting_peer_resolves_by_claim_without_re_pairing() {
        let (ctx, secret_1, _) = ctx_with_two_paired().await;
        assert_eq!(
            WsServer::resolve_sender_secret(&ctx, "ws-fresh", Some("dev_1")).await,
            Some(secret_1),
            "a paired peer reconnecting on a new socket must not have to re-pair"
        );
    }

    /// A mapped socket pointing at a device the hub has since forgotten (the
    /// desktop's database was reset, say) must fall back to the claim rather
    /// than resolving to nothing, so a re-paired peer recovers instead of
    /// going permanently silent.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stale_connection_mapping_does_not_strand_the_peer() {
        let ctx = create_test_ctx();
        let secret = "33".repeat(32);
        ctx.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "dev_new".to_string(),
            device_name: "Peer".to_string(),
            device_type: "phone".to_string(),
            shared_secret: secret.clone(),
            last_heartbeat: 0,
            battery_level: None,
        });
        // Mapped to a device that no longer exists.
        ctx.ws_to_device_id
            .write()
            .await
            .insert("ws-1".to_string(), "dev_gone".to_string());

        assert_eq!(
            WsServer::resolve_sender_secret(&ctx, "ws-1", Some("dev_new")).await,
            Some(secret),
            "a dangling mapping must not make the peer permanently undecryptable"
        );
    }

    /// REGRESSION: `commands::send_encrypted_message` pre-wraps the payload in
    /// an `EncryptedEnvelope` and hands it to `send_to`, whose per-socket writer
    /// used to encrypt *again*. The peer received `encrypted(encrypted(x))`,
    /// peeled one layer, and dispatched the inner envelope to a handler it had
    /// not registered — a silent drop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v3_an_already_encrypted_envelope_is_not_encrypted_twice() {
        let ctx = create_test_ctx();
        let secret = "11".repeat(32);
        ctx.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "dev_1".to_string(),
            device_name: "Peer".to_string(),
            device_type: "phone".to_string(),
            shared_secret: secret.clone(),
            last_heartbeat: 0,
            battery_level: None,
        });
        ctx.ws_to_device_id
            .write()
            .await
            .insert("ws-1".to_string(), "dev_1".to_string());

        // What `send_encrypted_message` puts on the wire.
        let inner = r#"{"type":"clipboard","action":"sync","content":"secret"}"#;
        let (nonce, ciphertext) = ctx.encryption.encrypt(&secret, inner).unwrap();
        let data_hex = hex::encode(&ciphertext);
        let hmac = ctx.encryption.generate_hmac(&secret, &data_hex).unwrap();
        let envelope = serde_json::to_string(&EncryptedEnvelope {
            msg_type: "encrypted".into(),
            nonce: hex::encode(nonce),
            hmac,
            data: data_hex,
            source_device: Some("hub".into()),
            protocol_version: Some(PROTOCOL_VERSION),
        })
        .unwrap();

        let out = WsServer::seal_for_peer(&ctx, "ws-1", &envelope).await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            parsed["type"], "encrypted",
            "an envelope must pass through unchanged, not be wrapped again"
        );
        // And it still decrypts to exactly one layer of plaintext.
        let decrypted = ctx
            .encryption
            .decrypt(
                &secret,
                &hex::decode(parsed["nonce"].as_str().unwrap()).unwrap(),
                &hex::decode(parsed["data"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
        assert_eq!(decrypted, inner);
    }

    /// A plain frame for a peer *with* a secret is still wrapped exactly once,
    /// so the fix did not accidentally disable encryption.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v3_a_plain_frame_for_a_paired_device_is_still_encrypted() {
        let ctx = create_test_ctx();
        let secret = "22".repeat(32);
        ctx.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "dev_1".to_string(),
            device_name: "Peer".to_string(),
            device_type: "phone".to_string(),
            shared_secret: secret.clone(),
            last_heartbeat: 0,
            battery_level: None,
        });
        ctx.ws_to_device_id
            .write()
            .await
            .insert("ws-1".to_string(), "dev_1".to_string());

        let plain = r#"{"type":"clipboard","action":"sync","content":"secret"}"#;
        let out = WsServer::seal_for_peer(&ctx, "ws-1", plain).await;
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["type"], "encrypted");
        assert!(
            !out.contains("secret"),
            "the plaintext must not be on the wire"
        );
    }

    /// `pairing` frames are what *establish* the secret, so they are never
    /// wrapped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v3_pairing_frames_are_never_encrypted() {
        let ctx = create_test_ctx();
        let secret = "33".repeat(32);
        ctx.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "dev_1".to_string(),
            device_name: "Peer".to_string(),
            device_type: "phone".to_string(),
            shared_secret: secret,
            last_heartbeat: 0,
            battery_level: None,
        });
        ctx.ws_to_device_id
            .write()
            .await
            .insert("ws-1".to_string(), "dev_1".to_string());

        let frame = r#"{"type":"pairing","action":"accept","public_key":"ab"}"#;
        let out = WsServer::seal_for_peer(&ctx, "ws-1", frame).await;
        assert_eq!(out, frame);
    }

    /// A pairing frame is recognised structurally, not by a substring match —
    /// the old `contains("\"type\":\"pairing\"")` test would misfire on an
    /// encrypted payload that happened to contain those bytes.
    #[test]
    fn message_type_is_does_not_match_a_substring() {
        assert!(message_type_is(r#"{"type":"pairing"}"#, "pairing"));
        assert!(!message_type_is(
            r#"{"type":"clipboard","content":"\"type\":\"pairing\""}"#,
            "pairing"
        ));
        assert!(!message_type_is("not json", "pairing"));
    }

    // ════════════════════════════════════════════════════════════════════════
    // V4 — discovery is reachable, `remove` has a handler, failures are answered
    // ════════════════════════════════════════════════════════════════════════

    /// REGRESSION: `discovery` was missing from `VALID_TYPES`, so
    /// `handle_discovery_announce` and its 55 lines were unreachable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v4_discovery_announce_reaches_the_handler() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "discovery", "action": "announce",
                "device_id": "dev_phone", "device_name": "Pixel",
                "device_type": "phone", "os": "android", "version": "1.0.0"
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the announce must be answered")
            .unwrap();
        assert!(
            resp.contains(r#""action":"announce""#) && !resp.contains("Conduit"),
            "expected a discovery announce reply, got: {resp}"
        );
        assert!(resp.contains("ws_port"));
    }

    /// `discovery` must not leak the hub's identity to an unpaired peer — the
    /// exemption it used to have from `requires_auth` is gone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v4_discovery_announce_requires_pairing() {
        let ctx = create_test_ctx();
        let tx = add_test_unpaired_client(&ctx, "ws_stranger").await;
        let mut rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "discovery", "action": "announce",
                "device_id": "d", "device_name": "n", "device_type": "phone",
                "os": "android", "version": "1"
            }))
            .unwrap(),
            "ws_stranger",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the peer must be told why")
            .unwrap();
        assert!(resp.contains("not_authenticated"), "got: {resp}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v4_discovery_remove_is_relayed_to_paired_peers() {
        let ctx = create_test_ctx();
        let sender = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let peer = add_test_paired_client(&ctx, "ws_tablet", "dev_tablet").await;
        let _sender_rx = sender.subscribe();
        let mut peer_rx = peer.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "discovery", "action": "remove", "device_id": "dev_gone"
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let got = tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv())
            .await
            .expect("a paired peer must learn about the removal")
            .unwrap();
        assert!(got.contains("dev_gone"), "got: {got}");
    }

    /// REGRESSION: a validation failure used to be a `warn!` and nothing else,
    /// so the peer could not tell "rejected" from "server gone".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v4_invalid_message_is_answered_with_an_error_frame() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        // `file/request` with a traversal filename fails validation.
        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "file", "action": "request",
                "id": "t1", "name": "../../etc/passwd", "size": 1
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("an invalid message must be answered, not dropped silently")
            .unwrap();
        assert!(resp.contains("invalid_message"), "got: {resp}");
    }

    /// `tv` / `watch` are valid types with no handler; the peer is told so
    /// instead of receiving silence.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v4_unimplemented_message_type_is_answered() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({"type": "tv", "action": "remote"})).unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("an unimplemented type must be answered")
            .unwrap();
        assert!(resp.contains("unsupported_message"), "got: {resp}");
    }

    // ════════════════════════════════════════════════════════════════════════
    // V5 — status validation, read limits, local capability plumbing
    // ════════════════════════════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v5_status_update_reaches_the_paired_device() {
        let ctx = create_test_ctx();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "status", "action": "update", "battery": 42
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let engine = ctx.sync_engine.read().await;
        assert_eq!(
            engine.get_client("dev_phone").unwrap().battery_level,
            Some(42),
            "a status update must be recorded against the stable device id"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v5_status_with_an_unknown_field_is_rejected() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "status", "action": "update", "battery": 42, "sneaky": true
            }))
            .unwrap(),
            "ws_phone",
            &ctx,
        )
        .await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("must be answered")
            .unwrap();
        assert!(resp.contains("invalid_message"), "got: {resp}");
        assert_eq!(
            ctx.sync_engine
                .read()
                .await
                .get_client("dev_phone")
                .unwrap()
                .battery_level,
            None,
            "a rejected status frame must not be partially applied"
        );
    }

    /// The WebSocket limits are pinned, not left at tungstenite's defaults.
    #[test]
    fn v5_websocket_read_limits_are_pinned_to_the_protocol_budget() {
        let config = ws_read_limits();
        assert_eq!(config.max_message_size, Some(security::MAX_MESSAGE_SIZE));
        assert_eq!(config.max_frame_size, Some(security::MAX_FRAME_SIZE));
        assert_eq!(
            config.max_write_buffer_size,
            security::MAX_SEND_BUFFER_BYTES,
            "an unbounded write buffer lets a non-reading client grow server memory"
        );
    }

    // ── local capability plumbing ─────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v5_local_auth_frame_grants_the_identity_end_to_end() {
        let ctx = create_test_ctx();
        let tx = add_test_unpaired_client(&ctx, "webview").await;
        let _rx = tx.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "pairing", "action": "local_auth",
                "token": security::local_capability().token()
            }))
            .unwrap(),
            "webview",
            &ctx,
        )
        .await;

        assert!(handlers::is_trusted_peer(&ctx, "webview").await);
    }

    /// And with the identity, the webview can drive the automation UI again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v5_local_auth_unlocks_automation_for_the_webview_only() {
        let ctx = create_test_ctx();
        let webview = add_test_unpaired_client(&ctx, "webview").await;
        let stranger = add_test_unpaired_client(&ctx, "stranger").await;
        let _webview_rx = webview.subscribe();
        let _stranger_rx = stranger.subscribe();

        WsServer::handle_message(
            &serde_json::to_string(&serde_json::json!({
                "type": "pairing", "action": "local_auth",
                "token": security::local_capability().token()
            }))
            .unwrap(),
            "webview",
            &ctx,
        )
        .await;

        let benign = serde_json::to_string(&serde_json::json!({
            "type": "automation", "action": "rule",
            "rule": {
                "id": "ui_1", "name": "notify", "enabled": true,
                "trigger": { "type": "time", "time": "03:00" },
                "action": { "type": "send_notification", "title": "t", "body": "b" }
            }
        }))
        .unwrap();
        WsServer::handle_message(&benign, "webview", &ctx).await;
        WsServer::handle_message(&benign, "stranger", &ctx).await;

        let ids: Vec<String> = ctx
            .storage
            .get_all_automation_rules()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(
            ids,
            vec!["ui_1".to_string()],
            "only the capability-proved webview may create the rule"
        );
    }

    // -----------------------------------------------------------------
    //  The relay's inbound path
    //
    //  Every test above drives `handle_message` with a `ws_*` client id that
    //  the test itself registered in the pairing registry. The relay connection
    //  does not: it dials the relay as `"relay_server"` and the relay forwards
    //  only the inner payload, so nothing ever puts that id in `ws_to_device_id`.
    //  These tests drive the id the relay actually uses, which is the only way
    //  to see what a phone that is not on the LAN experiences.
    // -----------------------------------------------------------------

    /// The client id the outbound relay connection is dispatched under.
    const RELAY_CLIENT_ID: &str = "relay_server";

    /// A relayed payload is the *inner* message only: the relay has already
    /// verified and discarded the `relay_route` envelope.
    ///
    /// Returns the bytes a relay would actually put on the wire for `inner`.
    async fn as_relay_forwarded_payload(ctx: &WsContext, from_device: &str, inner: &str) -> String {
        WsServer::seal_for_peer(ctx, from_device, inner).await
    }

    /// A phone on another network is paired, and the relay forwards a
    /// notification from it. It must reach storage.
    ///
    /// This is the whole point of the relay. Before `relay_delivery` existed
    /// the payload arrived as a bare `encrypted` envelope attributed to
    /// `relay_server`, which is in no pairing registry, so the auth gate
    /// dropped it and the notification was silently lost.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_notification_from_a_paired_phone_reaches_storage() {
        let ctx = create_test_ctx();
        let secret = "11".repeat(32);
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        ctx.sync_engine
            .write()
            .await
            .add_client(crate::sync::ConnectedClient {
                device_id: "dev_phone".into(),
                device_name: "Phone".into(),
                device_type: "phone".into(),
                shared_secret: secret.clone(),
                last_heartbeat: 0,
                battery_level: None,
            });

        let inner = serde_json::to_string(&serde_json::json!({
            "type": "notification",
            "action": "post",
            "id": "n_relayed",
            "app": "Slack",
            "title": "Relayed",
            "body": "across the internet",
            "timestamp": 1_700_000_000
        }))
        .unwrap();

        // What the relay puts on the wire: the sender's sealed payload, wrapped
        // in the `relay_delivery` envelope the relay emits after verifying the
        // route. Built with the real `RelayDelivery` constructor, so this test
        // cannot drift from what the relay actually produces.
        let sealed = as_relay_forwarded_payload(&ctx, "ws_phone", &inner).await;
        assert!(
            sealed.contains("\"encrypted\""),
            "the forwarded payload is still an encrypted envelope; got {sealed}"
        );
        let delivery = RelayDelivery::new(
            "dev_phone",
            "test-device",
            serde_json::from_str(&sealed).expect("sealed payload is JSON"),
        );
        let wire = serde_json::to_string(&delivery).expect("RelayDelivery serializes");

        WsServer::handle_message(&wire, RELAY_CLIENT_ID, &ctx).await;

        let stored = ctx.storage.get_notifications(10).await.unwrap();
        assert_eq!(
            stored.len(),
            1,
            "a relayed notification from a paired phone must be stored, not dropped"
        );
        assert_eq!(stored[0].id, "n_relayed");
    }

    /// The receiver must learn who the message was from, and must be able to
    /// tell a *different* paired device apart from the one that sent it.
    ///
    /// Attribution is the thing the relay verifies and then used to throw away.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_message_carries_its_sender_to_the_receiver() {
        let ctx = create_test_ctx();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        let delivery = serde_json::to_string(&serde_json::json!({
            "type": "relay_delivery",
            "from_device_id": "dev_phone",
            "to_device_id": "test-device",
            "payload": {
                "type": "clipboard", "action": "sync", "content": "hi"
            }
        }))
        .unwrap();

        let attributed = WsServer::unwrap_relay_delivery(&delivery, &ctx).await;
        assert_eq!(
            attributed.as_ref().map(|(from, _)| from.as_str()),
            Some("dev_phone"),
            "the receiver must be told which device sent this"
        );
        let (_from, body) = attributed.expect("envelope unwraps");
        assert_eq!(
            body.get("type").and_then(|v| v.as_str()),
            Some("clipboard"),
            "the unwrapped body is the original message, not the envelope"
        );
    }

    /// A `relay_delivery` naming a device that is not paired is refused, and
    /// its payload is never dispatched.
    ///
    /// The envelope is attacker-influenced input: the relay stamps
    /// `from_device_id`, but the desktop must still resolve it against its own
    /// registry rather than trusting the string.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_delivery_from_an_unpaired_device_is_refused() {
        let ctx = create_test_ctx();
        // No device named "dev_stranger" is paired.
        let forged = serde_json::to_string(&serde_json::json!({
            "type": "relay_delivery",
            "from_device_id": "dev_stranger",
            "payload": {
                "type": "notification", "action": "post",
                "id": "n_forged", "app": "Slack"
            }
        }))
        .unwrap();

        assert!(
            WsServer::unwrap_relay_delivery(&forged, &ctx)
                .await
                .is_none(),
            "an envelope claiming an unknown sender must not unwrap"
        );

        WsServer::handle_message(&forged, RELAY_CLIENT_ID, &ctx).await;
        assert!(
            ctx.storage.get_notifications(10).await.unwrap().is_empty(),
            "nothing from an unpaired sender may be stored"
        );
    }
    /// A relayed frame must not be dispatchable as if it came from the relay
    /// connection's own identity.
    ///
    /// `"relay_server"` is the id the relay socket is dispatched under. If a
    /// relayed message is fed in under that id *without* being unwrapped, it
    /// inherits whatever authority that id has. It must have none.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_relay_connection_id_is_never_a_trusted_peer() {
        let ctx = create_test_ctx();
        assert!(
            !handlers::is_trusted_peer(&ctx, RELAY_CLIENT_ID).await,
            "the relay socket must not be able to send protected messages as itself"
        );
    }

    // -----------------------------------------------------------------
    //  Clipboard history is written on the way in
    //
    //  `clipboard_history` had exactly one production writer, the
    //  `sync_clipboard` Tauri command, and the webview never calls it:
    //  `useClipboard` posts a raw `clipboard/sync` frame instead. The dispatch
    //  arm was a bare fan-out, so the table stayed empty forever — the UI's
    //  `clipboardItems` comes only from `get_clipboard_history`, which meant a
    //  permanent "Your clipboard is empty", no Clear button, and a dock badge
    //  stuck at 0.
    // -----------------------------------------------------------------

    /// REGRESSION: a `clipboard/sync` from a paired peer is stored, attributed
    /// to that peer, and still relayed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_inbound_clipboard_sync_is_stored_for_the_sending_device() {
        let ctx = create_test_ctx();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let other = add_test_paired_client(&ctx, "ws_tablet", "dev_tablet").await;
        let mut other_rx = other.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync",
            "content": "copied on the phone",
            "mime": "text/plain",
            // A lie: attribution must not come from the payload.
            "source_device": "dev_somebody_else",
            "timestamp": 1_700_000_000
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        let rows = ctx.storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(
            rows.len(),
            1,
            "an inbound clipboard sync must be persisted, not only relayed"
        );
        assert_eq!(rows[0].content, "copied on the phone");
        assert_eq!(rows[0].mime, "text/plain");
        assert_eq!(
            rows[0].source_device, "dev_phone",
            "source_device must come from the pairing registry, not the frame"
        );
        assert!(
            !rows[0].pinned,
            "a remote peer must not be able to pin a row that 'Clear clipboard' \
             then refuses to remove"
        );
        assert_eq!(
            rows[0].timestamp, 1_700_000_000,
            "a sane sender timestamp is kept, so every device sorts the shared \
             clipboard the same way"
        );

        // Persisting must not have replaced relaying.
        let relayed = tokio::time::timeout(std::time::Duration::from_millis(500), other_rx.recv())
            .await
            .expect("the other peer must still receive the sync")
            .unwrap();
        assert!(relayed.contains("copied on the phone"));
    }

    /// REGRESSION, relay path: the same frame from a phone on another network
    /// is stored exactly once, still attributed to the device the relay
    /// authenticated — and not twice, which is what would happen if the relay
    /// egress also persisted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_clipboard_sync_is_stored_exactly_once_for_the_relay_sender() {
        let ctx = create_test_ctx();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let inner = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync",
            "content": "copied on another network",
            "mime": "text/plain",
            "source_device": "dev_impostor"
        }))
        .unwrap();
        let sealed = as_relay_forwarded_payload(&ctx, "ws_phone", &inner).await;
        let delivery = RelayDelivery::new(
            "dev_phone",
            "test-device",
            serde_json::from_str(&sealed).expect("sealed payload is JSON"),
        );
        let wire = serde_json::to_string(&delivery).expect("RelayDelivery serializes");

        WsServer::handle_message(&wire, RELAY_CLIENT_ID, &ctx).await;

        let rows = ctx.storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(
            rows.len(),
            1,
            "one relayed sync must produce exactly one row, not two"
        );
        assert_eq!(rows[0].content, "copied on another network");
        assert_eq!(
            rows[0].source_device, "dev_phone",
            "a relayed sender has no registry entry; the relay-authenticated \
             device id is the identity"
        );
    }

    /// The desktop's own webview has the alias `local_desktop`, which is a
    /// transport identity and not a device. The row must record the desktop's
    /// real device id — the same one the webview puts in the frame, and the one
    /// the UI resolves against.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_desktops_own_copy_is_recorded_under_its_real_device_id() {
        let ctx = create_test_ctx();
        ctx.ws_to_device_id.write().await.insert(
            "ws_webview".to_string(),
            handlers::LOCAL_DESKTOP_ID.to_string(),
        );

        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync",
            "content": "copied on the desktop",
            "mime": "text/plain",
            "source_device": "test-device",
            "timestamp": 1_700_000_000
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_webview", &ctx).await;

        let rows = ctx.storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].source_device, "test-device",
            "the transport alias must not leak into a device column"
        );
    }

    /// A frame with no content is relayed but not stored: an empty row would
    /// render as a blank card.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_clipboard_sync_with_no_content_is_relayed_but_not_stored() {
        let ctx = create_test_ctx();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let other = add_test_paired_client(&ctx, "ws_tablet", "dev_tablet").await;
        let mut other_rx = other.subscribe();

        // `validate_clipboard_message` treats `content` as optional, so this
        // reaches the dispatcher rather than being rejected as malformed.
        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync", "mime": "text/plain"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        assert!(
            ctx.storage
                .get_clipboard_history(10)
                .await
                .unwrap()
                .is_empty(),
            "an empty clipboard frame must not create a blank history row"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), other_rx.recv())
                .await
                .is_ok(),
            "and it must still be relayed"
        );
    }

    /// A peer whose clock is far ahead must not be able to pin its content at
    /// the top of this desktop's history forever: `save_clipboard` prunes by
    /// `timestamp DESC`, so an unbounded future stamp is a retention attack.
    /// Arrival time is used instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_clipboard_sync_with_a_wildly_future_timestamp_uses_arrival_time() {
        let ctx = create_test_ctx();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let far_future = chrono::Utc::now().timestamp() + 86_400 * 30;

        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync",
            "content": "from a broken clock", "mime": "text/plain",
            "timestamp": far_future
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        let rows = ctx.storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].timestamp <= chrono::Utc::now().timestamp() + 1,
            "a 30-day-future timestamp must not be stored, got {}",
            rows[0].timestamp
        );
    }

    /// Clipboard sync off, nothing is stored — the settings gate runs before the
    /// handler, so turning the feature off really does stop the table growing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_clipboard_sync_is_not_stored_when_sync_clipboard_is_off() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("sync_clipboard", "false")
            .await
            .unwrap();
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        let text = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync",
            "content": "should not be kept", "mime": "text/plain"
        }))
        .unwrap();

        WsServer::handle_message(&text, "ws_phone", &ctx).await;

        assert!(
            ctx.storage
                .get_clipboard_history(10)
                .await
                .unwrap()
                .is_empty(),
            "a disabled clipboard sync must not leave history behind"
        );
    }

    // -----------------------------------------------------------------
    //  The relay egress seals what the LAN egress seals
    //
    //  `send_to` is the only desktop->relay egress (`relay_tx` has one reader),
    //  and its relay arm used to drop the caller's JSON straight into the route
    //  payload. The relay socket's writer sends verbatim, and sealing lives in
    //  the per-connection socket writer, so a directed frame reached a phone on
    //  the LAN sealed and the same frame reached the same phone through the
    //  relay in the clear.
    // -----------------------------------------------------------------

    /// The route key `send_to` would sign with. Arbitrary bytes: only the HMAC
    /// over the route cares about them, and these tests assert on the payload,
    /// not the signature.
    fn test_route_key() -> Vec<u8> {
        vec![0x5au8; 32]
    }

    /// A phone with a real shared secret, optionally *not* on this LAN.
    ///
    /// `with_socket = false` is the only situation `send_to`'s relay arm exists
    /// for: the device is paired (so the registry knows its secret) but has no
    /// socket here, so the frame has to travel through the relay.
    async fn ctx_with_phone(secret: &str, with_socket: bool) -> WsContext {
        let ctx = create_test_ctx();
        // Sets up the registry entry and a connection; the secret it installs is
        // overwritten below because these tests need a known one.
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        ctx.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "dev_phone".to_string(),
            device_name: "Phone".to_string(),
            device_type: "phone".to_string(),
            shared_secret: secret.to_string(),
            last_heartbeat: 0,
            battery_level: None,
        });
        if !with_socket {
            ctx.ws_to_device_id.write().await.remove("ws_phone");
            ctx.clients.write().await.remove("ws_phone");
        }
        ctx
    }

    /// Peel one `encrypted` layer, failing the test if it is not there.
    fn open_envelope(ctx: &WsContext, secret: &str, envelope: &Value) -> String {
        assert_eq!(
            envelope["type"], "encrypted",
            "expected an encrypted envelope, got {envelope}"
        );
        let data_hex = envelope["data"].as_str().expect("data");
        assert!(
            ctx.encryption
                .verify_hmac(secret, data_hex, envelope["hmac"].as_str().expect("hmac")),
            "the relayed envelope must carry a valid HMAC"
        );
        ctx.encryption
            .decrypt(
                secret,
                &hex::decode(envelope["nonce"].as_str().expect("nonce")).unwrap(),
                &hex::decode(data_hex).unwrap(),
            )
            .expect("a sealed frame must decrypt with the shared secret")
    }

    /// REGRESSION: a directed `file/chunk` — base64 payload and all — is sealed
    /// before it goes into the relay route.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_directed_frame_is_sealed_on_the_relay_egress() {
        let secret = "3c".repeat(32);
        let ctx = ctx_with_phone(&secret, false).await;
        let server = server_over(ctx.clone());

        let chunk = serde_json::json!({
            "type": "file", "action": "chunk", "id": "t1",
            "index": 3, "total": 9, "data": "c2VjcmV0LWJ5dGVz"
        });
        let wire = server
            .sealed_relay_route("dev_phone", &chunk.to_string(), &test_route_key())
            .await;

        let route: Value = serde_json::from_str(&wire).expect("the route is JSON");
        assert_eq!(route["type"], "relay_route");
        assert_eq!(route["to_device_id"], "dev_phone");
        assert_eq!(
            route["payload"]["type"], "encrypted",
            "the relay route's payload must be sealed, not the caller's JSON"
        );
        assert!(
            !wire.contains("c2VjcmV0LWJ5dGVz"),
            "the base64 chunk must not appear in the clear on the relay wire: {wire}"
        );
        assert_eq!(
            open_envelope(&ctx, &secret, &route["payload"]),
            chunk.to_string(),
            "one layer, and it must be the frame the caller passed"
        );
    }

    /// `commands::pairing::send_encrypted_message` hands `send_to` an envelope
    /// it built itself. Sealing must not add a second layer, or the phone peels
    /// one and dispatches the inner envelope to a handler it has not
    /// registered.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_already_sealed_frame_is_not_sealed_again_on_the_relay_egress() {
        let secret = "4d".repeat(32);
        let ctx = ctx_with_phone(&secret, false).await;
        let server = server_over(ctx.clone());

        let inner = r#"{"type":"call","action":"answer","content":"secret"}"#;
        let sealed = WsServer::seal_for_device(&ctx, "dev_phone", inner).await;
        let wire = server
            .sealed_relay_route("dev_phone", &sealed, &test_route_key())
            .await;

        let route: Value = serde_json::from_str(&wire).expect("the route is JSON");
        assert_eq!(route["payload"]["type"], "encrypted");
        assert_eq!(
            open_envelope(&ctx, &secret, &route["payload"]),
            inner,
            "peeling exactly one layer must yield the original frame, not another \
             envelope"
        );
    }

    /// The point of the fix: `send_to`'s two paths must agree. Same frame, same
    /// peer, two transports — both must be sealed, and both must open to the
    /// same plaintext.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn both_of_send_to_s_egress_paths_seal_the_same_frame() {
        let secret = "6e".repeat(32);
        let ctx = ctx_with_phone(&secret, true).await;
        let server = server_over(ctx.clone());

        let frame = r#"{"type":"file","action":"request","id":"t9","name":"a.txt"}"#;
        let lan = WsServer::seal_for_peer(&ctx, "ws_phone", frame).await;
        let wire = server
            .sealed_relay_route("dev_phone", frame, &test_route_key())
            .await;
        let route: Value = serde_json::from_str(&wire).expect("the route is JSON");
        let lan_value: Value = serde_json::from_str(&lan).expect("the LAN frame is JSON");

        assert_eq!(
            lan_value["type"], "encrypted",
            "the LAN socket writer seals, and always did"
        );
        assert_eq!(
            route["payload"]["type"], "encrypted",
            "the relay egress must seal identically, or the same frame is \
             confidential on one transport and readable on the other"
        );
        assert_eq!(
            open_envelope(&ctx, &secret, &route["payload"]),
            open_envelope(&ctx, &secret, &lan_value),
            "the relay copy and the LAN copy of one frame must open identically"
        );
    }

    /// The relay is not a blind pipe: a `pairing` frame is what establishes the
    /// shared secret, so sealing it would leave the peer unable to derive one.
    /// The exemption has to hold on this egress too.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pairing_frame_is_not_sealed_on_the_relay_egress() {
        let secret = "7f".repeat(32);
        let ctx = ctx_with_phone(&secret, false).await;
        let server = server_over(ctx);

        let frame = r#"{"type":"pairing","action":"accept","public_key":"ab"}"#;
        let wire = server
            .sealed_relay_route("dev_phone", frame, &test_route_key())
            .await;
        let route: Value = serde_json::from_str(&wire).expect("the route is JSON");
        assert_eq!(
            route["payload"]["type"], "pairing",
            "wrapping a pairing frame would make it undecryptable at the peer"
        );
    }

    /// A destination with no shared secret still gets plaintext, exactly as on
    /// the LAN: `seal_for_device` cannot invent a key, and the relay is not the
    /// place to fail closed against a peer we know nothing about.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_frame_for_an_unknown_device_is_still_routed_verbatim() {
        let ctx = create_test_ctx();
        let server = server_over(ctx);
        let frame = r#"{"type":"sms","action":"new","body":"hi"}"#;
        let wire = server
            .sealed_relay_route("dev_stranger", frame, &test_route_key())
            .await;
        let route: Value = serde_json::from_str(&wire).expect("the route is JSON");
        assert_eq!(route["payload"]["type"], "sms");
        assert_eq!(route["payload"]["body"], "hi");
    }

    // -----------------------------------------------------------------
    //  One frame, one count: the transport budget
    //
    //  The global limiter was charged twice for every inbound text frame —
    //  once by the reader, once by `dispatch` — against the same `client_id`
    //  bucket, so the real ceiling was half of `max_messages`. These drive
    //  frames through `admit_frame`, which is the reader's body, and watch
    //  the bucket itself rather than any handler's opinion of the traffic.
    // -----------------------------------------------------------------

    /// A context whose transport budget is a known, tiny number, so a test can
    /// name the frame at which a connection must run out.
    fn ctx_with_transport_budget(max_messages: u32, max_bytes: u64) -> WsContext {
        let mut ctx = create_test_ctx();
        ctx.rate_limiter = Arc::new(RateLimiter::new(security::RateLimitConfig {
            max_messages,
            max_bytes,
            window: std::time::Duration::from_secs(60),
        }));
        ctx
    }

    fn text_frame(msg: serde_json::Value) -> Message {
        Message::Text(
            serde_json::to_string(&msg)
                .expect("frame serializes")
                .into(),
        )
    }

    /// REGRESSION: one inbound text frame costs exactly one count.
    ///
    /// Asserted against the *bucket*, not against a handler: with the second
    /// charge site in place, `budget` frames moved the counter by `2 × budget`
    /// and the connection was cut off at frame `budget / 2` — which is how a
    /// 100/10 s ceiling became ~5 msg/s and killed mirroring, audio and file
    /// pushes alike.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_inbound_text_frame_costs_exactly_one_count() {
        let budget = 8;
        let ctx = ctx_with_transport_budget(budget, 1 << 20);
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        // `ping` is exempt from the auth gate, so it reaches `dispatch` and is
        // charged by every site that has ever existed.
        for i in 0..budget {
            assert!(
                WsServer::admit_frame(
                    &ctx,
                    "ws_phone",
                    text_frame(serde_json::json!({
                        "type": "ping", "seq": i
                    }))
                )
                .await,
                "frame {} of a {budget}-frame budget must be admitted; a second \
                 charge site runs the connection dry at {}",
                i + 1,
                budget / 2
            );
        }
        assert!(
            !WsServer::admit_frame(
                &ctx,
                "ws_phone",
                text_frame(serde_json::json!({"type": "ping", "seq": budget}))
            )
            .await,
            "the first frame past the budget must be refused, so the budget is \
             still a budget and not a formality"
        );
        assert_eq!(
            ctx.rate_limiter.window_messages("ws_phone").await,
            budget + 1,
            "{budget} admitted frames plus one refused must cost {} counts in \
             total — one each, and no more",
            budget + 1
        );
    }

    /// REGRESSION: a frame this desktop *refuses* still costs budget.
    ///
    /// Both halves matter, because they die in different places. An unparsable
    /// frame dies at `serde_json::from_str`; a well-formed frame from an
    /// unpaired peer dies at the auth gate. Both are *before* any handler, and
    /// both would be free if the limiter only counted admitted messages — so a
    /// peer could buy unlimited parse attempts by making the server reject its
    /// traffic.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rejected_frames_still_consume_the_transport_budget() {
        let budget = 4;
        let ctx = ctx_with_transport_budget(budget, 1 << 20);
        // No pairing: every well-formed frame below is refused by the auth gate.
        let tx = add_test_unpaired_client(&ctx, "ws_stranger").await;
        let mut rx = tx.subscribe();

        for i in 0..budget {
            let frame = if i % 2 == 0 {
                // Truncated JSON: never reaches `dispatch` at all.
                Message::Text("{\"type\":".to_string().into())
            } else {
                // Valid, and refused by the auth gate.
                text_frame(serde_json::json!({
                    "type": "file", "action": "chunk", "id": "t1",
                    "index": 0, "data": "AA=="
                }))
            };
            assert!(
                WsServer::admit_frame(&ctx, "ws_stranger", frame).await,
                "refused frame {i} must still be within budget; the charge \
                 happens before anything can reject it"
            );
        }
        assert!(
            !WsServer::admit_frame(
                &ctx,
                "ws_stranger",
                text_frame(serde_json::json!({"type": "ping"}))
            )
            .await,
            "a flood of frames this desktop refuses must still exhaust the \
             budget — otherwise refusal is free"
        );
        assert_eq!(
            ctx.rate_limiter.window_messages("ws_stranger").await,
            budget + 1,
            "every refused frame must be charged too"
        );
        // The frames really were refused for the *auth* reason while budget
        // remained, not silently dropped by the budget.
        let answered = rx.try_recv().expect("a refused frame must be answered");
        assert!(
            answered.contains("not_authenticated"),
            "the first refusal must carry a reason the peer can read, got {answered}"
        );
    }

    /// REGRESSION: ten seconds of screen mirroring fits one transport window.
    ///
    /// 30 fps × 2 messages is the documented ~60/s (15–30 frames/s measured), so
    /// a ten-second window holds ~600 frames. They go in back to back with no
    /// delay at all, which is strictly harder than the real stream: this asks
    /// only whether the *transport* budget ends it. (Whether the per-type
    /// budget would admit 600 inside a single millisecond is a different
    /// question, answered by `security`'s own 120/s streaming tests.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ten_seconds_of_screen_mirroring_survives_the_transport_budget() {
        let ctx = create_test_ctx(); // the production RateLimitConfig::default()
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        let frames = 60 * security::RateLimitConfig::default().window.as_secs();
        for i in 0..frames {
            assert!(
                WsServer::admit_frame(
                    &ctx,
                    "ws_phone",
                    text_frame(serde_json::json!({
                        "type": "screen_mirror", "action": "frame",
                        "data": "AAAA", "seq": i
                    }))
                )
                .await,
                "mirror frame {} of a ~60/s ten-second stream must not be \
                 dropped by the transport budget",
                i + 1
            );
        }
    }

    /// REGRESSION, and the same question for the third stream: a phone→desktop
    /// push is 64 KiB chunks at ~47/s, which the *byte* budget was explicitly
    /// sized to admit (~3 MB/s). The count budget has to admit it too, or the
    /// transfer stalls on a ceiling nobody was looking at.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_realistic_file_chunk_stream_survives_the_transport_budget() {
        let ctx = create_test_ctx();
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        // 64 KiB of payload, base64-encoded as the protocol carries it.
        use base64::Engine;
        let payload = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 64 * 1024]);
        let chunks = 47 * security::RateLimitConfig::default().window.as_secs();
        for i in 0..chunks {
            assert!(
                WsServer::admit_frame(
                    &ctx,
                    "ws_phone",
                    text_frame(serde_json::json!({
                        "type": "file", "action": "chunk", "id": "t1",
                        "index": i, "data": payload
                    }))
                )
                .await,
                "chunk {i} of a 64 KiB push must not be dropped by the \
                 transport budget"
            );
        }

        // End to end: a push this size must never be told to slow down.
        let mut throttled = false;
        while let Ok(frame) = rx.try_recv() {
            throttled |= frame.contains("rate_limited");
        }
        assert!(
            !throttled,
            "a realistic file push must not be refused by any limiter, including \
             the per-type one"
        );
    }

    /// Binary frames are charged once, like text frames — and, unlike text
    /// frames, they are charged **no** per-type budget.
    ///
    /// A binary frame has no `type` field until `handlers::files` decodes it as
    /// a chunk envelope, which is after the dispatcher's per-type check, so
    /// there is nothing to key a bucket on and `check_type_action_limit` is
    /// never called for one. That is a real gap — the transport budget is the
    /// only thing bounding a binary flood — and it is *reported* rather than
    /// closed here, because the per-type `file` budget (3 000 / 60 s ≈ 50/s) sits
    /// right on top of the documented chunk rate and applying it to this path is
    /// a DoS-policy decision, not a lookup. Both halves are pinned here so the
    /// "once" cannot regress silently and the gap stays visible.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_frames_are_charged_once_and_never_per_type_limited() {
        let budget = 6;
        let ctx = ctx_with_transport_budget(budget, 1 << 20);
        add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;

        // A short payload: too small to be a chunk envelope, so the handler
        // logs and returns without touching the disk.
        for i in 0..budget {
            assert!(
                WsServer::admit_frame(&ctx, "ws_phone", Message::Binary(vec![0u8; 4].into())).await,
                "binary frame {} of a {budget}-frame budget must be admitted",
                i + 1
            );
        }
        assert!(
            !WsServer::admit_frame(&ctx, "ws_phone", Message::Binary(vec![0u8; 4].into())).await,
            "and the frame past the budget must be refused"
        );
        assert_eq!(
            ctx.rate_limiter.window_messages("ws_phone").await,
            budget + 1,
            "a binary frame costs one count, like a text frame"
        );

        // Spend the whole per-type `file` budget, then show a binary frame is
        // unaffected by it: it never reaches the per-type limiter at all.
        let mut spent = 0;
        while ctx
            .per_type_limiter
            .check_type_limit("ws_phone", "file")
            .await
        {
            spent += 1;
            assert!(
                spent <= 10_000,
                "the per-type file budget must stay bounded"
            );
        }
        assert!(
            WsServer::admit_frame(&ctx, "ws_other", Message::Binary(vec![0u8; 4].into())).await,
            "a binary frame must not draw on the per-type `file` budget: it \
             never reaches `dispatch` ({spent} text chunks went to that bucket)"
        );
    }

    // -----------------------------------------------------------------
    //  Answering a sender that is not a connection id
    //
    //  `ctx.clients` is keyed by connection id. A relayed sender's `client_id`
    //  is the device id the relay authenticated, so every refusal addressed to
    //  it missed the map and vanished.
    // -----------------------------------------------------------------

    /// REGRESSION: a throttled relayed sender is told, not just dropped.
    ///
    /// Dispatched under its **device** id, exactly as `handle_message` does
    /// after unwrapping a `relay_delivery`. `pairing` is the cheapest budget to
    /// exhaust (5/min) and it is exempt from the auth gate, so this drives the
    /// real `rate_limited` arm without needing a stream to run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_dispatched_sender_is_told_when_it_is_rate_limited() {
        let ctx = create_test_ctx();
        // The device's socket is registered under its connection id, as
        // `accept_loop` does; the id it is *dispatched* under is the device id.
        let tx = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut rx = tx.subscribe();

        let attempt = r#"{"type":"pairing","action":"request","token":"NOT-A-REAL-TOKEN"}"#;
        for _ in 0..5 {
            WsServer::handle_message(attempt, "dev_phone", &ctx).await;
        }
        WsServer::handle_message(attempt, "dev_phone", &ctx).await;

        let answered = rx
            .try_recv()
            .expect("a sender throttled under its device id must be answered");
        assert!(
            answered.contains("rate_limited"),
            "expected a rate_limited error for the device-id sender, got {answered}"
        );
    }

    /// `answer_channel` still answers only sockets, and that is not a limitation
    /// any more — it is the deliberate half of the answer. Split out from the
    /// test that used to sit here so the two claims cannot be confused: "this
    /// map cannot reach a relay-only device" was true and stayed true; "so
    /// nothing can" was the bug.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn answer_channel_still_answers_sockets_only() {
        let ctx = create_test_ctx();
        ctx.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "dev_phone".to_string(),
            device_name: "Phone".to_string(),
            device_type: "phone".to_string(),
            shared_secret: "22".repeat(32),
            last_heartbeat: 0,
            battery_level: None,
        });
        assert!(
            answer_channel(&ctx, "dev_phone").await.is_none(),
            "a device with no socket has no channel — `answer` is what routes \
             around that, not a widened lookup"
        );
        assert!(
            answer_channel(&ctx, "ws_nobody").await.is_none(),
            "an unknown connection id has no channel either"
        );
    }

    // ======================================================================
    //  The relay is a destination, not a connection
    //
    //  `broadcast_to_others` and `WsServer::broadcast` both walked `ctx.clients`,
    //  and the relay socket is outbound-only: `relay_tx` is an `mpsc::Sender`,
    //  never a connection map entry. So a phone reachable only through the relay
    //  received nothing from any of the seventeen handlers that fan out —
    //  clipboard, notifications, SMS, calls, discovery/remove, and every
    //  file-transfer control frame — and it failed *silently*, because
    //  iterating an empty set is indistinguishable from success.
    //
    //  Directed sends already worked: `send_to` has had a relay arm. That
    //  asymmetry is what made the feature look intermittent rather than broken.
    // ======================================================================

    const RELAY_PHONE_SECRET: &str =
        "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899";

    /// The flagship scenario, and the one the fix would silently skip: pair on
    /// the LAN, then walk out of range. The device is in the registry and has
    /// **no socket here**.
    ///
    /// That "no socket" part is the whole difficulty. `SyncEngine` — the map the
    /// old code would have enumerated — has the device removed from it the moment
    /// its LAN socket closes (`handle_client`), so a fan-out built on
    /// `SyncEngine` would produce an empty recipient set for precisely this case
    /// and appear to work in a test that models the device as connected.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_only_peer_receives_a_broadcast() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", RELAY_PHONE_SECRET)]).await;

        let message = r#"{"type":"sms","action":"new","body":"481920"}"#;
        broadcast_to_others(&ctx, "ws_desktop", message).await;

        let route = th::only_relay_frame(&mut relay_rx);
        assert_eq!(route["type"], "relay_route");
        assert_eq!(
            route["to_device_id"], "dev_phone",
            "the route must name the device that has no socket here"
        );
        assert_eq!(
            open_envelope(&ctx, RELAY_PHONE_SECRET, &route["payload"]),
            message,
            "and the phone must be able to open exactly what a LAN peer would"
        );
    }

    /// A device with **both** a socket and a relay route is one device, and must
    /// be served once.
    ///
    /// This is not hypothetical: it is exactly what a phone looks like during a
    /// network transition, and it is the failure mode a fan-out fix introduces
    /// most easily. Twice is worse than never — duplicate notifications,
    /// duplicate clipboard rows, and duplicated file chunks corrupting a
    /// transfer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_device_with_both_a_socket_and_a_relay_route_is_served_once() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", RELAY_PHONE_SECRET)]).await;
        // Same device, now also on the LAN.
        let phone = add_test_paired_client(&ctx, "ws_phone", "dev_phone").await;
        let mut phone_rx = phone.subscribe();

        broadcast_to_others(&ctx, "ws_desktop", r#"{"type":"call","action":"ring"}"#).await;

        assert!(
            phone_rx.try_recv().is_ok(),
            "the socket is the cheaper route and must still be served"
        );
        th::assert_relay_silent(&mut relay_rx);
    }

    /// The desktop registers itself under its own id
    /// (`DeviceRouteKeys::refresh`), so an unfiltered recipient enumeration has
    /// it route a frame to itself — which the relay delivers straight back,
    /// `unwrap_relay_delivery` accepts, `dispatch` re-enters, and *that* fans out
    /// again. Multiplicative, not linear, until the 1024-slot queue fills and
    /// then the hub-wide stalls with it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_desktop_is_never_its_own_relay_fan_out_recipient() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[]).await;
        assert!(
            ctx.route_keys
                .routable_device_ids()
                .iter()
                .any(|id| id == ctx.device_id.as_str()),
            "the fixture must really have registered this desktop, or the \
             exclusion below is untested"
        );

        broadcast_to_others(&ctx, "ws_desktop", r#"{"type":"call","action":"ring"}"#).await;

        th::assert_relay_silent(&mut relay_rx);
    }

    /// The sender exclusion has to be by **device id**, because that is what the
    /// relay addresses — and for a relayed sender `sender_id` *is* the device id.
    ///
    /// Comparing the raw string against device ids would exclude nothing here
    /// and hand the phone its own notification back. A phone that echoes its own
    /// SMS into the desktop's clipboard history is a support ticket nobody
    /// enjoys.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_sender_is_not_routed_its_own_broadcast_back() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(
            &ctx,
            &[
                ("dev_sender", &"11".repeat(32)),
                ("dev_other", &"22".repeat(32)),
            ],
        )
        .await;

        let inner = serde_json::to_string(&serde_json::json!({
            "type": "clipboard", "action": "sync", "content": "secret", "mime": "text/plain"
        }))
        .unwrap();
        let wire = relay_wire(&ctx, "dev_sender", &inner).await;
        WsServer::handle_message(&wire, WsServer::RELAY_CLIENT_ID, &ctx).await;

        // The frame really was dispatched — otherwise "nothing routed" is
        // trivially true because nothing happened.
        assert_eq!(
            ctx.storage.get_clipboard_history(10).await.unwrap().len(),
            1,
            "the relayed sync must have been handled, or this test proves nothing"
        );
        let route = th::only_relay_frame(&mut relay_rx);
        assert_eq!(
            route["to_device_id"], "dev_other",
            "the only other peer must get it; the sender must not"
        );
    }

    /// A relay-only frame must be **sealed**, not routed verbatim.
    ///
    /// The egress used to read the recipient's secret from `SyncEngine`, which
    /// is a *liveness* map — the device is absent from it precisely when it is
    /// relay-only — so every frame routed to that device fell through to
    /// plaintext. That was already true of directed `file/chunk` frames, base64
    /// payload and all, and it would have become true of clipboard and SMS
    /// bodies the moment fan-out learned to reach the device.
    ///
    /// So this asserts the property that made the fix safe to ship, not just
    /// that it works.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_only_frame_is_sealed_and_never_routed_in_the_clear() {
        let ctx = create_test_ctx();
        let secret = RELAY_PHONE_SECRET;
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", secret)]).await;

        let message = r#"{"type":"clipboard","action":"sync","content":"my 2FA code is 481920"}"#;
        broadcast_to_others(&ctx, "ws_desktop", message).await;

        let route = th::only_relay_frame(&mut relay_rx);
        assert_eq!(
            route["payload"]["type"], "encrypted",
            "a relay-only recipient has no socket, so `seal_for_peer` never runs \
             for it; the relay egress must seal by device id or it sends cleartext"
        );
        assert!(
            !serde_json::to_string(&route).unwrap().contains("481920"),
            "the clipboard body must not appear on the relay wire in the clear"
        );
        assert_eq!(open_envelope(&ctx, secret, &route["payload"]), message);
    }

    /// The pairing filter that `broadcast_to_others` exists to enforce must hold
    /// on the relay arm too, or the plaintext-leak class this function was
    /// written for comes straight back through the second transport.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unpaired_or_revoked_device_is_never_a_fan_out_recipient() {
        let ctx = create_test_ctx();
        // A row that exists but is not paired, with a perfectly usable secret.
        ctx.storage
            .save_device(&crate::storage::StoredDevice {
                id: "dev_revoked".to_string(),
                name: "Old phone".to_string(),
                device_type: "mobile".to_string(),
                os: "android".to_string(),
                public_key: "00".repeat(32),
                shared_secret: "33".repeat(32),
                paired_at: 0,
                last_seen: 0,
                battery: None,
                signal: None,
                status: "revoked".to_string(),
            })
            .await
            .unwrap();
        let mut relay_rx = th::add_test_relay(&ctx, &[]).await;
        assert!(
            !ctx.route_keys
                .routable_device_ids()
                .contains(&"dev_revoked".to_string()),
            "a revoked row must not be routable at all"
        );

        broadcast_to_others(
            &ctx,
            "ws_desktop",
            r#"{"type":"sms","action":"new","body":"x"}"#,
        )
        .await;

        th::assert_relay_silent(&mut relay_rx);
    }

    // -----------------------------------------------------------------
    //  One relayed frame, one count — against the device, not the socket
    //
    //  The audit recorded this as "every phone behind the relay shares one
    //  transport bucket keyed on the literal `"relay_server"`, so one file
    //  transfer throttles every other phone". The mechanism was wrong and the
    //  defect was worse: `admit_frame` has exactly one caller — the LAN reader —
    //  and the relay read loop calls `handle_message` directly, so the
    //  `"relay_server"` bucket was **never created**. Relayed frames were charged
    //  no count *and no bytes* by any phone, at all.
    //
    //  These tests pin the corrected invariant, which is strictly more accounting
    //  than before rather than a repartition of an existing charge.
    // -----------------------------------------------------------------

    /// What a relay actually puts on the wire for one frame from `sender`: the
    /// sender's sealed payload inside the `relay_delivery` envelope the relay
    /// emits after verifying the route.
    ///
    /// Sealed by **device** id, the way the relay egress seals, so a sender with
    /// no socket here still produces a real envelope. (Its predecessor went
    /// through `seal_for_peer`, which returns plaintext for an unmapped id — so
    /// it only ever sealed because both callers happened to pass a connection
    /// id.)
    async fn relay_wire(ctx: &WsContext, sender: &str, inner: &str) -> String {
        let sealed = WsServer::seal_for_device(ctx, sender, inner).await;
        serde_json::to_string(&RelayDelivery::new(
            sender,
            "test-device",
            serde_json::from_str(&sealed).expect("sealed payload is JSON"),
        ))
        .expect("RelayDelivery serializes")
    }

    fn relayed_notification(id: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "type": "notification", "action": "post", "id": id,
            "app": "Slack", "title": "t", "body": "b", "timestamp": 1_700_000_000
        }))
        .unwrap()
    }

    /// REGRESSION: a relayed frame is charged to its **sender's** device budget,
    /// once — and to the transport *not* at all, which is what makes this a
    /// charge rather than the double-charge `admit_frame`'s doc warns about.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_frame_costs_exactly_one_count_on_its_senders_budget() {
        let budget = 6u32;
        let ctx = ctx_with_transport_budget(budget, 1 << 20);
        th::add_test_relay(&ctx, &[("dev_phone", RELAY_PHONE_SECRET)]).await;

        for i in 0..budget {
            let wire = relay_wire(&ctx, "dev_phone", &relayed_notification(&format!("n{i}"))).await;
            WsServer::handle_message(&wire, WsServer::RELAY_CLIENT_ID, &ctx).await;
        }
        assert_eq!(
            ctx.rate_limiter.window_messages("dev_phone").await,
            budget,
            "{budget} relayed frames must cost {budget} counts on the sender's \
             budget — one each, and no more"
        );
        assert_eq!(
            ctx.rate_limiter
                .window_messages(WsServer::RELAY_CLIENT_ID)
                .await,
            0,
            "the transport must not be charged for a frame a device can be named \
             for, or every peer is throttled by every other one"
        );
        assert_eq!(
            ctx.storage.get_notifications(50).await.unwrap().len(),
            budget as usize,
            "and every one of them must actually have been handled"
        );
    }

    /// The fairness property the audit was reaching for, and the one that makes
    /// per-device charging worth having: one relayed peer's budget is its own.
    ///
    /// Before the fix this was unreachable in the *good* direction — no peer was
    /// charged anything — so a file transfer could not throttle a sibling. The
    /// risk of the fix is that it reintroduces the sharing, in the other
    /// direction. This is the test that says it did not.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_relay_peer_exhausting_its_budget_does_not_throttle_another() {
        let budget = 2u32;
        let ctx = ctx_with_transport_budget(budget, 1 << 20);
        th::add_test_relay(
            &ctx,
            &[
                ("dev_noisy", &"11".repeat(32)),
                ("dev_quiet", &"22".repeat(32)),
            ],
        )
        .await;

        // The noisy peer saturates its own budget, and the frame past it is
        // dropped.
        for i in 0..=budget {
            let wire = relay_wire(
                &ctx,
                "dev_noisy",
                &relayed_notification(&format!("noisy{i}")),
            )
            .await;
            WsServer::handle_message(&wire, WsServer::RELAY_CLIENT_ID, &ctx).await;
        }
        assert_eq!(
            ctx.storage.get_notifications(50).await.unwrap().len(),
            budget as usize,
            "the frame past the budget must be dropped, or this is not a budget"
        );

        // The quiet peer then gets through on its own untouched budget. This is
        // the assertion that would fail if the charge were still keyed on the
        // transport.
        WsServer::handle_message(
            &relay_wire(&ctx, "dev_quiet", &relayed_notification("quiet")).await,
            WsServer::RELAY_CLIENT_ID,
            &ctx,
        )
        .await;

        assert_eq!(
            ctx.rate_limiter.window_messages("dev_quiet").await,
            1,
            "a second peer must not be charged for the first one's traffic"
        );
        assert!(
            ctx.storage
                .get_notifications(50)
                .await
                .unwrap()
                .iter()
                .any(|n| n.id == "quiet"),
            "a peer the other one could not throttle must still be served"
        );
    }

    /// A frame this desktop *refuses* still costs its sender. Both halves have to
    /// hold or the budget can be evaded by making the server reject your traffic
    /// — the same reason `rejected_frames_still_consume_the_transport_budget`
    /// exists for the LAN reader.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_frame_this_desktop_refuses_still_costs_its_sender() {
        let ctx = ctx_with_transport_budget(10, 1 << 20);
        th::add_test_relay(&ctx, &[("dev_phone", RELAY_PHONE_SECRET)]).await;

        // A name the file handler rejects, so the frame is refused after the
        // budget and before the handler does anything useful.
        let inner = serde_json::to_string(&serde_json::json!({
            "type": "file", "action": "request", "id": "t1", "name": "../escape"
        }))
        .unwrap();
        WsServer::handle_message(
            &relay_wire(&ctx, "dev_phone", &inner).await,
            WsServer::RELAY_CLIENT_ID,
            &ctx,
        )
        .await;

        assert_eq!(
            ctx.rate_limiter.window_messages("dev_phone").await,
            1,
            "a refused relayed frame must still be charged to the peer that sent it"
        );
    }

    /// A relayed frame whose sender cannot be named — malformed, or claiming an
    /// id this desktop has not paired — is charged to the transport instead, so
    /// "refusal is free" does not become true on this path either. These are
    /// precisely the frames that never reach a handler at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unattributable_relayed_frame_is_charged_to_the_transport() {
        let ctx = ctx_with_transport_budget(10, 1 << 20);
        th::add_test_relay(&ctx, &[]).await;

        for i in 0..3 {
            let forged = serde_json::to_string(&serde_json::json!({
                "type": "relay_delivery",
                "from_device_id": format!("dev_stranger{i}"),
                "payload": {"type": "sms", "action": "new", "body": "x"}
            }))
            .unwrap();
            WsServer::handle_message(&forged, WsServer::RELAY_CLIENT_ID, &ctx).await;
        }

        assert_eq!(
            ctx.rate_limiter
                .window_messages(WsServer::RELAY_CLIENT_ID)
                .await,
            3,
            "frames from senders this desktop does not know must still cost \
             something, or an unauthenticated peer gets unlimited parse attempts"
        );
    }

    // -----------------------------------------------------------------
    //  Telling a relay-only peer why it was refused
    //
    //  `answer_channel` resolved a device id through the pairing registry, so it
    //  reached a relayed sender that *also* had a socket and missed every sender
    //  that did not. Every refusal — `rate_limited`, `invalid_message`,
    //  `not_authenticated`, a settings gate — was then looked up, missed, and
    //  dropped. The peer was refused and never told why, which from the far end
    //  is indistinguishable from a dropped socket.
    //
    //  Nothing about the wire format had to change: `relay_route`'s payload is an
    //  arbitrary JSON message and the relay forwards it without inspecting it,
    //  so an `error` was always a legal thing to route. The route simply was not
    //  being taken. The previous version of this test asserted the *absence* of
    //  the capability and called closing it "a protocol and relay-side decision".
    // -----------------------------------------------------------------

    /// REGRESSION, and the replacement for the test that used to pin this as
    /// unfixable: a phone that is only reachable through the relay is told why it
    /// was refused, in the same signed route every other relayed frame uses.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_only_sender_is_told_why_it_was_refused() {
        let ctx = create_test_ctx();
        let secret = RELAY_PHONE_SECRET;
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", secret)]).await;

        // `pairing` is exempt from the auth gate and has the cheapest budget to
        // exhaust (5/min), so this drives the real `rate_limited` arm without
        // needing a stream to run.
        let attempt = r#"{"type":"pairing","action":"request","token":"NOT-A-REAL-TOKEN"}"#;
        for _ in 0..5 {
            let wire = relay_wire(&ctx, "dev_phone", attempt).await;
            WsServer::handle_message(&wire, WsServer::RELAY_CLIENT_ID, &ctx).await;
        }
        let wire = relay_wire(&ctx, "dev_phone", attempt).await;
        WsServer::handle_message(&wire, WsServer::RELAY_CLIENT_ID, &ctx).await;

        let route = th::only_relay_frame(&mut relay_rx);
        assert_eq!(
            route["to_device_id"], "dev_phone",
            "the refusal must be addressed to the device that was refused"
        );
        let inner = open_envelope(&ctx, secret, &route["payload"]);
        assert!(
            inner.contains("rate_limited"),
            "the peer must be told it was throttled, not left guessing: {inner}"
        );
    }

    /// The same shape for a `ping`, because `handle_message` dispatches a
    /// relayed delivery under its **device id**, which is in no connection map.
    /// A relay-only peer could not even be probed for reachability.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_only_device_gets_its_pong() {
        let ctx = create_test_ctx();
        let secret = RELAY_PHONE_SECRET;
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", secret)]).await;

        let wire = relay_wire(&ctx, "dev_phone", r#"{"type":"ping","seq":1}"#).await;
        WsServer::handle_message(&wire, WsServer::RELAY_CLIENT_ID, &ctx).await;

        let route = th::only_relay_frame(&mut relay_rx);
        let inner = open_envelope(&ctx, secret, &route["payload"]);
        assert!(
            inner.contains("pong"),
            "a relay-only device must still be probeable: {inner}"
        );
    }

    /// A refusal aimed at an id that is not a paired device gets **no** signed
    /// route.
    ///
    /// This does not fail before the fix — before the fix it also produced
    /// nothing, for the wrong reason (the code simply dropped it). It is here
    /// because the fix turns a previously-harmless drop into a pre-auth egress
    /// that signs and encrypts on demand: `send_error` is reachable from
    /// `not_authenticated`, so without the `is_trusted_peer` gate in
    /// `relay_route_to` a stranger could make this desktop mint a signed, sealed
    /// route per refused frame just by sending junk.
    ///
    /// Labelled as the guard it is, rather than dressed up as a regression.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refusal_for_an_unknown_id_is_not_given_a_signed_relay_route() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[]).await;

        send_error(&ctx, "ws_stranger", "not_authenticated", "nope").await;
        th::assert_relay_silent(&mut relay_rx);

        // And the transport id must never acquire peer authority, which is what
        // would let a `relay_auth_rejected` frame on the relay socket mint one.
        assert!(
            !WsServer::RELAY_CLIENT_ID.eq(ctx.device_id.as_str()),
            "the transport id must not collide with this desktop's own id"
        );
        send_error(&ctx, WsServer::RELAY_CLIENT_ID, "invalid_message", "nope").await;
        th::assert_relay_silent(&mut relay_rx);
    }

    /// A relayed **binary** frame names its recipient, not its sender, so there is
    /// no device id to charge a per-sender budget against — but the transport
    /// budget is the only thing standing between that socket and a binary flood,
    /// so it is charged. It used not to be: the text arm was charged and the
    /// binary one was not, which made the cheapest flood the unaccounted one.
    ///
    /// Drives [`WsServer::admit_relay_binary`], which is the same function
    /// `spawn_relay_client`'s read loop calls — not a restatement of what the
    /// limiter does, which would pass whether or not the loop called it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_binary_frame_is_charged_to_the_transport_budget() {
        let budget = 3u32;
        let ctx = ctx_with_transport_budget(budget, 1 << 20);
        th::add_test_relay(&ctx, &[]).await;

        for i in 0..budget {
            assert!(
                WsServer::admit_relay_binary(&ctx, &[0u8; 4]).await,
                "relayed binary frame {} of a {budget}-frame budget must be admitted",
                i + 1
            );
        }
        assert!(
            !WsServer::admit_relay_binary(&ctx, &[0u8; 4]).await,
            "and the frame past the budget must be refused, or the budget is a \
             formality and a binary flood is unmetered"
        );
        assert_eq!(
            ctx.rate_limiter
                .window_messages(WsServer::RELAY_CLIENT_ID)
                .await,
            budget + 1,
            "a relayed binary frame costs one count on the transport, like a \
             relayed text frame costs one on the sender"
        );
    }

    /// The pairing filter has to hold on the relay arm too, or the plaintext-leak
    /// class `broadcast_to_others` was written for comes straight back through
    /// the second transport.
    ///
    /// Note what is *not* asserted: that `refresh()` skips a
    /// `status != "paired"` row. That is the filter's job and
    /// `relay::tests::an_unpaired_or_revoked_device_cannot_route` already pins it
    /// against the real registry. Restating it here would only test the fixture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_fan_out_recipient_set_is_exactly_the_routable_devices() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[]).await;

        // The set the fan-out draws from is the routing table and nothing else,
        // so anything absent from it cannot be named — not an unpaired device, not
        // a revoked one, not an id a peer made up.
        assert_eq!(
            ctx.route_keys.routable_device_ids(),
            vec![ctx.device_id.as_str().to_string()],
            "the fixture registers only this desktop; a device that is not routable \
             is not a fan-out target"
        );

        broadcast_to_others(
            &ctx,
            "ws_desktop",
            r#"{"type":"sms","action":"new","body":"x"}"#,
        )
        .await;

        th::assert_relay_silent(&mut relay_rx);
    }

    /// An oversized frame is refused by the relay's read ceiling — and crossing
    /// that ceiling **closes the connection** rather than dropping one frame, so a
    /// single padded `sms` from one paired peer would otherwise take every
    /// relay-only peer offline for a whole backoff interval.
    ///
    /// `validate_message` closes no field set for `sms`, so the frame below is
    /// accepted by the dispatcher; the guard has to be here rather than upstream
    /// or this is an availability bug reachable from a paired peer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_oversized_broadcast_is_refused_before_it_can_kill_the_relay_leg() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", RELAY_PHONE_SECRET)]).await;

        let oversized = serde_json::json!({
            "type": "sms", "action": "new", "body": "x",
            "pad": "a".repeat(WsServer::MAX_RELAY_BROADCAST_BYTES + 1),
        });
        assert!(
            oversized.to_string().len() > WsServer::MAX_RELAY_BROADCAST_BYTES,
            "the fixture frame must exceed the guard, or this asserts nothing"
        );

        assert_eq!(
            WsServer::fan_out_to_relay(&ctx, None, &oversized.to_string()).await,
            0,
            "an oversized frame must not be routed"
        );
        th::assert_relay_silent(&mut relay_rx);

        // And the guard must not be a blanket ban: a normal frame still goes.
        broadcast_to_others(
            &ctx,
            "ws_desktop",
            r#"{"type":"sms","action":"new","body":"hi"}"#,
        )
        .await;
        assert_eq!(
            th::only_relay_frame(&mut relay_rx)["to_device_id"],
            "dev_phone"
        );
    }

    /// REGRESSION: `WsServer::broadcast` is the *other* half of the fan-out
    /// defect, and it is the path every desktop-originated frame takes —
    /// `commands::notifications::forward_to_peers_checked` reaches it for a
    /// dismissal, a reply and a clipboard sync. A fix that taught only
    /// `broadcast_to_others` about the relay would leave the desktop's own
    /// notifications and clipboard on the LAN and nowhere else.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_desktops_own_broadcast_reaches_a_relay_only_peer() {
        let ctx = create_test_ctx();
        let mut relay_rx = th::add_test_relay(&ctx, &[("dev_phone", RELAY_PHONE_SECRET)]).await;
        let server = server_over(ctx.clone());
        let message = r#"{"type":"notification","action":"post","id":"n1"}"#;

        server.broadcast(message.to_string()).await;

        let route = th::only_relay_frame(&mut relay_rx);
        assert_eq!(route["to_device_id"], "dev_phone");
        assert_eq!(
            route["payload"]["type"], "encrypted",
            "a relay-only recipient has no socket, so the LAN writer's sealing \
             never runs for it"
        );
        assert_eq!(
            open_envelope(&ctx, RELAY_PHONE_SECRET, &route["payload"]),
            message
        );
    }

    /// REGRESSION: a relayed binary frame is attributed to the device the relay
    /// authenticated.
    ///
    /// Two separate defects sat on this path and both had to go for a chunk to
    /// arrive. `unwrap_relay_binary_frame` enumerated candidate senders from
    /// `SyncEngine` — a *liveness* map, so a phone that paired on the LAN and
    /// then moved networks was not in it and the frame was refused one layer
    /// early. Then `handle_lan_chunk` received the device id the unwrapper had
    /// correctly resolved and looked it up in `ws_to_device_id`, which is keyed
    /// the *other* way round, so `unwrap_or_default()` produced `""` and the
    /// chunk was dropped **after** the relay had verified its tag.
    ///
    /// Asserted on the attribution and on the secret resolution rather than on
    /// the file engine's byte count: the engine assertion needs its own
    /// temp-directory fixture, which would test the engine rather than the
    /// identity resolution that was the defect.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_only_senders_binary_frame_is_attributed_to_it() {
        use conduit_protocol::{CHUNK_HEADER_LEN, CHUNK_NONCE_LEN};
        let ctx = create_test_ctx();
        let secret = RELAY_PHONE_SECRET;
        th::add_test_relay(&ctx, &[("dev_phone", secret)]).await;

        let plaintext = b"chunk-bytes-from-a-phone".to_vec();
        let (nonce, ciphertext) = ctx
            .encryption
            .encrypt_binary(secret, &plaintext)
            .expect("chunk encryption");
        let metadata =
            serde_json::to_vec(&serde_json::json!({"id": "t_relay", "index": 0, "total": 2}))
                .expect("metadata serialises");
        let mut envelope = Vec::with_capacity(CHUNK_HEADER_LEN + metadata.len() + ciphertext.len());
        envelope.extend_from_slice(&nonce[..CHUNK_NONCE_LEN]);
        envelope.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
        envelope.extend_from_slice(&metadata);
        envelope.extend_from_slice(&ciphertext);

        let frame = conduit_protocol::build_binary_frame(
            &conduit_protocol::hmac::derive_route_key(&hex::decode(secret).unwrap(), "dev_phone"),
            "dev_phone",
            ctx.device_id.as_str(),
            1,
            &envelope,
        );

        let (attributed, payload) =
            handlers::files::unwrap_relay_binary_frame_for_tests(&frame, &ctx)
                .await
                .unwrap_or_else(|e| {
                    panic!("a tagged frame from a paired phone must be attributed: {e}")
                });
        assert_eq!(
            attributed, "dev_phone",
            "the sender is the device whose route key verified the tag"
        );
        assert_eq!(
            payload, envelope,
            "the payload is the chunk envelope, verbatim"
        );

        // And the step that was broken inside `handle_lan_chunk`: the device id
        // the unwrapper just resolved must itself resolve to a secret, where
        // before it was looked up in a connection-keyed map and came back `""`.
        assert_eq!(
            handlers::peer_secret(&ctx, &attributed).await.as_deref(),
            Some(secret),
            "a relay-only sender's device id must resolve to its pairing secret, \
             or the chunk is dropped after authentication"
        );
        assert!(
            handlers::peer_secret(&ctx, "ws_not_a_device")
                .await
                .is_none(),
            "and a connection id must not resolve to one"
        );
    }
}
