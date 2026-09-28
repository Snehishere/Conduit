use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use rand::Rng;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::ConduitError;

// ─── Long-lived secret storage ────────────────────────────────────────────────
//
// Two secrets must survive a restart: the X25519 identity key
// (`x25519_private_key`) and the SQLCipher database key (`sqlite_key`). The OS
// keyring is the only acceptable home for either, but the keyring is not
// always there:
//
//   * `keyring = "4.2"` with default features resolves `v1` to
//     apple-keychain + windows-CredMan + zbus-secret-service. On any Linux box
//     with no running Secret Service — headless servers, minimal containers,
//     most CI images, and this repo's own `docker-compose.yml` — the
//     `zbus-secret-service` backend returns `PlatformFailure` for *every*
//     operation, including writes.
//   * A keychain that is locked (screen lock, MDM) rejects reads and writes.
//
// The failure mode that made this a data-loss bug is the combination with
// `Storage::new`: a key that was generated but *not persisted* no longer
// matches `PRAGMA key`, so the next launch cannot decrypt the database — and a
// database that cannot be decrypted must never be overwritten (see
// `storage::DbFileKind`).
//
// The rule enforced by [`load_or_create_secret`], therefore, is:
//
//   A secret is never returned to the caller until it has been read back from
//   a durable store. If the keyring cannot be written or the write cannot be
//   verified, the secret goes to a `0600` key file under the app data
//   directory instead, and only a failure to write *that* is fatal.
//
// REDUCED PROTECTION. The key file is a plaintext file in the user's own
// profile directory, gated on POSIX by mode `0600` (and on Windows by the
// per-user `%LOCALAPPDATA%` ACL). That is materially weaker than a keychain:
//   * it is readable by anything running as the same user, including malware
//     and any process that can read the user's home directory;
//   * on Windows there is no per-file ACL, so anything that can read the
//     user's profile can read the key;
//   * it is not covered by OS-level keychain access prompts or auditing.
// This is a deliberate availability-over-confidentiality trade: a hub that
// cannot reach its keyring should still be able to reach its own database. The
// trade is logged at `warn` on every use, naming the reduced protection, and it
// self-heals — see `migrate_key_file_into_keyring`.

/// Keyring service name shared by every Conduit secret.
pub const KEYRING_SERVICE: &str = "conduit_app";
/// Account name of the long-lived X25519 identity secret.
pub const X25519_SECRET_ACCOUNT: &str = "x25519_private_key";
/// Account name of the SQLCipher database key.
pub const SQLITE_KEY_ACCOUNT: &str = "sqlite_key";

/// Directory Conduit keeps its own persistent state in.
///
/// Single definition, shared with [`crate::storage::Storage`], so the database
/// and the key that decrypts it can never end up under two different roots.
pub fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("conduit")
}

/// `0600` on POSIX. On Windows there is no POSIX mode: the key file lives in
/// the *per-user* LocalAppData tree, whose ACL already grants full control only
/// to that user (plus SYSTEM/Administrators). Tightening further would need an
/// explicit per-file DACL, which is deliberately out of scope here — the
/// fallback is documented as reduced protection instead.
#[cfg(unix)]
const KEY_FILE_MODE: u32 = 0o600;

/// The operations [`resolve_secret`] needs from a key store.
///
/// This exists so the "keyring is unavailable" paths can be exercised by tests
/// and fakes. The previous code hard-coded `keyring::Entry` and discarded the
/// result of `set_password`, which is why the data-loss path was untestable
/// *and* unreachable from any test.
pub(crate) trait KeyStore {
    /// Human-readable store name for log lines.
    fn name(&self) -> &'static str;

    /// `Ok(None)` when the store holds nothing for this account; `Err(msg)`
    /// when the store itself is unusable (no Secret Service, locked keychain,
    /// read-only profile). The two must not be conflated.
    fn load(&self) -> Result<Option<String>, String>;

    fn store(&self, secret: &str) -> Result<(), String>;

    /// Read back whatever [`KeyStore::store`] wrote, discarding errors.
    /// `None` means "could not confirm", which is treated as failure.
    fn confirm(&self) -> Option<String>;

    /// Forget the secret. Only ever called after `store` + `confirm` succeeded
    /// in a *different* store.
    fn clear(&self) -> Result<(), String>;
}

/// The real OS keyring (macOS Keychain / Windows Credential Manager /
/// freedesktop Secret Service, per `keyring`'s platform dispatch), bound to one
/// account name.
pub(crate) struct KeyringStore {
    account: String,
}

impl KeyringStore {
    pub(crate) fn for_account(account: &str) -> Self {
        KeyringStore {
            account: account.to_string(),
        }
    }
}

impl KeyStore for KeyringStore {
    fn name(&self) -> &'static str {
        "OS keyring"
    }

    fn load(&self) -> Result<Option<String>, String> {
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, &self.account).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn store(&self, secret: &str) -> Result<(), String> {
        keyring::Entry::new(KEYRING_SERVICE, &self.account)
            .map_err(|e| e.to_string())?
            .set_password(secret)
            .map_err(|e| e.to_string())
    }

    fn confirm(&self) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, &self.account)
            .ok()?
            .get_password()
            .ok()
    }

    fn clear(&self) -> Result<(), String> {
        match keyring::Entry::new(KEYRING_SERVICE, &self.account)
            .map_err(|e| e.to_string())?
            .delete_credential()
        {
            Ok(()) => Ok(()),
            // Already gone is success.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A `0600` key file under the app data directory.
pub(crate) struct KeyFileStore {
    path: PathBuf,
}

impl KeyFileStore {
    pub(crate) fn for_account(account: &str) -> Self {
        KeyFileStore {
            path: app_data_dir().join("keys").join(format!("{account}.key")),
        }
    }

    /// Path of the key file for `account`. Public so storage can name it in an
    /// error message the user can act on.
    pub(crate) fn path_for_account(account: &str) -> PathBuf {
        Self::for_account(account).path
    }
}

impl KeyStore for KeyFileStore {
    fn name(&self) -> &'static str {
        "0600 key file"
    }

    fn load(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => {
                let secret = raw.trim().to_string();
                if secret.is_empty() {
                    // An empty key file is worse than none: it would be used as
                    // the key and silently make the database unreadable.
                    Err(format!(
                        "key file {} exists but is empty",
                        self.path.display()
                    ))
                } else {
                    Ok(Some(secret))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", self.path.display())),
        }
    }

    fn store(&self, secret: &str) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            restrict_permissions(parent, true)?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(KEY_FILE_MODE);
        }
        let mut file = opts
            .open(&self.path)
            .map_err(|e| format!("cannot create {}: {e}", self.path.display()))?;
        use std::io::Write;
        file.write_all(secret.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", self.path.display()))?;
        // Durability matters more than speed here: a key that is generated but
        // lost on power failure is indistinguishable from a rotated key, and the
        // database it protects becomes unreadable.
        file.sync_all()
            .map_err(|e| format!("cannot flush {}: {e}", self.path.display()))?;
        // `create(true)` does not tighten the mode of a file that already
        // exists, so enforce it unconditionally.
        restrict_permissions(&self.path, false)?;
        Ok(())
    }

    fn confirm(&self) -> Option<String> {
        self.load().ok().flatten()
    }

    fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove {}: {e}", self.path.display())),
        }
    }
}

