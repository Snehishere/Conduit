//! # conduit-protocol
//!
//! Canonical wire protocol definitions for the Conduit ecosystem.
//!
//! This crate is the **single source of truth** for every JSON message type
//! exchanged between desktop, mobile, and relay. Both the Rust desktop app
//! and the relay server import these types to prevent protocol drift.
//!
//! Two limits on that claim, both real:
//!
//! * `schema.json` is **hand-maintained**, not generated from `types.rs`.
//!   The two are held together by tests (see ADR-0010), so drift fails the
//!   build rather than going unnoticed — but the JSON schema is not derived
//!   from this crate.
//! * The relay does not open `encrypted` envelopes, so it routes their bytes
//!   without reading them. The desktop hub *does* terminate them. The
//!   encryption is therefore per-peer hop encryption, not end-to-end; see
//!   ADR-0007.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────┐    JSON / binary frames    ┌──────────┐
//! │  Mobile  │◄──────────────────────────►│  Desktop │
//! └────┬─────┘                            └────┬─────┘
//!      │                                       │
//!      │          ┌─────────────┐              │
//!      └─────────►│   Relay     │◄─────────────┘
//!                 └─────────────┘
//! ```
//!
//! The relay authenticates connections, verifies `relay_route` signatures and
//! enforces per-connection limits; it is not a blind forwarder, and it cannot
//! read an `encrypted` envelope.

pub mod types;

pub use types::*;

/// Shared HMAC-SHA256 helpers, replay protection, and nonce persistence.
///
/// Single implementation used by the relay and the desktop app so signing
/// and verification can never drift between binaries.
pub mod hmac {
    use hmac::{Hmac, Mac};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::atomic::{AtomicU64, Ordering};
    use subtle::ConstantTimeEq;

    type HmacSha256 = Hmac<Sha256>;

    /// Maximum nonces retained in the replay cache (process-wide ceiling).
    pub const MAX_NONCES: usize = 10_000;

    /// Maximum nonces retained for a **single** authenticated device.
    ///
    /// Scoping the cache per device is what stops one high-volume client from
    /// evicting everybody else's replay protection: a device that fills its own
    /// quota only ever drops its own oldest nonces.
    pub const MAX_NONCES_PER_DEVICE: usize = 4_096;

    /// Separator used when a `(device_id, nonce)` pair is flattened into the
    /// single string key used by the persisted `nonces.json` format.
    ///
    /// ASCII unit separator (0x1F) — it cannot occur in a validated device id
    /// (`^[0-9a-f]{1,64}$`) and keeps the on-disk keys unambiguous.
    pub const SCOPE_SEPARATOR: char = '\u{1f}';

    /// Domain-separation label for a device's `relay_route` signing key.
    ///
    /// The device id is appended to this label, so two devices sharing a
    /// pairing secret still get unrelated keys. See [`derive_route_key`].
    pub const ROUTE_KEY_LABEL: &str = "conduit-relay/v1/route-key";

    /// Fixed prefix binding every derived key to this protocol and KDF version.
    ///
    /// Bumping this string retires *all* previously derived keys at once.
    const KDF_PREFIX: &[u8] = b"conduit-protocol/v1/derive:";

    // =====================================================================
    //  Domain separation
    // =====================================================================

    /// Derive a purpose-bound subkey from a master secret.
    ///
    /// This is a labelled PRF-based KDF: `HMAC-SHA256(master,
    /// KDF_PREFIX || label)` truncated to the 32-byte SHA-256 output. HMAC's
    /// PRF property gives the same guarantee HKDF-Extract does — knowledge of
    /// one derived key reveals nothing about the master secret — while
    /// remaining a single HMAC invocation with no extra dependency.
    ///
    /// The `label` is a **required** argument by design: it is the mechanism
    /// that stops a derived key from being the auth token or the master secret.
    /// Never call this with an empty label, and never pass a user-supplied
    /// string as the label.
    pub fn derive_key(master: &[u8], label: &str) -> [u8; 32] {
        assert!(
            !label.is_empty(),
            "derive_key requires a non-empty domain-separation label"
        );
        let mut mac = HmacSha256::new_from_slice(master).expect("HMAC accepts any key length");
        mac.update(KDF_PREFIX);
        mac.update(label.as_bytes());
        let out = mac.finalize().into_bytes();
        let mut key = [0u8; 32];
        key.copy_from_slice(&out);
        key
    }

    /// Derive the key a device signs its `relay_route` messages with.
    ///
    /// `pairing_secret` is the X25519 secret already established when this
    /// device paired with the desktop hub; `device_id` is the id the hub filed
    /// it under, and is bound into the label so the same secret under two
    /// device ids yields unrelated keys.
    ///
    /// # Why this replaces a shared signing key
    ///
    /// The relay used to verify every route against one key derived from its
    /// own master secret. That key was, by construction, something no client
    /// could hold — so no client could produce a valid route, and the whole
    /// relay path was unreachable from either end. The only alternative, giving
    /// clients the bearer token, lets any one of them forge a route claiming to
    /// be any other.
    ///
    /// Deriving from the per-device pairing secret closes both gaps at once: the
    /// client already holds this secret, and the hub can derive the same key
    /// from the row it stored at pairing time. A device can sign for itself and
    /// for nothing else, and a rotated secret (a re-pair) retires the old key
    /// automatically — which is why there is no rotation window here.
    ///
    /// The device id is caller-supplied and must be the one on the wire, or
    /// verification will not find the key. It is not treated as secret.
    pub fn derive_route_key(pairing_secret: &[u8], device_id: &str) -> [u8; 32] {
        let label = format!("{ROUTE_KEY_LABEL}:{device_id}");
        derive_key(pairing_secret, &label)
    }

    // =====================================================================
    //  Raw MAC
    // =====================================================================

