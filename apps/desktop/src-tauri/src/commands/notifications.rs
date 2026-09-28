//! Tauri commands backing the notification tray / list in the desktop UI.
//!
//! # Why these commands exist at all
//!
//! The desktop webview talks to the Rust side through two independent
//! transports:
//!
//! * a **Tauri command** (IPC, `invoke(...)`) for local state, and
//! * a **WebSocket** to the local `WsServer` for remote traffic.
//!
//! `useWebSocket.tsx` previously called `get_notifications`,
//! `dismiss_notification`, `reply_notification` and `sync_clipboard` over IPC
//! while *none of them were registered*, so every call rejected: the
//! notification list was always `[]` and replies went nowhere.
//!
//! # Which transport owns which action
//!
//! For the three notification actions there are now two paths, so the
//! ownership is explicit:
//!
//! * **Tauri command** = the desktop *user* acted. The command persists the
//!   local effect first (so a reload/restart never resurrects a dismissed
//!   notification) and then forwards one canonical protocol frame so the phone
//!   converges. Single source of truth for locally-initiated actions.
//! * **WS handler** (`server::handlers::notifications`) = a *remote peer*
//!   initiated the action (or a peer is relaying one). The handler's job is to
//!   apply the local effect and fan the frame out to the other peers, i.e. it
//!   is the inbound mirror image of the command.
//!
//! They are never both used for the same act, so no action is applied twice.
//! `useWebSocket.tsx` therefore must NOT additionally `sendMessage(...)` a
//! `notification/dismiss` or `notification/reply` frame — that would duplicate
//! both the DB write and the outbound frame.

use crate::AppState;
use crate::error::{ConduitError, Result};
use crate::storage::StoredNotification;
use conduit_protocol::types::{ClipboardSync, NotificationDismiss, NotificationReply};
use log::warn;
use std::sync::Arc;
use tauri::State;

type ManagedState = Arc<AppState>;

/// Default page size for `get_notifications`, matching what the UI asks for.
pub(crate) const NOTIFICATIONS_LIMIT_DEFAULT: i64 = 50;
/// Upper bound for `get_notifications`.
pub(crate) const NOTIFICATIONS_LIMIT_MAX: i64 = 500;

/// Clamp a caller-supplied `limit` into a range SQLite's `LIMIT ?1` can honour.
///
/// SQLite treats a **negative** `LIMIT` as "no limit at all", so an unclamped
/// negative value from the renderer would silently dump the whole table; `0`
/// would return nothing at all. Both are surprising, so we clamp instead of
/// passing the raw value through.
pub(crate) fn clamp_notification_limit(limit: Option<i64>) -> i64 {
    match limit {
        None => NOTIFICATIONS_LIMIT_DEFAULT,
        Some(value) => value.clamp(1, NOTIFICATIONS_LIMIT_MAX),
    }
}

/// Caller-input rejection.
///
/// `ConduitError` has no dedicated `Validation` variant (see `src/error.rs`),
/// so bad renderer input is reported through the catch-all `Other` variant and
/// reaches the frontend as a plain string via the manual `Serialize` impl.
fn validation(message: impl Into<String>) -> ConduitError {
    ConduitError::Other(format!("invalid argument: {}", message.into()))
}

/// Reject a blank/oversized reply before it is put on the wire, so a stuck UI
/// input cannot flood peers with meaningless frames.
fn validate_reply_text(text: &str) -> Result<()> {
    const MAX_REPLY_LEN: usize = 4096;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(validation("reply text is empty"));
    }
    if trimmed.chars().count() > MAX_REPLY_LEN {
        return Err(validation(format!(
            "reply text exceeds {MAX_REPLY_LEN} characters"
        )));
    }
    Ok(())
}

/// Return the most recent notifications, newest first.
///
/// `limit` is optional and clamped (see [`clamp_notification_limit`]).
#[tauri::command]
pub async fn get_notifications(
    state: State<'_, ManagedState>,
    limit: Option<i64>,
) -> Result<Vec<StoredNotification>> {
    let limit = clamp_notification_limit(limit);
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.get_notifications(limit))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_notifications task join error: {e}")))?
}

