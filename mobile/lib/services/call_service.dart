import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Call state detected on the phone
enum CallState { idle, ringing, active, ended }

/// Call event data
class CallEvent {
  final String callId;
  final String number;
  final String? name;
  final CallState state;
  final String deviceId;
  final int timestamp;

  CallEvent({
    required this.callId,
    required this.number,
    this.name,
    required this.state,
    required this.deviceId,
    required this.timestamp,
  });

  Map<String, dynamic> toJson() => {
    'call_id': callId,
    'number': number,
    'name': name,
    'state': state.name,
    'device_id': deviceId,
    'timestamp': timestamp,
  };
}

/// Audio output route
enum AudioOutputRoute { phone, desktop, bluetooth }

/// Service for detecting call state and routing audio on Android.
/// Uses EventChannel to receive real-time call state from native PhoneStateListener.
class CallService extends ChangeNotifier {
  static const _eventChannel = EventChannel('com.conduit.mobile/events');

  CallState _state = CallState.idle;
  CallEvent? _currentCall;
  AudioOutputRoute _currentRoute = AudioOutputRoute.phone;
  bool _initialized = false;
  StreamSubscription? _callSubscription;
  void Function(Map<String, dynamic>)? _sendMessage;

  CallState get state => _state;
  CallEvent? get currentCall => _currentCall;
  AudioOutputRoute get currentRoute => _currentRoute;

  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }

  /// Initialize call state detection.
  Future<void> initialize() async {
    if (_initialized) return;
    _initialized = true;

    try {
      // Listen for call events from native PhoneStateListener
      _callSubscription = _eventChannel.receiveBroadcastStream('calls').listen(
        (event) {
          if (event is Map) {
            final type = event['type'] as String?;
            if (type == 'call_state') {
              final state = event['state'] as String?;
              final number = event['number'] as String? ?? 'unknown';
              switch (state) {
                case 'ringing':
                  handleIncomingCall(
                    'call_${DateTime.now().millisecondsSinceEpoch}',
                    number,
                    null,
                  );
                  break;
                case 'active':
                  if (_currentCall != null) {
                    handleCallAnswered(_currentCall!.callId);
                  }
                  break;
                case 'idle':
                  if (_currentCall != null) {
                    handleCallEnded(_currentCall!.callId);
                  }
                  break;
              }
            }
          }
        },
        onError: (error) {
          debugPrint('Call event stream error: $error');
        },
      );
      debugPrint('CallService initialized with native listener');
    } catch (e) {
      debugPrint('Failed to initialize CallService: $e');
    }
  }

  /// Set audio output route.
  Future<void> setAudioRoute(AudioOutputRoute route) async {
    try {
      _currentRoute = route;
      _sendMessage?.call({
        'type': 'audio',
        'action': 'route',
        'route': route.name,
      });
      notifyListeners();
      debugPrint('Audio route set to: ${route.name}');
    } catch (e) {
      debugPrint('Failed to set audio route: $e');
    }
  }

  /// Answer an incoming call.
  Future<void> answerCall() async {
    try {
      if (_currentCall != null) {
        _state = CallState.active;
        _currentCall = CallEvent(
          callId: _currentCall!.callId,
          number: _currentCall!.number,
          name: _currentCall!.name,
          state: CallState.active,
          deviceId: _currentCall!.deviceId,
          timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
        );
        notifyListeners();
      }
    } catch (e) {
      debugPrint('Failed to answer call: $e');
    }
  }

  /// Reject/end a call.
  Future<void> rejectCall() async {
    try {
      _state = CallState.idle;
      _currentCall = null;
      notifyListeners();
    } catch (e) {
      debugPrint('Failed to reject call: $e');
    }
  }

  /// Forward call to another device (send via WebSocket).
  void forwardCall(String toDeviceId) {
    _sendMessage?.call({
      'type': 'call',
      'action': 'forward',
      'call_id': _currentCall?.callId ?? '',
      'to_device_id': toDeviceId,
    });
    debugPrint('Forwarding call to: $toDeviceId');
  }

  /// Handle incoming call event from platform channel.
  void handleIncomingCall(String callId, String number, String? name) {
    _state = CallState.ringing;
    _currentCall = CallEvent(
      callId: callId,
      number: number,
      name: name,
      state: CallState.ringing,
      deviceId: 'phone',
      timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    notifyListeners();
  }

  /// Handle call answered event from platform channel.
  void handleCallAnswered(String callId) {
    _state = CallState.active;
    if (_currentCall != null) {
      _currentCall = CallEvent(
        callId: callId,
        number: _currentCall!.number,
        name: _currentCall!.name,
        state: CallState.active,
        deviceId: 'phone',
        timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
      );
    }
    notifyListeners();
  }

  /// Handle call ended event from platform channel.
  void handleCallEnded(String callId) {
    _state = CallState.idle;
    _currentCall = null;
    notifyListeners();
  }

  @override
  void dispose() {
    _callSubscription?.cancel();
    super.dispose();
  }
}
