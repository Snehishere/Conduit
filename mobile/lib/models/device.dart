enum DeviceType { desktop, phone, tablet, earbuds, headphones, watch, tv }
enum DeviceOS { windows, macos, linux, android, ios, wearos }
enum SignalStrength { strong, medium, weak, none }
enum ConnectionStatus { connected, paired, offline }

class Device {
  final String id;
  final String name;
  final DeviceType type;
  final DeviceOS os;
  final int? battery;
  final SignalStrength? signal;
  final ConnectionStatus status;
  final int lastSeen;

  Device({
    required this.id,
    required this.name,
    required this.type,
    required this.os,
    this.battery,
    this.signal,
    required this.status,
    required this.lastSeen,
  });

  factory Device.fromJson(Map<String, dynamic> json) {
    return Device(
      id: (json['id'] as String?) ?? '',
      name: (json['name'] as String?) ?? 'Unknown',
      type: DeviceType.values.firstWhere(
        (e) => e.name == json['type'],
        orElse: () => DeviceType.phone,
      ),
      os: DeviceOS.values.firstWhere(
        (e) => e.name == json['os'],
        orElse: () => DeviceOS.android,
      ),
      battery: (json['battery'] as num?)?.toInt(),
      signal: json['signal'] != null
          ? SignalStrength.values.firstWhere(
              (e) => e.name == json['signal'],
              orElse: () => SignalStrength.none,
            )
          : null,
      status: ConnectionStatus.values.firstWhere(
        (e) => e.name == json['status'],
        orElse: () => ConnectionStatus.offline,
      ),
      lastSeen: (json['last_seen'] as num?)?.toInt() ?? 0,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'name': name,
      'type': type.name,
      'os': os.name,
      'battery': battery,
      'signal': signal?.name,
      'status': status.name,
      'last_seen': lastSeen,
    };
  }

  String get typeIcon {
    switch (type) {
      case DeviceType.desktop:
        return '💻';
      case DeviceType.phone:
        return '📱';
      case DeviceType.tablet:
        return '📱';
      case DeviceType.earbuds:
        return '🎧';
      case DeviceType.headphones:
        return '🎧';
      case DeviceType.watch:
        return '⌚';
      case DeviceType.tv:
        return '📺';
    }
  }
}
