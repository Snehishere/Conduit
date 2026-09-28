import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/call_service.dart';
import '../theme/app_theme.dart';

/// Full-screen overlay that appears when there's an incoming or active call.
class IncomingCallOverlay extends StatelessWidget {
  const IncomingCallOverlay({super.key});

  @override
  Widget build(BuildContext context) {
    return Consumer<CallService>(
      builder: (_, callService, _) {
        if (callService.state == CallState.idle) {
          return const SizedBox.shrink();
        }

        if (callService.state == CallState.ringing && callService.currentCall != null) {
          return _buildIncomingCall(context, callService);
        }

        if (callService.state == CallState.active && callService.currentCall != null) {
          return _buildActiveCall(context, callService);
        }

        return const SizedBox.shrink();
      },
    );
  }

  Widget _buildIncomingCall(BuildContext context, CallService callService) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final call = callService.currentCall!;

    return Positioned.fill(
      child: Container(
        color: Colors.black.withValues(alpha: 0.85),
        child: SafeArea(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              CircleAvatar(
                radius: 44,
                backgroundColor: colors.bg2,
                child: Text(
                  (call.name ?? call.number).isNotEmpty
                      ? (call.name ?? call.number)[0].toUpperCase()
                      : '?',
                  style: TextStyle(fontSize: 36, color: colors.accent, fontWeight: FontWeight.bold),
                ),
              ),
              const SizedBox(height: 20),
              Text(
                call.name ?? call.number,
                style: TextStyle(fontSize: 26, fontWeight: FontWeight.bold, color: colors.text1),
              ),
              const SizedBox(height: 6),
              Text(
                call.number,
                style: TextStyle(fontSize: 15, color: colors.text2),
              ),
              const SizedBox(height: 12),
              Text(
                'Incoming call...',
                style: TextStyle(fontSize: 14, color: colors.accent),
              ),
              const SizedBox(height: 56),
              Text(
                '← Reject          Accept →',
                style: TextStyle(
                  fontSize: 12,
                  color: Colors.white.withValues(alpha: 0.3),
                  letterSpacing: 1,
                ),
              ),
              const SizedBox(height: 16),
              Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  _buildCallButton(
                    icon: Icons.call_end_outlined,
                    color: colors.error,
                    label: 'Reject',
                    onTap: () => callService.rejectCall(),
                  ),
                  const SizedBox(width: 48),
                  _buildCallButton(
                    icon: Icons.call_outlined,
                    color: colors.accent,
                    label: 'Accept',
                    onTap: () => callService.answerCall(),
                  ),
                ],
              ),
              const SizedBox(height: 28),
              TextButton(
                onPressed: () => callService.forwardCall('desktop'),
                child: Text('Forward to Desktop', style: TextStyle(color: colors.text2)),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildActiveCall(BuildContext context, CallService callService) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final call = callService.currentCall!;

    return Positioned.fill(
      child: Container(
        color: Colors.black.withValues(alpha: 0.85),
        child: SafeArea(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              CircleAvatar(
                radius: 44,
                backgroundColor: colors.bg2,
                child: Text(
                  (call.name ?? call.number).isNotEmpty
                      ? (call.name ?? call.number)[0].toUpperCase()
                      : '?',
                  style: TextStyle(fontSize: 36, color: colors.accent, fontWeight: FontWeight.bold),
                ),
              ),
              const SizedBox(height: 20),
              Text(
                call.name ?? call.number,
                style: TextStyle(fontSize: 26, fontWeight: FontWeight.bold, color: colors.text1),
              ),
              const SizedBox(height: 6),
              Text(
                call.number,
                style: TextStyle(fontSize: 15, color: colors.text2),
              ),
              const SizedBox(height: 16),
              Icon(Icons.phone_in_talk_outlined, size: 32, color: colors.accent),
              const SizedBox(height: 12),
              Text(
                'Call in progress',
                style: TextStyle(fontSize: 14, color: colors.accent),
              ),
              const SizedBox(height: 56),
              _buildCallButton(
                icon: Icons.call_end_outlined,
                color: colors.error,
                label: 'End call',
                onTap: () => callService.rejectCall(),
              ),
            ],
          ),
        ),
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
            width: 68,
            height: 68,
            decoration: BoxDecoration(
              color: color,
              shape: BoxShape.circle,
              boxShadow: [
                BoxShadow(
                  color: color.withValues(alpha: 0.3),
                  blurRadius: 16,
                  spreadRadius: 2,
                ),
              ],
            ),
            child: Icon(icon, color: Colors.white, size: 30),
          ),
          const SizedBox(height: 10),
          Text(
            label,
            style: TextStyle(fontSize: 13, color: Colors.grey.shade400),
          ),
        ],
      ),
    );
  }
}
