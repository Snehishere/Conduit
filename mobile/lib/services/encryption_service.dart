import 'dart:convert';
import 'package:cryptography/cryptography.dart';

/// X25519 key exchange + XChaCha20-Poly1305 encryption.
/// Matches desktop EncryptionManager (x25519-dalek + chacha20poly1305).
class EncryptionService {
  static final _x25519 = Cryptography.instance.x25519();
  static final _xchacha = Cryptography.instance.xchacha20Poly1305Aead();

  SimplePublicKey? _publicKey;
  SimpleKeyPair? _keyPair;
  bool _initialized = false;

  String get publicKeyHex {
    final key = _publicKey;
    if (key == null) return '';
    return bytesToHex(key.bytes);
  }

  Future<void> initialize() async {
    if (_initialized) return;
    _keyPair = await _x25519.newKeyPair();
    _publicKey = await _keyPair!.extractPublicKey();
    _initialized = true;
  }

  /// Derive X25519 shared secret from our private key and peer's public key (hex).
  Future<List<int>> deriveSharedSecret(String peerPublicKeyHex) async {
    final peerBytes = hexToBytes(peerPublicKeyHex);
    final peerPublicKey = SimplePublicKey(peerBytes, type: KeyPairType.x25519);

    final sharedSecret = await _x25519.sharedSecretKey(
      keyPair: _keyPair!,
      remotePublicKey: peerPublicKey,
    );
    return sharedSecret.extractBytes();
  }

  /// Encrypt plaintext with XChaCha20-Poly1305 using the shared secret.
  /// Returns (nonce, combined ciphertext+mac) both as byte lists.
  /// The MAC is appended to the ciphertext so decrypt() can verify authenticity.
  Future<(List<int>, List<int>)> encrypt(
    List<int> sharedSecret,
    String plaintext,
  ) async {
    final secretBox = await _xchacha.encrypt(
      utf8.encode(plaintext),
      secretKey: SecretKey(sharedSecret),
    );
    // Concatenate ciphertext + mac.tag so the MAC is preserved on the wire.
    final combined = [...secretBox.cipherText, ...secretBox.mac.bytes];
    return (secretBox.nonce, combined);
  }

  /// Decrypt ciphertext with XChaCha20-Poly1305.
  /// Expects combined ciphertext+mac (last 16 bytes are the Poly1305 tag).
  Future<String> decrypt(
    List<int> sharedSecret,
    List<int> nonce,
    List<int> ciphertext,
  ) async {
    if (ciphertext.length < 16) {
      throw ArgumentError('Ciphertext too short: missing MAC');
    }
    final tag = ciphertext.sublist(ciphertext.length - 16);
    final cipherOnly = ciphertext.sublist(0, ciphertext.length - 16);
    final secretBox = SecretBox(cipherOnly, nonce: nonce, mac: Mac(tag));
    final plainBytes = await _xchacha.decrypt(
      secretBox,
      secretKey: SecretKey(sharedSecret),
    );
    return utf8.decode(plainBytes);
  }

  static String bytesToHex(List<int> bytes) {
    return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
  }

  static List<int> hexToBytes(String hex) {
    final result = <int>[];
    for (var i = 0; i < hex.length; i += 2) {
      result.add(int.parse(hex.substring(i, i + 2), radix: 16));
    }
    return result;
  }
}
