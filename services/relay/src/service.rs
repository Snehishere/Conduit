//! Starting, running and stopping the relay as a *service*.
//!
//! # Why this exists
//!
//! The relay used to be a program: it read the environment, bound its sockets,
//! ran until a signal arrived, and called [`std::process::exit`] when something
//! went wrong. That shape only works when the relay owns the process, and it no
//! longer does — the desktop app hosts the relay as a background task, and
//! killing the app must not be how a misconfiguration is reported.
//!
//! [`RelayService`] is the whole lifecycle in three steps, and every failure is
//! a value rather than an exit:
//!
//! ```no_run
//! # use conduit_relay::{RelayService, RouteKeys, StaticRouteKeys};
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! // The host owns the device registry, so it supplies the route keys. An
//! // empty resolver simply means no device can route yet.
//! let route_keys: std::sync::Arc<dyn RouteKeys> = std::sync::Arc::new(StaticRouteKeys::new());
//!
//! let handle = RelayService::builder()
//!     .wss_port(9529)
//!     .start(route_keys)
//!     .await?;
//!
//! // ... the relay is now accepting connections in the background ...
//!
//! let summary = handle.shutdown().await;
//! println!("{} connection(s) were live at shutdown", summary.active_at_exit);
//! # Ok(())
//! # }
//! ```
//!
//! # Who owns what
//!
//! The library installs **no signal handlers**. A host that is a process (a
//! test harness, a future headless tool) wires `ctrl_c`/SIGTERM to
//! [`ServiceHandle::request_shutdown`] itself. A host that is a GUI application
//! already has a lifecycle of its own and must not have one overwritten.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use conduit_protocol::hmac::NonceCache;
use log::{error, info, warn};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::config::Config;
use crate::connection::{drain_connections, handle_connection};
use crate::health::spawn_health_server;
use crate::limits::{MAX_CONNECTIONS, RateLimiter};
use crate::metrics::Metrics;
use crate::state::{AppState, ConnectionGuard};

/// Compare two SPKI pins, tolerating the formatting a human would type.
///
/// Both sides are `sha256/<base64>`. The prefix, surrounding whitespace and
/// base64 padding are all ignored, because an operator pasting a pin out of
/// `/pin` should not have to reproduce the exact padding. The digest itself is
/// compared as bytes, so a wrong key is a wrong key.
pub(crate) fn pins_match(expected: &str, actual: &str) -> bool {
    fn normalise(pin: &str) -> Option<Vec<u8>> {
        let body = pin.trim().strip_prefix("sha256/").unwrap_or(pin.trim());
        let body = body.replace([' ', '\n', '\t'], "");
        let body = body.trim_end_matches('=');
        if body.is_empty() {
            return None;
        }
        base64_decode(body).filter(|bytes| bytes.len() == 32)
    }

    match (normalise(expected), normalise(actual)) {
        (Some(a), Some(b)) => a == b,
        // An unparseable pin on either side is never a match. Silently treating
        // a typo as "no pin configured" would disable the check the operator
        // believes they have.
        _ => false,
    }
}

/// Standard base64 decode. Returns `None` on any invalid character.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in input.as_bytes() {
        acc = (acc << 6) | value(byte)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // Leftover bits must be zero, or the input was not a clean encoding.
    if bits > 0 && (acc & ((1 << bits) - 1)) != 0 {
        return None;
    }
    Some(out)
}

/// How often the replay cache is written to disk, and the routing table and
/// connection rate limiter are swept.
///
/// The nonce flush interval is deliberately shorter than the freshness window a
/// [`conduit_protocol::hmac::NonceCache`] enforces, so a crash cannot lose a
/// nonce that is still inside the window in which it would be accepted twice.
const HOUSEKEEPING_INTERVAL_SECS: u64 = 30;

