import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../services/websocket_service.dart';
import '../services/clipboard_service.dart';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  bool _syncNotifications = true;
  bool _syncClipboard = true;
  bool _autoClipboardSync = true;
  bool _autoAnswerCalls = false;
  bool _initialized = false;

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final prefs = await SharedPreferences.getInstance();
    setState(() {
      _syncNotifications = prefs.getBool('sync_notifications') ?? true;
      _syncClipboard = prefs.getBool('sync_clipboard') ?? true;
      _autoClipboardSync = prefs.getBool('auto_clipboard_sync') ?? true;
      _autoAnswerCalls = prefs.getBool('auto_answer_calls') ?? false;
      _initialized = true;
    });
  }

  Future<void> _saveSetting(String key, bool value) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(key, value);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        children: [
          _buildSection('Device', [
            _buildDeviceTile(),
          ]),
          _buildSection('Connection', [
            _buildConnectionTile(),
          ]),
          _buildSection('Sync', [
            SwitchListTile(
              secondary: const Icon(Icons.notifications_active, color: Color(0xFF6366F1)),
              title: const Text('Sync Notifications'),
              subtitle: const Text('Receive phone notifications on desktop'),
              value: _syncNotifications,
              onChanged: _initialized
                  ? (v) {
                      setState(() => _syncNotifications = v);
                      _saveSetting('sync_notifications', v);
                    }
                  : null,
            ),
            SwitchListTile(
              secondary: const Icon(Icons.content_paste, color: Color(0xFF2DD4BF)),
              title: const Text('Sync Clipboard'),
              subtitle: const Text('Share clipboard between devices'),
              value: _syncClipboard,
              onChanged: _initialized
                  ? (v) {
                      setState(() => _syncClipboard = v);
                      _saveSetting('sync_clipboard', v);
                    }
                  : null,
            ),
            SwitchListTile(
              secondary: const Icon(Icons.sync, color: Color(0xFF6366F1)),
              title: const Text('Auto-sync Clipboard'),
              subtitle: const Text('Automatically detect and sync clipboard changes'),
              value: _autoClipboardSync,
              onChanged: _initialized
                  ? (v) {
                      setState(() => _autoClipboardSync = v);
                      _saveSetting('auto_clipboard_sync', v);
                      final clipboard = context.read<ClipboardService>();
                      if (v) {
                        clipboard.startListening();
                      } else {
                        clipboard.stopListening();
                      }
                    }
                  : null,
            ),
          ]),
          _buildSection('Calls', [
            SwitchListTile(
              secondary: const Icon(Icons.phone, color: Color(0xFF06B6D4)),
              title: const Text('Auto-answer on Desktop'),
              subtitle: const Text('Automatically route calls to desktop'),
              value: _autoAnswerCalls,
              onChanged: _initialized
                  ? (v) {
                      setState(() => _autoAnswerCalls = v);
                      _saveSetting('auto_answer_calls', v);
                    }
                  : null,
            ),
          ]),
          _buildSection('About', [
            const ListTile(
              leading: Icon(Icons.info_outline),
              title: Text('Version'),
              trailing: Text('0.1.0', style: TextStyle(color: Color(0xFFA0A0B0))),
            ),
            const ListTile(
              leading: Icon(Icons.code),
              title: Text('Open Source'),
              subtitle: Text('MIT License'),
              trailing: Icon(Icons.open_in_new, size: 18),
            ),
          ]),
        ],
      ),
    );
  }

  Widget _buildSection(String title, List<Widget> children) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
          child: Text(
            title.toUpperCase(),
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              color: Colors.grey.shade500,
              letterSpacing: 0.5,
            ),
          ),
        ),
        ...children,
        const Divider(height: 1),
      ],
    );
  }

  Widget _buildDeviceTile() {
    return Consumer<WebSocketService>(
      builder: (_, ws, __) => ListTile(
        leading: const Icon(Icons.phone_android, color: Color(0xFF6366F1)),
        title: const Text('This Device'),
        subtitle: Text(ws.isConnected ? 'Connected' : 'Disconnected'),
        trailing: Container(
          width: 8,
          height: 8,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: ws.isConnected ? const Color(0xFF2DD4BF) : const Color(0xFFEF4444),
          ),
        ),
      ),
    );
  }

  Widget _buildConnectionTile() {
    return Consumer<WebSocketService>(
      builder: (_, ws, __) => ListTile(
        leading: const Icon(Icons.wifi, color: Color(0xFFEAB308)),
        title: const Text('Hub Address'),
        subtitle: Text(ws.isConnected ? 'ws://localhost:9527' : 'Not connected'),
      ),
    );
  }
}
