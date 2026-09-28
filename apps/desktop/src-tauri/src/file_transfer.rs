use base64::{Engine, engine::general_purpose::STANDARD as B64};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::error::ConduitError;
use crate::storage::Storage;

const CHUNK_SIZE: usize = 64 * 1024; // 64KB

/// Sub-directory created under the platform download folder.
const DOWNLOAD_SUBDIR: &str = "Conduit";

/// Settings key holding the user-chosen download folder. Re-read on every
/// finalize so changing it in Settings takes effect without a restart.
const DOWNLOAD_FOLDER_SETTING: &str = "default_download_folder";

/// Hard ceiling on the bytes accepted for a single inbound transfer.
///
/// `security::MAX_FILE_SIZE` already rejects a `file/request` whose *declared*
/// `size` exceeds 10 GB, but that value is chosen by the sender. This is the
/// second, independent bound, enforced against the bytes actually written to
/// disk — otherwise a peer can declare 1 KB and stream 10 GB of chunks.
pub const MAX_TRANSFER_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GB

/// Hard ceiling on a single outbound file.
///
/// Mirrors [`MAX_TRANSFER_BYTES`] so anything Conduit can receive it can also
/// send, while still refusing "checksum a 4 TB sparse file" denial of service.
pub const MAX_SEND_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GB

/// An inbound transfer with no chunk for this long is abandoned and its chunk
/// directory removed.
const INCOMING_TTL: Duration = Duration::from_secs(60 * 60);

/// An outbound transfer with no progress for this long is dropped. Comfortably
/// above the 30 s accept window plus the streaming time of a 10 GB transfer.
const OUTGOING_TTL: Duration = Duration::from_secs(30 * 60);

/// Upper bound on the session send-allowlist. The list is only appended to by
/// an explicit user pick, but a wedged frontend must not be able to grow it
/// without limit.
const MAX_SEND_ALLOWLIST_ENTRIES: usize = 256;

/// Upper bound on the "where did transfer X land" registry, for the same
/// reason: it is a cache, and the durable copy is the `file_transfers` table.
const MAX_COMPLETED_ENTRIES: usize = 64;

/// Longest file name `finalize_incoming` will write, in characters.
///
/// Windows caps a path at 260 characters (MAX_PATH) and a single component at
/// 255. A long download folder plus a collision suffix can therefore push a
/// perfectly reasonable name past what the OS will create — after which
/// `File::create` fails with os error 123 and every subsequent transfer with
/// that name is silently lost.
const MAX_SAVED_NAME_LEN: usize = 120;

/// How many `name (n)` variants to try before giving up on a name.
const MAX_NAME_COLLISIONS: u32 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransferInfo {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub from_device: String,
    pub to_device: String,
    pub status: String,
    pub chunks_received: u32,
    pub total_chunks: u32,
    pub saved_path: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug)]
struct IncomingTransfer {
    name: String,
    size: u64,
    mime: String,
    from_device: String,
    total_chunks: u32,
    /// Number of distinct chunks successfully written to disk.
    received: u32,
    /// Aggregate payload bytes actually written to disk.
    ///
    /// This — not the sender-declared `size` — is what the per-chunk budget in
    /// `receive_chunk_binary` is charged against.
    received_bytes: u64,
    temp_dir: PathBuf,
    /// Optional SHA-256 checksum of the complete file (hex-encoded).
    expected_checksum: Option<String>,
    created_at: Instant,
    last_activity: Instant,
    // NOTE: chunks are stored on disk only (not in memory) to prevent OOM
    // on large transfers. Use temp_dir/chunk_N files for reading.
}

#[derive(Debug)]
struct OutgoingTransfer {
    name: String,
    path: PathBuf,
    size: u64,
    mime: String,
    to_device: String,
    total_chunks: u32,
    next_chunk: u32,
    accepted: bool,
    created_at: Instant,
    last_activity: Instant,
}

pub struct FileTransferEngine {
    incoming: Arc<RwLock<HashMap<String, IncomingTransfer>>>,
    outgoing: Arc<RwLock<HashMap<String, OutgoingTransfer>>>,
    /// Optional handle to the settings database. When present the download
    /// root is re-read from `default_download_folder` for every transfer, so a
    /// setting changed at runtime is honoured without a restart.
    storage: Option<Arc<Storage>>,
    /// Explicit root set by the embedder; wins over the settings value.
    download_root_override: Arc<RwLock<Option<PathBuf>>>,
    temp_dir: PathBuf,
    /// Canonical paths the user picked in the native file dialog (or dropped on
    /// the window) during this session. `start_outgoing` refuses anything
    /// that is neither in here nor inside the download root, so no protocol
    /// frame and no remembered string can turn `send_file` into an
    /// arbitrary-file-read primitive.
    send_allowlist: Arc<RwLock<HashSet<PathBuf>>>,
    /// transfer id -> absolute path of the file it finalized to. Written only
    /// by `finalize_incoming` (local code), never from a message.
    completed: Arc<RwLock<HashMap<String, PathBuf>>>,
}

