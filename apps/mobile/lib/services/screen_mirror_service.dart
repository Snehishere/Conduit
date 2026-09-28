import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'websocket_service.dart';

class ScreenMirrorService extends ChangeNotifier {
  static const MethodChannel _channel = MethodChannel('com.conduit.mobile/native');

  final WebSocketService _ws;
  bool _isMirroring = false;
  String? _targetDeviceId;
  Timer? _frameTimer;
  Uint8List? _currentFrame;

  ScreenMirrorService(this._ws);

  bool get isMirroring => _isMirroring;
  String? get targetDeviceId => _targetDeviceId;
  Uint8List? get currentFrame => _currentFrame;

  void _handleMessage(Map<String, dynamic> msg) {
    final action = msg['action'] as String?;
    if (action == 'frame') {
      final b64 = msg['data'] as String?;
      if (b64 != null) {
        _currentFrame = base64Decode(b64);
        notifyListeners();
      }
    } else if (action == 'capture_stopped') {
      stopMirroring();
    }
  }

  Future<bool> startMirroring({
    required String deviceId,
    String quality = 'medium',
    int fps = 15,
  }) async {
    if (_isMirroring) return false;

    _targetDeviceId = deviceId;
    _isMirroring = true;
    _ws.registerHandler('screen_mirror', _handleMessage);

    // Ask the desktop to start capturing its screen: this screen shows the
    // desktop's frames, not this phone's.
    _ws.sendMessage({
      'type': 'screen_mirror',
      'action': 'start',
      'device_id': deviceId,
      'quality': quality,
      'fps': fps,
    });

    notifyListeners();
    return true;
  }

  Future<void> stopMirroring() async {
    if (!_isMirroring) return;

    _isMirroring = false;
    _targetDeviceId = null;
    _currentFrame = null;
    _ws.unregisterHandler('screen_mirror', _handleMessage);

    // Send stop message to desktop
    _ws.sendMessage({
      'type': 'screen_mirror',
      'action': 'stop',
    });
    
    notifyListeners();
  }

  // Handle touch events from desktop
  Future<void> handleTouchEvent({
    required double x,
    required double y,
    required String actionType,
  }) async {
    try {
      await _channel.invokeMethod('injectTouch', {
        'x': x,
        'y': y,
        'actionType': actionType,
      });
    } catch (e) {
      debugPrint('Failed to inject touch event: $e');
    }
  }

  // Handle key events from desktop
  Future<void> handleKeyEvent({
    required String key,
    List<String>? modifiers,
  }) async {
    try {
      await _channel.invokeMethod('injectKey', {
        'key': key,
        'modifiers': modifiers ?? [],
      });
    } catch (e) {
      debugPrint('Failed to inject key event: $e');
    }
  }

  @override
  void dispose() {
    _frameTimer?.cancel();
    _frameTimer = null;
    // stopMirroring() notifies listeners, and it runs its whole body
    // synchronously, so it has to happen while the notifier is still alive —
    // notifyListeners() after super.dispose() throws.
    unawaited(stopMirroring());
    super.dispose();
  }
}
