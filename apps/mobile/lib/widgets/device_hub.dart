import 'dart:math';
import 'dart:ui' show ImageFilter;
import 'package:flutter/material.dart';
import '../theme/app_theme.dart';
import '../models/device_info.dart';

/// Dot grid background painter for the device hub container.
class DotGridPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = Colors.white.withValues(alpha: 0.30)
      ..style = PaintingStyle.fill;

    const spacingX = 22.0;
    const spacingY = 22.0;
    const dotRadius = 0.9;

    for (double x = spacingX / 2; x < size.width; x += spacingX) {
      for (double y = spacingY / 2; y < size.height; y += spacingY) {
        final halo = Paint()
          ..color = Colors.white.withValues(alpha: 0.055)
          ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 3);
        canvas.drawCircle(Offset(x, y), 3.2, halo);
        canvas.drawCircle(Offset(x, y), dotRadius, paint);
      }
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

class DeviceHub extends StatefulWidget {
  final List<DeviceInfo> devices;

  const DeviceHub({super.key, this.devices = const []});

  @override
  State<DeviceHub> createState() => _DeviceHubState();
}

class _DeviceHubState extends State<DeviceHub> with TickerProviderStateMixin {
  late AnimationController _floatController;

  final Map<String, AnimationController> _entranceControllers = {};
  final Map<String, AnimationController> _exitControllers = {};
  final Map<String, DeviceInfo> _exitingDevices = {};
  final Set<String> _exitingDeviceIds = {};

