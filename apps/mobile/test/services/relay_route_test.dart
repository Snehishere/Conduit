import 'dart:convert';
import 'dart:typed_data';

import 'package:conduit/services/encryption_service.dart';
import 'package:conduit/services/relay_route.dart';
import 'package:flutter_test/flutter_test.dart';

/// The four constants below are produced by the Rust relay and pinned there by
/// `services/relay/src/suite.rs::interop_vector_for_the_dart_client`. If this
/// file fails, that one will fail too — re-emit with
/// `cargo test -p conduit-relay interop_vector -- --nocapture` and update both.
const String vectorRouteKey = '2cd721ceb5f78ef0f105593aa6feb7b5bc6f17c323d9ad32d6a34a166b770914';
const String vectorCanonical = '{"from_device_id":"deadbeef","key_id":"deadbeef","nonce":"nonce-1",'
    '"payload":{"alpha":2,"nested":{"a":1,"b":2},"type":"ping","zeta":1},'
    '"timestamp":1700000000000,"to_device_id":"cafebabe","type":"relay_route"}';
const String vectorHmac = '9c2018aa54f8a9fe7fa674899eee2665d891e7f34ee38077effc541132796dcb';
const String vectorSecret = '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f';

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
      expect(signHex(deriveRouteKey(_secret(), 'deadbeef'), vectorCanonical), vectorHmac);
    });

    test('verifies a route the Rust side signed', () {
      final message = _vectorMessage()..['hmac'] = vectorHmac;
      expect(verifyMessageHmac(deriveRouteKey(_secret(), 'deadbeef'), message), isTrue);
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

  group('route key derivation', () {
    test('binds the device id, so one secret gives unrelated keys', () {
      expect(deriveRouteKey(_secret(), 'aaa'), isNot(equals(deriveRouteKey(_secret(), 'bbb'))));
    });

    test('is deterministic', () {
      expect(deriveRouteKey(_secret(), 'aaa'), deriveRouteKey(_secret(), 'aaa'));
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
          'payload': [3, 1, 2]
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
      expect(verifyMessageHmac(deriveRouteKey(_secret(), 'ccc'), route), isFalse);
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
      expect(nonces.length, 200, reason: 'a repeated nonce is a dropped message');
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

    test('the sender id is inside the MAC input, so a frame cannot be replayed as another sender', () {
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
    });

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
}
