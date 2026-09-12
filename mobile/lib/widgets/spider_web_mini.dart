import 'dart:math';
import 'package:flutter/material.dart';

class DeviceInfo {
  final String id;
  final String name;
  final String type;
  final String status;
  final int? battery;

  DeviceInfo({
    required this.id,
    required this.name,
    required this.type,
    required this.status,
    this.battery,
  });

  IconData get typeIcon {
    switch (type) {
      case 'desktop': return Icons.desktop_windows_outlined;
      case 'phone': return Icons.smartphone_outlined;
      case 'tablet': return Icons.tablet_mac_outlined;
      case 'earbuds': return Icons.headphones_outlined;
      case 'headphones': return Icons.headphones_outlined;
      case 'watch': return Icons.watch_outlined;
      case 'tv': return Icons.tv_outlined;
      default: return Icons.smartphone_outlined;
    }
  }

  Color get nodeColor {
    switch (type) {
      case 'desktop': return const Color(0xFF2DD4BF);
      case 'phone': return const Color(0xFF2DD4BF);
      case 'earbuds': return const Color(0xFF2DD4BF);
      case 'headphones': return const Color(0xFF2DD4BF);
      case 'watch': return const Color(0xFF2DD4BF);
      case 'tv': return const Color(0xFF2DD4BF);
      default: return const Color(0xFF2DD4BF);
    }
  }

  factory DeviceInfo.fromMap(Map<String, dynamic> map) {
    return DeviceInfo(
      id: map['id'] as String? ?? '',
      name: map['name'] as String? ?? 'Unknown',
      type: map['type'] as String? ?? map['device_type'] as String? ?? 'phone',
      status: map['status'] as String? ?? 'paired',
      battery: map['battery'] as int?,
    );
  }
}

class SpiderWebMini extends StatelessWidget {
  final List<DeviceInfo> devices;
  final String? selectedDeviceId;

  const SpiderWebMini({
    super.key,
    this.devices = const [],
    this.selectedDeviceId,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Connected Devices', style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold, color: Color(0xFFE2E0E8))),
          const SizedBox(height: 8),
          Text(
            devices.isEmpty ? 'No devices connected' : '${devices.length} device(s)',
            style: const TextStyle(color: Color(0xFF9A96A4), fontSize: 13),
          ),
          const SizedBox(height: 16),
          Expanded(
            child: devices.isEmpty
                ? const Center(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(Icons.hub_outlined, size: 64, color: Color(0xFF2A2A36)),
                        SizedBox(height: 16),
                        Text('No devices paired', style: TextStyle(color: Color(0xFF9A96A4))),
                        SizedBox(height: 8),
                        Text('Tap the + button to pair a device', style: TextStyle(color: Color(0xFF5C586A), fontSize: 12)),
                      ],
                    ),
                  )
                : CustomPaint(
                    painter: SpiderWebPainter(devices: devices),
                    child: const Center(),
                  ),
          ),
          const SizedBox(height: 16),
          if (devices.isNotEmpty) _buildDeviceList(),
        ],
      ),
    );
  }

  Widget _buildDeviceList() {
    return Column(
      children: devices.map((device) => ListTile(
        dense: true,
        leading: Container(
          width: 36,
          height: 36,
          decoration: BoxDecoration(
            color: const Color(0xFF16161E),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: const Color(0xFF1E1E28)),
          ),
          child: Icon(device.typeIcon, size: 18, color: const Color(0xFF2DD4BF)),
        ),
        title: Text(device.name, style: const TextStyle(fontWeight: FontWeight.w500, color: Color(0xFFE2E0E8))),
        subtitle: Text(
          device.battery != null ? 'Battery: ${device.battery}%' : device.type,
          style: const TextStyle(color: Color(0xFF9A96A4), fontSize: 12),
        ),
        trailing: Container(
          width: 10, height: 10,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: device.status == 'connected' ? const Color(0xFF2DD4BF) : const Color(0xFF5C586A),
            boxShadow: device.status == 'connected'
                ? [BoxShadow(color: const Color(0xFF2DD4BF).withValues(alpha: 0.4), blurRadius: 6)]
                : null,
          ),
        ),
      )).toList(),
    );
  }
}

class SpiderWebPainter extends CustomPainter {
  final List<DeviceInfo> devices;

  SpiderWebPainter({required this.devices});

  @override
  void paint(Canvas canvas, Size size) {
    final center = Offset(size.width / 2, size.height / 2);
    final radius = min(size.width, size.height) * 0.35;

    // Hub (this device - phone)
    final hubPaint = Paint()..color = const Color(0xFF16161E);
    canvas.drawCircle(center, 25, hubPaint);
    final hubStroke = Paint()
      ..color = const Color(0xFF2DD4BF).withValues(alpha: 0.5)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    canvas.drawCircle(center, 25, hubStroke);
    _drawIcon(canvas, center, Icons.smartphone_outlined, 18, const Color(0xFF2DD4BF));

    // Connected devices in circle
    if (devices.isEmpty) return;

    for (var i = 0; i < devices.length; i++) {
      final angle = (2 * pi * i / devices.length) - pi / 2;
      final deviceCenter = Offset(
        center.dx + radius * cos(angle),
        center.dy + radius * sin(angle),
      );

      // Thread line
      final threadPaint = Paint()
        ..color = const Color(0xFF2DD4BF).withValues(alpha: 0.15)
        ..strokeWidth = 1.5;
      canvas.drawLine(center, deviceCenter, threadPaint);

      // Device node
      final nodePaint = Paint()..color = const Color(0xFF16161E);
      canvas.drawCircle(deviceCenter, 18, nodePaint);
      final nodeStroke = Paint()
        ..color = const Color(0xFF2DD4BF).withValues(alpha: 0.4)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5;
      canvas.drawCircle(deviceCenter, 18, nodeStroke);
      _drawIcon(canvas, deviceCenter, devices[i].typeIcon, 14, const Color(0xFF2DD4BF));
    }
  }

  void _drawIcon(Canvas canvas, Offset center, IconData icon, double size, Color color) {
    final tp = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontSize: size,
          fontFamily: icon.fontFamily,
          color: color,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, Offset(center.dx - tp.width / 2, center.dy - tp.height / 2));
  }

  @override
  bool shouldRepaint(covariant SpiderWebPainter oldDelegate) {
    return oldDelegate.devices.length != devices.length ||
        oldDelegate.devices.asMap().entries.any((e) =>
          e.value.id != devices[e.key].id || e.value.status != devices[e.key].status);
  }
}