/// Force `0600` on a file or `0700` on a directory (POSIX); no-op elsewhere.
fn restrict_permissions(path: &Path, is_dir: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if is_dir { 0o700 } else { KEY_FILE_MODE };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| format!("cannot restrict permissions on {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, is_dir);
    }
    Ok(())
}

/// Resolve a long-lived secret for `account`, persisting it before returning.
///
/// See the module-level comment for the data-loss bug this replaces and for the
/// reduced protection the key-file fallback implies.
pub fn load_or_create_secret(account: &str) -> Result<String, ConduitError> {
    resolve_secret(
        &KeyringStore::for_account(account),
        &KeyFileStore::for_account(account),
        account,
    )
}

/// The decision logic, parameterised over the two stores so it is testable.
///
/// Order of operations, and why:
///
/// 1. **The key file wins, if it exists.** It is the only proof that a secret
///    was persisted while the keyring was broken. Reading it first is what
///    makes the fallback *idempotent*: if the keyring later comes back, the
///    same key is still used, so `PRAGMA key` keeps matching. (A fresh
///    keyring read on a machine that had previously fallen back would mint a
///    second key and orphan the first one — the original bug.)
/// 2. **Otherwise ask the keyring.** An existing entry is used as-is.
/// 3. **Otherwise mint one, and do not return it until it is durable.** The
///    keyring write is confirmed by reading it back. If the write fails or
///    cannot be confirmed, the secret is written to the key file instead.
/// 4. **Only a failure of *both* stores is fatal**, and the error says why,
///    because that is the one case where continuing would destroy data.
pub(crate) fn resolve_secret(
    keyring_store: &dyn KeyStore,
    key_file_store: &dyn KeyStore,
    account: &str,
) -> Result<String, ConduitError> {
    // 1. A key file from a previous degraded run is authoritative.
    match key_file_store.load() {
        Ok(Some(secret)) => {
            if secret.is_empty() {
                return Err(empty_secret_error(key_file_store.name(), account));
            }
            migrate_key_file_into_keyring(keyring_store, key_file_store, &secret);
            return Ok(secret);
        }
        Ok(None) => {}
        Err(msg) => {
            // The key file is unreadable *and* we may have to fall back to it,
            // so we cannot treat the keyring as the only store any more.
            return Err(ConduitError::Encryption(format!(
                "the {} for `{account}` exists but is unusable ({msg}). \
                 Refusing to generate a new key: doing so would make the existing \
                 database and pairing identities permanently unreadable. \
                 Repair or delete the file and restart.",
                key_file_store.name()
            )));
        }
    }

    // 2/3. The keyring, then a freshly generated secret persisted durably.
    let existing = match keyring_store.load() {
        Ok(existing) => existing,
        Err(msg) => {
            log::warn!(
                "The {} is unavailable ({msg}); using a 0600 key file instead. \
                 REDUCED PROTECTION: that file is readable by anything running as this user \
                 and is not protected by the OS keychain.",
                keyring_store.name()
            );
            return create_in_key_file(key_file_store, account);
        }
    };
    if let Some(secret) = existing {
        // An empty entry is worse than no entry: it would be used as the key
        // and silently make the database unreadable.
        if secret.is_empty() {
            return Err(empty_secret_error(keyring_store.name(), account));
        }
        return Ok(secret);
    }

    let new_secret = generate_token_hex();
    match keyring_store.store(&new_secret) {
        Ok(()) => match keyring_store.confirm() {
            Some(back) if back == new_secret => return Ok(new_secret),
            // Either a concurrent writer won the race or the platform lied.
            // Never return a secret we could not read back.
            other => {
                log::warn!(
                    "The {} accepted a write but reading it back returned {:?}. \
                     Treating the keyring as unreliable and persisting to a 0600 key file.",
                    keyring_store.name(),
                    other.as_deref().map(|s| s.len())
                );
            }
        },
        Err(msg) => {
            log::warn!(
                "Failed to write to the {}: {msg}. Falling back to a 0600 key file. \
                 REDUCED PROTECTION: that file is readable by anything running as this user \
                 and is not protected by the OS keychain.",
                keyring_store.name()
            );
        }
    }
    create_in_key_file(key_file_store, account)
}

/// An empty secret is never usable — it would be handed to `PRAGMA key` as if
/// it were the real key and quietly render the database unreadable.
fn empty_secret_error(store_name: &str, account: &str) -> ConduitError {
    ConduitError::Encryption(format!(
        "The {store_name} holds an EMPTY entry for `{account}`. Refusing to use or replace it: \
         an empty key would make the existing database permanently unreadable. Delete the \
         entry and restart, accepting that the data encrypted with it cannot be recovered."
    ))
}

/// Mint a secret and persist it to the key file, or fail.
fn create_in_key_file(
    key_file_store: &dyn KeyStore,
    account: &str,
) -> Result<String, ConduitError> {
    // Another process may have created the file between our `load` and here.
    if let Ok(Some(existing)) = key_file_store.load() {
        return Ok(existing);
    }
    let secret = generate_token_hex();
    key_file_store.store(&secret).map_err(|e| {
        ConduitError::Encryption(format!(
            "Neither the OS keyring nor the 0600 key file could store the `{account}` secret \
             ({e}). Refusing to continue: a secret that cannot be persisted produces a \
             different key on every launch, which makes the existing database permanently \
             unreadable and used to destroy it."
        ))
    })?;
    log::warn!(
        "Stored the `{account}` secret in a 0600 key file because the OS keyring is \
         unavailable. REDUCED PROTECTION: the key now sits in a file under the app data \
         directory instead of the OS keychain, so it is readable by any other process \
         running as this user, is not covered by keychain access prompts or auditing, and \
         on Windows is protected only by the per-user directory ACL. It is moved back into \
         the keyring automatically once the keyring is reachable again."
    );
    Ok(secret)
}

/// Move a fallback key file back into the keyring now that the keyring works.
///
/// Only the key file is deleted, and only after the keyring has been written
/// *and read back*. If any step fails the file is kept: an extra plaintext key
/// on disk is a lesser evil than a rotated key.
fn migrate_key_file_into_keyring(
    keyring_store: &dyn KeyStore,
    key_file_store: &dyn KeyStore,
    secret: &str,
) {
    match keyring_store.load() {
        Ok(Some(existing)) if existing == secret => {
            // Already in both places. Drop the redundant file, keeping the
            // exposure window as short as possible.
            if let Err(e) = key_file_store.clear() {
                log::warn!(
                    "Could not remove the redundant {}: {e}",
                    key_file_store.name()
                );
            }
            return;
        }
        Ok(Some(_other)) => {
            log::error!(
                "The {} and the {} hold DIFFERENT secrets. Keeping the key file and trusting \
                 it, because regenerating the key would orphan the existing database and every \
                 paired device. Investigate before deleting either copy.",
                keyring_store.name(),
                key_file_store.name()
            );
            return;
        }
        Ok(None) => {}
        Err(msg) => {
            log::warn!(
                "The {} is still unavailable ({msg}); continuing with the {}.",
                keyring_store.name(),
                key_file_store.name()
            );
            return;
        }
    }

    if let Err(e) = keyring_store.store(secret) {
        log::warn!(
            "Could not move the secret into the {}: {e}. Continuing with the {} \
             (REDUCED PROTECTION: see the module comment).",
            keyring_store.name(),
            key_file_store.name()
        );
        return;
    }
    match keyring_store.confirm() {
        Some(back) if back == secret => {
            if let Err(e) = key_file_store.clear() {
                log::warn!(
                    "Secret is safely in the {} but the redundant {} could not be removed: {e}",
                    keyring_store.name(),
                    key_file_store.name()
                );
            } else {
                log::info!(
                    "Secret migrated from the {} back into the {}. Keychain protection restored.",
                    key_file_store.name(),
                    keyring_store.name()
                );
            }
        }
        _ => log::warn!(
            "The {} did not confirm the migration; keeping the {}. REDUCED PROTECTION continues.",
            keyring_store.name(),
            key_file_store.name()
        ),
    }
}