  @override
  void initState() {
    super.initState();
    _floatController = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 4),
    )..repeat();

  }

  @override
  void didUpdateWidget(covariant DeviceHub oldWidget) {
    super.didUpdateWidget(oldWidget);

    final oldIds = oldWidget.devices.map((d) => d.id).toSet();
    final newIds = widget.devices.map((d) => d.id).toSet();

    for (final id in newIds.difference(oldIds)) {
      _exitingDevices.remove(id);
      _exitingDeviceIds.remove(id);
      _exitControllers.remove(id)?.dispose();
      _entranceControllers[id]?.dispose();
      final ctrl = AnimationController(
        vsync: this,
        duration: const Duration(milliseconds: 500),
      );
      _entranceControllers[id] = ctrl;
      ctrl.addListener(() {
        if (mounted) setState(() {});
      });
      ctrl.addStatusListener((status) {
        if (status == AnimationStatus.completed) {
          _entranceControllers.remove(id);
          ctrl.dispose();
        }
      });
      ctrl.forward();
    }

    for (final id in oldIds.difference(newIds)) {
      if (_exitingDeviceIds.contains(id)) continue;
      _exitingDevices[id] = oldWidget.devices.firstWhere((device) => device.id == id);
      _exitControllers[id]?.dispose();
      final ctrl = AnimationController(
        vsync: this,
        duration: const Duration(milliseconds: 300),
      );
      _exitControllers[id] = ctrl;
      _exitingDeviceIds.add(id);
      ctrl.addListener(() {
        if (mounted) setState(() {});
      });
      ctrl.addStatusListener((status) {
        if (status == AnimationStatus.completed) {
          _exitingDeviceIds.remove(id);
          _exitingDevices.remove(id);
          _exitControllers.remove(id);
          ctrl.dispose();
        }
      });
      ctrl.forward();
    }
  }

  @override
  void dispose() {
    _floatController.dispose();
    for (final ctrl in _entranceControllers.values) {
      ctrl.dispose();
    }
    for (final ctrl in _exitControllers.values) {
      ctrl.dispose();
    }
    super.dispose();
  }

  double _getEntranceScale(String id) {
    final ctrl = _entranceControllers[id];
    if (ctrl == null) return 1.0;
    return Curves.elasticOut.transform(ctrl.value);
  }

  double _getExitScale(String id) {
    final ctrl = _exitControllers[id];
    if (ctrl == null) return 1.0;
    return 1.0 - Curves.easeInCubic.transform(ctrl.value);
  }

  double _getExitOpacity(String id) {
    final ctrl = _exitControllers[id];
    if (ctrl == null) return 1.0;
    return (1.0 - ctrl.value).clamp(0.0, 1.0);
  }

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Container(
      height: 260,
      margin: const EdgeInsets.symmetric(horizontal: 16),
      decoration: BoxDecoration(
        color: colors.bg0,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: colors.border),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(20),
        child: LayoutBuilder(
          builder: (context, constraints) {
            final size = Size(constraints.maxWidth, constraints.maxHeight);
            return Stack(
              children: [
                Positioned.fill(
                  child: CustomPaint(painter: DotGridPainter()),
                ),
                if (widget.devices.isEmpty)
                  _buildEmptyState(colors)
                else
                  // The idle float has to be read through a Listenable, or the
                  // controller ticks forever with nothing ever repainting.
                  Positioned.fill(
                    child: AnimatedBuilder(
                      animation: _floatController,
                      builder: (context, _) =>
                          Stack(children: _buildBubbles(colors, size)),
                    ),
                  ),
              ],
            );
          },
        ),
      ),
    );
  }

  Widget _buildEmptyState(AppColors colors) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.hub_outlined, size: 48, color: colors.text3),
          const SizedBox(height: 12),
          Text(
            'No devices connected',
            style: TextStyle(color: colors.text3, fontSize: 14),
          ),
          const SizedBox(height: 4),
          Text(
            'Tap + to pair a device',
            style: TextStyle(color: colors.text3.withValues(alpha: 0.5), fontSize: 12),
          ),
        ],
      ),
    );
  }

  List<Widget> _buildBubbles(AppColors colors, Size size) {
    // Read inside the AnimatedBuilder in build() so each tick repaints.
    final floatVal = _floatController.value;
    final devices = [...widget.devices, ..._exitingDevices.values];
    final count = devices.length;
    final positions = List.generate(count, (i) {
      final phase = i * 2 * pi / max(count, 1);
      return _getBubblePosition(i, count, size) + Offset(
        cos(floatVal * 1.7 * pi + phase) * 3,
        sin(floatVal * 2 * pi + phase) * 5,
      );
    });

    // Softly deflect close bubbles on the compact phone layout.
    for (var pass = 0; pass < 2; pass++) {
      for (var a = 0; a < positions.length; a++) {
        for (var b = a + 1; b < positions.length; b++) {
          final delta = positions[b] - positions[a];
          final distance = delta.distance;
          if (distance == 0 || distance >= 78) continue;
          final correction = delta / distance * ((78 - distance) * 0.28);
          positions[a] -= correction;
          positions[b] += correction;
        }
      }
    }
    for (var i = 0; i < positions.length; i++) {
      positions[i] = Offset(
        positions[i].dx.clamp(36.0, max(36.0, size.width - 36)).toDouble(),
        positions[i].dy.clamp(36.0, max(36.0, size.height - 36)).toDouble(),
      );
    }

    return List.generate(devices.length, (i) {
      final device = devices[i];
      final position = positions[i];

      final entranceScale = _getEntranceScale(device.id);
      final exitScale = _getExitScale(device.id);
      final exitOpacity = _getExitOpacity(device.id);
      final scale = entranceScale * exitScale;
      final exitProgress = _exitControllers[device.id]?.value ?? 0.0;

      return Positioned(
        left: position.dx - 35,
        top: position.dy - 35,
        child: IgnorePointer(
          ignoring: _exitingDeviceIds.contains(device.id),
          child: Transform.rotate(
            angle: exitProgress * 1.8 * pi,
            child: Stack(
              alignment: Alignment.center,
              children: [
                Opacity(
                  opacity: exitOpacity,
                  child: Transform.scale(
                    scale: scale,
                    child: _buildBubble(device, colors),
                  ),
                ),
                if (_exitingDeviceIds.contains(device.id))
                  Positioned.fill(
                    child: CustomPaint(
                      painter: _DisconnectBurstPainter(progress: exitProgress, color: colors.accent),
                    ),
                  ),
              ],
            ),
          ),
        ),
      );
    });
  }

  Widget _buildBubble(DeviceInfo device, AppColors colors) {
    return SizedBox(
      width: 70,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ClipOval(
            child: BackdropFilter(
              filter: ImageFilter.blur(sigmaX: 16, sigmaY: 16),
              child: Container(
                width: 60,
                height: 60,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: RadialGradient(
                    center: const Alignment(-0.4, -0.4),
                    radius: 0.8,
                    colors: [
                      Colors.white.withValues(alpha: 0.8),
                      Colors.white.withValues(alpha: 0.1),
                      colors.accent.withValues(alpha: 0.05),
                      Colors.black.withValues(alpha: 0.3),
                    ],
                    stops: const [0.0, 0.25, 0.6, 1.0],
                  ),
                  border: Border.all(
                    color: Colors.white.withValues(alpha: 0.3),
                    width: 1.0,
                  ),
                  boxShadow: device.status == 'connected'
                      ? [
                          BoxShadow(
                            color: colors.accent.withValues(alpha: 0.4),
                            blurRadius: 24,
                            spreadRadius: 2,
                          ),
                          BoxShadow(
                            color: Colors.black.withValues(alpha: 0.9),
                            blurRadius: 20,
                            spreadRadius: 0,
                            offset: const Offset(0, 10),
                          ),
                        ]
                      : [
                          BoxShadow(
                            color: Colors.black.withValues(alpha: 0.7),
                            blurRadius: 15,
                            spreadRadius: 0,
                            offset: const Offset(0, 8),
                          ),
                        ],
                ),
                child: Icon(
                  device.typeIcon,
                  color: colors.accent,
                  size: 24,
                ),
              ),
            ),
          ),
          const SizedBox(height: 6),
          Text(
            device.name,
            style: TextStyle(color: colors.text2, fontSize: 10),
            textAlign: TextAlign.center,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ],
      ),
    );
  }

  Offset _getBubblePosition(int index, int count, Size size) {
    switch (count) {
      case 1:
        return Offset(size.width * 0.5, size.height * 0.5);
      case 2:
        return [
          Offset(size.width * 0.34, size.height * 0.5),
          Offset(size.width * 0.66, size.height * 0.5),
        ][index];
      case 3:
        return [
          Offset(size.width * 0.5, size.height * 0.32),
          Offset(size.width * 0.27, size.height * 0.7),
          Offset(size.width * 0.73, size.height * 0.7),
        ][index];
      case 4:
        return [
          Offset(size.width * 0.28, size.height * 0.32),
          Offset(size.width * 0.72, size.height * 0.32),
          Offset(size.width * 0.28, size.height * 0.7),
          Offset(size.width * 0.72, size.height * 0.7),
        ][index];
      default:
        final cols = count <= 6 ? 3 : 4;
        final row = index ~/ cols;
        final col = index % cols;
        final xSpacing = size.width / cols;
        final ySpacing = size.height / ((count / cols).ceil() + 0.4);
        return Offset(
          xSpacing * (col + 0.5),
          ySpacing * (row + 0.7),
        );
    }
  }
}

class _DisconnectBurstPainter extends CustomPainter {
  final double progress;
  final Color color;
  const _DisconnectBurstPainter({required this.progress, required this.color});

  @override
  void paint(Canvas canvas, Size size) {
    final center = Offset(size.width / 2, size.height / 2);
    final paint = Paint()..color = color.withValues(alpha: (1 - progress).clamp(0.0, 1.0));
    for (var i = 0; i < 12; i++) {
      final angle = (i / 12) * 2 * pi;
      final distance = 8 + progress * 48;
      final point = center + Offset(cos(angle), sin(angle)) * distance;
      canvas.drawCircle(point, 2.8 * (1 - progress).clamp(0.0, 1.0), paint);
    }
  }

  @override
  bool shouldRepaint(covariant _DisconnectBurstPainter oldDelegate) =>
      oldDelegate.progress != progress || oldDelegate.color != color;
}
