//! `remote_input` — injects keyboard/mouse events into *this* machine.
//!
//! The wire contract is the `RemoteInput*` structs in `conduit-protocol`
//! (`move` | `click` | `scroll` | `key`). Frames are deserialised into those
//! types instead of being read field-by-field out of `serde_json::Value`, so a
//! client cannot invent a field name the injector silently ignores.
//!
//! Direction: the sender is the input *source*, the receiver injects. When the
//! frame names a different connected device in its `device_id` routing envelope
//! the frame is forwarded to that device (the desktop UI driving a phone);
//! otherwise the frame is injected locally through [`ENIGO`] (a phone acting as
//! a trackpad for this desktop).

use std::sync::LazyLock;

use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use log::{debug, warn};
use serde_json::Value;
use tokio::sync::Mutex;

use conduit_protocol::types::{
    RemoteInputClick, RemoteInputKey, RemoteInputMove, RemoteInputScroll,
};

use super::screen_mirror::resolve_target_client;
use super::{WsContext, send_to_client};

/// Process-wide input injector, shared with `screen_mirror`.
///
/// One instance on purpose: two independent `Enigo` handles would interleave
/// modifier key-down state and produce chords that never release.
pub(crate) static ENIGO: LazyLock<Mutex<Enigo>> =
    LazyLock::new(|| Mutex::new(Enigo::new(&Settings::default()).unwrap()));

/// Longest key name accepted for injection, in characters.
///
/// A `KeyboardEvent.key` value is either one character or a key name
/// (`"ArrowLeft"`, `"F11"`, …). Anything longer is a protocol violation, not
/// text to type — without this cap a `key` frame could inject an arbitrary
/// string into whatever window holds focus.
pub(crate) const MAX_KEY_LEN: usize = 32;

/// Largest pointer-move / scroll magnitude accepted from the wire.
pub(crate) const MAX_DELTA: f64 = 4096.0;

/// Mouse buttons the protocol defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseButton {
    Left,
    Right,
    Middle,
    /// Left button, clicked twice.
    DoubleLeft,
}

impl MouseButton {
    /// Map the protocol's `button` field. `None` for anything else, so an
    /// unknown button is rejected instead of silently becoming a left click.
    pub(crate) fn parse(button: &str) -> Option<Self> {
        match button {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            "double_left" => Some(Self::DoubleLeft),
            _ => None,
        }
    }
}

/// A validated `remote_input` frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RemoteInputCommand {
    Move {
        dx: f64,
        dy: f64,
    },
    Click {
        button: MouseButton,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    Key {
        key: String,
        modifiers: Vec<String>,
    },
    /// `action: "start" | "stop"`.
    ///
    /// The protocol has no `remote_input` session message, so these carry no
    /// state: they are accepted and ignored rather than warned about, because
    /// an older mobile build still sends them on every open/close.
    Session {
        action: &'static str,
    },
    /// Any other action.
    Unknown {
        action: String,
    },
}

/// Parse a `remote_input` frame into a [`RemoteInputCommand`].
///
/// `action` is the dispatch key and each arm then deserialises the *protocol
/// type* for that action, so a frame can never be read with the wrong field
/// names (`dx` vs `delta`, `actionType` vs `action_type`, …).
pub(crate) fn parse_command(msg: &Value) -> Result<RemoteInputCommand, String> {
    let action = msg.get("action").and_then(Value::as_str).unwrap_or("");

    match action {
        "move" => {
            let parsed: RemoteInputMove =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(RemoteInputCommand::Move {
                dx: parsed.dx,
                dy: parsed.dy,
            })
        }
        "click" => {
            let parsed: RemoteInputClick =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            let button = MouseButton::parse(&parsed.button)
                .ok_or_else(|| format!("unknown remote_input button: {}", parsed.button))?;
            Ok(RemoteInputCommand::Click { button })
        }
        "scroll" => {
            let parsed: RemoteInputScroll =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(RemoteInputCommand::Scroll {
                dx: parsed.dx,
                dy: parsed.dy,
            })
        }
        "key" => {
            let parsed: RemoteInputKey =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(RemoteInputCommand::Key {
                key: parsed.key,
                modifiers: parsed.modifiers.unwrap_or_default(),
            })
        }
        "start" | "stop" => Ok(RemoteInputCommand::Session {
            action: if action == "start" { "start" } else { "stop" },
        }),
        other => Ok(RemoteInputCommand::Unknown {
            action: other.to_string(),
        }),
    }
}

