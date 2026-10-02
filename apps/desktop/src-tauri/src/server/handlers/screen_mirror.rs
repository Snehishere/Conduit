//! `screen_mirror` — relays screen frames and routes mirror input.
//!
//! # Direction
//!
//! The capture side is the device that *has the screen*; the viewer is the
//! device that displays it. This handler never decides that by capturing "the
//! local monitor" — it resolves a target device id and either starts a local
//! capture (the requester is a phone asking to see this desktop) or forwards
//! the request and relays the frames the target streams back (the desktop
//! asking to see a phone).
//!
//! Evidence for that split, all in-tree:
//!
//! * `apps/mobile/android/.../ScreenMirrorService.kt` captures the *phone*
//!   screen with `MediaProjection` and emits `screen_mirror`/`frame`.
//! * `apps/mobile/lib/main.dart` starts that capture when a `screen_mirror`
//!   `start` arrives, and injects `touch`/`key`/`scroll` on the phone.
//! * `apps/desktop/src/components/screen-mirror/ScreenMirror.tsx` renders
//!   frames and sends `touch`/`key` back to the selected device.
//!
//! The old code captured `Monitor::from_point(0, 0)` for *every* `start`, so a
//! user who selected their phone was shown the desktop's own screen.
//!
//! # Session state
//!
//! Per-client, not global: viewer sessions are keyed by the viewer's
//! `client_id` and local captures by the requesting `client_id`, so a second
//! client gets its own stream instead of silently attaching to the first.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use enigo::{Button, Coordinate, Direction, Mouse};
use log::{error, info, warn};
use serde_json::Value;
use tokio::sync::Mutex;

use conduit_protocol::types::*;

use super::remote_input::{
    ENIGO, KeyOutcome, MAX_DELTA, clamp_delta_steps, inject_key_chord, relative_to_px,
};
use super::{WsContext, send_to_client};
use crate::error::ConduitError;

// ── Capture / frame-rate policy ─────────────────────────────────────────────

/// Frame rate used when a `start` frame does not ask for one.
const DEFAULT_FPS: u64 = 15;
/// Upper bound on the requested frame rate. Mirrors the Android capture
/// pipeline's `fps.coerceIn(1, 30)`: asking for more only wastes battery and
/// bandwidth, because the capture side would clamp it anyway.
const MAX_FPS: u64 = 30;
/// Back-off after a failed capture, so a dead monitor cannot spin the loop.
const CAPTURE_ERROR_BACKOFF_MS: u64 = 500;
/// `long_press` hold duration.
const LONG_PRESS_MS: u64 = 500;

/// JPEG quality for the desktop's own capture. The phone's capture side has
/// its own scale/quality ladder, so this only applies to a phone asking to see
/// this desktop's screen.
fn jpeg_quality(quality: Option<&str>) -> u8 {
    match quality {
        Some("low") => 30,
        Some("high") => 85,
        _ => 60,
    }
}

/// Frame interval for a requested `fps`, clamped to `1..=MAX_FPS`.
fn frame_interval(fps: Option<u32>) -> Duration {
    let fps = fps
        .map(|requested| f64::from(requested).clamp(1.0, MAX_FPS as f64) as u64)
        .unwrap_or(DEFAULT_FPS);
    Duration::from_millis(1000 / fps.max(1))
}

// ── Session state ───────────────────────────────────────────────────────────

/// A viewer waiting for frames from `target_device_id`.
struct ViewerSession {
    target_device_id: String,
}

/// Local desktop capture, one entry per requesting client.
struct LocalCapture {
    running: bool,
    last_frame: Instant,
    jpeg_quality: u8,
    interval: Duration,
}

static VIEWERS: LazyLock<Mutex<HashMap<String, ViewerSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static LOCAL_CAPTURES: LazyLock<Mutex<HashMap<String, LocalCapture>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Resolve `target_device_id` to the `client_id` of a *different* connected
/// device.
///
/// Returns `None` when the id is absent, unknown, or names the sender itself.
/// The last case is what makes "the phone names the desktop" fall through to
/// local injection, and stops a client forwarding frames to itself.
pub(crate) async fn resolve_target_client(
    ctx: &WsContext,
    sender_client_id: &str,
    target_device_id: Option<&str>,
) -> Option<String> {
    let device_id = target_device_id.filter(|id| !id.is_empty())?;
    let map = ctx.ws_to_device_id.read().await;
    if map
        .get(sender_client_id)
        .is_some_and(|own| own == device_id)
    {
        return None;
    }
    map.iter()
        .find(|(client_id, mapped)| {
            client_id.as_str() != sender_client_id && mapped.as_str() == device_id
        })
        .map(|(client_id, _)| client_id.clone())
}

/// Whether `client_id` is this machine's own UI connection.
///
/// `WsServer` maps loopback peers to the sentinel device id `local_desktop`, so
/// this is how a handler tells "the desktop app asking about a phone" from "a
/// remote device asking about this desktop". The desktop's real device id is
/// not in `ws_to_device_id`, so it cannot be matched by value.
async fn is_local_desktop(ctx: &WsContext, client_id: &str) -> bool {
    ctx.ws_to_device_id
        .read()
        .await
        .get(client_id)
        .is_some_and(|device_id| device_id == "local_desktop")
}

/// The device whose screen a `start` frame is asking for, if it is not this
/// machine.
///
/// Only the desktop app's own connection ever asks for *another* device's
/// screen. A remote requester is asking about the machine it is talking to —
/// that is the phone-as-viewer flow, and it captures locally.
async fn remote_mirror_target(
    ctx: &WsContext,
    client_id: &str,
    target_device_id: Option<&str>,
) -> Option<String> {
    let device_id = target_device_id.filter(|id| !id.is_empty())?;
    is_local_desktop(ctx, client_id)
        .await
        .then(|| device_id.to_string())
}

// ── Parsed commands ─────────────────────────────────────────────────────────

/// A validated `screen_mirror` frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScreenMirrorCommand {
    Start {
        /// Routing envelope: the device whose screen should be mirrored.
        target_device_id: Option<String>,
        quality: Option<String>,
        fps: Option<u32>,
    },
    Stop,
    /// A frame produced by a capturing device; relayed verbatim.
    Frame,
    Touch {
        x: f64,
        y: f64,
        action_type: Option<String>,
    },
    Key {
        key: String,
        modifiers: Vec<String>,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    /// Any other action, including the `configure` the desktop used to send on
    /// every quality/FPS change and that no capture side ever handled.
    Unknown {
        action: String,
    },
}

