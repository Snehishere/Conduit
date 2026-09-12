import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/discovery_service.dart';

class DiscoveryScreen extends StatefulWidget {
  const DiscoveryScreen({super.key});

  @override
  State<DiscoveryScreen> createState() => _DiscoveryScreenState();
}

class _DiscoveryScreenState extends State<DiscoveryScreen> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<DiscoveryService>().startDiscovery();
    });
  }

  @override
  void dispose() {
    context.read<DiscoveryService>().stopDiscovery();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Discover Devices'),
        actions: [
          Consumer<DiscoveryService>(
            builder: (_, svc, __) {
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
        builder: (_, svc, __) {
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
                          ? const Color(0xFF6366F1)
                          : Colors.grey.shade600,
                    ),
                  ),
                  const SizedBox(height: 24),
                  Text(
                    svc.isDiscovering
                        ? 'Searching for devices...'
                        : 'Tap play to start discovery',
                    style: TextStyle(
                      fontSize: 16,
                      color: Colors.grey.shade400,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Make sure devices are on the same network',
                    style: TextStyle(
                      fontSize: 13,
                      color: Colors.grey.shade600,
                    ),
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
                  leading: _getDeviceIcon(device.type),
                  title: Text(device.name),
                  subtitle: Text('${device.type} · ${device.address}'),
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () => _onDeviceTap(device),
                ),
              );
            },
          );
        },
      ),
    );
  }

  Widget _getDeviceIcon(String type) {
    IconData icon;
    Color color;
    switch (type.toLowerCase()) {
      case 'desktop':
        icon = Icons.computer;
        color = const Color(0xFF6366F1);
        break;
      case 'phone':
        icon = Icons.phone_android;
        color = const Color(0xFF2DD4BF);
        break;
      case 'tablet':
        icon = Icons.tablet;
        color = const Color(0xFF06B6D4);
        break;
      case 'earbuds':
        icon = Icons.headset;
        color = const Color(0xFFF59E0B);
        break;
      case 'watch':
        icon = Icons.watch;
        color = const Color(0xFF8B5CF6);
        break;
      default:
        icon = Icons.device_unknown;
        color = Colors.grey;
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

  void _onDeviceTap(DiscoveredDevice device) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(device.name),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Type: ${device.type}'),
            Text('Address: ${device.address}'),
            Text('Port: ${device.port}'),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () {
              Navigator.pop(ctx);
              Navigator.pushNamed(context, '/pairing');
            },
            child: const Text('Pair'),
          ),
        ],
      ),
    );
  }
}
