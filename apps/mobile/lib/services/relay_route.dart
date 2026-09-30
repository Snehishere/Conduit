/// Per-device `relay_route` signing, bit-compatible with the Rust relay.
///
/// # Why this exists
///
/// A device signs its relayed messages with a key derived from *its own*
/// pairing secret, not with the relay's bearer token:
///
/// ```text
/// route_key = HMAC-SHA256(
///     key = pairing_secret,
///     msg = "conduit-protocol/v1/derive:conduit-relay/v1/route-key:" + device_id
/// )
/// ```
///
/// A device therefore can sign for itself and for nothing else. The relay
/// resolves the key by looking up the sender the *connection* authenticated as
/// and requires that to match the claimed `from_device_id`, so attribution
/// cannot be rewritten even by a device holding its own valid key.
///
/// The historical alternative — signing with `relay_token` — meant any client
/// that had seen the token could forge a route as any other device, and left
/// `from_device_id` out of the signed fields entirely.
///
/// # Key ordering
///
/// The single most dangerous thing to get wrong here is the canonical string.
/// The relay parses the route into `serde_json::Value` and re-serialises the
/// signed fields, and serde_json is built **without** `preserve_order`, so it
/// sorts every object alphabetically at every depth. A client that hashes its
/// own insertion order therefore produces a different byte string and every
/// route is rejected with `hmac_invalid`.
///
/// [canonicalSigningString] sorts recursively for that reason.
/// `services/relay/src/suite.rs::interop_vector_for_the_dart_client` pins the
/// same constants from the Rust side.
library;

import 'dart:convert';
import 'dart:math' show Random;
import 'dart:typed_data';

import 'package:crypto/crypto.dart' as crypto;

import 'encryption_service.dart';

/// Domain-separation label for a device's route key.
///
/// Must match `conduit_protocol::hmac::ROUTE_KEY_LABEL`.
const String routeKeyLabel = 'conduit-relay/v1/route-key';

/// Fixed prefix binding every derived key to this protocol and KDF version.
const String kdfPrefix = 'conduit-protocol/v1/derive:';

/// The fields covered by a message HMAC, and the only ones.
///
/// Must match `conduit_protocol::hmac::SIGNED_FIELDS`. Present in the struct
/// for documentation and for [hasRequiredSignedFields]; the canonical string
/// sorts them anyway.
const List<String> signedFields = [
  'type',
  'from_device_id',
  'to_device_id',
  'payload',
  'timestamp',
  'nonce',
  'key_id',
];

/// Derive the key this device signs its `relay_route` messages with.
///
/// [pairingSecret] is the X25519 secret established when this device paired
/// with the desktop hub, and [deviceId] must be the id the hub filed it under —
/// the relay looks the key up under exactly that id.
List<int> deriveRouteKey(List<int> pairingSecret, String deviceId) {
  return crypto
      .Hmac(crypto.sha256, pairingSecret)
      .convert(utf8.encode('$kdfPrefix$routeKeyLabel:$deviceId'))
      .bytes;
}

/// Recursively sort every JSON object's keys, matching serde_json's BTreeMap.
Object? _sorted(Object? value) {
  if (value is Map) {
    final keys = value.keys.map((k) => k as String).toList()..sort();
    return <String, Object?>{
      for (final key in keys) key: _sorted(value[key]),
    };
  }
  if (value is List) {
    return value.map(_sorted).toList();
  }
  return value;
}

/// Rebuild the exact byte string a message HMAC is computed over.
///
/// Only [signedFields] participate, and absent fields are simply omitted, which
/// keeps a legacy message verifiable while the relay still refuses to route it.
String canonicalSigningString(Map<String, dynamic> message) {
  final canonical = <String, Object?>{};
  for (final field in signedFields) {
    if (message.containsKey(field)) {
      canonical[field] = message[field];
    }
  }
  // The sort is what makes this interop-correct, not cosmetic: the relay
  // re-serialises through serde_json, which orders keys alphabetically.
  return jsonEncode(_sorted(canonical));
}

/// Hex-encode the HMAC-SHA256 of [message] under [secret].
String signHex(List<int> secret, String message) {
  return EncryptionService.bytesToHex(
    crypto.Hmac(crypto.sha256, secret).convert(utf8.encode(message)).bytes,
  );
}

/// Verify the `hmac` field on a signed message.
///
/// Returns false rather than throwing for every failure, including a malformed
/// hex tag, so a hostile message cannot make the caller crash.
bool verifyMessageHmac(List<int> secret, Map<String, dynamic> message) {
  final tag = message['hmac'];
  if (tag is! String || tag.isEmpty) return false;
  return constantTimeHexEquals(signHex(secret, canonicalSigningString(message)), tag);
}

