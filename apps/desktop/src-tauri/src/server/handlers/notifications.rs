use log::warn;
use serde_json::Value;

use super::{WsContext, broadcast_to_others};

pub async fn handle_notification_post(msg: Value, client_id: &str, ctx: &WsContext) {
    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let device_id = msg
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or(client_id);
    let app = msg.get("app").and_then(|v| v.as_str()).unwrap_or("unknown");
    let title = msg.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let body = msg.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let timestamp = msg.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

    if !id.is_empty()
        && let Err(e) = ctx
            .storage
            .save_notification(&crate::storage::NotificationParams {
                id,
                device_id,
                app,
                title,
                body,
                timestamp,
                actions: None,
            })
            .await
    {
        warn!("Failed to persist notification from {}: {}", client_id, e);
    }

    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

/// Inbound mirror of the `dismiss_notification` Tauri command.
///
/// A remote peer (or the relay) initiated this, so the local effect is applied
/// *and* the frame is fanned out to the other peers. The desktop-originated
/// case goes through `commands::dismiss_notification` instead, which persists
/// first and then broadcasts — the two paths are never both used for one act,
/// so the row is written exactly once.
pub async fn handle_notification_dismiss(msg: Value, client_id: &str, ctx: &WsContext) {
    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");

    if !id.is_empty()
        && let Err(e) = ctx.storage.dismiss_notification(id).await
    {
        warn!("Failed to persist dismissal from {}: {}", client_id, e);
    }

    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

/// Intentionally a **pure relay**: `notifications` has no reply column and no
/// reply-history table (`migrations/001_initial.sql`), so there is nothing to
/// persist. The authoritative copy of a reply lives on the device that owns the
/// notification; the desktop is only forwarding it on behalf of the user.
/// The desktop-originated counterpart is `commands::reply_notification`.
pub async fn handle_notification_reply(msg: Value, client_id: &str, ctx: &WsContext) {
    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

/// Intentionally a **pure relay**: `notifications` has no `read` column, only
/// `dismissed` (`migrations/001_initial.sql`), so marking a notification as
/// read has no local representation to persist. Fanning the frame out to the
/// other peers is the whole job; a peer that *dismisses* it will persist.
pub async fn handle_notification_mark_read(msg: Value, client_id: &str, ctx: &WsContext) {
    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{add_test_client, create_test_ctx};

    // ── handle_notification_post ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_post_happy_persists_and_broadcasts_to_others() {
        let ctx = create_test_ctx();
        let _src_tx = add_test_client(&ctx, "src").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        let msg = serde_json::json!({
            "type": "notification",
            "action": "post",
            "id": "n1",
            "device_id": "phone_1",
            "app": "Slack",
            "title": "New message",
            "body": "Hello from phone",
            "timestamp": 1_700_000_000
        });

        handle_notification_post(msg.clone(), "src", &ctx).await;

        let stored = ctx.storage.get_notifications(10).await.unwrap();
        assert_eq!(stored.len(), 1, "notification must be persisted");
        assert_eq!(stored[0].id, "n1");
        assert_eq!(stored[0].app, "Slack");
        assert_eq!(stored[0].title, "New message");
        assert!(!stored[0].dismissed);

        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(received.is_ok(), "peer must receive the broadcast");
        let text = received.unwrap().unwrap();
        assert!(
            text.contains("\"n1\""),
            "broadcast must carry the id: {text}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_post_invalid_empty_id_skips_persistence_still_broadcasts() {
        let ctx = create_test_ctx();
        let _src_tx = add_test_client(&ctx, "src").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        // Missing id → handler must not persist, but relays to peers.
        let msg = serde_json::json!({
            "type": "notification",
            "action": "post",
            "title": "No id"
        });

        handle_notification_post(msg, "src", &ctx).await;

        assert!(
            ctx.storage.get_notifications(10).await.unwrap().is_empty(),
            "empty id must not create a storage row"
        );
        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(received.is_ok(), "broadcast still happens without id");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_post_edge_wrong_types_use_defaults_not_panic() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "src").await;

        // Wrong JSON types everywhere → as_str/as_i64 return None → defaults.
        let msg = serde_json::json!({
            "type": "notification",
            "action": "post",
            "id": 12345,
            "title": ["array"],
            "timestamp": "not-a-number"
        });

        handle_notification_post(msg, "src", &ctx).await;

        let stored = ctx.storage.get_notifications(10).await.unwrap();
        // id defaulted to "" → skipped; nothing stored, no panic.
        assert!(stored.is_empty());
    }

    // ── handle_notification_dismiss / reply ───────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_dismiss_happy_persists_and_broadcasts_to_others() {
        let ctx = create_test_ctx();
        let _src_tx = add_test_client(&ctx, "src").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        ctx.storage
            .save_notification(&crate::storage::NotificationParams {
                id: "n9",
                device_id: "phone_1",
                app: "Slack",
                title: "New message",
                body: "Hello from phone",
                timestamp: 1_700_000_000,
                actions: None,
            })
            .await
            .unwrap();
        assert!(!ctx.storage.get_notifications(10).await.unwrap()[0].dismissed);

        let msg = serde_json::json!({ "type": "notification", "action": "dismiss", "id": "n9" });
        handle_notification_dismiss(msg, "src", &ctx).await;

        // The regression this fixes: `dismissed = 1` was never written, so a
        // dismissed notification came back from `get_notifications` unchanged.
        let stored = ctx.storage.get_notifications(10).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].dismissed, "dismissal must be persisted");

        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(received.is_ok());
        assert!(received.unwrap().unwrap().contains("\"dismiss\""));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_dismiss_invalid_missing_id_skips_persistence_still_broadcasts() {
        let ctx = create_test_ctx();
        let _src_tx = add_test_client(&ctx, "src").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        ctx.storage
            .save_notification(&crate::storage::NotificationParams {
                id: "n9",
                device_id: "phone_1",
                app: "Slack",
                title: "New message",
                body: "Hello from phone",
                timestamp: 1_700_000_000,
                actions: None,
            })
            .await
            .unwrap();

        // No `id` field → nothing to key the UPDATE on, but the relay still runs.
        let msg = serde_json::json!({ "type": "notification", "action": "dismiss" });
        handle_notification_dismiss(msg, "src", &ctx).await;

        assert!(
            !ctx.storage.get_notifications(10).await.unwrap()[0].dismissed,
            "a frame without an id must not dismiss anything"
        );
        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(received.is_ok(), "relay happens regardless of id");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_dismiss_edge_wrong_type_id_does_not_panic() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "src").await;

        // Numeric id: as_str returns None → treated as "no id" → no UPDATE.
        let msg = serde_json::json!({ "type": "notification", "action": "dismiss", "id": 42 });
        handle_notification_dismiss(msg, "src", &ctx).await;

        assert!(ctx.storage.get_notifications(10).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_reply_happy_is_pure_relay_broadcasts_to_others() {
        let ctx = create_test_ctx();
        let _src_tx = add_test_client(&ctx, "src").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        let msg = serde_json::json!({
            "type": "notification",
            "action": "reply",
            "id": "n9",
            "text": "On my way"
        });
        handle_notification_reply(msg, "src", &ctx).await;

        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(received.is_ok());
        assert!(received.unwrap().unwrap().contains("On my way"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_mark_read_is_pure_relay_and_changes_nothing_locally() {
        let ctx = create_test_ctx();
        let _src_tx = add_test_client(&ctx, "src").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        ctx.storage
            .save_notification(&crate::storage::NotificationParams {
                id: "n9",
                device_id: "phone_1",
                app: "Slack",
                title: "New message",
                body: "Hello from phone",
                timestamp: 1_700_000_000,
                actions: None,
            })
            .await
            .unwrap();

        let msg = serde_json::json!({ "type": "notification", "action": "mark_read", "id": "n9" });
        handle_notification_mark_read(msg, "src", &ctx).await;

        // No `read` column exists, so mark_read must NOT touch `dismissed`.
        assert!(
            !ctx.storage.get_notifications(10).await.unwrap()[0].dismissed,
            "mark_read must not be conflated with dismiss"
        );
        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(received.is_ok(), "mark_read must still be relayed");
    }

    // ── unauthorized access (auth gate lives in WsServer::handle_message) ────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn notification_post_unauthenticated_rejected_by_dispatcher_not_saved() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "unpaired_ws").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "notification",
            "action": "post",
            "id": "evil_n",
            "title": "should never land",
            "body": "b"
        }))
        .unwrap();

        crate::server::WsServer::handle_message(&text, "unpaired_ws", &ctx).await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("dispatcher must respond to unpaired client")
            .expect("broadcast recv");
        assert!(
            resp.contains("not_authenticated"),
            "expected not_authenticated rejection, got: {resp}"
        );
        assert!(
            ctx.storage.get_notifications(10).await.unwrap().is_empty(),
            "unauthenticated notification must never be persisted"
        );
    }
}
