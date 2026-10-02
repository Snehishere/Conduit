import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// The EventChannel names the native side publishes each stream on.
///
/// Spelled out here rather than imported from the services under test. The
/// whole point is that the Dart reader and the native publisher have to agree
/// on a string across a process boundary, and `MainActivity.kt` /
/// `AppDelegate.swift` spell these names independently; a test that took the
/// name from the service it is exercising would pass even if both sides had
/// been moved onto the same wrong name.
const String notificationEventsChannel =
    'com.conduit.mobile/notification_events';
const String callEventsChannel = 'com.conduit.mobile/call_events';
const String screenMirrorEventsChannel =
    'com.conduit.mobile/screen_mirror_events';

/// A stand-in for the native half of those EventChannels.
///
/// Needs `TestWidgetsFlutterBinding.ensureInitialized()` before use.
///
/// This models what the platform actually provides, which is deliberately not
/// what the first version of `native_screen_capture_test.dart` claimed:
///
///  * Dart registers its incoming handler by channel **name** alone —
///    `channel_buffers.dart` says *"Only one listener may be set at a time.
///    Setting a new listener clears the previous one."* The listen argument
///    travels only in the `listen`/`cancel` control messages and routes
///    nothing on the way back.
///  * `io.flutter.plugin.common.EventChannel` keeps a single `activeSink` per
///    channel name, replaces it on every `listen`, calls `onCancel(null)` on
///    the sink it displaced, and drops events from any sink that is not the
///    active one.
///
/// So there is exactly one stream per channel name and the argument is
/// decorative: two streams sharing a name are mutually exclusive, and the
/// second to subscribe silently kills the first. The eviction is the Dart
/// framework's own behaviour and is *not* simulated here — it happens for
/// real inside [EventChannel.receiveBroadcastStream] — so a test built on this
/// class exercises the genuine single-slot rule rather than a model of it.
class NativeEventBus {
  /// Channel name -> whether a sink is currently active on it.
  final Map<String, bool> _active = <String, bool>{};

  /// Channel name -> every argument a `listen` arrived with, in order.
  final Map<String, List<Object?>> _listens = <String, List<Object?>>{};

  final Set<String> _watched = <String>{};

  /// Start modelling [channel]: from now on its `listen`/`cancel` control
  /// messages are recorded, and [emit] can reach whatever is subscribed to it.
  void watch(String channel) {
    if (!_watched.add(channel)) return;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(MethodChannel(channel), (
          MethodCall call,
        ) async {
          switch (call.method) {
            case 'listen':
              // Mirrors IncomingStreamRequestHandler.onListen: a new listen
              // replaces whichever sink was active on this name.
              _listens
                  .putIfAbsent(channel, () => <Object?>[])
                  .add(call.arguments);
              _active[channel] = true;
            case 'cancel':
              _active[channel] = false;
            default:
              break;
          }
          return null;
        });
  }

  /// Publish [event] on [channel] the way the native sink would.
  ///
  /// A no-op when nothing is listening, which is what
  /// `EventSinkImplementation.success()` does for a sink that is no longer the
  /// channel's active one: the event is dropped before it is ever encoded.
  void emit(String channel, Object? event) {
    if (_active[channel] != true) return;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          channel,
          const StandardMethodCodec().encodeSuccessEnvelope(event),
          (_) {},
        );
  }

  /// Whether a Dart listener is currently subscribed to [channel].
  bool isListening(String channel) => _active[channel] == true;

  /// How many `listen` control messages [channel] has seen.
  int listenCount(String channel) => _listens[channel]?.length ?? 0;

  /// Every argument a `listen` on [channel] arrived with, in order.
  List<Object?> listenArguments(String channel) =>
      List<Object?>.unmodifiable(_listens[channel] ?? const <Object?>[]);

  /// Whether [channel] has been cancelled at least once.
  bool wasCancelled(String channel) =>
      listenCount(channel) > 0 && _active[channel] == false;

  /// Stop modelling every watched channel.
  void uninstall() {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    for (final String channel in _watched) {
      messenger.setMockMethodCallHandler(MethodChannel(channel), null);
    }
    _watched.clear();
    _active.clear();
    _listens.clear();
  }
}