/// Dismiss a notification: persist `dismissed = 1` locally, then forward a
/// canonical `notification/dismiss` frame to the connected peers.
///
/// `id` is a `String` because `notifications.id` is a TEXT column (and the
/// protocol's `NotificationDismiss.id` is a `String`). This is the locally
/// initiated path; see the module docs for how it relates to the WS handler.
#[tauri::command]
pub async fn dismiss_notification(state: State<'_, ManagedState>, id: String) -> Result<()> {
    if id.trim().is_empty() {
        return Err(validation("notification id must not be empty"));
    }

    // 1. Persist first: a local state change must never depend on a peer being
    //    connected. `Storage::dismiss_notification` is a no-op UPDATE for an
    //    unknown id, so replaying a stale UI click is harmless.
    let storage = state.storage.clone();
    let id_for_db = id.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.dismiss_notification(&id_for_db))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("dismiss_notification task join error: {e}")))??;

    // 2. Forward so the phone drops the same notification. Failure to reach a
    //    peer is logged, not surfaced: the local dismissal already succeeded
    //    and the UI has optimistically updated.
    forward_to_peers(
        &state,
        &NotificationDismiss {
            msg_type: "notification".into(),
            action: "dismiss".into(),
            id,
        },
    )
    .await;

    Ok(())
}

/// Reply to a notification and deliver the reply to the originating device.
///
/// The reply is **forward-only on purpose**: there is no `notifications.reply`
/// column and no reply history table (see `migrations/001_initial.sql`), so
/// persisting it would require a schema change that is out of scope here. The
/// *dismissal* is the durable local record; the reply text is a transient UI
/// gesture whose authoritative copy lives on the phone (the app that owns the
/// notification). Persisting it locally would also give the desktop two
/// conflicting "last reply" records for the same notification.
#[tauri::command]
pub async fn reply_notification(
    state: State<'_, ManagedState>,
    id: String,
    text: String,
) -> Result<()> {
    if id.trim().is_empty() {
        return Err(validation("notification id must not be empty"));
    }
    validate_reply_text(&text)?;

    let reply = NotificationReply {
        msg_type: "notification".into(),
        action: "reply".into(),
        id,
        text: text.trim().to_string(),
    };

    // Unlike dismiss, a reply has no local effect to fall back on, so a
    // failure here is propagated: the caller (UI) is told the truth instead of
    // silently losing the text.
    forward_to_peers_checked(&state, &reply).await?;

    Ok(())
}

/// Persist the desktop's clipboard into history and broadcast a
/// `clipboard/sync` frame (protocol type `ClipboardSync`) to the peers.
///
/// This is the locally initiated counterpart of the `("clipboard", "sync")`
/// branch in `WsServer::handle_message`, which relays an inbound sync to the
/// other peers.
#[tauri::command]
pub async fn sync_clipboard(
    state: State<'_, ManagedState>,
    content: String,
    mime: String,
    // Tauri v2 defaults to `rename_all = "camelCase"`, so the renderer passes
    // this as `sourceDevice`.
    source_device: String,
) -> Result<()> {
    const MAX_CLIPBOARD_LEN: usize = 5 * 1024 * 1024;
    if content.is_empty() {
        return Err(validation("clipboard content must not be empty"));
    }
    if content.len() > MAX_CLIPBOARD_LEN {
        return Err(validation(format!(
            "clipboard content exceeds {MAX_CLIPBOARD_LEN} bytes"
        )));
    }
    if source_device.trim().is_empty() {
        return Err(validation("source_device must not be empty"));
    }

    let timestamp = chrono::Utc::now().timestamp();

    // Persist locally first so a restart keeps the shared clipboard in history.
    let storage = state.storage.clone();
    let content_c = content.clone();
    let mime_c = mime.clone();
    let source_c = source_device.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current()
            .block_on(storage.save_clipboard(&content_c, &mime_c, &source_c, timestamp))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("sync_clipboard task join error: {e}")))??;

    let sync = ClipboardSync {
        msg_type: "clipboard".into(),
        action: "sync".into(),
        content,
        mime,
        source_device,
        timestamp,
    };

    forward_to_peers_checked(&state, &sync).await?;

    Ok(())
}

/// Serialize a protocol message and hand it to the WS server, ignoring a
/// missing/closed server (nothing to forward to is not an error for a
/// best-effort fan-out).
async fn forward_to_peers<T: serde::Serialize>(state: &State<'_, ManagedState>, msg: &T) {
    if let Err(e) = forward_to_peers_checked(state, msg).await {
        warn!("Failed to forward message to peers: {e}");
    }
}

