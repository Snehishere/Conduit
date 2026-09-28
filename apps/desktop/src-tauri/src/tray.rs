use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    Manager, WindowEvent,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use log::{info, warn};

use crate::AppState;

/// Cached `minimize_to_tray` setting.
///
/// The window `CloseRequested` event is delivered on the main thread inside the
/// Tauri event loop, where the async `Storage::get_settings` (which uses
/// `block_in_place`) cannot be awaited. The value is therefore mirrored into this
/// atomic: loaded once from the database by [`setup_tray`] and refreshed on every
/// write by `commands::save_settings` / `commands::save_settings_field`, so
/// toggling the setting takes effect without a restart.
static MINIMIZE_TO_TRAY: AtomicBool = AtomicBool::new(true);

/// Serialises the tests that read or write [`MINIMIZE_TO_TRAY`]; the cache is
/// process-global, so the tests asserting on it must not overlap.
#[cfg(test)]
pub(crate) static MINIMIZE_TO_TRAY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Lock the global cache for testing.
#[cfg(test)]
pub(crate) async fn lock_minimize_to_tray() -> tokio::sync::MutexGuard<'static, ()> {
    MINIMIZE_TO_TRAY_LOCK.lock().await
}

/// Record a newly persisted `minimize_to_tray` value.
pub fn set_minimize_to_tray(enabled: bool) {
    MINIMIZE_TO_TRAY.store(enabled, Ordering::Relaxed);
}

/// The `minimize_to_tray` value the window-close handler acts on.
pub fn minimize_to_tray() -> bool {
    MINIMIZE_TO_TRAY.load(Ordering::Relaxed)
}

pub fn setup_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "Show Conduit").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Conduit - Your devices, one web")
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    // Prime the cache from the database so the very first close already honours a
    // previously saved preference instead of the built-in default.
    if let Some(state) = try_managed_state(app) {
        let storage = state.storage.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    match storage.get_settings().await {
                        Ok(settings) => set_minimize_to_tray(settings.minimize_to_tray),
                        Err(e) => warn!(
                            "Failed to read minimize_to_tray at startup, keeping default true: {e}"
                        ),
                    }
                });
            }
            Err(e) => {
                warn!("No tokio runtime available to load minimize_to_tray: {e}");
            }
        }
    }

    // Enforce `minimize_to_tray`: with it on, closing the window only hides it —
    // the WS server, discovery and the tray icon keep running, and the user
    // brings the window back from the tray. With it off, the close proceeds
    // normally and the app exits.
    if let Some(window) = app.get_webview_window("main") {
        let to_hide = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if minimize_to_tray() {
                    info!("Window close intercepted: minimize_to_tray is enabled — hiding to tray");
                    let _ = to_hide.hide();
                    api.prevent_close();
                } else {
                    info!("Window close allowed: minimize_to_tray is disabled — exiting");
                }
            }
        });
    }

    Ok(())
}

/// The managed `Arc<AppState>`, if the app has registered it yet.
fn try_managed_state(app: &tauri::AppHandle) -> Option<Arc<AppState>> {
    app.try_state::<Arc<AppState>>()
        .map(|state| Arc::clone(state.inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimize_to_tray_default_is_true() {
        // Immutable data, no lock needed: a fresh install hides to tray, and this
        // must agree with `ConduitSettings::default()` (which the settings
        // command parity test pins to serde's `default_true`).
        assert!(crate::commands::ConduitSettings::default().minimize_to_tray);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_and_read_minimize_to_tray_roundtrip() {
        let _guard = lock_minimize_to_tray().await;

        set_minimize_to_tray(false);
        assert!(!minimize_to_tray(), "window close must not be intercepted");
        set_minimize_to_tray(true);
        assert!(minimize_to_tray(), "window close must be intercepted");
    }
}
