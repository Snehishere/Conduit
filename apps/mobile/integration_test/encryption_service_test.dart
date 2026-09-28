import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:conduit/services/encryption_service.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  // ── X25519 Key Generation ────────────────────────────────────────

  group('X25519 key generation', () {
    testWidgets('generates a valid 32-byte public key', (tester) async {
      final service = EncryptionService();
      await service.initialize();

      final publicKeyHex = service.publicKeyHex;
      expect(publicKeyHex.length, 64); // 32 bytes = 64 hex chars
      // Should be valid hex
      expect(RegExp(r'^[0-9a-f]{64}$').hasMatch(publicKeyHex), isTrue);
    });

    testWidgets('generates deterministic keys from stored seed', (tester) async {
      final service1 = EncryptionService();
      await service1.initialize();
      final key1 = service1.publicKeyHex;

      // Second instance should restore from secure storage
      final service2 = EncryptionService();
      await service2.initialize();
      final key2 = service2.publicKeyHex;

      expect(key1, equals(key2));
    });

    testWidgets('initialize is idempotent', (tester) async {
      final service = EncryptionService();
      await service.initialize();
      final key1 = service.publicKeyHex;

      await service.initialize(); // second call
      final key2 = service.publicKeyHex;

      expect(key1, equals(key2));
    });
  });

  // ── Shared Secret Derivation ─────────────────────────────────────

  group('X25519 shared secret derivation', () {
    testWidgets('two services derive the same shared secret', (tester) async {
      final alice = EncryptionService();
      await alice.initialize();

      final bob = EncryptionService();
      await bob.initialize();

      // Alice derives secret using Bob's public key
      final secretFromAlice = await alice.deriveSharedSecret(bob.publicKeyHex);

      // Bob derives secret using Alice's public key
      final secretFromBob = await bob.deriveSharedSecret(alice.publicKeyHex);

      // Both should produce the same shared secret
      expect(secretFromAlice, equals(secretFromBob));
      expect(secretFromAlice.length, 32); // X25519 produces 32-byte secrets
    });

    testWidgets('shared secret is consistent across calls', (tester) async {
      final alice = EncryptionService();
      await alice.initialize();
      final bob = EncryptionService();
      await bob.initialize();

      final secret1 = await alice.deriveSharedSecret(bob.publicKeyHex);
      final secret2 = await alice.deriveSharedSecret(bob.publicKeyHex);

      expect(secret1, equals(secret2));
    });
  });

  // ── XChaCha20-Poly1305 Encrypt / Decrypt ─────────────────────────

  group('Encrypt and decrypt', () {
    late List<int> sharedSecret;

    setUp(() async {
      final alice = EncryptionService();
      await alice.initialize();
      final bob = EncryptionService();
      await bob.initialize();
      sharedSecret = await alice.deriveSharedSecret(bob.publicKeyHex);
    });

    testWidgets('encrypt then decrypt returns original plaintext', (tester) async {
      final service = EncryptionService();
      const plaintext = 'Hello, Conduit! This is a secret message.';

      final (nonce, ciphertext) = await service.encrypt(sharedSecret, plaintext);
      final decrypted = await service.decrypt(sharedSecret, nonce, ciphertext);

      expect(decrypted, equals(plaintext));
    });

    testWidgets('encrypt produces different ciphertext each time (random nonce)', (tester) async {
      final service = EncryptionService();
      const plaintext = 'Same message twice';

      final (nonce1, cipher1) = await service.encrypt(sharedSecret, plaintext);
      final (nonce2, cipher2) = await service.encrypt(sharedSecret, plaintext);

      // Different nonces mean different ciphertext
      expect(nonce1, isNot(equals(nonce2)));
      expect(cipher1, isNot(equals(cipher2)));
    });

    testWidgets('handles empty plaintext', (tester) async {
      final service = EncryptionService();
      final (nonce, ciphertext) = await service.encrypt(sharedSecret, '');
      final decrypted = await service.decrypt(sharedSecret, nonce, ciphertext);
      expect(decrypted, isEmpty);
    });

    testWidgets('handles large plaintext', (tester) async {
      final service = EncryptionService();
      final plaintext = 'A' * 10000; // 10KB

      final (nonce, ciphertext) = await service.encrypt(sharedSecret, plaintext);
      final decrypted = await service.decrypt(sharedSecret, nonce, ciphertext);

      expect(decrypted, equals(plaintext));
      expect(decrypted.length, 10000);
    });

    testWidgets('handles unicode plaintext', (tester) async {
      final service = EncryptionService();
      const plaintext = '你好世界 🌍 café ñ';

      final (nonce, ciphertext) = await service.encrypt(sharedSecret, plaintext);
      final decrypted = await service.decrypt(sharedSecret, nonce, ciphertext);

      expect(decrypted, equals(plaintext));
    });

    testWidgets('wrong shared secret fails decryption', (tester) async {
      final service = EncryptionService();
      const plaintext = 'Confidential data';

      final (nonce, ciphertext) = await service.encrypt(sharedSecret, plaintext);

      // Create a different shared secret (wrong key)
      final wrongAlice = EncryptionService();
      await wrongAlice.initialize();
      final wrongBob = EncryptionService();
      await wrongBob.initialize();
      final wrongSecret = await wrongAlice.deriveSharedSecret(wrongBob.publicKeyHex);

      // Decryption with wrong secret should throw
      expect(
        () => service.decrypt(wrongSecret, nonce, ciphertext),
        throwsA(anything),
      );
    });

    testWidgets('tampered ciphertext fails decryption', (tester) async {
      final service = EncryptionService();
      const plaintext = 'Tamper test';

      final (nonce, ciphertext) = await service.encrypt(sharedSecret, plaintext);

      // Tamper with the ciphertext
      final tampered = List<int>.from(ciphertext);
      tampered[0] ^= 0xFF; // flip bits in first byte

      expect(
        () => service.decrypt(sharedSecret, nonce, tampered),
        throwsA(anything),
      );
    });

    testWidgets('truncated ciphertext (missing MAC) fails', (tester) async {
      final service = EncryptionService();
      const plaintext = 'MAC check';

      final (nonce, ciphertext) = await service.encrypt(sharedSecret, plaintext);

      // Remove the last 16 bytes (the MAC tag)
      final truncated = ciphertext.sublist(0, ciphertext.length - 16);

      expect(
        () => service.decrypt(sharedSecret, nonce, truncated),
        throwsA(anything),
      );
    });
  });

  // ── Binary Encrypt / Decrypt ─────────────────────────────────────

  group('Binary encrypt and decrypt', () {
    late List<int> sharedSecret;

    setUp(() async {
      final alice = EncryptionService();
      await alice.initialize();
      final bob = EncryptionService();
      await bob.initialize();
      sharedSecret = await alice.deriveSharedSecret(bob.publicKeyHex);
    });

    testWidgets('encrypts and decrypts raw bytes', (tester) async {
      final service = EncryptionService();
      final plainBytes = [0, 1, 2, 3, 255, 128, 64, 32];

      final (nonce, ciphertext) = await service.encryptBinary(sharedSecret, plainBytes);
      final decrypted = await service.decryptBinary(sharedSecret, nonce, ciphertext);

      expect(decrypted, equals(plainBytes));
    });

    testWidgets('handles binary data with all byte values', (tester) async {
      final service = EncryptionService();
      final plainBytes = List<int>.generate(256, (i) => i);

      final (nonce, ciphertext) = await service.encryptBinary(sharedSecret, plainBytes);
      final decrypted = await service.decryptBinary(sharedSecret, nonce, ciphertext);

      expect(decrypted, equals(plainBytes));
    });
  });

  // ── HMAC-SHA256 ──────────────────────────────────────────────────

  group('HMAC-SHA256', () {
    testWidgets('generateHmac produces valid hex string', (tester) async {
      final secret = List<int>.generate(32, (i) => i);
      const message = 'test message for hmac';

      final hmac = EncryptionService.generateHmac(secret, message);

      expect(hmac.length, 64); // SHA-256 = 32 bytes = 64 hex chars
      expect(RegExp(r'^[0-9a-f]{64}$').hasMatch(hmac), isTrue);
    });

    testWidgets('verifyHmac returns true for matching HMAC', (tester) async {
      final secret = List<int>.generate(32, (i) => i);
      const message = 'authenticate me';

      final hmac = EncryptionService.generateHmac(secret, message);
      final valid = EncryptionService.verifyHmac(secret, message, hmac);

      expect(valid, isTrue);
    });

    testWidgets('verifyHmac returns false for wrong message', (tester) async {
      final secret = List<int>.generate(32, (i) => i);

      final hmac = EncryptionService.generateHmac(secret, 'correct message');
      final valid = EncryptionService.verifyHmac(secret, 'wrong message', hmac);

      expect(valid, isFalse);
    });

    testWidgets('verifyHmac returns false for wrong secret', (tester) async {
      final secret1 = List<int>.generate(32, (i) => i);
      final secret2 = List<int>.generate(32, (i) => i + 100);
      const message = 'shared message';

      final hmac = EncryptionService.generateHmac(secret1, message);
      final valid = EncryptionService.verifyHmac(secret2, message, hmac);

      expect(valid, isFalse);
    });

    testWidgets('verifyHmac returns false for wrong HMAC', (tester) async {
      final secret = List<int>.generate(32, (i) => i);
      const message = 'verify me';

      final fakeHmac = '0' * 64; // all zeros
      final valid = EncryptionService.verifyHmac(secret, message, fakeHmac);

      expect(valid, isFalse);
    });

    testWidgets('HMAC is deterministic for same inputs', (tester) async {
      final secret = List<int>.generate(32, (i) => i);
      const message = 'deterministic test';

      final hmac1 = EncryptionService.generateHmac(secret, message);
      final hmac2 = EncryptionService.generateHmac(secret, message);

      expect(hmac1, equals(hmac2));
    });
  });

  // ── Hex Conversion Utilities ─────────────────────────────────────

  group('Hex conversion utilities', () {
    testWidgets('bytesToHex and hexToBytes are inverse operations', (tester) async {
      final original = [0, 1, 15, 16, 127, 128, 255, 0];

      final hex = EncryptionService.bytesToHex(original);
      final restored = EncryptionService.hexToBytes(hex);

      expect(restored, equals(original));
    });

    testWidgets('bytesToHex produces lowercase hex', (tester) async {
      final bytes = [255, 171, 205];
      final hex = EncryptionService.bytesToHex(bytes);

      expect(hex, 'ffabcd');
    });

    testWidgets('hexToBytes handles empty string', (tester) async {
      final bytes = EncryptionService.hexToBytes('');
      expect(bytes, isEmpty);
    });

    testWidgets('hexToBytes handles uppercase hex', (tester) async {
      final bytes = EncryptionService.hexToBytes('FFABCD');
      expect(bytes, [255, 171, 205]);
    });
  });
}