/// Parse a `screen_mirror` frame into a [`ScreenMirrorCommand`].
///
/// Each arm deserialises the matching `conduit-protocol` type, so the
/// `actionType` enum validator, the required `x`/`y`/`key` fields and the
/// `modifiers` list are enforced at the boundary instead of being hand-rolled
/// `Value` lookups. `frame` is the one exception: its payload is opaque base64
/// from whichever device is capturing, and the Android sender adds fields
/// (`device_id`, `timestamp`) that `ScreenMirrorFrame` does not model, so the
/// frame is relayed exactly as received.
pub(crate) fn parse_command(msg: &Value) -> Result<ScreenMirrorCommand, String> {
    let action = msg.get("action").and_then(Value::as_str).unwrap_or("");

    match action {
        "start" => {
            let parsed: ScreenMirrorStart =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(ScreenMirrorCommand::Start {
                target_device_id: msg
                    .get("device_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                quality: parsed.quality,
                fps: parsed.fps,
            })
        }
        "stop" => Ok(ScreenMirrorCommand::Stop),
        "frame" => Ok(ScreenMirrorCommand::Frame),
        "touch" => {
            let parsed: ScreenMirrorTouch =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(ScreenMirrorCommand::Touch {
                x: parsed.x,
                y: parsed.y,
                action_type: parsed.action_type,
            })
        }
        "key" => {
            let parsed: ScreenMirrorKey =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(ScreenMirrorCommand::Key {
                key: parsed.key,
                modifiers: parsed.modifiers.unwrap_or_default(),
            })
        }
        "scroll" => {
            let parsed: ScreenMirrorScroll =
                serde_json::from_value(msg.clone()).map_err(|e| e.to_string())?;
            Ok(ScreenMirrorCommand::Scroll {
                dx: parsed.dx,
                dy: parsed.dy,
            })
        }
        other => Ok(ScreenMirrorCommand::Unknown {
            action: other.to_string(),
        }),
    }
}

// ── Dispatch ────────────────────────────────────────────────────────────────

pub async fn handle_screen_mirror(msg: Value, client_id: &str, ctx: &WsContext) {
    let command = match parse_command(&msg) {
        Ok(command) => command,
        Err(reason) => {
            warn!("Invalid screen_mirror frame from {}: {}", client_id, reason);
            return;
        }
    };

    match command {
        ScreenMirrorCommand::Start {
            target_device_id,
            quality,
            fps,
        } => {
            handle_start(target_device_id, quality, fps, client_id, ctx).await;
        }
        ScreenMirrorCommand::Stop => {
            handle_stop(client_id, ctx).await;
        }
        ScreenMirrorCommand::Frame => {
            relay_frame(&msg, client_id, ctx).await;
        }
        ScreenMirrorCommand::Touch { x, y, action_type } => {
            if let Some(target) = viewer_target_client(ctx, client_id).await {
                forward_touch(x, y, action_type, &target, ctx).await;
            } else {
                inject_touch(x, y, action_type.as_deref()).await;
            }
        }
        ScreenMirrorCommand::Key { key, modifiers } => {
            if let Some(target) = viewer_target_client(ctx, client_id).await {
                forward_key(&key, &modifiers, &target, ctx).await;
            } else if let KeyOutcome::Rejected(reason) = inject_key_chord(&key, &modifiers).await {
                warn!("Rejected screen_mirror key: {}", reason);
            }
        }
        ScreenMirrorCommand::Scroll { dx, dy } => {
            if let Some(target) = viewer_target_client(ctx, client_id).await {
                forward_scroll(dx, dy, &target, ctx).await;
            } else {
                inject_scroll(dx, dy).await;
            }
        }
        ScreenMirrorCommand::Unknown { action } => {
            warn!("Unknown screen_mirror action: {}", action);
        }
    }
}

/// The capture device this client is currently mirroring, if any.
async fn viewer_target(client_id: &str) -> Option<String> {
    VIEWERS
        .lock()
        .await
        .get(client_id)
        .map(|session| session.target_device_id.clone())
}

/// The `client_id` of the device this client is mirroring, if it is still
/// connected.
///
/// Resolved on every use rather than cached: a session stores a *device* id,
/// and a reconnect hands the device a new `client_id`. Sending to a device id
/// where a client id is expected silently delivers nothing.
async fn viewer_target_client(ctx: &WsContext, client_id: &str) -> Option<String> {
    let device_id = viewer_target(client_id).await?;
    resolve_target_client(ctx, client_id, Some(&device_id)).await
}

// ── start / stop ────────────────────────────────────────────────────────────

async fn handle_start(
    target_device_id: Option<String>,
    quality: Option<String>,
    fps: Option<u32>,
    client_id: &str,
    ctx: &WsContext,
) {
    // Only the desktop app's own connection ever asks for *another* device's
    // screen. A remote requester is asking about this machine — that is the
    // phone-as-viewer flow, and it is the one that captures locally.
    let remote_target = remote_mirror_target(ctx, client_id, target_device_id.as_deref()).await;

    if let Some(device_id) = remote_target {
        // A viewer of another device: register the session and ask that device
        // to start capturing. It streams `frame` back and `relay_frame` routes
        // them to this viewer.
        let Some(target) = resolve_target_client(ctx, client_id, Some(&device_id)).await else {
            // Falling back to a local capture here is exactly how a user ended
            // up looking at the desktop's own screen while "mirroring" a phone
            // that was not connected.
            error!(
                "Screen mirror refused: the desktop asked to mirror {}, which is not connected",
                device_id
            );
            notify_capture_stopped(client_id, ctx).await;
            return;
        };

        VIEWERS.lock().await.insert(
            client_id.to_string(),
            ViewerSession {
                target_device_id: device_id.to_string(),
            },
        );
        let request = ScreenMirrorStart {
            msg_type: "screen_mirror".into(),
            action: "start".into(),
            quality,
            fps,
        };
        let payload = serde_json::to_string(&request).expect("ScreenMirrorStart serializes");
        if !send_to_client(ctx, &target, &payload).await {
            warn!(
                "Screen mirror target {} unreachable — start dropped",
                target
            );
            VIEWERS.lock().await.remove(client_id);
            return;
        }
        info!(
            "Screen mirror: the desktop is now viewing {} (start forwarded)",
            device_id
        );
        return;
    }

    // Capture locally and stream the frames to the requester.
    let interval = frame_interval(fps);
    let jpeg_quality = jpeg_quality(quality.as_deref());
    {
        let mut captures = LOCAL_CAPTURES.lock().await;
        if captures
            .get(client_id)
            .is_some_and(|capture| capture.running)
        {
            info!(
                "Screen capture already running for {}, ignoring start",
                client_id
            );
            return;
        }
        captures.insert(
            client_id.to_string(),
            LocalCapture {
                running: true,
                last_frame: Instant::now(),
                jpeg_quality,
                interval,
            },
        );
    }

    info!(
        "Desktop screen capture started for {} (quality={}, {}ms/frame)",
        client_id,
        quality.as_deref().unwrap_or("medium"),
        interval.as_millis()
    );

    let ctx = ctx.clone();
    let client_id = client_id.to_string();
    std::thread::spawn(move || {
        capture_loop(&ctx, &client_id);
    });
}

async fn handle_stop(client_id: &str, ctx: &WsContext) {
    // A viewer stops the *remote* capture; otherwise it stops ours.
    if let Some(device_id) = viewer_target(client_id).await {
        VIEWERS.lock().await.remove(client_id);
        let request = ScreenMirrorStop {
            msg_type: "screen_mirror".into(),
            action: "stop".into(),
        };
        let payload = serde_json::to_string(&request).expect("ScreenMirrorStop serializes");
        if let Some(target) = resolve_target_client(ctx, client_id, Some(&device_id)).await {
            let _ = send_to_client(ctx, &target, &payload).await;
        }
        info!(
            "Screen mirror: {} stopped mirroring {}",
            client_id, device_id
        );
    } else {
        let was_running = {
            let mut captures = LOCAL_CAPTURES.lock().await;
            match captures.get_mut(client_id) {
                Some(capture) if capture.running => {
                    capture.running = false;
                    true
                }
                _ => false,
            }
        };
        info!(
            "Desktop screen capture stopped for {} (was running: {})",
            client_id, was_running
        );
    }

    notify_capture_stopped(client_id, ctx).await;
}

async fn notify_capture_stopped(client_id: &str, ctx: &WsContext) {
    let response = ScreenMirrorCaptureStopped {
        msg_type: "screen_mirror".into(),
        action: "capture_stopped".into(),
    };
    let payload = serde_json::to_string(&response).expect("ScreenMirrorCaptureStopped serializes");
    let _ = send_to_client(ctx, client_id, &payload).await;
}

// ── frame relay + local capture ─────────────────────────────────────────────

/// Route a `frame` from a capturing device to the viewers watching it.
async fn relay_frame(frame: &Value, client_id: &str, ctx: &WsContext) {
    let sender_device_id = sender_identity(ctx, client_id).await;

    let targets: Vec<String> = {
        let viewers = VIEWERS.lock().await;
        viewers
            .iter()
            .filter(|(_, session)| Some(&session.target_device_id) == sender_device_id.as_ref())
            .map(|(viewer, _)| viewer.clone())
            .collect()
    };

    if targets.is_empty() {
        warn!(
            "Dropping screen frame from {}: no viewer is watching {:?}",
            client_id, sender_device_id
        );
        return;
    }

    let payload = frame.to_string();
    for viewer in targets {
        if !send_to_client(ctx, &viewer, &payload).await {
            warn!("Failed to relay screen frame to viewer {}", viewer);
            VIEWERS.lock().await.remove(&viewer);
        }
    }
}

/// The stable device id a frame from `client_id` should be attributed to.
///
/// Two shapes of sender reach this handler and they are not interchangeable:
///
///  * a **LAN** socket, whose `client_id` is a per-connection UUID and whose
///    device id lives in the pairing registry; and
///  * a **relayed** frame, which `unwrap_relay_delivery` dispatches under the
///    sender's own device id. A phone on another network has no socket here, so
///    there is no registry entry to map and the lookup returns `None` — which is
///    what made every relayed frame look like it came from nobody and drop.
///
/// So when there is no registry entry, `client_id` is only accepted as the
/// device id if the device registry says that device is a paired peer with a
/// derived secret — the same bar `super::is_trusted_peer` applies, and the same
/// bar the dispatcher already cleared before this handler ran. An id-shaped
/// string from an unpaired socket is not identity, and gets no attribution.
async fn sender_identity(ctx: &WsContext, client_id: &str) -> Option<String> {
    if let Some(device_id) = super::paired_device_id(ctx, client_id).await {
        return Some(device_id);
    }
    let is_paired_device = ctx
        .sync_engine
        .read()
        .await
        .get_client(client_id)
        .is_some_and(|client| !client.shared_secret.is_empty());
    is_paired_device.then(|| client_id.to_string())
}

/// Periodically capture the local monitor and send JPEG frames to `client_id`.
///
/// Runs on a dedicated OS thread because `xcap` blocks, so the async mutex and
/// the client map are taken with their `blocking_*` variants.
fn capture_loop(ctx: &WsContext, client_id: &str) {
    loop {
        let (jpeg_quality, interval) = {
            let captures = LOCAL_CAPTURES.blocking_lock();
            match captures.get(client_id) {
                Some(capture) if capture.running => (capture.jpeg_quality, capture.interval),
                _ => break,
            }
        };

        let last_frame = LOCAL_CAPTURES
            .blocking_lock()
            .get(client_id)
            .map(|capture| capture.last_frame)
            .unwrap_or_else(Instant::now);
        let elapsed = Instant::now().duration_since(last_frame);
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }

        match capture_primary_screen(jpeg_quality) {
            Ok((jpeg_bytes, width, height)) => {
                let frame = ScreenMirrorFrame {
                    msg_type: "screen_mirror".into(),
                    action: "frame".into(),
                    data: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &jpeg_bytes,
                    ),
                    format: "jpeg".into(),
                    width,
                    height,
                    from_desktop: Some(true),
                };
                let payload = serde_json::to_string(&frame).expect("ScreenMirrorFrame serializes");

                let delivered = {
                    let clients = ctx.clients.blocking_read();
                    match clients.get(client_id) {
                        Some(tx) => tx.send(payload).is_ok(),
                        None => false,
                    }
                };
                if !delivered {
                    warn!("Client {} gone, stopping capture", client_id);
                    if let Some(capture) = LOCAL_CAPTURES.blocking_lock().get_mut(client_id) {
                        capture.running = false;
                    }
                    break;
                }

                if let Some(capture) = LOCAL_CAPTURES.blocking_lock().get_mut(client_id) {
                    capture.last_frame = Instant::now();
                }
            }
            Err(e) => {
                error!("Screen capture failed: {}", e);
                std::thread::sleep(Duration::from_millis(CAPTURE_ERROR_BACKOFF_MS));
            }
        }
    }

    info!("Desktop capture loop for {} exited", client_id);

    // Drop the entry so a client that reconnects many times does not leak one
    // map row per session. A `start` that arrived while this loop was winding
    // down has already re-inserted a running entry — leave that one alone.
    let mut captures = LOCAL_CAPTURES.blocking_lock();
    if captures
        .get(client_id)
        .is_some_and(|capture| !capture.running)
    {
        captures.remove(client_id);
    }
}

