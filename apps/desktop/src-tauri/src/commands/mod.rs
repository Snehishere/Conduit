mod clipboard;
mod file;
mod notifications;
mod pairing;
mod settings;
mod system;

// Re-export all types and functions at the `commands` level so
// that existing `commands::*` references in `main.rs` continue to work.

pub use clipboard::*;
pub use file::*;
pub use notifications::*;
pub use pairing::*;
pub use settings::*;
pub use system::*;

// ── Test helpers ──────────────────────────────────────────────────────────────
//
// Command functions take `tauri::State<'_, Arc<AppState>>`, which can only be
// produced from a (mock) Tauri app via `Manager::state()`. These helpers build
// an in-memory `AppState` and a `MockRuntime` app that manages it.

#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::AppState;
    use std::sync::Arc;

    /// Unique scratch directory under the OS temp dir.
    ///
    /// The file-transfer engine stages incoming chunks in a directory keyed
    /// by transfer id, and those files outlive the process. Tests that run
    /// concurrently in the same binary would otherwise share it.
    fn scratch_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "conduit_test_{label}_{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    /// Build an `AppState` backed by a fresh in-memory SQLite database
    /// (production schema) so command tests never touch user data.
    ///
    /// The file-transfer engine is attached to that database, so a test that
    /// writes a `default_download_folder` setting gets the engine to honour it
    /// exactly as production does.
    pub fn create_test_state() -> Arc<AppState> {
        let mut db = rusqlite::Connection::open_in_memory().expect("failed to open in-memory db");
        db.execute_batch("PRAGMA journal_mode=WAL;")
            .expect("failed to set WAL");
        crate::storage::run_migrations(&mut db).expect("failed to run migrations");
        let storage = Arc::new(crate::storage::Storage::from_connection(db));

        Arc::new(AppState {
            file_engine: Arc::new(
                crate::file_transfer::FileTransferEngine::new()
                    .with_storage(storage.clone())
                    .with_chunk_temp_dir(scratch_dir("chunks")),
            ),
            storage,
            sync_engine: Arc::new(tokio::sync::RwLock::new(crate::sync::SyncEngine::new())),
            ws_server: Arc::new(tokio::sync::RwLock::new(None)),
            discovery: Arc::new(tokio::sync::RwLock::new(None)),
            device_id: "test-hub-device".to_string(),
            encryption: crate::encryption::EncryptionManager::new_random(),
            token_store: Arc::new(crate::security::TokenStore::new(
                std::time::Duration::from_secs(60),
            )),
            automation_engine: Arc::new(tokio::sync::RwLock::new(
                crate::automation::AutomationEngine::new(),
            )),
            audio_stream: Arc::new(crate::audio::AudioStream::new()),
        })
    }

    /// Same as [`create_test_state`], but the file-transfer engine writes
    /// completed inbound files into `download_root` instead of the real user
    /// Downloads folder. Used by the `open_downloaded_file` tests, which have
    /// to prove containment against a root they own.
    pub fn create_test_state_with_download_root(
        download_root: std::path::PathBuf,
    ) -> Arc<AppState> {
        let mut db = rusqlite::Connection::open_in_memory().expect("failed to open in-memory db");
        db.execute_batch("PRAGMA journal_mode=WAL;")
            .expect("failed to set WAL");
        crate::storage::run_migrations(&mut db).expect("failed to run migrations");
        let storage = Arc::new(crate::storage::Storage::from_connection(db));

        Arc::new(AppState {
            file_engine: Arc::new(
                crate::file_transfer::FileTransferEngine::new()
                    .with_storage(storage.clone())
                    .with_download_root(download_root)
                    .with_chunk_temp_dir(scratch_dir("chunks")),
            ),
            storage,
            sync_engine: Arc::new(tokio::sync::RwLock::new(crate::sync::SyncEngine::new())),
            ws_server: Arc::new(tokio::sync::RwLock::new(None)),
            discovery: Arc::new(tokio::sync::RwLock::new(None)),
            device_id: "test-hub-device".to_string(),
            encryption: crate::encryption::EncryptionManager::new_random(),
            token_store: Arc::new(crate::security::TokenStore::new(
                std::time::Duration::from_secs(60),
            )),
            automation_engine: Arc::new(tokio::sync::RwLock::new(
                crate::automation::AutomationEngine::new(),
            )),
            audio_stream: Arc::new(crate::audio::AudioStream::new()),
        })
    }

    /// Mock Tauri app that manages `state`. Commands obtain their
    /// `State<'_, Arc<AppState>>` via `app.state::<Arc<AppState>>()`.
    ///
    /// The returned `App` must outlive any `State` borrowed from it.
    pub fn create_app_with_state(state: Arc<AppState>) -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("failed to build mock tauri app")
    }
}
