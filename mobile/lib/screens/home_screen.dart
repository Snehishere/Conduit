import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../services/websocket_service.dart';
import '../widgets/spider_web_mini.dart';
import '../widgets/connection_status.dart';
import 'files_screen.dart';
import 'messages_screen.dart';
import 'calls_screen.dart';
import 'notifications_screen.dart';
import 'settings_screen.dart';
import 'pairing_screen.dart';
import 'discovery_screen.dart';
import 'screen_mirror_screen.dart';
import 'remote_input_screen.dart';
import 'automation_rules_screen.dart';
import 'clipboard_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  int _selectedIndex = 0;
  final List<DeviceInfo> _connectedDevices = [];
  String? _selectedDeviceId;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      final ws = context.read<WebSocketService>();
      // Hub address is configurable (Settings → Hub Address). Defaults to
      // localhost for emulator use; on a physical device set the desktop's LAN IP.
      final prefs = await SharedPreferences.getInstance();
      final hubHost = prefs.getString('hub_host') ?? 'localhost';
      final hubPort = prefs.getInt('hub_port') ?? 9527;
      ws.connect('ws://$hubHost:$hubPort');

      // Listen for device announcements to build device list
      ws.registerHandler('discovery', (msg) {
        if (msg['action'] == 'announce') {
          final deviceId = msg['device_id'] as String?;
          if (deviceId != null && deviceId != ws.deviceId) {
            setState(() {
              _connectedDevices.removeWhere((d) => d.id == deviceId);
              _connectedDevices.add(DeviceInfo(
                id: deviceId,
                name: msg['device_name'] as String? ?? 'Unknown',
                type: msg['device_type'] as String? ?? 'unknown',
                status: 'connected',
                battery: msg['battery'] as int?,
              ));
            });
          }
        }
      });

      // Listen for pairing accept to add the paired device
      ws.registerHandler('pairing', (msg) {
        if (msg['action'] == 'accept') {
          final deviceInfo = msg['device_info'] as Map<String, dynamic>?;
          if (deviceInfo != null) {
            setState(() {
              _connectedDevices.removeWhere((d) => d.name == deviceInfo['name']);
              _connectedDevices.add(DeviceInfo.fromMap({
                'id': 'paired_${DateTime.now().millisecondsSinceEpoch}',
                'name': deviceInfo['name'] ?? 'Desktop',
                'device_type': deviceInfo['type'] ?? 'desktop',
                'status': 'connected',
                'battery': deviceInfo['battery'],
              }));
            });
          }
        }
      });

      // Listen for status updates to update battery
      ws.registerHandler('status', (msg) {
        final battery = msg['battery'] as int?;
        if (battery != null) {
          // Update the desktop device's battery
          setState(() {
            final desktop = _connectedDevices.firstWhere(
              (d) => d.type == 'desktop',
              orElse: () => DeviceInfo(id: '', name: '', type: '', status: ''),
            );
            if (desktop.id.isNotEmpty) {
              final idx = _connectedDevices.indexOf(desktop);
              _connectedDevices[idx] = DeviceInfo(
                id: desktop.id,
                name: desktop.name,
                type: desktop.type,
                status: desktop.status,
                battery: battery,
              );
            }
          });
        }
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Row(
          children: [
            Icon(Icons.hub_outlined, size: 24, color: Color(0xFF2DD4BF)),
            SizedBox(width: 8),
            Text('Conduit', style: TextStyle(fontWeight: FontWeight.bold, color: Color(0xFFE2E0E8))),
          ],
        ),
        actions: [
          Consumer<WebSocketService>(
            builder: (_, ws, __) => ConnectionStatus(
              connected: ws.isConnected,
              lastError: ws.lastError,
            ),
          ),
          IconButton(
            icon: const Icon(Icons.settings_outlined, color: Color(0xFF9A96A4)),
            onPressed: () => Navigator.push(
              context,
              MaterialPageRoute(builder: (_) => const SettingsScreen()),
            ),
          ),
        ],
      ),
      body: _buildBody(),
      floatingActionButton: FloatingActionButton(
        onPressed: _showDeviceMenu,
        backgroundColor: const Color(0xFF16161E),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(16),
          side: const BorderSide(color: Color(0xFF2DD4BF), width: 1),
        ),
        child: const Icon(Icons.devices_outlined, color: Color(0xFF2DD4BF)),
      ),
      bottomNavigationBar: BottomNavigationBar(
        currentIndex: _selectedIndex,
        onTap: (i) => setState(() => _selectedIndex = i),
        backgroundColor: const Color(0xFF0E0E14),
        selectedItemColor: const Color(0xFF2DD4BF),
        unselectedItemColor: const Color(0xFF5C586A),
        type: BottomNavigationBarType.fixed,
        items: const [
          BottomNavigationBarItem(icon: Icon(Icons.hub_outlined), label: 'Hub'),
          BottomNavigationBarItem(icon: Icon(Icons.notifications_outlined), label: 'Notifs'),
          BottomNavigationBarItem(icon: Icon(Icons.chat_bubble_outline), label: 'SMS'),
          BottomNavigationBarItem(icon: Icon(Icons.phone_outlined), label: 'Calls'),
          BottomNavigationBarItem(icon: Icon(Icons.folder_outlined), label: 'Files'),
        ],
      ),
    );
  }

  Widget _buildBody() {
    switch (_selectedIndex) {
      case 0: return SpiderWebMini(
        devices: _connectedDevices,
        selectedDeviceId: _selectedDeviceId,
      );
      case 1: return const NotificationsScreen();
      case 2: return const MessagesScreen();
      case 3: return const CallsScreen();
      case 4: return const FilesScreen();
      default: return const Center(child: Text('Coming soon', style: TextStyle(color: Color(0xFFA0A0B0))));
    }
  }

  void _showDeviceMenu() {
    showModalBottomSheet(
      context: context,
      backgroundColor: const Color(0xFF101018),
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(20)),
      ),
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SizedBox(height: 8),
            Container(
              width: 40,
              height: 4,
              decoration: BoxDecoration(
                color: const Color(0xFF2A2A36),
                borderRadius: BorderRadius.circular(2),
              ),
            ),
            const SizedBox(height: 16),
            ListTile(
              leading: const Icon(Icons.qr_code_scanner_outlined, color: Color(0xFF2DD4BF)),
              title: const Text('Pair Device'),
              subtitle: const Text('Scan QR code to pair with desktop'),
              onTap: () {
                Navigator.pop(ctx);
                Navigator.push(
                  context,
                  MaterialPageRoute(builder: (_) => const PairingScreen()),
                );
              },
            ),
            ListTile(
              leading: const Icon(Icons.wifi_find_outlined, color: Color(0xFF2DD4BF)),
              title: const Text('Discover Devices'),
              subtitle: const Text('Find nearby devices on the network'),
              onTap: () {
                Navigator.pop(ctx);
                Navigator.push(
                  context,
                  MaterialPageRoute(builder: (_) => const DiscoveryScreen()),
                );
              },
            ),
            if (_connectedDevices.any((d) => d.type == 'desktop'))
              ListTile(
                leading: const Icon(Icons.screen_share_outlined, color: Color(0xFF2DD4BF)),
                title: const Text('Screen Mirror'),
                subtitle: const Text('Mirror your screen to desktop'),
                onTap: () {
                  Navigator.pop(ctx);
                  final desktop = _connectedDevices.firstWhere(
                    (d) => d.type == 'desktop',
                  );
                  Navigator.push(
                    context,
                    MaterialPageRoute(
                      builder: (_) => ScreenMirrorScreen(
                        deviceId: desktop.id,
                        deviceName: desktop.name,
                      ),
                    ),
                  );
                },
              ),
            if (_connectedDevices.any((d) => d.type == 'desktop'))
              ListTile(
                leading: const Icon(Icons.mouse_outlined, color: Color(0xFF2DD4BF)),
                title: const Text('Remote Input'),
                subtitle: const Text('Use phone as trackpad for desktop'),
                onTap: () {
                  Navigator.pop(ctx);
                  final desktop = _connectedDevices.firstWhere(
                    (d) => d.type == 'desktop',
                  );
                  Navigator.push(
                    context,
                    MaterialPageRoute(
                      builder: (_) => RemoteInputScreen(
                        deviceId: desktop.id,
                        deviceName: desktop.name,
                      ),
                    ),
                  );
                },
              ),
            ListTile(
              leading: const Icon(Icons.bolt_outlined, color: Color(0xFF2DD4BF)),
              title: const Text('Automation Rules'),
              subtitle: const Text('Trigger-action rules for device events'),
              onTap: () {
                Navigator.pop(ctx);
                Navigator.push(
                  context,
                  MaterialPageRoute(
                    builder: (_) => const AutomationRulesScreen(),
                  ),
                );
              },
            ),
            ListTile(
              leading: const Icon(Icons.content_paste, color: Color(0xFF2DD4BF)),
              title: const Text('Clipboard History'),
              subtitle: const Text('View synced clipboard items'),
              onTap: () {
                Navigator.pop(ctx);
                Navigator.push(
                  context,
                  MaterialPageRoute(
                    builder: (_) => const ClipboardScreen(),
                  ),
                );
              },
            ),
            const SizedBox(height: 8),
          ],
        ),
      ),
    );
  }
}
