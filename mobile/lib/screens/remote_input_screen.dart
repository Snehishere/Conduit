import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/websocket_service.dart';
import '../services/remote_input_service.dart';
import '../widgets/connection_status.dart';

class RemoteInputScreen extends StatefulWidget {
  final String deviceId;
  final String deviceName;

  const RemoteInputScreen({
    super.key,
    required this.deviceId,
    required this.deviceName,
  });

  @override
  State<RemoteInputScreen> createState() => _RemoteInputScreenState();
}

class _RemoteInputScreenState extends State<RemoteInputScreen> {
  late RemoteInputService _remoteInputService;
  bool _isConnected = false;

  @override
  void initState() {
    super.initState();
    final ws = context.read<WebSocketService>();
    _remoteInputService = RemoteInputService(ws);
  }

  @override
  void dispose() {
    _remoteInputService.dispose();
    super.dispose();
  }

  Future<void> _startRemoteInput() async {
    final success = await _remoteInputService.startRemoteInput(
      deviceId: widget.deviceId,
    );

    if (success) {
      setState(() {
        _isConnected = true;
      });
    } else {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Failed to start remote input.'),
            backgroundColor: Colors.red,
          ),
        );
      }
    }
  }

  Future<void> _stopRemoteInput() async {
    await _remoteInputService.stopRemoteInput();
    setState(() {
      _isConnected = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text('Remote Input: ${widget.deviceName}'),
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
                // Status
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text(
                        'Status',
                        style: TextStyle(
                          color: Color(0xFFA0A0B0),
                          fontSize: 12,
                        ),
                      ),
                      const SizedBox(height: 4),
                      Row(
                        children: [
                          Container(
                            width: 8,
                            height: 8,
                            decoration: BoxDecoration(
                              shape: BoxShape.circle,
                              color: _isConnected
                                  ? const Color(0xFF2DD4BF)
                                  : const Color(0xFFEF4444),
                            ),
                          ),
                          const SizedBox(width: 8),
                          Text(
                            _isConnected ? 'Connected' : 'Disconnected',
                            style: const TextStyle(
                              color: Colors.white,
                              fontSize: 14,
                            ),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 16),
                // Start/Stop button
                ElevatedButton(
                  onPressed: _isConnected ? _stopRemoteInput : _startRemoteInput,
                  style: ElevatedButton.styleFrom(
                    backgroundColor: _isConnected
                        ? const Color(0xFFEF4444)
                        : const Color(0xFF6366F1),
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 24,
                      vertical: 12,
                    ),
                  ),
                  child: Text(_isConnected ? 'Stop' : 'Start'),
                ),
              ],
            ),
          ),
          // Remote input area
          Expanded(
            child: Container(
              color: const Color(0xFF0A0A0F),
              child: Center(
                child: _isConnected
                    ? const Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Icon(
                            Icons.mouse,
                            size: 64,
                            color: Color(0xFF6366F1),
                          ),
                          SizedBox(height: 16),
                          Text(
                            'Remote input active',
                            style: TextStyle(
                              color: Colors.white,
                              fontSize: 18,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          SizedBox(height: 8),
                          Text(
                            'Use your phone as a trackpad for the desktop',
                            style: TextStyle(
                              color: Color(0xFFA0A0B0),
                              fontSize: 14,
                            ),
                          ),
                          SizedBox(height: 24),
                          Text(
                            'Swipe to move cursor\nTap to click\nLong press for right click',
                            textAlign: TextAlign.center,
                            style: TextStyle(
                              color: Color(0xFF606070),
                              fontSize: 12,
                            ),
                          ),
                        ],
                      )
                    : const Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Icon(
                            Icons.mouse,
                            size: 64,
                            color: Color(0xFF606070),
                          ),
                          SizedBox(height: 16),
                          Text(
                            'Remote input not active',
                            style: TextStyle(
                              color: Colors.white,
                              fontSize: 18,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          SizedBox(height: 8),
                          Text(
                            'Tap "Start" to use your phone as a trackpad',
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