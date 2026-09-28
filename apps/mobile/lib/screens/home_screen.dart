import 'dart:ui';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/websocket_service.dart';
import '../services/notification_service.dart';
import '../services/clipboard_service.dart';
import '../services/file_service.dart';
import '../theme/app_theme.dart';
import '../models/device_info.dart';
import '../widgets/device_hub.dart';
import '../widgets/entrance.dart';
import '../widgets/actions_menu_sheet.dart';
import '../widgets/incoming_call_overlay.dart';
import '../widgets/animated_button.dart';
import '../widgets/status_badge.dart';
import '../widgets/conduit_logo.dart';
import 'notifications_screen.dart';
import 'settings_screen.dart';
import 'clipboard_screen.dart';
import 'files_screen.dart';
import 'search_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  int _selectedIndex = 0;
  final List<DeviceInfo> _connectedDevices = [];
  final Map<String, DeviceInfo> _deviceMap = {};

  // Store handler references for cleanup
  late final MessageHandler _discoveryHandler;
  late final MessageHandler _pairingHandler;
  late final MessageHandler _statusHandler;

  // Cached in initState — context.read is unsafe during dispose.
  late final WebSocketService _ws;

  @override
  void initState() {
    super.initState();

    final ws = context.read<WebSocketService>();
    _ws = ws;

    // Listen for device discovery
    _discoveryHandler = (data) {
      if (!mounted) return;
      final map = data;
      if (map.containsKey('devices') && map['devices'] is List) {
        for (final d in map['devices']) {
          final dm = d as Map<String, dynamic>;
          if (dm['id'] != null) {
            final device = DeviceInfo(
              id: dm['id'] as String,
              name: (dm['name'] as String?) ?? 'Unknown',
              type: (dm['type'] as String?) ?? 'unknown',
              status: 'connected',
              battery: dm['battery'] as int?,
            );
            _deviceMap[device.id] = device;
          }
        }
        _syncDeviceList();
      } else if (map['id'] != null) {
        final device = DeviceInfo(
          id: map['id'] as String,
          name: (map['name'] as String?) ?? 'Unknown',
          type: (map['type'] as String?) ?? 'unknown',
          status: 'connected',
          battery: map['battery'] as int?,
        );
        _deviceMap[device.id] = device;
        _syncDeviceList();
      }
    };
    ws.registerHandler('discovery', _discoveryHandler);

    // Listen for pairing events
    _pairingHandler = (data) {
      if (!mounted) return;
      final map = data;
      final status = map['status'] as String?;
      if (status == 'paired' || status == 'connected') {
        final device = DeviceInfo(
          id: map['device_id'] as String? ?? '',
          name: (map['device_name'] as String?) ?? 'Unknown',
          type: (map['device_type'] as String?) ?? 'unknown',
          status: 'connected',
          battery: map['battery'] as int?,
        );
        if (device.id.isNotEmpty) {
          _deviceMap[device.id] = device;
          _syncDeviceList();
        }
      } else if (status == 'disconnected') {
        final deviceId = map['device_id'] as String?;
        if (deviceId != null) {
          _deviceMap.remove(deviceId);
          _syncDeviceList();
        }
      }
    };
    ws.registerHandler('pairing', _pairingHandler);

    // Listen for status updates
    _statusHandler = (data) {
      if (!mounted) return;
      final map = data;
      if (map['device_id'] != null) {
        final deviceId = map['device_id'] as String;
        final status = map['status'] as String?;
        if (status == 'disconnected' || status == 'offline') {
          _deviceMap.remove(deviceId);
          _syncDeviceList();
        } else if (_deviceMap.containsKey(deviceId)) {
          final existing = _deviceMap[deviceId]!;
          _deviceMap[deviceId] = DeviceInfo(
            id: existing.id,
            name: existing.name,
            type: existing.type,
            status: status ?? existing.status,
            battery: (map['battery'] as int?) ?? existing.battery,
          );
          _syncDeviceList();
        }
      }
    };
    ws.registerHandler('status', _statusHandler);

    // Request discovery to get the latest list of devices, 
    // since the initial connection might have happened before HomeScreen was mounted
    ws.sendMessage({
      'type': 'discovery',
      'action': 'announce',
      'protocol_version': 1,
      'device_type': 'phone',
    });
  }

  void _syncDeviceList() {
    if (!mounted) return;
    setState(() {
      _connectedDevices
        ..clear()
        ..addAll(_deviceMap.values);
    });
  }

  @override
  void dispose() {
    _ws.unregisterHandler('discovery', _discoveryHandler);
    _ws.unregisterHandler('pairing', _pairingHandler);
    _ws.unregisterHandler('status', _statusHandler);
    super.dispose();
  }

  void _onItemTapped(int index) {
    if (index == 2) {
      // "+" tab — open actions menu
      _showActionsMenu();
      return;
    }
    if (index == 3) {
      // Settings — push as full screen
      Navigator.push(
        context,
        MaterialPageRoute(
          builder: (_) => ChangeNotifierProvider.value(
            value: context.read<ClipboardService>(),
            child: ChangeNotifierProvider.value(
              value: context.read<FileService>(),
              child: ChangeNotifierProvider.value(
                value: context.read<NotificationService>(),
                child: ChangeNotifierProvider.value(
                  value: context.read<WebSocketService>(),
                  child: const SettingsScreen(),
                ),
              ),
            ),
          ),
        ),
      );
      return;
    }
    setState(() {
      _selectedIndex = index;
    });
  }

  void _showActionsMenu() {
    showModalBottomSheet(
      context: context,
      backgroundColor: Colors.transparent,
      isScrollControlled: true,
      builder: (_) => ChangeNotifierProvider.value(
        value: context.read<ClipboardService>(),
        child: ChangeNotifierProvider.value(
          value: context.read<FileService>(),
          child: ChangeNotifierProvider.value(
            value: context.read<WebSocketService>(),
            child: ActionsMenuSheet(
              connectedDevices: _connectedDevices,
            ),
          ),
        ),
      ),
    );
  }

  Widget _buildBody() {
    switch (_selectedIndex) {
      case 0:
        return _buildHomePage();
      case 1:
        return const NotificationsScreen(embedded: true);
      default:
        return _buildHomePage();
    }
  }

  Widget _buildHomePage() {
    final colors = Theme.of(context).extension<AppColors>()!;

    return SingleChildScrollView(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Entrance(
            duration: const Duration(milliseconds: 400),
            slideY: 0.1,
            curve: Curves.easeOutQuad,
            child: DeviceHub(devices: _connectedDevices),
          ),
          const SizedBox(height: 24),
          Entrance(
            delay: const Duration(milliseconds: 100),
            duration: const Duration(milliseconds: 400),
            slideY: 0.2,
            curve: Curves.easeOutQuad,
            child: Text(
              'Quick access',
              style: TextStyle(
                fontSize: 16,
                fontWeight: FontWeight.w600,
                color: colors.text1,
              ),
            ),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: Entrance(
                  delay: const Duration(milliseconds: 150),
                  duration: const Duration(milliseconds: 400),
                  beginScale: 0.9,
                  curve: Curves.easeOutBack,
                  child: AnimatedButton(
                    onTap: () => Navigator.push(
                      context,
                      MaterialPageRoute(
                        builder: (_) => ChangeNotifierProvider.value(
                          value: context.read<ClipboardService>(),
                          child: ChangeNotifierProvider.value(
                            value: context.read<WebSocketService>(),
                            child: const ClipboardScreen(),
                          ),
                        ),
                      ),
                    ),
                    child: const _QuickAccessCard(
                      icon: Icons.content_paste_outlined,
                      label: 'Clipboard',
                      subtitle: 'View synced items',
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Entrance(
                  delay: const Duration(milliseconds: 200),
                  duration: const Duration(milliseconds: 400),
                  beginScale: 0.9,
                  curve: Curves.easeOutBack,
                  child: AnimatedButton(
                    onTap: () => Navigator.push(
                      context,
                      MaterialPageRoute(
                        builder: (_) => ChangeNotifierProvider.value(
                          value: context.read<FileService>(),
                          child: const FilesScreen(),
                        ),
                      ),
                    ),
                    child: const _QuickAccessCard(
                      icon: Icons.folder_outlined,
                      label: 'Files',
                      subtitle: 'View transfers',
                    ),
                  ),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final ws = context.watch<WebSocketService>();

    return Scaffold(
      appBar: AppBar(
        title: const Text('Conduit'),
        centerTitle: false,
        actions: [
          StatusBadge(
            status: ws.isConnected ? BadgeStatus.connected : BadgeStatus.offline,
          ),
          const SizedBox(width: 8),
          IconButton(
            icon: Icon(Icons.search, color: colors.text2),
            onPressed: () => Navigator.push(
              context,
              MaterialPageRoute(builder: (_) => const SearchScreen()),
            ),
          ),
        ],
      ),
      body: Stack(
        children: [
          Positioned.fill(child: CustomPaint(painter: _StarGridPainter())),
          _buildBody(),
          const IncomingCallOverlay(),
        ],
      ),
      extendBody: true,
      bottomNavigationBar: Container(
        decoration: BoxDecoration(
          border: Border(top: BorderSide(color: Colors.white.withValues(alpha: 0.2), width: 1.0)),
        ),
        child: ClipRect(
          child: BackdropFilter(
            filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
            child: Container(
              color: Colors.white.withValues(alpha: 0.12),
              child: BottomNavigationBar(
                backgroundColor: Colors.transparent,
                elevation: 0,
                currentIndex: _selectedIndex,
                onTap: _onItemTapped,
                type: BottomNavigationBarType.fixed,
                items: const [
          BottomNavigationBarItem(
            icon: Icon(Icons.home_outlined),
            activeIcon: Icon(Icons.home),
            label: 'Home',
          ),
          BottomNavigationBarItem(
            icon: Icon(Icons.notifications_outlined),
            activeIcon: Icon(Icons.notifications),
            label: 'Notifications',
          ),
          BottomNavigationBarItem(
            icon: ConduitLogo(size: 25),
            activeIcon: ConduitLogo(size: 29),
            label: 'Add',
          ),
          BottomNavigationBarItem(
            icon: Icon(Icons.settings_outlined),
            activeIcon: Icon(Icons.settings),
            label: 'Settings',
          ),
        ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Quick access card for the home screen (Clipboard or Files).
class _QuickAccessCard extends StatelessWidget {
  final IconData icon;
  final String label;
  final String subtitle;

  const _QuickAccessCard({
    required this.icon,
    required this.label,
    required this.subtitle,
  });

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: colors.bg1,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: colors.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 40,
            height: 40,
            decoration: BoxDecoration(
              color: colors.accent.withValues(alpha: 0.1),
              borderRadius: BorderRadius.circular(10),
            ),
            child: Icon(icon, color: colors.accent, size: 20),
          ),
          const SizedBox(height: 12),
          Text(
            label,
            style: TextStyle(
              fontSize: 14,
              fontWeight: FontWeight.w600,
              color: colors.text1,
            ),
          ),
          const SizedBox(height: 4),
          Text(
            subtitle,
            style: TextStyle(fontSize: 12, color: colors.text2),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ],
      ),
    );
  }
}

class _StarGridPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = Colors.white
      ..style = PaintingStyle.fill;
    
    const double spacing1 = 120.0;
    const double spacing2 = 120.0;

    for (double x = 10; x < size.width; x += spacing1) {
      for (double y = 10; y < size.height; y += spacing1) {
        paint.color = Colors.white.withValues(alpha: 0.4);
        canvas.drawCircle(Offset(x, y), 1.0, paint);
      }
    }

    for (double x = 50; x < size.width; x += spacing2) {
      for (double y = 30; y < size.height; y += spacing2) {
        paint.color = Colors.white.withValues(alpha: 0.2);
        canvas.drawCircle(Offset(x, y), 2.0, paint);
      }
    }

    for (double x = 90; x < size.width; x += spacing1) {
      for (double y = 80; y < size.height; y += spacing1) {
        paint.color = Colors.white.withValues(alpha: 0.1);
        canvas.drawCircle(Offset(x, y), 1.0, paint);
      }
    }

    for (double x = 40; x < size.width; x += spacing1) {
      for (double y = 90; y < size.height; y += spacing1) {
        paint.color = Colors.white.withValues(alpha: 0.3);
        canvas.drawCircle(Offset(x, y), 1.5, paint);
      }
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}
