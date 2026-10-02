import 'package:conduit/services/discovery_service.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('DiscoveryService.parseTxtPort', () {
    // The payload is newline-separated `key=value` entries. mdns-sd (Rust) and
    // multicast_dns (Dart) disagree about quoting and about CRLF vs LF, so all
    // three shapes have to work.

    test('reads a bare value', () {
      expect(DiscoveryService.parseTxtPort('wss_port=9531', 'wss_port'), 9531);
    });

    test('reads a quoted value', () {
      expect(DiscoveryService.parseTxtPort('"wss_port=9531"', 'wss_port'), 9531);
    });

    test('reads a value from a multi-entry CRLF record', () {
      const txt = 'device_id=dev-abc\r\ndevice_type=desktop\r\nws_port=9527\r\n'
          'wss_port=9531\r\nversion=0.1.0';
      expect(DiscoveryService.parseTxtPort(txt, 'wss_port'), 9531);
    });

    test('reads a value from a multi-entry LF record', () {
      const txt = 'device_id=dev-abc\ndevice_type=desktop\nwss_port=9531';
      expect(DiscoveryService.parseTxtPort(txt, 'wss_port'), 9531);
    });

    test('reads a quoted entry inside a multi-entry record', () {
      const txt = 'device_id=dev-abc\n"wss_port=9531"\nversion=0.1.0';
      expect(DiscoveryService.parseTxtPort(txt, 'wss_port'), 9531);
    });

    test('tolerates whitespace around the key and the value', () {
      expect(DiscoveryService.parseTxtPort('  wss_port = 9531  ', 'wss_port'), 9531);
    });

    test('returns null for a key the record does not carry', () {
      expect(DiscoveryService.parseTxtPort('ws_port=9527', 'wss_port'), isNull);
    });

    test('returns null for a non-numeric value rather than guessing', () {
      expect(DiscoveryService.parseTxtPort('wss_port=not-a-port', 'wss_port'), isNull);
    });

    test('returns null for out-of-range ports', () {
      expect(DiscoveryService.parseTxtPort('wss_port=0', 'wss_port'), isNull);
      expect(DiscoveryService.parseTxtPort('wss_port=65536', 'wss_port'), isNull);
      expect(DiscoveryService.parseTxtPort('wss_port=-1', 'wss_port'), isNull);
    });

    test('returns null for an empty record and for a bare key', () {
      expect(DiscoveryService.parseTxtPort('', 'wss_port'), isNull);
      expect(DiscoveryService.parseTxtPort('wss_port', 'wss_port'), isNull);
    });

    test('does not match a key that is a suffix of another', () {
      // `port=` must not be found by asking for `wss_port`, or vice versa: the
      // two records are different transports and confusing them silently dials
      // plaintext with a TLS handshake.
      expect(DiscoveryService.parseTxtPort('port=9527', 'wss_port'), isNull);
      expect(DiscoveryService.parseTxtPort('wss_port=9531', 'port'), isNull);
      expect(DiscoveryService.parseTxtPort('x_wss_port=9531', 'wss_port'), isNull);
    });
  });

  group('discovery TXT ↔ Dart port constants', () {
    // `advertised_txt_properties` in apps/desktop/src-tauri/src/discovery.rs
    // publishes crate::WS_PORT and crate::WSS_PORT. If either drifts, pairing
    // stops working and nothing here fails loudly — the mobile app just falls
    // back to kLanWssPort and times out.
    test('the advertised wss_port is the port the app dials', () {
      expect(kLanWssPort, 9531);
    });

    test('the advertised ws_port is the plaintext port and is not dialled', () {
      expect(kLanWsPort, 9527);
      expect(kLanWsPort, isNot(kLanWssPort));
    });

    test('both advertised ports survive a TXT round-trip', () {
      const txt = 'device_id=dev-abc\ndevice_type=desktop\nversion=0.1.0\n'
          'ws_port=$kLanWsPort\nwss_port=$kLanWssPort';
      expect(DiscoveryService.parseTxtPort(txt, 'ws_port'), kLanWsPort);
      expect(DiscoveryService.parseTxtPort(txt, 'wss_port'), kLanWssPort);
    });
  });

  group('DiscoveredDevice defaults', () {
    test('a peer with no advertised TLS port pairs on the shared TLS constant', () {
      final d = DiscoveredDevice(
        id: 'laptop.local.',
        name: 'Laptop',
        type: 'desktop',
        address: '192.168.1.5',
        port: kLanWsPort,
      );
      // Never the plaintext port: the certificate-pin bootstrap only runs on the
      // TLS handshake, so a wss:// dial against the plaintext listener leaves
      // the pin uncaptured and every later connection fails closed.
      expect(d.wssPort, kLanWssPort);
    });

    test('an advertised TLS port is recorded verbatim, never overwritten', () {
      final d = DiscoveredDevice(
        id: 'laptop.local.',
        name: 'Laptop',
        type: 'desktop',
        address: '192.168.1.5',
        port: kLanWsPort,
        wssPort: 19531,
      );
      expect(d.wssPort, 19531);
      expect(d.port, kLanWsPort);
    });
  });
}