/// Something that stopped the relay from starting.
///
/// A misconfiguration is *reported*, not exited on. The host decides whether
/// that is fatal: the desktop logs it and carries on LAN-only, because a phone
/// on the same network does not need the relay at all.
#[derive(Debug)]
pub enum StartError {
    /// The configuration is not usable — a missing token, an unreadable secret.
    Config(String),
    /// A listener could not bind. Carries the port and the OS error.
    Bind {
        /// The port that could not be bound.
        port: u16,
        /// The underlying OS error.
        source: std::io::Error,
    },
    /// No listener started: TLS was unavailable *and* plain WS was disabled.
    /// Starting anyway would produce a service that accepts nothing.
    NoListeners,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Config(msg) => write!(f, "relay configuration is invalid: {msg}"),
            StartError::Bind { port, source } => {
                write!(f, "failed to bind relay port {port}: {source}")
            }
            StartError::NoListeners => write!(
                f,
                "no relay listeners could start: TLS was unavailable and plain WS is disabled, \
                 so the relay would accept nothing"
            ),
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StartError::Bind { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// What a finished relay was doing when it was asked to stop.
///
/// Returned by [`ServiceHandle::shutdown`] so a host can log something truthful
/// about what it tore down, rather than reusing counts it read earlier.
/// Derives Serialize because this is a summary a host displays or logs; the
/// point of returning it is that a caller can say something truthful about what
/// it tore down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Shutdown {
    /// Connections open at the moment shutdown began.
    pub active_at_exit: usize,
    /// Peers still in the routing table once the listeners stopped.
    pub registered_devices: usize,
    /// Routes forwarded over the relay's lifetime.
    pub messages_routed: u64,
    /// Routes dropped, summed across every drop reason.
    pub messages_dropped: u64,
}

/// A point-in-time view of a running relay, for a status display.
/// A point-in-time view, for a host's status display.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Sockets currently accepted.
    pub active_connections: usize,
    /// Peers in the routing table.
    pub registered_devices: usize,
    /// Nonces currently held by the replay cache.
    pub cached_nonces: usize,
    /// `sha256/<base64>` SPKI pin of the served certificate, if TLS is up.
    pub tls_pin: Option<String>,
    /// The port the TLS listener bound, if TLS is up.
    pub wss_port: Option<u16>,
    /// The port the plaintext listener bound, if it is up.
    pub ws_port: Option<u16>,
}

/// Builder for a [`RelayService`].
///
/// Seeded from [`Config::resolve`] — the environment and the built-in defaults —
/// with each field overridable. A host that has its own settings store should
/// use [`RelayService::from_config`] so it does not restate every field.
pub struct RelayServiceBuilder {
    /// `Err` holds a deferred configuration error. It is carried rather than
    /// raised so an override still has somewhere to go, and so the caller gets
    /// one error from `start()` describing the real problem.
    config: Result<Config, String>,
}

impl RelayServiceBuilder {
    /// Seeded from the environment and the built-in defaults.
    pub fn new() -> Self {
        Self {
            config: Config::resolve(None),
        }
    }

    /// Port for the TLS WebSocket listener.
    pub fn wss_port(mut self, port: u16) -> Self {
        self.mutate(|c| c.wss_port = port);
        self
    }

    /// Port for the plaintext WebSocket listener. Off unless enabled.
    pub fn ws_port(mut self, port: u16) -> Self {
        self.mutate(|c| c.ws_port = port);
        self
    }

    /// Port for `/healthz`, `/health`, `/metrics` and `/pin`.
    pub fn health_port(mut self, port: u16) -> Self {
        self.mutate(|c| c.health_port = port);
        self
    }

    /// Bearer token every client presents as `relay_token` during `relay_auth`.
    pub fn relay_token(mut self, token: impl Into<String>) -> Self {
        self.mutate(|c| c.relay_token = token.into());
        self
    }

