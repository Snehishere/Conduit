import 'package:flutter/material.dart';
import '../theme/app_theme.dart';

/// Reusable search input field with clear button.
class ConduitSearchBar extends StatelessWidget {
  final String hint;
  final ValueChanged<String> onChanged;
  final VoidCallback? onClear;
  final TextEditingController? controller;
  final bool autofocus;

  const ConduitSearchBar({
    super.key,
    this.hint = 'Search...',
    required this.onChanged,
    this.onClear,
    this.controller,
    this.autofocus = false,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = theme.extension<AppColors>()!;

    return Container(
      height: 40,
      decoration: BoxDecoration(
        color: colors.bg2,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: colors.border),
      ),
      child: TextField(
        controller: controller,
        autofocus: autofocus,
        onChanged: onChanged,
        style: TextStyle(fontSize: 13, color: colors.text1),
        decoration: InputDecoration(
          hintText: hint,
          hintStyle: TextStyle(color: colors.text3, fontSize: 13),
          prefixIcon: Icon(Icons.search, size: 18, color: colors.text3),
          suffixIcon: controller != null && (controller?.text.isNotEmpty ?? false)
              ? IconButton(
                  icon: Icon(Icons.close, size: 16, color: colors.text3),
                  onPressed: () {
                    controller!.clear();
                    onChanged('');
                    onClear?.call();
                  },
                )
              : null,
          border: InputBorder.none,
          contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        ),
      ),
    );
  }
}
