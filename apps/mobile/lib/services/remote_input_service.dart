import 'dart:async';
import 'websocket_service.dart';

class RemoteInputService {
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

  void sendMouseMove(double dx, double dy) {
    if (!_isConnected || _targetDeviceId == null) return;
    _ws.sendMessage({
      'type': 'remote_input',
      'action': 'move',
      'device_id': _targetDeviceId!,
      'dx': dx,
      'dy': dy,
    });
  }

  void sendMouseClick(String button) {
    if (!_isConnected || _targetDeviceId == null) return;
    _ws.sendMessage({
      'type': 'remote_input',
      'action': 'click',
      'device_id': _targetDeviceId!,
      'button': button,
    });
  }

  void sendScroll(double dx, double dy) {
    if (!_isConnected || _targetDeviceId == null) return;
    _ws.sendMessage({
      'type': 'remote_input',
      'action': 'scroll',
      'device_id': _targetDeviceId!,
      'dx': dx,
      'dy': dy,
    });
  }

  void dispose() {
    stopRemoteInput();
  }
}