impl Default for FileTransferEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTransferEngine {
    pub fn new() -> Self {
        let temp_dir = std::env::temp_dir().join("conduit_chunks");
        if let Err(e) = std::fs::create_dir_all(&temp_dir) {
            warn!("Could not create chunk temp directory {temp_dir:?}: {e}");
        }
        info!("Chunk temp directory: {}", temp_dir.display());

        FileTransferEngine {
            incoming: Arc::new(RwLock::new(HashMap::new())),
            outgoing: Arc::new(RwLock::new(HashMap::new())),
            storage: None,
            download_root_override: Arc::new(RwLock::new(None)),
            temp_dir,
            send_allowlist: Arc::new(RwLock::new(HashSet::new())),
            completed: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Attach the settings database so `default_download_folder` is honoured.
    pub fn with_storage(mut self, storage: Arc<Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Pin the download root explicitly (tests, and a future embedder that has
    /// no settings database).
    pub fn with_download_root(mut self, root: PathBuf) -> Self {
        self.download_root_override = Arc::new(RwLock::new(Some(root)));
        self
    }

    /// Pin the directory incoming chunks are staged in.
    ///
    /// Production uses a single `std::env::temp_dir()/conduit_chunks`. Tests
    /// need their own: chunk files are named after the transfer id and live
    /// across process runs, so two tests sharing the directory would see each
    /// other's already-written chunks.
    pub fn with_chunk_temp_dir(mut self, dir: PathBuf) -> Self {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!("Could not create chunk temp directory {dir:?}: {e}");
        }
        self.temp_dir = dir;
        self
    }

    // ── download root ────────────────────────────────────────────────────────

    /// Directory completed inbound transfers are written to.
    ///
    /// Resolution order, first one that can actually be created wins:
    ///   1. an explicit root set with [`FileTransferEngine::with_download_root`]
    ///   2. `settings.default_download_folder` (re-read every call)
    ///   3. `dirs::download_dir()/Conduit`
    ///   4. `<app data>/Downloads`
    ///
    /// The previous implementation fell back to `PathBuf::from(".")` when
    /// `dirs::download_dir()` returned `None` (headless Linux, some CI images),
    /// which put received files in the *process working directory* — for a
    /// double-clicked bundle that is next to the executable. It also discarded
    /// the `create_dir_all` error, so a failed mkdir only surfaced much later
    /// as a confusing "file not found" during finalize.
    pub async fn resolve_download_root(&self) -> Result<PathBuf, ConduitError> {
        let mut candidates: Vec<PathBuf> = Vec::new();

        if let Some(explicit) = self.download_root_override.read().await.clone() {
            candidates.push(explicit);
        }

        if let Some(storage) = self.storage.as_ref() {
            match storage.get_setting(DOWNLOAD_FOLDER_SETTING).await {
                Some(raw) if !raw.trim().is_empty() => candidates.push(PathBuf::from(raw.trim())),
                _ => {}
            }
        }

        if let Some(downloads) = dirs::download_dir() {
            candidates.push(downloads.join(DOWNLOAD_SUBDIR));
        }

        // Deliberately NOT `PathBuf::from(".")`.
        candidates.push(app_data_root().join("Downloads"));

        let mut failures: Vec<String> = Vec::new();
        for candidate in candidates {
            match std::fs::create_dir_all(&candidate) {
                Ok(()) => {
                    let resolved =
                        canonicalize_normalised(&candidate).unwrap_or_else(|_| candidate.clone());
                    info!("File download root: {}", resolved.display());
                    return Ok(resolved);
                }
                Err(e) => failures.push(format!("{} ({e})", candidate.display())),
            }
        }

        Err(ConduitError::Storage(format!(
            "Could not create a download directory; tried: {}",
            failures.join(", ")
        )))
    }

    /// Human-readable download root, for the UI. Falls back to the resolved
    /// chain's last candidate when nothing can be created yet.
    pub async fn get_downloads_path(&self) -> String {
        match self.resolve_download_root().await {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                warn!("Download root unavailable: {e}");
                app_data_root()
                    .join("Downloads")
                    .to_string_lossy()
                    .to_string()
            }
        }
    }

    /// Root under which chunk files are staged.
    pub fn chunk_temp_root(&self) -> &Path {
        &self.temp_dir
    }

    // ── path authorisation ───────────────────────────────────────────────────

    /// Reject path strings that must never be handed to the OS.
    ///
    /// `open()`-style APIs accept any URI scheme. A `file/complete` frame can
    /// carry `\\attacker\share\x`, which on Windows makes the shell make an
    /// outbound SMB connection and leak the machine's NTLM hash.
    fn reject_unsafe_path_string(candidate: &str) -> Result<(), ConduitError> {
        let lower = candidate.trim().to_ascii_lowercase();
        let windows_form = lower.replace('/', "\\");

        // `\\?\C:\...` and `\\.\C:\...` are the verbatim (extended-length)
        // spellings `std::fs::canonicalize` produces on Windows. They are
        // local, and rejecting them would break every path this module
        // produces. `\\?\UNC\server\share` is a network share in disguise.
        let is_verbatim =
            windows_form.starts_with("\\\\?\\") || windows_form.starts_with("\\\\.\\");
        if windows_form.starts_with("\\\\?\\unc\\")
            || windows_form.starts_with("\\\\.\\unc\\")
            || (!is_verbatim && (windows_form.starts_with("\\\\") || lower.starts_with("//")))
        {
            return Err(ConduitError::Validation(
                "Refusing to use a network/UNC path; only local files are allowed".into(),
            ));
        }
        if is_verbatim {
            return Ok(());
        }

        // A URI scheme is two or more characters before a colon. `c:\...` must
        // survive: a single letter before the colon is a Windows drive letter.
        if let Some(colon) = lower.find(':') {
            let scheme = &lower[..colon];
            if scheme.len() >= 2
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            {
                return Err(ConduitError::Validation(
                    "Refusing to use a URI; only local files are allowed".into(),
                ));
            }
        }

        Ok(())
    }

    /// Resolve `candidate` to a real regular file that is provably inside
    /// `root`.
    ///
    /// This is the entire authorisation boundary for "open a file this device
    /// received". Checks, in order:
    ///   1. the string is a local path — no URI scheme, no UNC share
    ///   2. it contains no `..` component
    ///   3. `canonicalize` resolves every symlink, `..` and Windows 8.3 alias
    ///   4. the resolved path is a *component-wise* descendant of the resolved
    ///      root (a sibling like `<root>_evil` does not match)
    ///   5. the resolved path is a regular file
    pub fn validate_download_path(root: &Path, candidate: &str) -> Result<PathBuf, ConduitError> {
        Self::reject_unsafe_path_string(candidate)?;

        let raw = PathBuf::from(candidate);
        if raw.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(ConduitError::Validation(
                "Refusing to use a path containing '..'".into(),
            ));
        }

        let root_resolved = canonicalize_normalised(root)
            .map_err(|e| ConduitError::Storage(format!("Download root is unusable: {e}")))?;

        let resolved = canonicalize_normalised(&raw)
            .map_err(|e| ConduitError::Storage(format!("Failed to resolve file: {e}")))?;

        if !resolved.starts_with(&root_resolved) {
            return Err(ConduitError::Validation(format!(
                "Refusing to use a file outside the download folder: {}",
                resolved.display()
            )));
        }

        let meta = std::fs::metadata(&resolved)
            .map_err(|e| ConduitError::Storage(format!("Failed to stat file: {e}")))?;
        if !meta.is_file() {
            return Err(ConduitError::Validation(
                "Refusing to use something that is not a regular file".into(),
            ));
        }

        Ok(resolved)
    }

    /// Command used to hand a validated local file to the OS default handler.
    ///
    /// Windows uses `explorer.exe` rather than `cmd /C start` so a filename
    /// containing `&`, `|` or `^` is passed as a single argument and can never
    /// be re-read as shell syntax. `tauri-plugin-shell`'s `open()` is
    /// deprecated since 2.1 in favour of `tauri-plugin-opener`, which is not a
    /// dependency here — and a JS-reachable `shell:allow-open` is exactly the
    /// capability that must not exist, because a paired device chooses the
    /// path.
    pub fn open_command(path: &Path) -> (String, Vec<String>) {
        let p = path.to_string_lossy().to_string();
        if cfg!(target_os = "windows") {
            ("explorer.exe".to_string(), vec![p])
        } else if cfg!(target_os = "macos") {
            ("/usr/bin/open".to_string(), vec![p])
        } else {
            ("xdg-open".to_string(), vec![p])
        }
    }

    /// Hand a file that has already passed [`Self::validate_download_path`] to
    /// the operating system. The child is detached; its exit status is not
    /// observed because "the user closed the viewer" is indistinguishable from
    /// "the viewer failed" long after the fact.
    pub fn open_local_file(path: &Path) -> Result<(), ConduitError> {
        use std::process::{Command, Stdio};
        let (program, args) = Self::open_command(path);
        Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| ConduitError::Other(format!("Failed to open {}: {e}", path.display())))
    }

    // ── send allowlist ───────────────────────────────────────────────────────

    /// Record a path the user picked in the native file dialog (or dropped onto
    /// the window) as sendable.
    ///
    /// Canonicalised on the way in, so a symlink handed to the picker cannot
    /// smuggle a different target past [`Self::start_outgoing`]. Symlinks are
    /// refused outright: the user means "this document", not "whatever this
    /// link currently resolves to".
    pub async fn approve_send_path(&self, file_path: &str) -> Result<PathBuf, ConduitError> {
        Self::reject_unsafe_path_string(file_path)?;

        let meta = tokio::fs::symlink_metadata(file_path)
            .await
            .map_err(|e| ConduitError::Storage(format!("Failed to read file metadata: {e}")))?;
        if meta.file_type().is_symlink() {
            return Err(ConduitError::Validation(
                "Refusing to send a symbolic link; choose the file it points at instead".into(),
            ));
        }
        if !meta.is_file() {
            return Err(ConduitError::Validation(
                "Refusing to send something that is not a regular file".into(),
            ));
        }

        let canonical = tokio::fs::canonicalize(file_path)
            .await
            .map(strip_verbatim_prefix)
            .map_err(|e| ConduitError::Storage(format!("Failed to read file metadata: {e}")))?;

        let mut allowlist = self.send_allowlist.write().await;
        if allowlist.len() >= MAX_SEND_ALLOWLIST_ENTRIES {
            warn!("Send allowlist reached {MAX_SEND_ALLOWLIST_ENTRIES} entries; resetting it");
            allowlist.clear();
        }
        allowlist.insert(canonical.clone());
        Ok(canonical)
    }

    /// Canonicalise and authorise a file the user asked us to send.
    async fn validate_send_path(&self, file_path: &str) -> Result<PathBuf, ConduitError> {
        Self::reject_unsafe_path_string(file_path)?;

        let raw = PathBuf::from(file_path);
        if raw.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(ConduitError::Validation(
                "Refusing to send a path containing '..'".into(),
            ));
        }

        let link_meta = tokio::fs::symlink_metadata(&raw)
            .await
            .map_err(|e| ConduitError::Storage(format!("Failed to read file metadata: {e}")))?;
        if link_meta.file_type().is_symlink() {
            return Err(ConduitError::Validation(
                "Refusing to send a symbolic link; choose the file it points at instead".into(),
            ));
        }
        if !link_meta.is_file() {
            return Err(ConduitError::Validation(
                "Refusing to send something that is not a regular file".into(),
            ));
        }
        if link_meta.len() > MAX_SEND_SIZE {
            return Err(ConduitError::Validation(format!(
                "File is {} bytes, over the {} byte send limit",
                link_meta.len(),
                MAX_SEND_SIZE
            )));
        }