    /// Where the replay cache is persisted. Defaults to a file beside the app's
    /// data directory.
    pub fn nonce_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.mutate(|c| c.nonce_file = path.into());
        self
    }

    /// Enable the plaintext listener. It carries `relay_token` in the clear.
    pub fn enable_plain_ws(mut self, enable: bool) -> Self {
        self.mutate(|c| c.enable_plain_ws = enable);
        self
    }

    /// How long a socket may stay open without authenticating.
    pub fn auth_timeout(mut self, secs: u64) -> Self {
        self.mutate(|c| c.auth_timeout_secs = secs);
        self
    }

    /// Drop every override and re-read the environment.
    pub fn reload_from_env(mut self) -> Self {
        self.config = Config::resolve(None);
        self
    }

    /// Build the service, surfacing a deferred configuration error.
    pub fn build(self) -> Result<RelayService, StartError> {
        self.config
            .map(RelayService::from_config)
            .map_err(StartError::Config)
    }

    /// Build and start in one step. The common path.
    /// Start the relay, resolving route keys through `route_keys`.
    ///
    /// The host owns the device registry, so it owns the keys: this is what lets
    /// a re-pair rotate a device's signing key without reconfiguring the relay.
    pub async fn start(
        self,
        route_keys: Arc<dyn crate::state::RouteKeys>,
    ) -> Result<ServiceHandle, StartError> {
        self.build()?.start(route_keys).await
    }

    /// Apply `f` to the resolved config, if it resolved at all.
    fn mutate(&mut self, f: impl FnOnce(&mut Config)) {
        if let Ok(config) = self.config.as_mut() {
            f(config);
        }
    }
}

impl Default for RelayServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A relay that is ready to be started.
pub struct RelayService {
    config: Config,
}

impl RelayService {
    /// A builder seeded from the environment and the built-in defaults.
    pub fn builder() -> RelayServiceBuilder {
        RelayServiceBuilder::new()
    }

    /// A relay configured explicitly. This is the path the desktop takes: it has
    /// settings in an encrypted database, and the environment is consulted only
    /// for values the user has not chosen.
    pub fn from_config(config: Config) -> Self {
        Self { config }
    }

