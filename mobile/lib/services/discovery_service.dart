import 'dart:async';
import 'package:flutter/foundation.dart';

class DiscoveredDevice {
  final String id;
  final String name;
  final String type;
  final String address;
  final int port;

  DiscoveredDevice({
    required this.id,
    required this.name,
    required this.type,
    required this.address,
    required this.port,
  });
}

class DiscoveryService extends ChangeNotifier {
  final List<DiscoveredDevice> _devices = [];
  Timer? _broadcastTimer;
  bool _isDiscovering = false;

  List<DiscoveredDevice> get devices => List.unmodifiable(_devices);
  bool get isDiscovering => _isDiscovering;

  void startDiscovery() {
    _isDiscovering = true;
    notifyListeners();

    // Start broadcasting presence
    _broadcastTimer = Timer.periodic(
      const Duration(seconds: 5),
      (_) => _broadcastPresence(),
    );
  }

  void _broadcastPresence() {
    // In production, use mDNS/NSD to broadcast
    // For now, this is a placeholder
    debugPrint('Broadcasting Conduit presence...');
  }

  void stopDiscovery() {
    _broadcastTimer?.cancel();
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