/// Capture the primary monitor using xcap and encode to JPEG.
fn capture_primary_screen(_jpeg_quality: u8) -> Result<(Vec<u8>, u32, u32), ConduitError> {
    use xcap::Monitor;

    let monitor = Monitor::from_point(0, 0)
        .or_else(|_| {
            // Fallback: try to get the primary monitor
            let monitors = Monitor::all().map_err(|e| e.to_string())?;
            monitors
                .into_iter()
                .next()
                .ok_or_else(|| "No monitors found".to_string())
        })
        .map_err(|e| ConduitError::Other(format!("Failed to get monitor: {}", e)))?;

    let width = monitor.width();
    let height = monitor.height();

    let image = monitor
        .capture_image()
        .map_err(|e| ConduitError::Other(format!("Failed to capture screen: {}", e)))?;

    let mut jpeg_buf = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut jpeg_buf, image::ImageFormat::Jpeg)
        .map_err(|e| ConduitError::Other(format!("JPEG encode failed: {}", e)))?;

    Ok((jpeg_buf.into_inner(), width, height))
}

// ── input: forward to the mirrored device, or inject locally ────────────────

async fn forward_touch(x: f64, y: f64, action_type: Option<String>, target: &str, ctx: &WsContext) {
    let touch = ScreenMirrorTouch {
        msg_type: "screen_mirror".into(),
        action: "touch".into(),
        x,
        y,
        action_type,
    };
    let payload = serde_json::to_string(&touch).expect("ScreenMirrorTouch serializes");
    let _ = send_to_client(ctx, target, &payload).await;
}

