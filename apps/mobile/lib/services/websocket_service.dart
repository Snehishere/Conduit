import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/web_socket_channel.dart';
import 'package:web_socket_channel/io.dart';
import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'encryption_service.dart';

typedef MessageHandler = void Function(Map<String, dynamic> message);

/// The desktop's **plaintext** (`ws://`) LAN WebSocket listener.
///
/// Mirrors `conduit_protocol::types::LAN_WS_PORT` in
/// `packages/protocol/src/types.rs`. Nothing is encrypted in transit on this
/// port and no certificate is presented, so it must never be used for a peer
/// the app intends to trust.
///
/// A LAN peer is dialled on [kLanWssPort] only; see [PairingService.lanUrl].
const int kLanWsPort = 9527;

/// The desktop's **TLS** (`wss://`) LAN WebSocket listener.
///
/// Mirrors `conduit_protocol::types::LAN_WSS_PORT` in
/// `packages/protocol/src/types.rs`. A Rust test
/// (`dart_client_mirrors_the_lan_port_constants`) asserts this literal still
/// matches that constant, so the two cannot drift apart silently.
///
/// The certificate-pin trust bootstrap in [_verifyCertificatePin] only ever
/// runs on the `wss://` path, which is why every LAN dial is TLS: a
/// `wss://` handshake against [kLanWsPort] throws, the pin is never captured,
/// and every later connection fails closed — which is exactly the bug that
/// made pairing impossible.
const int kLanWssPort = 9531;

class WebSocketService extends ChangeNotifier {
  WebSocketChannel? _channel;
  bool _isConnected = false;
  String? _lastError;
  String? _deviceName;
  String? _deviceId;
  Timer? _reconnectTimer;
  Timer? _heartbeatTimer;
  int _reconnectAttempts = 0;
  bool _isRelayConnection = false;
  String? _relayUrl;
  String? _targetDeviceId;
  String? _apnsToken;
  String? _osVersion;
  String? _relayToken;
  final Map<String, List<MessageHandler>> _messageHandlers = {};
  final List<Map<String, dynamic>> _connectedDevices = [];

  /// SHA-256 pin over the server certificate's **public key**, in the form
  /// `"sha256/<base64 of sha256(SPKI-DER)>"`.
  ///
  /// Captured during pairing and persisted in secure storage. Hashing the
  /// SubjectPublicKeyInfo rather than the whole certificate means the pin
  /// survives certificate renewal: a re-issued certificate signed by the same
  /// key keeps the same pin, while any change of key breaks it. The desktop
  /// and the relay both publish pins computed this way — the relay's
  /// `GET /pin` reports `"algorithm": "spki-sha256"` — so this client has to
  /// agree with them.
  String? _pinnedCertSha256;

  /// Secure-storage key for the persisted certificate pin.
  static const String _pinStorageKey = 'pinned_cert_sha256';
  static const String _urlStorageKey = 'last_connected_url';
  static const FlutterSecureStorage _secureStorage = FlutterSecureStorage();

  /// Whether the persisted pin has been loaded from secure storage.
  bool _pinLoaded = false;

  /// Whether to prefer WSS (WebSocket Secure) when connecting to LAN desktop.
  bool _preferLanWss = false;

  /// TLS port used when upgrading a `ws://` LAN address to `wss://`.
  ///
  /// Defaults to the shared [kLanWssPort] (9531) and is overwritten by the port
  /// the desktop actually advertises via mDNS / `discovery/announce`.
  int _wssPort = kLanWssPort;

  /// The URI of the live connection, after any LAN `ws://` → `wss://` upgrade.
  ///
  /// This is the real peer address/scheme/port actually dialled, and is what
  /// the UI shows as the hub address. Null when disconnected.
  Uri? _connectedUri;

  bool get isConnected => _isConnected;
  String? get lastError => _lastError;
  String? get deviceName => _deviceName;
  String? get deviceId => _deviceId;
  List<Map<String, dynamic>> get connectedDevices => List.unmodifiable(_connectedDevices);

