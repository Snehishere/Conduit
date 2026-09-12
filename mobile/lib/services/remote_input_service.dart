import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'websocket_service.dart';

class RemoteInputService {
  static const MethodChannel _channel = MethodChannel('com.conduit.mobile/native');

  final WebSocketService _ws;
  bool _isConnected = false;
  String? _targetDeviceId;

  RemoteInputService(this._ws);

  bool get isConnected => _isConnected;
  String? get targetDeviceId => _targetDeviceId;

  Future<bool> startRemoteInput({required String deviceId}) async {
    if (_isConnected) return false;

    _targetDeviceId = deviceId;
    _isConnected = true;

    // Check if accessibility service is enabled
    try {
      final enabled = await _channel.invokeMethod<bool>('isAccessibilityServiceEnabled');
      if (enabled != true) {
        await _channel.invokeMethod('openAccessibilitySettings');
      }
    } catch (e) {
      debugPrint('Failed to check accessibility: $e');
    }

    // Send start message to desktop
    _ws.sendMessage({
      'type': 'remote_input',
      'action': 'start',
      'device_id': deviceId,
    });

    return true;
  }

  Future<void> stopRemoteInput() async {
    if (!_isConnected) return;

    if (_targetDeviceId != null) {
      _ws.sendMessage({
        'type': 'remote_input',
        'action': 'stop',
        'device_id': _targetDeviceId!,
      });
    }

    _isConnected = false;
    _targetDeviceId = null;
  }

  void handleMouseMove({required double x, required double y}) {
    _channel.invokeMethod('injectTouch', {
      'x': x,
      'y': y,
      'actionType': 'move',
    });
  }

  void handleMouseClick({
    required double x,
    required double y,
    required String button,
  }) {
    _channel.invokeMethod('injectTouch', {
      'x': x,
      'y': y,
      // Preserve button distinction: left=tap, right=long_press, middle=double_tap.
      'actionType': button == 'right' ? 'long_press' : (button == 'middle' ? 'double_tap' : 'tap'),
      'button': button,
    });
  }

  void handleKeyPress({
    required String key,
    List<String>? modifiers,
  }) {
    _channel.invokeMethod('injectKey', {
      'key': key,
      'modifiers': modifiers ?? [],
    });
  }

  void handleScroll({required double delta}) {
    _channel.invokeMethod('injectScroll', {
      'delta': delta,
    });
  }

  void dispose() {
    stopRemoteInput();
  }
}
