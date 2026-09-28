import 'package:flutter/material.dart';
import '../theme/app_theme.dart';

class ConnectionStatus extends StatelessWidget {
  final bool connected;
  final String? lastError;

  const ConnectionStatus({super.key, required this.connected, this.lastError});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final color = connected ? colors.accent : colors.error;

    return Tooltip(
      message: connected
          ? 'Connected to your desktop'
          : (lastError ?? 'Disconnected — retrying the connection'),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        decoration: BoxDecoration(
          color: color.withValues(alpha: 0.1),
          borderRadius: BorderRadius.circular(20),
          border: Border.all(
            color: color.withValues(alpha: 0.3),
            width: 1,
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: color,
                boxShadow: connected
                    ? [BoxShadow(color: color.withValues(alpha: 0.4), blurRadius: 6)]
                    : null,
              ),
            ),
            const SizedBox(width: 6),
            Text(
              connected ? 'Connected' : 'Disconnected',
              style: TextStyle(
                fontSize: 12,
                color: color,
                fontWeight: FontWeight.w500,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
