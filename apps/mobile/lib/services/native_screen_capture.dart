import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'websocket_service.dart';

/// Largest JPEG, in decoded bytes, this relay will put on the wire.
///
/// The hub accepts a message up to `MAX_MESSAGE_SIZE` (50 MiB,
/// `apps/desktop/src-tauri/src/security.rs:256`) *after* the ~4/3 base64
/// inflation, so the transport is not the binding constraint — this is. A
/// screen frame is the user's screen: an unbounded blob arriving from the
/// platform channel is refused here, at the only point where it is still
/// attributable to the native capture session, instead of being shipped to the
/// hub to be dropped there.
const int nativeScreenCaptureMaxFrameBytes = 4 * 1024 * 1024;

/// Relays this phone's native screen capture to the desktop.
///
/// # The hop this exists for
///
/// The capture side of a mirror session is the device that *has* the screen.
/// When the desktop asks to see a phone, the hub forwards `screen_mirror/
/// start` to that phone, the phone starts native capture (`MediaProjection`
/// on Android, `ReplayKit` on iOS) and the native side then pushes JPEG frames
/// onto `EventChannel('com.conduit.mobile/screen_mirror_events')` — see
/// `ScreenMirrorService.kt:152-165` and `AppDelegate.swift:399`.
///
/// Nothing on the Dart side read that stream. It was the third subscriber of a
/// `com.conduit.mobile/events` channel shared with notifications and calls, and
/// because `EventChannel` delivers to one sink per channel *name*, subscribing
/// here registered last and silently killed the other two: every frame the
/// native capture produced was produced and discarded, and so were every
/// notification and call event. Each stream now owns its own channel name.
///
/// # What it does *not* do
///
/// It adds no gate and removes none. Native capture still requires the OS
/// consent dialog (`MediaProjection.createScreenCaptureIntent`, the ReplayKit
/// `RPScreenRecorder.isAvailable` check), still only begins in response to a
/// `screen_mirror`/`start` the hub forwarded from a trusted peer, and the
/// frames still leave through `WebSocketService.sendMessage`, which wraps them
/// in the `encrypted` envelope the hub verifies. This only supplies the
/// missing reader.
class NativeScreenCapture {
  /// The channel the phone's native side publishes frames on. Dedicated to this
  /// stream: see the class doc for why it must not be shared.
  static const EventChannel _channel = EventChannel(
    'com.conduit.mobile/screen_mirror_events',
  );

  void Function(Map<String, dynamic>)? _sendMessage;
  StreamSubscription<dynamic>? _subscription;
  bool _attached = false;

  static NativeScreenCapture? _installed;

  /// Install the relay for the lifetime of the app. Idempotent.
  ///
  /// This has to be called from `main`, once, next to the other services:
  /// capture is started by the hub asking this phone to mirror, and the user
  /// never opens `ScreenMirrorScreen` in that direction — they are looking at
  /// the *desktop*. A subscription scoped to that screen would never exist when
  /// the frames do, which is the state the code was in before.
  ///
  ///     NativeScreenCapture.attachTo(websocketService);
  static void attachTo(WebSocketService ws) {
    _installed ??= NativeScreenCapture()
      ..setSendFunction(ws.sendMessage)
      ..attach();
  }

  /// Whether the native stream is currently being read.
  bool get isAttached => _attached;

  /// Start relaying. Idempotent: a second call while attached is a no-op, so
  /// the app can call this from anywhere without coordinating.
  void attach() {
    if (_attached) return;
    _attached = true;
    _subscription = _channel.receiveBroadcastStream().listen(
      _onNativeEvent,
      onError: (Object error) {
        debugPrint('Screen capture stream error: $error');
      },
      cancelOnError: false,
    );
  }

  void _onNativeEvent(Object? event) {
    final frame = normalizeNativeFrame(event);
    if (frame == null) return;
    _sendMessage?.call(frame);
  }

