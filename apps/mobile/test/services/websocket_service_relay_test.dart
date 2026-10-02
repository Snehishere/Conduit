import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:conduit/services/encryption_service.dart';
import 'package:conduit/services/relay_route.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:flutter_test/flutter_test.dart';

/// What the phone actually puts on the wire when it is talking to a relay.
///
/// `relay_route_test.dart` pins the primitives — the route key, the canonical
/// string, the frame builder — against vectors the Rust side also pins. It
/// cannot see whether the *caller* passes the right key in, though: the defect
/// this file exists for was a caller signing with the relay bearer token, which
/// every primitive would have hashed correctly. So this drives
/// [WebSocketService] against a loopback server that stands in for the relay
/// and inspects the frames that actually leave the phone.
const String vectorSecret =
    '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f';

const String relayToken = 'relay-token-shared-by-every-client';

/// The relay rebuilds the signed bytes by re-serialising the received JSON
/// through a `BTreeMap`, so every object comes out alphabetically ordered at
/// every depth. Written out here rather than called from `relay_route.dart` on
/// purpose: a caller that hashed its own insertion order would still agree with
/// the production signer, and only an independent implementation of the relay's
/// rule catches that.
Object? _sortedByKey(Object? value) {
  if (value is Map) {
    final keys = value.keys.map((k) => k as String).toList()..sort();
    return <String, Object?>{
      for (final key in keys) key: _sortedByKey(value[key]),
    };
  }
  if (value is List) return value.map(_sortedByKey).toList();
  return value;
}

String _relayCanonical(Map<String, dynamic> message) {
  final canonical = <String, Object?>{};
  for (final field in const [
    'type',
    'from_device_id',
    'to_device_id',
    'payload',
    'timestamp',
    'nonce',
    'key_id',
  ]) {
    if (message.containsKey(field)) canonical[field] = message[field];
  }
  return jsonEncode(_sortedByKey(canonical));
}

/// A loopback WebSocket server that records every frame it is sent.
class _FakeRelay {
  _FakeRelay(this._server) {
    _server.listen((request) async {
      final socket = await WebSocketTransformer.upgrade(request);
      socket.listen((message) {
        _frames.add(
          message is String
              ? message
              : Uint8List.fromList(message as List<int>),
        );
        if (!_waiting.isCompleted) _waiting.complete();
        _waiting = Completer<void>();
      });
    });
  }

  static Future<_FakeRelay> start() async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    return _FakeRelay(server);
  }

  final HttpServer _server;
  final List<Object> _frames = [];
  var _waiting = Completer<void>();

  String get url => 'ws://${_server.address.address}:${_server.port}';

  /// The next frame after the [skip] already-recorded ones.
  Future<Object> next(int skip) async {
    while (_frames.length <= skip) {
      await _waiting.future;
    }
    return _frames[skip];
  }

  int get count => _frames.length;

  Future<void> stop() => _server.close(force: true);
}

void main() {
  late _FakeRelay relay;
  late WebSocketService service;

  setUp(() async {
    relay = await _FakeRelay.start();
    service = WebSocketService();
    service.setDeviceId('phone-1');
    service.setRelayConfig(relay.url, 'desktop-1');
    service.setRelayToken(relayToken);
    service.getSharedSecret = () => vectorSecret;
    await service.connect('');
  });

  tearDown(() {
    service.dispose();
    relay.stop();
  });

  /// The relay authenticates the connection before anything can be routed, so
  /// the first frame is always `relay_auth`; routes start after it.
  Future<Map<String, dynamic>> nextRoute() async {
    final raw = await relay.next(1);
    final message = jsonDecode(raw as String) as Map<String, dynamic>;
    expect(message['type'], 'relay_route');
    return message;
  }

  group('what the phone sends over a relay connection', () {
    test('authenticates the connection with the token, as the relay requires',
        () async {
      final auth =
          jsonDecode(await relay.next(0) as String) as Map<String, dynamic>;
      expect(auth['type'], 'relay_auth');
      expect(auth['device_id'], 'phone-1');
      expect(auth['relay_token'], relayToken);
    });

    test('signs a relayed message with its own per-device route key', () async {
      service.sendClipboardSync('pasted text', 'text/plain');

      final route = await nextRoute();
      expect(route['from_device_id'], 'phone-1');
      expect(route['key_id'], 'phone-1');
      expect(route['to_device_id'], 'desktop-1');

      // The relay derives the key it verifies under from the pairing secret and
      // the authenticated device id, so this is the only key under which the
      // route can verify.
      final routeKey = deriveRouteKey(
        EncryptionService.hexToBytes(vectorSecret),
        'phone-1',
      );
      expect(verifyMessageHmac(routeKey, route), isTrue);
      // Recomputed here from the relay's own rule rather than through the
      // production signer, so a route that hashed its insertion order fails
      // even though the client and its own signer agree.
      expect(
        signHex(routeKey, _relayCanonical(route)),
        route['hmac'],
      );
    });

    test('never signs with the relay token', () async {
      // The exact defect: every client holds the token, so a route signed with
      // it can claim to be any device, and `from_device_id` was outside the
      // signature entirely. The token authenticates the connection; the key
      // proves who is speaking.
      service.sendClipboardSync('pasted text', 'text/plain');

      final route = await nextRoute();
      expect(
        verifyMessageHmac(utf8.encode(relayToken), route),
        isFalse,
        reason: 'the bearer token must never be a signing key',
      );
      expect(
        verifyMessageHmac(
          deriveRouteKey(
            EncryptionService.hexToBytes(vectorSecret),
            'desktop-1',
          ),
          route,
        ),
        isFalse,
        reason: "one device's route key must not sign for another",
      );
    });

    test('covers all seven signed fields, key_id included', () async {
      // A route missing any of these is refused by the relay before it is
      // routed, and one missing a signed field could have it rewritten in
      // flight without breaking the MAC.
      service.sendClipboardSync('pasted text', 'text/plain');

      final route = await nextRoute();
      for (final field in signedFields) {
        expect(route[field], isNotNull, reason: '$field must be present');
      }
      expect(hasRequiredSignedFields(route), isTrue);
    });

    test('emits a v2 binary frame, tagged and sequenced', () async {
      service.sendBinaryMessage(
        Uint8List.fromList(utf8.encode('chunk-payload')),
      );

      final frame = await relay.next(1) as Uint8List;
      expect(frame[0], 0x02, reason: 'the relay rejects 0x01 outright');
      expect(frame.length, greaterThanOrEqualTo(binaryHeaderLen));

      final parsed = parseBinaryFrame(frame);
      expect(parsed.targetId, 'desktop-1');
      expect(parsed.payload, utf8.encode('chunk-payload'));
      expect(
        verifyBinaryFrame(
          routeKey: deriveRouteKey(
            EncryptionService.hexToBytes(vectorSecret),
            'phone-1',
          ),
          fromDeviceId: 'phone-1',
          frame: frame,
        ),
        isTrue,
      );
      expect(
        verifyBinaryFrame(
          routeKey: utf8.encode(relayToken),
          fromDeviceId: 'phone-1',
          frame: frame,
        ),
        isFalse,
      );
    });

    test('sends nothing at all rather than an unsigned route', () async {
      // No device id means no key can be derived, and the relay would refuse
      // the route anyway. Failing closed keeps a broken configuration visible
      // instead of producing silent undelivered messages.
      service.setDeviceId('');
      service.sendClipboardSync('pasted text', 'text/plain');
      await Future<void>.delayed(const Duration(milliseconds: 100));

      expect(relay.count, 1, reason: 'only the relay_auth frame');
      expect(service.lastError, isNotNull);
    });
  });
}