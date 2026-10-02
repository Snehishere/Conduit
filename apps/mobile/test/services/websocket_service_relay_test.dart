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
/// [WebSocketService] against a loopback server standing in for the relay and
/// inspects the frames that actually leave the phone.
///
/// Every test reaches the relay the way the shipping app does: the phone pairs
/// over the LAN first, and the desktop's `pairing/accept` is the only source of
/// both device ids. It used to be written the other way round, with a target id
/// pushed in through `setRelayConfig`, which no production call site ever did —
/// so the whole file asserted a state the app cannot enter, including on the
/// test that exists to prove signing works.
const String vectorSecret =
    '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f';

const String relayToken = 'relay-token-shared-by-every-client';

/// The id the desktop files this phone under.
const String phoneId = 'phone-1';

/// The desktop's own id, as it arrives in `hub_device_id`.
///
/// The relay's routing table is keyed by the id each connection authenticated
/// with, so this is the address a relayed message is sent to.
const String hubId = 'desktop-1';

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

/// Undo the envelope the phone wrapped, exactly as the hub would.
Future<Map<String, dynamic>> decryptEnvelope(
  Map<String, dynamic> envelope,
) async {
  final secret = EncryptionService.hexToBytes(vectorSecret);
  expect(
    EncryptionService.verifyHmac(
      secret,
      envelope['data'] as String,
      envelope['hmac'] as String,
    ),
    isTrue,
    reason: 'the envelope must still be authentic under the shared secret',
  );
  final plaintext = await EncryptionService().decrypt(
    secret,
    EncryptionService.hexToBytes(envelope['nonce'] as String),
    EncryptionService.hexToBytes(envelope['data'] as String),
  );
  return jsonDecode(plaintext) as Map<String, dynamic>;
}

/// A loopback WebSocket server standing in for whichever peer the phone is
/// talking to. Records every frame it is sent and can push frames back.
class _FakePeer {
  _FakePeer(this._server) {
    _server.listen((request) async {
      final socket = await WebSocketTransformer.upgrade(request);
      _sockets.add(socket);
      _wakeSocketWaiters();
      socket.listen((message) {
        _frames.add(
          message is String
              ? message
              : Uint8List.fromList(message as List<int>),
        );
        if (!_waiting.isCompleted) _waiting.complete();
        _waiting = Completer<void>();
      });
      socket.done.then((_) {
        _sockets.remove(socket);
        _wakeSocketWaiters();
      });
    });
  }

