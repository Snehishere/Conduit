import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'app_theme.dart';

/// Persists theme mode and accent color choice across app restarts.
class ThemeProvider extends ChangeNotifier {
  static const _themeKey = 'theme_mode';
  static const _accentKey = 'accent_color';

  ThemeMode _themeMode = ThemeMode.dark;
  Color _accentColor = const Color(0xFF34D399);

  ThemeMode get themeMode => _themeMode;
  Color get accentColor => _accentColor;

  ThemeData get darkTheme => AppTheme.dark(_accentColor);
  ThemeData get lightTheme => AppTheme.light(_accentColor);

  /// Load persisted theme preferences.
  Future<void> init() async {
    final prefs = await SharedPreferences.getInstance();
    final themeIndex = prefs.getInt(_themeKey) ?? 0;
    _themeMode = ThemeMode.values[themeIndex.clamp(0, ThemeMode.values.length - 1)];

    final accentValue = prefs.getInt(_accentKey);
    if (accentValue != null) {
      _accentColor = Color(accentValue);
    }
    notifyListeners();
  }

  /// Switch theme mode (dark / light / system).
  Future<void> setThemeMode(ThemeMode mode) async {
    _themeMode = mode;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setInt(_themeKey, mode.index);
  }

  /// Change accent color.
  Future<void> setAccentColor(Color color) async {
    _accentColor = color;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setInt(_accentKey, color.toARGB32());
  }
}
