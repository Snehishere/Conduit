//! The shared, mutable state every connection and every HTTP request reads.
//!
//! [`AppState`] is created once by [`crate::RelayService`] and handed to each
//! connection task. It is behind an `Arc` and contains no I/O, so it can be
//! built in a test without binding a socket.

use std::collections::HashMap;
use std::sync::Arc;

use conduit_protocol::hmac::NonceCache;
use tokio::sync::{RwLock, mpsc};
use tungstenite::Message;

use super::config::Config;
use super::limits::RateLimiter;
use super::metrics::Metrics;

/// Where the relay gets a device's `relay_route` signing key.
///
/// The relay deliberately does not know how keys are produced. It knows only
/// that a route is signed by exactly one device, under a key derived from the
/// secret that device paired with, and it asks its host for that key.
///
/// The host owns the device registry, so this keeps the relay crate free of any
/// storage dependency and means a key can be rotated by the thing that holds
/// the registry — a re-pair — without the relay being reconfigured. See
/// `conduit_protocol::hmac::derive_route_key`.
pub trait RouteKeys: Send + Sync + std::fmt::Debug {
    /// The key `device_id` signs with, or `None` if it is not a known device.
    ///
    /// Returning `None` for an unknown device is what makes an unregistered
    /// device unroutable, so this must not fall back to a shared default.
    fn key_for(&self, device_id: &str) -> Option<Vec<u8>>;

    /// Every registered device id, for the startup log and diagnostics.
    fn device_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A [`RouteKeys`] backed by an in-memory map.
///
/// Useful on its own for a host that keeps its keys in a `HashMap`, and as the
/// thing to wrap when the registry is behind a lock.
#[derive(Debug, Default, Clone)]
pub struct StaticRouteKeys {
    keys: std::collections::HashMap<String, Vec<u8>>,
}

impl StaticRouteKeys {
    /// An empty set. Nothing verifies until a device is registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register every `(device_id, key)` pair at once.
    pub fn from_pairs<I, K>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, Vec<u8>)>,
        K: Into<String>,
    {
        let mut keys = Self::new();
        for (device_id, key) in pairs {
            keys.insert(device_id, key);
        }
        keys
    }
    /// Register or replace a device's key.
    pub fn insert(&mut self, device_id: impl Into<String>, key: impl Into<Vec<u8>>) {
        self.keys.insert(device_id.into(), key.into());
    }

    /// Forget a device, so an unpaired one stops being able to sign.
    pub fn remove(&mut self, device_id: &str) {
        self.keys.remove(device_id);
    }

    /// How many devices are registered.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether no device is registered.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

impl RouteKeys for StaticRouteKeys {
    fn key_for(&self, device_id: &str) -> Option<Vec<u8>> {
        self.keys.get(device_id).cloned()
    }

    fn device_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.keys.keys().cloned().collect();
        ids.sort();
        ids
    }
}

pub(crate) type Clients = Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>;

pub(crate) struct ConnectionGuard {
    pub(crate) count: Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) struct AppState {
    pub(crate) clients: Clients,
    pub(crate) active_connections: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) config: Config,
    /// Resolves a device id to the key it signs routes with. Supplied by the
    /// host, which owns the device registry.
    pub(crate) route_keys: Arc<dyn RouteKeys>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    /// Shared nonce replay cache — process-wide (not per-connection) so a
    /// reconnect cannot replay a previously accepted nonce, and persisted
    /// across restarts via `hmac::save_nonces`. Entries are scoped per
    /// authenticated device id so one client cannot evict another's.
    pub(crate) nonces: Arc<RwLock<NonceCache>>,
    /// `sha256/<base64>` SPKI pin of the served certificate, served by
    /// `GET /pin`. `None` when TLS failed to initialise.
    pub(crate) tls_pin: Option<String>,
}

/// Remove entries from the routing table whose outbound channel has closed.
///
/// The clean disconnect path already removes a device on the way out, but an
/// aborted or panicked `handle_connection` task leaves a dead `Sender` behind
/// forever, which both leaks memory and makes the device permanently appear
/// "connected". Returns the number of entries removed.
pub(crate) async fn reconcile_clients(clients: &Clients) -> usize {
    let mut map = clients.write().await;
    let before = map.len();
    map.retain(|_, tx| !tx.is_closed());
    before - map.len()
}