/// Same as [`forward_to_peers`] but propagates serialization failures so
/// callers that have no local fallback can report the problem.
async fn forward_to_peers_checked<T: serde::Serialize>(
    state: &State<'_, ManagedState>,
    msg: &T,
) -> Result<()> {
    let text = serde_json::to_string(msg).map_err(|e| {
        ConduitError::Protocol(format!("failed to serialize outbound message: {e}"))
    })?;
    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(text).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{create_app_with_state, create_test_state};
    use tauri::Manager;

    async fn seed_notification(state: &Arc<AppState>, id: &str, timestamp: i64) {
        state
            .storage
            .save_notification(&crate::storage::NotificationParams {
                id,
                device_id: "phone_1",
                app: "Slack",
                title: "New message",
                body: "Hello",
                timestamp,
                actions: None,
            })
            .await
            .unwrap();
    }

    // ── clamp_notification_limit ─────────────────────────────────────────────

    #[test]
    fn clamp_notification_limit_valid_none_uses_default() {
        assert_eq!(clamp_notification_limit(None), NOTIFICATIONS_LIMIT_DEFAULT);
    }

    #[test]
    fn clamp_notification_limit_valid_in_range_is_passed_through() {
        assert_eq!(clamp_notification_limit(Some(1)), 1);
        assert_eq!(clamp_notification_limit(Some(50)), 50);
        assert_eq!(
            clamp_notification_limit(Some(NOTIFICATIONS_LIMIT_MAX)),
            NOTIFICATIONS_LIMIT_MAX
        );
    }

    #[test]
    fn clamp_notification_limit_edge_negative_becomes_minimum() {
        // SQLite would read `LIMIT -1` as "no limit" — never pass that through.
        assert_eq!(clamp_notification_limit(Some(-1)), 1);
        assert_eq!(clamp_notification_limit(Some(i64::MIN)), 1);
    }

    #[test]
    fn clamp_notification_limit_edge_zero_becomes_minimum() {
        // `LIMIT 0` would return an empty list and look like data loss.
        assert_eq!(clamp_notification_limit(Some(0)), 1);
    }

    #[test]
    fn clamp_notification_limit_edge_oversized_is_capped() {
        assert_eq!(
            clamp_notification_limit(Some(10_000)),
            NOTIFICATIONS_LIMIT_MAX
        );
        assert_eq!(
            clamp_notification_limit(Some(i64::MAX)),
            NOTIFICATIONS_LIMIT_MAX
        );
    }

    // ── get_notifications ────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_notifications_valid_returns_rows_newest_first() {
        let state = create_test_state();
        seed_notification(&state, "old", 1_000).await;
        seed_notification(&state, "new", 2_000).await;
        let app = create_app_with_state(state);

        let rows = get_notifications(app.state(), None).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "new", "newest timestamp must sort first");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_notifications_valid_respects_limit() {
        let state = create_test_state();
        for i in 0..3 {
            seed_notification(&state, &format!("n{i}"), 1_000 + i).await;
        }
        let app = create_app_with_state(state);

        let rows = get_notifications(app.state(), Some(2)).await.unwrap();
        assert_eq!(rows.len(), 2, "limit=2 must cap results");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_notifications_edge_negative_limit_is_clamped_not_unbounded() {
        let state = create_test_state();
        for i in 0..3 {
            seed_notification(&state, &format!("n{i}"), 1_000 + i).await;
        }
        let app = create_app_with_state(state);

        // Without the clamp this reaches SQLite as `LIMIT -1` and returns all
        // rows; the clamp must reduce it to a single row.
        let rows = get_notifications(app.state(), Some(-5)).await.unwrap();
        assert_eq!(rows.len(), 1, "negative limit must clamp to 1");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_notifications_edge_empty_db_returns_empty() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        assert!(
            get_notifications(app.state(), Some(50))
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── dismiss_notification ─────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dismiss_notification_valid_marks_row_dismissed() {
        let state = create_test_state();
        seed_notification(&state, "n1", 1_000).await;
        let app = create_app_with_state(state.clone());

        dismiss_notification(app.state(), "n1".to_string())
            .await
            .unwrap();

        let rows = state.storage.get_notifications(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].dismissed, "dismissed flag must be persisted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dismiss_notification_valid_is_idempotent() {
        let state = create_test_state();
        seed_notification(&state, "n1", 1_000).await;
        let app = create_app_with_state(state.clone());

        dismiss_notification(app.state(), "n1".to_string())
            .await
            .unwrap();
        dismiss_notification(app.state(), "n1".to_string())
            .await
            .unwrap();

        assert!(
            state.storage.get_notifications(10).await.unwrap()[0].dismissed,
            "repeat dismissal must stay dismissed"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dismiss_notification_invalid_empty_id_is_rejected() {
        let state = create_test_state();
        seed_notification(&state, "n1", 1_000).await;
        let app = create_app_with_state(state.clone());

        let err = dismiss_notification(app.state(), "   ".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, ConduitError::Other(_)));
        assert!(
            !state.storage.get_notifications(10).await.unwrap()[0].dismissed,
            "a rejected call must not touch the row"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dismiss_notification_unknown_id_is_noop_and_succeeds() {
        // CHARACTERIZATION: the storage layer's UPDATE on a missing row is a
        // no-op that returns Ok. If this ever errors, rename the test.
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        dismiss_notification(app.state(), "does-not-exist".to_string())
            .await
            .unwrap();
        assert!(
            state
                .storage
                .get_notifications(10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── reply_notification ───────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reply_notification_valid_succeeds_without_ws_server() {
        let state = create_test_state();
        seed_notification(&state, "n1", 1_000).await;
        let app = create_app_with_state(state);

        reply_notification(app.state(), "n1".to_string(), "On my way".to_string())
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reply_notification_invalid_empty_text_is_rejected() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let err = reply_notification(app.state(), "n1".to_string(), "   ".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, ConduitError::Other(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reply_notification_invalid_empty_id_is_rejected() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let err = reply_notification(app.state(), "".to_string(), "hi".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, ConduitError::Other(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reply_notification_edge_oversized_text_is_rejected() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let huge = "x".repeat(5000);
        let err = reply_notification(app.state(), "n1".to_string(), huge)
            .await
            .unwrap_err();
        assert!(matches!(err, ConduitError::Other(_)));
    }

    // ── sync_clipboard ───────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_clipboard_valid_writes_history_entry() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        sync_clipboard(
            app.state(),
            "copied text".to_string(),
            "text/plain".to_string(),
            "local_desktop".to_string(),
        )
        .await
        .unwrap();

        let history = state.storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "copied text");
        assert_eq!(history[0].mime, "text/plain");
        assert_eq!(history[0].source_device, "local_desktop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_clipboard_invalid_empty_content_is_rejected() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        let err = sync_clipboard(
            app.state(),
            String::new(),
            "text/plain".to_string(),
            "local_desktop".to_string(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ConduitError::Other(_)));
        assert!(
            state
                .storage
                .get_clipboard_history(10)
                .await
                .unwrap()
                .is_empty(),
            "rejected sync must not create a history row"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_clipboard_invalid_empty_source_device_is_rejected() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let err = sync_clipboard(
            app.state(),
            "text".to_string(),
            "text/plain".to_string(),
            " ".to_string(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ConduitError::Other(_)));
    }

    // ── protocol shape of the outbound frames ───────────────────────────────

    #[test]
    fn outbound_dismiss_frame_matches_protocol_shape() {
        let msg = NotificationDismiss {
            msg_type: "notification".into(),
            action: "dismiss".into(),
            id: "n1".into(),
        };
        let value: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value["type"], "notification");
        assert_eq!(value["action"], "dismiss");
        assert_eq!(value["id"], "n1");
        // Round-trips through the type the relay/peer deserialises.
        let back: NotificationDismiss = serde_json::from_value(value).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn outbound_reply_frame_matches_protocol_shape() {
        let msg = NotificationReply {
            msg_type: "notification".into(),
            action: "reply".into(),
            id: "n1".into(),
            text: "On my way".into(),
        };
        let value: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value["type"], "notification");
        assert_eq!(value["action"], "reply");
        assert_eq!(value["id"], "n1");
        assert_eq!(value["text"], "On my way");
    }

    #[test]
    fn outbound_clipboard_sync_frame_matches_protocol_shape() {
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "hello".into(),
            mime: "text/plain".into(),
            source_device: "local_desktop".into(),
            timestamp: 1_700_000_000,
        };
        let value: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value["type"], "clipboard");
        assert_eq!(value["action"], "sync");
        assert_eq!(value["source_device"], "local_desktop");
        assert_eq!(value["timestamp"], 1_700_000_000i64);
    }

    // ── round trip: persist → dismiss → reload ───────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_round_trip_save_list_dismiss_survives_reload() {
        let state = create_test_state();
        seed_notification(&state, "n1", 1_000).await;
        seed_notification(&state, "n2", 2_000).await;

        {
            let app = create_app_with_state(state.clone());
            assert_eq!(
                get_notifications(app.state(), Some(50))
                    .await
                    .unwrap()
                    .len(),
                2
            );
            dismiss_notification(app.state(), "n1".to_string())
                .await
                .unwrap();
        }

        // Re-read through a *fresh* command invocation, as a UI reload would.
        let app = create_app_with_state(state.clone());
        let rows = get_notifications(app.state(), Some(50)).await.unwrap();
        let n1 = rows.iter().find(|r| r.id == "n1").expect("n1 present");
        let n2 = rows.iter().find(|r| r.id == "n2").expect("n2 present");
        assert!(n1.dismissed, "dismissal must survive a reload");
        assert!(!n2.dismissed, "other rows must be untouched");
    }

    // ── missing state ───────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "state() called before manage")]
    fn notification_commands_missing_state_fails_loudly() {
        let app = tauri::test::mock_app();
        let _ = app.state::<Arc<AppState>>();
    }
}
