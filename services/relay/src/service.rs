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
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use conduit_protocol::hmac::NonceCache;
use log::{error, info, warn};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Sleep;

use crate::config::Config;
use crate::connection::{drain_connections, handle_connection};
use crate::health::spawn_health_server;
use crate::limits::{
    MAX_CONNECTIONS, RateLimiter, TLS_HANDSHAKE_TIMEOUT_SECS, WS_UPGRADE_TIMEOUT_SECS,
};
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
                Duration::from_secs(TLS_HANDSHAKE_TIMEOUT_SECS),
                Duration::from_secs(WS_UPGRADE_TIMEOUT_SECS),
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
                Duration::from_secs(WS_UPGRADE_TIMEOUT_SECS),
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

/// A stream whose WebSocket-upgrade phase carries a deadline.
///
/// The upgrade — HTTP request in, `101` out — happens inside
/// [`handle_connection`], and it is the phase a peer can stretch for free:
/// open a socket, send half a request, trickle the rest, hold the counted
/// connection slot indefinitely. The upgrade cannot be wrapped in
/// [`tokio::time::timeout`] from out here without also putting a timer around
/// the entire life of the connection, so the budget travels *inside* the
/// stream instead: constructed immediately before the upgrade, armed until
/// the server writes its handshake response, and inert from then on, so a
/// healthy connection is never raced against a timer that has stopped meaning
/// anything.
///
/// The budget is a real [`Sleep`], not a deadline checked on I/O: a peer that
/// sends nothing produces no readiness event ever, so a bare timestamp would
/// never be observed. The sleep's waker is registered on the first poll and
/// wakes the task when the budget runs out even if the socket stays silent.
pub(crate) struct UpgradeDeadline<S> {
    inner: S,
    budget: Pin<Box<Sleep>>,
    timeout: Duration,
    /// Cleared once the server has written its handshake response — from that
    /// point the budget must never fire again.
    armed: bool,
}

impl<S> UpgradeDeadline<S> {
    pub(crate) fn new(inner: S, timeout: Duration) -> Self {
        Self {
            inner,
            budget: Box::pin(tokio::time::sleep(timeout)),
            timeout,
            armed: true,
        }
    }

    /// Whether the budget has run out while the upgrade is still pending.
    fn budget_expired(&mut self, cx: &mut Context<'_>) -> bool {
        if !self.armed {
            return false;
        }
        self.budget.as_mut().poll(cx).is_ready()
    }

    /// The refusal a stalled upgrade gets: an I/O error, which is what the
    /// WebSocket handshake already turns into the same rejection a malformed
    /// request gets, and what `handle_connection` already knows how to log.
    fn expired_error(&self) -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("WebSocket upgrade exceeded its {:?} budget", self.timeout),
        )
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for UpgradeDeadline<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.budget_expired(cx) {
            return Poll::Ready(Err(this.expired_error()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for UpgradeDeadline<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.budget_expired(cx) {
            return Poll::Ready(Err(this.expired_error()));
        }
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(written)) => {
                // The server writes only once the request is in — the `101`
                // response, or a rejection of a request it will not complete.
                // Either way the upgrade phase is over; disarming here is
                // what keeps the budget from reaching the established
                // connection.
                this.armed = false;
                Poll::Ready(Ok(written))
            }
            pending => pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.budget_expired(cx) {
            return Poll::Ready(Err(this.expired_error()));
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // Never refused: shutting the stream down is how every path —
        // including a budget that has just expired — releases the socket.
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Bind a listener on `addr`.
async fn bind(addr: std::net::IpAddr, port: u16) -> Result<TcpListener, StartError> {
    TcpListener::bind(SocketAddr::new(addr, port))
        .await
        .map_err(|source| StartError::Bind { port, source })
}

/// WSS accept loop.
///
/// The two handshake budgets are parameters rather than constants so a test
/// can drive this loop — accept loop, admission, TLS, upgrade — with budgets
/// it can wait out.
fn spawn_wss_listener(
    listener: TcpListener,
    state: Arc<AppState>,
    mut shutdown_rx: watch::Receiver<bool>,
    acceptor: tokio_rustls::TlsAcceptor,
    tls_budget: Duration,
    upgrade_budget: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        accept_loop(
            listener,
            state,
            &mut shutdown_rx,
            "WSS",
            move |stream, state, peer, sd| {
                let acceptor = acceptor.clone();
                async move {
                    run_wss_connection(
                        acceptor,
                        stream,
                        state,
                        peer,
                        sd,
                        tls_budget,
                        upgrade_budget,
                    )
                    .await;
                }
            },
        )
        .await;
    })
}

/// Plaintext WS accept loop. Budgets are parameters for the same reason as
/// in [`spawn_wss_listener`].
fn spawn_plain_ws_listener(
    listener: TcpListener,
    state: Arc<AppState>,
    mut shutdown_rx: watch::Receiver<bool>,
    upgrade_budget: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        accept_loop(
            listener,
            state,
            &mut shutdown_rx,
            "WS",
            move |stream, state, peer, sd| async move {
                run_plain_ws_connection(stream, state, peer, sd, upgrade_budget).await;
            },
        )
        .await;
    })
}