        let canonical = tokio::fs::canonicalize(&raw)
            .await
            .map(strip_verbatim_prefix)
            .map_err(|e| ConduitError::Storage(format!("Failed to read file metadata: {e}")))?;

        if self.send_allowlist.read().await.contains(&canonical) {
            return Ok(canonical);
        }

        if let Ok(root) = self.resolve_download_root().await
            && canonical.starts_with(&root)
        {
            return Ok(canonical);
        }

        Err(ConduitError::Validation(
            "This file has not been approved for sending. Pick it with the file dialog first."
                .into(),
        ))
    }

    // ── inbound ──────────────────────────────────────────────────────────────

    pub async fn start_incoming(
        &self,
        id: &str,
        name: &str,
        size: u64,
        mime: &str,
        from_device: &str,
        checksum: Option<String>,
    ) {
        // Cheap, rate-limited janitor tick: one inbound transfer arriving
        // flushes the temp directory of every transfer that was abandoned.
        self.cleanup_stale_incoming(INCOMING_TTL).await;

        let safe_id = sanitize_filename(id);
        let total_chunks = (size as usize).div_ceil(CHUNK_SIZE) as u32;
        let transfer_dir = self.temp_dir.join(safe_id);
        let td = transfer_dir.clone();
        // A `file/request` starts a *fresh* transfer. The staging directory is
        // keyed only by transfer id and survives process restarts, so without
        // this a peer that reuses an id (or a second run of the test suite)
        // would inherit chunks from a previous attempt — which, with the
        // duplicate-index short-circuit in `receive_chunk_binary`, would leave
        // `received` permanently short of `total_chunks` and the transfer
        // would never finalize. `resume_incoming` is the path that keeps them.
        tokio::fs::remove_dir_all(&td).await.ok();
        if let Err(e) = tokio::fs::create_dir_all(&td).await {
            warn!("Could not create chunk directory {td:?}: {e}");
        }

        if size > MAX_TRANSFER_BYTES {
            warn!(
                "Incoming transfer {id} declares {size} bytes, over the {} byte ceiling; \
                 chunks will be refused after {} bytes",
                MAX_TRANSFER_BYTES, MAX_TRANSFER_BYTES
            );
        }

        let now = Instant::now();
        let transfer = IncomingTransfer {
            name: name.to_string(),
            size,
            mime: mime.to_string(),
            from_device: from_device.to_string(),
            total_chunks,
            received: 0,
            received_bytes: 0,
            temp_dir: transfer_dir,
            expected_checksum: checksum,
            created_at: now,
            last_activity: now,
        };
        self.incoming.write().await.insert(id.to_string(), transfer);
        info!(
            "Incoming file transfer started: {} ({} chunks, declared {} bytes)",
            name, total_chunks, size
        );
    }

    pub async fn receive_chunk(
        &self,
        id: &str,
        index: u32,
        data_b64: &str,
    ) -> Result<u32, ConduitError> {
        let data = B64
            .decode(data_b64)
            .map_err(|e| ConduitError::Protocol(format!("Invalid base64: {}", e)))?;
        self.receive_chunk_binary(id, index, &data).await
    }

    /// Write one chunk, charging it against the transfer's byte budget.
    ///
    /// The declared `size` is a *claim*. Three independent checks stop a
    /// sender from turning it into a disk-fill primitive:
    ///   * the chunk index must be inside the declared chunk range, so an
    ///     attacker cannot scatter writes at arbitrary `chunk_N` offsets;
    ///   * a transfer declared as 0 bytes accepts no payload at all (the
    ///     protocol defaults a missing `size` to 0, which used to make
    ///     `is_complete()` vacuously true and write an attacker-named file
    ///     from the very first chunk);
    ///   * the running total may not exceed the declared size nor
    ///     [`MAX_TRANSFER_BYTES`]. Either breach aborts the transfer and
    ///     deletes its chunk directory.
    pub async fn receive_chunk_binary(
        &self,
        id: &str,
        index: u32,
        data: &[u8],
    ) -> Result<u32, ConduitError> {
        let mut incoming = self.incoming.write().await;
        let transfer = incoming
            .get_mut(id)
            .ok_or_else(|| ConduitError::Other(format!("Unknown transfer: {}", id)))?;

        let budget = transfer.size.min(MAX_TRANSFER_BYTES);
        if budget == 0 {
            return Err(ConduitError::Protocol(format!(
                "Transfer {id} declared 0 bytes; refusing to write a payload chunk"
            )));
        }

        if index >= transfer.total_chunks {
            return Err(ConduitError::Protocol(format!(
                "Chunk index {index} is outside the declared range 0..{} for transfer {id}",
                transfer.total_chunks
            )));
        }

        // A single frame can never be larger than the whole declared transfer.
        // Checked before the duplicate short-circuit below so an oversized
        // frame is always refused, even on a replayed index.
        if (data.len() as u64) > budget {
            return Err(self
                .abort_over_budget(&mut incoming, id, data.len() as u64)
                .await);
        }

        let chunk_path = transfer.temp_dir.join(format!("chunk_{:08}", index));
        let already_written = tokio::fs::metadata(&chunk_path).await.is_ok();

        if !already_written {
            let projected = transfer.received_bytes.saturating_add(data.len() as u64);
            if projected > budget {
                return Err(self.abort_over_budget(&mut incoming, id, projected).await);
            }

            // Write chunk to disk ONLY — never store in memory to avoid OOM on
            // large files.
            tokio::fs::write(&chunk_path, data).await.map_err(|e| {
                ConduitError::Storage(format!("Failed to write chunk {} to disk: {}", index, e))
            })?;

            transfer.received_bytes = projected;
            transfer.received += 1;
        }

        transfer.last_activity = Instant::now();
        Ok(transfer.received)
    }

    /// Drop a transfer that went over its byte budget, delete its chunk
    /// directory, and build the error to return to the peer.
    ///
    /// The directory is removed rather than left behind: a peer that declared
    /// 1 KB and kept streaming would otherwise leave the buffer on disk for
    /// the lifetime of the process, since only `finalize_incoming` and
    /// `cancel_incoming` clean up otherwise.
    async fn abort_over_budget(
        &self,
        incoming: &mut HashMap<String, IncomingTransfer>,
        id: &str,
        received: u64,
    ) -> ConduitError {
        let Some(transfer) = incoming.remove(id) else {
            return ConduitError::Other(format!("Unknown transfer: {id}"));
        };
        let temp_dir = transfer.temp_dir.clone();
        tokio::fs::remove_dir_all(&temp_dir).await.ok();
        warn!(
            "Aborting transfer {id} ('{}' from {}): {received} bytes received exceeds the {} \
             byte budget (declared {}); temp files removed",
            transfer.name,
            transfer.from_device,
            transfer.size.min(MAX_TRANSFER_BYTES),
            transfer.size,
        );
        ConduitError::Protocol(format!(
            "Transfer {id} exceeded its declared size of {} bytes ({received} received); aborted",
            transfer.size
        ))
    }

    /// Resume an incoming transfer by loading persisted chunks from disk
    pub async fn resume_incoming(
        &self,
        id: &str,
        name: &str,
        size: u64,
        mime: &str,
        from_device: &str,
        checksum: Option<String>,
    ) -> Result<u32, ConduitError> {
        self.cleanup_stale_incoming(INCOMING_TTL).await;

        let safe_id = sanitize_filename(id);
        let total_chunks = (size as usize).div_ceil(CHUNK_SIZE) as u32;
        let transfer_dir = self.temp_dir.join(safe_id);
        let td = transfer_dir.clone();
        if let Err(e) = tokio::fs::create_dir_all(&td).await {
            warn!("Could not create chunk directory {td:?}: {e}");
        }

        // Count how many chunk files already exist on disk, and re-charge the
        // byte budget for them: a resumed transfer that already holds more
        // bytes than it declared must not be allowed to accept more.
        let mut received = 0u32;
        let mut received_bytes = 0u64;
        for i in 0..total_chunks {
            let chunk_path = transfer_dir.join(format!("chunk_{:08}", i));
            if let Ok(md) = std::fs::metadata(&chunk_path) {
                received += 1;
                received_bytes = received_bytes.saturating_add(md.len());
            }
        }

        let now = Instant::now();
        let transfer = IncomingTransfer {
            name: name.to_string(),
            size,
            mime: mime.to_string(),
            from_device: from_device.to_string(),
            total_chunks,
            received,
            received_bytes,
            temp_dir: transfer_dir,
            expected_checksum: checksum,
            created_at: now,
            last_activity: now,
        };
        self.incoming.write().await.insert(id.to_string(), transfer);
        info!(
            "Resumed incoming transfer: {} ({} of {} chunks on disk, {} bytes)",
            name, received, total_chunks, received_bytes
        );
        Ok(received)
    }

    pub async fn is_complete(&self, id: &str) -> bool {
        let incoming = self.incoming.read().await;
        if let Some(transfer) = incoming.get(id) {
            transfer.received >= transfer.total_chunks
        } else {
            false
        }
    }

    /// Bytes buffered on disk for an inbound transfer (0 when unknown).
    pub async fn received_bytes(&self, id: &str) -> u64 {
        self.incoming
            .read()
            .await
            .get(id)
            .map(|t| t.received_bytes)
            .unwrap_or(0)
    }

    pub async fn finalize_incoming(&self, id: &str) -> Result<String, ConduitError> {
        // Resolved per transfer, not cached at construction: the user can
        // change `default_download_folder` while the app is running.
        let downloads_dir = self.resolve_download_root().await?;

        let mut incoming = self.incoming.write().await;
        let transfer = incoming
            .remove(id)
            .ok_or_else(|| ConduitError::Other(format!("Unknown transfer: {}", id)))?;

        // Assemble file from disk-only chunks (no in-memory chunks map).
        // Using a BufWriter + sequential reads avoids double-buffering the whole file.
        let safe_name = sanitize_filename(&transfer.name);
        if safe_name.is_empty() {
            return Err(ConduitError::Protocol(
                "Refusing to save a file with an empty name".into(),
            ));
        }

        // Keep the name inside the OS budget from the very first attempt, not
        // only when a collision forces a suffix: a sender-chosen name can be
        // arbitrarily long, and a single component over 255 characters (or a
        // full path over MAX_PATH) is refused by the OS outright.
        let safe_name = truncate_file_name(&safe_name, MAX_SAVED_NAME_LEN);

        // Pick a name that does not exist yet.
        //
        // The stem and extension are taken from `safe_name` *once*. Deriving
        // them from the previous candidate instead compounds the suffix
        // (`a.bin` → `a (1).bin` → `a (1) (1).bin` → …) and after a few dozen
        // repeat transfers of the same filename the path is longer than the
        // 260-character Windows MAX_PATH budget, at which point
        // `File::create` fails with os error 123 and *every* subsequent
        // transfer with that name is lost.
        let base_stem = PathBuf::from(&safe_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        let ext = PathBuf::from(&safe_name)
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();

        let mut save_path = downloads_dir.join(&safe_name);
        let mut counter: u32 = 1;
        while save_path.exists() {
            if counter > MAX_NAME_COLLISIONS {
                return Err(ConduitError::Storage(format!(
                    "Refusing to save '{safe_name}': {MAX_NAME_COLLISIONS} name variants already \
                     exist in {}",
                    downloads_dir.display()
                )));
            }
            let candidate =
                truncate_file_name(&format!("{base_stem} ({counter}){ext}"), MAX_SAVED_NAME_LEN);
            save_path = downloads_dir.join(candidate);
            counter += 1;
        }

        let total_chunks = transfer.total_chunks;
        let expected_checksum = transfer.expected_checksum.clone();
        let _transfer_name = transfer.name.clone();
        let transfer_size = transfer.size;
        let temp_dir = transfer.temp_dir.clone();

        let path_str = tokio::task::spawn_blocking(move || -> Result<String, ConduitError> {
            use std::io::Write;
            let out_file = std::fs::File::create(&save_path).map_err(|e| {
                ConduitError::Storage(format!("Failed to create output file: {}", e))
            })?;
            let mut writer = std::io::BufWriter::new(out_file);
            let mut hasher = Sha256::new();
            let mut written: u64 = 0;

            for i in 0..total_chunks {
                let chunk_path = temp_dir.join(format!("chunk_{:08}", i));
                let chunk_data = std::fs::read(&chunk_path).map_err(|_| {
                    ConduitError::Storage(format!("Missing chunk file for chunk {}", i))
                })?;
                hasher.update(&chunk_data);
                writer.write_all(&chunk_data).map_err(|e| {
                    ConduitError::Storage(format!("Write error at chunk {}: {}", i, e))
                })?;
                written = written.saturating_add(chunk_data.len() as u64);
            }
            writer
                .flush()
                .map_err(|e| ConduitError::Storage(format!("Flush error: {}", e)))?;

            // The assembled file must match the size the sender declared.
            // `receive_chunk_binary` bounds the budget, so this only fires
            // for a resumed transfer whose on-disk chunks disagree.
            if written != transfer_size {
                let _ = std::fs::remove_file(&save_path);
                return Err(ConduitError::Storage(format!(
                    "Assembled {written} bytes but the transfer declared {transfer_size}"
                )));
            }

            if let Some(ref expected) = expected_checksum {
                let actual = hex::encode(hasher.finalize());
                if actual != *expected {
                    let _ = std::fs::remove_file(&save_path);
                    return Err(ConduitError::Storage(format!(
                        "File integrity check failed: checksum mismatch (expected {})",
                        expected
                    )));
                }
            }

            let _ = std::fs::remove_dir_all(&temp_dir);
            Ok(save_path.to_string_lossy().to_string())
        })
        .await
        .map_err(|e| ConduitError::Other(format!("Task join error: {}", e)))??;

        // Remember where it landed. Local-only record; the durable copy is the
        // `file_transfers` table.
        {
            let mut completed = self.completed.write().await;
            if completed.len() >= MAX_COMPLETED_ENTRIES {
                warn!(
                    "Completed-transfer registry reached {MAX_COMPLETED_ENTRIES} entries; \
                     resetting it"
                );
                completed.clear();
            }
            let resolved = canonicalize_normalised(Path::new(&path_str))
                .unwrap_or_else(|_| PathBuf::from(&path_str));
            completed.insert(id.to_string(), resolved);
        }

        info!("File saved: {} ({} bytes)", path_str, transfer_size);
        Ok(path_str)
    }

    /// Absolute path a completed transfer was written to, if this process is
    /// the one that wrote it.
    pub async fn completed_path(&self, id: &str) -> Option<PathBuf> {
        self.completed.read().await.get(id).cloned()
    }

    pub async fn cancel_incoming(&self, id: &str) {
        if let Some(transfer) = self.incoming.write().await.remove(id) {
            let temp_dir = transfer.temp_dir.clone();
            tokio::fs::remove_dir_all(&temp_dir).await.ok();
            info!("Cancelled incoming transfer: {} (temp files cleaned)", id);
        }
    }

    // ── outbound ─────────────────────────────────────────────────────────────

    pub async fn start_outgoing(
        &self,
        id: &str,
        file_path: &str,
        to_device: &str,
    ) -> Result<(u64, String, String, u32, Option<String>), ConduitError> {
        self.cleanup_stale_outgoing(OUTGOING_TTL).await;

        // Authorise before touching the file: the caller's string is not
        // trusted until it is canonicalised and proven to be either inside the
        // download root or explicitly picked by the user.
        let path = self.validate_send_path(file_path).await?;

        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ConduitError::Storage(format!("Failed to read file metadata: {}", e)))?;
        let size = metadata.len();
        if size > MAX_SEND_SIZE {
            return Err(ConduitError::Validation(format!(
                "File is {size} bytes, over the {MAX_SEND_SIZE} byte send limit"
            )));
        }

        let checksum_path = path.clone();
        let checksum = tokio::task::spawn_blocking(move || -> Result<String, ConduitError> {
            use std::io::Read;
            let mut hasher = Sha256::new();
            let mut file = std::fs::File::open(&checksum_path).map_err(|e| {
                ConduitError::Storage(format!("Failed to open file for checksum: {}", e))
            })?;
            let mut buf = [0u8; 8192];
            loop {
                let n = file.read(&mut buf).map_err(|e| {
                    ConduitError::Storage(format!("Read error during checksum: {}", e))
                })?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        })
        .await
        .map_err(|e| ConduitError::Other(format!("Task join error: {}", e)))??;

        let mime = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        let total_chunks = (size as usize).div_ceil(CHUNK_SIZE) as u32;

        let now = Instant::now();
        let transfer = OutgoingTransfer {
            name: name.clone(),
            path,
            size,
            mime: mime.clone(),
            to_device: to_device.to_string(),
            total_chunks,
            next_chunk: 0,
            accepted: false,
            created_at: now,
            last_activity: now,
        };
        self.outgoing.write().await.insert(id.to_string(), transfer);
        info!(
            "Outgoing file transfer started: {} → {} ({} chunks, checksum: {})",
            name,
            to_device,
            total_chunks,
            &checksum[..16]
        );

        Ok((size, name, mime, total_chunks, Some(checksum)))
    }

    pub async fn get_next_chunk(&self, id: &str) -> Option<(u32, u32, String)> {
        if let Some((idx, total, data)) = self.get_next_chunk_binary(id).await {
            Some((idx, total, B64.encode(&data)))
        } else {
            None
        }
    }

    pub async fn get_next_chunk_binary(&self, id: &str) -> Option<(u32, u32, Vec<u8>)> {
        let mut outgoing = self.outgoing.write().await;
        let transfer = outgoing.get_mut(id)?;

        if transfer.next_chunk >= transfer.total_chunks {
            return None;
        }

        let idx = transfer.next_chunk;
        let start = (idx as usize) * CHUNK_SIZE;
        let end = std::cmp::min(start + CHUNK_SIZE, transfer.size as usize);
        let chunk_len = end - start;
        let file_path = transfer.path.clone();
        let total_chunks = transfer.total_chunks;

        transfer.next_chunk += 1;
        transfer.last_activity = Instant::now();
        drop(outgoing);

        let chunk_data = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(&file_path).ok()?;
            file.seek(SeekFrom::Start(start as u64)).ok()?;
            let mut chunk_data = vec![0u8; chunk_len];
            file.read_exact(&mut chunk_data).ok()?;
            Some(chunk_data)
        })
        .await
        .ok()??;

        Some((idx, total_chunks, chunk_data))
    }

    pub async fn remove_outgoing(&self, id: &str) {
        self.outgoing.write().await.remove(id);
    }

    pub async fn accept_outgoing(&self, id: &str) {
        if let Some(transfer) = self.outgoing.write().await.get_mut(id) {
            transfer.accepted = true;
            transfer.last_activity = Instant::now();
        }
    }

    pub async fn is_outgoing_accepted(&self, id: &str) -> bool {
        self.outgoing
            .read()
            .await
            .get(id)
            .is_some_and(|t| t.accepted)
    }

    // ── janitors ─────────────────────────────────────────────────────────────

    /// Drop inbound transfers that have gone quiet and delete their chunk
    /// directories.
    ///
    /// `finalize_incoming` and `cancel_incoming` are the only other paths that
    /// remove chunks, so a peer that opens a transfer and then disappears —
    /// or a device switched off mid-transfer — would otherwise leave gigabytes
    /// in the temp directory for the lifetime of the process.
    pub async fn cleanup_stale_incoming(&self, ttl: Duration) -> usize {
        let mut incoming = self.incoming.write().await;
        let stale: Vec<String> = incoming
            .iter()
            .filter(|(_, t)| t.last_activity.elapsed() > ttl)
            .map(|(id, _)| id.clone())
            .collect();

        let mut removed = 0usize;
        for id in stale {
            if let Some(transfer) = incoming.remove(&id) {
                let temp_dir = transfer.temp_dir.clone();
                info!(
                    "Janitor: dropping stale incoming transfer {id} ('{}', {}, {} bytes, \
                     declared {}, received {} bytes, from {}, age {:?})",
                    transfer.name,
                    transfer.mime,
                    transfer.total_chunks,
                    transfer.size,
                    transfer.received_bytes,
                    transfer.from_device,
                    transfer.created_at.elapsed(),
                );
                tokio::fs::remove_dir_all(&temp_dir).await.ok();
                removed += 1;
            }
        }
        removed
    }

    /// Drop outbound transfers that have gone quiet.
    pub async fn cleanup_stale_outgoing(&self, ttl: Duration) -> usize {
        let mut outgoing = self.outgoing.write().await;
        let stale: Vec<String> = outgoing
            .iter()
            .filter(|(_, t)| t.last_activity.elapsed() > ttl)
            .map(|(id, _)| id.clone())
            .collect();

        let mut removed = 0usize;
        for id in stale {
            if let Some(transfer) = outgoing.remove(&id) {
                warn!(
                    "Janitor: dropping stale outgoing transfer {id} ('{}', {} bytes → {}, \
                     {}, mime {}, age {:?})",
                    transfer.name,
                    transfer.size,
                    transfer.to_device,
                    transfer.total_chunks,
                    transfer.mime,
                    transfer.created_at.elapsed(),
                );
                removed += 1;
            }
        }
        removed
    }
}