pub async fn handle_remote_input(msg: Value, client_id: &str, ctx: &WsContext) {
    let command = match parse_command(&msg) {
        Ok(command) => command,
        Err(reason) => {
            warn!("Invalid remote_input frame from {}: {}", client_id, reason);
            return;
        }
    };

    if let RemoteInputCommand::Unknown { action } = &command {
        warn!("Unknown remote_input action: {}", action);
        return;
    }

    // A frame addressed at another connected device is that device's input,
    // not ours — forward it untouched so the receiving injector owns it.
    let target_device_id = msg.get("device_id").and_then(Value::as_str);
    if let Some(target) = resolve_target_client(ctx, client_id, target_device_id).await {
        debug!(
            "Forwarding remote_input {:?} from {} to {}",
            command, client_id, target
        );
        if !send_to_client(ctx, &target, &msg.to_string()).await {
            warn!("Remote input target {} unreachable — frame dropped", target);
        }
        return;
    }

    match command {
        RemoteInputCommand::Move { dx, dy } => {
            let dx = clamp_delta_steps(dx, MAX_DELTA);
            let dy = clamp_delta_steps(dy, MAX_DELTA);
            if dx == 0 && dy == 0 {
                return;
            }
            let mut enigo = ENIGO.lock().await;
            let _ = enigo.move_mouse(dx, dy, Coordinate::Rel);
        }
        RemoteInputCommand::Click { button } => {
            let mut enigo = ENIGO.lock().await;
            let (button, clicks) = match button {
                MouseButton::Left => (Button::Left, 1),
                MouseButton::Right => (Button::Right, 1),
                MouseButton::Middle => (Button::Middle, 1),
                MouseButton::DoubleLeft => (Button::Left, 2),
            };
            for _ in 0..clicks {
                let _ = enigo.button(button, Direction::Click);
            }
        }
        RemoteInputCommand::Scroll { dx, dy } => {
            let dx = clamp_delta_steps(dx, MAX_DELTA);
            let dy = clamp_delta_steps(dy, MAX_DELTA);
            if dx == 0 && dy == 0 {
                return;
            }
            let mut enigo = ENIGO.lock().await;
            let _ = enigo.scroll(dy, enigo::Axis::Vertical);
            let _ = enigo.scroll(dx, enigo::Axis::Horizontal);
        }
        RemoteInputCommand::Key { key, modifiers } => {
            if let KeyOutcome::Rejected(reason) = inject_key_chord(&key, &modifiers).await {
                warn!("Rejected remote_input key: {}", reason);
            }
        }
        // No session state exists for remote_input; see `Session`.
        RemoteInputCommand::Session { action } => {
            debug!("Ignoring stateless remote_input action: {}", action);
        }
        RemoteInputCommand::Unknown { .. } => unreachable!("handled before dispatch"),
    }
}

// ── Input helpers (shared with `screen_mirror`) ─────────────────────────────

/// Outcome of [`inject_key_chord`], so a rejection can be logged rather than
/// silently swallowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyOutcome {
    /// A named key (or single character) was injected.
    Injected,
    /// The frame named something that is neither: nothing was injected.
    Rejected(&'static str),
}

/// Clamp a wire coordinate to the protocol's `0.0..=1.0` relative range.
///
/// `NaN` maps to `0.0` so a malformed frame can never poison the pixel
/// conversion below.
pub(crate) fn clamp_relative(value: f64) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

/// Convert a clamped relative coordinate to absolute pixels on `extent`.
///
/// The cast used to be `(rel_x * width) as i32` on an unclamped value, so a
/// frame carrying `1e18` saturated the conversion and threw the cursor to an
/// arbitrary corner of the screen.
pub(crate) fn relative_to_px(relative: f64, extent: u32) -> i32 {
    let scaled = clamp_relative(relative) * f64::from(extent);
    scaled.clamp(0.0, f64::from(i32::MAX)) as i32
}

