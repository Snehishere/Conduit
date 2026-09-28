import 'package:flutter/material.dart';

class DeviceInfo {
  final String id;
  final String name;
  final String type;
  final String status;
  final int? battery;

  const DeviceInfo({
    required this.id,
    required this.name,
    required this.type,
    required this.status,
    this.battery,
  });

  IconData get typeIcon {
    switch (type) {
      case 'desktop':
      case 'laptop':
        return Icons.desktop_windows_outlined;
      case 'watch':
        return Icons.watch_outlined;
      case 'earbuds':
      case 'headphones':
        return Icons.headphones_outlined;
      case 'tablet':
        return Icons.tablet_mac_outlined;
      case 'phone':
      default:
        return Icons.smartphone_outlined;
    }
  }

  factory DeviceInfo.fromMap(Map<String, dynamic> map) => DeviceInfo(
        id: map['id'] as String? ?? '',
        name: map['name'] as String? ?? 'Unknown',
        type: map['type'] as String? ?? map['device_type'] as String? ?? 'phone',
        status: map['status'] as String? ?? 'paired',
        battery: map['battery'] as int?,
      );
}
