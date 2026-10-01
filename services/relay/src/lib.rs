//! The Conduit relay: a router that carries already-encrypted Conduit traffic
//! between two devices that are not on the same network.
//!
//! # What this is
//!
//! Conduit encrypts each message to its peer before it leaves the device
//! ([ADR-0007](../../docs/decisions/0007-refuse-to-ship-end-to-end-encryption-claims.md)).
//! The relay never sees plaintext and never terminates that encryption. It
//! authenticates a device, verifies that a route was signed by a key it
//! trusts, and hands the opaque payload to the named device. That is the whole
//! job, and the rest of this crate is the machinery for doing it without
//! becoming a liability.
//!
//! It is not a general-purpose message broker: it speaks one protocol,
//! [`conduit_protocol`], and refuses frames it does not recognise.
//!
//! # What this is *not*
//!
//! This is a **library**, not a program. The relay runs inside the desktop app
//! as a background task — see [`RelayService`] — so that pairing a phone from
//! another network is a setting rather than a deployment.
//!
//! It is therefore **not** end-to-end encryption, and it is not a privacy
//! boundary against the desktop. A message that crosses the relay was encrypted
//! peer-to-peer, but the operator of the machine running the relay can read the
//! ciphertext's metadata (who talked to whom, when, how much) even when they
//! cannot read the payload. See `SECURITY.md`.
//!
//! # Shape
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`service`] | Start, run, stop. The public entry point. |
//! | [`config`] | Settings, precedence, and the secrets. |
//! | [`connection`] | One socket: the auth gate and its read loop. |
//! | [`route`] | Verify a `relay_route`, then forward it. |
//! | [`state`] | The routing table and other shared mutable state. |
//! | [`health`] | `/healthz`, `/health`, `/metrics`, `/pin`. |
//! | [`metrics`] | Counters and their two exposition formats. |
//! | [`limits`] | Fixed capacities and rate limiters. |
//! | `tls` | Certificate generation and the SPKI pin. |
//! | `hmac` | Re-export of [`conduit_protocol::hmac`]. |
//!
//! # Example
//!
//! ```no_run
//! # use conduit_relay::{RelayService, RouteKeys, StaticRouteKeys};
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let route_keys: std::sync::Arc<dyn RouteKeys> = std::sync::Arc::new(StaticRouteKeys::new());
//! let relay = RelayService::builder().start(route_keys).await?;
//! // Relay is live. `relay` is a handle: drop does not stop it.
//! let _ = relay.shutdown().await;
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]
// `deny` rather than `forbid`, so the test suite can opt in. The one thing that
// legitimately needs `unsafe` is mutating the process environment, which is
// `unsafe` in edition 2024 and which the config-precedence tests must do.
#![deny(unsafe_code)]

pub mod config;
pub mod connection;
pub mod health;
pub mod limits;
pub mod metrics;
pub mod route;
pub mod service;
pub mod state;
/// Certificate generation, loading and the SPKI pin.
pub mod tls;

/// Re-export of `conduit_protocol::hmac`, the single implementation of
/// signing, key derivation and the replay cache.
pub(crate) mod hmac;

pub use config::{Config, Overrides};
pub use service::{RelayService, RelayServiceBuilder, ServiceHandle, Shutdown, StartError, Status};

/// The canonical message types and the crypto, re-exported so a host does not
/// have to depend on `conduit-protocol` directly to build a valid client.
pub use conduit_protocol::{
    BINARY_FRAME_VERSION, BINARY_HEADER_LEN, ErrorMessage, RelayAuth, RelayAuthOk,
    RelayAuthRejected, RelayRoute,
};

// Internals are re-exported for the in-crate test suite. Nothing here is
// reachable from outside the crate.

// The test suite is written against the relay as a whole rather than against a
// dozen module paths, so the internals are flattened onto the crate root for it.
// This is `cfg(test)`-only on purpose: in a real build these names are reached
// through their own modules, and re-exporting them here would be dead weight the
// compiler rightly complains about.
#[cfg(test)]
pub(crate) use config::bearer_token_authorized;
#[cfg(test)]
pub(crate) use connection::{DRAIN_TIMEOUT_SECS, drain_connections, handle_connection};
#[cfg(test)]
pub(crate) use health::spawn_health_server;
#[cfg(test)]
pub(crate) use limits::{MessageRateLimiter, RateLimiter, is_valid_device_id};
#[cfg(test)]
pub(crate) use metrics::{Metrics, build_metrics_json, build_prometheus_metrics};
#[cfg(test)]
pub(crate) use route::{
    Rejection, RejectionKind, forward_text, handle_binary_frame, validate_device_id,
};
#[cfg(test)]
pub(crate) use state::{AppState, Clients, ConnectionGuard, Queue, SendOutcome, reconcile_clients};

/// Route-key resolution is part of the host-facing contract, because the host
/// (not the relay) owns the device registry and therefore the keys.
pub use state::{RouteKeys, StaticRouteKeys};

#[cfg(test)]
#[cfg(test)]
mod suite;