    /// Raw SHA-256 digest.
    ///
    /// Exposed (rather than re-deriving the dependency in the relay) because
    /// certificate pinning needs a bare digest: the relay hashes the
    /// certificate's SubjectPublicKeyInfo to produce the `sha256/<base64>` pin
    /// that clients pin. Not an authenticator — use [`compute_hmac`] for that.
    pub fn sha256(bytes: &[u8]) -> [u8; 32] {
        let digest = Sha256::digest(bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        out
    }

    /// Compute HMAC-SHA256 and return the hex-encoded digest.
    pub fn compute_hmac(secret: &[u8], message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Verify an expected hex HMAC against `message` in constant time.
    pub fn verify_hmac(secret: &[u8], message: &str, expected_hex: &str) -> bool {
        let Ok(expected_bytes) = hex::decode(expected_hex) else {
            return false;
        };
        let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
            return false;
        };
        mac.update(message.as_bytes());
        let computed = mac.finalize().into_bytes();
        let computed_bytes: &[u8] = &computed;
        let expected_slice: &[u8] = &expected_bytes;
        bool::from(ConstantTimeEq::ct_eq(computed_bytes, expected_slice))
    }

    /// The canonical, **ordered** set of fields covered by a message HMAC.
    ///
    /// This list *is* the contract: it is what [`canonical_signing_string`]
    /// hashes and what [`verify_message_hmac`] reconstructs. Anything not in
    /// this list is unauthenticated — adding a security-relevant field to a
    /// signed message without adding it here is the exact class of bug that let
    /// `relay_route` carry an unauthenticated sender, so any new signed field
    /// MUST be added here **and** to the client signers.
    ///
    /// `from_device_id` and `key_id` are both covered because a route is
    /// verified by looking its key up under `key_id` and then requiring
    /// `from_device_id` to match. Both are signed, so neither can be rewritten
    /// to point a valid signature at a different device.
    pub const SIGNED_FIELDS: [&str; 7] = [
        "type",
        "from_device_id",
        "to_device_id",
        "payload",
        "timestamp",
        "nonce",
        "key_id",
    ];

    /// Rebuild the canonical JSON string that the message HMAC is computed over.
    ///
    /// Only [`SIGNED_FIELDS`] participate, in [`SIGNED_FIELDS`] order; fields
    /// that are absent from `json` are simply omitted, which keeps legacy
    /// unsigned-field messages verifiable while still failing closed on the
    /// relay (the relay requires the fields before it will route anything).
    pub fn canonical_signing_string(json: &Value) -> String {
        let mut map = serde_json::Map::new();
        for field in SIGNED_FIELDS {
            if let Some(v) = json.get(field) {
                map.insert(field.to_string(), v.clone());
            }
        }
        serde_json::to_string(&Value::Object(map)).unwrap_or_default()
    }

    /// Verify the `hmac` field on a signed JSON message (canonical field subset).
    pub fn verify_message_hmac(secret: &[u8], json: &Value) -> bool {
        verify_message_hmac_raw(secret, json, None)
    }

    /// Verify a signed message, optionally requiring a specific `key_id`.
    ///
    /// The primitive behind [`RouteKeyring::verify`]. It does **not** require
    /// `key_id` to be present: that is the keyring's job, and a caller checking a
    /// bare MAC is asking a narrower question.
    fn verify_message_hmac_raw(secret: &[u8], json: &Value, required_key_id: Option<&str>) -> bool {
        if let Some(hmac_hex) = json.get("hmac").and_then(|v| v.as_str()) {
            if let Some(want) = required_key_id {
                if json.get("key_id").and_then(|v| v.as_str()) != Some(want) {
                    return false;
                }
            }
            let message_str = canonical_signing_string(json);
            return verify_hmac(secret, &message_str, hmac_hex);
        }
        false
    }

    // =====================================================================
    //  Route keys
    // =====================================================================

    /// The per-device keys a relay accepts, keyed by device id.
    ///
    /// One key per device, each derived from that device's own pairing secret
    /// (see [`derive_route_key`]). There is deliberately no rotation window and
    /// no shared operator key:
    ///
    /// * a device can sign for itself and for nothing else, so a compromised
    ///   phone cannot forge traffic as the desktop or as another phone;
    /// * a key changes only when the device re-pairs, which is exactly when
    ///   its pairing secret changes, so there is no window to manage and no
    ///   `RELAY_SIGNING_KEY_PREVIOUS` to unset and forget.
    ///
    /// The relay builds this from its own device registry; a client holds one
    /// key, its own.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct RouteKeyring {
        keys: HashMap<String, Vec<u8>>,
    }

    impl RouteKeyring {
        /// An empty ring. Nothing verifies until a device is registered.
        pub fn new() -> Self {
            Self::default()
        }

        /// Register (or replace) the key for `device_id`.
        ///
        /// Replacing rather than rejecting is deliberate: re-pairing a device
        /// changes its secret, and a ring that pinned the old key would leave
        /// the device permanently unable to route.
        pub fn insert(&mut self, device_id: impl Into<String>, secret: impl Into<Vec<u8>>) {
            self.keys.insert(device_id.into(), secret.into());
        }

        /// Forget `device_id`. Called when a device is unpaired, so a revoked
        /// device cannot keep signing.
        pub fn remove(&mut self, device_id: &str) {
            self.keys.remove(device_id);
        }

        /// Retain only the devices in `keep`.
        pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
            self.keys.retain(|id, _| keep(id));
        }

        /// The key for `device_id`, if it is registered.
        pub fn key_for(&self, device_id: &str) -> Option<&[u8]> {
            self.keys.get(device_id).map(|v| v.as_slice())
        }

        /// Every registered device id, sorted.
        pub fn device_ids(&self) -> Vec<&str> {
            let mut ids: Vec<&str> = self.keys.keys().map(|s| s.as_str()).collect();
            ids.sort_unstable();
            ids
        }

        /// How many devices are registered.
        pub fn len(&self) -> usize {
            self.keys.len()
        }

        /// Whether no device is registered.
        pub fn is_empty(&self) -> bool {
            self.keys.is_empty()
        }