/// Compare two hex strings without leaking their contents through timing.
bool constantTimeHexEquals(String a, String b) {
  if (a.length != b.length) return false;
  var diff = 0;
  for (var i = 0; i < a.length; i++) {
    diff |= a.codeUnitAt(i) ^ b.codeUnitAt(i);
  }
  return diff == 0;
}

/// Whether [message] carries every field the relay requires before routing.
///
/// The relay rejects a route missing any of these, so a client that skips the
/// check would send a frame guaranteed to be refused.
bool hasRequiredSignedFields(Map<String, dynamic> message) {
  for (final field in const ['from_device_id', 'timestamp', 'nonce']) {
    if (message[field] == null) return false;
  }
  return true;
}

/// A cryptographically random route nonce, hex-encoded.
///
/// The previous scheme hashed `DateTime.microsecondsSinceEpoch`, which is
/// guessable from a rough clock reading and repeated on a device that sent
/// twice in the same microsecond. A replayed nonce is dropped by the relay, so
/// guessing one is a way to knock a device's own messages out.
String newNonce() {
  return EncryptionService.bytesToHex(
    Uint8List.fromList(
      List<int>.generate(16, (_) => Random.secure().nextInt(256)),
    ),
  );
}

/// Build a signed `relay_route` for [payload] to [toDeviceId].
///
/// [routeKey] must be this device's own key — see [deriveRouteKey] — and
/// [fromDeviceId] is the id the hub filed this device under. The two are sent
/// in the same message and both are signed, so a caller cannot point the key at
/// one device while claiming to be another.
Map<String, dynamic> signRoute({
  required List<int> routeKey,
  required String fromDeviceId,
  required String toDeviceId,
  required Object? payload,
  int? timestampMs,
  String? nonce,
}) {
  // Each value is bound once and then used twice — in the message and in the
  // canonical string that is signed. Generating a timestamp or nonce inline at
  // both sites would sign one value and transmit another, and the relay would
  // reject every such route with `hmac_invalid` for no visible reason.
  final timestamp = timestampMs ?? DateTime.now().millisecondsSinceEpoch;
  final routeNonce = nonce ?? newNonce();

  final unsigned = <String, dynamic>{
    'type': 'relay_route',
    'from_device_id': fromDeviceId,
    'to_device_id': toDeviceId,
    'payload': payload,
    'timestamp': timestamp,
    'nonce': routeNonce,
    'key_id': fromDeviceId,
  };

  return {...unsigned, 'hmac': signHex(routeKey, canonicalSigningString(unsigned))};
}

// ---------------------------------------------------------------------------
//  v2 binary frames
//
//  Layout, matching `conduit_protocol`:
//    [0]      version, always 0x02
//    [1..17]  target device id, 16 bytes, zero-padded
//    [17..21] sequence, big-endian u32
//    [21..53] HMAC-SHA256 tag, 32 raw bytes
//    [53..]   payload
//
//  The tag covers `from_device_id || 0x1f || frame[..21] || payload`, and it
//  is computed over the **hex encoding** of that byte string, not the bytes
//  themselves. That is what the relay does; matching it is the whole point.
//  v1 frames (version 0x01, no tag, no sequence) are no longer accepted.
// ---------------------------------------------------------------------------

/// Frame version byte. v1 is rejected by the relay.
const int binaryFrameVersion = 0x02;

/// Width of the target device id field.
const int binaryDeviceIdLen = 16;

/// Width of the sequence field.
const int binarySeqLen = 4;

/// Width of the HMAC tag field, in raw bytes.
const int binaryTagLen = 32;

/// Offset of the tag field.
const int binaryTagOffset = 1 + binaryDeviceIdLen + binarySeqLen; // 21

/// Offset of the payload, i.e. the size of the fixed header.
const int binaryHeaderLen = binaryTagOffset + binaryTagLen; // 53

/// Separator between the sender id and the frame in the MAC input.
const int binaryMacSeparator = 0x1f;

/// The exact byte string a binary frame's tag is computed over.
Uint8List binaryMacInput(String fromDeviceId, List<int> headerAndPayload) {
  final idBytes = utf8.encode(fromDeviceId);
  final input = Uint8List(idBytes.length + 1 + headerAndPayload.length)
    ..setRange(0, idBytes.length, idBytes)
    ..[idBytes.length] = binaryMacSeparator
    ..setRange(idBytes.length + 1, idBytes.length + 1 + headerAndPayload.length, headerAndPayload);
  return input;
}

