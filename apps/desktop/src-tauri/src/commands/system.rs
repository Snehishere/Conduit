use crate::AppState;
use crate::error::{ConduitError, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

type ManagedState = Arc<AppState>;

#[derive(Serialize, Deserialize)]
pub struct SystemInfo {
    pub ram_used_mb: f64,
    pub ram_total_mb: f64,
    pub ram_percent: f64,
}

/// Best-effort LAN IPv4 address of this machine, for the pairing QR code.
///
/// Returns `Err` when no usable address exists instead of the old
/// `"127.0.0.1"` fallback. That string was a *plausible-looking lie*: it is a
/// syntactically valid address, so nothing downstream complained, but it
/// encodes the *phone's* loopback — pairing against it can never reach this
/// hub, and the user is left with a QR code that silently cannot work.
///
/// Loopback and virtual adapters are skipped in the first pass, and the
/// `local_ip_address` fallback is only trusted if it is a real IPv4 address.
#[tauri::command]
pub fn get_local_ip() -> Result<String> {
    if let Ok(interfaces) = get_if_addrs::get_if_addrs() {
        // Try to find a real physical network adapter (skip WSL, Hyper-V, Loopback)
        for iface in interfaces {
            if iface.is_loopback() {
                continue;
            }
            if iface.name.contains("vEthernet")
                || iface.name.contains("WSL")
                || iface.name.contains("Virtual")
                || iface.name.contains("Hyper")
            {
                continue;
            }
            if let get_if_addrs::IfAddr::V4(addr) = iface.addr {
                return Ok(addr.ip.to_string());
            }
        }
    }
    // Fallback if physical not found
    if let Ok(ip) = local_ip_address::local_ip()
        && ip.is_ipv4()
    {
        return Ok(ip.to_string());
    }
    Err(ConduitError::Storage(
        "No LAN IPv4 address found. This machine has no usable network adapter, so a phone \
         cannot reach it. Connect to Wi-Fi or Ethernet, then restart pairing. \
         (Returning 127.0.0.1 here would produce a QR code pointing at the phone's own \
         loopback address, which can never pair.)"
            .into(),
    ))
}

/// TCP ports the Windows Firewall must allow inbound for LAN pairing to work.
///
/// Derived from the shared protocol constants rather than literals: the bug
/// this replaces opened `9527,9528` — 9528 is the *relay* service's plaintext
/// port, and the TLS port 9531 that the mobile app actually dials was missing
/// entirely, so "Troubleshoot Connection" never unblocked WSS.
pub fn firewall_ports() -> Vec<u16> {
    vec![crate::WS_PORT, crate::WSS_PORT]
}

/// Comma-separated `-LocalPort` argument for `New-NetFirewallRule`.
fn firewall_local_port_arg() -> String {
    firewall_ports()
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[tauri::command]
pub async fn fix_firewall() -> Result<bool> {
    let script = format!(
        "New-NetFirewallRule -DisplayName 'Conduit App' -Direction Inbound \
         -LocalPort {} -Protocol TCP -Action Allow -ErrorAction SilentlyContinue",
        firewall_local_port_arg()
    );
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        script
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect::<Vec<u8>>(),
    );

    match std::process::Command::new("powershell")
        .arg("-Command")
        .arg(format!("Start-Process powershell -ArgumentList '-NoProfile -ExecutionPolicy Bypass -EncodedCommand {}' -Verb RunAs -WindowStyle Hidden", encoded))
        .spawn()
    {
        Ok(_) => Ok(true),
        Err(e) => Err(ConduitError::Other(format!("Failed to elevate privileges: {}", e))),
    }
}

// ── System info ──────────────────────────────────────────────────────────────

/// Bytes in one mebibyte. `SystemInfo` is megabytes and the status bar renders
/// these numbers verbatim as `${ramMb} MB`, so every conversion is explicit.
const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// Round to one decimal place, so the UI never shows float noise.
fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Memory of this process and of the machine, in megabytes.
///
/// **Unit note.** `sysinfo` 0.26.0 changed `Process::memory()` from kilobytes to
/// bytes ("Switch memory unit from kilobytes to bytes"), and this crate is on
/// `sysinfo = "0.39"`, so `Process::memory()` returns **bytes** and must be
/// divided by [`BYTES_PER_MB`]. `System::total_memory()` and
/// `System::available_memory()` are bytes on the same versions. Anything
/// assuming kilobytes here is wrong by three orders of magnitude.
///
/// `ram_total_mb` and `ram_percent` were previously hardcoded `0.0` despite the
/// struct declaring them as `f64`.
#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo> {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.refresh_memory();

    // `total_memory()` and `available_memory()` are bytes in sysinfo >= 0.26.
    let total_mb = round1(sys.total_memory() as f64 / BYTES_PER_MB);
    let used_mb = sys.total_memory().saturating_sub(sys.available_memory());
    let ram_percent = if sys.total_memory() == 0 {
        0.0
    } else {
        round1(used_mb as f64 / sys.total_memory() as f64 * 100.0)
    };

    // `ram_used_mb` has always meant *this process's* resident set — it is the
    // only per-process number available, and the status bar shows it next to the
    // device count. What changed is the unit, not the meaning.
    let ram_used_mb = sys
        .process(pid)
        .map(|proc| round1(proc.memory() as f64 / BYTES_PER_MB))
        .unwrap_or(0.0);

    Ok(SystemInfo {
        ram_used_mb,
        ram_total_mb: total_mb,
        ram_percent,
    })
}

