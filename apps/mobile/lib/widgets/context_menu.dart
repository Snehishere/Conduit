import 'package:flutter/material.dart';
import '../theme/app_theme.dart';

/// A single item in a context menu, or a visual divider.
class ContextMenuItem {
  final IconData? icon;
  final String label;
  final bool danger;
  final VoidCallback? onTap;

  const ContextMenuItem({
    this.icon,
    required this.label,
    this.danger = false,
    this.onTap,
  });

  const ContextMenuItem.divider()
      : icon = null,
        label = '',
        danger = false,
        onTap = null;

  bool get isDivider => icon == null && label.isEmpty && onTap == null;
}

/// Wraps a child widget with a long-press gesture that shows a context menu.
class ContextMenu extends StatelessWidget {
  final List<ContextMenuItem> items;
  final Widget child;

  const ContextMenu({super.key, required this.items, required this.child});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onLongPress: () => _showMenu(context),
      child: child,
    );
  }

  void _showMenu(BuildContext context) {
    final theme = Theme.of(context);
    final colors = theme.extension<AppColors>()!;

    showModalBottomSheet(
      context: context,
      backgroundColor: Colors.transparent,
      builder: (_) => Container(
        margin: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: colors.bg2,
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: colors.border),
        ),
        child: SafeArea(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              // Drag handle
              Container(
                width: 36,
                height: 4,
                margin: const EdgeInsets.only(top: 12, bottom: 8),
                decoration: BoxDecoration(
                  color: colors.text3.withValues(alpha: 0.4),
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
              ...items.map((item) {
                if (item.isDivider) {
                  return Divider(height: 1, color: colors.border);
                }
                return Material(
                  type: MaterialType.transparency,
                  child: ListTile(
                    leading: Icon(
                      item.icon,
                      size: 20,
                      color: item.danger ? colors.error : colors.text2,
                    ),
                    title: Text(
                      item.label,
                      style: TextStyle(
                        fontSize: 14,
                        color: item.danger ? colors.error : colors.text1,
                      ),
                    ),
                    onTap: () {
                      Navigator.pop(context);
                      item.onTap?.call();
                    },
                  ),
                );
              }),
              const SizedBox(height: 4),
            ],
          ),
        ),
      ),
    );
  }
}
