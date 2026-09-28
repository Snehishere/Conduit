import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/call_service.dart';
import '../services/audio_stream_service.dart';
import '../theme/app_theme.dart';
import '../widgets/empty_state.dart';

class CallsScreen extends StatelessWidget {
  const CallsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Calls'),
      ),
      body: Consumer2<CallService, AudioStreamService>(
        builder: (_, callService, audioStream, _) {
          if (callService.state == CallState.ringing && callService.currentCall != null) {
            return _buildIncomingCall(context, callService, colors);
          }

          return Column(
            children: [
              _buildAudioRouteSelector(callService, colors),
              _buildAudioStreamControls(audioStream, colors),
              Divider(height: 1, color: colors.border),
              Expanded(
                child: callService.state == CallState.idle
                    ? _buildIdleState(colors)
                    : _buildActiveCall(context, callService, colors),
              ),
            ],
          );
        },
      ),
    );
  }

  Widget _buildAudioRouteSelector(CallService callService, AppColors colors) {
    return Container(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'AUDIO OUTPUT',
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              color: colors.text3,
              letterSpacing: 0.5,
            ),
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              _buildRouteButton(
                icon: Icons.phone_outlined,
                label: 'Phone',
                active: callService.currentRoute == AudioOutputRoute.phone,
                onTap: () => callService.setAudioRoute(AudioOutputRoute.phone),
                colors: colors,
              ),
              const SizedBox(width: 8),
              _buildRouteButton(
                icon: Icons.computer_outlined,
                label: 'Desktop',
                active: callService.currentRoute == AudioOutputRoute.desktop,
                onTap: () => callService.setAudioRoute(AudioOutputRoute.desktop),
                colors: colors,
              ),
              const SizedBox(width: 8),
              _buildRouteButton(
                icon: Icons.bluetooth_outlined,
                label: 'Bluetooth',
                active: callService.currentRoute == AudioOutputRoute.bluetooth,
                onTap: () => callService.setAudioRoute(AudioOutputRoute.bluetooth),
                colors: colors,
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildAudioStreamControls(AudioStreamService audioStream, AppColors colors) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'AUDIO STREAMING',
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              color: colors.text3,
              letterSpacing: 0.5,
            ),
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: GestureDetector(
                  onTap: () async {
                    if (audioStream.isStreaming) {
                      await audioStream.stopStreaming();
                    } else {
                      await audioStream.startStreaming();
                    }
                  },
                  child: Container(
                    padding: const EdgeInsets.symmetric(vertical: 12),
                    decoration: BoxDecoration(
                      color: audioStream.isStreaming
                          ? colors.accent.withValues(alpha: 0.1)
                          : colors.bg2,
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(
                        color: audioStream.isStreaming
                            ? colors.accent.withValues(alpha: 0.4)
                            : colors.border,
                      ),
                    ),
                    child: Column(
                      children: [
                        Icon(
                          audioStream.isStreaming ? Icons.mic : Icons.mic_off,
                          color: audioStream.isStreaming ? colors.accent : colors.text3,
                          size: 20,
                        ),
                        const SizedBox(height: 4),
                        Text(
                          audioStream.isStreaming
                            ? 'Streaming'
                            : 'Start stream',
                          style: TextStyle(
                            fontSize: 11,
                            color: audioStream.isStreaming ? colors.accent : colors.text3,
                            fontWeight: audioStream.isStreaming ? FontWeight.w600 : FontWeight.normal,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: GestureDetector(
                  onTap: () async {
                    if (audioStream.isPlaying) {
                      await audioStream.stopPlayback();
                    }
                  },
                  child: Container(
                    padding: const EdgeInsets.symmetric(vertical: 12),
                    decoration: BoxDecoration(
                      color: audioStream.isPlaying
                          ? colors.accent.withValues(alpha: 0.1)
                          : colors.bg2,
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(
                        color: audioStream.isPlaying
                            ? colors.accent.withValues(alpha: 0.4)
                            : colors.border,
                      ),
                    ),
                    child: Column(
                      children: [
                        Icon(
                          audioStream.isPlaying ? Icons.stop : Icons.volume_up,
                          color: audioStream.isPlaying ? colors.accent : colors.text3,
                          size: 20,
                        ),
                        const SizedBox(height: 4),
                        Text(
                          audioStream.isPlaying ? 'Playing' : 'Playback',
                          style: TextStyle(
                            fontSize: 11,
                            color: audioStream.isPlaying ? colors.accent : colors.text3,
                            fontWeight: audioStream.isPlaying ? FontWeight.w600 : FontWeight.normal,
                          ),
                        ),
                      ],
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

  Widget _buildRouteButton({
    required IconData icon,
    required String label,
    required bool active,
    required VoidCallback onTap,
    required AppColors colors,
  }) {
    return Expanded(
      child: GestureDetector(
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.symmetric(vertical: 12),
          decoration: BoxDecoration(
            color: active ? colors.accent.withValues(alpha: 0.1) : colors.bg2,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: active ? colors.accent.withValues(alpha: 0.4) : colors.border,
            ),
          ),
          child: Column(
            children: [
              Icon(icon, color: active ? colors.accent : colors.text3, size: 20),
              const SizedBox(height: 4),
              Text(
                label,
                style: TextStyle(
                  fontSize: 11,
                  color: active ? colors.accent : colors.text3,
                  fontWeight: active ? FontWeight.w600 : FontWeight.normal,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildIncomingCall(BuildContext context, CallService callService, AppColors colors) {
    final call = callService.currentCall!;
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          CircleAvatar(
            radius: 40,
            backgroundColor: colors.bg2,
            child: Text(
              (call.name ?? call.number).isNotEmpty
                  ? (call.name ?? call.number)[0].toUpperCase()
                  : '?',
              style: TextStyle(fontSize: 32, color: colors.accent, fontWeight: FontWeight.bold),
            ),
          ),
          const SizedBox(height: 16),
          Text(
            call.name ?? call.number,
            style: TextStyle(fontSize: 24, fontWeight: FontWeight.bold, color: colors.text1),
          ),
          const SizedBox(height: 4),
          Text(
            call.number,
            style: TextStyle(fontSize: 14, color: colors.text2),
          ),
          const SizedBox(height: 8),
          Text(
            'Incoming call...',
            style: TextStyle(fontSize: 13, color: colors.accent),
          ),
          const SizedBox(height: 48),
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              _buildCallButton(
                icon: Icons.call_end_outlined,
                color: colors.error,
                label: 'Reject',
                onTap: () => callService.rejectCall(),
                colors: colors,
              ),
              const SizedBox(width: 32),
              _buildCallButton(
                icon: Icons.call_outlined,
                color: colors.accent,
                label: 'Answer',
                onTap: () => callService.answerCall(),
                colors: colors,
              ),
            ],
          ),
          const SizedBox(height: 24),
          TextButton(
            onPressed: () => callService.forwardCall('desktop'),
            child: Text('Forward to Desktop', style: TextStyle(color: colors.accent)),
          ),
        ],
      ),
    );
  }

  Widget _buildActiveCall(BuildContext context, CallService callService, AppColors colors) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.phone_in_talk_outlined, size: 48, color: colors.accent),
          const SizedBox(height: 16),
          Text(
            'Call in progress',
            style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold, color: colors.text1),
          ),
          const SizedBox(height: 32),
          _buildCallButton(
            icon: Icons.call_end_outlined,
            color: colors.error,
            label: 'End',
            onTap: () => callService.rejectCall(),
            colors: colors,
          ),
        ],
      ),
    );
  }

  Widget _buildCallButton({
    required IconData icon,
    required Color color,
    required String label,
    required VoidCallback onTap,
    required AppColors colors,
  }) {
    return GestureDetector(
      onTap: onTap,
      child: Column(
        children: [
          Container(
            width: 64,
            height: 64,
            decoration: BoxDecoration(
              color: color,
              shape: BoxShape.circle,
            ),
            child: Icon(icon, color: Colors.white, size: 28),
          ),
          const SizedBox(height: 8),
          Text(label, style: TextStyle(fontSize: 12, color: colors.text2)),
        ],
      ),
    );
  }

  Widget _buildIdleState(AppColors colors) {
    return const EmptyState(
      icon: Icons.phone_disabled_outlined,
      title: 'No active calls',
      description: 'Incoming calls will appear here',
    );
  }
}
