import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/web_socket_channel.dart';
import 'package:web_socket_channel/io.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'encryption_service.dart';
import 'relay_route.dart'
    show
        BinaryFrame,
        buildBinaryFrame,
        computeSpkiPin,
        deriveRouteKey,
        parseBinaryFrame,
        signRoute,
        verifyBinaryFrame;

/// Shorthand for the hex decode used all over the relay paths.
List<int> hexToBytes(String hex) => EncryptionService.hexToBytes(hex);

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

/// Nonce width in the hub's binary chunk envelope. Matches `CHUNK_NONCE_LEN`
/// in `apps/desktop/src-tauri/src/server/handlers/files.rs` — XChaCha20-Poly1305
/// uses a 24-byte nonce, and the two cannot disagree.
const int chunkNonceLen = 24;

/// Width of the little-endian metadata-length field in the same envelope.
const int chunkLenFieldLen = 4;

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

  /// The v2 binary frame counter. Strictly increasing per connection, because
  /// the relay drops a frame whose sequence does not advance — that is its
  /// replay defence for file chunks, which carry no nonce to dedupe on.
  int _binarySequence = 0;

  /// Highest inbound binary sequence accepted from the current peer, for the
  /// same reason: a relayed chunk replayed at the client must not reappear.
  int? _lastInboundSequence;

  /// The desktop's own device id, learned at pairing.
  ///
  /// Needed to verify anything the relay forwards: a v2 frame is checked under
  /// the *sender's* route key, and the sender's id is bound into the key's
  /// derivation, so a phone that does not know the hub's id has no way to check
  /// a relayed frame and must drop it.
  String? _relayPeerDeviceId;
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

  /// Secure-storage key for the id the desktop assigned us at pairing time.
  ///
  /// This is not a local choice: the desktop picks the `devices.id` and we are
  /// told it in `pairing/accept`. It has to survive a restart because it is
  /// the value stamped into `source_device` on every `encrypted` envelope, and
  /// the desktop resolves the shared secret by that field. Sending a guess
  /// means the hub finds no secret and drops the frame.
  static const String _deviceIdStorageKey = 'paired_device_id';

  /// The desktop's own device id, learned in the same `pairing/accept` that
  /// assigns ours. Cleared on revoke alongside [_deviceIdStorageKey].
  static const String _hubDeviceIdStorageKey = 'hub_device_id';
  static const FlutterSecureStorage _secureStorage = FlutterSecureStorage();

  /// Whether the persisted device id has been loaded from secure storage.
  bool _deviceIdLoaded = false;

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
  List<Map<String, dynamic>> get connectedDevices =>
      List.unmodifiable(_connectedDevices);

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

  /// Forget the assigned device id, in memory and in secure storage.
  ///
  /// Call when the pairing is deliberately ended (revoke, or an explicit
  /// unpair). Leaving a stale id behind would have the next session name the
  /// phone as a device the hub no longer knows.
  void clearDeviceId() {
    if (_deviceId == null) return;
    _deviceId = null;
    unawaited(_secureStorage.delete(key: _deviceIdStorageKey));
    // The desktop's id goes with it: it is part of the same pairing, and a
    // re-pair may land on a different hub with a different id.
    _relayPeerDeviceId = null;
    _lastInboundSequence = null;
    _binarySequence = 0;
    unawaited(_secureStorage.delete(key: _hubDeviceIdStorageKey));
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
    unawaited(
      _secureStorage
          .write(key: _pinStorageKey, value: sha256Fingerprint)
          .catchError((Object e) {
            debugPrint('WS: Failed to persist certificate pin: $e');
          }),
    );
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

  /// Load the device id the desktop assigned us at pairing time.
  ///
  /// Must complete before the first frame is sent: until it does, [deviceId] is
  /// null and every `encrypted` envelope carries the placeholder `"mobile"`,
  /// which the hub cannot resolve to a shared secret. Call once during startup,
  /// before [autoConnect].
  Future<void> restoreDeviceId() async {
    if (_deviceIdLoaded) return;
    try {
      final stored = await _secureStorage.read(key: _deviceIdStorageKey);
      if (stored != null && stored.isNotEmpty) {
        _deviceId = stored;
        debugPrint('WS: Restored paired device id');
      }
    } catch (e) {
      debugPrint('WS: Failed to load paired device id: $e');
    }
    try {
      final hubId = await _secureStorage.read(key: _hubDeviceIdStorageKey);
      if (hubId != null && hubId.isNotEmpty) {
        _relayPeerDeviceId = hubId;
        debugPrint('WS: Restored desktop device id');
      }
    } catch (e) {
      debugPrint('WS: Failed to load desktop device id: $e');
    }
    _deviceIdLoaded = true;
  }

  /// Record the id the desktop assigned us, and persist it for next launch.
  ///
  /// Called from the `pairing/accept` handler. A no-op when the field is
  /// absent, which is how a desktop predating assigned ids is handled: the
  /// hub falls back to the connection identity in that case.
  void _adoptAssignedDeviceId(String? id) {
    if (id == null || id.isEmpty || id == _deviceId) return;
    _deviceId = id;
    debugPrint('WS: Desktop assigned device id');
    unawaited(_secureStorage.write(key: _deviceIdStorageKey, value: id));
  }

  /// Record the desktop's own device id, and persist it for next launch.
  ///
  /// Sent alongside the assigned id in the same `pairing/accept`, and needed
  /// for the same reason from the other direction: verifying a relayed frame
  /// means deriving the *sender's* route key, which is bound to the sender's id.
  void _adoptHubDeviceId(String? id) {
    if (id == null || id.isEmpty || id == _relayPeerDeviceId) return;
    _relayPeerDeviceId = id;
    debugPrint('WS: Learned the desktop device id');
    unawaited(_secureStorage.write(key: _hubDeviceIdStorageKey, value: id));
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
      actualSha256 = computeSpkiPin(cert.der);
    } on FormatException catch (e) {
      debugPrint('Certificate pin verification error: ${e.message}');
      return false;
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
      debugPrint(
        'Certificate pin verification failed: no pin configured — rejecting connection',
      );
      return false; // Fail closed — never accept without a pin
    }

    final expectedSha256 = _normalizePin(_pinnedCertSha256!);
    if (actualSha256 != expectedSha256) {
      debugPrint(
        'Certificate pin mismatch: expected $expectedSha256, got $actualSha256',
      );
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
              // The two binary formats are unrelated and must not be confused:
              // on a relay connection it is a v2 frame (version byte, target
              // id, sequence, tag, payload) addressed to us, while on a LAN
              // connection it is the hub's own chunk envelope
              // (24-byte nonce, 4-byte length, JSON metadata, ciphertext).
              // Decoding a v2 frame as the LAN form would read the target id
              // as a nonce and silently drop every relayed file.
              if (_isRelayConnection) {
                await _handleRelayBinary(bytes);
              } else {
                await _handleLanBinary(bytes);
              }
              return;
            }

            final rawMessage =
                jsonDecode(data as String) as Map<String, dynamic>;
            final type = rawMessage['type'] as String?;

            if (type == 'ping') {
              _sendMessage({'type': 'pong'});
              return;
            }

            if (type == 'relay_delivery') {
              // The relay verified the route signature and forwards the
              // attribution with it. Unwrap before anything else, so the rest
              // of the app sees the message exactly as a LAN one.
              final sender = rawMessage['from_device_id'];
              final inner = rawMessage['payload'];
              if (sender is! String || inner is! Map<String, dynamic>) {
                debugPrint('WS: malformed relay_delivery envelope');
                return;
              }
              _handleAttributed(inner, sender).then((_) {});
              return;
            }

            if (type == 'encrypted') {
              _handleEncrypted(rawMessage);
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
        'device_id':
            _deviceId ?? 'mobile_${DateTime.now().millisecondsSinceEpoch}',
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

  /// Verify and handle a v2 binary frame the relay forwarded to us.
  ///
  /// Fails closed on every check: a frame addressed to someone else, one whose
  /// tag does not verify under the sending device's route key, and one whose
  /// sequence does not advance are all dropped rather than surfaced.
  Future<void> _handleRelayBinary(Uint8List bytes) async {
    final BinaryFrame frame;
    try {
      frame = parseBinaryFrame(bytes);
    } on FormatException catch (e) {
      debugPrint('WS: malformed relay binary frame: ${e.message}');
      return;
    }

    // The relay only forwards to the named recipient, so a frame naming anyone
    // else means the routing table and the wire disagree.
    if (frame.targetId != _deviceId) {
      debugPrint('WS: relay frame addressed to ${frame.targetId}, not us');
      return;
    }

    // The tag is what proves the sender. The route key comes from the pairing
    // secret, which this device shares with the hub — so this check proves the
    // frame came through the relay from the hub itself, and not from whoever
    // else can reach that relay.
    final secretHex = getSharedSecret?.call();
    final sender = _relayPeerDeviceId;
    if (secretHex == null || sender == null) {
      debugPrint(
        'WS: cannot verify a relayed frame — '
        '${secretHex == null ? 'no shared secret' : 'the desktop device id was never learned'}',
      );
      return;
    }
    if (!verifyBinaryFrame(
      routeKey: deriveRouteKey(hexToBytes(secretHex), sender),
      fromDeviceId: sender,
      frame: bytes,
    )) {
      debugPrint('WS: relay frame tag did not verify; dropping');
      return;
    }

    final last = _lastInboundSequence;
    if (last != null && frame.sequence <= last) {
      debugPrint(
        'WS: replayed relay frame (sequence ${frame.sequence} <= $last); dropping',
      );
      return;
    }
    _lastInboundSequence = frame.sequence;

    await _deliverChunkPayload(frame.payload);
  }

  /// Verify, decrypt and dispatch an `encrypted` envelope.
  ///
  /// Fails closed: no shared secret, a bad HMAC, or malformed base64 all mean
  /// the message is dropped. It is never surfaced undecrypted, because the
  /// alternative is a peer receiving a peer handshake's contents in clear.
  Future<void> _handleEncrypted(Map<String, dynamic> envelope) async {
    final sharedSecretHex = getSharedSecret?.call();
    if (sharedSecretHex == null) {
      debugPrint(
        'WS: Received encrypted message but no shared secret available',
      );
      return;
    }
    try {
      final secretBytes = hexToBytes(sharedSecretHex);
      final nonceBytes = hexToBytes(envelope['nonce'] as String);
      final hmacHex = envelope['hmac'] as String;
      final dataHex = envelope['data'] as String;

      if (!EncryptionService.verifyHmac(secretBytes, dataHex, hmacHex)) {
        debugPrint('WS: HMAC verification failed');
        return;
      }

      final decrypted = await EncryptionService().decrypt(
        secretBytes,
        nonceBytes,
        hexToBytes(dataHex),
      );
      _handleMessage(jsonDecode(decrypted) as Map<String, dynamic>);
    } catch (e) {
      debugPrint('WS: could not decrypt an encrypted message: $e');
    }
  }

  /// Handle a message the relay forwarded, stamped with the sender it verified.
  ///
  /// The relay only produces this envelope after checking the route signature
  /// against the connection's authenticated device, so `sender` is the
  /// relay's assertion rather than something the payload claims about itself.
  /// It is recorded on the message so a handler that cares about who is talking
  /// does not have to trust a self-reported `source_device`.
  Future<void> _handleAttributed(
    Map<String, dynamic> message,
    String sender,
  ) async {
    _lastRelaySender = sender;
    if (message['type'] == 'encrypted') {
      // Reuse the ordinary decrypt-and-dispatch path; the envelope is the same
      // one the hub sends on the LAN, and its own HMAC still has to check out.
      _handleEncrypted(message);
      return;
    }
    _handleMessage(message);
  }

  /// The most recent sender the relay has vouched for, or null on a LAN
  /// connection where the connection itself is the attribution.
  String? _lastRelaySender;

  /// The device id the relay last attributed an inbound message to.
  ///
  /// Set only on a relay connection, and only from the relay's own verified
  /// attribution rather than anything the payload says about itself. Null means
  /// "the connection is the attribution" — a phone is paired with one hub, so on
  /// the LAN there is nothing to add.
  String? get relayPeerDeviceId => _lastRelaySender ?? _relayPeerDeviceId;

  /// Decrypt a relayed file chunk and dispatch it.
  ///
  /// The payload inside a v2 frame is the same chunk envelope the LAN path
  /// uses — a 24-byte nonce, a 4-byte little-endian metadata length, the JSON
  /// metadata, then the ciphertext — so it is parsed by the same code rather
  /// than a second, differently-shaped decoder.
  Future<void> _deliverChunkPayload(Uint8List payload) async {
    final sharedSecretHex = getSharedSecret?.call();
    if (sharedSecretHex == null) {
      debugPrint('WS: received a relayed chunk with no shared secret');
      return;
    }
    try {
      final nonce = payload.sublist(0, chunkNonceLen);
      final jsonLen = ByteData.sublistView(
        payload,
        chunkNonceLen,
        chunkNonceLen + chunkLenFieldLen,
      ).getUint32(0, Endian.little);
      if (payload.length < chunkNonceLen + chunkLenFieldLen + jsonLen) {
        debugPrint('WS: relayed chunk metadata length mismatch');
        return;
      }
      final metadataBytes = payload.sublist(
        chunkNonceLen + chunkLenFieldLen,
        chunkNonceLen + chunkLenFieldLen + jsonLen,
      );
      final ciphertext = payload.sublist(
        chunkNonceLen + chunkLenFieldLen + jsonLen,
      );
      final metadata =
          jsonDecode(utf8.decode(metadataBytes)) as Map<String, dynamic>;
      final decrypted = await EncryptionService().decryptBinary(
        hexToBytes(sharedSecretHex),
        nonce,
        ciphertext,
      );
      _handleMessage({
        'type': 'file',
        'action': 'chunk_binary',
        'id': metadata['id'],
        'index': metadata['index'],
        'data': decrypted,
      });
    } catch (e) {
      debugPrint('WS: failed to decrypt relayed binary chunk: $e');
    }
  }

  /// Verify and handle the hub's own LAN chunk envelope.
  ///
  /// Unchanged in shape from the relay path: a 24-byte nonce, a 4-byte
  /// little-endian metadata length, the JSON metadata, then the ciphertext.
  Future<void> _handleLanBinary(Uint8List bytes) async {
    if (bytes.length < chunkNonceLen + chunkLenFieldLen) {
      debugPrint('WS: Binary message too short');
      return;
    }
    final nonce = bytes.sublist(0, chunkNonceLen);
    final byteData = ByteData.sublistView(
      bytes,
      chunkNonceLen,
      chunkNonceLen + chunkLenFieldLen,
    );
    final jsonLen = byteData.getUint32(0, Endian.little);
    if (bytes.length < chunkNonceLen + chunkLenFieldLen + jsonLen) {
      debugPrint('WS: Binary message metadata length mismatch');
      return;
    }
    final metadataBytes = bytes.sublist(
      chunkNonceLen + chunkLenFieldLen,
      chunkNonceLen + chunkLenFieldLen + jsonLen,
    );
    final ciphertext = bytes.sublist(
      chunkNonceLen + chunkLenFieldLen + jsonLen,
    );

    final metadata =
        jsonDecode(utf8.decode(metadataBytes)) as Map<String, dynamic>;

    final sharedSecretHex = getSharedSecret?.call();
    if (sharedSecretHex == null) {
      debugPrint('WS: Received binary chunk but no shared secret available');
      return;
    }
    try {
      final decrypted = await EncryptionService().decryptBinary(
        hexToBytes(sharedSecretHex),
        nonce,
        ciphertext,
      );
      _handleMessage({
        'type': 'file',
        'action': 'chunk_binary',
        'id': metadata['id'],
        'index': metadata['index'],
        'data': decrypted,
      });
    } catch (e) {
      debugPrint('WS: Failed to decrypt binary chunk: $e');
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
    final delaySeconds = (3 * (1 << (_reconnectAttempts - 1).clamp(0, 4)))
        .clamp(3, 60);
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
      // The desktop tells us the id it filed us under. This is the only time we
      // can learn it, and without it every envelope we send names us
      // "mobile" and is dropped by the hub's decrypt step.
      _adoptAssignedDeviceId(message['device_id'] as String?);
      // It also tells us its own id in the same breath, and that is equally
      // load-bearing in the other direction: a relayed frame is verified under
      // the *sender's* route key, and the sender's id is bound into the key's
      // derivation. Without it every relayed file chunk would be unverifiable.
      _adoptHubDeviceId(message['hub_device_id'] as String?);
      final deviceInfo = message['device_info'] as Map<String, dynamic>?;
      if (deviceInfo != null) {
        final device = {
          'device_id': message['device_id'] as String? ?? 'unknown',
          'device_name': deviceInfo['name'] as String? ?? 'Unknown',
          'device_type': deviceInfo['type'] as String? ?? 'desktop',
        };
        _connectedDevices.removeWhere(
          (d) => d['device_id'] == device['device_id'],
        );
        _connectedDevices.add(device);
        notifyListeners();
      }
    } else if (type == 'pairing' && action == 'revoke') {
      // P1.7: Server revoked this device's pairing — notify the UI
      debugPrint('WS: Device pairing revoked by server');
      clearDeviceId();
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
      _connectedDevices.removeWhere(
        (d) => d['device_id'] == device['device_id'],
      );
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

  /// The key this device signs its relayed routes with, or null if it cannot
  /// sign yet.
  ///
  /// Derived from the pairing secret under this device's own id, so the relay
  /// can verify it without a shared operator key and another device's key is
  /// useless here. Null until pairing has produced both a secret and an id,
  /// which is the correct state to be in: an unroutable device, not a device
  /// that signs with something weaker.
  List<int>? _routeKey() {
    final secretHex = getSharedSecret?.call();
    final deviceId = _deviceId;
    if (secretHex == null || secretHex.isEmpty) return null;
    if (deviceId == null || deviceId.isEmpty) return null;
    return deriveRouteKey(hexToBytes(secretHex), deviceId);
  }

  /// Wrap [message] in a signed `relay_route` addressed to [_targetDeviceId].
  ///
  /// Returns null when this device cannot sign, which the caller must treat as
  /// "do not send" rather than "send it unsigned": the relay rejects an
  /// unsigned route anyway, and quietly sending one hides the real fault.
  Map<String, dynamic>? _asSignedRoute(Object? message) {
    final target = _targetDeviceId;
    final deviceId = _deviceId;
    final key = _routeKey();
    if (target == null || deviceId == null || key == null) {
      debugPrint(
        'WS: cannot route through the relay — '
        'missing ${target == null
            ? 'target'
            : deviceId == null
            ? 'own device id'
            : 'pairing secret'}',
      );
      return null;
    }
    return signRoute(
      routeKey: key,
      fromDeviceId: deviceId,
      toDeviceId: target,
      payload: message,
    );
  }

  /// Next value for the v2 binary frame sequence counter.
  int _nextSequence() => _binarySequence = (_binarySequence + 1) & 0xFFFFFFFF;

  void sendBinaryMessage(dynamic message) {
    if (_channel == null || !_isConnected) return;

    if (_isRelayConnection && _targetDeviceId != null) {
      final target = _targetDeviceId!;
      final deviceId = _deviceId;
      final key = _routeKey();
      if (deviceId == null || key == null) {
        debugPrint(
          'WS: cannot route a binary frame through the relay: no route key',
        );
        return;
      }
      final Uint8List payload = message is Uint8List
          ? message
          : Uint8List.fromList(utf8.encode(jsonEncode(message)));
      try {
        _channel!.sink.add(
          buildBinaryFrame(
            routeKey: key,
            fromDeviceId: deviceId,
            targetDeviceId: target,
            sequence: Uint32List.fromList([_nextSequence()]),
            payload: payload,
          ),
        );
      } on FormatException catch (e) {
        _surfaceSendError(
          'Could not build the relayed file frame: ${e.message}',
        );
      }
      return;
    }

    if (message is Uint8List) {
      _channel!.sink.add(message);
    } else {
      _channel!.sink.add(jsonEncode(message));
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
          final (nonceBytes, combinedBytes) = await EncryptionService().encrypt(
            secretBytes,
            plaintext,
          );

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
        // Signed with this device's own route key, never the bearer token: the
        // token authenticates the connection, the key proves which device is
        // speaking. Signing with the token let any paired phone forge a route
        // claiming to be the desktop or another phone.
        final route = _asSignedRoute(payloadToSend);
        if (route == null) {
          _surfaceSendError(
            'Cannot reach the other device through the relay — this device is not '
            'paired yet, so it has no key to sign with',
          );
          return;
        }
        _channel!.sink.add(jsonEncode(route));
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
    _sendMessage({'type': 'notification', 'action': 'dismiss', 'id': id});
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
    _sendMessage({'type': 'notification', 'action': 'mark_read', 'id': id});
  }

  void sendStatusUpdate({int? battery}) {
    _sendMessage({
      'type': 'status',
      'action': 'update',
      'battery': ?battery,
    });
  }

  void sendPairingRequest(
    String token,
    String publicKey,
    Map<String, dynamic> deviceInfo,
  ) {
    _sendMessage({
      'type': 'pairing',
      'action': 'request',
      'token': token,
      'public_key': publicKey,
      'device_info': deviceInfo,
    });
  }

  void sendSmsMessage(String to, String body) {
    _sendMessage({'type': 'sms', 'action': 'send', 'to': to, 'body': body});
  }

  void sendCallAction(
    String action, {
    String? callId,
    String? toDeviceId,
    String? route,
  }) {
    _sendMessage({
      'type': 'call',
      'action': action,
      'call_id': ?callId,
      'to_device_id': ?toDeviceId,
      'route': ?route,
    });
  }

  void sendFileRequest(
    String id,
    String name,
    int size,
    String mime,
    String toDeviceId,
  ) {
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
