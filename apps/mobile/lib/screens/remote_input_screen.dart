import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/websocket_service.dart';
import '../services/remote_input_service.dart';
import '../widgets/connection_status.dart';
import '../theme/app_theme.dart';

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
        final colors = Theme.of(context).extension<AppColors>()!;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: const Text('Could not start remote input.'),
            backgroundColor: colors.error,
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
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: Text('Remote input: ${widget.deviceName}'),
        actions: [
          Consumer<WebSocketService>(
            builder: (_, ws, _) => ConnectionStatus(connected: ws.isConnected),
          ),
        ],
      ),
      body: Column(
        children: [
          // Controls
          Container(
            padding: const EdgeInsets.all(16),
            color: colors.bg1,
            child: Row(
              children: [
                // Status
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('Status', style: TextStyle(color: colors.text2, fontSize: 12)),
                      const SizedBox(height: 4),
                      Row(
                        children: [
                          Container(
                            width: 8,
                            height: 8,
                            decoration: BoxDecoration(
                              shape: BoxShape.circle,
                              color: _isConnected ? colors.accent : colors.error,
                            ),
                          ),
                          const SizedBox(width: 8),
                          Text(
                            _isConnected ? 'Connected' : 'Disconnected',
                            style: TextStyle(color: colors.text1, fontSize: 14),
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
                    backgroundColor: _isConnected ? colors.error : colors.accentSecondary,
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
                  ),
                  child: Text(_isConnected ? 'Stop' : 'Start'),
                ),
              ],
            ),
          ),
          // Remote input area
          Expanded(
            child: Container(
              color: colors.bg0,
              child: Center(
                child: _isConnected
                    ? Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Expanded(
                            child: GestureDetector(
                              onPanUpdate: (details) {
                                // Square the delta (preserving sign) so slow drags stay precise
                                // while fast flicks travel further than 1:1.
                                final dx = details.delta.dx;
                                final dy = details.delta.dy;
                                final accelDx = dx.sign * (dx * dx * 0.1).abs() + dx * 0.5;
                                final accelDy = dy.sign * (dy * dy * 0.1).abs() + dy * 0.5;
                                _remoteInputService.sendMouseMove(accelDx, accelDy);
                              },
                              onTap: () {
                                _remoteInputService.sendMouseClick('left');
                              },
                              onLongPress: () {
                                _remoteInputService.sendMouseClick('right');
                              },
                              onDoubleTap: () {
                                _remoteInputService.sendMouseClick('double_left');
                              },
                              child: Container(
                                color: Colors.transparent,
                                width: double.infinity,
                                height: double.infinity,
                                child: Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    Icon(Icons.touch_app, size: 64, color: colors.accentSecondary.withValues(alpha: 0.5)),
                                    const SizedBox(height: 16),
                                    Text(
                                      'Trackpad active',
                                      style: TextStyle(
                                        color: colors.text1,
                                        fontSize: 18,
                                        fontWeight: FontWeight.bold,
                                      ),
                                    ),
                                    const SizedBox(height: 8),
                                    Text(
                                      'Swipe to move • Tap to click • Long press for right click',
                                      style: TextStyle(color: colors.text2, fontSize: 14),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          ),
                        ],
                      )
                    : Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Icon(Icons.mouse, size: 64, color: colors.text3),
                          const SizedBox(height: 16),
                          Text(
                            'Remote input not active',
                            style: TextStyle(
                              color: colors.text1,
                              fontSize: 18,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          const SizedBox(height: 8),
                          Text(
                            'Tap Start to control ${widget.deviceName} '
                            'with this phone as a trackpad',
                            style: TextStyle(color: colors.text2, fontSize: 14),
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
