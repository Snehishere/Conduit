import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'websocket_service.dart';

class ScreenMirrorService {
  static const MethodChannel _channel = MethodChannel('com.conduit.mobile/native');
  static const EventChannel _eventChannel = EventChannel('com.conduit.mobile/events');

  final WebSocketService _ws;
  bool _isMirroring = false;
  String? _targetDeviceId;
  Timer? _frameTimer;
  StreamSubscription? _frameSub;

  ScreenMirrorService(this._ws);

  bool get isMirroring => _isMirroring;
  String? get targetDeviceId => _targetDeviceId;

  Future<bool> startMirroring({
    required String deviceId,
    String quality = 'medium',
    int fps = 15,
  }) async {
    if (_isMirroring) return false;

    _targetDeviceId = deviceId;
    _isMirroring = true;

    // Tell native to start screen capture
    try {
      await _channel.invokeMethod('startScreenMirror', {
        'quality': quality,
        'fps': fps,
      });
    } catch (e) {
      debugPrint('Failed to start native screen mirror: $e');
    }

    // Send start message to desktop
    _ws.sendMessage({
      'type': 'screen_mirror',
      'action': 'start',
      'device_id': deviceId,
      'quality': quality,
      'fps': fps,
    });

    // Relay native frames (EventChannel 'screen_mirror') to the desktop via WS.
    await _frameSub?.cancel();
    _frameSub = _eventChannel.receiveBroadcastStream('screen_mirror').listen(
      (event) {
        if (event is Map && _isMirroring) {
          final frame = Map<String, dynamic>.from(event);
          frame['device_id'] ??= deviceId;
          _ws.sendMessage(frame);
        }
      },
      onError: (e) {
        debugPrint('Screen mirror frame stream error: $e');
      },
    );

    return true;
  }

  Future<void> stopMirroring() async {
    if (!_isMirroring) return;

    _frameTimer?.cancel();
    _frameTimer = null;
    await _frameSub?.cancel();
    _frameSub = null;

    if (_targetDeviceId != null) {
      _ws.sendMessage({
        'type': 'screen_mirror',
        'action': 'stop',
        'device_id': _targetDeviceId!,
      });
    }

    _isMirroring = false;
    _targetDeviceId = null;

    try {
      await _channel.invokeMethod('stopScreenMirror');
    } catch (e) {
      // Ignore
    }
  }

  // Handle touch events from desktop
  void handleTouchEvent({
    required double x,
    required double y,
    required String actionType,
  }) {
    _channel.invokeMethod('injectTouch', {
      'x': x,
      'y': y,
      'actionType': actionType,
    });
  }

  // Handle key events from desktop
  void handleKeyEvent({
    required String key,
    List<String>? modifiers,
  }) {
    _channel.invokeMethod('injectKey', {
      'key': key,
      'modifiers': modifiers ?? [],
    });
  }

  void dispose() {
    _frameTimer?.cancel();
    stopMirroring();
  }
}
