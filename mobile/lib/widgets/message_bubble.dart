import 'package:flutter/material.dart';

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
              ? const Color(0xFF1A2E2B)
              : const Color(0xFF16161E),
          borderRadius: BorderRadius.only(
            topLeft: const Radius.circular(16),
            topRight: const Radius.circular(16),
            bottomLeft: Radius.circular(isOutgoing ? 16 : 4),
            bottomRight: Radius.circular(isOutgoing ? 4 : 16),
          ),
          border: Border.all(
            color: isOutgoing
                ? const Color(0xFF2DD4BF).withValues(alpha: 0.2)
                : const Color(0xFF1E1E28),
            width: 1,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            Text(
              body,
              style: TextStyle(
                color: isOutgoing ? const Color(0xFF5EEAD4) : const Color(0xFFE2E0E8),
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
                        ? Colors.white.withValues(alpha: 0.6)
                        : const Color(0xFF606070),
                    fontSize: 10,
                  ),
                ),
                if (isOutgoing) ...[
                  const SizedBox(width: 4),
                  Icon(
                    read ? Icons.done_all : Icons.done,
                    size: 12,
                    color: isOutgoing
                        ? Colors.white.withValues(alpha: 0.6)
                        : const Color(0xFF606070),
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
