import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/services/native_screen_capture.dart';

const _events = MethodChannel('com.conduit.mobile/events');

/// The JPEG the native side encodes. One byte is enough to prove a payload
/// survived base64 decoding, and keeps the assertions readable.
final _jpeg = base64Encode(<int>[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);

/// Models the platform side of `com.conduit.mobile/events`.
///
/// Both native implementations keep one sink *per argument* —
/// `ConduitStreamHandler.sinks` on iOS, the `screenMirrorEventSink` field on
/// Android — so an event published under one argument is delivered only to the
/// listener that asked for that argument. Modelling that is what makes the
/// argument load-bearing in the tests below rather than decorative.
class _NativeEventBus {
  final _listened = <Object?>{};
  final _listenArgs = <Object?>[];

  void install() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_events, (call) async {
      switch (call.method) {
        case 'listen':
          _listened.add(call.arguments);
          _listenArgs.add(call.arguments);
          return null;
        case 'cancel':
          _listened.remove(call.arguments);
          return null;
        default:
          return null;
      }
    });
  }

  void uninstall() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_events, null);
  }

  /// Every argument a listener has subscribed under, in order.
  List<Object?> get listenArguments => List<Object?>.unmodifiable(_listenArgs);

  /// Publish `event` on the sink for [argument]. A no-op when nothing is
  /// listening for it, which is what a native sink with no Dart listener does.
  void emit(Object? argument, Map<String, dynamic> event) {
    if (!_listened.contains(argument)) return;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
      _events.name,
      const StandardMethodCodec().encodeSuccessEnvelope(event),
      (_) {},
    );
  }
}

/// The argument the Android and iOS capture pipelines publish under.
const _nativeMirrorSink = 'screen_mirror';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late _NativeEventBus bus;
  late List<Map<String, dynamic>> sent;
  late NativeScreenCapture capture;

  setUp(() async {
    bus = _NativeEventBus()..install();
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

  test('the native screen stream is subscribed with the screen_mirror argument',
      () async {
    // The two other subscribers of this channel are 'notifications' and
    // 'calls'. Listening under any other argument would tap someone else's
    // sink, and the frames would still go nowhere.
    expect(bus.listenArguments, <Object?>[_nativeMirrorSink]);
  });

  test('an Android-shaped native frame is forwarded to the hub as a frame',
      () async {
    // `ScreenMirrorService.kt:154-161`.
    bus.emit(_nativeMirrorSink, {
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
    expect(base64Decode(frame['data'] as String), <int>[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
  });

  test('the legacy iOS payload shape is understood too', () async {
    // `AppDelegate.swift` used to emit `screen_mirror_frame` with the JPEG
    // under `frame`. A reader that only knows the canonical shape silently
    // drops every frame from an iOS build that still emits this.
    bus.emit(_nativeMirrorSink, {
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
    expect(base64Decode(sent.single['data'] as String),
        <int>[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
  });

  test('another stream on the same channel is not mistaken for a frame',
      () async {
    // The channel is shared. Nothing that is not a frame may be relayed as
    // one — and the dangerous case is a *different* payload that happens to
    // carry a base64 `data`, which is exactly the legacy `file`/`chunk` shape
    // `main.dart:222-221` still reads off this channel.
    bus.emit(_nativeMirrorSink, {'type': 'notification', 'action': 'post', 'id': 'n1'});
    bus.emit(_nativeMirrorSink, {'type': 'call_state', 'state': 'idle'});
    bus.emit(_nativeMirrorSink, {
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
    bus.emit(_nativeMirrorSink, {'type': 'screen_mirror', 'action': 'frame'}); // no data
    bus.emit(_nativeMirrorSink, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': 'not base64 !!!',
      'width': 1,
      'height': 1,
    });
    bus.emit(_nativeMirrorSink, {'type': 'screen_mirror', 'action': 'start'}); // control frame
    await pumpEventQueue();

    expect(sent, isEmpty);
  });

  test('an oversized frame is refused before it reaches the socket', () async {
    // A frame is the user's screen. A native payload that will not fit in the
    // hub's message budget is refused here rather than shipped and dropped.
    bus.emit(_nativeMirrorSink, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': base64Encode(List<int>.filled(nativeScreenCaptureMaxFrameBytes + 1, 0)),
      'width': 8,
      'height': 8,
    });
    await pumpEventQueue();

    expect(sent, isEmpty);
  });

  test('a frame at exactly the size cap is still relayed', () async {
    bus.emit(_nativeMirrorSink, {
      'type': 'screen_mirror',
      'action': 'frame',
      'data': base64Encode(List<int>.filled(nativeScreenCaptureMaxFrameBytes, 0)),
      'width': 8,
      'height': 8,
    });
    await pumpEventQueue();

    expect(sent, hasLength(1));
  });

  test('disposing cancels the subscription and stops relaying', () async {
    capture.dispose();
    bus.emit(_nativeMirrorSink, {
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