#[derive(Clone)]
pub struct EncryptionManager {
    private_key: Arc<StaticSecret>,
    public_key: Arc<PublicKey>,
}

impl EncryptionManager {
    pub fn new() -> Result<Self, ConduitError> {
        let hex_key = load_or_create_secret(X25519_SECRET_ACCOUNT)?;

        let bytes = hex::decode(hex_key.trim())
            .map_err(|e| ConduitError::Encryption(format!("Invalid hex in keyring: {}", e)))?;
        if bytes.len() != 32 {
            return Err(ConduitError::Encryption(
                "Invalid key length in keyring".into(),
            ));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        let private_key = StaticSecret::from(key_bytes);

        let public_key = PublicKey::from(&private_key);

        Ok(EncryptionManager {
            private_key: Arc::new(private_key),
            public_key: Arc::new(public_key),
        })
    }

    pub fn new_random() -> Self {
        let mut rng = rand::rng();
        let private_key = StaticSecret::random_from_rng(&mut rng);
        let public_key = PublicKey::from(&private_key);
        EncryptionManager {
            private_key: Arc::new(private_key),
            public_key: Arc::new(public_key),
        }
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key.as_bytes())
    }

    pub fn derive_shared_secret(
        &self,
        peer_public_key_hex: &str,
    ) -> Result<[u8; 32], ConduitError> {
        let peer_bytes = hex::decode(peer_public_key_hex)
            .map_err(|e| ConduitError::Encryption(format!("Invalid public key hex: {}", e)))?;
        if peer_bytes.len() != 32 {
            return Err(ConduitError::Encryption(format!(
                "Invalid public key length: expected 32 bytes, got {}",
                peer_bytes.len()
            )));
        }
        let mut peer_key_bytes = [0u8; 32];
        peer_key_bytes.copy_from_slice(&peer_bytes);

        let peer_public = PublicKey::from(peer_key_bytes);
        let shared_secret = self.private_key.diffie_hellman(&peer_public);
        Ok(*shared_secret.as_bytes())
    }