    /// Bind every listener and begin accepting connections.
    ///
    /// Every socket is bound *before* this returns, so a port conflict is
    /// reported here rather than surfacing later as a task that silently died.
    /// The returned [`ServiceHandle`] owns every running task.
    /// Start the relay, resolving route keys through `route_keys`.
    ///
    /// The host owns the device registry, so it owns the keys: this is what lets
    /// a re-pair rotate a device's signing key without reconfiguring the relay.
    pub async fn start(
        self,
        route_keys: Arc<dyn crate::state::RouteKeys>,
    ) -> Result<ServiceHandle, StartError> {
        let config = self.config;

        // --- TLS -------------------------------------------------------------
        // An operator-configured pin is a hard requirement, checked before the
        // context is built: it is the only statement the operator has made about
        // which key this relay is allowed to serve, and serving a different one
        // is precisely the failure they were trying to prevent.
        if let Some(expected) = config.relay_cert_pin.as_deref().map(str::trim)
            && !expected.is_empty()
        {
            let actual = match crate::tls::load_tls_context() {
                Ok(ctx) => ctx.spki_pin.clone(),
                Err(e) => {
                    return Err(StartError::Config(format!(
                        "a certificate pin is configured but the TLS context could not be built: {e}"
                    )));
                }
            };
            if !pins_match(expected, &actual) {
                return Err(StartError::Config(format!(
                    "the relay certificate does not match the configured pin \
                     (expected {expected}, serving {actual}). If the certificate was \
                     renewed, update the pin; if it was not, do not."
                )));
            }
            info!("Relay certificate matches the configured pin {expected}");
        }

        // Non-fatal: the relay is still useful without it, and a certificate
        // that cannot be created or read is a warning, not a refusal to start.
        let tls_context = match crate::tls::load_tls_context() {
            Ok(ctx) => {
                crate::tls::warn_if_sans_may_not_match(&ctx);
                info!(
                    "Relay TLS ready (TLS 1.3, {:?} certificate, SANs {:?}) — pin={}",
                    ctx.source, ctx.subject_alt_names, ctx.spki_pin
                );
                Some(ctx)
            }
            Err(e) => {
                warn!("Relay TLS initialisation failed: {e}. WSS will be unavailable.");
                None
            }
        };
        let tls_acceptor = tls_context.as_ref().map(|c| c.acceptor.clone());
        let tls_pin = tls_context.as_ref().map(|c| c.spki_pin.clone());
        let wss_port = tls_context.as_ref().map(|_| config.wss_port);

        // --- Shared state ----------------------------------------------------
        let loaded_nonces = crate::hmac::load_nonces(&config.nonce_file);
        info!(
            "Relay replay cache: loaded {} nonce(s) from {}",
            loaded_nonces.len(),
            config.nonce_file.display()
        );
        info!(
            "Relay route keys: {} device(s) can sign — {:?}",
            route_keys.device_ids().len(),
            route_keys.device_ids()
        );

        let state = Arc::new(AppState {
            clients: Arc::new(RwLock::new(HashMap::new())),
            active_connections: Arc::new(AtomicUsize::new(0)),
            metrics: Arc::new(Metrics::new()),
            rate_limiter: Arc::new(RateLimiter::new(10, 60)),
            nonces: Arc::new(RwLock::new(NonceCache::from_map(&loaded_nonces))),
            tls_pin,
            config,
            route_keys,
        });

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut listeners: Vec<JoinHandle<()>> = Vec::new();

        // --- HTTP: health, metrics, pin --------------------------------------
        let health = spawn_health_server(state.clone(), shutdown_rx.clone())
            .await
            .map_err(|source| StartError::Bind {
                port: state.config.health_port,
                source,
            })?;
        listeners.push(health);

        // --- TLS WebSocket ---------------------------------------------------
        if let Some(acceptor) = tls_acceptor {
            // The TLS listener is the one that has to be reachable from the
            // network: a phone on another network dials this port.
            let listener =
                bind(std::net::IpAddr::from([0, 0, 0, 0]), state.config.wss_port).await?;
            info!("Relay WSS listening on port {}", state.config.wss_port);
            listeners.push(spawn_wss_listener(
                listener,
                state.clone(),
                shutdown_rx.clone(),
                acceptor,
            ));
        } else {
            warn!("Relay has no TLS context; the WSS listener was not started.");
        }

        // --- Plaintext WebSocket, opt-in -------------------------------------
        if state.config.enable_plain_ws {
            let listener = bind(state.config.ws_bind, state.config.ws_port).await?;
            if state.config.ws_bind.is_loopback() {
                // Safe: the token in the clear never leaves the machine, and
                // this is how a host that *is* the relay joins its own routing
                // table without having to trust its own certificate.
                info!(
                    "Relay plain WS listening on 127.0.0.1:{} (loopback only)",
                    state.config.ws_port
                );
            } else {
                warn!(
                    "Relay plain WS listening on {}:{} — INSECURE, relay_token is sent in \
                     cleartext to anything that can reach this port",
                    state.config.ws_bind, state.config.ws_port
                );
            }
            listeners.push(spawn_plain_ws_listener(
                listener,
                state.clone(),
                shutdown_rx.clone(),
            ));
        }

        // A relay that bound nothing would sit there accepting no connections
        // while reporting itself healthy. Refuse rather than pretend.
        if listeners.len() == 1 {
            return Err(StartError::NoListeners);
        }

        let housekeeping = spawn_housekeeping(state.clone());

        info!(
            "Relay running in-process — WSS: {}, plain WS: {}, health: {}",
            wss_port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "off".into()),
            if state.config.enable_plain_ws {
                state.config.ws_port.to_string()
            } else {
                "off".into()
            },
            state.config.health_port,
        );

        Ok(ServiceHandle {
            state,
            shutdown_tx,
            listeners,
            housekeeping,
        })
    }
}

/// Controls a running relay.
///
/// Dropping this does **not** stop the relay — the tasks are already spawned and
/// an abandoned `ServiceHandle` should not silently take the service down. Call
/// [`ServiceHandle::shutdown`] (or [`ServiceHandle::request_shutdown`], then
/// [`ServiceHandle::join`]) to stop it.
pub struct ServiceHandle {
    state: Arc<AppState>,
    shutdown_tx: watch::Sender<bool>,
    listeners: Vec<JoinHandle<()>>,
    housekeeping: JoinHandle<()>,
}

impl ServiceHandle {
    /// Ask every listener to stop, without waiting for them.
    ///
    /// Safe to call more than once. The desktop uses this on window close, and
    /// then awaits [`ServiceHandle::join`] on a background task so the UI is not
    /// held up by the drain window.
    pub fn request_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// The resolved configuration, for a status display or a log line.
    pub fn config(&self) -> &Config {
        &self.state.config
    }

