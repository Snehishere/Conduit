import 'package:flutter/material.dart';
import '../theme/app_theme.dart';

enum BadgeStatus { connected, syncing, offline, error }

/// Compact status indicator badge with dot + label.
class StatusBadge extends StatelessWidget {
  final BadgeStatus status;
  final String? label;

  const StatusBadge({super.key, required this.status, this.label});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = theme.extension<AppColors>();
    final accent = colors?.accent ?? theme.colorScheme.primary;

    final (color, text, animated) = switch (status) {
      BadgeStatus.connected => (accent, label ?? 'Connected', false),
      BadgeStatus.syncing => (accent, label ?? 'Syncing', true),
      BadgeStatus.offline => (colors?.text3 ?? Colors.grey, label ?? 'Offline', false),
      BadgeStatus.error => (colors?.error ?? Colors.red, label ?? 'Error', false),
    };

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: color.withValues(alpha: 0.3)),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (animated)
            SizedBox(
              width: 8,
              height: 8,
              child: CircularProgressIndicator(
                strokeWidth: 1.5,
                color: color,
              ),
            )
          else
            Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: color,
                boxShadow: status == BadgeStatus.connected
                    ? [BoxShadow(color: color.withValues(alpha: 0.4), blurRadius: 6)]
                    : null,
              ),
            ),
          const SizedBox(width: 6),
          Text(
            text,
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w500,
              color: color,
            ),
          ),
        ],
      ),
    );
  }
}

