import 'dart:convert';
import 'dart:typed_data';

import 'package:conduit/services/encryption_service.dart';
import 'package:conduit/services/relay_route.dart';
import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter_test/flutter_test.dart';

/// The four constants below are produced by the Rust relay and pinned there by
/// `services/relay/src/suite.rs::interop_vector_for_the_dart_client`. If this
/// file fails, that one will fail too — re-emit with
/// `cargo test -p conduit-relay interop_vector -- --nocapture` and update both.
const String vectorRouteKey =
    '2cd721ceb5f78ef0f105593aa6feb7b5bc6f17c323d9ad32d6a34a166b770914';
const String vectorCanonical =
    '{"from_device_id":"deadbeef","key_id":"deadbeef","nonce":"nonce-1",'
    '"payload":{"alpha":2,"nested":{"a":1,"b":2},"type":"ping","zeta":1},'
    '"timestamp":1700000000000,"to_device_id":"cafebabe","type":"relay_route"}';
const String vectorHmac =
    '9c2018aa54f8a9fe7fa674899eee2665d891e7f34ee38077effc541132796dcb';
const String vectorSecret =
    '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f';

/// Whole-frame vectors, byte for byte, produced by
/// `conduit_protocol::build_binary_frame` — the single Rust producer the relay
/// itself re-frames with (`services/relay/src/connection.rs`), so if these
/// match, a frame this client builds is a frame the relay parses and tags
/// identically. `buildBinaryFrame` below must reproduce them exactly.
///
/// The Dart tests for v2 frames were previously self-consistent only: they
/// built a frame and then parsed it back with Dart code, which cannot detect
/// the two implementations drifting apart. These constants are that check.
///
/// Re-emit from the Rust producer — `build_binary_frame` takes the same route
/// key the vector above is derived from, so this is the exact call:
///
/// ```text
/// conduit_protocol::build_binary_frame(
///     &hmac::derive_route_key(&secret, "deadbeef"),
///     "deadbeef", "cafebabe", 7, b"hello",
/// )
/// ```
///
/// `packages/protocol/src/types.rs` documents that its own tests pin this pair,
/// and the Rust half of that assertion does not exist yet; whoever owns that
/// file should add it so neither side is a one-way pin.
const String vectorFrameHello =
    '026361666562616265000000000000000000000007b721e258479b9f7c7b4b0ca343acde53f'
    '1a495b884cb568297114c7dca056c5f68656c6c6f';
const String vectorFrameEmpty =
    '026361666562616265000000000000000000000001138f68922f828ce72b7573213db3e'
    '2f798245c651512f1155623ce3aeb7ee865';
const String vectorFrameLong =
    '02303132333435363738396162636465660102030475dcdfbaa081e2127b1d6df93df6de'
    '99614b28792e84b5e40baedfa4a1d826b874686520717569636b2062726f776e20666f78';
const String vectorFrameTruncated =
    '0230313233343536373839616263646566000000022129374e159f29fd2130c0bd1f17b'
    '616dee807e3994120f91bcbb0e35fcffb6178';

/// A frame addressed to a real 36-character device id — the case the other
/// vectors cannot reach. `53553065383430302d653239622d3431` is "550e8400-e29b-41",
/// the id's first 16 bytes and nothing else, so this vector is also the pin on
/// *what the 16-byte field means* (PROTOCOL.md §5.1.4), not just on the framing.
///
/// Pinned on both sides: `packages/protocol/src/types.rs::the_dart_client_builds_
/// the_same_frames` asserts the same bytes from `build_binary_frame`.
const String vectorFrameUuid =
    '0235353065383430302d653239622d343100000009032b8e9c8245ecb70eac249da51aad2'
    '851efb926aec7bd389496aa1ccd308c9066696c652d6368756e6b';

List<int> _secret() => EncryptionService.hexToBytes(vectorSecret);

Map<String, dynamic> _vectorMessage() => {
  'type': 'relay_route',
  'from_device_id': 'deadbeef',
  'to_device_id': 'cafebabe',
  'payload': {
    'type': 'ping',
    'zeta': 1,
    'alpha': 2,
    'nested': {'b': 2, 'a': 1},
  },
  'timestamp': 1700000000000,
  'nonce': 'nonce-1',
  'key_id': 'deadbeef',
};