/// One WSS connection: admit, TLS handshake under a budget, upgrade under a
/// budget, then serve.
///
/// Admission runs *first* — before the TLS handshake — so a peer that never
/// sends a `ClientHello` is counted against the connection cap and bounded by
/// the per-IP budget instead of holding an invisible task and fd forever.
/// The guard returned by [`admit`] is bound in this future, so every early
/// return below drops it and releases the slot; nothing else can outlive the
/// task holding it.
///
/// The two budgets are parameters rather than constants so a test can
/// exercise both give-up paths without waiting out the production values.
async fn run_wss_connection(
    acceptor: tokio_rustls::TlsAcceptor,
    stream: TcpStream,
    state: Arc<AppState>,
    peer: SocketAddr,
    sd: watch::Receiver<bool>,
    tls_budget: Duration,
    upgrade_budget: Duration,
) {
    let Some(_guard) = admit(&state, &peer) else {
        return;
    };
    let Some(tls) = accept_tls(&acceptor, stream, peer, tls_budget).await else {
        return;
    };
    handle_connection(
        UpgradeDeadline::new(Stream::Tls(Box::new(tls)), upgrade_budget),
        state,
        peer,
        sd,
    )
    .await;
}

/// One plaintext WS connection: admit, upgrade under a budget, then serve.
///
/// Same shape as [`run_wss_connection`] without the TLS phase. The rate limit
/// and the cap both run before the upgrade, so a peer over its budget is
/// refused before any handshake work happens.
async fn run_plain_ws_connection(
    stream: TcpStream,
    state: Arc<AppState>,
    peer: SocketAddr,
    sd: watch::Receiver<bool>,
    upgrade_budget: Duration,
) {
    let Some(_guard) = admit(&state, &peer) else {
        return;
    };
    handle_connection(
        UpgradeDeadline::new(Stream::Plain(stream), upgrade_budget),
        state,
        peer,
        sd,
    )
    .await;
}

/// Complete one TLS handshake, or give up when `timeout` elapses.
///
/// The budget is a parameter rather than the constant so a test can exercise
/// the give-up path without waiting out the production value. A timeout
/// yields the same verdict as a handshake that failed outright — `None`, the
/// task ends, and the connection slot [`admit`] took is released by the
/// guard's drop.
async fn accept_tls<S>(
    acceptor: &tokio_rustls::TlsAcceptor,
    stream: S,
    peer: SocketAddr,
    timeout: Duration,
) -> Option<tokio_rustls::server::TlsStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match tokio::time::timeout(timeout, acceptor.accept(stream)).await {
        Ok(Ok(tls)) => Some(tls),
        Ok(Err(e)) => {
            error!("TLS handshake error from {peer}: {e}");
            None
        }
        Err(_) => {
            // The peer held a task, an fd and a counted slot without ever
            // speaking TLS: silence gets the same verdict as a handshake
            // that actually failed.
            warn!("TLS handshake from {peer} timed out after {timeout:?}");
            None
        }
    }
}

