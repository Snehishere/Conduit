use crate::AppState;
use crate::error::{ConduitError, Result};
use crate::file_transfer::FileTransferEngine;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

type ManagedState = Arc<AppState>;

/// How many transfer rows to scan when resolving a frame's peer device. The
/// table is ordered by timestamp descending and pruned by the UI at 50 rows, so
/// this is comfortably more than the UI can ever hold.
const PEER_LOOKUP_LIMIT: i64 = 200;

/// The pseudo device id the UI assigns to the local device for inbound
/// transfers. It is never a routable WS client, so it can never be a peer.
const LOCAL_DEVICE_ALIAS: &str = "local";

/// Work out which single peer a transfer frame belongs to.
///
/// Every `file/*` frame used to go out with `ws.broadcast`, which had two
/// effects:
///
///   * the desktop's own webview is a WS client on the desktop's own server, so
///     the device received its own outgoing transfer back as an inbound
///     `file/request` and rendered the user's own sent file as something they
///     had to accept;
///   * every 64 KB `file/chunk` was echoed to every client, so the webview that
///     produced the transfer JSON-parsed its own payload on every chunk.
///
/// Candidates come from the durable `file_transfers` row first and from the
/// caller second; the first candidate that is neither this device nor the
/// `local` alias wins. `None` means "do not send", never "broadcast".
async fn resolve_peer_device(
    state: &Arc<AppState>,
    id: &str,
    from_device: Option<String>,
    to_device: Option<String>,
) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();

    if let Ok(rows) = state.storage.get_file_transfers(PEER_LOOKUP_LIMIT).await
        && let Some(row) = rows.iter().find(|r| r.id == id)
    {
        candidates.push(row.from_device.clone());
        candidates.push(row.to_device.clone());
    }
    if let Some(from) = from_device {
        candidates.push(from);
    }
    if let Some(to) = to_device {
        candidates.push(to);
    }

    candidates
        .into_iter()
        .find(|c| !c.is_empty() && c != &state.device_id && c != LOCAL_DEVICE_ALIAS)
}

/// Deliver a frame to exactly one peer.
///
/// A missing WS server is a no-op (the app has not finished starting, and the
/// command tests have no server). A peer that is not currently connected is a
/// warning, not a failure: cancelling a transfer whose sender already left must
/// still update the local row.
async fn send_to_peer(state: &Arc<AppState>, peer: &str, message: &str) {
    let guard = state.ws_server.read().await;
    match guard.as_ref() {
        Some(ws) => {
            if !ws.send_to(peer, message.to_string()).await {
                log::warn!("Peer {peer} is not connected; frame not delivered");
            }
        }
        None => log::debug!("WebSocket server not running; frame not delivered"),
    }
}

/// Shared prelude for the accept/cancel commands: mark the local row, work out
/// the peer, and only ever send to that peer.
async fn apply_local_state_and_route(
    state: &Arc<AppState>,
    id: &str,
    status: &str,
    from_device: Option<String>,
    to_device: Option<String>,
) -> Result<Option<String>> {
    let storage = state.storage.clone();
    let id_c = id.to_string();
    let status_c = status.to_string();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current()
            .block_on(storage.update_file_transfer_progress(&id_c, &status_c, 0, None))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("{status} task join error: {e}")))??;

    let peer = resolve_peer_device(state, id, from_device, to_device).await;

    if peer.is_none() && state.ws_server.read().await.is_some() {
        return Err(ConduitError::Validation(format!(
            "Refusing to send a '{status}' frame for transfer {id}: the peer device is unknown, \
             and broadcasting it would deliver the frame to this device as well"
        )));
    }

    Ok(peer)
}

