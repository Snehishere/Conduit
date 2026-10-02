import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/services/call_service.dart';
import 'package:conduit/services/native_screen_capture.dart';
import 'package:conduit/services/notification_service.dart';

import 'native_event_bus.dart';

/// Stands in for the local-notifications plugin registration that never runs
/// under `flutter test`.
///
/// `NotificationService.initialize` awaits `FlutterLocalNotifications.initialize`
/// before it subscribes, and that resolves
/// `resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>()`
/// — which is null unless `FlutterLocalNotificationsPlatform.instance` has been
/// set by the generated plugin registrant. Leaving it unset makes initialize()
/// throw a `LateInitializationError` before the subscription is ever reached,
/// which is exactly the hop these tests are about.
class _NoLocalNotificationsPlatform extends FlutterLocalNotificationsPlatform {}

const MethodChannel _native = MethodChannel('com.conduit.mobile/native');

/// One byte of JPEG: enough to prove a payload survived base64 decoding.
final _jpeg = base64Encode(<int>[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late NativeEventBus bus;
  late List<Map<String, dynamic>> sent;
  late NotificationService notifications;
  late CallService calls;
  late NativeScreenCapture capture;

  setUp(() async {
    FlutterLocalNotificationsPlatform.instance =
        _NoLocalNotificationsPlatform();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_native, (MethodCall call) async => true);

    bus = NativeEventBus()
      ..watch(notificationEventsChannel)
      ..watch(callEventsChannel)
      ..watch(screenMirrorEventsChannel);
    sent = <Map<String, dynamic>>[];

    notifications = NotificationService()..setSendFunction(sent.add);
    calls = CallService()..setSendFunction(sent.add);
    capture = NativeScreenCapture()..setSendFunction(sent.add);

    // The order `main()` uses: notifications (:74), calls (:107), then the
    // screen-mirror relay (:346). On a single shared channel name that order
    // decided which two streams were dead; on three names it is irrelevant,
    // and the test below does not care which order it runs in.
    await notifications.initialize();
    await calls.initialize();
    capture.attach();
    await pumpEventQueue();
  });

  tearDown(() {
    capture.dispose();
    calls.dispose();
    notifications.dispose();
    bus.uninstall();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_native, null);
  });

  test('each stream subscribes on a channel name of its own', () {
    // One `listen` each, on three different names. Had any two shared a name,
    // that name would show two listens and the other none — and only the last
    // subscriber on it would ever have received an event.
    expect(bus.listenCount(notificationEventsChannel), 1);
    expect(bus.listenCount(callEventsChannel), 1);
    expect(bus.listenCount(screenMirrorEventsChannel), 1);
    expect(bus.isListening(notificationEventsChannel), isTrue);
    expect(bus.isListening(callEventsChannel), isTrue);
    expect(bus.isListening(screenMirrorEventsChannel), isTrue);
  });

  test('no stream passes a listen argument, because nothing routes on one', () {
    // The argument only ever travels in the `listen`/`cancel` control
    // messages; incoming events are dispatched by channel name. Recording it
    // documents that these streams no longer pretend it selects a sink.
    for (final String channel in <String>[
      notificationEventsChannel,
      callEventsChannel,
      screenMirrorEventsChannel,
    ]) {
      expect(bus.listenArguments(channel), <Object?>[null], reason: channel);
    }
  });

  test('notifications, calls and screen frames are all live at once', () async {
    // The regression. On the shared `com.conduit.mobile/events` channel only
    // the last of these three subscriptions survived: the screen-mirror relay
    // evicted both notification and call, so the phone recorded zero
    // notifications and zero call-state events and forwarded none, while the
    // frames got through. One event per channel name, all three must land.
    bus.emit(notificationEventsChannel, <String, dynamic>{
      'type': 'notification',
      'action': 'post',
      'id': 'n1',
      'device_id': 'phone',
      'app': 'WhatsApp',
      'title': 'Ada',
      'body': 'hello',
      'timestamp': 1700000000,
    });
    bus.emit(callEventsChannel, <String, dynamic>{
      'type': 'call_state',
      'state': 'ringing',
      'number': '+15551234',
    });
    bus.emit(screenMirrorEventsChannel, <String, dynamic>{
      'type': 'screen_mirror',
      'action': 'frame',
      'data': _jpeg,
      'width': 540,
      'height': 1170,
    });
    await pumpEventQueue();

    expect(notifications.notifications.map((n) => n.id), <String>[
      'n1',
    ], reason: 'the notification subscriber must still be live');
    expect(calls.state, CallState.ringing);
    expect(calls.currentCall?.number, '+15551234');

    // ... and each one still reached the hub on its own path.
    expect(sent.where((m) => m['type'] == 'notification'), hasLength(1));
    expect(sent.where((m) => m['type'] == 'call'), hasLength(1));
    expect(
      sent.where((m) => m['type'] == 'screen_mirror' && m['action'] == 'frame'),
      hasLength(1),
    );
    expect(sent, hasLength(3));
  });

  test('one stream still filters on type before acting', () async {
    // The channel is private to a stream now, but the payload check is the
    // only thing standing between a native bug on that channel and a wrong
    // action, so it stays.
    bus.emit(callEventsChannel, <String, dynamic>{
      'type': 'notification',
      'action': 'post',
      'id': 'n1',
    });
    bus.emit(notificationEventsChannel, <String, dynamic>{
      'type': 'call_state',
      'state': 'ringing',
      'number': '+15551234',
    });
    await pumpEventQueue();

    expect(notifications.notifications, isEmpty);
    expect(calls.state, CallState.idle);
    expect(sent, isEmpty);
  });

  test('cancelling one stream leaves the others receiving', () async {
    // `MainActivity`'s onCancel for these channels nulls one sink and
    // deliberately does not call stopListening(), because the notification
    // receiver and the phone-state listener are shared and startListening() is
    // guarded by `listeningStarted`. The Dart half of that has to hold too:
    // tearing down one subscription must not disturb the other two.
    capture.dispose();
    await pumpEventQueue();
    expect(bus.wasCancelled(screenMirrorEventsChannel), isTrue);

    bus.emit(notificationEventsChannel, <String, dynamic>{
      'type': 'notification',
      'action': 'post',
      'id': 'n1',
    });
    bus.emit(callEventsChannel, <String, dynamic>{
      'type': 'call_state',
      'state': 'ringing',
      'number': '+15551234',
    });
    bus.emit(screenMirrorEventsChannel, <String, dynamic>{
      'type': 'screen_mirror',
      'action': 'frame',
      'data': _jpeg,
      'width': 8,
      'height': 8,
    });
    await pumpEventQueue();

    expect(notifications.notifications, hasLength(1));
    expect(calls.state, CallState.ringing);
    expect(
      sent.where((m) => m['type'] == 'screen_mirror'),
      isEmpty,
      reason: 'the cancelled relay must stop relaying',
    );
  });

  group('why the streams cannot share a channel name', () {
    // Two `receiveBroadcastStream` calls on one name are mutually exclusive,
    // and this is the framework's own behaviour rather than something the
    // harness models: `channel_buffers.dart` replaces the message handler for a
    // name on every listen ("Only one listener may be set at a time. Setting a
    // new listener clears the previous one."), and `EventChannel` likewise
    // keeps one `activeSink` per name. The first stream is not errored, it is
    // simply never fed again — which is why this shipped looking healthy.

    const String shared = 'com.conduit.mobile/events';

    test('the second listener on a name silently kills the first', () async {
      bus.watch(shared);
      const channel = EventChannel(shared);

      final first = <Object?>[];
      final second = <Object?>[];
      final subA = channel.receiveBroadcastStream().listen(first.add);
      await pumpEventQueue();
      final subB = channel.receiveBroadcastStream().listen(second.add);
      await pumpEventQueue();

      bus.emit(shared, 'one');
      await pumpEventQueue();

      expect(second, <Object?>['one'], reason: 'the last subscriber wins');
      expect(
        first,
        isEmpty,
        reason: 'the earlier subscriber on the same name is dead, silently',
      );
      expect(
        bus.listenCount(shared),
        2,
        reason: 'both listens reached the platform; only one sink survived',
      );

      await subA.cancel();
      await subB.cancel();
    });

    test('and cancelling the surviving listener does not revive it', () async {
      bus.watch(shared);
      const channel = EventChannel(shared);

      final first = <Object?>[];
      final subA = channel.receiveBroadcastStream().listen(first.add);
      await pumpEventQueue();
      await subA.cancel();
      await pumpEventQueue();

      final second = <Object?>[];
      final subB = channel.receiveBroadcastStream().listen(second.add);
      await pumpEventQueue();
      bus.emit(shared, 'two');
      await pumpEventQueue();

      expect(first, isEmpty);
      expect(second, <Object?>['two']);

      await subB.cancel();
    });
  });
}