async fn forward_key(key: &str, modifiers: &[String], target: &str, ctx: &WsContext) {
    let message = ScreenMirrorKey {
        msg_type: "screen_mirror".into(),
        action: "key".into(),
        key: key.to_string(),
        modifiers: Some(modifiers.to_vec()),
    };
    let payload = serde_json::to_string(&message).expect("ScreenMirrorKey serializes");
    let _ = send_to_client(ctx, target, &payload).await;
}

async fn forward_scroll(dx: f64, dy: f64, target: &str, ctx: &WsContext) {
    let message = ScreenMirrorScroll {
        msg_type: "screen_mirror".into(),
        action: "scroll".into(),
        dx,
        dy,
    };
    let payload = serde_json::to_string(&message).expect("ScreenMirrorScroll serializes");
    let _ = send_to_client(ctx, target, &payload).await;
}

/// Inject a touch into *this* machine — the requester is mirroring our screen.
async fn inject_touch(x: f64, y: f64, action_type: Option<&str>) {
    let (width, height) = monitor_dimensions();
    let abs_x = relative_to_px(x, width);
    let abs_y = relative_to_px(y, height);

    let mut enigo = ENIGO.lock().await;
    // Move first, unconditionally: every action type below acts at this point.
    let _ = enigo.move_mouse(abs_x, abs_y, Coordinate::Abs);

    match action_type.unwrap_or("tap") {
        "tap" | "click" => {
            let _ = enigo.button(Button::Left, Direction::Click);
        }
        "double_tap" | "double_click" => {
            let _ = enigo.button(Button::Left, Direction::Click);
            let _ = enigo.button(Button::Left, Direction::Click);
        }
        "long_press" => {
            let _ = enigo.button(Button::Left, Direction::Press);
            drop(enigo);
            tokio::time::sleep(Duration::from_millis(LONG_PRESS_MS)).await;
            let mut enigo = ENIGO.lock().await;
            let _ = enigo.button(Button::Left, Direction::Release);
        }
        "right_click" => {
            let _ = enigo.button(Button::Right, Direction::Click);
        }
        // "move" (hover / drag update) and anything unrecognised: the pointer
        // is already where it should be. An unrecognised action must not
        // become a click, so the default is "do nothing", not "tap".
        _ => {}
    }
}

/// Inject a scroll into *this* machine.
async fn inject_scroll(dx: f64, dy: f64) {
    let dx = clamp_delta_steps(dx, MAX_DELTA);
    let dy = clamp_delta_steps(dy, MAX_DELTA);
    if dx == 0 && dy == 0 {
        return;
    }
    let mut enigo = ENIGO.lock().await;
    let _ = enigo.scroll(dy, enigo::Axis::Vertical);
    let _ = enigo.scroll(dx, enigo::Axis::Horizontal);
}