/// Clamp a wire delta to `±limit` and round it to whole pixels / steps.
pub(crate) fn clamp_delta_steps(value: f64, limit: f64) -> i32 {
    if value.is_nan() {
        return 0;
    }
    value.clamp(-limit, limit).round() as i32
}

/// Map a key name to an enigo key. Single characters are *not* handled here.
pub(crate) fn map_key_name(name: &str) -> Option<Key> {
    Some(match name {
        "Enter" | "enter" => Key::Return,
        "Backspace" | "backspace" => Key::Backspace,
        "Tab" | "tab" => Key::Tab,
        "Escape" | "escape" | "esc" => Key::Escape,
        "Delete" | "delete" | "del" => Key::Delete,
        "ArrowUp" | "arrowUp" | "Up" => Key::UpArrow,
        "ArrowDown" | "arrowDown" | "Down" => Key::DownArrow,
        "ArrowLeft" | "arrowLeft" | "Left" => Key::LeftArrow,
        "ArrowRight" | "arrowRight" | "Right" => Key::RightArrow,
        "Home" | "home" => Key::Home,
        "End" | "end" => Key::End,
        "PageUp" | "pageUp" => Key::PageUp,
        "PageDown" | "pageDown" => Key::PageDown,
        " " | "Space" | "space" => Key::Space,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        _ => return None,
    })
}

/// Map a modifier name to an enigo key.
///
/// The protocol spells the wire values `shift` | `control` | `alt` | `meta`;
/// the `*Left` / `*Right` spellings are accepted because both desktop and
/// mobile keyboards report them.
pub(crate) fn map_modifier(name: &str) -> Option<Key> {
    Some(match name {
        "shift" | "shiftLeft" | "shiftRight" => Key::Shift,
        "control" | "controlLeft" | "controlRight" | "ctrl" => Key::Control,
        "alt" | "altLeft" | "altRight" => Key::Alt,
        "meta" | "metaLeft" | "metaRight" | "command" => Key::Meta,
        _ => return None,
    })
}

