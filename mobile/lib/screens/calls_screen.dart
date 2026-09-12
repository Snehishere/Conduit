import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/call_service.dart';
import '../services/audio_stream_service.dart';

class CallsScreen extends StatelessWidget {
  const CallsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Calls'),
      ),
      body: Consumer2<CallService, AudioStreamService>(
        builder: (_, callService, audioStream, __) {
          if (callService.state == CallState.ringing && callService.currentCall != null) {
            return _buildIncomingCall(context, callService);
          }

          return Column(
            children: [
              _buildAudioRouteSelector(callService),
              _buildAudioStreamControls(audioStream),
              const Divider(height: 1),
              Expanded(
                child: callService.state == CallState.idle
                    ? _buildIdleState()
                    : _buildActiveCall(context, callService),
              ),
            ],
          );
        },
      ),
    );
  }

  Widget _buildAudioRouteSelector(CallService callService) {
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
              color: Colors.grey.shade500,
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
              ),
              const SizedBox(width: 8),
              _buildRouteButton(
                icon: Icons.computer_outlined,
                label: 'Desktop',
                active: callService.currentRoute == AudioOutputRoute.desktop,
                onTap: () => callService.setAudioRoute(AudioOutputRoute.desktop),
              ),
              const SizedBox(width: 8),
              _buildRouteButton(
                icon: Icons.bluetooth_outlined,
                label: 'Bluetooth',
                active: callService.currentRoute == AudioOutputRoute.bluetooth,
                onTap: () => callService.setAudioRoute(AudioOutputRoute.bluetooth),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildAudioStreamControls(AudioStreamService audioStream) {
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
              color: Colors.grey.shade500,
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
                          ? const Color(0xFF1A2E2B)
                          : const Color(0xFF101018),
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(
                        color: audioStream.isStreaming
                            ? const Color(0xFF2DD4BF).withValues(alpha: 0.4)
                            : const Color(0xFF1E1E28),
                      ),
                    ),
                    child: Column(
                      children: [
                        Icon(
                          audioStream.isStreaming ? Icons.mic : Icons.mic_off,
                          color: audioStream.isStreaming
                              ? const Color(0xFF2DD4BF)
                              : const Color(0xFF5C586A),
                          size: 20,
                        ),
                        const SizedBox(height: 4),
                        Text(
                          audioStream.isStreaming ? 'Streaming' : 'Start Stream',
                          style: TextStyle(
                            fontSize: 11,
                            color: audioStream.isStreaming
                                ? const Color(0xFF2DD4BF)
                                : const Color(0xFF5C586A),
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
                          ? const Color(0xFF1A2E2B)
                          : const Color(0xFF101018),
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(
                        color: audioStream.isPlaying
                            ? const Color(0xFF2DD4BF).withValues(alpha: 0.4)
                            : const Color(0xFF1E1E28),
                      ),
                    ),
                    child: Column(
                      children: [
                        Icon(
                          audioStream.isPlaying ? Icons.stop : Icons.volume_up,
                          color: audioStream.isPlaying
                              ? const Color(0xFF2DD4BF)
                              : const Color(0xFF5C586A),
                          size: 20,
                        ),
                        const SizedBox(height: 4),
                        Text(
                          audioStream.isPlaying ? 'Playing' : 'Playback',
                          style: TextStyle(
                            fontSize: 11,
                            color: audioStream.isPlaying
                                ? const Color(0xFF2DD4BF)
                                : const Color(0xFF5C586A),
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
  }) {
    return Expanded(
      child: GestureDetector(
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.symmetric(vertical: 12),
          decoration: BoxDecoration(
            color: active ? const Color(0xFF1A2E2B) : const Color(0xFF101018),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: active ? const Color(0xFF2DD4BF).withValues(alpha: 0.4) : const Color(0xFF1E1E28),
            ),
          ),
          child: Column(
            children: [
              Icon(icon, color: active ? const Color(0xFF2DD4BF) : const Color(0xFF5C586A), size: 20),
              const SizedBox(height: 4),
              Text(
                label,
                style: TextStyle(
                  fontSize: 11,
                  color: active ? const Color(0xFF2DD4BF) : const Color(0xFF5C586A),
                  fontWeight: active ? FontWeight.w600 : FontWeight.normal,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildIncomingCall(BuildContext context, CallService callService) {
    final call = callService.currentCall!;
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          CircleAvatar(
            radius: 40,
            backgroundColor: const Color(0xFF16161E),
            child: Text(
              (call.name ?? call.number).isNotEmpty
                  ? (call.name ?? call.number)[0].toUpperCase()
                  : '?',
              style: const TextStyle(fontSize: 32, color: Color(0xFF2DD4BF), fontWeight: FontWeight.bold),
            ),
          ),
          const SizedBox(height: 16),
          Text(
            call.name ?? call.number,
            style: const TextStyle(fontSize: 24, fontWeight: FontWeight.bold, color: Color(0xFFE8E8F0)),
          ),
          const SizedBox(height: 4),
          Text(
            call.number,
            style: const TextStyle(fontSize: 14, color: Color(0xFFA0A0B0)),
          ),
          const SizedBox(height: 8),
          const Text(
            'Incoming call...',
            style: TextStyle(fontSize: 13, color: Color(0xFF2DD4BF)),
          ),
          const SizedBox(height: 48),
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              _buildCallButton(
                icon: Icons.call_end_outlined,
                color: const Color(0xFFEF4444),
                label: 'Reject',
                onTap: () => callService.rejectCall(),
              ),
              const SizedBox(width: 32),
              _buildCallButton(
                icon: Icons.call_outlined,
                color: const Color(0xFF2DD4BF),
                label: 'Answer',
                onTap: () => callService.answerCall(),
              ),
            ],
          ),
          const SizedBox(height: 24),
          TextButton(
            onPressed: () => callService.forwardCall('desktop'),
            child: const Text('Forward to Desktop'),
          ),
        ],
      ),
    );
  }

  Widget _buildActiveCall(BuildContext context, CallService callService) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.phone_in_talk_outlined, size: 48, color: Color(0xFF2DD4BF)),
          const SizedBox(height: 16),
          const Text(
            'Call in progress',
            style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold, color: Color(0xFFE8E8F0)),
          ),
          const SizedBox(height: 32),
          _buildCallButton(
            icon: Icons.call_end_outlined,
            color: const Color(0xFFEF4444),
            label: 'End',
            onTap: () => callService.rejectCall(),
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
          Text(label, style: TextStyle(fontSize: 12, color: Colors.grey.shade400)),
        ],
      ),
    );
  }

  Widget _buildIdleState() {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.phone_disabled_outlined, size: 64, color: const Color(0xFF55515E)),
          const SizedBox(height: 16),
          Text(
            'No active calls',
            style: TextStyle(fontSize: 16, color: Colors.grey.shade400),
          ),
          const SizedBox(height: 8),
          Text(
            'Incoming calls will appear here',
            style: TextStyle(fontSize: 13, color: Colors.grey.shade600),
          ),
        ],
      ),
    );
  }
}
