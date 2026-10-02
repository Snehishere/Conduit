import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/services/native_screen_capture.dart';

import 'native_event_bus.dart';

/// The JPEG the native side encodes. One byte is enough to prove a payload
/// survived base64 decoding, and keeps the assertions readable.
final _jpeg = base64Encode(<int>[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late NativeEventBus bus;
  late List<Map<String, dynamic>> sent;
  late NativeScreenCapture capture;

  setUp(() async {
    bus = NativeEventBus()..watch(screenMirrorEventsChannel);
    sent = <Map<String, dynamic>>[];
    capture = NativeScreenCapture()
      ..setSendFunction(sent.add)
      ..attach();
    // One turn of the loop so the `listen` handshake has reached the platform.
    await pumpEventQueue();
  });

  tearDown(() {
    capture.dispose();
    bus.uninstall();
  });

  test('the native screen stream subscribes on its own channel', () async {
    // `com.conduit.mobile/screen_mirror_events` is not a sub-slot of a shared
    // channel: notifications and calls have their own names
    // (`com.conduit.mobile/notification_events`, `com.conduit.mobile/call_events`)
    // because `EventChannel` routes by name and by nothing else. Listening
    // anywhere else would either find no native sink or displace somebody's.
    expect(bus.listenCount(screenMirrorEventsChannel), 1);
    expect(bus.isListening(screenMirrorEventsChannel), isTrue);
  });

  test(
    'an Android-shaped native frame is forwarded to the hub as a frame',
    () async {
      // `ScreenMirrorService.kt:154-161`.
      bus.emit(screenMirrorEventsChannel, {
        'type': 'screen_mirror',
        'action': 'frame',
        'device_id': 'phone-1',
        'data': _jpeg,
        'width': 540,
        'height': 1170,
        'timestamp': 1700000000,
      });
      await pumpEventQueue();

      expect(sent, hasLength(1));
      final frame = sent.single;
      expect(frame['type'], 'screen_mirror');
      expect(frame['action'], 'frame');
      // The desktop sizes its canvas from these, and `ScreenMirrorFrame`
      // (packages/protocol/src/types.rs:594) requires them.
      expect(frame['width'], 540);
      expect(frame['height'], 1170);
      expect(frame['format'], 'jpeg');
      expect(base64Decode(frame['data'] as String), <int>[
        0xFF,
        0xD8,
        0xFF,
        0xE0,
        0x00,
        0x10,
      ]);
    },
  );

  test('the legacy iOS payload shape is understood too', () async {
    // `AppDelegate.swift` used to emit `screen_mirror_frame` with the JPEG
    // under `frame`. A reader that only knows the canonical shape silently
    // drops every frame from an iOS build that still emits this.
    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror_frame',
      'frame': _jpeg,
      'width': 640,
      'height': 360,
      'timestamp': 1700000000,
    });
    await pumpEventQueue();

    expect(sent, hasLength(1));
    expect(sent.single['type'], 'screen_mirror');
    expect(sent.single['action'], 'frame');
    expect(base64Decode(sent.single['data'] as String), <int>[
      0xFF,
      0xD8,
      0xFF,
      0xE0,
      0x00,
      0x10,
    ]);
  });

  test('a payload that is not a frame is dropped rather than relayed', () async {
    // The channel carries screen frames and nothing else, so the shape check is
    // a second line of defence rather than the routing mechanism. The case
    // that matters is a payload that happens to carry a base64 `data`, which is
    // what would be relayed as a picture if only `data` were looked at.
    bus.emit(screenMirrorEventsChannel, {
      'type': 'notification',
      'action': 'post',
      'id': 'n1',
    });
    bus.emit(screenMirrorEventsChannel, {
      'type': 'call_state',
      'state': 'idle',
    });
    bus.emit(screenMirrorEventsChannel, {
      'type': 'file',
      'action': 'chunk',
      'id': 't1',
      'index': 0,
      'data': _jpeg,
    });
    await pumpEventQueue();

    expect(sent, isEmpty);
  });

  test('a malformed payload is dropped rather than relayed', () async {
    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror',
      'action': 'frame',
    }); // no data
    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': 'not base64 !!!',
      'width': 1,
      'height': 1,
    });
    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror',
      'action': 'start',
    }); // control frame
    await pumpEventQueue();

    expect(sent, isEmpty);
  });

  test('an oversized frame is refused before it reaches the socket', () async {
    // A frame is the user's screen. A native payload that will not fit in the
    // hub's message budget is refused here rather than shipped and dropped.
    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': base64Encode(
        List<int>.filled(nativeScreenCaptureMaxFrameBytes + 1, 0),
      ),
      'width': 8,
      'height': 8,
    });
    await pumpEventQueue();

    expect(sent, isEmpty);
  });

  test('a frame at exactly the size cap is still relayed', () async {
    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': base64Encode(
        List<int>.filled(nativeScreenCaptureMaxFrameBytes, 0),
      ),
      'width': 8,
      'height': 8,
    });
    await pumpEventQueue();

    expect(sent, hasLength(1));
  });

  test('disposing cancels the subscription and stops relaying', () async {
    capture.dispose();
    await pumpEventQueue();
    expect(bus.wasCancelled(screenMirrorEventsChannel), isTrue);

    bus.emit(screenMirrorEventsChannel, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': _jpeg,
      'width': 1,
      'height': 1,
    });
    await pumpEventQueue();

    expect(sent, isEmpty);
  });
}
