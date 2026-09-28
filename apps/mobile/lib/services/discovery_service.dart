import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:multicast_dns/multicast_dns.dart';
import 'websocket_service.dart';

/// A Conduit peer found on the local network.
class DiscoveredDevice {
  final String id;
  final String name;
  final String type;
  final String address;

  /// The port from the peer's mDNS **SRV** record.
  ///
  /// For a Conduit desktop this is the *plaintext* listener. It is recorded
  /// verbatim as advertised — it is never overwritten with a guess.
  final int port;

  /// The peer's **TLS** port, from the mDNS TXT record's `wss_port` property.
  ///
  /// This is the port the app must dial: pairing and every subsequent
  /// connection are `wss://`, because the certificate-pin trust bootstrap only
  /// runs on the TLS path. When a peer advertises no `wss_port` the fallback is
  /// the shared [kLanWssPort] — never [kLanWsPort].
  final int wssPort;

  DiscoveredDevice({
    required this.id,
    required this.name,
    required this.type,
    required this.address,
    required this.port,
    this.wssPort = kLanWssPort,
  });
}

class DiscoveryService extends ChangeNotifier {
  final List<DiscoveredDevice> _devices = [];
  final MDnsClient _client = MDnsClient();
  bool _isDiscovering = false;

  /// How long to wait for a peer's TXT record before falling back to
  /// [kLanWssPort]. Kept short so a peer without TXT support still appears.
  static const Duration _txtLookupTimeout = Duration(milliseconds: 800);

  List<DiscoveredDevice> get devices => List.unmodifiable(_devices);
  bool get isDiscovering => _isDiscovering;

  Future<void> startDiscovery() async {
    if (_isDiscovering) return;
    _isDiscovering = true;
    _devices.clear();
    notifyListeners();

    try {
      await _client.start();
      _listenForServices();
    } catch (e) {
      debugPrint('mDNS start failed: $e');
      _isDiscovering = false;
      notifyListeners();
    }
  }

  Future<void> _listenForServices() async {
    try {
      const String name = '_conduit._tcp.local';
      await for (final PtrResourceRecord ptr in _client
          .lookup<PtrResourceRecord>(ResourceRecordQuery.serverPointer(name))) {
        await for (final SrvResourceRecord srv in _client
            .lookup<SrvResourceRecord>(ResourceRecordQuery.service(ptr.domainName))) {
          // Resolve the peer's TLS port from its TXT properties. Done once per
          // service, before the address loop, so it is not re-queried for each
          // interface address.
          final wssPort = await _resolveWssPort(ptr.domainName);

          await for (final IPAddressResourceRecord ip in _client
              .lookup<IPAddressResourceRecord>(ResourceRecordQuery.addressIPv4(srv.target))) {
            final address = ip.address.address.isNotEmpty
                ? ip.address.address
                : ip.address.host;
            if (address.isEmpty) continue;

            final device = DiscoveredDevice(
              id: srv.target,
              name: ptr.domainName.split('.').first,
              type: 'desktop',
              address: address,
              // The SRV port is authoritative. Do NOT force it to a literal:
              // doing so discarded whatever the peer actually advertised and,
              // because callers dial TLS, pointed them at the plaintext
              // listener. A peer advertising only the plaintext port still
              // gets paired over [wssPort].
              port: srv.port,
              wssPort: wssPort,
            );
            deviceFound(device);
          }
        }
      }
    } catch (e) {
      debugPrint('mDNS lookup failed: $e');
    }
  }

  /// Read the `wss_port` TXT property advertised by [serviceName].
  ///
  /// Falls back to the shared TLS constant when the record is absent, empty,
  /// unparseable, or the lookup times out. It never falls back to the
  /// plaintext WS port: a `wss://` handshake against a plaintext socket
  /// throws, and a silent `ws://` downgrade would defeat certificate pinning.
  Future<int> _resolveWssPort(String serviceName) async {
    try {
      final txt = await _client
          .lookup<TxtResourceRecord>(ResourceRecordQuery.text(serviceName))
          .first
          .timeout(_txtLookupTimeout);
      final advertised = parseTxtPort(txt.text, 'wss_port');
      if (advertised != null) return advertised;
      debugPrint(
          'mDNS: $serviceName advertises no usable wss_port; using $kLanWssPort');
    } catch (e) {
      debugPrint(
          'mDNS: no TXT record for $serviceName ($e); using $kLanWssPort');
    }
    return kLanWssPort;
  }

  /// Extract `key` from a raw mDNS TXT payload, e.g. `wss_port=9531`.
  ///
  /// The payload is newline-separated `key=value` entries and some responders
  /// quote each entry, so quotes and whitespace are stripped. Returns null for
  /// a missing key or a non-numeric value.
  static int? parseTxtPort(String text, String key) {
    for (final rawEntry in text.split(RegExp(r'[\r\n]'))) {
      var entry = rawEntry.trim();
      if (entry.length < 2) continue;
      // Strip a surrounding pair of quotes, if present.
      if (entry.startsWith('"') && entry.endsWith('"')) {
        entry = entry.substring(1, entry.length - 1);
      }
      final separator = entry.indexOf('=');
      if (separator <= 0) continue;
      if (entry.substring(0, separator).trim() != key) continue;
      final port = int.tryParse(entry.substring(separator + 1).trim());
      if (port == null || port <= 0 || port > 65535) {
        debugPrint('mDNS: ignoring out-of-range $key value in TXT record');
        return null;
      }
      return port;
    }
    return null;
  }

  void stopDiscovery() {
    _client.stop();
    _isDiscovering = false;
    notifyListeners();
  }

  void deviceFound(DiscoveredDevice device) {
    if (!_devices.any((d) => d.id == device.id)) {
      _devices.add(device);
      notifyListeners();
    }
  }

  void deviceLost(String deviceId) {
    _devices.removeWhere((d) => d.id == deviceId);
    notifyListeners();
  }

  @override
  void dispose() {
    stopDiscovery();
    super.dispose();
  }
}