// ── Auto-Update ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_current_version() -> Result<String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

// ─── Window State Persistence ───

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct WindowState {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub maximized: bool,
}

/// Largest window dimension accepted from the renderer, in logical pixels.
///
/// Anything beyond this is either corrupt JSON the frontend mis-parsed or a
/// caller trying to drive the window off-screen. `f64::INFINITY`/`NAN` would
/// otherwise reach `PhysicalPosition`/`PhysicalSize`, whose `as i32`/`as u32`
/// casts are saturating — so the window silently lands somewhere arbitrary.
const MAX_WINDOW_EXTENT: f64 = 100_000.0;

/// Reject renderer-supplied window state before it can corrupt the saved value.
///
/// Reported as [`ConduitError::Validation`] so the frontend can tell "the
/// window state you sent was nonsense" from "the database write failed" —
/// both used to arrive as an opaque `Other`.
impl WindowState {
    fn validate(&self) -> Result<()> {
        for (label, value) in [("x", self.x), ("y", self.y)] {
            if let Some(v) = value
                && (!v.is_finite() || v.abs() > MAX_WINDOW_EXTENT)
            {
                return Err(ConduitError::Validation(format!(
                    "window position `{label}` must be a finite number within ±{MAX_WINDOW_EXTENT}, got {v}"
                )));
            }
        }
        for (label, value) in [("width", self.width), ("height", self.height)] {
            match value {
                Some(v) if !v.is_finite() || v <= 0.0 || v > MAX_WINDOW_EXTENT => {
                    return Err(ConduitError::Validation(format!(
                        "window size `{label}` must be a finite number between 0 and \
                         {MAX_WINDOW_EXTENT}, got {v}"
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn save_window_state(
    state: State<'_, ManagedState>,
    window_state: WindowState,
) -> Result<()> {
    window_state.validate()?;
    let json =
        serde_json::to_string(&window_state).map_err(|e| ConduitError::Other(e.to_string()))?;
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.save_setting("window_state", &json))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("save_window_state task join error: {e}")))?
}

#[tauri::command]
pub async fn get_window_state(state: State<'_, ManagedState>) -> Result<WindowState> {
    let storage = state.storage.clone();
    let value = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.try_get_setting("window_state"))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_window_state task join error: {e}")))?;
    match value? {
        Some(json) => serde_json::from_str(&json).map_err(|e| ConduitError::Other(e.to_string())),
        None => Ok(WindowState::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{create_app_with_state, create_test_state};
    use tauri::Manager;

    // ── valid input ───────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_current_version_valid_returns_semver_string() {
        let v = get_current_version().await.unwrap();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
        assert!(!v.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_local_ip_valid_returns_parseable_ip() {
        // SKIP when the host genuinely has no LAN adapter (headless CI): the
        // command now reports that as an error instead of inventing an address.
        match get_local_ip() {
            Ok(ip) => assert!(
                ip.parse::<std::net::IpAddr>().is_ok(),
                "get_local_ip should return a parseable IP, got: {ip}"
            ),
            Err(e) => eprintln!("SKIP get_local_ip_valid_returns_parseable_ip: {e}"),
        }
    }

    /// REGRESSION — `get_local_ip` used to fall back to `"127.0.0.1"`, which is
    /// a syntactically valid address pointing at the *phone's own* loopback. The
    /// pairing QR code looked fine and could never work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_local_ip_never_reports_a_loopback_or_placeholder_address() {
        match get_local_ip() {
            Ok(ip) => {
                let parsed: std::net::IpAddr =
                    ip.parse().unwrap_or_else(|_| panic!("not an IP: {ip}"));
                assert!(
                    !parsed.is_loopback(),
                    "a loopback address cannot reach this hub from a phone: {ip}"
                );
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("No LAN IPv4 address found"),
                    "the error must explain the situation: {msg}"
                );
                assert!(
                    msg.contains("127.0.0.1"),
                    "the error must say why the old placeholder was removed: {msg}"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_system_info_valid_returns_non_negative_ram() {
        let info = get_system_info().await.unwrap();
        assert!(
            info.ram_used_mb >= 0.0,
            "ram_used_mb must be non-negative, got {}",
            info.ram_used_mb
        );
    }

    /// REGRESSION — `get_system_info` reported `proc.memory() / 1024.0` on the
    /// assumption that sysinfo returns kilobytes. sysinfo switched to **bytes**
    /// in 0.26.0 and this crate is on 0.39, so the "MB" figure was three
    /// orders of magnitude too small. `ram_total_mb` and `ram_percent` were
    /// hardcoded `0.0`.
    ///
    /// These assertions are the tripwire for the next unit change: any
    /// mis-scaling breaks the `total >= used` relation or pushes the percentage
    /// out of range, because a machine never has more RAM in use than it has.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_system_info_units_are_megabytes_and_are_self_consistent() {
        let info = get_system_info().await.unwrap();

        assert!(
            info.ram_total_mb > 0.0,
            "ram_total_mb must be populated, not hardcoded 0.0 (got {})",
            info.ram_total_mb
        );
        // A real machine has between ~16 MB and ~64 TB of RAM. Anything outside
        // that means the byte→MB conversion is wrong by a power of 1024.
        assert!(
            (16.0..=65_536.0).contains(&info.ram_total_mb),
            "ram_total_mb {} is not a plausible megabyte figure",
            info.ram_total_mb
        );
        assert!(
            info.ram_used_mb <= info.ram_total_mb,
            "this process cannot use more RAM than the machine has ({} > {})",
            info.ram_used_mb,
            info.ram_total_mb
        );
        assert!(
            (0.0..=100.0).contains(&info.ram_percent),
            "ram_percent {} must be a percentage",
            info.ram_percent
        );
        assert!(
            info.ram_percent > 0.0,
            "some memory is always in use; a hardcoded 0.0 means the field is not wired up"
        );
    }

    /// The reported numbers are rounded to one decimal, so the status bar never
    /// shows float noise. (A strict `== (v * 10).round() / 10.0` would be
    /// flaky: `1.3 * 10.0` is not exactly `13.0` in binary floating point.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_system_info_values_are_rounded_to_one_decimal() {
        let info = get_system_info().await.unwrap();
        for (label, value) in [
            ("ram_used_mb", info.ram_used_mb),
            ("ram_total_mb", info.ram_total_mb),
            ("ram_percent", info.ram_percent),
        ] {
            let tenths = value * 10.0;
            assert!(
                (tenths - tenths.round()).abs() < 1e-9,
                "{label} = {value} is not rounded to one decimal place"
            );
        }
    }

    #[test]
    fn megabyte_conversion_is_exact() {
        assert_eq!(BYTES_PER_MB, 1_048_576.0);
        // 512 MB expressed in bytes, converted back.
        assert_eq!((512.0 * 1024.0 * 1024.0) / BYTES_PER_MB, 512.0);
        assert_eq!(round1(1.24), 1.2);
        assert_eq!(round1(1.25), 1.3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_window_state_valid_roundtrips_through_settings() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let saved = WindowState {
            x: Some(10.0),
            y: Some(20.0),
            width: Some(800.0),
            height: Some(600.0),
            maximized: true,
        };
        save_window_state(app.state(), saved).await.unwrap();

        let loaded = get_window_state(app.state()).await.unwrap();
        assert_eq!(loaded.x, Some(10.0));
        assert_eq!(loaded.y, Some(20.0));
        assert_eq!(loaded.width, Some(800.0));
        assert_eq!(loaded.height, Some(600.0));
        assert!(loaded.maximized);
    }

    // ── invalid input ─────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_window_state_invalid_corrupt_json_returns_error() {
        let state = create_test_state();
        // Corrupt the persisted value directly — simulates manual DB tampering.
        state
            .storage
            .save_setting("window_state", "{definitely-not-json")
            .await
            .unwrap();
        let app = create_app_with_state(state);

        let result = get_window_state(app.state()).await;
        assert!(
            result.is_err(),
            "corrupt window_state JSON must produce Err, got Ok({:?})",
            result.ok()
        );
    }

    // ── edge case ─────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_window_state_missing_returns_default() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let ws = get_window_state(app.state()).await.unwrap();
        // WindowState has no PartialEq — compare field-by-field.
        assert_eq!(ws.x, None);
        assert_eq!(ws.y, None);
        assert_eq!(ws.width, None);
        assert_eq!(ws.height, None);
        assert!(!ws.maximized);
    }

    // ── window-state validation (ConduitError::Validation) ────────────────────

    /// A bad value from the renderer must be reported as *the caller's fault*,
    /// with the `VALIDATION: ` prefix the frontend branches on, not as a
    /// generic `Other` that is indistinguishable from a database failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_window_state_invalid_nan_size_returns_a_validation_error() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let err = save_window_state(
            app.state(),
            WindowState {
                x: Some(0.0),
                y: Some(0.0),
                width: Some(f64::NAN),
                height: Some(600.0),
                maximized: false,
            },
        )
        .await
        .expect_err("a NaN width must be rejected");
        assert!(
            matches!(err, ConduitError::Validation(_)),
            "expected Validation, got {err:?}"
        );
        assert!(
            err.to_string().starts_with(crate::error::VALIDATION_PREFIX),
            "the message must carry the JS-facing prefix: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_window_state_invalid_geometry_is_rejected() {
        let cases = [
            (
                "negative width",
                WindowState {
                    width: Some(-1.0),
                    ..Default::default()
                },
            ),
            (
                "zero height",
                WindowState {
                    height: Some(0.0),
                    ..Default::default()
                },
            ),
            (
                "infinite x",
                WindowState {
                    x: Some(f64::INFINITY),
                    ..Default::default()
                },
            ),
            (
                "absurd width",
                WindowState {
                    width: Some(MAX_WINDOW_EXTENT * 10.0),
                    ..Default::default()
                },
            ),
            (
                "absurd x",
                WindowState {
                    x: Some(-MAX_WINDOW_EXTENT * 10.0),
                    ..Default::default()
                },
            ),
        ];
        for (label, ws) in cases {
            let err = ws
                .validate()
                .expect_err(&format!("{label} must be rejected"));
            assert!(
                matches!(err, ConduitError::Validation(_)),
                "{label}: expected Validation, got {err:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_window_state_valid_still_round_trips() {
        // The validation must not break the ordinary path, and a rejected
        // geometry must not be persisted.
        let state = create_test_state();
        let app = create_app_with_state(state);
        let ws = WindowState {
            x: Some(10.0),
            y: Some(20.0),
            width: Some(800.0),
            height: Some(600.0),
            maximized: true,
        };
        ws.validate().expect("a sane window state is accepted");

        save_window_state(app.state(), ws.clone()).await.unwrap();
        let loaded = get_window_state(app.state()).await.unwrap();
        assert_eq!(loaded.x, Some(10.0));
        assert_eq!(loaded.y, Some(20.0));
        assert_eq!(loaded.width, Some(800.0));
        assert_eq!(loaded.height, Some(600.0));
        assert!(loaded.maximized);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_window_state_invalid_input_is_not_persisted() {
        // Rejected input must not reach the database, or a corrupt geometry
        // would be restored on every subsequent launch.
        let state = create_test_state();
        let app = create_app_with_state(state.clone());
        let result = save_window_state(
            app.state(),
            WindowState {
                width: Some(-5.0),
                ..Default::default()
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            state.storage.get_setting("window_state").await,
            None,
            "a rejected window state must not be written"
        );
    }

    // ── missing state ─────────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "state() called before manage")]
    fn system_commands_missing_state_fails_loudly() {
        let app = tauri::test::mock_app();
        let _ = app.state::<Arc<AppState>>();
    }

    // NOTE: `fix_firewall` is intentionally NOT called from a test — it spawns
    // an elevated PowerShell process (UAC prompt) as a side effect. Its
    // firewall rule *arguments* are covered by the pure-function tests below.

    // ── firewall port list ──────────────────────────────────────────────────

    #[test]
    fn firewall_ports_include_the_tls_lan_port() {
        // The mobile app only ever dials `wss://` on LAN. If this port is not
        // opened, the handshake is blocked by the firewall and pairing can
        // never complete, regardless of any other fix.
        assert!(
            firewall_ports().contains(&crate::WSS_PORT),
            "firewall_ports() must include the TLS LAN port {}",
            crate::WSS_PORT
        );
    }

    #[test]
    fn firewall_ports_include_the_plaintext_lan_port() {
        assert!(
            firewall_ports().contains(&crate::WS_PORT),
            "firewall_ports() must include the plaintext LAN port {}",
            crate::WS_PORT
        );
    }

    #[test]
    fn firewall_ports_are_exactly_the_two_desktop_listeners() {
        assert_eq!(firewall_ports(), vec![crate::WS_PORT, crate::WSS_PORT]);
    }

    #[test]
    fn firewall_ports_exclude_the_dead_relay_port() {
        // 9528 is the relay service's plaintext port, not a desktop listener.
        // Opening it was never useful and gave users a false sense of repair.
        assert!(
            !firewall_ports().contains(&9528),
            "9528 is the relay port and must not be opened by the desktop"
        );
    }

    #[test]
    fn firewall_ports_have_no_duplicates() {
        let ports = firewall_ports();
        let mut deduped = ports.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), ports.len());
    }

    #[test]
    fn firewall_local_port_arg_matches_the_constants() {
        assert_eq!(firewall_local_port_arg(), "9527,9531");
        assert_eq!(
            firewall_local_port_arg(),
            format!("{},{}", crate::WS_PORT, crate::WSS_PORT)
        );
    }
}