/// Inject one key chord: modifiers down, key, modifiers up (reverse order).
///
/// Rejects anything that is not a key name or a single character. The previous
/// behaviour fell through to `enigo.text(key.to_lowercase())` for unrecognised
/// multi-character keys, which typed an arbitrary attacker-supplied string
/// into whatever window had focus.
pub(crate) async fn inject_key_chord(key: &str, modifiers: &[String]) -> KeyOutcome {
    if key.is_empty() {
        return KeyOutcome::Rejected("empty key");
    }
    if key.chars().count() > MAX_KEY_LEN {
        return KeyOutcome::Rejected("key longer than MAX_KEY_LEN");
    }

    let mut enigo = ENIGO.lock().await;

    // Dedupe while preserving order: a repeated modifier would otherwise be
    // pressed twice and released once.
    let mut held: Vec<Key> = Vec::new();
    for modifier in modifiers {
        if let Some(mapped) = map_modifier(modifier)
            && !held.contains(&mapped)
        {
            held.push(mapped);
        }
    }
    for keycode in &held {
        let _ = enigo.key(*keycode, Direction::Press);
    }

    if let Some(mapped) = map_key_name(key) {
        let _ = enigo.key(mapped, Direction::Press);
        let _ = enigo.key(mapped, Direction::Release);
    } else if key.chars().count() == 1 {
        // A single printable character: Unicode text entry, case preserved.
        let _ = enigo.text(key);
    } else {
        for keycode in held.iter().rev() {
            let _ = enigo.key(*keycode, Direction::Release);
        }
        return KeyOutcome::Rejected("unrecognised key name");
    }

    for keycode in held.iter().rev() {
        let _ = enigo.key(*keycode, Direction::Release);
    }
    KeyOutcome::Injected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{
        add_test_client, add_test_client_mapped, add_test_unpaired_client, create_test_ctx,
    };

    // SAFETY: no test in this module injects anything. The injection paths are
    // reached only through a non-zero delta, a real click, or a key frame that
    // is a named key / single character; every test below either uses a
    // zero/malformed/unknown action or asserts on the pure parse + clamp
    // helpers. `inject_key_chord` is exercised only with input it rejects
    // before touching `ENIGO`.

    // ── action dispatch (the bug: every frame hit the `_` arm) ──────────────

    #[test]
    fn parse_command_dispatches_move_with_deltas() {
        let msg =
            serde_json::json!({ "type": "remote_input", "action": "move", "dx": 12, "dy": -3 });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Move { dx: 12.0, dy: -3.0 }
        );
    }

    #[test]
    fn parse_command_dispatches_click_per_button() {
        for (wire, expected) in [
            ("left", MouseButton::Left),
            ("right", MouseButton::Right),
            ("middle", MouseButton::Middle),
            ("double_left", MouseButton::DoubleLeft),
        ] {
            let msg =
                serde_json::json!({ "type": "remote_input", "action": "click", "button": wire });
            assert_eq!(
                parse_command(&msg).unwrap(),
                RemoteInputCommand::Click { button: expected },
                "button {wire} must map to its own variant"
            );
        }
    }

    #[test]
    fn parse_command_dispatches_scroll_with_deltas() {
        let msg = serde_json::json!({ "type": "remote_input", "action": "scroll", "dx": 1.5, "dy": -3.5 });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Scroll { dx: 1.5, dy: -3.5 }
        );
    }

    #[test]
    fn parse_command_dispatches_key_with_modifiers() {
        let msg = serde_json::json!({
            "type": "remote_input", "action": "key", "key": "c",
            "modifiers": ["control", "shift"]
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Key {
                key: "c".into(),
                modifiers: vec!["control".into(), "shift".into()],
            }
        );
    }

    #[test]
    fn parse_command_key_without_modifiers_defaults_to_empty() {
        let msg = serde_json::json!({ "type": "remote_input", "action": "key", "key": "Enter" });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Key {
                key: "Enter".into(),
                modifiers: vec![],
            }
        );
    }

    /// The exact frame the desktop UI used to send for every keystroke. It must
    /// not reach the key branch — that is the whole defect.
    #[test]
    fn parse_command_legacy_event_envelope_is_not_a_key() {
        let msg = serde_json::json!({
            "type": "remote_input", "action": "event",
            "event_type": "key_press", "key": "a"
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Unknown {
                action: "event".into()
            }
        );
    }

    #[test]
    fn parse_command_legacy_start_and_stop_are_stateless_session_frames() {
        for action in ["start", "stop"] {
            let msg = serde_json::json!({ "type": "remote_input", "action": action });
            assert_eq!(
                parse_command(&msg).unwrap(),
                RemoteInputCommand::Session { action }
            );
        }
    }

    #[test]
    fn parse_command_unknown_action_is_reported_not_guessed() {
        let msg = serde_json::json!({ "type": "remote_input", "action": "launch_missiles" });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Unknown {
                action: "launch_missiles".into()
            }
        );
    }

    #[test]
    fn parse_command_missing_action_is_unknown_empty() {
        let msg = serde_json::json!({ "type": "remote_input" });
        assert_eq!(
            parse_command(&msg).unwrap(),
            RemoteInputCommand::Unknown {
                action: String::new()
            }
        );
    }

    // ── malformed frames are rejected, not half-applied ────────────────────

    #[test]
    fn parse_command_move_requires_numeric_deltas() {
        let msg = serde_json::json!({ "action": "move", "dx": "left", "dy": "up" });
        assert!(parse_command(&msg).is_err());
    }

    #[test]
    fn parse_command_move_requires_both_deltas() {
        assert!(parse_command(&serde_json::json!({ "action": "move" })).is_err());
        assert!(parse_command(&serde_json::json!({ "action": "move", "dx": 1 })).is_err());
    }

    #[test]
    fn parse_command_click_requires_a_known_button() {
        assert!(parse_command(&serde_json::json!({ "action": "click" })).is_err());
        assert!(
            parse_command(&serde_json::json!({ "action": "click", "button": "thumb" })).is_err()
        );
    }

    #[test]
    fn parse_command_key_requires_a_key_field() {
        assert!(parse_command(&serde_json::json!({ "action": "key" })).is_err());
    }

    // ── clamping ────────────────────────────────────────────────────────────

    #[test]
    fn clamp_relative_bounds_values_to_the_unit_interval() {
        assert_eq!(clamp_relative(0.0), 0.0);
        assert_eq!(clamp_relative(0.5), 0.5);
        assert_eq!(clamp_relative(1.0), 1.0);
        assert_eq!(clamp_relative(-1e18), 0.0);
        assert_eq!(clamp_relative(1e18), 1.0);
        assert_eq!(clamp_relative(f64::NAN), 0.0);
    }

    #[test]
    fn relative_to_px_saturates_instead_of_overflowing() {
        assert_eq!(relative_to_px(0.0, 1920), 0);
        assert_eq!(relative_to_px(0.5, 1920), 960);
        assert_eq!(relative_to_px(1.0, 1920), 1920);
        // 1e18 * 1920 saturated the old `as i32` cast.
        assert_eq!(relative_to_px(1e18, 1920), 1920);
        assert_eq!(relative_to_px(-1e18, 1080), 0);
    }

    #[test]
    fn clamp_delta_steps_caps_and_rounds() {
        assert_eq!(clamp_delta_steps(0.0, MAX_DELTA), 0);
        assert_eq!(clamp_delta_steps(4.4, MAX_DELTA), 4);
        assert_eq!(clamp_delta_steps(-4.6, MAX_DELTA), -5);
        assert_eq!(clamp_delta_steps(1e18, MAX_DELTA), MAX_DELTA as i32);
        assert_eq!(clamp_delta_steps(-1e18, MAX_DELTA), -(MAX_DELTA as i32));
        assert_eq!(clamp_delta_steps(f64::NAN, MAX_DELTA), 0);
    }

    // ── key handling ────────────────────────────────────────────────────────

    #[test]
    fn map_key_name_covers_named_keys_only() {
        assert_eq!(map_key_name("Enter"), Some(Key::Return));
        assert_eq!(map_key_name("ArrowLeft"), Some(Key::LeftArrow));
        assert_eq!(map_key_name("F12"), Some(Key::F12));
        assert_eq!(map_key_name("a"), None);
        assert_eq!(map_key_name("not a key"), None);
    }

    #[test]
    fn map_modifier_accepts_the_protocol_spelling() {
        assert_eq!(map_modifier("shift"), Some(Key::Shift));
        assert_eq!(map_modifier("control"), Some(Key::Control));
        assert_eq!(map_modifier("alt"), Some(Key::Alt));
        assert_eq!(map_modifier("meta"), Some(Key::Meta));
        assert_eq!(map_modifier("hyper"), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inject_key_chord_rejects_empty_key() {
        assert_eq!(
            inject_key_chord("", &[]).await,
            KeyOutcome::Rejected("empty key")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inject_key_chord_rejects_overlong_key_before_typing_it() {
        let key = "x".repeat(MAX_KEY_LEN + 1);
        assert_eq!(
            inject_key_chord(&key, &[]).await,
            KeyOutcome::Rejected("key longer than MAX_KEY_LEN")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inject_key_chord_rejects_unrecognised_multi_char_key() {
        // Previously this reached `enigo.text(key.to_lowercase())`, typing an
        // arbitrary string into the focused window.
        assert_eq!(
            inject_key_chord("rm -rf", &[]).await,
            KeyOutcome::Rejected("unrecognised key name")
        );
    }

    // ── handler: shapes and routing ─────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_move_happy_zero_delta_does_not_panic() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({ "type": "remote_input", "action": "move", "dx": 0, "dy": 0 });
        // Delta 0,0 → the zero-delta guard short-circuits; nothing visible happens.
        handle_remote_input(msg, "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_invalid_move_missing_coordinates_is_noop() {
        let ctx = create_test_ctx();
        // No dx/dy → the protocol type rejects the frame → nothing injected.
        handle_remote_input(serde_json::json!({ "action": "move" }), "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_invalid_wrong_coordinate_types_is_noop() {
        let ctx = create_test_ctx();
        // Strings don't parse as f64 → nothing injected.
        let msg = serde_json::json!({ "action": "move", "dx": "left", "dy": "up" });
        handle_remote_input(msg, "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_invalid_click_without_button_is_noop() {
        let ctx = create_test_ctx();
        // button missing → protocol type rejects the frame → no click.
        handle_remote_input(serde_json::json!({ "action": "click" }), "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_unknown_action_is_noop() {
        let ctx = create_test_ctx();
        // Unknown action → warn only.
        handle_remote_input(
            serde_json::json!({ "action": "launch_missiles" }),
            "c1",
            &ctx,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_scroll_zero_delta_is_guarded_noop() {
        let ctx = create_test_ctx();
        let msg = serde_json::json!({ "action": "scroll", "dx": 0, "dy": 0 });
        handle_remote_input(msg, "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_missing_action_is_noop() {
        let ctx = create_test_ctx();
        handle_remote_input(serde_json::json!({}), "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_legacy_start_and_stop_frames_are_inert() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();
        handle_remote_input(
            serde_json::json!({ "type": "remote_input", "action": "start", "device_id": "d1" }),
            "c1",
            &ctx,
        )
        .await;
        handle_remote_input(
            serde_json::json!({ "type": "remote_input", "action": "stop", "device_id": "d1" }),
            "c1",
            &ctx,
        )
        .await;
        let got = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(got.is_err(), "session frames must not produce a response");
    }

    /// A frame addressed at another connected device is forwarded, not
    /// injected locally: the desktop UI drives the phone, so injecting here
    /// would move this machine's cursor instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_addressed_to_another_device_is_forwarded() {
        let ctx = create_test_ctx();
        let _local = add_test_client(&ctx, "local_desktop").await;
        let phone_tx = add_test_client_mapped(&ctx, "ws_phone", "phone-1").await;
        let mut phone_rx = phone_tx.subscribe();

        let msg = serde_json::json!({
            "type": "remote_input", "action": "move", "dx": 0, "dy": 0, "device_id": "phone-1"
        });
        handle_remote_input(msg, "local_desktop", &ctx).await;

        let forwarded =
            tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv())
                .await
                .expect("frame must be forwarded to the named device")
                .expect("broadcast recv");
        let value: serde_json::Value = serde_json::from_str(&forwarded).unwrap();
        assert_eq!(value["action"], "move");
        assert_eq!(value["device_id"], "phone-1");
    }

    /// A frame whose `device_id` is not connected is injected locally — that is
    /// the phone-as-trackpad flow, which sends the desktop's own device id.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_unreachable_device_id_falls_back_to_local_injection() {
        let ctx = create_test_ctx();
        let _local = add_test_client(&ctx, "local_desktop").await;
        let tx = add_test_client(&ctx, "local_desktop").await;
        let mut rx = tx.subscribe();

        handle_remote_input(
            serde_json::json!({
                "type": "remote_input", "action": "move", "dx": 0, "dy": 0, "device_id": "offline"
            }),
            "local_desktop",
            &ctx,
        )
        .await;

        let got = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(
            got.is_err(),
            "an unresolvable target must not be forwarded anywhere"
        );
    }

    // ── unauthorized access (auth gate lives in WsServer::handle_message) ────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_input_unauthenticated_rejected_by_dispatcher_never_reaches_handler() {
        let ctx = create_test_ctx();
        // `add_test_unpaired_client`, not `add_test_client`: this test asserts the
        // client has NO identity in `ws_to_device_id`, and `add_test_client`
        // models "identity registered, secret not yet derived" — the transient
        // state a real connection passes through. The two helpers deliberately
        // differ so a test never has to guess which one it means.
        let tx = add_test_unpaired_client(&ctx, "unpaired_ws").await;
        let mut rx = tx.subscribe();

        // A real (non-zero) move — if the dispatcher failed to gate this, the
        // handler would move the actual cursor. It must be rejected first.
        let text = serde_json::to_string(&serde_json::json!({
            "type": "remote_input",
            "action": "move",
            "dx": 500,
            "dy": 500
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
            ctx.ws_to_device_id.read().await.is_empty(),
            "client must remain unmapped (unpaired)"
        );
    }
}