  /// The address this app is actually connected to, e.g. `wss://192.168.1.5:9531`.
  ///
  /// Null while disconnected. Never synthesised — callers must handle the
  /// disconnected case rather than being shown a plausible-looking default.
  String? get connectedAddress => _connectedUri?.toString();

  String? Function()? getSharedSecret;

  WebSocketService();

  void setDeviceId(String id) {
    _deviceId = id;
  }

  void setDeviceInfo(String name, String osVersion) {
    _deviceName = name;
    _osVersion = osVersion;
  }

  void setApnsToken(String token) {
    _apnsToken = token;
  }

  void setRelayConfig(String? url, String? targetId) {
    _relayUrl = url;
    _targetDeviceId = targetId;
  }

  void setRelayToken(String? token) {
    _relayToken = token;
  }

  /// Set the expected SPKI pin for certificate pinning.
  ///
  /// [sha256Fingerprint] is `"sha256/<base64>"` over the peer's
  /// SubjectPublicKeyInfo, matching what `scripts/generate-cert-pin.sh` and
  /// the relay's `GET /pin` produce. Persisted in secure storage so validation
  /// survives app restarts.
  void setPinnedCertificate(String sha256Fingerprint) {
    _pinnedCertSha256 = sha256Fingerprint;
    _pinLoaded = true;
    unawaited(_secureStorage
        .write(key: _pinStorageKey, value: sha256Fingerprint)
        .catchError((Object e) {
      debugPrint('WS: Failed to persist certificate pin: $e');
    }));
  }

  /// Load the certificate pin captured during a previous pairing.
  Future<void> _ensurePinLoaded() async {
    if (_pinLoaded) return;
    try {
      final stored = await _secureStorage.read(key: _pinStorageKey);
      if (stored != null && stored.isNotEmpty) {
        _pinnedCertSha256 = stored;
      }
    } catch (e) {
      debugPrint('WS: Failed to load certificate pin: $e');
    }
    _pinLoaded = true;
  }

  /// Enable or disable prefer WSS for LAN connections.
  ///
  /// [wssPort] must be the peer's *TLS* port (as advertised in its mDNS TXT
  /// record or `discovery/announce` `wss_port` field), never the plaintext
  /// WS port. Defaults to the shared [kLanWssPort].
  void setPreferLanWss(bool prefer, {int wssPort = kLanWssPort}) {
    _preferLanWss = prefer;
    _wssPort = wssPort;
  }