/// Per-app data directory, used as the last-resort download root.
fn app_data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("conduit")
}

/// Strip the Windows verbatim (`\\?\`) prefix that [`std::fs::canonicalize`]
/// adds to its result.
///
/// A verbatim path skips all path parsing, which cuts both ways: the Win32
/// `\\?\` form is *rejected* for SUBST drives (a real configuration — it is
/// how `%TEMP%` and the agent's home directory are often mapped in CI), and
/// `explorer.exe`, `cmd.exe` and the shell's file-association lookup do not
/// understand it either. Every path this module hands back to the OS goes
/// through here.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy().to_string();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

/// [`std::fs::canonicalize`], minus the Windows verbatim prefix.
fn canonicalize_normalised(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(strip_verbatim_prefix)
}

/// Shorten `name` to at most `max_chars`, keeping the extension.
fn truncate_file_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let as_path = PathBuf::from(name);
    let ext = as_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let ext_len = ext.chars().count();
    let keep = max_chars.saturating_sub(ext_len);
    let stem: String = as_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .chars()
        .take(keep)
        .collect();
    format!("{stem}{ext}")
}

fn sanitize_filename(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/'
            || c == '\\'
            || c == ':'
            || c == '*'
            || c == '?'
            || c == '"'
            || c == '<'
            || c == '>'
            || c == '|'
        {
            result.push('_');
            i += 1;
        } else if c == '.' {
            let start = i;
            while i < chars.len() && chars[i] == '.' {
                i += 1;
            }
            let run_len = i - start;
            let is_extension =
                run_len == 1 && chars.get(i).is_some_and(|next| next.is_alphanumeric());
            if is_extension {
                result.push('.');
            } else {
                result.push('_');
            }
        } else {
            result.push(c);
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("../../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("my file (1).png"), "my file (1).png");
    }

    #[test]
    fn sanitize_path_traversal_doubledots() {
        assert_eq!(sanitize_filename(".."), "_");
        assert_eq!(sanitize_filename("...."), "_");
    }

    #[test]
    fn sanitize_special_chars() {
        assert_eq!(sanitize_filename("a:b*c?d\"e<f>g|h"), "a_b_c_d_e_f_g_h");
    }

    #[test]
    fn sanitize_preserves_extension_dot() {
        assert_eq!(sanitize_filename("archive.tar.gz"), "archive.tar.gz");
        assert_eq!(sanitize_filename("noext"), "noext");
    }

    #[test]
    fn sanitize_leading_dots_not_extension() {
        assert_eq!(sanitize_filename(".hidden"), ".hidden");
        assert_eq!(sanitize_filename("..hidden"), "_hidden");
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_filename(""), "");
    }

    // --- FileTransferEngine unit tests ---
    /// Engine pinned to throwaway directories so no test ever writes into the
    /// developer's real Downloads folder, and so two tests cannot see each
    /// other's staged chunks.
    fn make_engine() -> (TempDir, FileTransferEngine) {
        let dir = tempfile::tempdir().expect("tempdir");
        let engine = FileTransferEngine::new()
            .with_download_root(dir.path().join("downloads"))
            .with_chunk_temp_dir(dir.path().join("chunks"));
        (dir, engine)
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_and_check_incoming() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming("t1", "test.txt", 128, "text/plain", "device_a", None)
            .await;
        assert!(!engine.is_complete("t1").await);
        assert!(!engine.is_complete("nonexistent").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receive_chunks_and_complete() {
        let (_dir, engine) = make_engine();
        // 2 full chunks: 128 bytes would fit in a *single* 64 KB chunk, so the
        // old two-chunk fixture only ever exercised `received > total`.
        let size = (CHUNK_SIZE * 2) as u64;
        engine
            .start_incoming("t2", "f.bin", size, "application/octet-stream", "dev", None)
            .await;
        let chunk = b64(&[1u8; CHUNK_SIZE]);
        let count = engine.receive_chunk("t2", 0, &chunk).await.unwrap();
        assert_eq!(count, 1);
        let count = engine.receive_chunk("t2", 1, &chunk).await.unwrap();
        assert_eq!(count, 2);
        assert_eq!(engine.received_bytes("t2").await, size);
        assert!(engine.is_complete("t2").await);

        let path = engine.finalize_incoming("t2").await.unwrap();
        assert!(Path::new(&path).exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receive_chunk_invalid_base64() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming("t3", "f.bin", 64, "application/octet-stream", "dev", None)
            .await;
        let result = engine.receive_chunk("t3", 0, "!!!not-base64!!!").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid base64"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receive_chunk_unknown_transfer() {
        let (_dir, engine) = make_engine();
        let result = engine.receive_chunk("nonexistent", 0, "AA==").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown transfer"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finalize_incomplete_fails() {
        let (_dir, engine) = make_engine();
        // 200KB needs 4 chunks at 64KB each; send only 1
        engine
            .start_incoming(
                "t4",
                "f.bin",
                200_000,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        let chunk = b64(&[0u8; 64]);
        engine.receive_chunk("t4", 0, &chunk).await.unwrap();
        let result = engine.finalize_incoming("t4").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing chunk"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finalize_unknown_transfer() {
        let (_dir, engine) = make_engine();
        let result = engine.finalize_incoming("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown transfer"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_incoming_cleans_up() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming("t5", "f.bin", 64, "application/octet-stream", "dev", None)
            .await;
        engine.cancel_incoming("t5").await;
        assert!(!engine.is_complete("t5").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_nonexistent_is_noop() {
        let (_dir, engine) = make_engine();
        engine.cancel_incoming("ghost").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finalize_with_checksum_mismatch() {
        let (_dir, engine) = make_engine();
        let chunk = b64(&[0u8; 64]);
        engine
            .start_incoming(
                "t6",
                "f.bin",
                64,
                "application/octet-stream",
                "dev",
                Some(
                    "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                ),
            )
            .await;
        engine.receive_chunk("t6", 0, &chunk).await.unwrap();
        let result = engine.finalize_incoming("t6").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("checksum mismatch")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_downloads_path_returns_string() {
        let (dir, engine) = make_engine();
        let path = engine.get_downloads_path().await;
        assert!(!path.is_empty());
        let as_path = Path::new(&path);
        assert!(as_path.is_absolute(), "{path} must be absolute");
        // `resolve_download_root` canonicalises, so compare canonical forms.
        let expected = canonicalize_normalised(&dir.path().join("downloads")).unwrap();
        assert_eq!(as_path, expected);
    }

    // --- VULN 3: the declared size is a claim; the bytes are the truth ---

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receive_chunk_rejects_bytes_beyond_the_declared_size() {
        let (_dir, engine) = make_engine();
        // Declares 128 bytes, i.e. one chunk of budget.
        engine
            .start_incoming(
                "t_over",
                "greedy.bin",
                128,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;

        let first = engine.receive_chunk_binary("t_over", 0, &[0u8; 128]).await;
        assert!(first.is_ok(), "the first chunk fits the declared size");

        // Second chunk of the same transfer blows the budget: abort.
        let second = engine.receive_chunk_binary("t_over", 0, &[0u8; 4096]).await;
        assert!(second.is_err());
        assert!(second.unwrap_err().to_string().contains("exceeded"));

        // The transfer is gone and its chunk directory removed, so a peer
        // cannot keep streaming into a half-registered transfer.
        assert!(!engine.is_complete("t_over").await);
        let result = engine.receive_chunk_binary("t_over", 0, &[0u8; 8]).await;
        assert!(
            result.unwrap_err().to_string().contains("Unknown transfer"),
            "an aborted transfer must not accept further chunks"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborting_an_oversized_transfer_deletes_its_chunk_directory() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming(
                "t_abort",
                "f.bin",
                64,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        engine
            .receive_chunk_binary("t_abort", 0, &[0u8; 64])
            .await
            .unwrap();

        let transfer_dir = engine.chunk_temp_root().join("t_abort");
        assert!(transfer_dir.exists(), "chunk dir must exist mid-transfer");

        let err = engine
            .receive_chunk_binary("t_abort", 0, &[0u8; 65])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exceeded"));
        assert!(
            !transfer_dir.exists(),
            "an aborted transfer must not leave buffered chunks on disk"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receive_chunk_rejects_index_outside_the_declared_range() {
        let (_dir, engine) = make_engine();
        // 64 bytes → exactly one chunk, so index 1 does not exist.
        engine
            .start_incoming(
                "t_idx",
                "f.bin",
                64,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        let err = engine
            .receive_chunk_binary("t_idx", 1, &[0u8; 64])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("outside the declared range"),
            "unexpected error: {err}"
        );
    }

    /// VULN 9: `handlers/files.rs` defaults a missing `size` to 0, which makes
    /// `total_chunks == 0` and `is_complete()` vacuously true. A zero-length
    /// declaration must accept no payload at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receive_chunk_rejects_zero_declared_size() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming(
                "t_zero",
                "pwn.exe",
                0,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        let err = engine
            .receive_chunk_binary("t_zero", 0, &[0u8; 16])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("declared 0 bytes"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn duplicate_chunk_index_is_idempotent_and_not_charged_twice() {
        let (_dir, engine) = make_engine();
        let size = (CHUNK_SIZE * 2) as u64;
        engine
            .start_incoming(
                "t_dup",
                "f.bin",
                size,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        let chunk = b64(&[7u8; CHUNK_SIZE]);
        assert_eq!(engine.receive_chunk("t_dup", 0, &chunk).await.unwrap(), 1);
        // A peer replaying index 0 must not be able to grow the on-disk bytes
        // or the `received` count.
        assert_eq!(engine.receive_chunk("t_dup", 0, &chunk).await.unwrap(), 1);
        assert_eq!(engine.received_bytes("t_dup").await, size / 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn janitor_removes_stale_incoming_transfers_and_their_files() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming(
                "t_stale",
                "big.bin",
                1024,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        engine
            .receive_chunk_binary("t_stale", 0, &[0u8; 1024])
            .await
            .unwrap();
        let transfer_dir = engine.chunk_temp_root().join("t_stale");
        assert!(transfer_dir.exists());

        engine
            .start_incoming(
                "t_live",
                "live.bin",
                64,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;

        // A zero TTL makes every existing transfer stale; the newly started
        // one is only spared because `start_incoming` janitors *before*
        // inserting, which is exactly the ordering we want.
        let removed = engine.cleanup_stale_incoming(Duration::ZERO).await;

        assert_eq!(removed, 2, "both transfers are past a zero TTL");
        assert!(!engine.is_complete("t_stale").await);
        assert!(!engine.is_complete("t_live").await);
        assert!(
            !transfer_dir.exists(),
            "the janitor must delete the buffered chunk directory"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn janitor_keeps_recent_transfers() {
        let (_dir, engine) = make_engine();
        engine
            .start_incoming(
                "t_fresh",
                "f.bin",
                64,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        let removed = engine
            .cleanup_stale_incoming(Duration::from_secs(3600))
            .await;
        assert_eq!(removed, 0);
        assert!(!engine.is_complete("t_fresh").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn janitor_removes_stale_outgoing_transfers() {
        let (_dir, engine) = make_engine();
        let src = _dir.path().join("src.bin");
        std::fs::write(&src, vec![1u8; 32]).unwrap();
        engine
            .approve_send_path(&src.to_string_lossy())
            .await
            .unwrap();
        engine
            .start_outgoing("out_stale", &src.to_string_lossy(), "dev_b")
            .await
            .unwrap();

        assert_eq!(
            engine
                .cleanup_stale_outgoing(Duration::from_secs(3600))
                .await,
            0
        );
        assert!(engine.get_next_chunk("out_stale").await.is_some());
        assert_eq!(engine.cleanup_stale_outgoing(Duration::ZERO).await, 1);
        assert!(engine.get_next_chunk("out_stale").await.is_none());
    }

    #[test]
    fn reject_unsafe_path_string_allows_windows_verbatim_paths() {
        // `std::fs::canonicalize` on Windows returns `\\?\C:\...`. The whole
        // module round-trips through canonical paths, so rejecting the
        // verbatim spelling would make every received file unopenable.
        for candidate in [
            r"\\?\C:\Users\dev\Downloads\Conduit\a.bin",
            r"\\.\C:\Users\dev\Downloads\Conduit\a.bin",
        ] {
            FileTransferEngine::reject_unsafe_path_string(candidate)
                .unwrap_or_else(|e| panic!("{candidate} must be accepted: {e}"));
        }
    }

    #[test]
    fn reject_unsafe_path_string_rejects_unc_even_in_verbatim_form() {
        for candidate in [
            r"\\?\UNC\attacker\share\payload.exe",
            r"\\.\UNC\attacker\share\payload.exe",
            r"\\attacker\share\payload.exe",
        ] {
            let err = FileTransferEngine::reject_unsafe_path_string(candidate)
                .expect_err("a UNC share must be refused");
            assert!(err.to_string().contains("only local files are allowed"));
        }
    }

    #[test]
    fn reject_unsafe_path_string_rejects_uri_schemes() {
        for candidate in [
            "file:///etc/passwd",
            "https://example.com/x.exe",
            "smb://attacker/share/x",
        ] {
            let err = FileTransferEngine::reject_unsafe_path_string(candidate)
                .expect_err("a URI must be refused");
            assert!(err.to_string().contains("only local files are allowed"));
        }
    }

    #[test]
    fn reject_unsafe_path_string_allows_ordinary_paths() {
        for candidate in [
            "/home/dev/Downloads/a.bin",
            "C:\\Users\\dev\\Downloads\\a.bin",
            "relative/a.bin",
        ] {
            FileTransferEngine::reject_unsafe_path_string(candidate)
                .unwrap_or_else(|e| panic!("{candidate} must be accepted: {e}"));
        }
    }

    // --- VULN 4: the download root is never the process CWD ---

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_root_is_never_the_process_working_directory() {
        // No override, no storage: falls through to `dirs::download_dir()` or
        // the app-data directory. It must be absolute, and must never be
        // `.` / `./` / an empty path.
        let engine = FileTransferEngine::new();
        let root = engine
            .resolve_download_root()
            .await
            .expect("a root resolves");
        assert!(root.is_absolute(), "root must be absolute: {root:?}");
        assert_ne!(root, PathBuf::from("."));
        assert_ne!(root, PathBuf::from(""));
        assert!(root.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_root_override_is_used_and_created() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("nested").join("downloads");
        let engine = FileTransferEngine::new().with_download_root(wanted.clone());
        let root = engine.resolve_download_root().await.unwrap();
        assert_eq!(root, canonicalize_normalised(&wanted).unwrap());
        assert!(root.exists());
    }

    // --- VULN 1: path authorisation for "open a received file" ---

    #[test]
    fn validate_download_path_accepts_a_file_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("report.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        let resolved = FileTransferEngine::validate_download_path(root, &file.to_string_lossy())
            .expect("an in-root file is allowed");
        assert_eq!(resolved, canonicalize_normalised(&file).unwrap());
    }

    #[test]
    fn validate_download_path_accepts_a_nested_file_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("dl");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let file = root.join("sub").join("a.txt");
        std::fs::write(&file, b"hi").unwrap();
        assert!(FileTransferEngine::validate_download_path(&root, &file.to_string_lossy()).is_ok());
    }

    #[test]
    fn validate_download_path_rejects_a_path_outside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("dl");
        let outside_dir = dir.path().join("secret");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        let secret = outside_dir.join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        let err = FileTransferEngine::validate_download_path(&root, &secret.to_string_lossy())
            .expect_err("a file outside the root must be refused");
        assert!(err.to_string().contains("outside the download folder"));
    }

    #[test]
    fn validate_download_path_rejects_dot_dot_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("dl");
        let outside_dir = dir.path().join("secret");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("id_rsa"), b"PRIVATE KEY").unwrap();

        let traversal = root.join("..").join("secret").join("id_rsa");
        let err = FileTransferEngine::validate_download_path(&root, &traversal.to_string_lossy())
            .expect_err("'..' must be refused before canonicalisation");
        assert!(err.to_string().contains("'..'"));
    }

    #[test]
    fn validate_download_path_rejects_a_sibling_directory_with_a_shared_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("dl");
        let sibling = dir.path().join("dl_evil");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let file = sibling.join("payload.exe");
        std::fs::write(&file, b"x").unwrap();

        // `<root>_evil` is a plain string prefix of `<root>` but not a path
        // prefix: a naive `starts_with` on the *string* would let this through.
        let err = FileTransferEngine::validate_download_path(&root, &file.to_string_lossy())
            .expect_err("a shared string prefix must not count as containment");
        assert!(err.to_string().contains("outside the download folder"));
    }

    #[test]
    fn validate_download_path_rejects_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let err = FileTransferEngine::validate_download_path(root, &sub.to_string_lossy())
            .expect_err("a directory is not openable as a document");
        assert!(err.to_string().contains("not a regular file"));
    }

    /// Every path this module hands back to the OS goes through
    /// [`canonicalize_normalised`], because the Win32 verbatim form is refused
    /// for SUBST drives (`S:`) and is not understood by `explorer.exe` or the
    /// shell's file-association lookup. A received file simply failed to save
    /// with `os error 123` before this.
    #[test]
    fn canonical_paths_never_carry_a_windows_verbatim_prefix() {
        for candidate in [
            r"\\?\C:\Users\dev\Downloads\a.bin",
            r"\\?\S:\Conduit\a.bin",
            r"\\?\UNC\server\share\a.bin",
        ] {
            let stripped = strip_verbatim_prefix(PathBuf::from(candidate));
            let text = stripped.to_string_lossy().to_string();
            assert!(
                !text.starts_with(r"\\?\"),
                "{candidate} still carries a verbatim prefix: {text}"
            );
        }
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\C:\a.bin")),
            PathBuf::from(r"C:\a.bin")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\srv\share\a.bin")),
            PathBuf::from(r"\\srv\share\a.bin")
        );
        // Non-Windows (and already-plain Windows) paths are untouched.
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from("/home/dev/a.bin")),
            PathBuf::from("/home/dev/a.bin")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"C:\a.bin")),
            PathBuf::from(r"C:\a.bin")
        );
    }

    #[test]
    fn validate_download_path_rejects_unc_and_uri_strings() {
        let dir = tempfile::tempdir().unwrap();
        for candidate in [
            r"\\attacker\share\payload.exe",
            "//attacker/share/payload.exe",
            "file:///etc/passwd",
            "https://example.com/x.exe",
        ] {
            let err = FileTransferEngine::validate_download_path(dir.path(), candidate)
                .expect_err("non-local paths must be refused");
            assert!(
                err.to_string().contains("only local files are allowed"),
                "{candidate} produced: {err}"
            );
        }
    }

    #[test]
    fn validate_download_path_allows_a_windows_drive_letter() {
        // `C:\...` must not be mistaken for a URI scheme. On a real Windows
        // install this resolves; elsewhere the error is "failed to resolve",
        // never "URI refused".
        let err = FileTransferEngine::validate_download_path(
            Path::new("/"),
            r"C:\definitely\not\here\file.txt",
        );
        if let Err(e) = err {
            assert!(
                !e.to_string().contains("URI"),
                "a drive letter is not a URI scheme: {e}"
            );
        }
    }

    #[test]
    fn validate_download_path_rejects_a_symlink_that_escapes_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("dl");
        let outside_dir = dir.path().join("secret");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        let secret = outside_dir.join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        let link = root.join("innocent.txt");
        if let Err(reason) = symlink_file(&secret, &link) {
            // Windows refuses to create symlinks without Developer Mode or
            // elevation. The canonicalisation check above (sibling prefix +
            // outside-root) already covers the logic; note the gap loudly
            // rather than pretending the test passed.
            eprintln!(
                "SKIPPED symlink-escape assertion: cannot create symlinks here ({reason:?}). \
                 Enable Developer Mode to run this test."
            );
            return;
        }

        let err = FileTransferEngine::validate_download_path(&root, &link.to_string_lossy())
            .expect_err("a symlink resolving outside the root must be refused");
        assert!(err.to_string().contains("outside the download folder"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approve_send_path_refuses_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        std::fs::write(&real, b"data").unwrap();
        let link = dir.path().join("link.txt");
        if symlink_file(&real, &link).is_err() {
            eprintln!("SKIPPED: cannot create symlinks on this host");
            return;
        }
        let engine = FileTransferEngine::new();
        let err = engine
            .approve_send_path(&link.to_string_lossy())
            .await
            .expect_err("a symlink must not be sendable");
        assert!(err.to_string().contains("symbolic link"));
    }

    // --- VULN 2: send_file is not an arbitrary-file-read primitive ---

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_outgoing_refuses_a_file_that_was_never_picked() {
        let (dir, engine) = make_engine();
        let secret = dir.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        let err = engine
            .start_outgoing("o1", &secret.to_string_lossy(), "dev_b")
            .await
            .expect_err("an unpicked file must not be sendable");
        assert!(err.to_string().contains("not been approved for sending"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_outgoing_refuses_a_directory() {
        let (dir, engine) = make_engine();
        let sub = dir.path().join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let err = engine
            .start_outgoing("o2", &sub.to_string_lossy(), "dev_b")
            .await
            .expect_err("a directory is not sendable");
        assert!(err.to_string().contains("not a regular file"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_outgoing_refuses_dot_dot_traversal() {
        let (dir, engine) = make_engine();
        let downloads = dir.path().join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let secret = dir.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        let traversal = downloads.join("..").join("id_rsa");
        let err = engine
            .start_outgoing("o3", &traversal.to_string_lossy(), "dev_b")
            .await
            .expect_err("'..' must be refused");
        assert!(err.to_string().contains("'..'"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_outgoing_allows_a_picked_file_and_one_inside_the_download_root() {
        let (dir, engine) = make_engine();

        // (a) explicitly picked via the dialog
        let picked = dir.path().join("picked.txt");
        std::fs::write(&picked, b"hello").unwrap();
        engine
            .approve_send_path(&picked.to_string_lossy())
            .await
            .unwrap();
        let (size, name, _mime, total_chunks, _cs) = engine
            .start_outgoing("o_picked", &picked.to_string_lossy(), "dev_b")
            .await
            .expect("a picked file is sendable");
        assert_eq!(size, 5);
        assert_eq!(name, "picked.txt");
        assert_eq!(total_chunks, 1);

        // (b) anything already sitting in the download root
        let root = engine.resolve_download_root().await.unwrap();
        std::fs::write(root.join("received.txt"), b"world").unwrap();
        let (size, name, _, _, _) = engine
            .start_outgoing(
                "o_root",
                &root.join("received.txt").to_string_lossy(),
                "dev_b",
            )
            .await
            .expect("a file in the download root is sendable");
        assert_eq!(size, 5);
        assert_eq!(name, "received.txt");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_outgoing_refuses_a_file_over_the_send_limit() {
        let (dir, engine) = make_engine();
        let big = dir.path().join("huge.bin");
        let file = std::fs::File::create(&big).unwrap();
        if let Err(e) = file.set_len(MAX_SEND_SIZE + 1) {
            eprintln!("SKIPPED MAX_SEND_SIZE assertion: cannot size a sparse file ({e})");
            return;
        }
        drop(file);
        engine
            .approve_send_path(&big.to_string_lossy())
            .await
            .unwrap();

        let err = engine
            .start_outgoing("o_big", &big.to_string_lossy(), "dev_b")
            .await
            .expect_err("an over-limit file must not be sendable");
        assert!(err.to_string().contains("send limit"));
    }

    #[test]
    fn open_command_never_goes_through_a_shell() {
        let (program, args) = FileTransferEngine::open_command(Path::new("/tmp/a&b|c.txt"));
        assert_eq!(args.len(), 1, "the path must be a single argument");
        assert_eq!(args[0], "/tmp/a&b|c.txt");
        assert!(
            !program.contains("cmd") && !program.contains("sh -c"),
            "a shell would re-interpret '&' and '|' in a filename: {program}"
        );
    }

    #[test]
    fn truncate_file_name_keeps_the_extension() {
        assert_eq!(truncate_file_name("a.bin", 120), "a.bin");
        let long = format!("{}.bin", "x".repeat(400));
        let short = truncate_file_name(&long, MAX_SAVED_NAME_LEN);
        assert_eq!(short.chars().count(), MAX_SAVED_NAME_LEN);
        assert!(short.ends_with(".bin"), "extension must survive: {short}");
    }

    #[test]
    fn truncate_file_name_handles_a_name_with_no_extension() {
        let long = "x".repeat(400);
        let short = truncate_file_name(&long, MAX_SAVED_NAME_LEN);
        assert_eq!(short.chars().count(), MAX_SAVED_NAME_LEN);
        assert!(!short.contains('.'));
    }

    /// The "do not overwrite" loop used to re-derive the stem from the previous
    /// candidate, so a name grew `a.bin` → `a (1).bin` → `a (1) (1).bin` → … and
    /// eventually exceeded MAX_PATH, after which every transfer with that name
    /// failed with os error 123.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finalize_picks_a_bounded_non_compounding_name_on_collision() {
        // `_dir` must stay bound: it owns the download root these writes land in.
        let (_dir, engine) = make_engine();
        let root = engine.resolve_download_root().await.unwrap();

        for _ in 0..5 {
            engine
                .start_incoming(
                    "t_dup_name",
                    "report.pdf",
                    3,
                    "application/pdf",
                    "dev",
                    None,
                )
                .await;
            engine
                .receive_chunk_binary("t_dup_name", 0, b"abc")
                .await
                .unwrap();
            let saved = engine.finalize_incoming("t_dup_name").await.unwrap();
            let name = Path::new(&saved)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            assert!(
                name.chars().count() <= MAX_SAVED_NAME_LEN,
                "{name} exceeds the name budget"
            );
            assert!(
                Path::new(&saved).starts_with(&root),
                "{name} must land in the download root"
            );
        }

        let names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names.len(), 5, "got {names:?}");
        assert!(names.contains(&"report.pdf".to_string()));
        // The suffix is appended to the *original* stem, exactly once.
        assert!(names.contains(&"report (1).pdf".to_string()));
        assert!(names.contains(&"report (4).pdf".to_string()));
        assert!(
            !names.iter().any(|n| n.contains("(1) (")),
            "suffix must not compound: {names:?}"
        );
    }

    /// A very long declared name must still produce a file the OS accepts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finalize_survives_a_pathologically_long_declared_name() {
        // `_dir` must stay bound: it owns the download root this write lands in.
        let (_dir, engine) = make_engine();
        let long_name = format!("{}.bin", "n".repeat(400));
        engine
            .start_incoming(
                "t_long",
                &long_name,
                3,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;
        engine
            .receive_chunk_binary("t_long", 0, b"abc")
            .await
            .unwrap();
        let saved = engine.finalize_incoming("t_long").await.unwrap();
        let saved_path = Path::new(&saved);
        assert!(saved_path.exists());
        assert!(
            saved_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .chars()
                .count()
                <= MAX_SAVED_NAME_LEN,
            "the saved name must fit the OS component budget"
        );
    }

    // --- round trips that must keep working ---

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn outgoing_chunk_round_trip() {
        let (dir, engine) = make_engine();
        let file_path = dir.path().join("test_out.bin");
        let data = vec![42u8; 100];
        std::fs::write(&file_path, &data).unwrap();
        engine
            .approve_send_path(&file_path.to_string_lossy())
            .await
            .unwrap();

        let result = engine
            .start_outgoing("out1", &file_path.to_string_lossy(), "device_b")
            .await
            .unwrap();
        let (size, name, _mime, total_chunks, checksum) = result;
        assert_eq!(size, 100);
        assert_eq!(name, "test_out.bin");
        assert_eq!(total_chunks, 1); // 100 bytes < 64KB chunk
        assert!(checksum.is_some());

        let chunk = engine.get_next_chunk("out1").await.unwrap();
        assert_eq!(chunk.0, 0); // index
        assert_eq!(chunk.1, 1); // total
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&chunk.2)
            .unwrap();
        assert_eq!(decoded, data);

        // No more chunks
        assert!(engine.get_next_chunk("out1").await.is_none());

        engine.remove_outgoing("out1").await;
        assert!(engine.get_next_chunk("out1").await.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_next_chunk_unknown_transfer() {
        let (_dir, engine) = make_engine();
        assert!(engine.get_next_chunk("ghost").await.is_none());
    }

    /// Create a file symlink on whichever platform the test runs on.
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
    }
}
