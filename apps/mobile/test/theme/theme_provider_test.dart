import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/theme/theme_provider.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('ThemeProvider', () {
    test('defaults to dark theme', () {
      final provider = ThemeProvider();
      expect(provider.themeMode, ThemeMode.dark);
    });

    test('darkTheme uses accent color', () {
      final provider = ThemeProvider();
      final theme = provider.darkTheme;
      expect(theme.colorScheme.primary, provider.accentColor);
    });

    test('lightTheme uses accent color', () {
      final provider = ThemeProvider();
      final theme = provider.lightTheme;
      expect(theme.colorScheme.primary, provider.accentColor);
    });
  });
}
