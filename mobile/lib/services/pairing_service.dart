import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:permission_handler/permission_handler.dart';
import 'encryption_service.dart';
import 'websocket_service.dart';

/// Pairing state
enum PairingState { idle, scanning, connecting, connected, failed }

/// Data from a scanned QR code
class PairingData {
  final String token;
  final String publicKey;
  final String deviceName;
  final String deviceType;

  PairingData({
    required this.token,
    required this.publicKey,
    required this.deviceName,
    required this.deviceType,
  });

  factory PairingData.fromJson(Map<String, dynamic> json) {
    return PairingData(
      token: json['token'] as String,
      publicKey: json['public_key'] as String,
      deviceName: json['device_name'] as String? ?? 'Unknown',
      deviceType: json['device_type'] as String? ?? 'desktop',
    );
  }
}

/// Service for pairing with desktop via QR code or manual token.
/// Uses X25519 key exchange matching the desktop's x25519-dalek.
class PairingService extends ChangeNotifier {
  final EncryptionService _encryptionService;

  PairingState _state = PairingState.idle;
  PairingData? _pairedDevice;
  bool _hasCameraPermission = false;
  String? _error;
  String? _sharedSecret;

  PairingState get state => _state;
  PairingData? get pairedDevice => _pairedDevice;
  bool get hasCameraPermission => _hasCameraPermission;
  String? get error => _error;
  String? get publicKey => _encryptionService.publicKeyHex;
  String? get sharedSecret => _sharedSecret;

  PairingService(this._encryptionService);

  /// Request camera permission for QR scanning
  Future<bool> requestCameraPermission() async {
    final status = await Permission.camera.request();
    _hasCameraPermission = status.isGranted;
    notifyListeners();
    return _hasCameraPermission;
  }

  /// Parse QR code data and initiate pairing via WebSocket
  Future<bool> pairFromQr(String qrData, WebSocketService wsService) async {
    try {
      _state = PairingState.connecting;
      _error = null;
      notifyListeners();

      final data = jsonDecode(qrData) as Map<String, dynamic>;
      final pairingData = PairingData.fromJson(data);

      if (pairingData.token.isEmpty || pairingData.publicKey.isEmpty) {
        throw Exception('Invalid pairing data');
      }

      // Send pairing request with our X25519 public key
      wsService.sendPairingRequest(
        pairingData.token,
        _encryptionService.publicKeyHex,
        {
          'name': wsService.deviceName ?? 'Mobile Device',
          'type': 'phone',
          'os': 'android',
        },
      );

      // Derive shared secret from desktop's X25519 public key
      final secret = await _encryptionService.deriveSharedSecret(pairingData.publicKey);
      _sharedSecret = EncryptionService.bytesToHex(secret);

      _pairedDevice = pairingData;
      _state = PairingState.connected;
      notifyListeners();
      return true;
    } catch (e) {
      _state = PairingState.failed;
      _error = 'Failed to parse QR code: $e';
      notifyListeners();
      return false;
    }
  }

  /// Pair from manual code entry (token only — desktop must be on same network)
  /// Returns true only when the desktop accepts (via pairing/accept handler).
  /// Callers should listen to state changes; this future resolves after the
  /// accept window without faking success.
  Future<bool> pairFromCode(String code, WebSocketService wsService) async {
    try {
      _state = PairingState.connecting;
      _error = null;
      notifyListeners();

      // Send pairing request with our X25519 public key
      wsService.sendPairingRequest(
        code,
        _encryptionService.publicKeyHex,
        {
          'name': wsService.deviceName ?? 'Mobile Device',
          'type': 'phone',
          'os': 'android',
        },
      );

      // Wait briefly for acceptance (the response comes via WS handler
      // which calls completeKeyExchange + marks connected).
      for (var i = 0; i < 10; i++) {
        await Future.delayed(const Duration(seconds: 1));
        if (_state != PairingState.connecting) break;
      }

      if (_state == PairingState.connecting) {
        // No accept received — do NOT fake success.
        _state = PairingState.failed;
        _error = 'Pairing timed out: desktop did not accept within 10s';
        notifyListeners();
        return false;
      }

      return _state == PairingState.connected;
    } catch (e) {
      _state = PairingState.failed;
      _error = 'Failed to connect: $e';
      notifyListeners();
      return false;
    }
  }

  /// Called when we receive the desktop's public key from pairing/accept
  Future<void> completeKeyExchange(String desktopPublicKeyHex) async {
    try {
      final secret = await _encryptionService.deriveSharedSecret(desktopPublicKeyHex);
      _sharedSecret = EncryptionService.bytesToHex(secret);
      notifyListeners();
    } catch (e) {
      debugPrint('Key exchange failed: $e');
    }
  }

  /// Disconnect from paired device
  void disconnect() {
    _pairedDevice = null;
    _state = PairingState.idle;
    _error = null;
    _sharedSecret = null;
    notifyListeners();
  }

  /// Reset pairing state
  void reset() {
    _state = PairingState.idle;
    _error = null;
    _sharedSecret = null;
    notifyListeners();
  }
}