  static Future<_FakePeer> start() async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    return _FakePeer(server);
  }

  final HttpServer _server;
  final List<Object> _frames = [];
  final List<WebSocket> _sockets = [];
  final List<Completer<void>> _socketWaiters = [];
  var _waiting = Completer<void>();

  String get url => 'ws://${_server.address.address}:${_server.port}';

  int get count => _frames.length;

  /// How many `relay_route` frames were recorded at or after [from].
  int routesFrom(int from) {
    var routes = 0;
    for (var i = from; i < _frames.length; i++) {
      final frame = _frames[i];
      if (frame is String &&
          (jsonDecode(frame) as Map<String, dynamic>)['type'] == 'relay_route') {
        routes++;
      }
    }
    return routes;
  }

  int get connectionCount => _sockets.length;

  void _wakeSocketWaiters() {
    final waiters = List<Completer<void>>.from(_socketWaiters);
    _socketWaiters.clear();
    for (final waiter in waiters) {
      if (!waiter.isCompleted) waiter.complete();
    }
  }

  /// The frame at [index], waiting for the earlier ones to arrive.
  Future<Object> next(int index) async {
    while (_frames.length <= index) {
      await _waiting.future;
    }
    return _frames[index];
  }

  /// The first frame at or after [from] that [match] accepts.
  ///
  /// Scanning beats a fixed index because the number of frames a connection
  /// opens with is not fixed: a relay connection authenticates *and* announces,
  /// a LAN connection only announces. Asserting on "the second frame" once meant
  /// asserting on the announce.
  Future<T> awaitFrame<T>(
    FutureOr<T?> Function(Map<String, dynamic> frame) match, {
    int from = 0,
    Duration timeout = const Duration(seconds: 5),
  }) async {
    var index = from;
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      if (index < _frames.length) {
        final raw = _frames[index];
        if (raw is String) {
          final matched = await match(jsonDecode(raw) as Map<String, dynamic>);
          if (matched != null) return matched;
        }
        index++;
      } else {
        // Timed rather than awaited outright: the deadline above can only be
        // checked between frames, so a peer that goes quiet has to wake the
        // loop up or the test hangs instead of failing.
        try {
          await _waiting.future.timeout(const Duration(milliseconds: 200));
        } on TimeoutException {
          // Re-check the deadline.
        }
      }
    }
    fail('no frame matching the expectation arrived within $timeout');
  }

  /// The first JSON frame at or after [from] whose `type` is [type].
  Future<Map<String, dynamic>> awaitType(
    String type, {
    int from = 0,
  }) {
    return awaitFrame(
      (frame) => frame['type'] == type ? frame : null,
      from: from,
    );
  }

  /// The first binary frame at or after [from].
  Future<Uint8List> awaitBinary({
    int from = 0,
    Duration timeout = const Duration(seconds: 5),
  }) async {
    var index = from;
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      if (index < _frames.length) {
        final raw = _frames[index];
        if (raw is Uint8List) return raw;
        index++;
      } else {
        try {
          await _waiting.future.timeout(const Duration(milliseconds: 200));
        } on TimeoutException {
          // Re-check the deadline.
        }
      }
    }
    fail('no binary frame arrived within $timeout');
  }

  /// Wait until the phone has opened more than [count] connections.
  Future<void> awaitConnectionAfter(int count) async {
    while (_sockets.length <= count) {
      final waiter = Completer<void>();
      _socketWaiters.add(waiter);
      // The socket can land between the length check and the add; re-check
      // rather than wait forever on a waiter nobody will complete.
      if (_sockets.length > count) {
        _socketWaiters.remove(waiter);
        return;
      }
      await waiter.future;
    }
  }

  /// Push a frame to the phone, the way the relay pushes a routing decision.
  ///
  /// Bytes go out as a binary frame and maps as text, which is the whole
  /// distinction the client is about: decoding a v2 frame as JSON is what made
  /// it undecodable.
  ///
  /// Defaults to the newest connection, which is the live one: the phone opens
  /// a second socket to switch to the relay and keeps the paired LAN one open.
  Future<void> send(Object message, {int? connection}) async {
    final index = connection ?? _sockets.length - 1;
    await awaitConnectionAfter(index);
    _sockets[index].add(
      message is String || message is List<int>
          ? message
          : jsonEncode(message),
    );
  }

  /// The desktop's answer to a pairing request.
  ///
  /// [extra] is merged over the base frame, which is how the relay endpoint is
  /// tested: a desktop that hosts a relay delivers its address, token and
  /// certificate pin here, and one that does not simply omits them.
  Future<void> acceptPairing({
    String deviceId = phoneId,
    String? hubDeviceId = hubId,
    Map<String, dynamic> extra = const {},
  }) {
    return send({
      'type': 'pairing',
      'action': 'accept',
      'protocol_version': 1,
      'public_key': 'ab${'cd' * 31}',
      'device_info': {'name': 'Conduit Desktop', 'type': 'desktop'},
      'device_id': deviceId,
      'hub_device_id': ?hubDeviceId,
      ...extra,
    });
  }

  Future<void> stop() => _server.close(force: true);
}