#[tauri::command]
pub async fn send_file(
    state: State<'_, ManagedState>,
    target_device_id: String,
    file_path: String,
) -> Result<serde_json::Value> {
    if target_device_id.is_empty() {
        return Err(ConduitError::Validation("No target device selected".into()));
    }
    if target_device_id == state.device_id {
        return Err(ConduitError::Validation(
            "Cannot send a file to this device".into(),
        ));
    }

    let transfer_id = uuid::Uuid::new_v4().to_string();
    let file_engine = state.file_engine.clone();
    // Authorisation lives in the engine: `file_path` is canonicalised, refused
    // if it is a symlink or a directory, refused if it is over the send limit,
    // and refused unless it is inside the download root or was explicitly
    // picked by the user. See `FileTransferEngine::approve_send_path`.
    let (size, name, mime, total_chunks, checksum) = file_engine
        .start_outgoing(&transfer_id, &file_path, &target_device_id)
        .await?;

    let timestamp = chrono::Utc::now().timestamp();
    let transfer_id_for_ws = transfer_id.clone();

    // Save to database
    let storage = state.storage.clone();
    let tid_c = transfer_id.clone();
    let name_c = name.clone();
    let mime_c = mime.clone();
    let device_id_c = state.device_id.clone();
    let target_c = target_device_id.clone();
    let size_i = size as i64;
    let total_i = total_chunks as i32;
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.save_file_transfer(
            &crate::storage::FileTransferParams {
                id: &tid_c,
                name: &name_c,
                size: size_i,
                mime: &mime_c,
                from_device: &device_id_c,
                to_device: &target_c,
                status: "pending",
                chunks_received: 0,
                total_chunks: total_i,
                saved_path: None,
                timestamp,
            },
        ))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("send_file task join error: {e}")))??;

    // Send file request to the target device (includes SHA-256 checksum for integrity)
    let mut request = serde_json::json!({
        "type": "file",
        "action": "request",
        "id": transfer_id_for_ws,
        "name": name,
        "size": size,
        "mime": mime,
        "from": state.device_id,
        "to": target_device_id,
    });
    if let Some(ref cs) = checksum {
        request.as_object_mut().unwrap().insert(
            "checksum".to_string(),
            serde_json::Value::String(cs.clone()),
        );
    }

    send_to_peer(&state, &target_device_id, &request.to_string()).await;

    // Spawn a task to send chunks after a short delay (waiting for accept)
    let ws_server = state.ws_server.clone();
    let engine = file_engine;
    let chunk_target = target_device_id.clone();
    tokio::spawn(async move {
        // Wait for accept message (poll every 500ms for up to 30 seconds)
        let mut accepted = false;
        for _ in 0..60 {
            if engine.is_outgoing_accepted(&transfer_id_for_ws).await {
                accepted = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        if accepted {
            while let Some((idx, total, chunk_data)) =
                engine.get_next_chunk(&transfer_id_for_ws).await
            {
                let chunk_msg = serde_json::json!({
                    "type": "file",
                    "action": "chunk",
                    "id": transfer_id_for_ws,
                    "index": idx,
                    "total": total,
                    "data": chunk_data,
                });
                let guard = ws_server.read().await;
                if let Some(ws) = guard.as_ref() {
                    // Directed, not broadcast: a broadcast echoes the chunk
                    // back to the very webview that produced it, and hands
                    // the payload to every other client on the hub.
                    if !ws.send_to(&chunk_target, chunk_msg.to_string()).await {
                        log::warn!("Peer {chunk_target} disconnected mid-transfer; stopping");
                        break;
                    }
                }
                drop(guard);
                // tiny yield to not block the thread completely
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }
        engine.remove_outgoing(&transfer_id_for_ws).await;
    });

    Ok(serde_json::json!({
        "transfer_id": transfer_id.clone(),
        "name": name,
        "size": size,
        "mime": mime,
        "total_chunks": total_chunks,
    }))
}

/// Record paths the user chose in the native file dialog (or dropped on the
/// window) as sendable.
///
/// `send_file` refuses anything that is not inside the download root or listed
/// here, so a `send_file` invocation cannot be aimed at an arbitrary file on
/// the machine. Canonicalisation and the symlink refusal happen in the engine,
/// not here — this command is only the "the user picked this" signal.
#[tauri::command]
pub async fn approve_files_for_send(
    state: State<'_, ManagedState>,
    paths: Vec<String>,
) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Err(ConduitError::Validation("No files selected".into()));
    }
    let mut approved = Vec::with_capacity(paths.len());
    for path in &paths {
        approved.push(
            state
                .file_engine
                .approve_send_path(path)
                .await?
                .to_string_lossy()
                .to_string(),
        );
    }
    Ok(approved)
}

/// Open a file this device received, in the OS default application.
///
/// Takes a **transfer id**, never a path. The `file/complete` frame that
/// finishes a transfer carries a `path` chosen by the *sender*; feeding that
/// into an `open()` call let any paired device decide what this machine
/// launches, including a `\\attacker\share\file` UNC path (an NTLM hash
/// disclosure on Windows) or a `file://` URI, with success or failure as a
/// side channel it can read back.
///
/// This command is why `shell:allow-open` is no longer in
/// `capabilities/default.json`: the webview has no shell capability at all, and
/// the only path that reaches the OS is one this process wrote itself.
#[tauri::command]
pub async fn open_downloaded_file(state: State<'_, ManagedState>, id: String) -> Result<()> {
    let path = resolve_downloaded_file(&state, &id).await?;
    FileTransferEngine::open_local_file(&path)
}