    pub fn encrypt(
        &self,
        shared_secret_hex: &str,
        plaintext: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), ConduitError> {
        let shared_secret = Self::parse_secret(shared_secret_hex)?;
        let cipher = XChaCha20Poly1305::new((&shared_secret).into());

        let mut nonce_bytes = [0u8; 24];
        let mut rng = rand::rng();
        rng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::try_from(nonce_bytes.as_slice()).unwrap();

        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| ConduitError::Encryption(format!("Encryption failed: {}", e)))?;

        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    pub fn decrypt(
        &self,
        shared_secret_hex: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<String, ConduitError> {
        let plaintext = self.decrypt_binary(shared_secret_hex, nonce, ciphertext)?;
        String::from_utf8(plaintext)
            .map_err(|e| ConduitError::Encryption(format!("Invalid UTF-8: {}", e)))
    }

    pub fn encrypt_binary(
        &self,
        shared_secret_hex: &str,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), ConduitError> {
        let shared_secret = Self::parse_secret(shared_secret_hex)?;
        let cipher = XChaCha20Poly1305::new((&shared_secret).into());

        let mut nonce_bytes = [0u8; 24];
        let mut rng = rand::rng();
        rng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::try_from(nonce_bytes.as_slice()).unwrap();

        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| ConduitError::Encryption(format!("Encryption failed: {}", e)))?;

        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    pub fn decrypt_binary(
        &self,
        shared_secret_hex: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, ConduitError> {
        let shared_secret = Self::parse_secret(shared_secret_hex)?;
        let cipher = XChaCha20Poly1305::new((&shared_secret).into());
        // Nonce length is attacker-controllable on the network path
        // (server/mod.rs feeds hex-decoded nonce straight through here).
        // Never panic on bad input — reject with a typed error.
        let nonce: &XNonce = nonce.try_into().map_err(|_| {
            ConduitError::Encryption(format!(
                "Invalid nonce length: expected 24 bytes, got {}",
                nonce.len()
            ))
        })?;

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| ConduitError::Encryption(format!("Decryption failed: {}", e)))
    }

    pub fn generate_hmac(
        &self,
        shared_secret_hex: &str,
        message: &str,
    ) -> Result<String, ConduitError> {
        let shared_secret = Self::parse_secret(shared_secret_hex)?;
        Ok(conduit_protocol::hmac::compute_hmac(
            &shared_secret,
            message,
        ))
    }

    pub fn verify_hmac(&self, shared_secret_hex: &str, message: &str, expected_hex: &str) -> bool {
        let shared_secret = match Self::parse_secret(shared_secret_hex) {
            Ok(s) => s,
            Err(_) => return false,
        };
        conduit_protocol::hmac::verify_hmac(&shared_secret, message, expected_hex)
    }

    fn parse_secret(hex: &str) -> Result<[u8; 32], ConduitError> {
        let bytes = hex::decode(hex)
            .map_err(|e| ConduitError::Encryption(format!("Invalid hex secret: {}", e)))?;
        if bytes.len() != 32 {
            return Err(ConduitError::Encryption(format!(
                "Invalid secret length: expected 32, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

pub fn generate_token_hex() -> String {
    let mut token = [0u8; 32];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut token);
    hex::encode(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Scripted key stores ────────────────────────────────────────────────────
    //
    // The data-loss bug lived in the "keyring is unavailable" path, which by
    // definition cannot be reached on a developer machine with a working
    // keychain. These fakes make the failure modes reproducible.

    use std::cell::RefCell;

    struct FakeStore {
        name: &'static str,
        /// `Err(_)` simulates a platform failure: no Secret Service, a locked
        /// keychain, a read-only profile.
        state: RefCell<Result<Option<String>, String>>,
        /// When set, writes fail with this message.
        store_error: Option<String>,
        /// When true, writes are accepted and then forgotten — the "lying
        /// platform" case where `set_password` reports success but nothing is
        /// actually stored.
        forget_writes: bool,
    }

    impl FakeStore {
        fn broken(name: &'static str, msg: &str) -> Self {
            FakeStore {
                name,
                state: RefCell::new(Err(msg.to_string())),
                store_error: None,
                forget_writes: false,
            }
        }

        fn empty(name: &'static str) -> Self {
            FakeStore {
                name,
                state: RefCell::new(Ok(None)),
                store_error: None,
                forget_writes: false,
            }
        }

        fn holding(name: &'static str, secret: &str) -> Self {
            FakeStore {
                name,
                state: RefCell::new(Ok(Some(secret.to_string()))),
                store_error: None,
                forget_writes: false,
            }
        }

        fn unwritable(mut self, msg: &str) -> Self {
            self.store_error = Some(msg.to_string());
            self
        }

        fn lying(mut self) -> Self {
            self.forget_writes = true;
            self
        }

        fn contents(&self) -> Option<String> {
            self.state.borrow().as_ref().ok().cloned().flatten()
        }
    }

    impl KeyStore for FakeStore {
        fn name(&self) -> &'static str {
            self.name
        }
        fn load(&self) -> Result<Option<String>, String> {
            match &*self.state.borrow() {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(e.clone()),
            }
        }
        fn store(&self, secret: &str) -> Result<(), String> {
            if let Some(e) = &self.store_error {
                return Err(e.clone());
            }
            if self.forget_writes {
                return Ok(());
            }
            *self.state.borrow_mut() = Ok(Some(secret.to_string()));
            Ok(())
        }
        fn confirm(&self) -> Option<String> {
            self.load().ok().flatten()
        }
        fn clear(&self) -> Result<(), String> {
            *self.state.borrow_mut() = Ok(None);
            Ok(())
        }
    }

    const ACCOUNT: &str = "sqlite_key";

    // ── The happy paths ────────────────────────────────────────────────────────

    #[test]
    fn healthy_keyring_gets_the_secret_and_no_key_file_is_created() {
        let keyring = FakeStore::empty("OS keyring");
        let key_file = FakeStore::empty("0600 key file");

        let secret = resolve_secret(&keyring, &key_file, ACCOUNT).expect("must succeed");

        assert_eq!(secret.len(), 64, "a 32-byte key as 64 hex characters");
        assert_eq!(
            keyring.contents().as_deref(),
            Some(secret.as_str()),
            "the keyring must hold exactly what was returned"
        );
        assert_eq!(
            key_file.contents(),
            None,
            "a working keyring must not leave a plaintext key on disk"
        );
    }

    #[test]
    fn existing_keyring_secret_is_reused_never_regenerated() {
        let keyring = FakeStore::holding("OS keyring", &"a".repeat(64));
        let key_file = FakeStore::empty("0600 key file");

        let secret = resolve_secret(&keyring, &key_file, ACCOUNT).unwrap();

        assert_eq!(secret, "a".repeat(64));
    }

    // ── The data-loss bug ──────────────────────────────────────────────────────
    //
    // REGRESSION. `Storage::new` used to do:
    //
    //     match entry.get_password() {
    //         Ok(key) => key,
    //         Err(_)  => { let new_key = generate_token_hex();
    //                      let _ = entry.set_password(&new_key);   // discarded
    //                      new_key }
    //     }
    //
    // On a Linux host with no Secret Service, `set_password` returns
    // `PlatformFailure` every time, so a *different* key came back on every
    // launch, `PRAGMA key` stopped matching, and the recovery path renamed the
    // user's database to a fresh `.bak` and started empty — every launch.

    #[test]
    fn keyring_write_failure_falls_back_to_the_key_file() {
        let keyring =
            FakeStore::empty("OS keyring").unwritable("PlatformFailure: no secret service");
        let key_file = FakeStore::empty("0600 key file");

        let secret =
            resolve_secret(&keyring, &key_file, ACCOUNT).expect("the fallback must carry it");

        assert_eq!(
            key_file.contents().as_deref(),
            Some(secret.as_str()),
            "the returned secret must be durably stored, not merely generated"
        );
    }

    #[test]
    fn keyring_that_ignores_writes_falls_back_to_the_key_file() {
        // `set_password` returns Ok but nothing is readable afterwards. Trusting
        // the write here is what produced an unusable, per-launch key.
        let keyring = FakeStore::empty("OS keyring").lying();
        let key_file = FakeStore::empty("0600 key file");

        let secret = resolve_secret(&keyring, &key_file, ACCOUNT).expect("must fall back");

        assert_eq!(key_file.contents().as_deref(), Some(secret.as_str()));
    }

    #[test]
    fn key_is_stable_across_launches_while_the_keyring_is_broken() {
        // THE regression test for the data-loss bug: two "launches" with a
        // broken keyring must resolve to the *same* key, or the database
        // becomes undecryptable after the first one.
        let key_file = FakeStore::empty("0600 key file");

        let broken = || FakeStore::broken("OS keyring", "PlatformFailure: no secret service");
        let first = resolve_secret(&broken(), &key_file, ACCOUNT).unwrap();
        let second = resolve_secret(&broken(), &key_file, ACCOUNT).unwrap();
        let third = resolve_secret(&broken(), &key_file, ACCOUNT).unwrap();

        assert_eq!(first, second, "launch 2 must not rotate the key");
        assert_eq!(second, third, "launch 3 must not rotate the key");
        assert_eq!(key_file.contents().as_deref(), Some(first.as_str()));
    }

    #[test]
    fn key_file_secret_survives_the_keyring_coming_back() {
        // Scenario: launch 1 has no Secret Service (fallback file created),
        // launch 2 runs on a host where it works. The key must not change.
        let key_file = FakeStore::empty("0600 key file");
        let original =
            resolve_secret(&FakeStore::broken("OS keyring", "down"), &key_file, ACCOUNT).unwrap();

        // A keyring that now works but holds nothing.
        let healthy = FakeStore::empty("OS keyring");
        let after_recovery = resolve_secret(&healthy, &key_file, ACCOUNT).unwrap();
        assert_eq!(
            after_recovery, original,
            "a keyring outage must not orphan the existing database"
        );
    }

    // ── Recovery: the fallback is temporary ───────────────────────────────────

    #[test]
    fn key_file_is_migrated_back_into_a_recovered_keyring() {
        let key_file = FakeStore::empty("0600 key file");
        let secret =
            resolve_secret(&FakeStore::broken("OS keyring", "down"), &key_file, ACCOUNT).unwrap();

        let healthy = FakeStore::empty("OS keyring");
        let after = resolve_secret(&healthy, &key_file, ACCOUNT).unwrap();

        assert_eq!(after, secret, "the key must not change during migration");
        assert_eq!(
            healthy.contents().as_deref(),
            Some(secret.as_str()),
            "the secret must now live in the keyring"
        );
        assert_eq!(
            key_file.contents(),
            None,
            "the plaintext key file must be removed once the keyring holds it"
        );
    }

    #[test]
    fn key_file_is_kept_when_the_keyring_migration_cannot_be_confirmed() {
        let key_file = FakeStore::empty("0600 key file");
        let secret =
            resolve_secret(&FakeStore::broken("OS keyring", "down"), &key_file, ACCOUNT).unwrap();

        let healthy = FakeStore::empty("OS keyring").lying();
        resolve_secret(&healthy, &key_file, ACCOUNT).unwrap();

        assert_eq!(
            key_file.contents().as_deref(),
            Some(secret.as_str()),
            "the only durable copy must never be deleted on an unconfirmed write"
        );
    }

    #[test]
    fn key_file_wins_when_the_keyring_holds_a_different_secret() {
        // Regenerating here would orphan the database, so the file is trusted
        // and *nothing* is deleted.
        let key_file = FakeStore::empty("0600 key file");
        let from_file = "b".repeat(64);
        *key_file.state.borrow_mut() = Ok(Some(from_file.clone()));

        let keyring = FakeStore::holding("OS keyring", &"c".repeat(64));
        let secret = resolve_secret(&keyring, &key_file, ACCOUNT).unwrap();

        assert_eq!(secret, from_file);
        assert_eq!(
            keyring.contents().as_deref(),
            Some("c".repeat(64).as_str()),
            "the divergent keyring entry must be left alone for investigation"
        );
        assert_eq!(key_file.contents().as_deref(), Some(from_file.as_str()));
    }

    // ── Failing loudly ─────────────────────────────────────────────────────────

    #[test]
    fn an_unusable_key_file_is_an_error_not_an_empty_key() {
        // An empty key file would be used *as the key*, silently making the
        // database unreadable.
        let key_file = FakeStore::empty("0600 key file");
        *key_file.state.borrow_mut() = Ok(Some(String::new()));

        let err = resolve_secret(
            &FakeStore::holding("OS keyring", &"a".repeat(64)),
            &key_file,
            ACCOUNT,
        )
        .expect_err("an empty key file must not be used");
        assert!(err.to_string().contains("EMPTY"), "unexpected error: {err}");
    }

    #[test]
    fn a_broken_keyring_and_an_unwritable_key_file_is_a_clear_error() {
        let keyring = FakeStore::broken("OS keyring", "PlatformFailure");
        let key_file =
            FakeStore::empty("0600 key file").unwritable("EACCES: read-only file system");

        let err = resolve_secret(&keyring, &key_file, ACCOUNT)
            .expect_err("there is no durable store left, so we must refuse to continue");
        let msg = err.to_string();
        assert!(
            msg.contains("Neither the OS keyring nor the 0600 key file"),
            "the error must name both stores: {msg}"
        );
        assert!(
            msg.contains("read-only file system"),
            "the error must carry the underlying cause: {msg}"
        );
    }

    // ── The real key-file store ───────────────────────────────────────────────

    #[test]
    fn key_file_store_round_trips_and_reports_absence_distinctly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = KeyFileStore {
            path: dir.path().join("keys").join("sqlite_key.key"),
        };

        assert_eq!(store.load().expect("absent is not an error"), None);
        assert_eq!(store.confirm(), None, "absent means 'cannot confirm'");

        let secret = generate_token_hex();
        store.store(&secret).expect("store");
        assert_eq!(
            store.load().expect("load").as_deref(),
            Some(secret.as_str())
        );
        assert_eq!(store.confirm().as_deref(), Some(secret.as_str()));

        store.clear().expect("clear");
        assert_eq!(store.load().expect("absent again"), None);
        store.clear().expect("clearing twice is not an error");
    }

    #[test]
    fn key_file_is_empty_which_is_unusable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.key");
        std::fs::write(&path, "   \n").expect("write");
        let store = KeyFileStore { path };
        let err = store
            .load()
            .expect_err("an empty key file must be reported, not used");
        assert!(err.contains("is empty"), "unexpected error: {err}");
    }

    #[test]
    fn key_file_path_lives_under_the_app_data_dir() {
        let path = KeyFileStore::path_for_account(SQLITE_KEY_ACCOUNT);
        let expected_root = app_data_dir();
        assert_eq!(path, expected_root.join("keys").join("sqlite_key.key"));
        assert_eq!(
            path.parent().and_then(|p| p.parent()),
            Some(expected_root.as_path()),
            "the fallback key must sit inside the app data directory, not a temp dir"
        );
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_created_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("keys").join("sqlite_key.key");
        let store = KeyFileStore { path: path.clone() };
        store.store(&generate_token_hex()).expect("store");

        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "key file must be 0600, got {mode:o}");
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(
            dir_mode & 0o777,
            0o700,
            "the key directory must be 0700, got {dir_mode:o}"
        );
    }

    #[test]
    fn app_data_dir_is_conduit_scoped() {
        assert_eq!(
            app_data_dir().file_name().and_then(|n| n.to_str()),
            Some("conduit")
        );
    }

    #[test]
    fn new_creates_valid_keypair() {
        let em = EncryptionManager::new_random();
        let hex = em.public_key_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_instances_have_different_keys() {
        let a = EncryptionManager::new_random();
        let b = EncryptionManager::new_random();
        assert_ne!(a.public_key_hex(), b.public_key_hex());
    }

    #[test]
    fn derive_shared_secret_round_trip() {
        let alice = EncryptionManager::new_random();
        let bob = EncryptionManager::new_random();

        let alice_secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let bob_secret = bob.derive_shared_secret(&alice.public_key_hex()).unwrap();
        assert_eq!(alice_secret, bob_secret);
    }

    #[test]
    fn derive_shared_secret_invalid_hex() {
        let em = EncryptionManager::new_random();
        let result = em.derive_shared_secret("not-hex!!!");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid public key hex")
        );
    }

    #[test]
    fn derive_shared_secret_wrong_length() {
        let em = EncryptionManager::new_random();
        let short_key = hex::encode([0u8; 16]);
        let result = em.derive_shared_secret(&short_key);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid public key length")
        );
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let alice = EncryptionManager::new_random();
        let bob = EncryptionManager::new_random();
        let secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();

        let plaintext = "Hello, Conduit!";
        let (nonce, ciphertext) = alice.encrypt(&hex::encode(secret), plaintext).unwrap();
        assert!(!ciphertext.is_empty());
        assert_eq!(nonce.len(), 24);

        let decrypted = bob
            .decrypt(&hex::encode(secret), &nonce, &ciphertext)
            .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_time() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (n1, c1) = em.encrypt(&hex::encode(secret), "test").unwrap();
        let (n2, c2) = em.encrypt(&hex::encode(secret), "test").unwrap();
        // Nonces are random, so ciphertext should differ (with overwhelming probability)
        assert_ne!(c1, c2);
        assert_ne!(n1, n2);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let alice = EncryptionManager::new_random();
        let bob = EncryptionManager::new_random();
        let eve = EncryptionManager::new_random();

        let alice_bob_secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let (nonce, ciphertext) = alice
            .encrypt(&hex::encode(alice_bob_secret), "secret")
            .unwrap();

        let eve_secret = eve.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let result = eve.decrypt(&hex::encode(eve_secret), &nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (nonce, mut ciphertext) = em.encrypt(&hex::encode(secret), "test data").unwrap();
        if let Some(byte) = ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        let result = em.decrypt(&hex::encode(secret), &nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_empty_plaintext() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (nonce, ciphertext) = em.encrypt(&hex::encode(secret), "").unwrap();
        let decrypted = em
            .decrypt(&hex::encode(secret), &nonce, &ciphertext)
            .unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn decrypt_long_plaintext() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let plaintext = "x".repeat(100_000);
        let (nonce, ciphertext) = em.encrypt(&hex::encode(secret), &plaintext).unwrap();
        let decrypted = em
            .decrypt(&hex::encode(secret), &nonce, &ciphertext)
            .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn generate_token_hex_length_and_randomness() {
        let t1 = generate_token_hex();
        let t2 = generate_token_hex();
        assert_eq!(t1.len(), 64);
        assert_eq!(t2.len(), 64);
        assert_ne!(t1, t2);
        assert!(t1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn public_key_hex_is_stable() {
        let em = EncryptionManager::new_random();
        let k1 = em.public_key_hex();
        let k2 = em.public_key_hex();
        assert_eq!(k1, k2);
    }

    #[test]
    fn clone_shares_same_keys() {
        let em1 = EncryptionManager::new_random();
        let em2 = em1.clone();
        assert_eq!(em1.public_key_hex(), em2.public_key_hex());
    }

    // ---------------------------------------------------------------
    //  Nonce uniqueness – statistical test
    // ---------------------------------------------------------------

    #[test]
    fn encrypt_produces_unique_nonces_across_many_encryptions() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let mut nonces = std::collections::HashSet::new();
        let count = 200;

        for _ in 0..count {
            let (nonce, _ciphertext) = em.encrypt(&secret_hex, "test").unwrap();
            let nonce_hex = hex::encode(&nonce);
            assert!(
                nonces.insert(nonce_hex),
                "duplicate nonce detected — nonce uniqueness violated"
            );
        }
        assert_eq!(nonces.len(), count, "all nonces should be unique");
    }

    // ---------------------------------------------------------------
    //  Invalid keys – various error scenarios
    // ---------------------------------------------------------------

    #[test]
    fn derive_shared_secret_too_short_returns_error() {
        let em = EncryptionManager::new_random();
        let short_key = hex::encode([0u8; 15]); // 15 bytes
        let result = em.derive_shared_secret(&short_key);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn derive_shared_secret_too_long_returns_error() {
        let em = EncryptionManager::new_random();
        let long_key = hex::encode([0u8; 64]); // 64 bytes
        let result = em.derive_shared_secret(&long_key);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn derive_shared_secret_empty_string_returns_error() {
        let em = EncryptionManager::new_random();
        let result = em.derive_shared_secret("");
        assert!(result.is_err());
    }

    #[test]
    fn derive_shared_secret_odd_length_hex_returns_error() {
        let em = EncryptionManager::new_random();
        // Odd-length hex string is invalid
        let result = em.derive_shared_secret("abc");
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_with_invalid_secret_hex_fails() {
        let em = EncryptionManager::new_random();
        let result = em.encrypt("not-valid-hex!!!", "test");
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_with_wrong_length_secret_hex_fails() {
        let em = EncryptionManager::new_random();
        let short_secret = hex::encode([0u8; 16]); // 16 bytes, not 32
        let result = em.encrypt(&short_secret, "test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid secret length")
        );
    }

    #[test]
    fn decrypt_with_invalid_secret_hex_fails() {
        let em = EncryptionManager::new_random();
        let result = em.decrypt("not-hex", &[0u8; 24], &[0u8; 16]);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_wrong_length_secret_fails() {
        let em = EncryptionManager::new_random();
        let short = hex::encode([0u8; 8]);
        let result = em.decrypt(&short, &[0u8; 24], &[0u8; 16]);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    //  Tampered ciphertext – additional edge cases
    // ---------------------------------------------------------------

    #[test]
    fn decrypt_with_tampered_last_byte_fails() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (nonce, mut ciphertext) = em.encrypt(&hex::encode(secret), "sensitive data").unwrap();
        if let Some(last) = ciphertext.last_mut() {
            *last ^= 0xFF;
        }
        let result = em.decrypt(&hex::encode(secret), &nonce, &ciphertext);
        assert!(result.is_err(), "tampered last byte should fail decryption");
    }

    #[test]
    fn decrypt_with_appended_bytes_fails() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (nonce, mut ciphertext) = em.encrypt(&hex::encode(secret), "data").unwrap();
        ciphertext.push(0xFF); // append extra byte
        let result = em.decrypt(&hex::encode(secret), &nonce, &ciphertext);
        assert!(result.is_err(), "appended bytes should fail decryption");
    }

    #[test]
    fn decrypt_with_removed_byte_fails() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (nonce, mut ciphertext) = em.encrypt(&hex::encode(secret), "data").unwrap();
        ciphertext.pop(); // remove last byte
        let result = em.decrypt(&hex::encode(secret), &nonce, &ciphertext);
        assert!(result.is_err(), "removed byte should fail decryption");
    }

    #[test]
    fn decrypt_with_completely_empty_ciphertext_fails() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let nonce = [0u8; 24];
        let result = em.decrypt(&hex::encode(secret), &nonce, &[]);
        assert!(result.is_err(), "empty ciphertext should fail");
    }

    #[test]
    fn decrypt_with_wrong_nonce_fails() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();

        let (_nonce, ciphertext) = em.encrypt(&hex::encode(secret), "data").unwrap();
        let wrong_nonce = [1u8; 24];
        let result = em.decrypt(&hex::encode(secret), &wrong_nonce, &ciphertext);
        assert!(result.is_err(), "wrong nonce should fail decryption");
    }

    // ---------------------------------------------------------------
    //  Binary encryption roundtrip
    // ---------------------------------------------------------------

    #[test]
    fn encrypt_binary_decrypt_binary_roundtrip() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let plaintext = b"binary data with \x00\xFF bytes";
        let (nonce, ciphertext) = em.encrypt_binary(&secret_hex, plaintext).unwrap();
        let decrypted = em.decrypt_binary(&secret_hex, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_binary_empty_data() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (nonce, ciphertext) = em.encrypt_binary(&secret_hex, b"").unwrap();
        let decrypted = em.decrypt_binary(&secret_hex, &nonce, &ciphertext).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn encrypt_binary_large_data() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let plaintext = vec![0xAB; 500_000]; // 500KB
        let (nonce, ciphertext) = em.encrypt_binary(&secret_hex, &plaintext).unwrap();
        let decrypted = em.decrypt_binary(&secret_hex, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_binary_produces_different_ciphertext_each_time() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let data = b"same input";
        let (n1, c1) = em.encrypt_binary(&secret_hex, data).unwrap();
        let (n2, c2) = em.encrypt_binary(&secret_hex, data).unwrap();
        assert_ne!(c1, c2);
        assert_ne!(n1, n2);
    }

    // ---------------------------------------------------------------
    //  HMAC generation and verification
    // ---------------------------------------------------------------

    #[test]
    fn hmac_generate_and_verify_roundtrip() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let message = "important message to authenticate";
        let hmac_hex = em.generate_hmac(&secret_hex, message).unwrap();
        assert!(em.verify_hmac(&secret_hex, message, &hmac_hex));
    }

    #[test]
    fn hmac_verify_rejects_wrong_message() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let hmac_hex = em.generate_hmac(&secret_hex, "original message").unwrap();
        assert!(!em.verify_hmac(&secret_hex, "tampered message", &hmac_hex));
    }

    #[test]
    fn hmac_verify_rejects_wrong_secret() {
        let alice = EncryptionManager::new_random();
        let bob = EncryptionManager::new_random();
        let eve = EncryptionManager::new_random();

        // ECDH is symmetric: alice↔bob derive the SAME shared secret.
        // A genuinely wrong secret must come from an unrelated party.
        let alice_secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let wrong_secret = bob.derive_shared_secret(&eve.public_key_hex()).unwrap();
        assert_ne!(alice_secret, wrong_secret);

        let hmac_hex = alice
            .generate_hmac(&hex::encode(alice_secret), "msg")
            .unwrap();
        assert!(!bob.verify_hmac(&hex::encode(wrong_secret), "msg", &hmac_hex));
    }

    #[test]
    fn hmac_generate_produces_64_char_hex() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let hmac_hex = em.generate_hmac(&secret_hex, "test").unwrap();
        assert_eq!(hmac_hex.len(), 64, "HMAC-SHA256 should be 64 hex chars");
        assert!(hmac_hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hmac_generate_different_messages_produce_different_hmacs() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let h1 = em.generate_hmac(&secret_hex, "message 1").unwrap();
        let h2 = em.generate_hmac(&secret_hex, "message 2").unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn hmac_verify_rejects_invalid_hex_expected() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        assert!(!em.verify_hmac(&secret_hex, "msg", "not-valid-hex!!!"));
    }

    #[test]
    fn hmac_verify_rejects_empty_expected() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        assert!(!em.verify_hmac(&secret_hex, "msg", ""));
    }

    #[test]
    fn hmac_verify_rejects_truncated_expected() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let full_hmac = em.generate_hmac(&secret_hex, "msg").unwrap();
        let truncated = &full_hmac[..32]; // half length
        assert!(!em.verify_hmac(&secret_hex, "msg", truncated));
    }

    // ---------------------------------------------------------------
    //  generate_token_hex edge cases
    // ---------------------------------------------------------------

    #[test]
    fn generate_token_hex_always_64_chars() {
        for _ in 0..50 {
            let token = generate_token_hex();
            assert_eq!(token.len(), 64);
        }
    }

    #[test]
    fn generate_token_hex_only_hex_chars() {
        for _ in 0..50 {
            let token = generate_token_hex();
            assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn generate_token_hex_unique_across_runs() {
        let mut tokens = std::collections::HashSet::new();
        for _ in 0..100 {
            let token = generate_token_hex();
            assert!(tokens.insert(token), "duplicate token generated");
        }
        assert_eq!(tokens.len(), 100);
    }

    // ---------------------------------------------------------------
    //  parse_secret – internal validation
    // ---------------------------------------------------------------

    #[test]
    fn parse_secret_valid_32_bytes() {
        let secret = hex::encode([0xAA; 32]);
        let result = EncryptionManager::new_random();
        // We test parse_secret indirectly through encrypt which calls it
        let (nonce, ciphertext) = result.encrypt(&secret, "test").unwrap();
        assert!(!ciphertext.is_empty());
        assert_eq!(nonce.len(), 24);
    }

    #[test]
    fn parse_secret_rejects_short_hex() {
        let em = EncryptionManager::new_random();
        let short = hex::encode([0u8; 31]); // 31 bytes
        let result = em.encrypt(&short, "test");
        assert!(result.is_err());
    }

    #[test]
    fn parse_secret_rejects_long_hex() {
        let em = EncryptionManager::new_random();
        let long = hex::encode([0u8; 33]); // 33 bytes
        let result = em.encrypt(&long, "test");
        assert!(result.is_err());
    }

    #[test]
    fn parse_secret_rejects_invalid_hex() {
        let em = EncryptionManager::new_random();
        let result = em.encrypt("zzzz_not_hex", "test");
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    //  Cross-party encryption: verify different key pairs produce
    //  different shared secrets
    // ---------------------------------------------------------------

    #[test]
    fn different_key_pairs_produce_different_shared_secrets() {
        let a = EncryptionManager::new_random();
        let b = EncryptionManager::new_random();
        let c = EncryptionManager::new_random();

        let ab = a.derive_shared_secret(&b.public_key_hex()).unwrap();
        let ac = a.derive_shared_secret(&c.public_key_hex()).unwrap();
        assert_ne!(
            ab, ac,
            "different peers should yield different shared secrets"
        );
    }

    #[test]
    fn self_shared_secret_is_deterministic() {
        let em = EncryptionManager::new_random();
        let s1 = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let s2 = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        assert_eq!(s1, s2, "same key pair should yield same shared secret");
    }

    // ---------------------------------------------------------------
    //  Encrypt then decrypt with non-UTF-8 binary data
    // ---------------------------------------------------------------

    #[test]
    fn decrypt_binary_non_utf8_data() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        // Data that is NOT valid UTF-8
        let binary_data = vec![0x00, 0xFF, 0xFE, 0xFD, 0x80, 0xC0, 0xF0];
        let (nonce, ciphertext) = em.encrypt_binary(&secret_hex, &binary_data).unwrap();
        let decrypted = em.decrypt_binary(&secret_hex, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, binary_data);

        // The string decrypt variant should fail on non-UTF-8
        let result = em.decrypt(&secret_hex, &nonce, &ciphertext);
        assert!(result.is_err(), "non-UTF-8 should fail string decrypt");
    }

    // ---------------------------------------------------------------
    //  Multiple encryption operations don't interfere
    // ---------------------------------------------------------------

    #[test]
    fn multiple_independent_encryptions_dont_interfere() {
        let alice = EncryptionManager::new_random();
        let bob = EncryptionManager::new_random();

        let secret_ab = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let secret_ab_hex = hex::encode(secret_ab);

        let msg1 = "first message";
        let msg2 = "second message";

        let (n1, c1) = alice.encrypt(&secret_ab_hex, msg1).unwrap();
        let (n2, c2) = alice.encrypt(&secret_ab_hex, msg2).unwrap();

        let d1 = bob.decrypt(&secret_ab_hex, &n1, &c1).unwrap();
        let d2 = bob.decrypt(&secret_ab_hex, &n2, &c2).unwrap();

        assert_eq!(d1, msg1);
        assert_eq!(d2, msg2);
    }

    // ---------------------------------------------------------------
    //  Encrypt binary then decrypt as string (valid UTF-8)
    // ---------------------------------------------------------------

    #[test]
    fn encrypt_binary_decrypt_as_string_utf8() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let plaintext = "valid UTF-8 text \u{1F600}";
        let (nonce, ciphertext) = em
            .encrypt_binary(&secret_hex, plaintext.as_bytes())
            .unwrap();
        let decrypted = em.decrypt(&secret_hex, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    // ---------------------------------------------------------------
    //  Verify public key format stability
    // ---------------------------------------------------------------

    #[test]
    fn public_key_hex_always_64_hex_chars() {
        for _ in 0..20 {
            let em = EncryptionManager::new_random();
            let pk = em.public_key_hex();
            assert_eq!(
                pk.len(),
                64,
                "X25519 public key should be 32 bytes = 64 hex chars"
            );
            assert!(pk.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn public_key_hex_deterministic_across_calls() {
        let em = EncryptionManager::new_random();
        let k1 = em.public_key_hex();
        let k2 = em.public_key_hex();
        let k3 = em.public_key_hex();
        assert_eq!(k1, k2);
        assert_eq!(k2, k3);
    }

    // ---------------------------------------------------------------
    //  Encryption with very long plaintext
    // ---------------------------------------------------------------

    #[test]
    fn encrypt_decrypt_1mb_plaintext() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let plaintext = "A".repeat(1_000_000); // 1MB
        let (nonce, ciphertext) = em.encrypt(&secret_hex, &plaintext).unwrap();
        let decrypted = em.decrypt(&secret_hex, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted.len(), 1_000_000);
        assert_eq!(decrypted, plaintext);
    }

    // ---------------------------------------------------------------
    //  EncryptionManager::new (keyring) – test new_random fallback
    // ---------------------------------------------------------------

    #[test]
    fn new_random_creates_functional_instance() {
        let em = EncryptionManager::new_random();
        // Should be able to derive shared secret with own public key
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        assert_eq!(secret.len(), 32);
    }

    #[test]
    fn new_random_instances_are_independent() {
        let em1 = EncryptionManager::new_random();
        let em2 = EncryptionManager::new_random();

        // Different keys
        assert_ne!(em1.public_key_hex(), em2.public_key_hex());

        // Can still interoperate
        let s1 = em1.derive_shared_secret(&em2.public_key_hex()).unwrap();
        let s2 = em2.derive_shared_secret(&em1.public_key_hex()).unwrap();
        assert_eq!(s1, s2);
    }

    // ---------------------------------------------------------------
    //  Edge case: encrypt with empty plaintext
    // ---------------------------------------------------------------

    #[test]
    fn encrypt_empty_plaintext_produces_valid_ciphertext() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (nonce, ciphertext) = em.encrypt(&secret_hex, "").unwrap();
        // XChaCha20-Poly1305 with empty plaintext produces only the 16-byte auth tag
        assert_eq!(ciphertext.len(), 16);
        assert_eq!(nonce.len(), 24);

        let decrypted = em.decrypt(&secret_hex, &nonce, &ciphertext).unwrap();
        assert_eq!(decrypted, "");
    }

    // ---------------------------------------------------------------
    //  Nonce length validation (regression: network-reachable panic)
    //
    //  server/mod.rs feeds an attacker-controlled hex nonce into decrypt().
    //  A wrong-length nonce previously panicked via XNonce::try_from().unwrap().
    // ---------------------------------------------------------------

    #[test]
    fn decrypt_binary_rejects_nonce_shorter_than_24_bytes() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (nonce, ciphertext) = em.encrypt(&secret_hex, "data").unwrap();
        assert_eq!(nonce.len(), 24);

        let short = &nonce[..16];
        let result = em.decrypt_binary(&secret_hex, short, &ciphertext);
        assert!(result.is_err(), "16-byte nonce must be rejected, not panic");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid nonce length"),
            "error should mention nonce length, got: {}",
            err
        );
        assert!(
            err.contains("got 16"),
            "error should report actual length, got: {}",
            err
        );
    }

    #[test]
    fn decrypt_binary_rejects_empty_nonce() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (_, ciphertext) = em.encrypt(&secret_hex, "data").unwrap();
        let result = em.decrypt_binary(&secret_hex, &[], &ciphertext);
        assert!(result.is_err(), "empty nonce must be rejected, not panic");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid nonce length")
        );
    }

    #[test]
    fn decrypt_binary_rejects_nonce_longer_than_24_bytes() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (_, ciphertext) = em.encrypt(&secret_hex, "data").unwrap();
        let long = vec![0u8; 32];
        let result = em.decrypt_binary(&secret_hex, &long, &ciphertext);
        assert!(result.is_err(), "32-byte nonce must be rejected, not panic");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid nonce length") && err.contains("got 32"),
            "error should mention length 32, got: {}",
            err
        );
    }

    #[test]
    fn decrypt_string_path_rejects_wrong_length_nonce() {
        // decrypt() delegates to decrypt_binary — same validation applies.
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (nonce, ciphertext) = em.encrypt(&secret_hex, "msg").unwrap();
        let result = em.decrypt(&secret_hex, &nonce[..8], &ciphertext);
        assert!(result.is_err(), "string decrypt must also reject bad nonce");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid nonce length")
        );
    }

