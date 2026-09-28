use crate::AppState;
use crate::error::{ConduitError, Result};
use std::sync::Arc;
use tauri::State;

type ManagedState = Arc<AppState>;

#[tauri::command]
pub async fn get_clipboard_history(
    state: State<'_, ManagedState>,
    limit: Option<i64>,
) -> Result<Vec<crate::storage::ClipboardEntry>> {
    let limit = limit.unwrap_or(100);
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.get_clipboard_history(limit))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_clipboard_history task join error: {e}")))?
}

#[tauri::command]
pub async fn toggle_clipboard_pin(state: State<'_, ManagedState>, id: i64) -> Result<()> {
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.toggle_clipboard_pin(id))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("toggle_clipboard_pin task join error: {e}")))?
}

#[tauri::command]
pub async fn delete_clipboard_entry(state: State<'_, ManagedState>, id: i64) -> Result<()> {
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.delete_clipboard_entry(id))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("delete_clipboard_entry task join error: {e}")))?
}

#[tauri::command]
pub async fn clear_clipboard_history(state: State<'_, ManagedState>) -> Result<()> {
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.clear_clipboard_history())
    })
    .await
    .map_err(|e| ConduitError::Other(format!("clear_clipboard_history task join error: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{create_app_with_state, create_test_state};
    use tauri::Manager;

    async fn seed_clipboard(state: &Arc<AppState>, content: &str) -> i64 {
        state
            .storage
            .save_clipboard(
                content,
                "text/plain",
                "dev_a",
                chrono::Utc::now().timestamp(),
            )
            .await
            .unwrap();
        state
            .storage
            .get_clipboard_history(10)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.content == content)
            .expect("seeded entry")
            .id
    }

    // ── get_clipboard_history ─────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_clipboard_history_valid_respects_limit() {
        let state = create_test_state();
        for i in 0..3 {
            seed_clipboard(&state, &format!("entry {i}")).await;
        }
        let app = create_app_with_state(state.clone());

        let rows = get_clipboard_history(app.state(), Some(2)).await.unwrap();
        assert_eq!(rows.len(), 2, "limit=2 must cap results");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_clipboard_history_valid_none_limit_returns_entries() {
        let state = create_test_state();
        seed_clipboard(&state, "only").await;
        let app = create_app_with_state(state);

        let rows = get_clipboard_history(app.state(), None).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    // ── toggle_clipboard_pin ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn toggle_clipboard_pin_valid_pins_then_unpins() {
        let state = create_test_state();
        let id = seed_clipboard(&state, "pin me").await;
        let app = create_app_with_state(state.clone());

        toggle_clipboard_pin(app.state(), id).await.unwrap();
        let rows = state.storage.get_clipboard_history(10).await.unwrap();
        assert!(rows.iter().find(|e| e.id == id).unwrap().pinned);

        toggle_clipboard_pin(app.state(), id).await.unwrap();
        let rows = state.storage.get_clipboard_history(10).await.unwrap();
        assert!(!rows.iter().find(|e| e.id == id).unwrap().pinned);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn toggle_clipboard_pin_invalid_unknown_id_is_noop() {
        // CHARACTERIZATION: UPDATE on missing row → Ok (no-op). If this later
        // errors, rename → toggle_clipboard_pin_unknown_id_returns_error.
        let state = create_test_state();
        let app = create_app_with_state(state);

        toggle_clipboard_pin(app.state(), 999_999).await.unwrap();
    }

    // ── delete_clipboard_entry ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_clipboard_entry_valid_removes_row() {
        let state = create_test_state();
        let id = seed_clipboard(&state, "delete me").await;
        let app = create_app_with_state(state.clone());

        delete_clipboard_entry(app.state(), id).await.unwrap();

        assert!(
            state
                .storage
                .get_clipboard_history(10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── clear_clipboard_history ───────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clear_clipboard_history_valid_removes_unpinned_keeps_pinned() {
        let state = create_test_state();
        let pinned_id = seed_clipboard(&state, "pinned").await;
        seed_clipboard(&state, "unpinned").await;
        state.storage.toggle_clipboard_pin(pinned_id).await.unwrap();
        let app = create_app_with_state(state.clone());

        clear_clipboard_history(app.state()).await.unwrap();

        let rows = state.storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(rows.len(), 1, "only the pinned entry must survive");
        assert_eq!(rows[0].content, "pinned");
    }

    // ── missing state ─────────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "state() called before manage")]
    fn clipboard_commands_missing_state_fails_loudly() {
        let app = tauri::test::mock_app();
        let _ = app.state::<Arc<AppState>>();
    }
}