/// Resolve the on-disk path of a completed transfer from local state only.
///
/// Sources, in order: the `file_transfers` row (durable, and only ever written
/// by this app) and the file-transfer engine's record of what it just wrote.
/// A protocol message is never a source.
async fn resolve_downloaded_file(state: &Arc<AppState>, id: &str) -> Result<PathBuf> {
    if id.trim().is_empty() {
        return Err(ConduitError::Validation("A transfer id is required".into()));
    }

    let mut candidates: Vec<String> = Vec::new();

    if let Ok(rows) = state.storage.get_file_transfers(PEER_LOOKUP_LIMIT).await
        && let Some(row) = rows.iter().find(|r| r.id == id)
        && let Some(saved) = row.saved_path.clone()
    {
        candidates.push(saved);
    }

    if let Some(path) = state.file_engine.completed_path(id).await
        && let Some(text) = path.to_str()
    {
        candidates.push(text.to_string());
    }

    if candidates.is_empty() {
        return Err(ConduitError::Other(format!(
            "No completed file is recorded for transfer {id}"
        )));
    }

    let root = state.file_engine.resolve_download_root().await?;

    let mut last_error: Option<ConduitError> = None;
    for candidate in candidates {
        match FileTransferEngine::validate_download_path(&root, &candidate) {
            Ok(resolved) => return Ok(resolved),
            Err(e) => last_error = Some(e),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ConduitError::Other(format!("No completed file is recorded for transfer {id}"))
    }))
}

