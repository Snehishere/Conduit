import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

typedef MessageHandler = void Function(Map<String, dynamic> message);

class WebSocketService extends ChangeNotifier {
  WebSocketChannel? _channel;
  bool _isConnected = false;
  String? _deviceName;
  String? _deviceId;
  Timer? _reconnectTimer;
  int _reconnectAttempts = 0;
  final Map<String, List<MessageHandler>> _messageHandlers = {};
  final List<Map<String, dynamic>> _connectedDevices = [];

  bool get isConnected => _isConnected;
  String? get deviceName => _deviceName;
  String? get deviceId => _deviceId;
  List<Map<String, dynamic>> get connectedDevices => List.unmodifiable(_connectedDevices);

  WebSocketService();

  void setDeviceId(String id) {
    _deviceId = id;
  }

  void registerHandler(String type, MessageHandler handler) {
    _messageHandlers.putIfAbsent(type, () => []).add(handler);
  }

  void unregisterHandler(String type, [MessageHandler? handler]) {
    if (handler == null) {
      _messageHandlers.remove(type);
    } else {
      _messageHandlers[type]?.remove(handler);
      if (_messageHandlers[type]?.isEmpty ?? false) {
        _messageHandlers.remove(type);
      }
    }
  }

  Future<void> connect(String url) async {
    try {
      _channel = WebSocketChannel.connect(Uri.parse(url));

      await _channel!.ready;

      _isConnected = true;
      _reconnectAttempts = 0;
      notifyListeners();

      _channel!.stream.listen(
        (data) {
          try {
            final message = jsonDecode(data as String) as Map<String, dynamic>;
            _handleMessage(message);
          } catch (e) {
            debugPrint('WS: failed to decode message: $e');
          }
        },
        onDone: () {
          _isConnected = false;
          _connectedDevices.clear();
          notifyListeners();
          _scheduleReconnect(url);
        },
        onError: (e) {
          debugPrint('WS stream error: $e');
          _isConnected = false;
          _connectedDevices.clear();
          notifyListeners();
          _scheduleReconnect(url);
        },
      );

      // Announce presence
      _sendMessage({
        'type': 'discovery',
        'action': 'announce',
        'device_name': _deviceName ?? 'Mobile Device',
        'device_type': 'phone',
        'device_id': _deviceId ?? 'mobile_${DateTime.now().millisecondsSinceEpoch}',
      });
    } catch (e) {
      debugPrint('WebSocket error: $e');
      _isConnected = false;
      notifyListeners();
      _scheduleReconnect(url);
    }
  }

  void _scheduleReconnect(String url) {
    _reconnectTimer?.cancel();
    // Exponential backoff: 3s, 6s, 12s ... capped at 60s.
    _reconnectAttempts += 1;
    final delaySeconds = (3 * (1 << (_reconnectAttempts - 1).clamp(0, 4))).clamp(3, 60);
    _reconnectTimer = Timer(Duration(seconds: delaySeconds), () {
      if (!_isConnected) connect(url);
    });
  }

  void _handleMessage(Map<String, dynamic> message) {
    final type = message['type'] as String?;
    final action = message['action'] as String?;
    debugPrint('WS received: $type/$action');

    // Track connected devices
    if (type == 'pairing' && action == 'accept') {
      final deviceInfo = message['device_info'] as Map<String, dynamic>?;
      if (deviceInfo != null) {
        final device = {
          'device_id': message['device_id'] as String? ?? 'unknown',
          'device_name': deviceInfo['name'] as String? ?? 'Unknown',
          'device_type': deviceInfo['type'] as String? ?? 'desktop',
        };
        _connectedDevices.removeWhere((d) => d['device_id'] == device['device_id']);
        _connectedDevices.add(device);
        notifyListeners();
      }
    } else if (type == 'discovery' && action == 'announce') {
      final device = {
        'device_id': message['device_id'] as String? ?? 'unknown',
        'device_name': message['device_name'] as String? ?? 'Unknown',
        'device_type': message['device_type'] as String? ?? 'desktop',
      };
      _connectedDevices.removeWhere((d) => d['device_id'] == device['device_id']);
      _connectedDevices.add(device);
      notifyListeners();
    } else if (type == 'discovery' && action == 'remove') {
      final deviceId = message['device_id'] as String?;
      if (deviceId != null) {
        _connectedDevices.removeWhere((d) => d['device_id'] == deviceId);
        notifyListeners();
      }
    }

    final handlers = _messageHandlers[type];
    if (type != null && handlers != null) {
      for (final h in List<MessageHandler>.from(handlers)) {
        try {
          h(message);
        } catch (e) {
          debugPrint('WS handler error for $type: $e');
        }
      }
    }
  }

  void sendMessage(Map<String, dynamic> message) {
    _sendMessage(message);
  }

  void _sendMessage(Map<String, dynamic> message) {
    if (_channel != null && _isConnected) {
      _channel!.sink.add(jsonEncode(message));
    }
  }

  void sendClipboardSync(String content, String mime) {
    _sendMessage({
      'type': 'clipboard',
      'action': 'sync',
      'content': content,
      'mime': mime,
      'source_device': _deviceId ?? 'mobile',
      'timestamp': DateTime.now().millisecondsSinceEpoch ~/ 1000,
    });
  }

  void sendNotificationDismiss(String id) {
    _sendMessage({
      'type': 'notification',
      'action': 'dismiss',
      'id': id,
    });
  }

  void sendNotificationReply(String id, String text) {
    _sendMessage({
      'type': 'notification',
      'action': 'reply',
      'id': id,
      'text': text,
    });
  }

  void sendStatusUpdate({int? battery}) {
    _sendMessage({
      'type': 'status',
      'action': 'update',
      if (battery != null) 'battery': battery,
    });
  }

  void sendPairingRequest(String token, String publicKey, Map<String, dynamic> deviceInfo) {
    _sendMessage({
      'type': 'pairing',
      'action': 'request',
      'token': token,
      'public_key': publicKey,
      'device_info': deviceInfo,
    });
  }

  void sendSmsMessage(String to, String body) {
    _sendMessage({
      'type': 'sms',
      'action': 'send',
      'to': to,
      'body': body,
    });
  }

  void sendCallAction(String action, {String? callId, String? toDeviceId, String? route}) {
    _sendMessage({
      'type': 'call',
      'action': action,
      if (callId != null) 'call_id': callId,
      if (toDeviceId != null) 'to_device_id': toDeviceId,
      if (route != null) 'route': route,
    });
  }

  void sendFileRequest(String id, String name, int size, String mime, String toDeviceId) {
    _sendMessage({
      'type': 'file',
      'action': 'request',
      'id': id,
      'name': name,
      'size': size,
      'mime': mime,
      'from': _deviceId ?? 'mobile',
      'to': toDeviceId,
    });
  }

  void sendFileAccept(String id) {
    _sendMessage({'type': 'file', 'action': 'accept', 'id': id});
  }

  void sendFileCancel(String id) {
    _sendMessage({'type': 'file', 'action': 'cancel', 'id': id});
  }

  void sendFileChunk(String id, int index, String dataB64) {
    _sendMessage({
      'type': 'file',
      'action': 'chunk',
      'id': id,
      'index': index,
      'data': dataB64,
    });
  }

  @override
  void dispose() {
    _reconnectTimer?.cancel();
    _channel?.sink.close();
    super.dispose();
  }
}