/// Apply the per-IP rate limit and the connection cap, and account for one
/// live connection.
///
/// Returns the [`ConnectionGuard`] that holds the slot; the caller binds it
/// for as long as the connection task runs, so every early return — a rate
/// refusal, a full cap, a timed-out handshake, a rejected upgrade — drops it
/// and gives the slot back. Admission runs before both handshakes precisely
/// so those paths are the ones the guard covers.
///
/// The guard is created *before* the counter is incremented, so the reject
/// path decrements a slot it never took.
fn admit(state: &Arc<AppState>, peer: &SocketAddr) -> Option<ConnectionGuard> {
    // The rate limit comes first: it is the cheapest check, and refusing
    // here means an IP that spent its budget never touches the connection
    // counter or a handshake at all. The connection handler applies the same
    // check again after the upgrade, so a connection costs two window slots;
    // that post-upgrade check lives in connection.rs and is out of scope
    // here.
    if !state.rate_limiter.allow(peer.ip()) {
        warn!("Rate limit exceeded for {peer}");
        state
            .metrics
            .messages_dropped_rate_limit
            .fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let guard = ConnectionGuard {
        count: state.active_connections.clone(),
    };
    if state.active_connections.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
        warn!("Connection limit reached, rejecting {peer}");
        return None;
    }
    state
        .metrics
        .connections_connected
        .fetch_add(1, Ordering::Relaxed);
    Some(guard)
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

// ================================================================
//  W6.7 — handshakes are timed and counted
//
//  Admission runs before both handshakes, both handshakes carry a
//  budget, and the guard that counts a connection is held by the task
//  that owns it. Every test drives those seams with a budget it chose,
//  so nothing here waits out a production timeout.
// ================================================================

#[cfg(test)]
mod service_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use rustls::pki_types::ServerName;
    use tokio_tungstenite::connect_async;
    use tungstenite::Message;

    /// The bearer token test connections present during `relay_auth`.
    const TEST_TOKEN: &str = "test-secret-for-unit-tests!!!";

    /// A peer address for admission tests. The rate limiter keys on the IP,
    /// so the port only has to make the address well-formed.
    fn peer(port: u16) -> SocketAddr {
        format!("203.0.113.7:{port}").parse().expect("peer address")
    }

    /// Connections currently held, as admission maintains it.
    fn active(state: &AppState) -> usize {
        state.active_connections.load(Ordering::Relaxed)
    }

    /// An [`AppState`] with nothing listening: only the fields admission and
    /// the per-connection pipelines touch are meaningful here. `max_attempts`
    /// is the per-IP connection budget.
    fn test_state(max_attempts: usize) -> Arc<AppState> {
        Arc::new(AppState {
            clients: Arc::new(RwLock::new(HashMap::new())),
            active_connections: Arc::new(AtomicUsize::new(0)),
            metrics: Arc::new(Metrics::new()),
            rate_limiter: Arc::new(RateLimiter::new(max_attempts, 60)),
            nonces: Arc::new(RwLock::new(NonceCache::new())),
            tls_pin: None,
            route_keys: Arc::new(crate::state::StaticRouteKeys::new()),
            config: Config {
                ws_port: 0,
                ws_bind: "127.0.0.1".parse().expect("loopback"),
                wss_port: 0,
                health_port: 0,
                health_bind: "127.0.0.1".parse().expect("loopback"),
                hmac_secret: TEST_TOKEN.as_bytes().to_vec(),
                hmac_secret_file: std::env::temp_dir().join("relay-service-test-hmac"),
                health_token: TEST_TOKEN.into(),
                metrics_token: None,
                relay_token: TEST_TOKEN.into(),
                tls: crate::tls::TlsParams::default(),
                enable_plain_ws: true,
                nonce_file: std::env::temp_dir()
                    .join(format!("relay-service-tests-{}.json", std::process::id())),
                auth_timeout_secs: 10,
                relay_cert_pin: None,
            },
        })
    }

    /// Poll `check` until it holds or `budget` runs out.
    async fn wait_for(budget: Duration, mut check: impl FnMut() -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if check() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A self-signed certificate and its key, in PEM.
    fn self_signed_pair() -> (String, String) {
        let key = rcgen::KeyPair::generate().expect("key gen");
        let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params");
        let cert = params.self_signed(&key).expect("self signed");
        (cert.pem(), key.serialize_pem())
    }

    /// An acceptor serving exactly [`self_signed_pair`] material, built
    /// through the real server-configuration path.
    fn test_acceptor(cert_pem: &str, key_pem: &str) -> tokio_rustls::TlsAcceptor {
        let config = crate::tls::build_tls_server_config(cert_pem, key_pem).expect("server config");
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
    }

    /// A client that trusts the test certificate, so a real handshake can be
    /// driven against [`test_acceptor`].
    fn trusting_client(cert_pem: &str) -> tokio_rustls::TlsConnector {
        let mut roots = rustls::RootCertStore::empty();
        let cert = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem.as_bytes()))
            .next()
            .expect("the test certificate parses to one entry")
            .expect("readable certificate");
        roots.add(cert).expect("trust the test certificate");
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 is supported")
        .with_root_certificates(roots)
        .with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(config))
    }

    // ---------------------------------------------------------------
    //  Admission: rate limit, cap, and the slot the guard holds
    // ---------------------------------------------------------------

    #[test]
    fn admit_holds_its_slot_until_the_guard_drops() {
        let state = test_state(10);
        let guard = admit(&state, &peer(40000)).expect("first connection admits");

        // This is what makes an in-progress handshake countable at all: the
        // slot is held for as long as the caller holds the guard, not for
        // the duration of `admit` itself.
        assert_eq!(active(&state), 1);
        assert_eq!(
            state.metrics.connections_connected.load(Ordering::Relaxed),
            1
        );

        drop(guard);
        assert_eq!(active(&state), 0);
    }

    #[test]
    fn admit_rejects_at_the_cap_without_leaking_the_probe() {
        // Saturate the counter directly: driving 10k admissions through the
        // rate limiter would only re-prove the limiter's budget, not the cap.
        let state = test_state(usize::MAX);
        state
            .active_connections
            .fetch_add(MAX_CONNECTIONS, Ordering::Relaxed);

        assert!(admit(&state, &peer(40001)).is_none(), "at the cap");
        // The probe incremented before it was rejected and must have been
        // given back — a reject path that leaks a slot would slowly deny the
        // whole relay service.
        assert_eq!(active(&state), MAX_CONNECTIONS);
        // ...and a rejected connection was never counted as connected.
        assert_eq!(
            state.metrics.connections_connected.load(Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn admit_refuses_a_spent_ip_before_taking_a_slot() {
        let state = test_state(1);
        let first = admit(&state, &peer(40002)).expect("the budget allows the first");

        assert!(
            admit(&state, &peer(40003)).is_none(),
            "the second connection from the same IP must be refused"
        );
        assert_eq!(
            state
                .metrics
                .messages_dropped_rate_limit
                .load(Ordering::Relaxed),
            1
        );
        // Only the first connection holds a slot: the refusal never touched
        // the counter.
        assert_eq!(active(&state), 1);

        drop(first);
        assert_eq!(active(&state), 0);
    }

    // ---------------------------------------------------------------
    //  The TLS handshake budget
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn tls_handshake_gives_up_on_a_peer_that_never_speaks() {
        let (cert_pem, key_pem) = self_signed_pair();
        let acceptor = test_acceptor(&cert_pem, &key_pem);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");

        // The client connects and then says nothing at all. Without the
        // budget inside `accept_tls`, the future below would never resolve —
        // which is the defect itself, so the outer timeout is only a safety
        // net that turns "hangs forever" into a failure.
        let _silent = TcpStream::connect(addr).await.expect("connect");
        let (stream, _) = listener.accept().await.expect("accept");

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            accept_tls(&acceptor, stream, addr, Duration::from_millis(100)),
        )
        .await;
        let accepted = outcome.expect("accept_tls must give up on its own, not hang");
        assert!(
            accepted.is_none(),
            "a silent peer must not complete a handshake"
        );
    }

    #[tokio::test]
    async fn tls_handshake_completes_within_the_budget_for_a_real_client() {
        let (cert_pem, key_pem) = self_signed_pair();
        let acceptor = test_acceptor(&cert_pem, &key_pem);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let connector = trusting_client(&cert_pem);
        let server_name = ServerName::try_from("localhost".to_string()).expect("server name");

        let client = tokio::spawn(async move {
            let tcp = TcpStream::connect(addr).await.expect("connect");
            connector
                .connect(server_name, tcp)
                .await
                .expect("client handshake");
        });
        let (stream, _) = listener.accept().await.expect("accept");
        let server = accept_tls(&acceptor, stream, peer(40004), Duration::from_secs(5)).await;
        assert!(
            server.is_some(),
            "a real handshake must survive the timeout wrapper"
        );
        tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .expect("client handshake must be bounded")
            .expect("client task must not panic");
    }

    // ---------------------------------------------------------------
    //  The upgrade budget, through the real accept loop
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn plain_accept_loop_drops_a_peer_that_never_sends_a_request() {
        let state = test_state(1000);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_plain_ws_listener(
            listener,
            state.clone(),
            shutdown_rx,
            Duration::from_millis(100),
        );

        // TCP completes, the HTTP request never does.
        let _silent = TcpStream::connect(addr).await.expect("connect");
        assert!(
            wait_for(Duration::from_secs(2), || active(&state) == 1).await,
            "a peer waiting on the upgrade is counted while it waits"
        );
        assert!(
            wait_for(Duration::from_secs(3), || active(&state) == 0).await,
            "the upgrade budget must end the silent peer and release its slot \
             (still {})",
            active(&state)
        );

        let _ = shutdown_tx.send(true);
        task.abort();
    }

    #[tokio::test]
    async fn upgrade_budget_is_released_once_the_upgrade_completes() {
        let state = test_state(1000);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_plain_ws_listener(
            listener,
            state.clone(),
            shutdown_rx,
            Duration::from_millis(100),
        );

        let (mut ws, _) = connect_async(format!("ws://{addr}"))
            .await
            .expect("upgrade inside the budget");
        assert!(
            wait_for(Duration::from_secs(2), || active(&state) == 1).await,
            "the upgraded connection holds a counted slot"
        );

        // Outlive the original budget. Everything after the handshake must
        // pass through untouched, or a connection that took a little longer
        // to authenticate would be killed by a timer that has stopped meaning
        // anything.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let auth = serde_json::json!({
            "type": "relay_auth",
            "device_id": "f157",
            "relay_token": TEST_TOKEN,
        });
        ws.send(Message::Text(auth.to_string()))
            .await
            .expect("send auth after the budget expired");
        let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("auth must be answered")
            .expect("connection must still be open")
            .expect("readable");
        match reply {
            Message::Text(text) => assert!(
                text.contains("relay_auth_ok"),
                "auth sent after the budget expired must be served, got: {text}"
            ),
            other => panic!("expected relay_auth_ok, got {other:?}"),
        }
        assert!(state.clients.read().await.contains_key("f157"));

        drop(ws);
        assert!(
            wait_for(Duration::from_secs(2), || active(&state) == 0).await,
            "dropping the client must release the slot (still {})",
            active(&state)
        );

        let _ = shutdown_tx.send(true);
        task.abort();
    }

    #[tokio::test]
    async fn wss_accept_loop_counts_a_silent_peer_then_drops_it_on_the_handshake_budget() {
        let (cert_pem, key_pem) = self_signed_pair();
        let acceptor = test_acceptor(&cert_pem, &key_pem);
        let state = test_state(1000);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_wss_listener(
            listener,
            state.clone(),
            shutdown_rx,
            acceptor,
            Duration::from_millis(500),
            Duration::from_secs(5),
        );

        // TCP completes; no ClientHello ever arrives — and the peer counts
        // for every millisecond it stalls. Before admission moved ahead of
        // the handshake, this slot never existed at all.
        let _silent = TcpStream::connect(addr).await.expect("connect");
        assert!(
            wait_for(Duration::from_secs(2), || active(&state) == 1).await,
            "a handshake in progress must hold a counted slot"
        );
        assert_eq!(
            state.metrics.connections_connected.load(Ordering::Relaxed),
            1
        );
        assert!(
            wait_for(Duration::from_secs(3), || active(&state) == 0).await,
            "the handshake budget must release the slot (still {})",
            active(&state)
        );

        let _ = shutdown_tx.send(true);
        task.abort();
    }

    #[tokio::test]
    async fn wss_accept_loop_drops_a_stalled_upgrade_and_releases_its_slot() {
        let (cert_pem, key_pem) = self_signed_pair();
        let acceptor = test_acceptor(&cert_pem, &key_pem);
        let connector = trusting_client(&cert_pem);
        let state = test_state(1000);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_wss_listener(
            listener,
            state.clone(),
            shutdown_rx,
            acceptor,
            Duration::from_secs(5),
            Duration::from_millis(100),
        );

        // A real TLS handshake, then silence: no HTTP request ever follows,
        // so only the upgrade budget can end this task.
        let server_name = ServerName::try_from("localhost".to_string()).expect("server name");
        let tcp = TcpStream::connect(addr).await.expect("connect");
        let _tls = connector
            .connect(server_name, tcp)
            .await
            .expect("client TLS handshake");
        assert!(
            wait_for(Duration::from_secs(2), || active(&state) == 1).await,
            "the connection is counted across the stalled upgrade"
        );
        assert!(
            wait_for(Duration::from_secs(3), || active(&state) == 0).await,
            "the upgrade budget must release the slot (still {})",
            active(&state)
        );

        let _ = shutdown_tx.send(true);
        task.abort();
    }

    // ---------------------------------------------------------------
    //  Rate limit bounds the pre-upgrade path
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn over_budget_peer_is_refused_before_the_upgrade() {
        // The production accept loop with the production budgets: the
        // refusal happens before any handshake work, so nothing here waits
        // one out.
        let state = test_state(1);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_plain_ws_listener(
            listener,
            state.clone(),
            shutdown_rx,
            Duration::from_secs(WS_UPGRADE_TIMEOUT_SECS),
        );

        connect_async(format!("ws://{addr}"))
            .await
            .expect("the first connection spends the budget and upgrades");

        // Same IP, budget spent. The upgrade must never happen, so there is
        // no handshake response to read — the client sees the refusal as a
        // failed upgrade, and it arrives promptly rather than by timeout.
        let second = tokio::time::timeout(
            Duration::from_secs(3),
            connect_async(format!("ws://{addr}")),
        )
        .await;
        assert!(
            matches!(second, Ok(Err(_))),
            "the over-budget peer must be refused before any upgrade, got {second:?}"
        );
        assert!(
            wait_for(Duration::from_secs(2), || {
                state
                    .metrics
                    .messages_dropped_rate_limit
                    .load(Ordering::Relaxed)
                    >= 1
            })
            .await,
            "the refusal must be counted in messages_dropped_rate_limit"
        );

        let _ = shutdown_tx.send(true);
        task.abort();
    }
}