  /// Record the TLS port advertised by a discovered peer, so a `ws://` LAN
  /// address is upgraded to `wss://` on the port the peer actually serves TLS
  /// on rather than a hardcoded guess.
  void setAdvertisedWssPort(int port) {
    _wssPort = port;
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

  /// Verify the server certificate against the pinned SPKI hash.
  ///
  /// - During pairing (`isPairing: true`) the certificate presented on the
  ///   handshake is the trust bootstrap — the user scanned the QR code or typed
  ///   the code, which is the out-of-band consent — so its pin is captured and
  ///   stored here. Any later connection is validated against that pin.
  /// - On every non-pairing connection the pin MUST be present and MUST match.
  ///   When no pin is configured the connection is **rejected**; there is no
  ///   path that accepts a certificate blindly.
  bool _verifyCertificatePin(X509Certificate cert, {required bool isPairing}) {
    String actualSha256;
    try {
      actualSha256 = _computeSha256Sync(cert.der).replaceAll('=', '').trim();
    } catch (e) {
      debugPrint('Certificate pin verification error: $e');
      return false;
    }

    if (isPairing) {
      // Pairing is the trust bootstrap (scanned QR / entered code is the
      // out-of-band consent). Store the pin of the certificate we are pairing
      // with so every subsequent connection can be validated.
      final storedPin = 'sha256/$actualSha256';
      if (_pinnedCertSha256 == null || _pinnedCertSha256!.isEmpty) {
        debugPrint('WS: Stored certificate pin during pairing: $storedPin');
      } else if (_normalizePin(_pinnedCertSha256!) != actualSha256) {
        debugPrint(
          'WS: Desktop public key changed since last pairing — re-pinning',
        );
      }
      setPinnedCertificate(storedPin);
      return true;
    }

    if (_pinnedCertSha256 == null || _pinnedCertSha256!.isEmpty) {
      debugPrint('Certificate pin verification failed: no pin configured — rejecting connection');
      return false; // Fail closed — never accept without a pin
    }

    final expectedSha256 = _normalizePin(_pinnedCertSha256!);
    if (actualSha256 != expectedSha256) {
      debugPrint('Certificate pin mismatch: expected $expectedSha256, got $actualSha256');
      return false;
    }
    return true;
  }

  /// Normalize a pin for comparison: strip the `sha256/` prefix, whitespace and
  /// base64 padding.
  String _normalizePin(String pin) {
    return pin
        .replaceAll('sha256/', '')
        .replaceAll(' ', '')
        .replaceAll('=', '')
        .trim();
  }

  /// Compute SHA-256 hash of bytes, returning base64-encoded result.
  String _computeSha256Sync(List<int> bytes) {
    return base64.encode(crypto.sha256.convert(bytes).bytes);
  }

  /// Try connecting to the last known URL (called on startup)
  Future<void> autoConnect() async {
    try {
      final lastUrl = await _secureStorage.read(key: _urlStorageKey);
      if (lastUrl != null && lastUrl.isNotEmpty) {
        debugPrint('WS: Auto-connecting to saved URL: $lastUrl');
        try {
          await connect(lastUrl).timeout(const Duration(seconds: 3));
        } catch (e) {
          debugPrint('WS: LAN connect failed or timed out: $e');
          if (_relayUrl != null) {
            debugPrint('WS: Auto-connecting to relay fallback');
            await connect('');
          }
        }
      } else if (_relayUrl != null) {
        debugPrint('WS: Auto-connecting to relay fallback');
        await connect('');
      }
    } catch (e) {
      debugPrint('WS: Auto-connect failed: $e');
    }
  }

  /// Connect to a device or relay.
  ///
  /// [isPairing] must only be true for the explicit pairing handshake —
  /// it is the sole code path allowed to capture (store) a certificate
  /// fingerprint when no pin exists yet.
  Future<void> connect(String url, {bool isPairing = false}) async {
    try {
      // Load the pin captured during a previous pairing before verifying.
      await _ensurePinLoaded();

      bool connectingToRelay = false;
      Uri uri;
      if (url.isEmpty && _relayUrl != null) {
        uri = Uri.parse(_relayUrl!);
        connectingToRelay = true;
      } else {
        uri = Uri.parse(url);
      }

      // For LAN connections, prefer WSS if configured
      if (!connectingToRelay && _preferLanWss && uri.scheme == 'ws') {
        final host = uri.host;
        if (_isLanAddress(host)) {
          debugPrint('WS: Preferring WSS for LAN address $host');
          uri = uri.replace(scheme: 'wss', port: _wssPort);
        }
      }

      // For WSS connections, use secure WebSocket with certificate pinning.
      if (uri.scheme == 'wss') {
        // A fresh SecurityContext with NO trusted roots: every certificate
        // (LAN self-signed or relay CA-signed) fails platform validation and
        // is therefore always routed through badCertificateCallback below,
        // where it must pass pin verification. No path skips the pin check.
        final client = HttpClient(context: SecurityContext())
          ..badCertificateCallback =
              (X509Certificate cert, String host, int port) {
            // Never accept blindly. The pin was captured and stored during
            // pairing; every other connection must match it. If no pin is
            // configured (and this is not the pairing handshake itself) the
            // certificate is rejected.
            return _verifyCertificatePin(cert, isPairing: isPairing);
          };

        _channel = IOWebSocketChannel.connect(
          uri,
          pingInterval: const Duration(seconds: 15),
          customClient: client,
        );
      } else {
        _channel = IOWebSocketChannel.connect(
          uri,
          pingInterval: const Duration(seconds: 15),
        );
      }

      await _channel!.ready;

      _isConnected = true;
      _isRelayConnection = connectingToRelay;
      _connectedUri = uri;
      _lastError = null;
      _reconnectAttempts = 0;
      _startHeartbeat();
      notifyListeners();

      if (url.isNotEmpty && !connectingToRelay) {
        // Persist the *resolved* URI (post scheme/port upgrade), not the raw
        // input, so a later cold start reconnects to the same real endpoint
        // even if the prefer-LAN-WSS setting is not yet applied.
        unawaited(
          _secureStorage.write(key: _urlStorageKey, value: uri.toString()),
        );
      }

      if (_isRelayConnection) {
        await _sendRelayAuth();
      }

      _channel!.stream.listen(
        (data) async {
          try {
            if (data is Uint8List || data is List<int>) {
              final bytes = data is Uint8List ? data : Uint8List.fromList(data);
              if (bytes.length < 28) {
                debugPrint('WS: Binary message too short');
                return;
              }
              final nonce = bytes.sublist(0, 24);
              final byteData = ByteData.sublistView(bytes, 24, 28);
              final jsonLen = byteData.getUint32(0, Endian.little);
              if (bytes.length < 28 + jsonLen) {
                debugPrint('WS: Binary message metadata length mismatch');
                return;
              }
              final metadataBytes = bytes.sublist(28, 28 + jsonLen);
              final ciphertext = bytes.sublist(28 + jsonLen);
              
              final metadata = jsonDecode(utf8.decode(metadataBytes)) as Map<String, dynamic>;
              
              final sharedSecretHex = getSharedSecret?.call();
              if (sharedSecretHex != null) {
                final secretBytes = EncryptionService.hexToBytes(sharedSecretHex);
                try {
                  final decrypted = await EncryptionService().decryptBinary(secretBytes, nonce, ciphertext);
                  final msg = {
                    'type': 'file',
                    'action': 'chunk_binary',
                    'id': metadata['id'],
                    'index': metadata['index'],
                    'data': decrypted,
                  };
                  _handleMessage(msg);
                } catch (e) {
                  debugPrint('WS: Failed to decrypt binary chunk: $e');
                }
              }
              return;
            }

            final rawMessage = jsonDecode(data as String) as Map<String, dynamic>;
            final type = rawMessage['type'] as String?;
            
            if (type == 'ping') {
              _sendMessage({'type': 'pong'});
              return;
            }

            if (type == 'encrypted') {
              final sharedSecretHex = getSharedSecret?.call();
              if (sharedSecretHex != null) {
                final secretBytes = EncryptionService.hexToBytes(sharedSecretHex);
                final nonceBytes = EncryptionService.hexToBytes(rawMessage['nonce'] as String);
                final hmacHex = rawMessage['hmac'] as String;
                final dataHex = rawMessage['data'] as String;

                if (!EncryptionService.verifyHmac(secretBytes, dataHex, hmacHex)) {
                  debugPrint('WS: HMAC verification failed');
                  return;
                }

                final dataBytes = EncryptionService.hexToBytes(dataHex);
                final decrypted = await EncryptionService().decrypt(secretBytes, nonceBytes, dataBytes);
                final message = jsonDecode(decrypted) as Map<String, dynamic>;
                _handleMessage(message);
              } else {
                debugPrint('WS: Received encrypted message but no shared secret available');
              }
            } else {
              _handleMessage(rawMessage);
            }
          } catch (e) {
            debugPrint('WS: failed to decode message: $e');
          }
        },
        onDone: () {
          _isConnected = false;
          _connectedUri = null;
          _connectedDevices.clear();
          notifyListeners();
          _scheduleReconnect(url);
        },
        onError: (e) {
          debugPrint('WS stream error: $e');
          _lastError = 'Connection lost: $e';
          _isConnected = false;
          _connectedUri = null;
          _connectedDevices.clear();
          notifyListeners();
          _scheduleReconnect(url);
        },
      );

      // Announce presence with canonical fields (P2.7)
      _sendMessage({
        'type': 'discovery',
        'action': 'announce',
        'protocol_version': 1,
        'device_name': _deviceName ?? 'Mobile Device',
        'device_type': 'phone',
        'device_id': _deviceId ?? 'mobile_${DateTime.now().millisecondsSinceEpoch}',
        'os': Platform.operatingSystem,
        'version': _osVersion ?? '1.0.0',
        'apns_token': _apnsToken,
      });
    } catch (e) {
      debugPrint('WebSocket error: $e');
      _lastError = 'Connection failed: $e';
      _isConnected = false;
      _connectedUri = null;
      // If LAN connection fails and we have a relay URL, try relay immediately
      if (!_isRelayConnection && _relayUrl != null) {
        debugPrint('WS LAN failed, trying Relay...');
        _scheduleReconnect(''); // empty URL triggers relay logic
      } else {
        _scheduleReconnect(url);
      }
    }
  }

  /// Send the relay authentication message.
  /// The token is sent in plaintext over the WSS (TLS) connection
  /// because it is used by the relay itself to authorize the connection.
  Future<void> _sendRelayAuth() async {
    final token = _relayToken;
    final authPayload = <String, dynamic>{
      'type': 'relay_auth',
      'device_id': _deviceId,
      'apns_token': _apnsToken,
    };

    if (token != null && token.isNotEmpty) {
      authPayload['relay_token'] = token;
    }

    _channel?.sink.add(jsonEncode(authPayload));
  }

  void _startHeartbeat() {
    _heartbeatTimer?.cancel();
    _heartbeatTimer = Timer.periodic(const Duration(seconds: 25), (_) {
      if (_isConnected && _channel != null) {
        _sendMessage({'type': 'ping'});
      }
    });
  }

  void _scheduleReconnect(String url) {
    _heartbeatTimer?.cancel();
    _heartbeatTimer = null;
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
    } else if (type == 'pairing' && action == 'revoke') {
      // P1.7: Server revoked this device's pairing — notify the UI
      debugPrint('WS: Device pairing revoked by server');
      _isConnected = false;
      _connectedUri = null;
      _connectedDevices.clear();
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
      // Close the connection so the UI can react to revocation
      _channel?.sink.close();
      _channel = null;
      notifyListeners();
      // Dispatch revoke event to registered handlers
      final handlers = _messageHandlers['pairing'];
      if (handlers != null) {
        for (final h in List<MessageHandler>.from(handlers)) {
          try {
            h(message);
          } catch (e) {
            debugPrint('WS revoke handler error: $e');
          }
        }
      }
      return; // Don't re-dispatch below
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

  void sendBinaryMessage(dynamic message) {
    if (_channel != null && _isConnected) {
      if (_isRelayConnection && _targetDeviceId != null && (message is Map ? message['type'] != 'relay_auth' : true)) {
        if (message is Uint8List) {
          final targetBytes = utf8.encode(_targetDeviceId!.padRight(16).substring(0, 16));
          final routedBytes = Uint8List(1 + 16 + message.length);
          routedBytes[0] = 0x01;
          routedBytes.setRange(1, 17, targetBytes);
          routedBytes.setRange(17, routedBytes.length, message);
          _channel!.sink.add(routedBytes);
        } else {
          final timestamp = DateTime.now().millisecondsSinceEpoch;
          final nonceBytes = crypto.sha256.convert(utf8.encode(DateTime.now().microsecondsSinceEpoch.toString())).bytes;
          final nonce = EncryptionService.bytesToHex(nonceBytes).substring(0, 32);
          final messageForHmac = {
            'type': 'relay_route',
            'to_device_id': _targetDeviceId,
            'payload': message,
            'timestamp': timestamp,
            'nonce': nonce,
          };
          final hmacHex = EncryptionService.generateHmac(utf8.encode(_relayToken ?? ''), jsonEncode(messageForHmac));
          
          _channel!.sink.add(jsonEncode({
            ...messageForHmac,
            'hmac': hmacHex,
          }));
        }
      } else {
        if (message is Uint8List) {
          _channel!.sink.add(message);
        } else {
          _channel!.sink.add(jsonEncode(message));
        }
      }
    }
  }

  void sendMessage(Map<String, dynamic> message) {
    _sendMessage(message);
  }

  /// Log an error and surface it to the UI via [lastError].
  void _surfaceSendError(String message) {
    debugPrint('WS: $message');
    _lastError = message;
    notifyListeners();
  }

  void _sendMessage(Map<String, dynamic> message) async {
    if (_channel != null && _isConnected) {
      final type = message['type'] as String?;
      final isPairing = type == 'pairing';
      final isRelayAuth = type == 'relay_auth';
      final sharedSecretHex = getSharedSecret?.call();

      Map<String, dynamic> payloadToSend = message;

      if (!isPairing && !isRelayAuth && sharedSecretHex != null) {
        try {
          final secretBytes = EncryptionService.hexToBytes(sharedSecretHex);
          final plaintext = jsonEncode(message);
          final (nonceBytes, combinedBytes) = await EncryptionService().encrypt(secretBytes, plaintext);
          
          final dataHex = EncryptionService.bytesToHex(combinedBytes);
          final hmacHex = EncryptionService.generateHmac(secretBytes, dataHex);
          
          payloadToSend = {
            'type': 'encrypted',
            'source_device': _deviceId ?? 'mobile',
            'nonce': EncryptionService.bytesToHex(nonceBytes),
            'hmac': hmacHex,
            'data': dataHex,
          };
        } catch (e) {
          // Security: NEVER fall back to sending the plaintext message.
          // Drop it, log the failure and surface it to the UI.
          _surfaceSendError('Encryption failed — message not sent: $e');
          return;
        }
      }

      if (_isRelayConnection && _targetDeviceId != null && !isRelayAuth) {
        final timestamp = DateTime.now().millisecondsSinceEpoch;
        final nonceBytes = crypto.sha256.convert(utf8.encode(DateTime.now().microsecondsSinceEpoch.toString())).bytes;
        final nonce = EncryptionService.bytesToHex(nonceBytes).substring(0, 32);
        final messageForHmac = {
          'type': 'relay_route',
          'to_device_id': _targetDeviceId,
          'payload': payloadToSend,
          'timestamp': timestamp,
          'nonce': nonce,
        };
        final hmacHex = EncryptionService.generateHmac(utf8.encode(_relayToken ?? ''), jsonEncode(messageForHmac));
        
        _channel!.sink.add(jsonEncode({
          ...messageForHmac,
          'hmac': hmacHex,
        }));
      } else {
        _channel!.sink.add(jsonEncode(payloadToSend));
      }
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

  void sendNotificationMarkRead(String id) {
    _sendMessage({
      'type': 'notification',
      'action': 'mark_read',
      'id': id,
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
      'call_id': ?callId,
      'to_device_id': ?toDeviceId,
      'route': ?route,
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

  /// Check if an address is a LAN address (private IP range).
  bool _isLanAddress(String host) {
    // Localhost
    if (host == 'localhost' || host == '127.0.0.1' || host == '::1') {
      return false; // Already local, no need for WSS
    }

    // Parse IP and check private ranges
    try {
      final parts = host.split('.');
      if (parts.length == 4) {
        final first = int.parse(parts[0]);
        final second = int.parse(parts[1]);
        // 10.x.x.x
        if (first == 10) return true;
        // 172.16.x.x - 172.31.x.x
        if (first == 172 && second >= 16 && second <= 31) return true;
        // 192.168.x.x
        if (first == 192 && second == 168) return true;
      }
    } catch (_) {}

    return false;
  }

  @override
  void dispose() {
    _heartbeatTimer?.cancel();
    _reconnectTimer?.cancel();
    _channel?.sink.close();
    super.dispose();
  }
}
