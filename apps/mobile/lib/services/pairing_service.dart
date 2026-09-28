import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
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

  Map<String, dynamic> toJson() {
    return {
      'token': token,
      'public_key': publicKey,
      'device_name': deviceName,
      'device_type': deviceType,
    };
  }
}

/// Service for pairing with desktop via QR code or manual token.
/// Uses X25519 key exchange matching the desktop's x25519-dalek.
class PairingService extends ChangeNotifier {
  final EncryptionService _encryptionService;
  static const _secureStorage = FlutterSecureStorage();
  static const _sharedSecretKey = 'pairing_shared_secret';
  static const _pairedDeviceKey = 'pairing_device_data';

  /// TLS port used for a LAN peer when the desktop did not advertise one.
  ///
  /// Mirrors `conduit_protocol::types::LAN_WSS_PORT`. This is the *TLS* port
  /// (9531), never the plaintext WS port (9527): pairing runs over `wss://`
  /// because that handshake is the trust bootstrap which captures the
  /// certificate pin, and a `wss://` dial against the plaintext listener
  /// throws — leaving the pin uncaptured and every later connect failing
  /// closed. There is deliberately no plaintext LAN fallback.
  static const int _wssPort = kLanWssPort;

  /// Build the pairing URL for a LAN peer.
  ///
  /// Always `wss://` — pairing a peer is an explicit act of trust, and the
  /// certificate presented on this handshake is what gets pinned.
  static String lanUrl(String ip, [int? advertisedWssPort]) {
    final port = advertisedWssPort ?? _wssPort;
    return 'wss://$ip:$port';
  }

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

  /// Initialize service and load persisted data
  Future<void> initialize() async {
    await _loadPersistedData();
  }

  /// Load persisted shared secret and device data from secure storage on startup.
  Future<void> _loadPersistedData() async {
    try {
      _sharedSecret = await _secureStorage.read(key: _sharedSecretKey);
      final deviceJsonString = await _secureStorage.read(key: _pairedDeviceKey);
      if (deviceJsonString != null && _sharedSecret != null) {
        _pairedDevice = PairingData.fromJson(jsonDecode(deviceJsonString));
        _state = PairingState.connected;
      }
      notifyListeners();
    } catch (e) {
      debugPrint('Failed to load persisted pairing data: $e');
    }
  }

  /// Persist the shared secret and device data to secure storage.
  Future<void> _persistSharedSecret(String secret) async {
    try {
      await _secureStorage.write(key: _sharedSecretKey, value: secret);
      if (_pairedDevice != null) {
        await _secureStorage.write(
            key: _pairedDeviceKey, value: jsonEncode(_pairedDevice!.toJson()));
      }
    } catch (e) {
      debugPrint('Failed to persist pairing data: $e');
    }
  }

  /// Clear the persisted shared secret and device data from secure storage.
  Future<void> _clearSharedSecret() async {
    try {
      await _secureStorage.delete(key: _sharedSecretKey);
      await _secureStorage.delete(key: _pairedDeviceKey);
    } catch (e) {
      debugPrint('Failed to clear pairing data: $e');
    }
  }

  /// Request camera permission for QR scanning
  Future<bool> requestCameraPermission() async {
    final status = await Permission.camera.request();
    _hasCameraPermission = status.isGranted;
    notifyListeners();
    return _hasCameraPermission;
  }

  /// Parse QR code data and initiate pairing via WebSocket
  ///
  /// [wssPort] is the TLS port the desktop advertised (mDNS TXT `wss_port` or
  /// `discovery/announce` `wss_port`). When absent the shared constant
  /// [kLanWssPort] is used. The QR payload may also carry a `wss_port`, which
  /// takes precedence over the constant.
  Future<bool> pairFromQr(
    String qrData,
    WebSocketService wsService, {
    int? wssPort,
  }) async {
    try {
      _state = PairingState.connecting;
      _error = null;
      notifyListeners();

      final data = jsonDecode(qrData) as Map<String, dynamic>;
      final pairingData = PairingData.fromJson(data);

      if (pairingData.token.isEmpty || pairingData.publicKey.isEmpty) {
        throw Exception('Invalid pairing data');
      }

      // Connect over TLS first: the certificate presented on this handshake
      // is the trust bootstrap, so `isPairing: true` lets the pin be captured.
      final String? ip = data['ip'];

      if (ip == null) {
        throw Exception('No IP address found in QR code. Please update Conduit on your desktop.');
      }

      // A QR payload that carries the peer's own wss_port wins; otherwise
      // use the caller-supplied advertised port; otherwise the shared constant.
      final qrWssPort = _portFrom(data['wss_port']);
      final target = lanUrl(ip, qrWssPort ?? wssPort);
      await wsService.connect(target, isPairing: true);

      if (!wsService.isConnected) {
        final specificError =
            wsService.lastError != null ? '\nDetails: ${wsService.lastError}' : '';
        throw Exception('Failed to connect to desktop at $target$specificError');
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
      await _persistSharedSecret(_sharedSecret!);

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

  /// Parse a port that arrived as JSON (may be `int`, `double` or `String`).
  static int? _portFrom(Object? value) {
    if (value is int) return value;
    if (value is num) return value.toInt();
    if (value is String) return int.tryParse(value.trim());
    return null;
  }

  /// Pair from manual code entry (token only — desktop must be on same network)
  /// Returns true only when the desktop accepts (via pairing/accept handler).
  /// Callers should listen to state changes; this future resolves after the
  /// accept window without faking success.
  ///
  /// [port] is the port from the discovery record (the desktop's *plaintext*
  /// SRV port). It is deliberately **not** used to dial: pairing is TLS-only.
  /// Use [wssPort] — the peer's advertised TLS port — to override the default.
  Future<bool> pairFromCode(
    String code,
    WebSocketService wsService, {
    String? ip,
    int? port,
    int? wssPort,
  }) async {
    try {
      _state = PairingState.connecting;
      _error = null;
      notifyListeners();

      if (ip != null && port != null) {
        // Capture and store the certificate pin for this pairing handshake
        // (the trust bootstrap). Always wss:// on the TLS port.
        final target = lanUrl(ip, wssPort);
        await wsService.connect(target, isPairing: true);
        if (!wsService.isConnected) {
          final specificError =
              wsService.lastError != null ? '\nDetails: ${wsService.lastError}' : '';
          throw Exception(
              'Not connected to desktop at $target$specificError\nDid you select a device from the Nearby list first?');
        }
      }

      if (!wsService.isConnected) {
        final specificError = wsService.lastError != null ? '\nDetails: ${wsService.lastError}' : '';
        throw Exception('Not connected to desktop.$specificError\nDid you select a device from the Nearby list first?');
      }

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
        // No accept arrived before the deadline — report it rather than
        // pretending the pairing succeeded.
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
      await _persistSharedSecret(_sharedSecret!);
      notifyListeners();
    } catch (e) {
      debugPrint('Key exchange failed: $e');
    }
  }

  /// Disconnect from paired device
  Future<void> disconnect() async {
    _pairedDevice = null;
    _state = PairingState.idle;
    _error = null;
    _sharedSecret = null;
    await _clearSharedSecret();
    notifyListeners();
  }

  /// Reset pairing state
  Future<void> reset() async {
    _state = PairingState.idle;
    _error = null;
    _sharedSecret = null;
    await _clearSharedSecret();
    notifyListeners();
  }
}
