//! The shared, mutable state every connection and every HTTP request reads.
//!
//! [`AppState`] is created once by [`crate::RelayService`] and handed to each
//! connection task. It is behind an `Arc` and contains no I/O, so it can be
//! built in a test without binding a socket.

use std::collections::HashMap;
use std::sync::Arc;

use conduit_protocol::hmac::NonceCache;
use tokio::sync::{RwLock, Semaphore, mpsc};
use tungstenite::Message;

use super::config::Config;
use super::limits::{QUEUE_BYTE_BUDGET, RateLimiter};
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

/// What happened when a message was offered to a connection's queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendOutcome {
    /// Queued. The message holds budget until the writer dequeues it.
    Sent,
    /// The receiving end is gone. Any reservation was released on the way out.
    Closed,
}

/// One connection's outbound queue, and the byte budget that bounds it.
///
/// The channel underneath is `mpsc::channel(QUEUE_DEPTH)`, which bounds the
/// *number* of queued messages and says nothing about their size. Nothing in
/// the path bounded that size either until `MAX_BINARY_SIZE` existed, so a
/// single target connection could hold 1024 messages of whatever an authenticated
/// peer chose to send — at tungstenite's default read cap that was 64 GiB.
///
/// The budget makes the bound a bound on bytes. Every queued message holds
/// `message.len()` permits from a [`Semaphore`] seeded with
/// [`QUEUE_BYTE_BUDGET`], and returns them as the writer dequeues it, so queued
/// bytes are capped no matter how deep the channel runs.
///
/// The raw `Sender` is not exposed, deliberately. This accounting only holds if
/// *every* producer reserves and the writer releases for exactly what it took
/// — a producer that enqueues without reserving, or a writer that releases for
/// a message nobody reserved, quietly inflates the budget back towards
/// unbounded. Forcing producers through [`Queue::send`] is what keeps the two
/// halves in step.
#[derive(Clone)]
pub(crate) struct Queue {
    tx: mpsc::Sender<Message>,
    budget: Arc<Semaphore>,
    /// Which connection this queue belongs to.
    ///
    /// One [`Queue::new`] per connection, and every clone carries the same id
    /// — that is what lets deregistration tell "my own entry" apart from "an
    /// entry a later connection for the same device wrote". See
    /// [`deregister_if_current`].
    connection_id: u64,
}

/// Process-wide counter that makes each [`Queue`] unique.
///
/// Deliberately monotonic rather than random: the id is only ever compared
/// for equality within one process, and a counter makes "a clone of my queue
/// is still my queue" and "some other connection's queue is not" follow from
/// one shared fact.
static NEXT_CONNECTION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Queue {
    pub(crate) fn new(tx: mpsc::Sender<Message>) -> Self {
        Self {
            tx,
            budget: Arc::new(Semaphore::new(QUEUE_BYTE_BUDGET)),
            connection_id: NEXT_CONNECTION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// The connection this queue serves. Stable across clones; distinct for
    /// every [`Queue::new`].
    pub(crate) fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// True once the connection's reader has gone away.
    ///
    /// What `reconcile_clients` prunes on: a task that panics or is aborted
    /// never reaches its own cleanup, and would otherwise leave a dead entry
    /// that makes the device look connected forever.
    pub(crate) fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Reserve the message's bytes, then queue it.
    ///
    /// Waiting on the budget and waiting on a full channel are the same
    /// condition from the caller's point of view — the target is not accepting
    /// — so both surface as `Sent`/`Closed` here and let the caller's timeout
    /// decide how long to wait. A `Closed` return means the reservation has
    /// already been given back.
    pub(crate) async fn send(&self, message: Message) -> SendOutcome {
        let units = budget_units(message.len());
        // Never truncates and never yields 0: ingress is capped at
        // `MAX_BINARY_SIZE`, well below `u32`, and `budget_units` returns at
        // least 1.
        let permit = match self.budget.clone().acquire_many_owned(units as u32).await {
            Ok(permit) => permit,
            // Only when the budget itself has been dropped, which means the
            // connection is gone.
            Err(_) => return SendOutcome::Closed,
        };
        match self.tx.send(message).await {
            Ok(()) => {
                // Forget rather than drop. The writer returns exactly these
                // units when it dequeues, and dropping the permit here would
                // return them a second time.
                permit.forget();
                SendOutcome::Sent
            }
            Err(_) => SendOutcome::Closed,
        }
    }

    /// Give `len` bytes back. Called once per message by the writer, for every
    /// message [`Queue::send`] accepted.
    pub(crate) fn release(&self, len: usize) {
        self.budget.add_permits(budget_units(len));
    }
}

/// Units a message of `len` bytes costs.
///
/// A zero-length message reserves one rather than zero: both sides use this
/// same function, so they agree either way, and it sidesteps having to reason
/// about whether acquiring nothing is well-behaved.
fn budget_units(len: usize) -> usize {
    len.max(1)
}

pub(crate) type Clients = Arc<RwLock<HashMap<String, Queue>>>;

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

/// Remove `device_id`'s routing entry, but only if it is still
/// `connection_id`'s.
///
/// The clean-disconnect path used to remove by key alone. A device that
/// reconnects while its previous connection is still winding down *overwrites*
/// its own entry, so the older connection's late cleanup then evicts the newer
/// connection's registration: the socket stays up, the device looks connected
/// to itself, and every route to it is dropped as `not_found` forever —
/// `reconcile_clients` cannot repair it, because the live entry's sender is
/// open and healthy.
///
/// Matching on [`Queue::connection_id`] closes that window: the older
/// connection finds an entry it does not own and leaves it alone. Returns
/// whether an entry was actually removed, so callers and tests can tell "I
/// deregistered" from "I was already superseded".
pub(crate) async fn deregister_if_current(
    clients: &Clients,
    device_id: &str,
    connection_id: u64,
) -> bool {
    let mut map = clients.write().await;
    let owned = map
        .get(device_id)
        .is_some_and(|queue| queue.connection_id() == connection_id);
    if owned {
        map.remove(device_id);
    }
    owned
}