/// A relayed file chunk, as the relay delivers one: a v2 frame carrying the
/// hub's own chunk envelope.
Future<Uint8List> relayedChunk({
  required int sequence,
  String targetId = phoneId,
  String fromId = hubId,
  String transferId = 'transfer-1',
}) async {
  final metadata = utf8.encode(
    jsonEncode({'id': transferId, 'index': 0, 'total': 1}),
  );
  final length = ByteData(4)..setUint32(0, metadata.length, Endian.little);
  final (nonce, ciphertext) = await EncryptionService().encryptBinary(
    EncryptionService.hexToBytes(vectorSecret),
    utf8.encode('chunk-bytes'),
  );
  final payload = BytesBuilder()
    ..add(nonce)
    ..add(length.buffer.asUint8List())
    ..add(metadata)
    ..add(ciphertext);
  return buildBinaryFrame(
    routeKey: deriveRouteKey(EncryptionService.hexToBytes(vectorSecret), fromId),
    fromDeviceId: fromId,
    targetDeviceId: targetId,
    sequence: Uint32List.fromList([sequence]),
    payload: payload.toBytes(),
  );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late _FakePeer peer;
  late WebSocketService service;

  /// Resolve once the phone has processed a `pairing/accept`.
  ///
  /// `_handleMessage` adopts both ids *before* it dispatches to handlers, so a
  /// handler seeing the accept is proof the ids are in place — the only order
  /// the production path produces them in.
  Future<void> acceptPairing({
    String? hubDeviceId = hubId,
    Map<String, dynamic> extra = const {},
  }) async {
    final accepted = Completer<void>();
    service.registerHandler('pairing', (message) {
      if (message['action'] == 'accept' && !accepted.isCompleted) {
        accepted.complete();
      }
    });
    await peer.acceptPairing(hubDeviceId: hubDeviceId, extra: extra);
    await accepted.future;
  }

  /// Switch the paired phone onto the relay, as it does once the hub is not
  /// reachable, and wait until the connection has actually settled.
  ///
  /// Waiting matters: the connect-time announce is a routed frame, so a test
  /// that measures "no route left the phone" has to let it out first or it
  /// measures its own race.
  Future<void> openRelay() async {
    final before = peer.connectionCount;
    final routesBefore = peer.count;
    await service.connect('');
    await peer.awaitConnectionAfter(before);
    await peer.awaitType('relay_auth');
    await peer.awaitFrame(
      (frame) => frame['type'] == 'relay_route' ? frame : null,
      from: routesBefore,
    );
  }

  /// The route carrying the clipboard message, checked end to end.
  ///
  /// Matching on the *decrypted payload* rather than on a frame index is the
  /// point: the discovery announce sent at connect is itself a routed frame, so
  /// a test that read "the frame after the auth frame" was reading the announce
  /// and asserting nothing about the clipboard at all.
  Future<Map<String, dynamic>> clipboardRoute({int from = 0}) {
    return peer.awaitFrame((frame) async {
      if (frame['type'] != 'relay_route') return null;
      final envelope = frame['payload'];
      if (envelope is! Map<String, dynamic>) return null;
      final message = await decryptEnvelope(envelope);
      if (message['type'] == 'clipboard' && message['action'] == 'sync') {
        expect(message['content'], 'pasted text');
        return frame;
      }
      return null;
    }, from: from);
  }

  setUp(() async {
    peer = await _FakePeer.start();
    service = WebSocketService();
    service.getSharedSecret = () => vectorSecret;
    // The relay the desktop would have delivered, pointing at the stand-in.
    service.setRelayConfig(peer.url);
    service.setRelayToken(relayToken);
    // Pair over the LAN first: this is the order production produces, and it is
    // the only way this phone has ever learned either id.
    await service.connect(peer.url);
    await peer.awaitConnectionAfter(0);
    await acceptPairing();
  });

  tearDown(() {
    service.dispose();
    peer.stop();
  });

  group('what the phone sends over a relay connection', () {
    test('authenticates the connection with the token, as the relay requires',
        () async {
      await openRelay();

      final auth = await peer.awaitType('relay_auth');
      expect(
        auth['device_id'],
        phoneId,
        reason: 'the id the desktop assigned at pairing, not a placeholder',
      );
      expect(auth['relay_token'], relayToken);
    });

    test('signs a relayed message with its own per-device route key', () async {
      await openRelay();
      service.sendClipboardSync('pasted text', 'text/plain');

      final route = await clipboardRoute();
      expect(route['from_device_id'], phoneId);
      expect(route['key_id'], phoneId);
      expect(route['to_device_id'], hubId);

      // The relay derives the key it verifies under from the pairing secret and
      // the authenticated device id, so this is the only key under which the
      // route can verify.
      final routeKey = deriveRouteKey(
        EncryptionService.hexToBytes(vectorSecret),
        phoneId,
      );
      expect(verifyMessageHmac(routeKey, route), isTrue);
      // Recomputed here from the relay's own rule rather than through the
      // production signer, so a route that hashed its insertion order fails
      // even though the client and its own signer agree.
      expect(signHex(routeKey, _relayCanonical(route)), route['hmac']);
    });

    test('never signs with the relay token', () async {
      // The exact defect: every client holds the token, so a route signed with
      // it can claim to be any device, and `from_device_id` was outside the
      // signature entirely. The token authenticates the connection; the key
      // proves who is speaking.
      await openRelay();
      service.sendClipboardSync('pasted text', 'text/plain');

      final route = await clipboardRoute();
      expect(
        verifyMessageHmac(utf8.encode(relayToken), route),
        isFalse,
        reason: 'the bearer token must never be a signing key',
      );
      expect(
        verifyMessageHmac(
          deriveRouteKey(EncryptionService.hexToBytes(vectorSecret), hubId),
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
      await openRelay();
      service.sendClipboardSync('pasted text', 'text/plain');

      final route = await clipboardRoute();
      for (final field in signedFields) {
        expect(route[field], isNotNull, reason: '$field must be present');
      }
      expect(hasRequiredSignedFields(route), isTrue);
    });

    test('emits a v2 binary frame, tagged and sequenced', () async {
      await openRelay();
      service.sendBinaryMessage(
        Uint8List.fromList(utf8.encode('chunk-payload')),
      );

      final frame = await peer.awaitBinary();
      expect(frame[0], 0x02, reason: 'the relay rejects 0x01 outright');
      expect(frame.length, greaterThanOrEqualTo(binaryHeaderLen));

      final parsed = parseBinaryFrame(frame);
      expect(parsed.targetId, hubId);
      expect(parsed.payload, utf8.encode('chunk-payload'));
      expect(
        verifyBinaryFrame(
          routeKey: deriveRouteKey(
            EncryptionService.hexToBytes(vectorSecret),
            phoneId,
          ),
          fromDeviceId: phoneId,
          frame: frame,
        ),
        isTrue,
      );
      expect(
        verifyBinaryFrame(
          routeKey: utf8.encode(relayToken),
          fromDeviceId: phoneId,
          frame: frame,
        ),
        isFalse,
      );
    });

    test('sends nothing at all rather than an unsigned route', () async {
      // No device id means no key can be derived, and the relay would refuse the
      // route anyway. Failing closed keeps a broken configuration visible instead
      // of producing silent undelivered messages.
      await openRelay();
      service.setDeviceId('');
      final before = peer.count;
      service.sendClipboardSync('pasted text', 'text/plain');
      await Future<void>.delayed(const Duration(milliseconds: 300));

      expect(peer.routesFrom(before), 0, reason: 'no routable frame at all');
      expect(service.lastError, isNotNull);
    });

    test('addresses the route to the hub id from the pairing in effect',
        () async {
      // The target is not a preference and not a remembered choice: it is the id
      // the paired hub holds in the relay's routing table. A second pairing can
      // only come from a second hub, so the address has to move with it.
      await openRelay();
      await acceptPairing(hubDeviceId: 'desktop-2');

      service.sendClipboardSync('pasted text', 'text/plain');
      final route = await clipboardRoute();
      expect(route['to_device_id'], 'desktop-2');
    });
  });

  group('where the relay address comes from', () {
    test('pairing/accept hands over the address, the token and the pin',
        () async {
      // What the harness pointed the service at is replaced by what the desktop
      // said, which is what proves the value is learned rather than retained.
      await acceptPairing(extra: {
        'relay_url': 'wss://relay.example:9529',
        'relay_token': 'token-from-the-desktop',
        'relay_cert_pin': 'sha256/3q2+7w==',
      });

      // This is what makes the relay reachable with no manual step: the desktop
      // generated the token and owns the listener, so it is the only party that
      // can hand over both. The pin matters just as much — the relay serves its
      // own certificate, and verifying it against the hub's pin fails closed.
      expect(service.relayUrl, 'wss://relay.example:9529');
      expect(service.relayToken, 'token-from-the-desktop');
    });

    test('an older desktop that omits them leaves the configuration alone',
        () async {
      service.setRelayConfig('wss://kept.example:9529');
      service.setRelayToken('kept-token');

      // No `relay_url`, no `relay_token`, no pin — a desktop that does not host
      // a relay, or one older than this. Whatever was configured has to survive
      // untouched and the relay path has to keep working regardless.
      await acceptPairing();
      expect(service.relayUrl, 'wss://kept.example:9529');
      expect(service.relayToken, 'kept-token');
    });
  });

  group('the LAN path', () {
    test('is untouched: a bare encrypted envelope, never a signed route',
        () async {
      service.sendClipboardSync('pasted text', 'text/plain');

      final frame = await peer.awaitFrame((frame) async {
        if (frame['type'] != 'encrypted') return null;
        final message = await decryptEnvelope(frame);
        return message['type'] == 'clipboard' ? frame : null;
      });

      // Not `relay_route`: the hub terminates, it does not route. Wrapping LAN
      // traffic in a route would hand every message to a relay that is not
      // there, and it is exactly the over-reach that fixing the relay path has
      // to avoid.
      expect(frame['type'], 'encrypted');
      expect(frame.containsKey('to_device_id'), isFalse);
      expect(frame.containsKey('hmac') && frame.containsKey('route'), isFalse);
    });

    test('sends file chunks in the hub envelope, not as a v2 frame', () async {
      service.sendBinaryMessage(
        Uint8List.fromList(utf8.encode('chunk-payload')),
      );

      expect(await peer.awaitBinary(), utf8.encode('chunk-payload'),
          reason: 'the LAN chunk envelope is passed straight through');
    });
  });

  group('connection lifetime', () {
    test('accepts a relayed chunk after a reconnect re-bases the sequence',
        () async {
      await openRelay();
      final delivered = <Map<String, dynamic>>[];
      service.registerHandler('file', (message) {
        if (message['action'] == 'chunk_binary') delivered.add(message);
      });

      // First relay connection: the relay numbers what it delivers from 1.
      await peer.send(await relayedChunk(sequence: 5));
      await peer.send(await relayedChunk(sequence: 6));
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(delivered.length, 2);

      // A frame whose sequence does not advance is a replay, on this connection.
      await peer.send(await relayedChunk(sequence: 6));
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(delivered.length, 2, reason: 'a replayed sequence is dropped');

      // Reconnect. The relay re-bases its outbound sequence per connection, so
      // the new one starts again from 1 — below the high-water mark the previous
      // one left behind. Keeping that mark drops every relayed chunk for the rest
      // of the process's life. Driven through `connect()` rather than by waiting
      // for the backoff timer, which calls exactly this.
      final before = peer.connectionCount;
      await service.connect('');
      await peer.awaitConnectionAfter(before);

      await peer.send(await relayedChunk(sequence: 1));
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(
        delivered.length,
        3,
        reason: 'the inbound high-water mark is per connection, so it resets',
      );
    });

    test('answers the relay ping on the connection, not as a routed message',
        () async {
      await openRelay();
      await peer.send({'type': 'ping'});

      // A keep-alive belongs to the socket. Routed, it would be delivered to the
      // desktop and the relay would never see an answer.
      expect(await peer.awaitType('pong'), {'type': 'pong'});
    });

    test("surfaces the relay's refusal instead of dropping it silently",
        () async {
      await openRelay();
      await peer.send({
        'type': 'error',
        'code': 'not_wrapped_in_relay_route',
        'message': "'encrypted' must be wrapped in a signed relay_route",
      });
      await Future<void>.delayed(const Duration(milliseconds: 200));

      // This is the difference between a phone that is connected and a phone
      // that works: the refusal is the only place the real fault is stated.
      expect(service.lastError, contains('not_wrapped_in_relay_route'));
      expect(service.lastError, contains('relay_route'));
    });

    test('reports a rejected relay authentication', () async {
      await openRelay();
      await peer.send({
        'type': 'relay_auth_rejected',
        'reason': 'invalid_token',
      });
      await Future<void>.delayed(const Duration(milliseconds: 200));

      // Otherwise the socket just closes and the app reconnects every three
      // seconds forever with nothing ever said about why.
      expect(service.lastError, contains('invalid_token'));
    });
  });
}