/// Primary monitor size, used to turn relative touch coordinates into pixels.
fn monitor_dimensions() -> (u32, u32) {
    use xcap::Monitor;

    Monitor::from_point(0, 0)
        .or_else(|_| {
            let monitors = Monitor::all().unwrap_or_default();
            monitors
                .into_iter()
                .next()
                .ok_or_else(|| "No monitors".to_string())
        })
        .map(|monitor| (monitor.width(), monitor.height()))
        .unwrap_or((1920, 1080))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{
        add_test_client, add_test_client_mapped, add_test_unpaired_client, create_test_ctx,
    };

    // SAFETY: no test here starts a local capture — every `start` names a
    // connected target, which only forwards — and none injects input: the
    // `touch`/`key`/`scroll` paths are either forwarded to a target device or
    // short-circuited by a zero-delta guard.
    //
    // Client ids are unique per test because `VIEWERS` / `LOCAL_CAPTURES` are
    // process-global: shared ids would let one test's frame land in another
    // test's receiver.

    // ── frame-rate / quality policy ─────────────────────────────────────────

    #[test]
    fn frame_interval_defaults_to_fifteen_and_clamps_the_request() {
        assert_eq!(frame_interval(None), Duration::from_millis(66));
        assert_eq!(frame_interval(Some(15)), Duration::from_millis(66));
        assert_eq!(frame_interval(Some(30)), Duration::from_millis(33));
        // Asking for more than the capture pipeline supports is clamped rather
        // than ignored: the desktop used to hardcode 15 fps and never read it.
        assert_eq!(frame_interval(Some(60)), Duration::from_millis(33));
        assert_eq!(frame_interval(Some(0)), Duration::from_millis(1000));
    }

    #[test]
    fn jpeg_quality_maps_the_protocol_presets() {
        assert_eq!(jpeg_quality(Some("low")), 30);
        assert_eq!(jpeg_quality(Some("medium")), 60);
        assert_eq!(jpeg_quality(Some("high")), 85);
        assert_eq!(jpeg_quality(None), 60);
        assert_eq!(jpeg_quality(Some("nonsense")), 60);
    }

    // ── action dispatch ─────────────────────────────────────────────────────

    #[test]
    fn parse_command_dispatches_start_with_routing_and_settings() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "start",
            "device_id": "start-phone", "quality": "high", "fps": 30
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Start {
                target_device_id: Some("start-phone".into()),
                quality: Some("high".into()),
                fps: Some(30),
            }
        );
    }

    #[test]
    fn parse_command_start_without_settings_is_still_valid() {
        let msg = serde_json::json!({ "type": "screen_mirror", "action": "start" });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Start {
                target_device_id: None,
                quality: None,
                fps: None,
            }
        );
    }

    #[test]
    fn parse_command_dispatches_stop() {
        let msg = serde_json::json!({ "type": "screen_mirror", "action": "stop" });
        assert_eq!(parse_command(&msg).unwrap(), ScreenMirrorCommand::Stop);
    }

    /// `configure` was sent on every quality/FPS change and hit the catch-all
    /// arm. No capture side ever handled it, so it is reported as unknown.
    #[test]
    fn parse_command_configure_is_unknown_not_a_silent_noop() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "configure", "quality": "low", "fps": 60
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Unknown {
                action: "configure".into()
            }
        );
    }

    /// The desktop UI sent `action_type` (snake_case) while the protocol and
    /// the injector both spell it `actionType` (camelCase) — so every gesture
    /// silently arrived as a tap.
    #[test]
    fn parse_command_touch_reads_camel_case_action_type() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "touch",
            "x": 0.5, "y": 0.25, "actionType": "double_tap"
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Touch {
                x: 0.5,
                y: 0.25,
                action_type: Some("double_tap".into()),
            }
        );
    }

    #[test]
    fn parse_command_touch_snake_case_action_type_is_dropped_not_guessed() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "touch",
            "x": 0.5, "y": 0.25, "action_type": "double_tap"
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Touch {
                x: 0.5,
                y: 0.25,
                action_type: None,
            },
            "an unrecognised field must not be read as a gesture"
        );
    }

    #[test]
    fn parse_command_touch_rejects_unknown_action_type() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "touch",
            "x": 0.0, "y": 0.0, "actionType": "explode"
        });
        assert!(
            parse_command(&msg).is_err(),
            "the actionType enum validator must reject unknown gestures"
        );
    }

    #[test]
    fn parse_command_touch_requires_numeric_coordinates() {
        assert!(parse_command(&serde_json::json!({ "action": "touch" })).is_err());
        assert!(parse_command(&serde_json::json!({ "action": "touch", "x": 0.5 })).is_err());
        assert!(
            parse_command(&serde_json::json!({
                "action": "touch", "x": "left", "y": "up"
            }))
            .is_err()
        );
    }

    #[test]
    fn parse_command_dispatches_key_with_modifiers() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "key", "key": "c",
            "modifiers": ["control", "shift"]
        });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Key {
                key: "c".into(),
                modifiers: vec!["control".into(), "shift".into()],
            }
        );
    }

    #[test]
    fn parse_command_key_requires_a_key_field() {
        assert!(parse_command(&serde_json::json!({ "action": "key" })).is_err());
    }

    #[test]
    fn parse_command_dispatches_scroll() {
        let msg =
            serde_json::json!({ "type": "screen_mirror", "action": "scroll", "dx": 0, "dy": -3 });
        assert_eq!(
            parse_command(&msg).unwrap(),
            ScreenMirrorCommand::Scroll { dx: 0.0, dy: -3.0 }
        );
    }

    /// The phone's frame payload carries fields `ScreenMirrorFrame` does not
    /// model, so it is relayed rather than parsed.
    #[test]
    fn parse_command_selects_the_frame_branch() {
        let msg = serde_json::json!({
            "type": "screen_mirror", "action": "frame", "data": "xx",
            "device_id": "start-phone", "width": 1080, "height": 2340
        });
        assert_eq!(parse_command(&msg).unwrap(), ScreenMirrorCommand::Frame);
    }

    #[test]
    fn parse_command_unknown_and_missing_action_are_unknown() {
        assert_eq!(
            parse_command(&serde_json::json!({ "action": "hologram" })).unwrap(),
            ScreenMirrorCommand::Unknown {
                action: "hologram".into()
            }
        );
        assert_eq!(
            parse_command(&serde_json::json!({ "type": "screen_mirror" })).unwrap(),
            ScreenMirrorCommand::Unknown {
                action: String::new()
            }
        );
    }

    // ── coordinate clamping ─────────────────────────────────────────────────

    #[test]
    fn touch_coordinates_are_clamped_to_the_display() {
        let (width, height) = (1920u32, 1080u32);
        assert_eq!(relative_to_px(0.0, width), 0);
        assert_eq!(relative_to_px(0.5, width), 960);
        assert_eq!(relative_to_px(1.0, width), 1920);
        // Out-of-range and absurd values land on the display edge rather than
        // saturating the `as i32` cast and throwing the cursor.
        assert_eq!(relative_to_px(-5.0, width), 0);
        assert_eq!(relative_to_px(1e18, width), 1920);
        assert_eq!(relative_to_px(1e18, height), 1080);
        assert_eq!(relative_to_px(f64::NAN, width), 0);
    }

    #[test]
    fn monitor_dimensions_are_never_zero() {
        let (width, height) = monitor_dimensions();
        assert!(width > 0 && height > 0, "display size must be usable");
    }

    // ── routing ─────────────────────────────────────────────────────────────

    /// The direction rule, pinned without starting a capture thread: only the
    /// desktop's own connection asks for another device's screen, so a phone
    /// asking to mirror "the desktop" must still resolve to a *local* capture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_mirror_target_only_applies_to_the_desktops_own_connection() {
        let ctx = create_test_ctx();
        let _ = add_test_client_mapped(&ctx, "sm_dir_desktop", "local_desktop").await;
        let _ = add_test_client_mapped(&ctx, "sm_dir_phone", "sm-dir-phone").await;

        assert_eq!(
            remote_mirror_target(&ctx, "sm_dir_desktop", Some("sm-dir-phone")).await,
            Some("sm-dir-phone".to_string()),
            "the desktop asking about a phone is a remote mirror request"
        );
        assert_eq!(
            remote_mirror_target(&ctx, "sm_dir_phone", Some("desktop-uuid")).await,
            None,
            "a phone asking about the desktop must mirror locally"
        );
        assert_eq!(
            remote_mirror_target(&ctx, "sm_dir_desktop", None).await,
            None,
            "a start with no target has no remote side"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_against_a_connected_device_registers_a_viewer_and_forwards() {
        let ctx = create_test_ctx();
        // The desktop's own UI connection is the loopback sentinel.
        let viewer_tx = add_test_client_mapped(&ctx, "sm_start_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        let phone_tx = add_test_client_mapped(&ctx, "sm_start_phone", "sm-start-phone").await;
        let mut phone_rx = phone_tx.subscribe();

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "start",
                "device_id": "sm-start-phone", "quality": "medium", "fps": 30
            }),
            "sm_start_viewer",
            &ctx,
        )
        .await;

        // The phone (capture side) receives the start; the viewer does not.
        let forwarded =
            tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv())
                .await
                .expect("start must be forwarded to the capture side")
                .expect("broadcast recv");
        let value: serde_json::Value = serde_json::from_str(&forwarded).unwrap();
        assert_eq!(value["action"], "start");
        assert_eq!(value["fps"], serde_json::json!(30));

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(150), viewer_rx.recv()).await;
        assert!(leaked.is_err(), "the viewer must not receive its own start");

        // The session is what routes frames back.
        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "frame", "data": "c3RhcnQ="
            }),
            "sm_start_phone",
            &ctx,
        )
        .await;
        let frame = tokio::time::timeout(std::time::Duration::from_millis(500), viewer_rx.recv())
            .await
            .expect("frame must be relayed to the viewer")
            .expect("broadcast recv");
        assert!(
            frame.contains("c3RhcnQ="),
            "relayed frame lost its payload: {frame}"
        );
    }

    /// A phone on another network captures through the relay. It has no LAN
    /// socket, so `ws_to_device_id` has no entry for it and the relay
    /// dispatches its frames under the phone's **device id** rather than a
    /// connection id (`unwrap_relay_delivery`). The frame must still reach the
    /// viewer that asked for it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relayed_frame_reaches_the_viewer_that_asked_for_the_phone() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_relay_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();

        // The capture side is reachable only through the relay, so the desktop
        // asks for it by device id and gets its `start` delivered over the
        // relay socket rather than a LAN one.
        let _relay_tx = add_test_client(&ctx, "relay_server").await;
        VIEWERS.lock().await.insert(
            "sm_relay_viewer".to_string(),
            ViewerSession {
                target_device_id: "sm-relay-phone".to_string(),
            },
        );
        ctx.sync_engine
            .write()
            .await
            .add_client(crate::sync::ConnectedClient {
                device_id: "sm-relay-phone".into(),
                device_name: "Relayed phone".into(),
                device_type: "phone".into(),
                shared_secret: "22".repeat(32),
                last_heartbeat: 0,
                battery_level: None,
            });

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "frame", "data": "cmVsYXk="
            }),
            "sm-relay-phone",
            &ctx,
        )
        .await;

        let frame = tokio::time::timeout(std::time::Duration::from_millis(500), viewer_rx.recv())
            .await
            .expect("a relayed frame must reach the viewer watching that device")
            .expect("broadcast recv");
        assert!(
            frame.contains("cmVsYXk="),
            "relayed frame lost its payload: {frame}"
        );

        VIEWERS.lock().await.remove("sm_relay_viewer");
    }

    /// The LAN case still works: a paired socket is mapped through
    /// `ws_to_device_id`, not through its own connection id.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_lan_frame_is_attributed_through_the_pairing_registry() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_lan_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        let _phone_tx = add_test_client_mapped(&ctx, "sm_lan_phone", "sm-lan-phone").await;
        VIEWERS.lock().await.insert(
            "sm_lan_viewer".to_string(),
            ViewerSession {
                target_device_id: "sm-lan-phone".to_string(),
            },
        );

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "frame", "data": "bGFu"
            }),
            "sm_lan_phone",
            &ctx,
        )
        .await;

        let frame = tokio::time::timeout(std::time::Duration::from_millis(500), viewer_rx.recv())
            .await
            .expect("a LAN frame must reach the viewer")
            .expect("broadcast recv");
        assert!(frame.contains("bGFu"), "got: {frame}");

        VIEWERS.lock().await.remove("sm_lan_viewer");
    }

    /// KNOWN GAP — not a fix. A phone that is reachable only through the relay
    /// cannot be asked to start capturing, because forwarding a `start` means
    /// resolving the target to a *socket* on this machine
    /// (`resolve_target_client` reads `ws_to_device_id`) and a relayed device
    /// has none. So the desktop refuses with `capture_stopped` and the relay
    /// socket is never used, which means `relay_frame`'s relay path — the one
    /// `a_relayed_frame_reaches_the_viewer_that_asked_for_the_phone` covers —
    /// is unreachable from the UI today.
    ///
    /// This test pins the gap so whoever closes it has a red test to turn
    /// green, and so nobody "fixes" it by falling back to a local capture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn known_gap_a_relay_only_target_cannot_be_asked_to_start() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_gap_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        let relay_tx = add_test_client(&ctx, "relay_server").await;
        let mut relay_rx = relay_tx.subscribe();
        // Paired and known to the hub, but on another network: no LAN socket.
        ctx.sync_engine
            .write()
            .await
            .add_client(crate::sync::ConnectedClient {
                device_id: "sm-gap-phone".into(),
                device_name: "Relay-only phone".into(),
                device_type: "phone".into(),
                shared_secret: "33".repeat(32),
                last_heartbeat: 0,
                battery_level: None,
            });

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "start", "device_id": "sm-gap-phone"
            }),
            "sm_gap_viewer",
            &ctx,
        )
        .await;

        let relayed =
            tokio::time::timeout(std::time::Duration::from_millis(150), relay_rx.recv()).await;
        assert!(
            relayed.is_err(),
            "today the start never reaches the relay connection either"
        );
        assert!(
            viewer_target("sm_gap_viewer").await.is_none(),
            "and no session may be registered, or frames would route to a \
             device that was never asked to capture"
        );
        assert!(
            !LOCAL_CAPTURES.lock().await.contains_key("sm_gap_viewer"),
            "and it must not fall back to capturing the desktop's own screen"
        );
        let ack = tokio::time::timeout(std::time::Duration::from_millis(500), viewer_rx.recv())
            .await
            .expect("the viewer must be told the capture did not start")
            .expect("broadcast recv");
        assert!(ack.contains("capture_stopped"), "got: {ack}");

        VIEWERS.lock().await.remove("sm_gap_viewer");
    }

    /// Attribution is not widened: falling back to the connection id must still
    /// require that id to name a *paired device*, so it cannot become a way to
    /// deliver a frame from a socket the auth gate would have refused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_frame_from_an_unpaired_connection_reaches_nobody() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_anon_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        // A connection UUID with no registry entry and no `ConnectedClient` —
        // the state the hub's auth gate refuses before the handler runs.
        let _phone_tx = add_test_unpaired_client(&ctx, "sm-anon-uuid").await;
        VIEWERS.lock().await.insert(
            "sm_anon_viewer".to_string(),
            ViewerSession {
                target_device_id: "sm-anon-phone".to_string(),
            },
        );

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "frame", "data": "YW5vbg=="
            }),
            "sm-anon-uuid",
            &ctx,
        )
        .await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(150), viewer_rx.recv()).await;
        assert!(
            leaked.is_err(),
            "an unpaired sender's frame must reach nobody"
        );

        VIEWERS.lock().await.remove("sm_anon_viewer");
    }

    /// The sharp case: the sender's id *matches* the viewer's target device id
    /// as a string, but nothing is paired under it. A string match is not
    /// identity — only the device registry is.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_id_shaped_like_a_device_id_is_not_enough_to_be_one() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_imp_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        add_test_unpaired_client(&ctx, "sm-imp-device").await;
        VIEWERS.lock().await.insert(
            "sm_imp_viewer".to_string(),
            ViewerSession {
                target_device_id: "sm-imp-device".to_string(),
            },
        );

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "frame", "data": "aW1wb3N0b3I="
            }),
            "sm-imp-device",
            &ctx,
        )
        .await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(150), viewer_rx.recv()).await;
        assert!(
            leaked.is_err(),
            "an unpaired id must not be able to claim a viewer's target device"
        );

        VIEWERS.lock().await.remove("sm_imp_viewer");
    }

    /// A paired device with an empty secret is not a peer either, so a frame
    /// attributed to it must not be routed on the strength of the id alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_device_row_with_no_secret_does_not_earn_relay_attribution() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_nosecret_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        ctx.sync_engine
            .write()
            .await
            .add_client(crate::sync::ConnectedClient {
                device_id: "sm-nosecret-phone".into(),
                device_name: "Half-paired".into(),
                device_type: "phone".into(),
                shared_secret: String::new(),
                last_heartbeat: 0,
                battery_level: None,
            });
        VIEWERS.lock().await.insert(
            "sm_nosecret_viewer".to_string(),
            ViewerSession {
                target_device_id: "sm-nosecret-phone".to_string(),
            },
        );

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "frame", "data": "bm9zZWNyZXQ="
            }),
            "sm-nosecret-phone",
            &ctx,
        )
        .await;

        let leaked =
            tokio::time::timeout(std::time::Duration::from_millis(150), viewer_rx.recv()).await;
        assert!(
            leaked.is_err(),
            "an unpaired device must not reach a viewer"
        );

        VIEWERS.lock().await.remove("sm_nosecret_viewer");
    }

    /// Two viewers of two devices: each has its own session and only its own
    /// frames. The old code had one global capture flag, so a second client
    /// silently got nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_viewers_have_independent_sessions() {
        let ctx = create_test_ctx();
        let a_tx = add_test_client_mapped(&ctx, "sm_pair_viewer_a", "local_desktop").await;
        let mut a_rx = a_tx.subscribe();
        let b_tx = add_test_client_mapped(&ctx, "sm_pair_viewer_b", "local_desktop").await;
        let mut b_rx = b_tx.subscribe();
        let _ = add_test_client_mapped(&ctx, "sm_pair_phone_a", "sm-pair-phone-a").await;
        let _ = add_test_client_mapped(&ctx, "sm_pair_phone_b", "sm-pair-phone-b").await;

        for (viewer, device) in [
            ("sm_pair_viewer_a", "sm-pair-phone-a"),
            ("sm_pair_viewer_b", "sm-pair-phone-b"),
        ] {
            handle_screen_mirror(
                serde_json::json!({
                    "type": "screen_mirror", "action": "start", "device_id": device
                }),
                viewer,
                &ctx,
            )
            .await;
        }

        handle_screen_mirror(
            serde_json::json!({ "type": "screen_mirror", "action": "frame", "data": "Ql8=" }),
            "sm_pair_phone_b",
            &ctx,
        )
        .await;

        let b_frame = tokio::time::timeout(std::time::Duration::from_millis(500), b_rx.recv())
            .await
            .expect("viewer_b must receive its device's frame")
            .expect("broadcast recv");
        assert!(b_frame.contains("Ql8="), "got: {b_frame}");
        let a_frame =
            tokio::time::timeout(std::time::Duration::from_millis(150), a_rx.recv()).await;
        assert!(a_frame.is_err(), "viewer_b's frame must not reach viewer_a");
    }

    /// The wrong-screen defect: the desktop asking to mirror a device that is
    /// not connected must NOT fall back to capturing the local monitor.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_against_a_disconnected_device_never_captures_locally() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_offline_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "start", "device_id": "sm-not-connected"
            }),
            "sm_offline_viewer",
            &ctx,
        )
        .await;

        assert!(
            !LOCAL_CAPTURES
                .lock()
                .await
                .contains_key("sm_offline_viewer"),
            "a mirror request for another device must not start a local capture"
        );
        assert!(
            viewer_target("sm_offline_viewer").await.is_none(),
            "no session may be registered for a refused start"
        );

        let response =
            tokio::time::timeout(std::time::Duration::from_millis(500), viewer_rx.recv())
                .await
                .expect("the viewer must be told capture stopped")
                .expect("broadcast recv");
        assert!(
            response.contains("capture_stopped"),
            "expected capture_stopped, got: {response}"
        );
    }

    /// A viewer never injects into its own machine: its input belongs to the
    /// device it is mirroring.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn viewer_input_is_forwarded_to_the_mirrored_device() {
        let ctx = create_test_ctx();
        let _viewer = add_test_client_mapped(&ctx, "sm_input_viewer", "local_desktop").await;
        let phone_tx = add_test_client_mapped(&ctx, "sm_input_phone", "sm-input-phone").await;
        let mut phone_rx = phone_tx.subscribe();

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "start", "device_id": "sm-input-phone"
            }),
            "sm_input_viewer",
            &ctx,
        )
        .await;
        // Drain the forwarded start.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv()).await;

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "touch",
                "x": 0.5, "y": 0.25, "actionType": "right_click"
            }),
            "sm_input_viewer",
            &ctx,
        )
        .await;
        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "key", "key": "c",
                "modifiers": ["control"]
            }),
            "sm_input_viewer",
            &ctx,
        )
        .await;

        let touch = tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv())
            .await
            .expect("touch must be forwarded")
            .expect("broadcast recv");
        let touch: serde_json::Value = serde_json::from_str(&touch).unwrap();
        assert_eq!(touch["action"], "touch");
        assert_eq!(touch["actionType"], "right_click");
        assert_eq!(touch["x"], serde_json::json!(0.5));

        let key = tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv())
            .await
            .expect("key must be forwarded")
            .expect("broadcast recv");
        let key: serde_json::Value = serde_json::from_str(&key).unwrap();
        assert_eq!(key["action"], "key");
        assert_eq!(key["modifiers"], serde_json::json!(["control"]));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_forwards_to_the_mirrored_device_and_acks_the_viewer() {
        let ctx = create_test_ctx();
        let viewer_tx = add_test_client_mapped(&ctx, "sm_stop_viewer", "local_desktop").await;
        let mut viewer_rx = viewer_tx.subscribe();
        let phone_tx = add_test_client_mapped(&ctx, "sm_stop_phone", "sm-stop-phone").await;
        let mut phone_rx = phone_tx.subscribe();

        handle_screen_mirror(
            serde_json::json!({
                "type": "screen_mirror", "action": "start", "device_id": "sm-stop-phone"
            }),
            "sm_stop_viewer",
            &ctx,
        )
        .await;
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv()).await;

        handle_screen_mirror(
            serde_json::json!({ "type": "screen_mirror", "action": "stop" }),
            "sm_stop_viewer",
            &ctx,
        )
        .await;

        let forwarded =
            tokio::time::timeout(std::time::Duration::from_millis(500), phone_rx.recv())
                .await
                .expect("stop must be forwarded to the capture side")
                .expect("broadcast recv");
        assert!(forwarded.contains("\"stop\""), "got: {forwarded}");

        let ack = tokio::time::timeout(std::time::Duration::from_millis(500), viewer_rx.recv())
            .await
            .expect("stop must ack the requester")
            .expect("broadcast recv");
        assert!(ack.contains("capture_stopped"), "got: {ack}");
        assert!(
            viewer_target("sm_stop_viewer").await.is_none(),
            "the viewer session must be cleared"
        );
    }

    /// A frame from a device nobody is watching is dropped, not broadcast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn frame_without_a_viewer_is_dropped() {
        let ctx = create_test_ctx();
        let bystander_tx = add_test_client(&ctx, "sm_stray_bystander").await;
        let mut bystander_rx = bystander_tx.subscribe();
        let phone_tx = add_test_client_mapped(&ctx, "sm_stray_phone", "sm-stray-phone").await;
        let mut phone_rx = phone_tx.subscribe();

        handle_screen_mirror(
            serde_json::json!({ "type": "screen_mirror", "action": "frame", "data": "QQ==" }),
            "sm_stray_phone",
            &ctx,
        )
        .await;

        let stray =
            tokio::time::timeout(std::time::Duration::from_millis(200), bystander_rx.recv()).await;
        assert!(stray.is_err(), "frames must not be broadcast to everyone");
        let echoed =
            tokio::time::timeout(std::time::Duration::from_millis(150), phone_rx.recv()).await;
        assert!(echoed.is_err(), "frames must not be echoed to the sender");
    }

    // ── behaviour that must not regress ─────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_stop_happy_acks_capture_stopped() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();

        let msg = serde_json::json!({ "type": "screen_mirror", "action": "stop" });
        handle_screen_mirror(msg, "c1", &ctx).await;

        let received =
            tokio::time::timeout(std::time::Duration::from_millis(1000), rx.recv()).await;
        assert!(received.is_ok(), "stop must ack the requester");
        let text = received.unwrap().unwrap();
        assert!(
            text.contains("capture_stopped"),
            "expected capture_stopped ack, got: {text}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_invalid_unknown_action_is_noop() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();

        let msg = serde_json::json!({ "type": "screen_mirror", "action": "hologram" });
        handle_screen_mirror(msg, "c1", &ctx).await;

        let got = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(got.is_err(), "unknown action must not produce a response");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_invalid_key_without_key_field_returns_early() {
        let ctx = create_test_ctx();
        // The protocol type rejects the frame, so nothing is injected.
        let msg = serde_json::json!({ "type": "screen_mirror", "action": "key" });
        handle_screen_mirror(msg, "c1", &ctx).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_frame_from_client_is_acked_with_noop() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();

        let msg = serde_json::json!({ "type": "screen_mirror", "action": "frame", "data": "xx" });
        handle_screen_mirror(msg, "c1", &ctx).await;

        // No viewer is watching, so the frame is dropped: nothing is sent back.
        let got = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(got.is_err(), "frame messages are not acknowledged");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_scroll_zero_delta_is_guarded_noop() {
        let ctx = create_test_ctx();
        let msg =
            serde_json::json!({ "type": "screen_mirror", "action": "scroll", "dx": 0, "dy": 0 });
        handle_screen_mirror(msg, "c1", &ctx).await;
        // dy==0 && dx==0 → the guard returns before ENIGO is even locked.
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_missing_action_is_noop() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();

        handle_screen_mirror(serde_json::json!({ "type": "screen_mirror" }), "c1", &ctx).await;

        let got = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(got.is_err(), "missing action must not produce a response");
    }

    // ── unauthorized access (auth gate lives in WsServer::handle_message) ────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn screen_mirror_unauthenticated_rejected_by_dispatcher_never_reaches_handler() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "unpaired_ws").await;
        let mut rx = tx.subscribe();

        // `stop` from an unpaired client must be gated before any handler runs.
        let text = serde_json::to_string(&serde_json::json!({
            "type": "screen_mirror",
            "action": "stop"
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
            !resp.contains("capture_stopped"),
            "unpaired client must not receive capture_stopped (handler never ran)"
        );
    }
}
