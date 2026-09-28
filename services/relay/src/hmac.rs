//! Re-export of the shared HMAC / replay-protection implementation.
//!
//! The canonical code lives in `conduit_protocol::hmac` so the relay and
//! desktop cannot drift. This module keeps `crate::hmac::…` call sites stable.

pub use conduit_protocol::hmac::*;
