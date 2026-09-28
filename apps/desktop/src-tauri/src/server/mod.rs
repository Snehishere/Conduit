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

use handlers::{Clients, WsContext, WsToDeviceId, broadcast_to_others};

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

    /// Encrypt one outbound frame for `client_id`, if that peer has a secret.
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
    /// (`local_desktop`), which has no key pair — receives plaintext by
    /// necessity. That is sound because reaching that state requires the
    /// per-launch capability, and because broadcasts are filtered on pairing, so
    /// a stranger never gets here.
    async fn seal_for_peer(ctx: &WsContext, client_id: &str, msg_text: &str) -> String {
        if message_type_is(msg_text, "pairing") || message_type_is(msg_text, "encrypted") {
            return msg_text.to_string();
        }
        let stable_id = match handlers::paired_device_id(ctx, client_id).await {
            Some(id) => id,
            // Unpaired: should be unreachable for anything but a reply to this
            // peer's own request, but never leak a fan-out if it happens.
            None => return msg_text.to_string(),
        };
        let Some(client) = ctx.sync_engine.read().await.get_client(&stable_id).cloned() else {
            return msg_text.to_string();
        };
        match ctx.encryption.encrypt(&client.shared_secret, msg_text) {
            Ok((nonce, ciphertext)) => {
                let data_hex = hex::encode(ciphertext);
                let hmac_hex = ctx
                    .encryption
                    .generate_hmac(&client.shared_secret, &data_hex)
                    .unwrap_or_default();
                let envelope = EncryptedEnvelope {
                    msg_type: "encrypted".into(),
                    nonce: hex::encode(nonce),
                    hmac: hmac_hex,
                    data: data_hex,
                    source_device: Some(stable_id.clone()),
                    protocol_version: Some(PROTOCOL_VERSION),
                };
                serde_json::to_string(&envelope).expect("EncryptedEnvelope serializes")
            }
            Err(e) => {
                // Fail closed: never fall back to plaintext for a peer we do
                // have a secret for.
                error!(
                    "Failed to encrypt outbound message for {}: {}",
                    client_id, e
                );
                drop_message(msg_text)
            }
        }
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
                    if let Message::Text(text) = msg {
                        // Per-connection byte budget. The global limiter counts
                        // messages; without this a peer could sit under the
                        // message cap while pushing up to `MAX_MESSAGE_SIZE` per
                        // message. Charged before parsing so an unparsable
                        // flood still costs the attacker.
                        let text = text.as_str();
                        if !ctx
                            .rate_limiter
                            .check(&client_id, "", text.len() as u64)
                            .await
                        {
                            warn!(
                                "Byte budget exceeded by {} ({} bytes in window) — dropping frame",
                                client_id,
                                ctx.rate_limiter.window_bytes(&client_id).await
                            );
                            continue;
                        }
                        let preview: String = text.chars().take(120).collect();
                        info!("Received from {}: {}...", client_id, preview);
                        Self::handle_message(text, &client_id, &ctx).await;
                    } else if let Message::Binary(bytes) = msg {
                        let len = bytes.len();
                        if !ctx.rate_limiter.check(&client_id, "", len as u64).await {
                            warn!(
                                "Byte budget exceeded by {} — dropping {len}-byte binary frame",
                                client_id
                            );
                            continue;
                        }
                        info!("Received binary frame from {}: {} bytes", client_id, len);
                        handlers::files::handle_binary_message(bytes.to_vec(), &client_id, &ctx)
                            .await;
                    } else if let Message::Pong(_) = msg {
                        info!("Received pong from {}", client_id);
                    }
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

    async fn handle_message(text: &str, client_id: &str, ctx: &WsContext) {
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
            let clients_lock = ctx.clients.read().await;
            if let Some(tx) = clients_lock.get(client_id) {
                let _ = tx.send(err_resp);
            }
            return;
        }

        let raw_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if !ctx.rate_limiter.check(client_id, raw_type, 0).await {
            warn!("Rate limited: {} (type: {})", client_id, raw_type);
            return;
        }

        let processed_msg = if raw_type == "encrypted" {
            let mut stable_id = msg
                .get("source_device")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if stable_id.is_none() {
                stable_id = ctx.ws_to_device_id.read().await.get(client_id).cloned();
            }
            let stable_id = stable_id.unwrap_or_default();

            let shared_secret =
                if let Some(client) = ctx.sync_engine.read().await.get_client(&stable_id) {
                    client.shared_secret.clone()
                } else {
                    warn!(
                        "Received encrypted message but no shared secret found for {}",
                        client_id
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
        if !ctx
            .per_type_limiter
            .check_type_limit(client_id, msg_type)
            .await
        {
            warn!("Per-type rate limited: {} (type: {})", client_id, msg_type);
            let err_resp = ErrorMessage {
                msg_type: "error".into(),
                code: "rate_limited".into(),
                message: format!("Rate limit exceeded for message type '{}'", msg_type),
                server_version: None,
            };
            let err_resp = serde_json::to_string(&err_resp).expect("ErrorMessage serializes");
            let clients_lock = ctx.clients.read().await;
            if let Some(tx) = clients_lock.get(client_id) {
                let _ = tx.send(err_resp);
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
                let clients_lock = ctx.clients.read().await;
                if let Some(tx) = clients_lock.get(client_id) {
                    let _ = tx.send(pong);
                }
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

    /// Fan a message out to every **paired** connection.
    ///
    /// Unpaired connections are skipped. Every accepted socket used to be in
    /// `ctx.clients` unconditionally, and because the per-socket encryption
    /// only fires when `ws_to_device_id` yields a peer with a shared secret, an
    /// unpaired socket was handed the **raw plaintext** of every broadcast.
    pub async fn broadcast(&self, message: String) {
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
        let relay_lock = self.ctx.relay_tx.read().await;
        if let Some(relay_tx) = &*relay_lock {
            let mut relay_val = serde_json::json!({
                "type": "relay_route",
                "to_device_id": device_id,
                "payload": serde_json::from_str::<serde_json::Value>(&message).unwrap_or(serde_json::Value::Null),
                "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
                "nonce": uuid::Uuid::new_v4().to_string(),
            });

            let mut message_for_hmac = serde_json::Map::new();
            if let Some(t) = relay_val.get("type") {
                message_for_hmac.insert("type".to_string(), t.clone());
            }
            if let Some(to) = relay_val.get("to_device_id") {
                message_for_hmac.insert("to_device_id".to_string(), to.clone());
            }
            if let Some(payload) = relay_val.get("payload") {
                message_for_hmac.insert("payload".to_string(), payload.clone());
            }
            if let Some(ts) = relay_val.get("timestamp") {
                message_for_hmac.insert("timestamp".to_string(), ts.clone());
            }
            if let Some(n) = relay_val.get("nonce") {
                message_for_hmac.insert("nonce".to_string(), n.clone());
            }

            let message_str = serde_json::to_string(&serde_json::Value::Object(message_for_hmac))
                .unwrap_or_default();
            let relay_token = std::env::var("RELAY_TOKEN").unwrap_or_default();
            let hmac_hex =
                conduit_protocol::hmac::compute_hmac(relay_token.as_bytes(), &message_str);

            relay_val
                .as_object_mut()
                .unwrap()
                .insert("hmac".to_string(), serde_json::Value::String(hmac_hex));

            let relay_msg_str = serde_json::to_string(&relay_val).unwrap_or_default();
            if let Err(e) = relay_tx.send(relay_msg_str).await {
                warn!("Failed to route message through relay: {}", e);
                return false;
            }
            return true;
        }
        false
    }

    pub fn spawn_relay_client(&self, relay_url: String, server_id: String) {
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

                        let relay_token = std::env::var("RELAY_TOKEN").unwrap_or_default();
                        let auth_msg = RelayAuth::new(server_id.as_str(), relay_token);
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
                                    Self::handle_message(&text, "relay_server", &ctx).await;
                                }
                                Message::Binary(bytes) => {
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

/// Tell the sender *why* its message was dropped, and say so in the log.
///
/// A silent drop leaves the peer waiting for an ack that never arrives; an
/// `error` frame with a stable `code` lets both sides record a meaningful
/// reason. Unicast, so it also reaches a peer that is not paired yet.
async fn send_error(ctx: &WsContext, client_id: &str, code: &str, detail: &str) {
    let err_resp = ErrorMessage {
        msg_type: "error".into(),
        code: code.to_string(),
        message: detail.to_string(),
        server_version: Some(PROTOCOL_VERSION),
    };
    let err_resp = serde_json::to_string(&err_resp).expect("ErrorMessage serializes");
    let clients_lock = ctx.clients.read().await;
    if let Some(tx) = clients_lock.get(client_id) {
        let _ = tx.send(err_resp);
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
}
