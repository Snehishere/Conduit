import 'package:flutter/material.dart';
import '../theme/app_theme.dart';

class MessageBubble extends StatelessWidget {
  final String body;
  final int timestamp;
  final bool isOutgoing;
  final bool read;

  const MessageBubble({
    super.key,
    required this.body,
    required this.timestamp,
    required this.isOutgoing,
    this.read = false,
  });

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Align(
      alignment: isOutgoing ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.of(context).size.width * 0.75,
        ),
        margin: EdgeInsets.only(
          left: isOutgoing ? 48 : 8,
          right: isOutgoing ? 8 : 48,
          top: 2,
          bottom: 2,
        ),
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
        decoration: BoxDecoration(
          color: isOutgoing
              ? colors.accent.withValues(alpha: 0.12)
              : colors.bg2,
          borderRadius: BorderRadius.only(
            topLeft: const Radius.circular(16),
            topRight: const Radius.circular(16),
            bottomLeft: Radius.circular(isOutgoing ? 16 : 4),
            bottomRight: Radius.circular(isOutgoing ? 4 : 16),
          ),
          border: Border.all(
            color: isOutgoing
                ? colors.accent.withValues(alpha: 0.2)
                : colors.border,
            width: 1,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            Text(
              body,
              style: TextStyle(
                color: isOutgoing ? colors.accent : colors.text1,
                fontSize: 14,
                height: 1.3,
              ),
            ),
            const SizedBox(height: 4),
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  _formatTime(timestamp),
                  style: TextStyle(
                    color: isOutgoing
                        ? colors.accent.withValues(alpha: 0.6)
                        : colors.text3,
                    fontSize: 10,
                  ),
                ),
                if (isOutgoing) ...[
                  const SizedBox(width: 4),
                  Icon(
                    read ? Icons.done_all : Icons.done,
                    size: 12,
                    color: isOutgoing
                        ? colors.accent.withValues(alpha: 0.6)
                        : colors.text3,
                  ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }

  String _formatTime(int timestamp) {
    final date = DateTime.fromMillisecondsSinceEpoch(timestamp * 1000);
    return '${date.hour.toString().padLeft(2, '0')}:${date.minute.toString().padLeft(2, '0')}';
  }
}