/// Build a v2 binary frame addressed to [targetDeviceId].
///
/// [routeKey] is this device's own key and [fromDeviceId] is its id: the
/// sender identity is inside the MAC input, so a frame captured from another
/// device cannot be replayed on this connection.
Uint8List buildBinaryFrame({
  required List<int> routeKey,
  required String fromDeviceId,
  required String targetDeviceId,
  required Uint32List sequence,
  required List<int> payload,
}) {
  if (sequence.length != 1) {
    throw ArgumentError('sequence must be a single 32-bit value');
  }

  final frame = Uint8List(binaryHeaderLen + payload.length);
  frame[0] = binaryFrameVersion;

  // The 16-byte field truncates a longer id. A real device id fits, and a
  // truncated id simply will not resolve on the far side.
  final targetBytes = utf8.encode(targetDeviceId);
  final n = targetBytes.length < binaryDeviceIdLen ? targetBytes.length : binaryDeviceIdLen;
  frame.setRange(1, 1 + n, targetBytes.sublist(0, n));

  final seqBytes = ByteData(4)..setUint32(0, sequence[0], Endian.big);
  frame.setRange(1 + binaryDeviceIdLen, 1 + binaryDeviceIdLen + binarySeqLen, seqBytes.buffer.asUint8List());

  final authenticated = Uint8List(binaryTagOffset + payload.length)
    ..setRange(0, binaryTagOffset, frame)
    ..setRange(binaryTagOffset, binaryTagOffset + payload.length, payload);
  final macInput = binaryMacInput(fromDeviceId, authenticated);
  final tag = crypto.Hmac(crypto.sha256, routeKey).convert(utf8.encode(hexEncode(macInput))).bytes;
  frame.setRange(binaryTagOffset, binaryTagOffset + binaryTagLen, tag);
  frame.setRange(binaryHeaderLen, binaryHeaderLen + payload.length, payload);
  return frame;
}

/// A parsed, not-yet-verified v2 binary frame.
class BinaryFrame {
  const BinaryFrame({
    required this.targetId,
    required this.sequence,
    required this.payload,
    required this.tag,
  });

  /// The recipient device id, zero-padding stripped.
  final String targetId;

  /// The per-connection frame counter, for replay detection.
  final int sequence;

  /// The frame body.
  final Uint8List payload;

  /// The raw 32-byte tag, before verification.
  final Uint8List tag;
}

/// Parse a v2 binary frame's structure without verifying the tag.
///
/// Throws [FormatException] for anything malformed; a caller that needs to
/// survive hostile input should catch it. Verification is separate on purpose
/// so a frame can be structurally parsed for logging without its tag being
/// implicitly trusted.
BinaryFrame parseBinaryFrame(Uint8List frame) {
  if (frame.length < binaryHeaderLen) {
    throw FormatException(
      'binary frame too short: ${frame.length} bytes, need at least $binaryHeaderLen',
    );
  }
  if (frame[0] != binaryFrameVersion) {
    throw FormatException('unsupported binary frame version ${frame[0]}');
  }

  final idField = frame.sublist(1, 1 + binaryDeviceIdLen);
  // NUL is the padding, not part of the id.
  var end = idField.length;
  while (end > 0 && idField[end - 1] == 0) {
    end--;
  }
  final targetId = utf8.decode(idField.sublist(0, end), allowMalformed: true);

  final sequence = ByteData.sublistView(
    frame,
    1 + binaryDeviceIdLen,
    1 + binaryDeviceIdLen + binarySeqLen,
  ).getUint32(0, Endian.big);

  return BinaryFrame(
    targetId: targetId,
    sequence: sequence,
    tag: Uint8List.fromList(frame.sublist(binaryTagOffset, binaryTagOffset + binaryTagLen)),
    payload: Uint8List.fromList(frame.sublist(binaryHeaderLen)),
  );
}

/// Verify [frame]'s tag against [routeKey] for the sender [fromDeviceId].
bool verifyBinaryFrame({
  required List<int> routeKey,
  required String fromDeviceId,
  required Uint8List frame,
}) {
  final parsed = parseBinaryFrame(frame);
  final authenticated = Uint8List(binaryTagOffset + parsed.payload.length)
    ..setRange(0, binaryTagOffset, frame)
    ..setRange(binaryTagOffset, binaryTagOffset + parsed.payload.length, parsed.payload);
  final expected = crypto
      .Hmac(crypto.sha256, routeKey)
      .convert(utf8.encode(hexEncode(binaryMacInput(fromDeviceId, authenticated))))
      .bytes;
  return constantTimeBytesEquals(expected, parsed.tag);
}

/// Length-independent, content-constant-time byte comparison.
bool constantTimeBytesEquals(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  var diff = 0;
  for (var i = 0; i < a.length; i++) {
    diff |= a[i] ^ b[i];
  }
  return diff == 0;
}

/// Lowercase hex, matching the Rust side's `hex::encode`.
String hexEncode(List<int> bytes) {
  final buffer = StringBuffer();
  for (final byte in bytes) {
    buffer.write(byte.toRadixString(16).padLeft(2, '0'));
  }
  return buffer.toString();
}
