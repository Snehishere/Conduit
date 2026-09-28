import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/websocket_service.dart';
import '../services/screen_mirror_service.dart';
import '../widgets/connection_status.dart';
import '../theme/app_theme.dart';

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
    _mirrorService.addListener(_onMirrorStateChanged);
  }

  void _onMirrorStateChanged() {
    if (mounted) {
      setState(() {
        _isMirroring = _mirrorService.isMirroring;
      });
    }
  }

  @override
  void dispose() {
    _mirrorService.removeListener(_onMirrorStateChanged);
    _mirrorService.dispose();
    super.dispose();
  }

  Future<void> _startMirroring() async {
    final success = await _mirrorService.startMirroring(
      deviceId: widget.deviceId,
      quality: _quality,
      fps: _fps,
    );

    if (!mounted) return;

    if (success) {
      setState(() {
        _isMirroring = true;
      });
    } else {
      final colors = Theme.of(context).extension<AppColors>()!;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: const Text(
            'Could not start screen mirroring. Check capture permissions.',
          ),
          backgroundColor: colors.error,
        ),
      );
    }
  }

  Future<void> _stopMirroring() async {
    await _mirrorService.stopMirroring();
    if (!mounted) return;
    setState(() {
      _isMirroring = false;
    });
  }

  Widget _buildMirrorView() {
    final colors = Theme.of(context).extension<AppColors>()!;

    if (!_isMirroring || _mirrorService.currentFrame == null) {
      // Either mirroring was never started, or it was started and the desktop
      // has not pushed a frame yet. Both are dead ends for the user unless we
      // say what to do next.
      final waiting = _isMirroring;
      return Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(
              Icons.screen_lock_portrait,
              size: 64,
              color: waiting ? colors.accentSecondary : colors.text3,
            ),
            const SizedBox(height: 16),
            Text(
              waiting
                  ? 'Waiting for a frame from ${widget.deviceName}...'
                  : 'Screen mirroring is not active',
              style: TextStyle(
                color: colors.text1,
                fontSize: 18,
                fontWeight: FontWeight.bold,
              ),
            ),
            const SizedBox(height: 8),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 32),
              child: Text(
                waiting
                    ? 'Keep ${widget.deviceName} unlocked and awake. The first frame arrives once its screen capture starts.'
                    // This screen shows the desktop's screen, not this phone's.
                    : 'Tap Start to show the ${widget.deviceName} screen here',
                textAlign: TextAlign.center,
                style: TextStyle(color: colors.text2, fontSize: 13),
              ),
            ),
          ],
        ),
      );
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        return GestureDetector(
          onTapDown: (details) {
            _handleTouch(details.localPosition, constraints.maxWidth, constraints.maxHeight, 'tap');
          },
          onDoubleTapDown: (details) {
            _handleTouch(details.localPosition, constraints.maxWidth, constraints.maxHeight, 'double_tap');
          },
          onLongPressStart: (details) {
            _handleTouch(details.localPosition, constraints.maxWidth, constraints.maxHeight, 'long_press');
          },
          onPanUpdate: (details) {
            _handleTouch(details.localPosition, constraints.maxWidth, constraints.maxHeight, 'move');
          },
          child: Image.memory(
            _mirrorService.currentFrame!,
            fit: BoxFit.contain,
            width: double.infinity,
            height: double.infinity,
            gaplessPlayback: true,
          ),
        );
      },
    );
  }

  void _handleTouch(Offset localPosition, double widgetWidth, double widgetHeight, String actionType) {
    if (!_isMirroring) return;
    
    // The frame is rendered with BoxFit.contain, so the raw tap offset is not
    // the desktop's pixel coordinate. Send normalised 0..1 fractions instead;
    // the accessibility service scales them by the real display size and also
    // accepts raw pixels, so the two coordinate spaces never have to be
    // reconciled here.
    final relX = localPosition.dx / widgetWidth;
    final relY = localPosition.dy / widgetHeight;
    
    _mirrorService.handleTouchEvent(
      x: relX,
      y: relY,
      actionType: actionType,
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: Text('Screen mirror: ${widget.deviceName}'),
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
                // Quality selector
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('Quality', style: TextStyle(color: colors.text2, fontSize: 12)),
                      const SizedBox(height: 4),
                      DropdownButton<String>(
                        value: _quality,
                        isExpanded: true,
                        dropdownColor: colors.bg2,
                        style: TextStyle(color: colors.text1),
                        items: const [
                          DropdownMenuItem(value: 'low', child: Text('Low (480p)')),
                          DropdownMenuItem(value: 'medium', child: Text('Medium (720p)')),
                          DropdownMenuItem(value: 'high', child: Text('High (1080p)')),
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
                      Text('FPS', style: TextStyle(color: colors.text2, fontSize: 12)),
                      const SizedBox(height: 4),
                      DropdownButton<int>(
                        value: _fps,
                        isExpanded: true,
                        dropdownColor: colors.bg2,
                        style: TextStyle(color: colors.text1),
                        items: const [
                          DropdownMenuItem(value: 15, child: Text('15 FPS')),
                          DropdownMenuItem(value: 30, child: Text('30 FPS')),
                          DropdownMenuItem(value: 60, child: Text('60 FPS')),
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
                    backgroundColor: _isMirroring ? colors.error : colors.accentSecondary,
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
                  ),
                  child: Text(_isMirroring ? 'Stop' : 'Start'),
                ),
              ],
            ),
          ),
          // Mirror view
          Expanded(
            child: Container(
              color: colors.bg0,
              child: _buildMirrorView(),
            ),
          ),
        ],
      ),
    );
  }
}