  /// Stop relaying and release the platform sink.
  void dispose() {
    _attached = false;
    unawaited(_subscription?.cancel());
    _subscription = null;
  }

  /// The frame to put on the wire for a native event, or null to drop it.
  ///
  /// Exposed for tests and kept pure so the shape rules are checkable without
  /// a platform channel.
  ///
  /// Two shapes are accepted, and only two:
  ///
  ///  * the canonical one, `{type: screen_mirror, action: frame, data, width,
  ///    height}`, which is what `ScreenMirrorService.kt` emits and what
  ///    `ScreenMirrorFrame` in `packages/protocol/src/types.rs:594`
  ///    deserialises; and
  ///  * the legacy iOS one, `{type: screen_mirror_frame, frame, width, height}`,
  ///    which `AppDelegate.swift` emitted before it was corrected to the
  ///    canonical shape. It is still accepted because an iOS build in the
  ///    field speaks it, and a reader that rejects it drops every frame from
  ///    that build with no error anywhere.
  ///
  /// Everything else is dropped. The channel carries screen frames and nothing
  /// else, and a frame is the user's screen: an event this reader does not
  /// recognise is not something to guess at.
  static Map<String, dynamic>? normalizeNativeFrame(Object? event) {
    if (event is! Map) return null;
    final msg = Map<String, dynamic>.from(event);
    final type = msg['type'];

    final String? encoded;
    if (type == 'screen_mirror') {
      if (msg['action'] != 'frame') return null;
      encoded = msg['data'] as String?;
    } else if (type == 'screen_mirror_frame') {
      encoded = msg['frame'] as String?;
    } else {
      return null;
    }
    if (encoded == null || encoded.isEmpty) return null;

    // Decoded only to prove the payload is real base64 and inside the cap; the
    // wire carries the original encoding, not a re-encode of it.
    if (_decodeBounded(encoded) == null) return null;

    return <String, dynamic>{
      'type': 'screen_mirror',
      'action': 'frame',
      'data': encoded,
      'format': 'jpeg',
      // The desktop sizes its canvas from these
      // (`ScreenMirror.tsx:97-100`), and the protocol requires them, so they
      // are carried through rather than defaulted. A frame that arrives
      // without them is still relayed — the desktop falls back to the element
      // size — because dropping it would lose the picture over metadata.
      'width': _asPositiveInt(msg['width']) ?? 0,
      'height': _asPositiveInt(msg['height']) ?? 0,
    };
  }

  /// Decode `encoded`, refusing anything that is not base64 or is larger than
  /// [nativeScreenCaptureMaxFrameBytes].
  ///
  /// Decoding rather than length-checking the string first is deliberate: it
  /// validates the payload and bounds it in one pass, so malformed base64 is
  /// dropped here instead of being handed to the hub as opaque text.
  static List<int>? _decodeBounded(String encoded) {
    // Base64 expands by 4/3; refuse anything whose encoded form alone already
    // exceeds what the decoded cap allows, so a huge string is rejected
    // without allocating a decoded buffer for it.
    if (encoded.length > (nativeScreenCaptureMaxFrameBytes * 4) ~/ 3 + 4) {
      return null;
    }
    try {
      final bytes = base64Decode(encoded);
      if (bytes.isEmpty || bytes.length > nativeScreenCaptureMaxFrameBytes) {
        return null;
      }
      return bytes;
    } on FormatException {
      return null;
    }
  }

  static int? _asPositiveInt(Object? value) {
    final int parsed;
    if (value is int) {
      parsed = value;
    } else if (value is num) {
      parsed = value.toInt();
    } else if (value is String) {
      parsed = int.tryParse(value) ?? 0;
    } else {
      return null;
    }
    return parsed > 0 ? parsed : null;
  }

  /// Hand the relay the function that puts a message on the wire.
  ///
  /// Same shape as `CallService.setSendFunction` / `SmsService.setSendFunction`:
  /// the service does not own a socket.
  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }
}
