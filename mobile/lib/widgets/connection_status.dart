import 'package:flutter/material.dart';

class ConnectionStatus extends StatelessWidget {
  final bool connected;

  const ConnectionStatus({super.key, required this.connected});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      decoration: BoxDecoration(
        color: connected
            ? const Color(0xFF2DD4BF).withValues(alpha: 0.1)
            : const Color(0xFFF87171).withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(20),
        border: Border.all(
          color: connected
              ? const Color(0xFF2DD4BF).withValues(alpha: 0.3)
              : const Color(0xFFF87171).withValues(alpha: 0.3),
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
              color: connected
                  ? const Color(0xFF2DD4BF)
                  : const Color(0xFFF87171),
              boxShadow: connected
                  ? [BoxShadow(color: const Color(0xFF2DD4BF).withValues(alpha: 0.4), blurRadius: 6)]
                  : null,
            ),
          ),
          const SizedBox(width: 6),
          Text(
            connected ? 'Connected' : 'Disconnected',
            style: TextStyle(
              fontSize: 12,
              color: connected
                  ? const Color(0xFF2DD4BF)
                  : const Color(0xFFF87171),
              fontWeight: FontWeight.w500,
            ),
          ),
        ],
      ),
    );
  }
}
