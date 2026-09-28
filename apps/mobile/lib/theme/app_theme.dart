import 'package:flutter/material.dart';

/// Semantic color tokens matching the desktop app's CSS custom properties.
@immutable
class AppColors extends ThemeExtension<AppColors> {
  final Color bg0;
  final Color bg1;
  final Color bg2;
  final Color bg3;
  final Color text1;
  final Color text2;
  final Color text3;
  final Color accent;
  final Color accentSecondary;
  final Color border;
  final Color borderStrong;
  final Color error;
  final Color success;
  final Color warning;

  const AppColors({
    required this.bg0,
    required this.bg1,
    required this.bg2,
    required this.bg3,
    required this.text1,
    required this.text2,
    required this.text3,
    required this.accent,
    required this.accentSecondary,
    required this.border,
    required this.borderStrong,
    required this.error,
    required this.success,
    required this.warning,
  });

  static const dark = AppColors(
    bg0: Color(0xFF000000), // Pure black
    bg1: Color(0xFF000000), // Pure black
    bg2: Color(0x0DFFFFFF), // rgba(255, 255, 255, 0.05)
    bg3: Color(0x1AFFFFFF), // rgba(255, 255, 255, 0.1)
    text1: Color(0xFFFFFFFF),
    text2: Color(0xB3FFFFFF), // rgba(255, 255, 255, 0.7)
    text3: Color(0x66FFFFFF), // rgba(255, 255, 255, 0.4)
    accent: Color(0xFF00F0FF),
    accentSecondary: Color(0xFF00F0FF),
    border: Color(0x33FFFFFF), // rgba(255, 255, 255, 0.2)
    borderStrong: Color(0x66FFFFFF), // rgba(255, 255, 255, 0.4)
    error: Color(0xFFEF4444),
    success: Color(0xFF22C55E),
    warning: Color(0xFFFFB300),
  );

  static const light = AppColors(
    bg0: Color(0xFFF8F9FA),
    bg1: Color(0xFFFFFFFF),
    bg2: Color(0xFFF0F1F3),
    bg3: Color(0xFFE4E6E9),
    text1: Color(0xFF1A1C20),
    text2: Color(0xFF6B6E75),
    text3: Color(0xFF9EA1A7),
    accent: Color(0xFF00F0FF),
    accentSecondary: Color(0xFF00F0FF),
    border: Color(0x14000000),
    borderStrong: Color(0x29000000),
    error: Color(0xFFDC2626),
    success: Color(0xFF16A34A),
    warning: Color(0xFFFFB300),
  );

  @override
  AppColors copyWith({
    Color? bg0, Color? bg1, Color? bg2, Color? bg3,
    Color? text1, Color? text2, Color? text3,
    Color? accent, Color? accentSecondary,
    Color? border, Color? borderStrong,
    Color? error, Color? success, Color? warning,
  }) {
    return AppColors(
      bg0: bg0 ?? this.bg0,
      bg1: bg1 ?? this.bg1,
      bg2: bg2 ?? this.bg2,
      bg3: bg3 ?? this.bg3,
      text1: text1 ?? this.text1,
      text2: text2 ?? this.text2,
      text3: text3 ?? this.text3,
      accent: accent ?? this.accent,
      accentSecondary: accentSecondary ?? this.accentSecondary,
      border: border ?? this.border,
      borderStrong: borderStrong ?? this.borderStrong,
      error: error ?? this.error,
      success: success ?? this.success,
      warning: warning ?? this.warning,
    );
  }

  @override
  AppColors lerp(AppColors? other, double t) {
    if (other is! AppColors) return this;
    return AppColors(
      bg0: Color.lerp(bg0, other.bg0, t)!,
      bg1: Color.lerp(bg1, other.bg1, t)!,
      bg2: Color.lerp(bg2, other.bg2, t)!,
      bg3: Color.lerp(bg3, other.bg3, t)!,
      text1: Color.lerp(text1, other.text1, t)!,
      text2: Color.lerp(text2, other.text2, t)!,
      text3: Color.lerp(text3, other.text3, t)!,
      accent: Color.lerp(accent, other.accent, t)!,
      accentSecondary: Color.lerp(accentSecondary, other.accentSecondary, t)!,
      border: Color.lerp(border, other.border, t)!,
      borderStrong: Color.lerp(borderStrong, other.borderStrong, t)!,
      error: Color.lerp(error, other.error, t)!,
      success: Color.lerp(success, other.success, t)!,
      warning: Color.lerp(warning, other.warning, t)!,
    );
  }
}

/// 8 accent color presets matching desktop's accent color picker.
const accentColorPresets = <String, Color>{
  'emerald': Color(0xFF00F0FF),
  'blue': Color(0xFF60A5FA),
  'purple': Color(0xFFA78BFA),
  'pink': Color(0xFFF472B6),
  'orange': Color(0xFFFB923C),
  'yellow': Color(0xFFFBBF24),
  'red': Color(0xFFF87171),
  'cyan': Color(0xFF00F0FF),
};

class AppTheme {
  AppTheme._();

  static ThemeData dark([Color? accent]) {
    final accentColor = accent ?? const Color(0xFF00F0FF);
    return ThemeData(
      brightness: Brightness.dark,
      scaffoldBackgroundColor: AppColors.dark.bg0,
      colorScheme: ColorScheme.dark(
        primary: accentColor,
        secondary: AppColors.dark.accentSecondary,
        surface: AppColors.dark.bg1,
        error: AppColors.dark.error,
      ),
      cardColor: AppColors.dark.bg1,
      dividerColor: AppColors.dark.border,
      fontFamily: 'Inter',
      appBarTheme: AppBarTheme(
        backgroundColor: AppColors.dark.bg0,
        elevation: 0,
        titleTextStyle: TextStyle(
          color: AppColors.dark.text1,
          fontSize: 18,
          fontWeight: FontWeight.w600,
        ),
        iconTheme: IconThemeData(color: AppColors.dark.text2),
      ),
      bottomNavigationBarTheme: BottomNavigationBarThemeData(
        backgroundColor: AppColors.dark.bg0,
        selectedItemColor: accentColor,
        unselectedItemColor: AppColors.dark.text3,
      ),
      extensions: const [AppColors.dark],
    );
  }

  static ThemeData light([Color? accent]) {
    final accentColor = accent ?? const Color(0xFF00F0FF);
    return ThemeData(
      brightness: Brightness.light,
      scaffoldBackgroundColor: AppColors.light.bg0,
      colorScheme: ColorScheme.light(
        primary: accentColor,
        secondary: AppColors.light.accentSecondary,
        surface: AppColors.light.bg1,
        error: AppColors.light.error,
      ),
      cardColor: AppColors.light.bg1,
      dividerColor: AppColors.light.border,
      fontFamily: 'Inter',
      appBarTheme: AppBarTheme(
        backgroundColor: AppColors.light.bg0,
        elevation: 0,
        titleTextStyle: TextStyle(
          color: AppColors.light.text1,
          fontSize: 18,
          fontWeight: FontWeight.w600,
        ),
        iconTheme: IconThemeData(color: AppColors.light.text2),
      ),
      bottomNavigationBarTheme: BottomNavigationBarThemeData(
        backgroundColor: AppColors.light.bg0,
        selectedItemColor: accentColor,
        unselectedItemColor: AppColors.light.text3,
      ),
      extensions: const [AppColors.light],
    );
  }
}