#[tauri::command]
pub async fn accept_file_transfer(
    state: State<'_, ManagedState>,
    id: String,
    from_device: Option<String>,
    to_device: Option<String>,
) -> Result<()> {
    let peer =
        apply_local_state_and_route(&state, &id, "transferring", from_device, to_device).await?;

    if let Some(peer) = peer {
        let accept = serde_json::json!({
            "type": "file",
            "action": "accept",
            "id": id,
        });
        send_to_peer(&state, &peer, &accept.to_string()).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn cancel_file_transfer(
    state: State<'_, ManagedState>,
    id: String,
    from_device: Option<String>,
    to_device: Option<String>,
) -> Result<()> {
    let peer =
        apply_local_state_and_route(&state, &id, "cancelled", from_device, to_device).await?;

    if let Some(peer) = peer {
        let cancel = serde_json::json!({
            "type": "file",
            "action": "cancel",
            "id": id,
            "reason": "User cancelled",
        });
        send_to_peer(&state, &peer, &cancel.to_string()).await;
    }
    // A cancelled transfer must also release the local buffer, whether the
    // transfer was inbound (chunks on disk) or outbound (queued chunks).
    state.file_engine.cancel_incoming(&id).await;
    state.file_engine.remove_outgoing(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn get_file_transfers(
    state: State<'_, ManagedState>,
    limit: Option<i64>,
) -> Result<Vec<crate::file_transfer::FileTransferInfo>> {
    let limit = limit.unwrap_or(50);
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.get_file_transfers(limit))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_file_transfers task join error: {e}")))?
}

#[tauri::command]
pub async fn resume_file_transfer(
    state: State<'_, ManagedState>,
    id: String,
    name: String,
    size: u64,
    mime: String,
    from_device: String,
    checksum: Option<String>,
) -> Result<serde_json::Value> {
    let loaded = state
        .file_engine
        .resume_incoming(&id, &name, size, &mime, &from_device, checksum)
        .await?;

    let storage = state.storage.clone();
    let id_c = id.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.update_file_transfer_progress(
            &id_c,
            "transferring",
            loaded as i32,
            None,
        ))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("resume_file_transfer task join error: {e}")))??;

    Ok(serde_json::json!({
        "transfer_id": id,
        "chunks_loaded": loaded,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{
        create_app_with_state, create_test_state, create_test_state_with_download_root,
    };
    use crate::storage::FileTransferParams;
    use std::path::Path;
    use tauri::Manager;

    fn transfer_params<'a>(id: &'a str, name: &'a str) -> FileTransferParams<'a> {
        FileTransferParams {
            id,
            name,
            size: 10,
            mime: "application/octet-stream",
            from_device: "dev_a",
            to_device: "dev_b",
            status: "pending",
            chunks_received: 0,
            total_chunks: 1,
            saved_path: None,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }

    /// Real on-disk file inside a self-cleaning temp directory (tempfile crate).
    fn temp_file_with_bytes(bytes: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("send_test.bin");
        std::fs::write(&path, bytes).expect("write temp file");
        let path_str = path.to_string_lossy().to_string();
        (dir, path_str)
    }

    // ── send_file ─────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_file_valid_tempfile_returns_transfer_metadata_and_persists_row() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let (_dir, path) = temp_file_with_bytes(&[1u8; 128]);

        // The user picked this file with the dialog, so it is sendable.
        state
            .file_engine
            .approve_send_path(&path)
            .await
            .expect("picked file is approved for sending");

        let out = send_file(app.state(), "target_dev".to_string(), path)
            .await
            .expect("send_file on an existing, approved file must succeed");

        let transfer_id = out["transfer_id"]
            .as_str()
            .expect("transfer_id present")
            .to_string();
        assert!(!transfer_id.is_empty());
        assert_eq!(out["name"], "send_test.bin");
        assert_eq!(out["size"], 128);
        assert!(out["total_chunks"].as_u64().unwrap() >= 1);

        let rows = state.storage.get_file_transfers(10).await.unwrap();
        assert_eq!(rows.len(), 1, "transfer row must be persisted");
        assert_eq!(rows[0].id, transfer_id);
        assert_eq!(rows[0].status, "pending");
        assert_eq!(rows[0].to_device, "target_dev");
    }

    /// VULN 2: without an explicit user pick, `send_file` must not be able to
    /// read a file from anywhere on the machine.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_file_invalid_unpicked_file_is_refused() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let (_dir, path) = temp_file_with_bytes(b"PRIVATE KEY");

        let result = send_file(app.state(), "target_dev".to_string(), path.clone()).await;

        let err = result.expect_err("an unpicked file must not be sendable");
        assert!(
            err.to_string().contains("not been approved for sending"),
            "unexpected error: {err}"
        );
        let rows = state.storage.get_file_transfers(10).await.unwrap();
        assert!(
            rows.is_empty(),
            "nothing may be persisted for a refused send"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_file_invalid_symlink_is_refused() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        std::fs::write(&real, b"data").unwrap();
        let link = dir.path().join("link.txt");

        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(&real, &link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&real, &link);

        if created.is_err() {
            eprintln!("SKIPPED: cannot create symlinks on this host");
            return;
        }

        let result = send_file(
            app.state(),
            "target_dev".to_string(),
            link.to_string_lossy().to_string(),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symbolic link"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_file_invalid_target_is_this_device_is_refused() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let (_dir, path) = temp_file_with_bytes(&[1u8; 8]);
        state.file_engine.approve_send_path(&path).await.unwrap();

        // Routing to ourselves would make the desktop receive its own outgoing
        // transfer back as an inbound request.
        let result = send_file(app.state(), state.device_id.clone(), path).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot send a file to this device")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_file_invalid_missing_file_returns_error() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        let result = send_file(
            app.state(),
            "target_dev".to_string(),
            r"C:\conduit-test\no_such_file\missing.bin".to_string(),
        )
        .await;

        assert!(result.is_err(), "missing file must produce Err");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read file metadata"),
            "error should explain the metadata failure"
        );
        // Nothing must be persisted for a failed send.
        let rows = state.storage.get_file_transfers(10).await.unwrap();
        assert!(rows.is_empty());
    }

    // ── approve_files_for_send ───────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approve_files_for_send_invalid_empty_list_returns_error() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let result = approve_files_for_send(app.state(), Vec::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approve_files_for_send_invalid_unc_path_returns_error() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let result =
            approve_files_for_send(app.state(), vec![r"\\attacker\share\x".to_string()]).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("only local files are allowed")
        );
    }

    // ── VULN 1: open_downloaded_file ──────────────────────────────────────────

    /// State whose download root is a throwaway directory, plus a
    /// `file_transfers` row for `id` claiming `saved_path` is where the file
    /// landed. `saved_path` is exactly the field a `file/complete` frame
    /// influences in the UI, so it is the interesting injection point.
    async fn state_claiming_saved_path(
        id: &str,
        saved_path: Option<&str>,
    ) -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = create_test_state_with_download_root(dir.path().join("downloads"));
        let device_id = state.device_id.clone();
        state
            .storage
            .save_file_transfer(&FileTransferParams {
                id,
                name: "payload.bin",
                size: 3,
                mime: "application/octet-stream",
                from_device: "dev_a",
                to_device: device_id.as_str(),
                status: "complete",
                chunks_received: 1,
                total_chunks: 1,
                saved_path,
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await
            .unwrap();
        (dir, state)
    }

    /// VULN 1, positive case: a file this process actually wrote into the
    /// download root is authorised, and the argv handed to the OS is built
    /// from that resolved path.
    ///
    /// The final `open_local_file` spawn is deliberately not invoked here — a
    /// test must not launch a viewer. What is asserted is the whole
    /// authorisation step plus the exact argument the spawn would receive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_valid_in_root_path_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = create_test_state_with_download_root(dir.path().join("downloads"));
        // Drive a real inbound transfer through the engine so the file and the
        // "where it landed" record both come from local code, as in production.
        state
            .file_engine
            .start_incoming("ok_1", "payload.bin", 3, "text/plain", "dev_a", None)
            .await;
        state
            .file_engine
            .receive_chunk_binary("ok_1", 0, b"abc")
            .await
            .unwrap();
        let written = state.file_engine.finalize_incoming("ok_1").await.unwrap();
        let root = state.file_engine.resolve_download_root().await.unwrap();

        let resolved = resolve_downloaded_file(&state, "ok_1").await.unwrap();
        assert_eq!(
            resolved,
            root.join("payload.bin"),
            "the resolved path must be the file this device wrote"
        );
        assert!(Path::new(&written).exists());
        assert!(
            resolved.starts_with(&root),
            "{:?} must be inside {:?}",
            resolved,
            root
        );

        let (program, args) = FileTransferEngine::open_command(&resolved);
        assert_eq!(args, vec![resolved.to_string_lossy().to_string()]);
        assert!(!program.is_empty());

        std::fs::remove_file(&resolved).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_path_outside_root_is_rejected() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        let (_dir, state) =
            state_claiming_saved_path("evil_1", Some(&secret.to_string_lossy())).await;
        let app = create_app_with_state(state.clone());

        let err = resolve_downloaded_file(&state, "evil_1")
            .await
            .expect_err("a path outside the download root must be refused");
        assert!(err.to_string().contains("outside the download folder"));

        let err = open_downloaded_file(app.state(), "evil_1".to_string())
            .await
            .expect_err("the command must refuse before launching anything");
        assert!(err.to_string().contains("outside the download folder"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_dot_dot_traversal_is_rejected() {
        let (dir, state) = state_claiming_saved_path("trav_1", None).await;

        // A real file one level above the download root, reachable only by
        // walking out of it.
        let root = dir.path().join("downloads");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("id_rsa");
        std::fs::write(&outside, b"PRIVATE KEY").unwrap();

        let traversal = root.join("..").join("id_rsa");
        state
            .storage
            .update_file_transfer_progress(
                "trav_1",
                "complete",
                1,
                Some(&traversal.to_string_lossy()),
            )
            .await
            .unwrap();

        let app = create_app_with_state(state.clone());
        let err = resolve_downloaded_file(&state, "trav_1")
            .await
            .expect_err("'..' must be rejected before canonicalisation");
        assert!(err.to_string().contains("'..'"), "unexpected error: {err}");

        let err = open_downloaded_file(app.state(), "trav_1".to_string())
            .await
            .expect_err("the command must refuse '..'");
        assert!(err.to_string().contains("'..'"));
    }

    /// A symlink planted inside the download root that points outside it: the
    /// canonical path escapes, so the containment check must fail.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_symlink_escaping_the_root_is_rejected() {
        let (dir, state) = state_claiming_saved_path("link_1", None).await;
        let root = dir.path().join("downloads");
        std::fs::create_dir_all(&root).unwrap();

        let outside_dir = dir.path().join("secret");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let secret = outside_dir.join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        let link = root.join("innocent.txt");
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(&secret, &link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&secret, &link);

        if created.is_err() {
            eprintln!(
                "SKIPPED symlink-escape assertion: this host refuses symlink creation \
                 (Windows needs Developer Mode or elevation). Enable it to run this test."
            );
            return;
        }

        state
            .storage
            .update_file_transfer_progress("link_1", "complete", 1, Some(&link.to_string_lossy()))
            .await
            .unwrap();

        let app = create_app_with_state(state.clone());
        let err = open_downloaded_file(app.state(), "link_1".to_string())
            .await
            .expect_err("a symlink resolving outside the root must be refused");
        assert!(
            err.to_string().contains("outside the download folder"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_unc_path_is_rejected() {
        let (_dir, state) =
            state_claiming_saved_path("unc_1", Some(r"\\attacker\share\payload.exe")).await;
        let err = resolve_downloaded_file(&state, "unc_1")
            .await
            .expect_err("a UNC path must be rejected");
        assert!(err.to_string().contains("only local files are allowed"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_file_uri_is_rejected() {
        let (_dir, state) = state_claiming_saved_path("uri_1", Some("file:///etc/passwd")).await;
        let err = resolve_downloaded_file(&state, "uri_1")
            .await
            .expect_err("a file:// URI must be rejected");
        assert!(err.to_string().contains("only local files are allowed"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_unknown_id_returns_error() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let err = open_downloaded_file(app.state(), "nope".to_string())
            .await
            .expect_err("an unknown transfer id must be an error");
        assert!(err.to_string().contains("No completed file is recorded"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_downloaded_file_invalid_empty_id_returns_error() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        assert!(
            open_downloaded_file(app.state(), "  ".to_string())
                .await
                .is_err()
        );
    }

    /// The webview must not be able to reach an "open anything" primitive, so
    /// `shell:allow-open` is gone. This is the behavioural half of that
    /// capability change; `capability_file_does_not_reintroduce_the_dangerous_grants`
    /// is the declaration half.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn there_is_no_command_that_opens_a_caller_supplied_path() {
        // The only command that reaches the OS "open with default handler"
        // path is `open_downloaded_file`, and it resolves its own path.
        let (_dir, state) = state_claiming_saved_path("opaque", Some(r"\\evil\share\x.exe")).await;
        let err = resolve_downloaded_file(&state, "opaque")
            .await
            .expect_err("an opaque path must never resolve");
        assert!(err.to_string().contains("only local files are allowed"));
    }

    // ── VULN 5: frames go to one peer, never to ourselves ─────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolve_peer_device_never_returns_this_device() {
        let state = create_test_state();
        state
            .storage
            .save_file_transfer(&transfer_params("p_self", "a.bin"))
            .await
            .unwrap();

        // The DB row says from=dev_a, to=dev_b; the caller insists the peer is
        // this device. Self must never win.
        let peer = resolve_peer_device(
            &state,
            "p_self",
            Some(state.device_id.clone()),
            Some(state.device_id.clone()),
        )
        .await;
        assert_eq!(peer.as_deref(), Some("dev_a"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolve_peer_device_ignores_the_local_alias() {
        let state = create_test_state();
        let peer = resolve_peer_device(&state, "unknown", Some("local".into()), None).await;
        assert_eq!(peer, None, "'local' is not a routable WS client");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolve_peer_device_uses_the_callers_device_ids_when_no_row_exists() {
        let state = create_test_state();
        // An inbound transfer is never written to `file_transfers`, so the UI
        // supplies the sender id.
        let peer = resolve_peer_device(
            &state,
            "ghost",
            Some("peer_dev".into()),
            Some("local".into()),
        )
        .await;
        assert_eq!(peer.as_deref(), Some("peer_dev"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolve_peer_device_returns_none_for_an_unknown_transfer() {
        let state = create_test_state();
        assert_eq!(resolve_peer_device(&state, "ghost", None, None).await, None);
    }

    // ── accept / cancel ───────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accept_file_transfer_valid_marks_row_transferring() {
        let state = create_test_state();
        state
            .storage
            .save_file_transfer(&transfer_params("acc_1", "a.bin"))
            .await
            .unwrap();
        let app = create_app_with_state(state.clone());

        accept_file_transfer(app.state(), "acc_1".to_string(), None, None)
            .await
            .unwrap();

        let rows = state.storage.get_file_transfers(10).await.unwrap();
        assert_eq!(rows[0].status, "transferring");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accept_file_transfer_invalid_unknown_id_is_noop_not_panic() {
        // CHARACTERIZATION: UPDATE on a missing row is a silent no-op today.
        // If a future change starts returning Err for unknown ids, rename this
        // test to accept_file_transfer_unknown_id_returns_error.
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        accept_file_transfer(app.state(), "ghost_id".to_string(), None, None)
            .await
            .unwrap();
        let rows = state.storage.get_file_transfers(10).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_file_transfer_valid_marks_row_cancelled() {
        let state = create_test_state();
        state
            .storage
            .save_file_transfer(&transfer_params("can_1", "c.bin"))
            .await
            .unwrap();
        let app = create_app_with_state(state.clone());

        cancel_file_transfer(app.state(), "can_1".to_string(), None, None)
            .await
            .unwrap();

        let rows = state.storage.get_file_transfers(10).await.unwrap();
        assert_eq!(rows[0].status, "cancelled");
    }

    /// Cancelling an inbound transfer must release the buffer on disk, not just
    /// the database row.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_file_transfer_invalid_inbound_releases_the_chunk_buffer() {
        let state = create_test_state();
        state
            .file_engine
            .start_incoming(
                "can_buf",
                "big.bin",
                1024,
                "application/octet-stream",
                "dev_a",
                None,
            )
            .await;
        state
            .file_engine
            .receive_chunk_binary("can_buf", 0, &[0u8; 1024])
            .await
            .unwrap();
        let transfer_dir = state.file_engine.chunk_temp_root().join("can_buf");
        assert!(transfer_dir.exists());

        let app = create_app_with_state(state.clone());
        cancel_file_transfer(
            app.state(),
            "can_buf".to_string(),
            Some("dev_a".to_string()),
            Some("local".to_string()),
        )
        .await
        .unwrap();

        assert!(!transfer_dir.exists());
        assert!(!state.file_engine.is_complete("can_buf").await);
    }

    // ── get_file_transfers ────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_file_transfers_valid_respects_limit() {
        let state = create_test_state();
        for i in 0..3 {
            state
                .storage
                .save_file_transfer(&transfer_params(&format!("t{i}"), &format!("f{i}.bin")))
                .await
                .unwrap();
        }
        let app = create_app_with_state(state.clone());

        let rows = get_file_transfers(app.state(), Some(2)).await.unwrap();
        assert_eq!(rows.len(), 2, "limit=2 must cap results");
        // Direct storage cross-check keeps the command honest about its passthrough.
        let raw = state.storage.get_file_transfers(2).await.unwrap();
        assert_eq!(raw.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_file_transfers_valid_none_limit_defaults_to_50() {
        let state = create_test_state();
        state
            .storage
            .save_file_transfer(&transfer_params("t_only", "only.bin"))
            .await
            .unwrap();
        let app = create_app_with_state(state);

        let rows = get_file_transfers(app.state(), None).await.unwrap();
        assert_eq!(rows.len(), 1, "None limit must still return rows");
    }

    // ── resume_file_transfer ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_file_transfer_valid_unknown_transfer_returns_zero_chunks() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let out = resume_file_transfer(
            app.state(),
            "fresh_id".to_string(),
            "file.bin".to_string(),
            1024,
            "application/octet-stream".to_string(),
            "dev_a".to_string(),
            None,
        )
        .await
        .expect("resume recreates the incoming transfer descriptor");

        assert_eq!(out["transfer_id"], "fresh_id");
        assert_eq!(out["chunks_loaded"], 0, "no chunks on disk yet");
    }

    // ── missing state ─────────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "state() called before manage")]
    fn file_commands_missing_state_fails_loudly() {
        let app = tauri::test::mock_app();
        let _ = app.state::<Arc<AppState>>();
    }

    // ── VULN 6/7: the capability file must describe what the app imports ─────

    /// One representative frontend import per granted permission.
    ///
    /// If a permission is added without a caller, the app carries a capability
    /// nothing can use. If an import appears without a permission, the app
    /// breaks at runtime with "not allowed by the scope" — which is exactly how
    /// the Settings → Import button stayed broken: `@tauri-apps/plugin-fs` was
    /// imported with no Rust plugin and no `fs:*` permission.
    const CAPABILITY_IMPORTS: &[(&str, &str)] = &[
        ("core:default", "@tauri-apps/api/core"),
        ("core:window:default", "@tauri-apps/api/window"),
        ("core:window:allow-show", "@tauri-apps/api/window"),
        (
            "clipboard-manager:allow-read-text",
            "@tauri-apps/plugin-clipboard-manager",
        ),
        (
            "clipboard-manager:allow-write-text",
            "@tauri-apps/plugin-clipboard-manager",
        ),
        ("dialog:allow-open", "@tauri-apps/plugin-dialog"),
        ("fs:allow-read-text-file", "@tauri-apps/plugin-fs"),
    ];

    /// Permissions that must NOT come back. Each one is a capability whose only
    /// consumer was dead code, or whose path was remote-controlled.
    const FORBIDDEN_PERMISSIONS: &[(&str, &str)] = &[
        (
            "shell:allow-open",
            "a paired device chose the path, including \\\\attacker\\share (NTLM disclosure)",
        ),
        (
            "process:allow-exit",
            "the frontend never imports @tauri-apps/plugin-process",
        ),
        (
            "process:allow-restart",
            "the frontend never imports @tauri-apps/plugin-process",
        ),
        (
            "core:window:allow-close",
            "only the never-imported TitleBar.tsx used it",
        ),
        (
            "core:window:allow-minimize",
            "only the never-imported TitleBar.tsx used it",
        ),
        (
            "core:window:allow-maximize",
            "only the never-imported TitleBar.tsx used it",
        ),
        (
            "core:window:allow-unmaximize",
            "only the never-imported TitleBar.tsx used it",
        ),
        (
            "core:window:allow-set-focus",
            "only the never-imported TitleBar.tsx used it",
        ),
    ];

    fn manifest_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
    }

    fn capability_permissions() -> Vec<String> {
        let path = manifest_dir().join("capabilities").join("default.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let json: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        json["permissions"]
            .as_array()
            .expect("permissions must be an array")
            .iter()
            .map(|p| {
                p.as_str()
                    .expect("permission entries are strings")
                    .to_string()
            })
            .collect()
    }

    /// Every `.ts`/`.tsx` file under `apps/desktop/src`.
    fn frontend_sources() -> Vec<std::path::PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("ts") | Some("tsx")
                ) {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(&manifest_dir().join("..").join("src"), &mut out);
        assert!(!out.is_empty(), "no frontend sources found");
        out
    }

    fn frontend_corpus() -> String {
        let mut corpus = String::new();
        for path in frontend_sources() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                corpus.push_str(&text);
            }
        }
        corpus
    }

    #[test]
    fn capability_file_grants_nothing_the_frontend_cannot_use() {
        let corpus = frontend_corpus();
        for (permission, import) in CAPABILITY_IMPORTS {
            assert!(
                corpus.contains(import),
                "{permission} is granted but nothing under apps/desktop/src imports {import}"
            );
        }
    }

    #[test]
    fn every_tauri_plugin_the_frontend_imports_has_a_capability() {
        let permissions = capability_permissions();
        let corpus = frontend_corpus();
        for plugin in [
            "fs",
            "dialog",
            "clipboard-manager",
            "process",
            "shell",
            "opener",
        ] {
            if !corpus.contains(&format!("@tauri-apps/plugin-{plugin}")) {
                continue;
            }
            assert!(
                permissions
                    .iter()
                    .any(|p| p.starts_with(&format!("{plugin}:"))),
                "apps/desktop/src imports @tauri-apps/plugin-{plugin} but no `{plugin}:*` \
                 permission is granted — the call would fail at runtime"
            );
        }
    }

    #[test]
    fn capability_file_does_not_reintroduce_the_dangerous_grants() {
        let permissions = capability_permissions();
        for (permission, why) in FORBIDDEN_PERMISSIONS {
            assert!(
                !permissions.iter().any(|p| p == permission),
                "{permission} must not be granted: {why}"
            );
        }
    }

    #[test]
    fn capability_file_only_contains_known_permissions() {
        let permissions = capability_permissions();
        for permission in &permissions {
            let known = CAPABILITY_IMPORTS.iter().any(|(p, _)| p == permission)
                || permission == "core:window:allow-is-maximized"
                || permission == "core:window:allow-is-visible"
                || permission == "core:event:default"
                || permission == "core:app:default"
                || permission == "core:path:default"
                || permission == "core:resources:default"
                || permission == "core:tray:default"
                || permission == "core:webview:default";
            assert!(
                known,
                "{permission} is not a permission this test knows a consumer for; add it to \
                 CAPABILITY_IMPORTS (with its importer) or remove it"
            );
        }
    }

    #[test]
    fn frontend_does_not_import_the_shell_plugin() {
        let corpus = frontend_corpus();
        assert!(
            !corpus.contains("@tauri-apps/plugin-shell"),
            "the webview must not import @tauri-apps/plugin-shell: opening a received file \
             goes through the `open_downloaded_file` command, which resolves the path from \
             local state instead of from a protocol message"
        );
    }
}