    /// A current view of the relay.
    pub async fn status(&self) -> Status {
        Status {
            active_connections: self.state.active_connections.load(Ordering::Relaxed),
            registered_devices: self.state.clients.read().await.len(),
            cached_nonces: self.state.nonces.read().await.to_map().len(),
            tls_pin: self.state.tls_pin.clone(),
            wss_port: self
                .state
                .tls_pin
                .is_some()
                .then_some(self.state.config.wss_port),
            ws_port: self
                .state
                .config
                .enable_plain_ws
                .then_some(self.state.config.ws_port),
        }
    }

    /// Stop the relay and wait for the listeners to finish draining.
    ///
    /// Returns what the relay was doing at the moment it was asked to stop, and
    /// performs a final replay-cache flush so a nonce accepted moments before
    /// shutdown cannot be replayed after a restart.
    pub async fn shutdown(self) -> Shutdown {
        let ServiceHandle {
            state,
            shutdown_tx,
            listeners,
            housekeeping,
        } = self;

        let active_at_exit = state.active_connections.load(Ordering::Relaxed);
        info!("Relay shutting down ({active_at_exit} connection(s) live)");

        let _ = shutdown_tx.send(true);
        housekeeping.abort();
        for listener in listeners {
            if let Err(e) = listener.await {
                // A listener that panicked is a bug worth surfacing, not a
                // reason to skip the rest of the teardown.
                error!("Relay listener task failed: {e}");
            }
        }

        let registered_devices = state.clients.read().await.len();
        let m = &state.metrics;
        use std::sync::atomic::Ordering as O;
        let messages_routed = m.messages_routed.load(O::Relaxed);
        let messages_dropped = m.messages_dropped_not_found.load(O::Relaxed)
            + m.messages_dropped_timeout.load(O::Relaxed)
            + m.messages_dropped_rate_limit.load(O::Relaxed)
            + m.messages_dropped_replay.load(O::Relaxed)
            + m.messages_dropped_hmac_failed.load(O::Relaxed)
            + m.messages_dropped_unknown_type.load(O::Relaxed);

        // `save_nonces` is synchronous I/O. Running it here is safe: every
        // listener has already stopped, so there is no worker thread to starve.
        let snapshot = state.nonces.read().await.to_map();
        let file = state.config.nonce_file.clone();
        match crate::hmac::save_nonces(&file, &snapshot) {
            Ok(()) => info!("Relay replay cache persisted to {}", file.display()),
            Err(e) => warn!("Failed to persist the replay cache on shutdown: {e}"),
        }

        Shutdown {
            active_at_exit,
            registered_devices,
            messages_routed,
            messages_dropped,
        }
    }

    /// Wait for every listener to finish. Call after [`Self::request_shutdown`].
    pub async fn join(self) {
        self.housekeeping.abort();
        for listener in self.listeners {
            if let Err(e) = listener.await {
                // A listener that panicked is a bug worth surfacing, not a
                // reason to skip the rest of the teardown.
                error!("Relay listener task failed: {e}");
            }
        }
    }
}

/// A connection stream that is either plaintext or TLS-wrapped.
///
/// The accept loop is written once rather than twice. Without this it would be
/// duplicated for WSS and plain WS, and the two copies would drift in exactly
/// the ways that matter: the connection cap, the shutdown branch, the drain.
pub enum Stream {
    /// Accepted over plain TCP.
    Plain(TcpStream),
    /// Accepted over TLS.
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Bind a listener on `addr`.
async fn bind(addr: std::net::IpAddr, port: u16) -> Result<TcpListener, StartError> {
    TcpListener::bind(SocketAddr::new(addr, port))
        .await
        .map_err(|source| StartError::Bind { port, source })
}

/// WSS accept loop.
fn spawn_wss_listener(
    listener: TcpListener,
    state: Arc<AppState>,
    mut shutdown_rx: watch::Receiver<bool>,
    acceptor: tokio_rustls::TlsAcceptor,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        accept_loop(
            listener,
            state,
            &mut shutdown_rx,
            "WSS",
            move |stream, state, peer, sd| {
                let acceptor = acceptor.clone();
                let addr = peer;
                async move {
                    let Ok(tls) = acceptor.accept(stream).await else {
                        error!("TLS handshake error from {addr}");
                        return;
                    };
                    if !admit(&state, &addr) {
                        return;
                    }
                    handle_connection(Stream::Tls(Box::new(tls)), state, addr, sd).await;
                }
            },
        )
        .await;
    })
}