        /// Verify a signed `relay_route`.
        ///
        /// Returns the device id that validated it. `None` when the message
        /// does not name its signer, names one that is not registered, or the
        /// MAC fails.
        ///
        /// The signer is taken from `key_id`, and `from_device_id` is required
        /// to equal it. Requiring the two to agree is what stops a valid
        /// signature from being replayed under a different claimed sender: the
        /// key is looked up by the same name the payload is attributed to.
        pub fn verify(&self, json: &Value) -> Option<String> {
            // The key id is what the key is looked up by, so it is mandatory:
            // defaulting it would attribute a route to whichever device happened
            // to be registered.
            let signer = json.get("key_id").and_then(|v| v.as_str())?;
            let claimed = json.get("from_device_id").and_then(|v| v.as_str())?;
            if signer != claimed {
                return None;
            }
            let secret = self.key_for(signer)?;
            verify_message_hmac_raw(secret, json, Some(signer)).then(|| signer.to_string())
        }
    }

    // =====================================================================
    //  Replay protection
    // =====================================================================

    /// Accept a fresh timestamp+nonce (within clock-skew window) exactly once.
    ///
    /// Unscoped single-map form retained for callers that keep one map per
    /// device. Prefer [`NonceCache`], which scopes by device id and never
    /// clears wholesale.
    pub fn check_replay(
        timestamp: i64,
        nonce: &str,
        nonces: &mut std::collections::HashMap<String, i64>,
    ) -> bool {
        let now = now_millis();
        let age = now - timestamp;
        if !(-5_000..=30_000).contains(&age) {
            return false;
        }

        let nonce_key = nonce.to_string();
        if nonces.contains_key(&nonce_key) {
            return false;
        }

        if nonces.len() >= MAX_NONCES {
            // Prune what the freshness cutoff allows...
            let cutoff = now - 60_000;
            nonces.retain(|_, ts| *ts > cutoff);
            // ...then evict oldest-first until back under the cap. A full
            // `clear()` here would let any client that can reach the cap
            // instantly re-enable replay for every nonce ever accepted.
            while nonces.len() >= MAX_NONCES {
                let oldest = nonces
                    .iter()
                    .min_by_key(|(_, ts)| **ts)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => {
                        nonces.remove(&k);
                    }
                    None => break,
                }
            }
        }

        nonces.insert(nonce_key, now);
        true
    }

    /// Per-device nonce state: insertion order plus a membership set.
    #[derive(Debug, Default)]
    struct DeviceNonces {
        /// `(nonce, accepted_at_ms)`, oldest first — the eviction queue.
        order: VecDeque<(String, i64)>,
        /// Fast duplicate rejection; kept in sync with `order`.
        seen: HashSet<String>,
    }

    impl DeviceNonces {
        fn prune_before(&mut self, cutoff: i64) {
            while let Some((nonce, ts)) = self.order.front() {
                if *ts > cutoff {
                    break;
                }
                let nonce = nonce.clone();
                self.order.pop_front();
                self.seen.remove(&nonce);
            }
        }

        fn evict_oldest(&mut self) -> Option<(String, i64)> {
            let (nonce, ts) = self.order.pop_front()?;
            self.seen.remove(&nonce);
            Some((nonce, ts))
        }

        fn len(&self) -> usize {
            self.order.len()
        }
    }

    /// Device-scoped replay cache.
    ///
    /// Two properties this type exists to guarantee:
    ///
    /// 1. **No global reset.** When a quota is reached the cache evicts
    ///    *oldest-first* — never `clear()`. There is therefore no input that
    ///    re-enables replay for previously accepted nonces.
    /// 2. **Per-device quotas.** Each device gets its own
    ///    [`MAX_NONCES_PER_DEVICE`] budget, so one legitimate high-volume
    ///    client cannot push another client's recent nonces out of the cache.
    #[derive(Debug, Default)]
    pub struct NonceCache {
        devices: HashMap<String, DeviceNonces>,
    }

    impl NonceCache {
        pub fn new() -> Self {
            Self::default()
        }

        /// Rebuild from the flat `"<device>\u{1f}<nonce>" -> ms` map produced by
        /// [`load_nonces`] / [`NonceCache::to_map`].
        pub fn from_map(map: &HashMap<String, i64>) -> Self {
            let mut cache = Self::new();
            // Insert oldest-first so `order` stays timestamp-ordered — the
            // eviction queue's front must always be the globally oldest entry.
            let mut entries: Vec<(&String, &i64)> = map.iter().collect();
            entries.sort_by_key(|(_, ts)| **ts);
            for (key, ts) in entries {
                match key.split_once(SCOPE_SEPARATOR) {
                    Some((device, nonce)) => cache.insert_raw(device, nonce.to_string(), *ts),
                    None => {
                        // Unscoped legacy entry: keep it reachable under a
                        // reserved bucket rather than dropping it (fail closed).
                        let nonce = key.clone();
                        cache.insert_raw(LEGACY_SCOPE, nonce, *ts);
                    }
                }
            }
            cache
        }

        /// Flatten for persistence. Round-trips through [`NonceCache::from_map`].
        pub fn to_map(&self) -> HashMap<String, i64> {
            let mut out = HashMap::with_capacity(self.devices.len());
            for (device, state) in &self.devices {
                for (nonce, ts) in &state.order {
                    out.insert(format!("{device}{SCOPE_SEPARATOR}{nonce}"), *ts);
                }
            }
            out
        }

        /// Total number of remembered nonces across all devices.
        pub fn total_len(&self) -> usize {
            self.devices.values().map(DeviceNonces::len).sum()
        }

        /// Number of remembered nonces for one device.
        pub fn len_for(&self, device_id: &str) -> usize {
            self.devices.get(device_id).map_or(0, DeviceNonces::len)
        }

        /// Whether `nonce` is currently remembered for `device_id`.
        pub fn contains(&self, device_id: &str, nonce: &str) -> bool {
            self.devices
                .get(device_id)
                .is_some_and(|s| s.seen.contains(nonce))
        }

        fn insert_raw(&mut self, device: &str, nonce: String, ts: i64) {
            let state = self.devices.entry(device.to_string()).or_default();
            if state.seen.insert(nonce.clone()) {
                state.order.push_back((nonce, ts));
            }
        }

        /// Accept a fresh timestamp+nonce for `device_id` exactly once.
        ///
        /// `device_id` MUST be the *authenticated* connection identity, never a
        /// value taken from the message body — otherwise one client can evict
        /// another client's replay protection by naming them.
        pub fn check_replay(&mut self, device_id: &str, timestamp: i64, nonce: &str) -> bool {
            let now = now_millis();
            let age = now - timestamp;
            if !(-5_000..=30_000).contains(&age) {
                return false;
            }
            if nonce.is_empty() {
                return false;
            }
            if self.contains(device_id, nonce) {
                return false;
            }

            self.prune_before(now - 60_000);
            self.enforce_quota(device_id);

            let state = self.devices.entry(device_id.to_string()).or_default();
            state.seen.insert(nonce.to_string());
            state.order.push_back((nonce.to_string(), now));
            true
        }

        /// Enforce the per-device quota then the process-wide ceiling, always by
        /// evicting the oldest remembered nonce. Never clears.
        fn enforce_quota(&mut self, device_id: &str) {
            let cutoff = now_millis() - 60_000;
            if let Some(state) = self.devices.get_mut(device_id) {
                state.prune_before(cutoff);
                while state.len() >= MAX_NONCES_PER_DEVICE {
                    if state.evict_oldest().is_none() {
                        break;
                    }
                }
            }

            // Process-wide ceiling. Reached only when the sum of per-device
            // quotas exceeds MAX_NONCES; evict the globally oldest entry.
            let mut guard = 0usize;
            while self.total_len() >= MAX_NONCES {
                let Some(victim) = self.oldest_device() else {
                    break;
                };
                let Some(state) = self.devices.get_mut(&victim) else {
                    break;
                };
                if state.evict_oldest().is_none() {
                    break;
                }
                guard += 1;
                if guard > MAX_NONCES + 1 {
                    break;
                }
            }
            self.devices.retain(|_, state| state.len() > 0);
        }

        /// The device whose oldest remembered nonce is the globally oldest.
        fn oldest_device(&self) -> Option<String> {
            self.devices
                .iter()
                .filter_map(|(device, state)| state.order.front().map(|(_, ts)| (device, *ts)))
                .min_by_key(|(_, ts)| *ts)
                .map(|(device, _)| device.clone())
        }

        /// Drop every entry older than 60 s.
        pub fn prune_before(&mut self, cutoff: i64) {
            for state in self.devices.values_mut() {
                state.prune_before(cutoff);
            }
            self.devices.retain(|_, state| state.len() > 0);
        }
    }

    /// Bucket for unscoped entries restored from a pre-scoping `nonces.json`.
    ///
    /// Authenticated device ids are `^[0-9a-f]{1,64}$`, so this cannot collide
    /// with a real device id.
    const LEGACY_SCOPE: &str = "legacy-unscoped";

    /// Nonce component counter — makes [`new_nonce`] unique within a process.
    static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Generate a fresh replay nonce.
    ///
    /// Built from `now_millis` + pid + a monotonic counter, hashed with
    /// SHA-256. The inputs are all observable to a local process, so this is
    /// uniqueness within a relay process, **not** unpredictability: it carries
    /// no security property against a peer that can guess the output.
    /// [`NonceCache::check_replay`] needs uniqueness, which this provides, so
    /// it is adequate for the relay's own bookkeeping. A client putting a nonce
    /// on the wire MUST use a cryptographically random one instead. This
    /// constructor exists for tests and for local tooling that has no RNG.
    pub fn new_nonce() -> String {
        let counter = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let seed = format!(
            "{}|{}|{}|{}",
            now_millis(),
            std::process::id(),
            counter,
            NONCE_COUNTER.load(Ordering::Relaxed)
        );
        hex::encode(Sha256::digest(seed.as_bytes()))
    }

    /// Current wall-clock time in milliseconds since the Unix epoch.
    pub fn now_millis() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    /// Load the persisted nonce replay cache from `path`.
    ///
    /// - Missing file → empty cache (first start).
    /// - Unreadable file → empty cache, silently. A permissions error is
    ///   indistinguishable from "first start" here, so a relay whose nonce file
    ///   cannot be read loses replay protection across restarts without a log
    ///   line. Only a *parse* failure is logged.
    /// - Corrupt file → empty cache + warn. Fail-open here is acceptable: the
    ///   worst case is one replayed message inside the 30s window being accepted
    ///   once after a corrupt-state restart — losing every nonce would be worse.
    /// - Entries older than 60s are pruned on load (matches `check_replay` cutoff).
    /// - If more than [`MAX_NONCES`] entries survive the prune, only the most
    ///   recent are kept.
    pub fn load_nonces(path: &std::path::Path) -> std::collections::HashMap<String, i64> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return std::collections::HashMap::new(),
        };
        let map: std::collections::HashMap<String, i64> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "Nonce cache at {} is corrupt ({}); starting empty",
                    path.display(),
                    e
                );
                return std::collections::HashMap::new();
            }
        };

        let now = now_millis();
        let cutoff = now - 60_000;
        let mut fresh: Vec<(String, i64)> =
            map.into_iter().filter(|(_, ts)| *ts > cutoff).collect();

        // Enforce the same cap as check_replay: keep the most recent entries.
        if fresh.len() > MAX_NONCES {
            fresh.sort_by_key(|(_, ts)| std::cmp::Reverse(*ts));
            fresh.truncate(MAX_NONCES);
        }

        fresh.into_iter().collect()
    }

    /// Persist the nonce replay cache atomically (tmp + rename), pruning entries
    /// older than 60s. Never logs nonce values.
    pub fn save_nonces(
        path: &std::path::Path,
        nonces: &std::collections::HashMap<String, i64>,
    ) -> Result<(), String> {
        let now = now_millis();
        let cutoff = now - 60_000;
        let pruned: std::collections::HashMap<&str, i64> = nonces
            .iter()
            .filter(|(_, ts)| **ts > cutoff)
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        let json = serde_json::to_string(&pruned)
            .map_err(|e| format!("Failed to serialize nonce cache: {}", e))?;

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
            }
        }

        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("Failed to replace {}: {}", path.display(), e))?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // ---------------------------------------------------------------
        //  verify_hmac tests
        // ---------------------------------------------------------------

        #[test]
        fn verify_hmac_known_vector_roundtrip() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let message = "the quick brown fox";
            let expected = compute_hmac(secret, message);
            assert!(verify_hmac(secret, message, &expected));
        }

        #[test]
        fn verify_hmac_wrong_secret_rejects() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let wrong = b"wrong-key-aaaaaaaaaaaaaaaaaaaaaaa";
            let mac = compute_hmac(secret, "payload");
            assert!(!verify_hmac(wrong, "payload", &mac));
        }

        #[test]
        fn verify_hmac_wrong_message_rejects() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let mac = compute_hmac(secret, "payload-v1");
            assert!(!verify_hmac(secret, "payload-v2", &mac));
        }

        #[test]
        fn verify_hmac_invalid_hex_returns_false() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            assert!(!verify_hmac(secret, "msg", "not-hex!"));
        }

        #[test]
        fn verify_hmac_empty_hex_returns_false() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            assert!(!verify_hmac(secret, "msg", ""));
        }

        #[test]
        fn verify_hmac_truncated_hex_returns_false() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let full = compute_hmac(secret, "msg");
            // Truncate to half – still valid hex but wrong length
            assert!(!verify_hmac(secret, "msg", &full[..full.len() / 2]));
        }

        #[test]
        fn verify_hmac_empty_secret_and_message() {
            let secret = b"";
            let msg = "";
            let mac = compute_hmac(secret, msg);
            assert!(verify_hmac(secret, msg, &mac));
        }

        // ---------------------------------------------------------------
        //  verify_message_hmac tests
        // ---------------------------------------------------------------

        /// Build a relay_route JSON value with a valid HMAC.
        fn make_signed_route_message(
            secret: &[u8],
            to_device: &str,
            payload: &serde_json::Value,
            timestamp: i64,
            nonce: &str,
        ) -> serde_json::Value {
            // Replicate the canonical signing logic from verify_message_hmac:
            // Build an ordered map containing exactly the fields that are signed.
            let mut sign_map = serde_json::Map::new();
            sign_map.insert("type".into(), serde_json::json!("relay_route"));
            sign_map.insert("to_device_id".into(), serde_json::json!(to_device));
            sign_map.insert("payload".into(), payload.clone());
            sign_map.insert("timestamp".into(), serde_json::json!(timestamp));
            sign_map.insert("nonce".into(), serde_json::json!(nonce));
            let sign_str = serde_json::to_string(&Value::Object(sign_map)).unwrap();
            let hmac_hex = compute_hmac(secret, &sign_str);

            serde_json::json!({
                "type": "relay_route",
                "to_device_id": to_device,
                "payload": payload,
                "timestamp": timestamp,
                "nonce": nonce,
                "hmac": hmac_hex,
            })
        }

        #[test]
        fn verify_message_hmac_valid_signature_accepted() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let msg = make_signed_route_message(
                secret,
                "dev-001",
                &serde_json::json!({"data": "hello"}),
                1_700_000_000_000,
                "nonce-aaa",
            );
            assert!(verify_message_hmac(secret, &msg));
        }

        #[test]
        fn verify_message_hmac_missing_hmac_field_rejects() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let msg = serde_json::json!({
                "type": "relay_route",
                "to_device_id": "dev-001",
                "payload": {"data": "hello"},
                "timestamp": 1_700_000_000_000_i64,
                "nonce": "nonce-aaa",
            });
            assert!(!verify_message_hmac(secret, &msg));
        }

        #[test]
        fn verify_message_hmac_tampered_payload_rejects() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let mut msg = make_signed_route_message(
                secret,
                "dev-001",
                &serde_json::json!({"data": "hello"}),
                1_700_000_000_000,
                "nonce-bbb",
            );
            // Tamper with the payload after signing
            msg["payload"] = serde_json::json!({"data": "tampered"});
            assert!(!verify_message_hmac(secret, &msg));
        }

        #[test]
        fn verify_message_hmac_tampered_to_device_rejects() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let mut msg = make_signed_route_message(
                secret,
                "dev-001",
                &serde_json::json!({"x": 1}),
                1_700_000_000_000,
                "nonce-ccc",
            );
            msg["to_device_id"] = serde_json::json!("dev-999");
            assert!(!verify_message_hmac(secret, &msg));
        }

        #[test]
        fn verify_message_hmac_wrong_secret_rejects() {
            let secret = b"hmac-test-secret-key-32bytes!!";
            let wrong = b"wrong-key-aaaaaaaaaaaaaaaaaaaaaaa";
            let msg = make_signed_route_message(
                secret,
                "dev-001",
                &serde_json::json!("x"),
                1_700_000_000_000,
                "nonce-ddd",
            );
            assert!(!verify_message_hmac(wrong, &msg));
        }

        #[test]
        fn verify_message_hmac_non_relay_type_without_hmac_returns_false() {
            let secret = b"test";
            let msg = serde_json::json!({"type": "not_relay"});
            assert!(!verify_message_hmac(secret, &msg));
        }

        // ---------------------------------------------------------------
        //  Signed field list
        // ---------------------------------------------------------------

        /// Sign `fields` with the canonical scheme (this is what a client does).
        fn sign_fields(secret: &[u8], fields: &[(&str, serde_json::Value)]) -> Value {
            let mut json = serde_json::Map::new();
            for (k, v) in fields {
                json.insert((*k).to_string(), v.clone());
            }
            let value = Value::Object(json);
            let sign_str = canonical_signing_string(&value);
            let mut signed = value;
            signed.as_object_mut().unwrap().insert(
                "hmac".to_string(),
                serde_json::json!(compute_hmac(secret, &sign_str)),
            );
            signed
        }

        fn full_route_fields<'a>(
            from: &'a str,
            to: &'a str,
            nonce: &'a str,
        ) -> Vec<(&'a str, serde_json::Value)> {
            vec![
                ("type", serde_json::json!("relay_route")),
                ("from_device_id", serde_json::json!(from)),
                ("to_device_id", serde_json::json!(to)),
                ("payload", serde_json::json!({"type": "ping"})),
                ("timestamp", serde_json::json!(now_millis())),
                ("nonce", serde_json::json!(nonce)),
                ("key_id", serde_json::json!(from)),
            ]
        }

        #[test]
        fn signed_fields_list_covers_the_full_route_envelope() {
            // Every field the relay requires for a `relay_route` must be
            // authenticated; anything missing from this list is silently
            // unauthenticated, which is the bug this list exists to prevent.
            for required in [
                "type",
                "from_device_id",
                "to_device_id",
                "payload",
                "timestamp",
                "nonce",
                "key_id",
            ] {
                assert!(
                    SIGNED_FIELDS.contains(&required),
                    "{required} must be in SIGNED_FIELDS"
                );
            }
        }

        #[test]
        fn canonical_signing_string_covers_from_device_id() {
            let a = sign_fields(b"k", &full_route_fields("alice", "bob", "n-1"));
            let mut b = a.clone();
            b["from_device_id"] = serde_json::json!("mallory");
            assert_ne!(
                canonical_signing_string(&a),
                canonical_signing_string(&b),
                "from_device_id must change the canonical signing input"
            );
        }

        #[test]
        fn verify_message_hmac_covers_from_device_id() {
            let secret = b"route-signing-key-32-bytes-ok!!";
            let signed = sign_fields(secret, &full_route_fields("alice", "bob", "n-forgery"));
            assert!(verify_message_hmac(secret, &signed));

            // A client cannot re-point a validly signed route at itself/another
            // device by rewriting from_device_id.
            let mut forged = signed.clone();
            forged["from_device_id"] = serde_json::json!("alice-forged");
            assert!(
                !verify_message_hmac(secret, &forged),
                "rewriting from_device_id must invalidate the signature"
            );
        }

        #[test]
        fn verify_message_hmac_covers_key_id() {
            let secret = b"route-signing-key-32-bytes-ok!!";
            let signed = sign_fields(secret, &full_route_fields("alice", "bob", "n-keyid"));
            let mut forged = signed;
            forged["key_id"] = serde_json::json!("attacker-chosen");
            assert!(!verify_message_hmac(secret, &forged));
        }

        #[test]
        fn verify_message_hmac_legacy_message_without_new_fields_still_verifies() {
            // Backwards compatibility: a message that omits the newly-signed
            // fields still verifies against the same canonical rule (absent
            // fields are simply not part of the input).
            let secret = b"legacy-secret";
            let msg = make_signed_route_message(
                secret,
                "dev-001",
                &serde_json::json!({"data": "hello"}),
                1_700_000_000_000,
                "nonce-legacy",
            );
            assert!(verify_message_hmac(secret, &msg));
        }

        // ---------------------------------------------------------------
        //  derive_key (domain separation)
        // ---------------------------------------------------------------

        #[test]
        fn derive_key_is_deterministic_and_label_dependent() {
            let master = b"master-secret";
            let a = derive_key(master, ROUTE_KEY_LABEL);
            let b = derive_key(master, ROUTE_KEY_LABEL);
            assert_eq!(a, b, "derivation must be deterministic");
            assert_ne!(
                a,
                derive_key(master, "conduit-relay/v1/some-other-purpose"),
                "different labels must give different keys"
            );
        }

        #[test]
        #[should_panic(expected = "non-empty domain-separation label")]
        fn derive_key_rejects_empty_label() {
            let _ = derive_key(b"master", "");
        }

        #[test]
        fn a_derived_route_key_does_not_verify_mac_made_with_the_pairing_secret() {
            // The point of the derivation: holding the secret that a route key is
            // derived from is not the same as holding the route key, so a peer
            // that knows the secret cannot forge a MAC for a different purpose.
            let secret = b"pairing-secret";
            let route_key = derive_route_key(secret, "mallory");
            let forged = sign_fields(secret, &full_route_fields("mallory", "bob", "n-x"));
            assert!(
                !verify_message_hmac(&route_key, &forged),
                "a MAC under the pairing secret must not validate as a route key"
            );
            assert!(verify_message_hmac(
                &route_key,
                &sign_fields(&route_key, &full_route_fields("mallory", "bob", "n-x"))
            ));
        }

        // ---------------------------------------------------------------
        //  RouteKeyring (per-device route keys)
        // ---------------------------------------------------------------

        /// Sign a route envelope as `signer`, with `secret` as its route key.
        fn signed_as(signer: &str, secret: &[u8], to: &str, nonce: &str) -> Value {
            let fields: Vec<(&str, serde_json::Value)> = full_route_fields(signer, to, nonce)
                .into_iter()
                .map(|(k, v)| {
                    if k == "key_id" {
                        (k, serde_json::json!(signer))
                    } else {
                        (k, v)
                    }
                })
                .collect();
            sign_fields(secret, &fields)
        }

        fn ring_of(entries: &[(&str, &[u8])]) -> RouteKeyring {
            let mut ring = RouteKeyring::new();
            for (id, key) in entries {
                ring.insert(*id, key.to_vec());
            }
            ring
        }

        #[test]
        fn route_keyring_verifies_a_devices_own_route() {
            let ring = ring_of(&[("alice", b"alice-route-key")]);
            let msg = signed_as("alice", b"alice-route-key", "bob", "n1");
            assert_eq!(ring.verify(&msg).as_deref(), Some("alice"));
            assert_eq!(ring.device_ids(), vec!["alice"]);
        }

        #[test]
        fn route_keyring_rejects_a_key_that_belongs_to_another_device() {
            // Bob signs a route and attributes it to Alice. Alice's key is the
            // one the ring holds for that id, so the MAC cannot match.
            let ring = ring_of(&[("alice", b"alice-route-key")]);
            let forged = signed_as("alice", b"bob-route-key", "carol", "n1");
            assert!(
                ring.verify(&forged).is_none(),
                "a device must not be able to sign as another device"
            );
        }

        #[test]
        fn route_keyring_rejects_an_unregistered_device() {
            let ring = ring_of(&[("alice", b"alice-route-key")]);
            let stranger = signed_as("mallory", b"mallory-route-key", "bob", "n1");
            assert!(ring.verify(&stranger).is_none());
        }

        #[test]
        fn route_keyring_rejects_a_missing_key_id() {
            // key_id is what the key is looked up by. Without it there is no key
            // to check against, and defaulting it would attribute the route to
            // whichever device happened to be registered.
            let ring = ring_of(&[("alice", b"alice-route-key")]);
            let mut msg = signed_as("alice", b"alice-route-key", "bob", "n1");
            msg.as_object_mut().unwrap().remove("key_id");
            assert!(ring.verify(&msg).is_none());
        }

        #[test]
        fn route_keyring_rejects_a_key_id_that_disagrees_with_the_sender() {
            let ring = ring_of(&[("alice", b"alice-route-key"), ("bob", b"bob-route-key")]);
            let mut msg = signed_as("alice", b"alice-route-key", "carol", "n1");
            // Re-label the route to Bob. `from_device_id` is signed, so this
            // cannot verify as Bob.
            msg["from_device_id"] = serde_json::json!("bob");
            assert!(
                ring.verify(&msg).is_none(),
                "a valid signature must not be re-attributed to another device"
            );
        }

        #[test]
        fn route_keyring_forgets_an_unpaired_device() {
            let mut ring = ring_of(&[("alice", b"alice-route-key"), ("bob", b"bob-route-key")]);
            ring.remove("bob");
            let revoked = signed_as("bob", b"bob-route-key", "alice", "n1");
            assert!(
                ring.verify(&revoked).is_none(),
                "an unpaired device must stop being able to sign"
            );
            assert_eq!(ring.device_ids(), vec!["alice"]);
        }

        #[test]
        fn route_keyring_re_registering_replaces_the_key() {
            // A re-pair changes the device's secret, and the old key must not
            // keep working afterwards.
            let mut ring = ring_of(&[("alice", b"old-route-key")]);
            ring.insert("alice", b"new-route-key".to_vec());
            let stale = signed_as("alice", b"old-route-key", "bob", "n1");
            assert!(ring.verify(&stale).is_none());
            let fresh = signed_as("alice", b"new-route-key", "bob", "n2");
            assert_eq!(ring.verify(&fresh).as_deref(), Some("alice"));
        }

        #[test]
        fn route_keyring_retain_drops_unlisted_devices() {
            let mut ring = ring_of(&[("alice", b"alice-route-key"), ("bob", b"bob-route-key")]);
            ring.retain(|id| id == "alice");
            assert_eq!(ring.len(), 1);
            assert!(!ring.is_empty());
            assert!(ring
                .verify(&signed_as("bob", b"bob-route-key", "alice", "n1"))
                .is_none());
        }

        #[test]
        fn an_empty_route_keyring_verifies_nothing() {
            let ring = RouteKeyring::new();
            assert!(ring.is_empty());
            assert!(ring
                .verify(&signed_as("alice", b"a", "bob", "n1"))
                .is_none());
        }

        // ---------------------------------------------------------------
        //  derive_route_key
        // ---------------------------------------------------------------

        #[test]
        fn derive_route_key_is_deterministic_per_device() {
            let secret = b"pairing-secret";
            assert_eq!(
                derive_route_key(secret, "alice"),
                derive_route_key(secret, "alice"),
                "both ends must derive the same key"
            );
        }

        #[test]
        fn derive_route_key_separates_by_device_id() {
            let secret = b"pairing-secret";
            assert_ne!(
                derive_route_key(secret, "alice"),
                derive_route_key(secret, "bob"),
                "the device id is part of the derivation, so one secret under two \
                 ids must not yield one key"
            );
        }

        #[test]
        fn derive_route_key_never_returns_the_pairing_secret() {
            let secret = b"a-32-byte-pairing-secret-value!!";
            let derived = derive_route_key(secret, "alice");
            assert_ne!(&derived[..], &secret[..]);
            assert_ne!(hex::encode(derived), hex::encode(secret));
        }

        #[test]
        fn derive_route_key_differs_from_other_purposes() {
            let secret = b"a-32-byte-pairing-secret-value!!";
            assert_ne!(
                derive_route_key(secret, "alice"),
                derive_key(secret, ROUTE_KEY_LABEL),
                "the device id must be bound into the label"
            );
        }

        // ---------------------------------------------------------------
        //  NonceCache (per-device scoped replay protection)
        // ---------------------------------------------------------------

        #[test]
        fn nonce_cache_accepts_then_rejects_same_nonce_for_same_device() {
            let mut cache = NonceCache::new();
            let now = now_millis();
            assert!(cache.check_replay("alice", now, "n1"));
            assert!(!cache.check_replay("alice", now, "n1"));
            assert_eq!(cache.len_for("alice"), 1);
        }

        #[test]
        fn nonce_cache_scopes_nonces_per_device() {
            // The same nonce string from two different devices is two distinct
            // nonces — a client must not be able to burn another client's
            // nonces by guessing/colliding them.
            let mut cache = NonceCache::new();
            let now = now_millis();
            assert!(cache.check_replay("alice", now, "shared"));
            assert!(cache.check_replay("bob", now, "shared"));
            assert!(!cache.check_replay("alice", now, "shared"));
            assert!(!cache.check_replay("bob", now, "shared"));
        }

        #[test]
        fn nonce_cache_rejects_empty_nonce() {
            let mut cache = NonceCache::new();
            assert!(!cache.check_replay("alice", now_millis(), ""));
            assert_eq!(cache.total_len(), 0);
        }

        #[test]
        fn nonce_cache_enforces_timestamp_window() {
            let mut cache = NonceCache::new();
            let now = now_millis();
            assert!(!cache.check_replay("alice", now - 31_000, "too-old"));
            assert!(!cache.check_replay("alice", now + 6_000, "too-far-future"));
            assert!(cache.check_replay("alice", now - 29_000, "in-window"));
        }

        #[test]
        fn nonce_cache_never_clears_another_devices_nonces() {
            // VULNERABILITY 6: one client flooding nonces must not disable
            // replay protection for everybody else.
            let now = now_millis();
            let mut cache = NonceCache::new();
            assert!(cache.check_replay("victim", now, "victim-nonce-0"));

            // "flooder" exhausts its own per-device quota, several times over.
            for i in 0..(MAX_NONCES_PER_DEVICE * 3) {
                let nonce = format!("flood-{i}");
                assert!(
                    cache.check_replay("flooder", now, &nonce),
                    "the flooder's own fresh nonces must still be accepted"
                );
                if i < MAX_NONCES_PER_DEVICE {
                    assert!(
                        !cache.check_replay("flooder", now, &nonce),
                        "the flooder's own replayed nonces must be rejected \
                         while they are still inside its quota"
                    );
                }
            }

            assert!(
                cache.contains("victim", "victim-nonce-0"),
                "the victim's remembered nonce must survive another client's flood"
            );
            assert!(
                !cache.check_replay("victim", now, "victim-nonce-0"),
                "the victim must still be replay-protected"
            );
            assert!(
                cache.total_len() < MAX_NONCES,
                "a single device must never be able to fill the whole cache"
            );
        }

        #[test]
        fn nonce_cache_per_device_quota_is_bounded() {
            let now = now_millis();
            let mut cache = NonceCache::new();
            for i in 0..(MAX_NONCES_PER_DEVICE + 500) {
                cache.check_replay("greedy", now, &format!("g{i}"));
            }
            assert_eq!(
                cache.len_for("greedy"),
                MAX_NONCES_PER_DEVICE,
                "per-device storage must stay bounded"
            );
        }

        #[test]
        fn nonce_cache_global_ceiling_is_respected() {
            let now = now_millis();
            let mut cache = NonceCache::new();
            // Enough devices to blow past the process-wide ceiling.
            for d in 0..4 {
                for i in 0..(MAX_NONCES_PER_DEVICE + 100) {
                    cache.check_replay(&format!("dev-{d}"), now, &format!("n{d}-{i}"));
                }
            }
            assert!(
                cache.total_len() <= MAX_NONCES,
                "global cap {} must hold, got {}",
                MAX_NONCES,
                cache.total_len()
            );
        }

        #[test]
        fn nonce_cache_map_roundtrip_preserves_scoping() {
            let now = now_millis();
            let mut cache = NonceCache::new();
            assert!(cache.check_replay("alice", now, "a-1"));
            assert!(cache.check_replay("bob", now, "b-1"));

            let flat = cache.to_map();
            assert_eq!(flat.len(), 2);
            assert!(flat.contains_key(&format!("alice{SCOPE_SEPARATOR}a-1")));
            assert!(flat.contains_key(&format!("bob{SCOPE_SEPARATOR}b-1")));

            let mut restored = NonceCache::from_map(&flat);
            assert!(restored.contains("alice", "a-1"));
            assert!(restored.contains("bob", "b-1"));
            assert!(
                !restored.check_replay("alice", now, "a-1"),
                "restored nonce must still be replay-protected"
            );
        }

        #[test]
        fn nonce_cache_prune_before_drops_stale_entries() {
            let mut cache = NonceCache::new();
            // Insert oldest-first (the invariant `from_map` also establishes).
            cache.insert_raw("alice", "stale".to_string(), now_millis() - 120_000);
            cache.insert_raw("alice", "fresh".to_string(), now_millis());
            cache.prune_before(now_millis() - 60_000);
            assert!(cache.contains("alice", "fresh"));
            assert!(!cache.contains("alice", "stale"));
            assert_eq!(cache.len_for("alice"), 1);
        }

        #[test]
        fn nonce_cache_from_map_keeps_unscoped_legacy_entries() {
            // Pre-scoping nonces.json files have bare nonce keys; they must not
            // be silently dropped (dropping would re-enable replay).
            let now = now_millis();
            let mut flat = std::collections::HashMap::new();
            flat.insert("legacy-nonce".to_string(), now);
            let cache = NonceCache::from_map(&flat);
            assert_eq!(cache.total_len(), 1);
            assert!(cache
                .to_map()
                .contains_key(&format!("legacy-unscoped{SCOPE_SEPARATOR}legacy-nonce")));
        }

        #[test]
        fn new_nonce_is_unique_across_calls() {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..1000 {
                assert!(seen.insert(new_nonce()), "nonces must be unique");
            }
            assert_eq!(
                new_nonce().len(),
                64,
                "nonce should be 32 hex-encoded bytes"
            );
        }

        // ---------------------------------------------------------------
        //  check_replay tests
        // ---------------------------------------------------------------

        #[test]
        fn check_replay_accepts_fresh_nonce() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            assert!(check_replay(now, "n1", &mut nonces));
            assert_eq!(nonces.len(), 1);
        }

        #[test]
        fn check_replay_rejects_duplicate_nonce() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            assert!(check_replay(now, "n1", &mut nonces));
            assert!(!check_replay(now, "n1", &mut nonces));
        }

        #[test]
        fn check_replay_rejects_expired_timestamp() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            // 31 seconds in the past → age > 30_000
            assert!(!check_replay(now - 31_000, "old-nonce", &mut nonces));
        }

        #[test]
        fn check_replay_rejects_timestamp_too_far_future() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            // 6 seconds in the future → age < -5_000
            assert!(!check_replay(now + 6_000, "future-nonce", &mut nonces));
        }

        #[test]
        fn check_replay_accepts_within_clock_skew_window() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            // 4 seconds in the future (within -5s tolerance)
            assert!(check_replay(now + 4_000, "skew-ok", &mut nonces));
        }

        #[test]
        fn check_replay_accepts_timestamp_29s_old() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            // 29 seconds old – within 30s window
            assert!(check_replay(now - 29_000, "n29", &mut nonces));
        }

        #[test]
        fn check_replay_accepts_timestamp_near_window_edge() {
            let mut nonces = std::collections::HashMap::new();
            // check_replay re-reads the clock internally, so an exact 30_000 age is
            // racy (1ms drift → rejected). 29_000 is deterministically inside the
            // 30s window; the exact-30_000 edge is covered by the reject tests.
            let ts = now_millis() - 29_000;
            assert!(check_replay(ts, "n-near-edge", &mut nonces));
        }

        #[test]
        fn check_replay_nonce_dedup_evicts_old_entries_at_cap() {
            // Rewritten from the version that documented a full `clear()` at the
            // cap. A wholesale clear let any client that could reach MAX_NONCES
            // re-enable replay for every nonce ever accepted; the cache must now
            // evict oldest-first and stay exactly at the cap.
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();

            // Fill to MAX_NONCES with *distinct* timestamps so "oldest" is
            // deterministic: n0 is newest, n{MAX-1} is oldest.
            for i in 0..MAX_NONCES {
                nonces.insert(format!("n{i}"), now - i as i64);
            }
            assert_eq!(nonces.len(), MAX_NONCES);

            // A fresh nonce at the cap must be accepted, and must displace
            // exactly one entry — not the whole cache.
            assert!(check_replay(now, "overflow-nonce", &mut nonces));
            assert_eq!(
                nonces.len(),
                MAX_NONCES,
                "eviction must keep the cache at the cap, never shrink it to 1"
            );
            assert!(nonces.contains_key("overflow-nonce"));
            assert!(
                !nonces.contains_key(&format!("n{}", MAX_NONCES - 1)),
                "the OLDEST entry must be the one evicted"
            );

            // The newest previously-accepted nonce is still remembered.
            assert!(
                !check_replay(now, "n0", &mut nonces),
                "recently accepted nonces must stay protected at the cap"
            );
            // The evicted oldest nonce is no longer remembered (accepted again).
            assert!(
                check_replay(now, &format!("n{}", MAX_NONCES - 1), &mut nonces),
                "the evicted oldest nonce is no longer replay-protected"
            );
        }

        #[test]
        fn check_replay_evicts_stale_entries_then_keeps_fresh() {
            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();

            // Insert MAX_NONCES entries with timestamps 70s ago (stale)
            let stale_ts = now - 70_000;
            for i in 0..MAX_NONCES {
                let nonce = format!("stale-{}", i);
                // These pass the initial timestamp check because check_replay
                // is called with stale_ts, and age = now - stale_ts = 70000 > 30000,
                // so they would be rejected. We need to insert them manually
                // to set up the scenario.
                nonces.insert(nonce, stale_ts);
            }
            assert_eq!(nonces.len(), MAX_NONCES);

            // Now a fresh nonce comes in. The stale entries (>60s old) should be evicted.
            assert!(check_replay(now, "fresh-nonce", &mut nonces));
            assert_eq!(nonces.len(), 1);
            assert!(nonces.contains_key("fresh-nonce"));
        }

        // ---------------------------------------------------------------
        //  now_millis basic sanity
        // ---------------------------------------------------------------

        #[test]
        fn now_millis_returns_reasonable_value() {
            let ts = now_millis();
            // Should be a 13-digit timestamp (trillions of milliseconds since epoch)
            assert!(ts > 1_000_000_000_000); // After Sept 2001
            assert!(ts < 2_000_000_000_000); // Before Nov 2033
        }

        // ---------------------------------------------------------------
        //  Nonce persistence (load_nonces / save_nonces)
        // ---------------------------------------------------------------

        #[test]
        fn save_and_load_nonces_roundtrip_prunes_stale() {
            let path = std::env::temp_dir()
                .join(format!("relay-nonce-roundtrip-{}.json", std::process::id()));
            let _ = std::fs::remove_file(&path);

            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            nonces.insert("fresh-nonce".to_string(), now);
            nonces.insert("stale-nonce".to_string(), now - 120_000); // >60s → pruned

            save_nonces(&path, &nonces).expect("save_nonces should succeed");

            let loaded = load_nonces(&path);
            assert!(
                loaded.contains_key("fresh-nonce"),
                "fresh nonce must survive the roundtrip"
            );
            assert!(
                !loaded.contains_key("stale-nonce"),
                "stale nonce (>60s) must be pruned on save/load"
            );

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn load_nonces_missing_file_returns_empty() {
            let path = std::env::temp_dir()
                .join(format!("relay-nonce-missing-{}.json", std::process::id()));
            let _ = std::fs::remove_file(&path);

            let loaded = load_nonces(&path);
            assert!(loaded.is_empty(), "missing file should yield empty cache");
        }

        #[test]
        fn load_nonces_corrupt_file_returns_empty() {
            let path = std::env::temp_dir()
                .join(format!("relay-nonce-corrupt-{}.json", std::process::id()));
            std::fs::write(&path, "not valid json {{{").expect("write corrupt file");

            let loaded = load_nonces(&path);
            assert!(loaded.is_empty(), "corrupt file should yield empty cache");

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn save_nonces_creates_parent_directory() {
            let dir = std::env::temp_dir().join(format!("relay-nonce-dir-{}", std::process::id()));
            let path = dir.join("nested").join("nonces.json");
            let _ = std::fs::remove_dir_all(&dir);

            let mut nonces = std::collections::HashMap::new();
            nonces.insert("n1".to_string(), now_millis());
            save_nonces(&path, &nonces).expect("should create parent dirs");

            assert!(path.exists(), "file should exist after save");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn check_replay_rejects_nonce_restored_from_persisted_cache() {
            // A device disconnects, its nonces are persisted, then a reconnect
            // (or restart) loads them — a previously-accepted nonce must still
            // be rejected. This is the cross-restart replay protection guarantee.
            let path = std::env::temp_dir()
                .join(format!("relay-nonce-restored-{}.json", std::process::id()));
            let _ = std::fs::remove_file(&path);

            let now = now_millis();
            let mut nonces = std::collections::HashMap::new();
            assert!(check_replay(now, "persisted-nonce", &mut nonces));
            save_nonces(&path, &nonces).expect("save should succeed");

            // Simulate restart: load into a fresh cache.
            let mut restored = load_nonces(&path);
            assert!(
                restored.contains_key("persisted-nonce"),
                "fresh nonce must survive the persistence roundtrip"
            );
            assert!(
                !check_replay(now, "persisted-nonce", &mut restored),
                "nonce restored from disk must still be rejected as a replay"
            );

            let _ = std::fs::remove_file(&path);
        }
    }
}
