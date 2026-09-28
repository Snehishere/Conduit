use log::{error, info, warn};
use serde_json::Value;

use super::WsContext;

pub async fn handle_automation_rule(msg: Value, ctx: &WsContext) {
    let rule = match crate::automation::parse_rule_packet(&msg) {
        Ok(r) => r,
        Err(e) => {
            warn!("Automation rule rejected: {}", e);
            return;
        }
    };
    if let Err(e) = crate::automation::validate_rule(&rule) {
        warn!("Automation rule rejected ({}): {}", rule.id, e);
        return;
    }
    if let Err(e) = ctx.storage.save_automation_rule(&rule).await {
        error!("Automation rule: failed to save {}: {}", rule.id, e);
        return;
    }
    let reloaded = ctx
        .storage
        .get_all_automation_rules()
        .await
        .unwrap_or_default();
    let mut engine = ctx.automation_engine.write().await;
    engine.update_rule(rule.clone());
    engine.load_rules(reloaded);
    info!("Automation rule upserted: {} '{}'", rule.id, rule.name);
}

pub async fn handle_automation_delete(msg: Value, ctx: &WsContext) {
    let rule_id = msg
        .get("rule_id")
        .and_then(|v| v.as_str())
        .or_else(|| msg.get("id").and_then(|v| v.as_str()))
        .or_else(|| {
            msg.get("rule")
                .and_then(|r| r.get("id"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    if rule_id.is_empty() {
        warn!("Automation delete: missing rule_id");
        return;
    }
    if let Err(e) = ctx.storage.delete_automation_rule(rule_id).await {
        warn!("Automation delete: failed to delete {}: {}", rule_id, e);
        return;
    }
    ctx.automation_engine.write().await.remove_rule(rule_id);
    info!("Automation rule deleted: {}", rule_id);
}

pub async fn handle_automation_sync(msg: Value, ctx: &WsContext) {
    let Some(rules_json) = msg.get("rules") else {
        warn!("Automation sync: missing rules array");
        return;
    };
    let rules: Vec<Value> = match rules_json.as_array() {
        Some(arr) => arr.clone(),
        None => {
            error!("Automation sync: rules must be an array");
            return;
        }
    };

    let mut incoming_ids = Vec::new();
    for value in rules {
        let rule = match crate::automation::parse_rule_packet(&value) {
            Ok(r) => r,
            Err(e) => {
                warn!("Automation sync: skipping invalid rule: {}", e);
                continue;
            }
        };
        if let Err(e) = crate::automation::validate_rule(&rule) {
            warn!("Automation sync: skipping invalid rule {}: {}", rule.id, e);
            continue;
        }
        incoming_ids.push(rule.id.clone());
        if let Err(e) = ctx.storage.save_automation_rule(&rule).await {
            error!("Automation sync: failed to save rule {}: {}", rule.id, e);
        }
    }

    for existing in ctx
        .storage
        .get_all_automation_rules()
        .await
        .unwrap_or_default()
    {
        if !incoming_ids.contains(&existing.id) {
            let full_sync = msg
                .get("full_sync")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if full_sync {
                let _ = ctx.storage.delete_automation_rule(&existing.id).await;
            } else {
                info!(
                    "Automation sync: keeping existing rule {} (not in partial sync)",
                    existing.id
                );
            }
        }
    }

    let reloaded = ctx
        .storage
        .get_all_automation_rules()
        .await
        .unwrap_or_default();
    let rule_count = reloaded.len();
    let mut engine = ctx.automation_engine.write().await;
    engine.load_rules(reloaded);
    info!("Automation rules synced: {} rules", rule_count);
}

pub async fn handle_automation_triggered(msg: Value, ctx: &WsContext) {
    let rule_id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let trigger_type = msg
        .get("trigger_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let device_id = msg.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    if rule_id.is_empty() || trigger_type.is_empty() {
        warn!("Automation triggered: missing id or trigger_type");
        return;
    }

    let battery = if device_id.is_empty() {
        None
    } else {
        ctx.sync_engine
            .read()
            .await
            .connected_clients
            .get(device_id)
            .and_then(|c| c.battery_level)
    };

    let engine = ctx.automation_engine.read().await;
    let Some(rule) = engine.get_rule(rule_id) else {
        warn!("Automation triggered: unknown rule {}", rule_id);
        return;
    };

    if !crate::automation::re_evaluate_rule(rule, trigger_type, device_id, battery) {
        info!(
            "Automation triggered: rule {} skipped (not enabled / tag or scope mismatch)",
            rule_id
        );
        return;
    }

    let timestamp = chrono::Utc::now().timestamp();
    if crate::automation::is_desktop_executable(&rule.action) {
        let log = crate::automation::execute_action(&rule.action);
        let _ = ctx
            .storage
            .log_automation_execution(
                &rule.id,
                trigger_type,
                log.timestamp,
                log.success,
                log.message.as_deref(),
            )
            .await;
        info!("Automation: rule {} executed ({})", rule_id, trigger_type);
    } else {
        let _ = ctx
            .storage
            .log_automation_execution(
                &rule.id,
                trigger_type,
                timestamp,
                true,
                Some("action executes on the target phone/tablet; desktop logged only"),
            )
            .await;
        info!(
            "Automation: rule {} targets a remote device ({}), no desktop action",
            rule_id,
            crate::automation::action_tag(&rule.action)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{add_test_client, create_test_ctx};

    /// Wire-format packet the frontend sends: nested `rule` object holding
    /// `trigger` + `action`. Uses a `send_notification` action because it is
    /// desktop-executable but has NO real side effects (just logs a row).
    fn rule_packet(id: &str) -> Value {
        serde_json::json!({
            "type": "automation",
            "action": "rule",
            "rule": {
                "id": id,
                "name": format!("Rule {id}"),
                "enabled": true,
                "trigger": { "type": "device_connect", "device_id": "*" },
                "action": { "type": "send_notification", "title": "T", "body": "B" }
            }
        })
    }

    // ── handle_automation_rule ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_rule_happy_persists_and_loads_engine() {
        let ctx = create_test_ctx();

        handle_automation_rule(rule_packet("r1"), &ctx).await;

        let rules = ctx.storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 1, "valid rule must be saved");
        assert_eq!(rules[0].id, "r1");
        assert_eq!(rules[0].name, "Rule r1");

        let engine = ctx.automation_engine.read().await;
        assert!(
            engine.get_rule("r1").is_some(),
            "rule must be loaded into the automation engine"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_rule_invalid_missing_trigger_rejected() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({
            "type": "automation",
            "action": "rule",
            "rule": {
                "id": "bad1",
                "name": "No trigger",
                "action": { "type": "send_notification", "title": "t", "body": "b" }
            }
        });

        handle_automation_rule(msg, &ctx).await;

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty()
        );
        let engine = ctx.automation_engine.read().await;
        assert!(engine.get_rule("bad1").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_rule_invalid_time_fails_validation_not_saved() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({
            "type": "automation",
            "action": "rule",
            "rule": {
                "id": "bad2",
                "name": "Bad time",
                "trigger": { "type": "time", "time": "99:99" },
                "action": { "type": "send_notification", "title": "t", "body": "b" }
            }
        });

        handle_automation_rule(msg, &ctx).await;

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty(),
            "rule failing validate_rule must not be persisted"
        );
    }

    // ── handle_automation_delete ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_delete_happy_removes_from_storage_and_engine() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("del1"), &ctx).await;

        handle_automation_delete(serde_json::json!({ "rule_id": "del1" }), &ctx).await;

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty()
        );
        let engine = ctx.automation_engine.read().await;
        assert!(engine.get_rule("del1").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_delete_missing_id_is_noop() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("keep_me"), &ctx).await;

        // No rule_id / id / rule.id anywhere → warn and return.
        handle_automation_delete(serde_json::json!({}), &ctx).await;

        assert_eq!(
            ctx.storage.get_all_automation_rules().await.unwrap().len(),
            1,
            "delete without id must not remove anything"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_delete_edge_accepts_nested_rule_id_and_bare_id() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("n1"), &ctx).await;
        handle_automation_rule(rule_packet("n2"), &ctx).await;

        // Nested variant: { rule: { id } }
        handle_automation_delete(serde_json::json!({ "rule": { "id": "n1" } }), &ctx).await;
        // Bare-id variant: { id }
        handle_automation_delete(serde_json::json!({ "id": "n2" }), &ctx).await;

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── handle_automation_sync ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_sync_happy_saves_all_incoming_rules() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({
            "type": "automation",
            "action": "sync",
            "rules": [rule_packet("s1"), rule_packet("s2")]
        });

        handle_automation_sync(msg, &ctx).await;

        let rules = ctx.storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 2);
        let engine = ctx.automation_engine.read().await;
        assert!(engine.get_rule("s1").is_some() && engine.get_rule("s2").is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_sync_invalid_rules_not_array_rejected() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({
            "type": "automation",
            "action": "sync",
            "rules": "not-an-array"
        });

        handle_automation_sync(msg, &ctx).await;

        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_sync_missing_rules_key_is_noop() {
        let ctx = create_test_ctx();
        handle_automation_sync(serde_json::json!({ "type": "automation" }), &ctx).await;
        assert!(
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_sync_edge_skips_invalid_entries_keeps_valid() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({
            "type": "automation",
            "action": "sync",
            "rules": [
                { "id": "broken" },                       // no trigger → parse error
                rule_packet("good")
            ]
        });

        handle_automation_sync(msg, &ctx).await;

        let rules = ctx.storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 1, "only the valid rule must be saved");
        assert_eq!(rules[0].id, "good");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_sync_full_sync_removes_rules_absent_from_sync() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("stale"), &ctx).await;

        let msg = serde_json::json!({
            "type": "automation",
            "action": "sync",
            "full_sync": true,
            "rules": [rule_packet("fresh")]
        });
        handle_automation_sync(msg, &ctx).await;

        let rules = ctx.storage.get_all_automation_rules().await.unwrap();
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"fresh"));
        assert!(
            !ids.contains(&"stale"),
            "full_sync must delete absent rules"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_sync_partial_keeps_existing_rules_not_in_sync() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("local"), &ctx).await;

        let msg = serde_json::json!({
            "type": "automation",
            "action": "sync",
            "rules": [rule_packet("remote")]
            // no full_sync → partial sync
        });
        handle_automation_sync(msg, &ctx).await;

        let rules = ctx.storage.get_all_automation_rules().await.unwrap();
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"local"), "partial sync must keep local rules");
        assert!(ids.contains(&"remote"));
    }

    // ── handle_automation_triggered ───────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_triggered_happy_executes_and_writes_log() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("tr1"), &ctx).await;

        handle_automation_triggered(
            serde_json::json!({
                "id": "tr1",
                "trigger_type": "device_connect",
                "device_id": "dev_x"
            }),
            &ctx,
        )
        .await;

        let logs = ctx.storage.get_automation_logs(10).await.unwrap();
        assert_eq!(
            logs.len(),
            1,
            "matching trigger must write an execution log"
        );
        assert_eq!(logs[0].id, "tr1");
        assert!(logs[0].success);
        assert_eq!(logs[0].trigger_type, "device_connect");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_triggered_invalid_unknown_rule_writes_no_log() {
        let ctx = create_test_ctx();

        handle_automation_triggered(
            serde_json::json!({ "id": "ghost", "trigger_type": "device_connect" }),
            &ctx,
        )
        .await;

        assert!(
            ctx.storage
                .get_automation_logs(10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_triggered_missing_fields_is_noop() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("tr2"), &ctx).await;

        // Missing id AND trigger_type → early return.
        handle_automation_triggered(serde_json::json!({}), &ctx).await;

        assert!(
            ctx.storage
                .get_automation_logs(10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_triggered_edge_disabled_rule_is_skipped() {
        let ctx = create_test_ctx();
        let mut packet = rule_packet("off1");
        packet["rule"]["enabled"] = serde_json::json!(false);
        handle_automation_rule(packet, &ctx).await;

        handle_automation_triggered(
            serde_json::json!({ "id": "off1", "trigger_type": "device_connect" }),
            &ctx,
        )
        .await;

        assert!(
            ctx.storage
                .get_automation_logs(10)
                .await
                .unwrap()
                .is_empty(),
            "disabled rules must not execute"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_triggered_edge_mismatched_trigger_type_is_skipped() {
        let ctx = create_test_ctx();
        handle_automation_rule(rule_packet("tr3"), &ctx).await;

        // Rule triggers on device_connect, event says battery_level → skip.
        handle_automation_triggered(
            serde_json::json!({ "id": "tr3", "trigger_type": "battery_level" }),
            &ctx,
        )
        .await;

        assert!(
            ctx.storage
                .get_automation_logs(10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── unauthorized access (auth gate lives in WsServer::handle_message) ────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_rule_unauthenticated_rejected_by_dispatcher_not_saved() {
        let ctx = create_test_ctx();
        // Registered as a WS client, but NOT mapped in ws_to_device_id → unpaired.
        let tx = add_test_client(&ctx, "unpaired_ws").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&rule_packet("evil")).unwrap();
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
            ctx.storage
                .get_all_automation_rules()
                .await
                .unwrap()
                .is_empty(),
            "unauthenticated automation rule must never be saved"
        );
        let engine = ctx.automation_engine.read().await;
        assert!(engine.get_rule("evil").is_none());
    }
}
