import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/discovery_service.dart';
import '../theme/app_theme.dart';
import 'pairing_screen.dart';

class DiscoveryScreen extends StatefulWidget {
  const DiscoveryScreen({super.key});

  @override
  State<DiscoveryScreen> createState() => _DiscoveryScreenState();
}

class _DiscoveryScreenState extends State<DiscoveryScreen> {
  // Cached in initState — context.read is unsafe during dispose.
  late final DiscoveryService _service;

  @override
  void initState() {
    super.initState();
    _service = context.read<DiscoveryService>();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _service.startDiscovery();
    });
  }

  @override
  void dispose() {
    _service.stopDiscovery();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Discover devices'),
        actions: [
          Consumer<DiscoveryService>(
            builder: (_, svc, _) {
              return IconButton(
                icon: Icon(svc.isDiscovering ? Icons.stop : Icons.play_arrow),
                tooltip: svc.isDiscovering ? 'Stop' : 'Start',
                onPressed: () {
                  if (svc.isDiscovering) {
                    svc.stopDiscovery();
                  } else {
                    svc.startDiscovery();
                  }
                },
              );
            },
          ),
        ],
      ),
      body: Consumer<DiscoveryService>(
        builder: (_, svc, _) {
          if (svc.devices.isEmpty) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  SizedBox(
                    width: 80,
                    height: 80,
                    child: CircularProgressIndicator(
                      strokeWidth: 3,
                      color: svc.isDiscovering
                          ? colors.accentSecondary
                          : colors.text3,
                    ),
                  ),
                  const SizedBox(height: 24),
                  Text(
                    svc.isDiscovering
                        ? 'Searching for devices...'
                        : 'Tap the play button to start discovery',
                    style: TextStyle(fontSize: 16, color: colors.text2),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Make sure devices are on the same network',
                    style: TextStyle(fontSize: 13, color: colors.text3),
                  ),
                ],
              ),
            );
          }

          return ListView.builder(
            padding: const EdgeInsets.all(12),
            itemCount: svc.devices.length,
            itemBuilder: (_, i) {
              final device = svc.devices[i];
              return Card(
                margin: const EdgeInsets.only(bottom: 8),
                child: ListTile(
                  leading: _getDeviceIcon(device.type, colors),
                  title: Text(device.name, style: TextStyle(color: colors.text1)),
                  subtitle: Text('${device.type} · ${device.address}',
                      style: TextStyle(color: colors.text2)),
                  trailing: Icon(Icons.chevron_right, color: colors.text3),
                  onTap: () => _onDeviceTap(device, colors),
                ),
              );
            },
          );
        },
      ),
    );
  }

  Widget _getDeviceIcon(String type, AppColors colors) {
    IconData icon;
    Color color;
    switch (type.toLowerCase()) {
      case 'desktop':
        icon = Icons.computer;
        color = colors.accentSecondary;
        break;
      case 'phone':
        icon = Icons.phone_android;
        color = colors.accent;
        break;
      case 'tablet':
        icon = Icons.tablet;
        color = const Color(0xFF06B6D4);
        break;
      case 'earbuds':
        icon = Icons.headset;
        color = const Color(0xFFF59E0B);
        break;

      default:
        icon = Icons.device_unknown;
        color = colors.text3;
    }
    return Container(
      width: 40,
      height: 40,
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.15),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Icon(icon, color: color, size: 20),
    );
  }

  void _onDeviceTap(DiscoveredDevice device, AppColors colors) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text(device.name, style: TextStyle(color: colors.text1)),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Type: ${device.type}', style: TextStyle(color: colors.text2)),
            Text('Address: ${device.address}', style: TextStyle(color: colors.text2)),
            // Pairing always dials the peer's TLS port (wssPort). `port` is the
            // plaintext SRV port mDNS advertised; it is shown only so the record
            // matches what was seen on the network.
            Text(
              'Service port: ${device.port}',
              style: TextStyle(color: colors.text2),
            ),
            Text(
              'TLS port: ${device.wssPort}',
              style: TextStyle(color: colors.text2),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text2)),
          ),
          TextButton(
            onPressed: () {
              Navigator.pop(ctx);
              Navigator.push(
                context,
                MaterialPageRoute(
                  builder: (context) => PairingScreen(
                    ip: device.address,
                    port: device.port,
                  ),
                ),
              );
            },
            child: Text('Pair', style: TextStyle(color: colors.accent)),
          ),
        ],
      ),
    );
  }
}