/// Plaintext WS accept loop.
fn spawn_plain_ws_listener(
    listener: TcpListener,
    state: Arc<AppState>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        accept_loop(
            listener,
            state,
            &mut shutdown_rx,
            "WS",
            |stream, state, peer, sd| async move {
                if !admit(&state, &peer) {
                    return;
                }
                handle_connection(Stream::Plain(stream), state, peer, sd).await;
            },
        )
        .await;
    })
}

/// Apply the connection cap and account for one live connection.
///
/// The guard must be created *before* the counter is incremented, so the reject
/// path decrements a slot it never took. The TLS handshake happens before this
/// is reached, which is a known gap: a peer that opens a socket and never sends
/// a ClientHello holds a task until the handshake times out.
fn admit(state: &Arc<AppState>, peer: &SocketAddr) -> bool {
    let _guard = ConnectionGuard {
        count: state.active_connections.clone(),
    };
    if state.active_connections.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
        warn!("Connection limit reached, rejecting {peer}");
        return false;
    }
    state
        .metrics
        .connections_connected
        .fetch_add(1, Ordering::Relaxed);
    true
}

/// The accept loop, shared by both listeners.
///
/// `connections` is a [`JoinSet`] so shutdown can actually reap tasks instead of
/// leaking them and their `state.clients` entries.
async fn accept_loop<F, Fut>(
    listener: TcpListener,
    state: Arc<AppState>,
    shutdown_rx: &mut watch::Receiver<bool>,
    label: &'static str,
    handshake: F,
) where
    F: Fn(TcpStream, Arc<AppState>, SocketAddr, watch::Receiver<bool>) -> Fut
        + Send
        + Sync
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut connections: JoinSet<()> = JoinSet::new();
    let handshake = Arc::new(handshake);
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, peer)) => {
                        info!("New {label} connection from {peer}");
                        let state = state.clone();
                        let handshake = handshake.clone();
                        let conn_shutdown = shutdown_rx.clone();
                        connections.spawn(async move {
                            handshake(stream, state, peer, conn_shutdown).await;
                        });
                    }
                    Err(e) => error!("{label} accept error: {e}"),
                }
            }
            Some(joined) = connections.join_next(), if !connections.is_empty() => {
                if let Err(e) = joined {
                    error!("{label} connection task failed: {e}");
                }
            }
            _ = shutdown_rx.changed() => {
                info!("{label} listener shutting down");
                break;
            }
        }
    }
    drain_connections(&mut connections, label).await;
}

/// Periodic maintenance: persist the replay cache, and prune two structures that
/// are otherwise only pruned on an event involving the same key.
///
/// The connection rate limiter and the routing table both grow without bound
/// otherwise — the first when an attacker rotates source addresses, the second
/// when a task is aborted mid-connection and its `Sender` is never dropped.
fn spawn_housekeeping(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(HOUSEKEEPING_INTERVAL_SECS));
        let nonce_file = state.config.nonce_file.clone();
        loop {
            interval.tick().await;

            // `save_nonces` is synchronous `std::fs` I/O; on the blocking pool,
            // or it stalls a worker thread for the duration of every flush.
            let snapshot = state.nonces.read().await.to_map();
            let file = nonce_file.clone();
            match tokio::task::spawn_blocking({
                let file = file.clone();
                move || crate::hmac::save_nonces(&file, &snapshot)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!("Failed to persist the replay cache: {e}"),
                Err(e) => error!("Replay cache persistence task failed: {e}"),
            }

            let swept = state.rate_limiter.sweep();
            let removed = crate::state::reconcile_clients(&state.clients).await;
            if swept > 0 || removed > 0 {
                info!(
                    "Relay housekeeping: evicted {swept} expired rate-limit window(s), \
                     reconciled {removed} dead client entr{}",
                    if removed == 1 { "y" } else { "ies" }
                );
            }
        }
    })
}