void main() {
  group('interop with the Rust relay', () {
    test('derives the same route key', () {
      expect(hexEncode(deriveRouteKey(_secret(), 'deadbeef')), vectorRouteKey);
    });

    test('canonicalises to the same byte string', () {
      // The payload deliberately has non-alphabetical keys at two levels: the
      // relay re-serialises through serde_json, which sorts every object. A
      // client that hashed insertion order would get a different string here
      // and every route would be rejected with hmac_invalid.
      expect(canonicalSigningString(_vectorMessage()), vectorCanonical);
    });

    test('produces the same HMAC', () {
      expect(
        signHex(deriveRouteKey(_secret(), 'deadbeef'), vectorCanonical),
        vectorHmac,
      );
    });

    test('verifies a route the Rust side signed', () {
      final message = _vectorMessage()..['hmac'] = vectorHmac;
      expect(
        verifyMessageHmac(deriveRouteKey(_secret(), 'deadbeef'), message),
        isTrue,
      );
    });

    test('a route built here is accepted by the Rust key derivation', () {
      final route = signRoute(
        routeKey: deriveRouteKey(_secret(), 'deadbeef'),
        fromDeviceId: 'deadbeef',
        toDeviceId: 'cafebabe',
        payload: _vectorMessage()['payload'],
        timestampMs: 1700000000000,
        nonce: 'nonce-1',
      );
      expect(route['hmac'], vectorHmac);
    });
  });

  group('binary frame interop with the Rust producer', () {
    List<int> key() => deriveRouteKey(_secret(), 'deadbeef');
    Uint32List seq(int n) => Uint32List.fromList([n]);

    test('builds the exact frame conduit_protocol::build_binary_frame builds', () {
      // One short payload, an empty payload (the 53-byte minimum), a full
      // 16-byte target id with a large sequence and a long payload, and a
      // target id too long for the field. Together they cover every branch in
      // the builder: zero padding, truncation, an empty body and a normal one.
      expect(
        hexEncode(
          buildBinaryFrame(
            routeKey: key(),
            fromDeviceId: 'deadbeef',
            targetDeviceId: 'cafebabe',
            sequence: seq(7),
            payload: utf8.encode('hello'),
          ),
        ),
        vectorFrameHello,
      );
      expect(
        hexEncode(
          buildBinaryFrame(
            routeKey: key(),
            fromDeviceId: 'deadbeef',
            targetDeviceId: 'cafebabe',
            sequence: seq(1),
            payload: Uint8List(0),
          ),
        ),
        vectorFrameEmpty,
      );
      expect(
        hexEncode(
          buildBinaryFrame(
            routeKey: key(),
            fromDeviceId: 'deadbeef',
            targetDeviceId: '0123456789abcdef',
            sequence: seq(0x01020304),
            payload: utf8.encode('the quick brown fox'),
          ),
        ),
        vectorFrameLong,
      );
      expect(
        hexEncode(
          buildBinaryFrame(
            routeKey: key(),
            fromDeviceId: 'deadbeef',
            targetDeviceId: '0123456789abcdefEXTRA',
            sequence: seq(2),
            payload: utf8.encode('x'),
          ),
        ),
        vectorFrameTruncated,
      );
      expect(
        hexEncode(
          buildBinaryFrame(
            routeKey: key(),
            fromDeviceId: 'deadbeef',
            targetDeviceId: '550e8400-e29b-41d4-a716-446655440000',
            sequence: seq(9),
            payload: utf8.encode('file-chunk'),
          ),
        ),
        vectorFrameUuid,
        reason:
            'a 36-character device id is carried as its 16-byte prefix, and the '
            'Rust producer must write exactly the same bytes',
      );
    });

    test('the tagged region is the sender id, 0x1F, header and payload', () {
      // Asserted separately from the whole-frame equality above because this is
      // the part the relay recomputes: it rebuilds the MAC input from the bytes
      // it received, so a client that hashed anything else would still build a
      // self-consistent frame and would be rejected at the relay.
      //
      // deadbeef | 0x1F | 02 cafebabe<8 NUL> 00000007 | hello
      const macInput =
          '64656164626565661f02636166656261626500000000000000000000000768656c6c6f';
      final frame = Uint8List.fromList(
        EncryptionService.hexToBytes(vectorFrameHello),
      );
      final headerAndPayload = Uint8List(binaryTagOffset + 5)
        ..setRange(0, binaryTagOffset, frame)
        ..setRange(binaryTagOffset, binaryTagOffset + 5, utf8.encode('hello'));
      expect(hexEncode(binaryMacInput('deadbeef', headerAndPayload)), macInput);
    });

    test('a Rust-built frame verifies with the Dart verifier', () {
      final frame = Uint8List.fromList(
        EncryptionService.hexToBytes(vectorFrameHello),
      );
      expect(frame[0], binaryFrameVersion);
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'deadbeef', frame: frame),
        isTrue,
      );
      // And a Dart-built frame verifies as the Rust relay would: the tag is
      // keyed by the route key, not by the connection's bearer token, and names
      // the sender the relay authenticated on that connection.
      expect(
        verifyBinaryFrame(
          routeKey: utf8.encode('relay-token'),
          fromDeviceId: 'deadbeef',
          frame: frame,
        ),
        isFalse,
      );
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'cafebabe', frame: frame),
        isFalse,
        reason: 'the sender id is inside the MAC input',
      );
    });
  });

  group('route key derivation', () {
    test('binds the device id, so one secret gives unrelated keys', () {
      expect(
        deriveRouteKey(_secret(), 'aaa'),
        isNot(equals(deriveRouteKey(_secret(), 'bbb'))),
      );
    });

    test('is deterministic', () {
      expect(
        deriveRouteKey(_secret(), 'aaa'),
        deriveRouteKey(_secret(), 'aaa'),
      );
    });

    test('is not the pairing secret itself', () {
      // Otherwise a peer that knew the secret could sign as any device.
      expect(deriveRouteKey(_secret(), 'aaa'), isNot(equals(_secret())));
    });

    test('is not the relay token', () {
      final token = EncryptionService.bytesToHex(List<int>.filled(32, 0xAB));
      expect(deriveRouteKey(_secret(), 'aaa'), isNot(equals(token)));
    });
  });

  group('canonical signing string', () {
    test('covers exactly the seven signed fields', () {
      final canonical = canonicalSigningString({
        ..._vectorMessage(),
        'hmac': 'deadbeef',
        'not_signed': 'ignored',
      });
      expect(canonical, isNot(contains('hmac')));
      expect(canonical, isNot(contains('not_signed')));
    });

    test('omits absent fields rather than inventing them', () {
      final message = _vectorMessage()..remove('key_id');
      expect(canonicalSigningString(message), isNot(contains('key_id')));
    });

    test('sorts nested objects, not just the top level', () {
      final canonical = canonicalSigningString({
        'payload': {
          'z': {'b': 1, 'a': 2},
          'a': 3,
        },
      });
      expect(canonical, '{"payload":{"a":3,"z":{"a":2,"b":1}}}');
    });

    test('preserves list order, which is data rather than keys', () {
      expect(
        canonicalSigningString({
          'payload': [3, 1, 2],
        }),
        '{"payload":[3,1,2]}',
      );
    });
  });

  group('signed routes', () {
    List<int> key() => deriveRouteKey(_secret(), 'aaa');

    test('key_id is the sender id', () {
      // The relay resolves the key by this id and then requires the attributed
      // sender to match, so the two cannot be pointed at different devices.
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      );
      expect(route['key_id'], 'aaa');
      expect(route['from_device_id'], 'aaa');
    });

    test('verifies under the sender key', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      );
      expect(verifyMessageHmac(key(), route), isTrue);
    });

    test('does not verify under another device key', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      );
      expect(
        verifyMessageHmac(deriveRouteKey(_secret(), 'ccc'), route),
        isFalse,
      );
    });

    test('a tampered recipient breaks the MAC', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      )..['to_device_id'] = 'ccc';
      expect(verifyMessageHmac(key(), route), isFalse);
    });

    test('a tampered payload breaks the MAC', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      )..['payload'] = {'type': 'pong'};
      expect(verifyMessageHmac(key(), route), isFalse);
    });

    test('a rewritten sender breaks the MAC', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      )..['from_device_id'] = 'ccc';
      expect(verifyMessageHmac(key(), route), isFalse);
    });

    test('carries every field the relay requires', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      );
      expect(hasRequiredSignedFields(route), isTrue);
    });
  });

  group('verification is not fooled', () {
    List<int> key() => deriveRouteKey(_secret(), 'aaa');

    test('a missing hmac is rejected rather than treated as unsigned', () {
      expect(verifyMessageHmac(key(), _vectorMessage()), isFalse);
    });

    test('a non-string hmac is rejected', () {
      final message = _vectorMessage()..['hmac'] = 12345;
      expect(verifyMessageHmac(key(), message), isFalse);
    });

    test('an empty hmac is rejected', () {
      final message = _vectorMessage()..['hmac'] = '';
      expect(verifyMessageHmac(key(), message), isFalse);
    });

    test('a short hex tag is rejected without throwing', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      )..['hmac'] = 'abcd';
      expect(verifyMessageHmac(key(), route), isFalse);
    });

    test('a tag of the wrong length is rejected', () {
      final route = signRoute(
        routeKey: key(),
        fromDeviceId: 'aaa',
        toDeviceId: 'bbb',
        payload: {'type': 'ping'},
      )..['hmac'] = vectorHmac.substring(0, 32);
      expect(verifyMessageHmac(key(), route), isFalse);
    });
  });

  group('nonces', () {
    test('differ between calls', () {
      final nonces = List.generate(200, (_) => newNonce()).toSet();
      expect(
        nonces.length,
        200,
        reason: 'a repeated nonce is a dropped message',
      );
    });

    test('are long enough to not be guessable', () {
      expect(newNonce().length, 32, reason: '16 random bytes, hex-encoded');
    });
  });

  group('v2 binary frames', () {
    List<int> key() => deriveRouteKey(_secret(), 'aaa');
    Uint32List seq(int n) => Uint32List.fromList([n]);

    test('have the documented layout', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(7),
        payload: utf8.encode('hello'),
      );
      expect(frame[0], binaryFrameVersion);
      expect(frame.length, binaryHeaderLen + 5);
      expect(binaryHeaderLen, 53);
    });

    test('round-trip through parse', () {
      final payload = utf8.encode('hello world!!!!!');
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(7),
        payload: payload,
      );
      final parsed = parseBinaryFrame(frame);
      expect(parsed.targetId, 'bbb');
      expect(parsed.sequence, 7);
      expect(parsed.payload, payload);
    });

    test('verify under the sender key', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      );
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'aaa', frame: frame),
        isTrue,
      );
    });

    test('do not verify under another device key', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      );
      expect(
        verifyBinaryFrame(
          routeKey: deriveRouteKey(_secret(), 'ccc'),
          fromDeviceId: 'aaa',
          frame: frame,
        ),
        isFalse,
      );
    });

    test(
      'the sender id is inside the MAC input, so a frame cannot be replayed as another sender',
      () {
        final frame = buildBinaryFrame(
          routeKey: key(),
          fromDeviceId: 'aaa',
          targetDeviceId: 'bbb',
          sequence: seq(1),
          payload: utf8.encode('hello'),
        );
        expect(
          verifyBinaryFrame(routeKey: key(), fromDeviceId: 'ccc', frame: frame),
          isFalse,
        );
      },
    );

    test('a tampered payload fails the tag', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      );
      frame[binaryHeaderLen] ^= 0xFF;
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'aaa', frame: frame),
        isFalse,
      );
    });

    test('a retargeted frame fails the tag', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      );
      frame[1] = 0x78; // 'x'
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'aaa', frame: frame),
        isFalse,
      );
    });

    test('a rewritten sequence fails the tag', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      );
      frame[17] = 0x7F;
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'aaa', frame: frame),
        isFalse,
      );
    });

    test('an empty payload still produces a valid frame', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'b',
        sequence: seq(0),
        payload: Uint8List(0),
      );
      expect(frame.length, binaryHeaderLen);
      final parsed = parseBinaryFrame(frame);
      expect(parsed.targetId, 'b');
      expect(parsed.payload, isEmpty);
      expect(
        verifyBinaryFrame(routeKey: key(), fromDeviceId: 'aaa', frame: frame),
        isTrue,
      );
    });

    test('the 16-byte id field truncates rather than overflowing', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: '0123456789abcdefEXTRA',
        sequence: seq(1),
        payload: utf8.encode('x'),
      );
      expect(parseBinaryFrame(frame).targetId, '0123456789abcdef');
    });

    test('a short frame is rejected', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: Uint8List(0),
      );
      expect(
        () => parseBinaryFrame(frame.sublist(0, binaryHeaderLen - 1)),
        throwsA(isA<FormatException>()),
      );
    });

    test('a v1 frame is rejected, because the relay no longer accepts one', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      )..[0] = 0x01;
      expect(() => parseBinaryFrame(frame), throwsA(isA<FormatException>()));
    });

    test('a v0 frame is rejected', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      )..[0] = 0x00;
      expect(() => parseBinaryFrame(frame), throwsA(isA<FormatException>()));
    });

    test('a 255 version is rejected', () {
      final frame = buildBinaryFrame(
        routeKey: key(),
        fromDeviceId: 'aaa',
        targetDeviceId: 'bbb',
        sequence: seq(1),
        payload: utf8.encode('hello'),
      )..[0] = 0xFF;
      expect(() => parseBinaryFrame(frame), throwsA(isA<FormatException>()));
    });
  });

  // -------------------------------------------------------------------------
  //  The 16-byte target field
  //
  //  The field is 16 bytes and every device id the app mints is a 36-character
  //  UUID, so what it carries is the id's *prefix* — the rule is defined once, in
  //  `conduit_protocol::binary_target_field`, and pinned on both sides
  //  (`packages/protocol/src/types.rs::the_dart_client_builds_the_same_frames`).
  //  These assert what the field therefore is from this side, using only the
  //  public API — including the consequence that keeps breaking receivers.
  // -------------------------------------------------------------------------
  group('the 16-byte target field', () {
    const uuid = '550e8400-e29b-41d4-a716-446655440000';
    const twin = '550e8400-e29b-41d4-b716-446655440000';
    List<int> key() => deriveRouteKey(_secret(), 'deadbeef');
    Uint32List seq(int n) => Uint32List.fromList([n]);
    Uint8List addressedTo(String target) => buildBinaryFrame(
      routeKey: key(),
      fromDeviceId: 'deadbeef',
      targetDeviceId: target,
      sequence: seq(1),
      payload: utf8.encode('x'),
    );

    test('holds the first 16 bytes of a 36-character id', () {
      expect(
        addressedTo(uuid).sublist(1, 1 + binaryDeviceIdLen),
        ascii.encode('550e8400-e29b-41'),
      );
      expect(
        addressedTo('b145d').sublist(1, 1 + binaryDeviceIdLen),
        [...ascii.encode('b145d'), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        reason: 'a short id is the whole field, NUL-padded',
      );
    });

    test('the parsed target is the prefix, not the device id', () {
      // The defect, pinned from this side: a receiver that compares the parsed
      // target against its own 36-character id (`websocket_service.dart`, before
      // the fix) refuses every frame the relay correctly resolved to it. A
      // receiver must compare the 16 bytes with this device's own canonical
      // field — the same rule the relay's router and the desktop's
      // `binary_target_matches` use.
      final parsed = parseBinaryFrame(addressedTo(uuid));
      expect(parsed.targetId, '550e8400-e29b-41');
      expect(
        parsed.targetId,
        isNot(uuid),
        reason: 'the field cannot hold the whole id, so this comparison is the bug',
      );
    });

    test('two ids sharing those 16 bytes are indistinguishable on the wire', () {
      // Nothing in the frame format can separate them. This is why the relay
      // resolves the field to exactly one connected device and refuses it when
      // two match, and why a receiver cannot be the one that catches it: it sees
      // identical bytes either way.
      expect(
        addressedTo(uuid).sublist(1, 1 + binaryDeviceIdLen),
        addressedTo(twin).sublist(1, 1 + binaryDeviceIdLen),
      );
      expect(uuid.substring(0, 16), twin.substring(0, 16));
      expect(uuid, isNot(twin));
    });
  });

  group('field offsets', () {
    test('match the Rust constants exactly', () {
      // A drift here silently produces frames the relay misparses.
      expect(binaryTagOffset, 21);
      expect(binaryHeaderLen, 53);
      expect(binaryTagLen, 32);
      expect(binaryDeviceIdLen, 16);
      expect(binarySeqLen, 4);
      expect(binaryFrameVersion, 0x02);
      expect(binaryMacSeparator, 0x1f);
    });

    test('the label and prefix match the Rust KDF', () {
      expect(routeKeyLabel, 'conduit-relay/v1/route-key');
      expect(kdfPrefix, 'conduit-protocol/v1/derive:');
    });

    test('the signed field list matches the Rust one', () {
      expect(signedFields, [
        'type',
        'from_device_id',
        'to_device_id',
        'payload',
        'timestamp',
        'nonce',
        'key_id',
      ]);
    });
  });

  // -------------------------------------------------------------------------
  //  SPKI certificate pinning
  //
  //  The pin has to be over the SubjectPublicKeyInfo, because that is what the
  //  relay publishes (`GET /pin`, `"algorithm": "spki-sha256"`). This client
  //  used to hash `cert.der` — the whole certificate — which is a *different*
  //  value, so every pinned connection was rejected. The vector below is a real
  //  self-signed certificate whose SPKI was extracted by the Rust relay and
  //  cross-checked against `openssl` in `tls.rs`.
  // -------------------------------------------------------------------------
  group('SPKI certificate pinning', () {
    // Emitted by
    // `cargo test -p conduit-relay --lib emit_spki_vector_fixture -- --ignored --nocapture`
    // and re-read as a literal here so the two suites cannot drift apart
    // silently: if the certificate is ever regenerated, this test fails until
    // both sides are updated.
    const String spkiVectorCertDerBase64 =
        'MIIBdjCCARygAwIBAgIUTdtheOiQaKMm5JZ9DDVj7ziE8L8wCgYIKoZIzj0EAwIwITEfMB0GA1UEAwwWcmNnZW4gc2VsZiBzaWduZWQgY2VydDAgFw03NTAxMDEwMDAwMDBaGA80MDk2MDEwMTAwMDAwMFowITEfMB0GA1UEAwwWcmNnZW4gc2VsZiBzaWduZWQgY2VydDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABJ65zpdFT26LNjZKDwFXZe51IVtiCoCAT2d6s+ykrkQC+BKPMvzfXSCAxJRDI8fJ+RgygKrGaTYnmpHXa+z5mg2jMDAuMCwGA1UdEQQlMCOCCWxvY2FsaG9zdIcEfwAAAYcQAAAAAAAAAAAAAAAAAAAAATAKBggqhkjOPQQDAgNIADBFAiB1RBMMWyAOyR7vSNI7vub/qS9hScdjY87SyXzzy4izzAIhAJ2964KrwyvrXoIweTw+4Ggdv7OUbfs65Las3EIr7/Q5';
    const String spkiVectorPin = 'MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag=';

    final Uint8List spkiVectorCertDer = Uint8List.fromList(
      base64.decode(spkiVectorCertDerBase64),
    );

    test('the pin matches the one the relay publishes', () {
      expect(computeSpkiPin(spkiVectorCertDer), 'sha256/$spkiVectorPin');
    });

    test('the pin is over the SPKI, not the whole certificate', () {
      // The exact defect this replaces. If these ever compare equal, the pin has
      // silently reverted to hashing `cert.der`.
      final wholeCertPin = base64.encode(
        crypto.sha256.convert(spkiVectorCertDer).bytes,
      );
      expect(
        spkiVectorPin,
        isNot(wholeCertPin),
        reason:
            'SPKI pin and whole-certificate hash are different values; '
            'the relay publishes the SPKI one',
      );
    });

    test('the extracted SPKI is the real SubjectPublicKeyInfo', () {
      final spki = extractSpkiDer(spkiVectorCertDer);
      expect(spki[0], 0x30, reason: 'SPKI must be a DER SEQUENCE');
      // 30 59 => SEQUENCE, 89 content bytes, 91 total. Asserted as a literal
      // because the point is that both implementations agree on *these* bytes,
      // not that the value has some property.
      expect(spki.sublist(0, 2), [0x30, 0x59]);
      expect(spki.length, 91);
      // SubjectPublicKeyInfo ::= SEQUENCE { algorithm, subjectPublicKey BIT STRING }
      //
      //   30 59                      SPKI, 89 bytes
      //     30 13                    AlgorithmIdentifier, 19 bytes
      //       06 07 2a 86 48 ce 3d 02 01     id-ecPublicKey (1.2.840.10045.2.1)
      //       06 08 2a 86 48 ce 3d 03 01 07  prime256v1 (1.2.840.10045.3.1.7)
      //     03 42 00 04 ...          subjectPublicKey, 66 bytes, 0 unused bits
      expect(spki.sublist(0, 4), [
        0x30,
        0x59,
        0x30,
        0x13,
      ], reason: 'SPKI SEQUENCE, then the algorithm SEQUENCE');
      expect(spki.sublist(4, 13), [
        0x06,
        0x07,
        0x2a,
        0x86,
        0x48,
        0xce,
        0x3d,
        0x02,
        0x01,
      ], reason: 'id-ecPublicKey, 1.2.840.10045.2.1');
      expect(spki.sublist(13, 23), [
        0x06,
        0x08,
        0x2a,
        0x86,
        0x48,
        0xce,
        0x3d,
        0x03,
        0x01,
        0x07,
      ], reason: 'the named-curve parameter, prime256v1');
      expect(spki[23], 0x03, reason: 'subjectPublicKey is a BIT STRING');
      expect(spki[24], 0x42, reason: '66 bytes of key data');
      expect(spki[25], 0x00, reason: 'zero unused bits');
      // 0x04 || X(32) || Y(32) runs from [26] to the end.
      expect(spki[26], 0x04, reason: 'uncompressed EC point');
      expect(spki.length, 26 + 1 + 32 + 32);
    });

    test('extraction is deterministic', () {
      expect(
        extractSpkiDer(spkiVectorCertDer),
        extractSpkiDer(spkiVectorCertDer),
      );
    });

    test('malformed input throws rather than hashing the wrong bytes', () {
      // A pin computed over garbage would be a stable, wrong value that a
      // subsequent connection could match. Failing closed is the only safe
      // answer.
      expect(
        () => extractSpkiDer(Uint8List.fromList([0x01, 0x02, 0x03])),
        throwsFormatException,
      );
      expect(() => extractSpkiDer(Uint8List(0)), throwsFormatException);
      // Truncated: a valid SEQUENCE header promising more bytes than exist.
      expect(
        () => extractSpkiDer(Uint8List.fromList([0x30, 0x7f, 0x00])),
        throwsFormatException,
      );
      // Indefinite length, which DER forbids.
      expect(
        () => extractSpkiDer(Uint8List.fromList([0x30, 0x80, 0x00, 0x00])),
        throwsFormatException,
      );
    });

    test('long-form DER lengths are walked correctly', () {
      // A real certificate's tbsCertificate is well over 127 bytes, so its
      // length uses the long form (0x82). A walker that only handles the short
      // form breaks on every real certificate while passing any tiny hand-made
      // one — which is exactly the kind of bug a synthetic test would miss.
      final cert = spkiVectorCertDer;
      expect(cert[0], 0x30);
      expect(
        cert[1] & 0x80,
        isNot(0),
        reason: 'the outer SEQUENCE length must use the long form',
      );
      expect(cert[1], 0x82, reason: 'two length bytes follow');
      expect(
        (cert[2] << 8) | cert[3],
        cert.length - 4,
        reason: 'the declared outer length is the whole certificate',
      );
    });

    test('a truncated certificate is refused, not clamped', () {
      // Trailing bytes do not corrupt the walk — the outer SEQUENCE still ends
      // where it is declared — so this must still produce the same SPKI. What
      // must be refused is a *truncation*: cutting the certificate short leaves
      // a declared length the input cannot satisfy, and returning a partial
      // SPKI would mint a stable pin over the wrong bytes.
      final padded = Uint8List.fromList([
        ...spkiVectorCertDer,
        ...List<int>.filled(64, 0),
      ]);
      expect(extractSpkiDer(padded), extractSpkiDer(spkiVectorCertDer));

      final cut = spkiVectorCertDer.sublist(0, spkiVectorCertDer.length - 8);
      expect(() => extractSpkiDer(cut), throwsFormatException);
    });
  });
}