    #[test]
    fn decrypt_binary_rejects_tampered_ciphertext() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (nonce, mut ciphertext) = em.encrypt_binary(&secret_hex, b"payload").unwrap();
        if let Some(b) = ciphertext.first_mut() {
            *b ^= 0xFF;
        }
        let result = em.decrypt_binary(&secret_hex, &nonce, &ciphertext);
        assert!(
            result.is_err(),
            "tampered ciphertext must fail authentication"
        );
    }

    // ---------------------------------------------------------------
    //  HMAC invalid secret handling
    // ---------------------------------------------------------------

    #[test]
    fn generate_hmac_invalid_secret_returns_error() {
        let em = EncryptionManager::new_random();
        let result = em.generate_hmac("not-valid-hex!!!", "msg");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("Invalid hex"),
            "should surface the parse_secret error"
        );
    }

    #[test]
    fn generate_hmac_wrong_length_secret_returns_error() {
        let em = EncryptionManager::new_random();
        let short = hex::encode([0u8; 16]);
        let result = em.generate_hmac(&short, "msg");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid secret length")
        );
    }

    #[test]
    fn verify_hmac_invalid_secret_returns_false() {
        let em = EncryptionManager::new_random();
        // Invalid secrets must return false, never panic.
        assert!(!em.verify_hmac("not-hex!!!", "msg", "aabbcc"));
        assert!(!em.verify_hmac("", "msg", ""));
        let short = hex::encode([0u8; 8]);
        assert!(!em.verify_hmac(&short, "msg", "deadbeef"));
    }

    #[test]
    fn hmac_verify_accepts_uppercase_hex() {
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let mac = em.generate_hmac(&secret_hex, "message").unwrap();
        let upper = mac.to_uppercase();
        assert!(
            em.verify_hmac(&secret_hex, "message", &upper),
            "hex decode is case-insensitive; uppercase MAC should verify"
        );
    }

    #[test]
    fn generate_hmac_matches_shared_protocol_impl() {
        // X25 consolidation: EncryptionManager must use conduit_protocol::hmac.
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let via_manager = em.generate_hmac(&secret_hex, "shared-impl").unwrap();
        let via_protocol = conduit_protocol::hmac::compute_hmac(&secret, "shared-impl");
        assert_eq!(
            via_manager, via_protocol,
            "desktop generate_hmac must match conduit_protocol::hmac::compute_hmac"
        );
        assert!(em.verify_hmac(&secret_hex, "shared-impl", &via_protocol));
        assert!(conduit_protocol::hmac::verify_hmac(
            &secret,
            "shared-impl",
            &via_manager
        ));
    }

    #[test]
    fn decrypt_with_tampered_nonce_fails() {
        // Right length, wrong value — AEAD authentication must fail (not panic).
        let em = EncryptionManager::new_random();
        let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
        let secret_hex = hex::encode(secret);

        let (nonce, ciphertext) = em.encrypt(&secret_hex, "confidential").unwrap();
        let mut bad_nonce = nonce.clone();
        bad_nonce[0] ^= 0xFF;
        let result = em.decrypt(&secret_hex, &bad_nonce, &ciphertext);
        assert!(result.is_err(), "wrong nonce must fail decryption");
    }

    // ---------------------------------------------------------------
    //  EncryptionManager::new (keyring) – persistence across calls
    // ---------------------------------------------------------------

    #[test]
    fn keyring_new_persists_key_across_calls() {
        // Two successful constructions must yield the same public key
        // (keyring entry is read, not regenerated). Skip gracefully if the
        // OS keyring is unavailable — never write invalid data.
        match (EncryptionManager::new(), EncryptionManager::new()) {
            (Ok(a), Ok(b)) => {
                assert_eq!(
                    a.public_key_hex(),
                    b.public_key_hex(),
                    "keyring must persist the same key across EncryptionManager::new() calls"
                );
            }
            (Err(e), _) | (_, Err(e)) => {
                eprintln!(
                    "SKIP keyring_new_persists_key_across_calls: keyring unavailable: {}",
                    e
                );
            }
        }
    }

    #[test]
    fn keyring_new_produces_functional_instance_when_available() {
        match EncryptionManager::new() {
            Ok(em) => {
                let secret = em.derive_shared_secret(&em.public_key_hex()).unwrap();
                assert_eq!(secret.len(), 32);
                let (nonce, ct) = em.encrypt(&hex::encode(secret), "hi").unwrap();
                assert_eq!(em.decrypt(&hex::encode(secret), &nonce, &ct).unwrap(), "hi");
            }
            Err(e) => eprintln!(
                "SKIP keyring_new_produces_functional_instance_when_available: {}",
                e
            ),
        }
    }
}
