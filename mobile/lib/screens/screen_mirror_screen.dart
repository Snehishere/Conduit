import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/websocket_service.dart';
import '../services/screen_mirror_service.dart';
import '../widgets/connection_status.dart';

class ScreenMirrorScreen extends StatefulWidget {
  final String deviceId;
  final String deviceName;

  const ScreenMirrorScreen({
    super.key,
    required this.deviceId,
    required this.deviceName,
  });

  @override
  State<ScreenMirrorScreen> createState() => _ScreenMirrorScreenState();
}

class _ScreenMirrorScreenState extends State<ScreenMirrorScreen> {
  late ScreenMirrorService _mirrorService;
  bool _isMirroring = false;
  String _quality = 'medium';
  int _fps = 15;

  @override
  void initState() {
    super.initState();
    final ws = context.read<WebSocketService>();
    _mirrorService = ScreenMirrorService(ws);
  }

  @override
  void dispose() {
    _mirrorService.dispose();
    super.dispose();
  }

  Future<void> _startMirroring() async {
    final success = await _mirrorService.startMirroring(
      deviceId: widget.deviceId,
      quality: _quality,
      fps: _fps,
    );

    if (success) {
      setState(() {
        _isMirroring = true;
      });
    } else {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Failed to start screen mirroring. Check permissions.'),
            backgroundColor: Colors.red,
          ),
        );
      }
    }
  }

  Future<void> _stopMirroring() async {
    await _mirrorService.stopMirroring();
    setState(() {
      _isMirroring = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text('Screen Mirror: ${widget.deviceName}'),
        actions: [
          Consumer<WebSocketService>(
            builder: (_, ws, __) => ConnectionStatus(connected: ws.isConnected),
          ),
        ],
      ),
      body: Column(
        children: [
          // Controls
          Container(
            padding: const EdgeInsets.all(16),
            color: const Color(0xFF12121A),
            child: Row(
              children: [
                // Quality selector
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text(
                        'Quality',
                        style: TextStyle(
                          color: Color(0xFFA0A0B0),
                          fontSize: 12,
                        ),
                      ),
                      const SizedBox(height: 4),
                      DropdownButton<String>(
                        value: _quality,
                        isExpanded: true,
                        dropdownColor: const Color(0xFF1A1A25),
                        style: const TextStyle(color: Colors.white),
                        items: const [
                          DropdownMenuItem(
                            value: 'low',
                            child: Text('Low (480p)'),
                          ),
                          DropdownMenuItem(
                            value: 'medium',
                            child: Text('Medium (720p)'),
                          ),
                          DropdownMenuItem(
                            value: 'high',
                            child: Text('High (1080p)'),
                          ),
                        ],
                        onChanged: (value) {
                          if (value != null) {
                            setState(() {
                              _quality = value;
                            });
                          }
                        },
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 16),
                // FPS selector
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text(
                        'FPS',
                        style: TextStyle(
                          color: Color(0xFFA0A0B0),
                          fontSize: 12,
                        ),
                      ),
                      const SizedBox(height: 4),
                      DropdownButton<int>(
                        value: _fps,
                        isExpanded: true,
                        dropdownColor: const Color(0xFF1A1A25),
                        style: const TextStyle(color: Colors.white),
                        items: const [
                          DropdownMenuItem(
                            value: 15,
                            child: Text('15 FPS'),
                          ),
                          DropdownMenuItem(
                            value: 30,
                            child: Text('30 FPS'),
                          ),
                          DropdownMenuItem(
                            value: 60,
                            child: Text('60 FPS'),
                          ),
                        ],
                        onChanged: (value) {
                          if (value != null) {
                            setState(() {
                              _fps = value;
                            });
                          }
                        },
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 16),
                // Start/Stop button
                ElevatedButton(
                  onPressed: _isMirroring ? _stopMirroring : _startMirroring,
                  style: ElevatedButton.styleFrom(
                    backgroundColor: _isMirroring
                        ? const Color(0xFFEF4444)
                        : const Color(0xFF6366F1),
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 24,
                      vertical: 12,
                    ),
                  ),
                  child: Text(_isMirroring ? 'Stop' : 'Start'),
                ),
              ],
            ),
          ),
          // Mirror view
          Expanded(
            child: Container(
              color: const Color(0xFF0A0A0F),
              child: Center(
                child: _isMirroring
                    ? const Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Icon(
                            Icons.screen_lock_portrait,
                            size: 64,
                            color: Color(0xFF6366F1),
                          ),
                          SizedBox(height: 16),
                          Text(
                            'Screen mirroring active',
                            style: TextStyle(
                              color: Colors.white,
                              fontSize: 18,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          SizedBox(height: 8),
                          Text(
                            'Your screen is being mirrored to the desktop',
                            style: TextStyle(
                              color: Color(0xFFA0A0B0),
                              fontSize: 14,
                            ),
                          ),
                        ],
                      )
                    : const Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Icon(
                            Icons.screen_lock_portrait,
                            size: 64,
                            color: Color(0xFF606070),
                          ),
                          SizedBox(height: 16),
                          Text(
                            'Screen mirroring not active',
                            style: TextStyle(
                              color: Colors.white,
                              fontSize: 18,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          SizedBox(height: 8),
                          Text(
                            'Tap "Start" to begin mirroring your screen',
                            style: TextStyle(
                              color: Color(0xFFA0A0B0),
                              fontSize: 14,
                            ),
                          ),
                        ],
                      ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}