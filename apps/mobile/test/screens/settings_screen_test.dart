import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:conduit/screens/settings_screen.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/services/clipboard_service.dart';
import 'package:conduit/services/file_service.dart';
import 'package:conduit/theme/theme_provider.dart';
import 'package:conduit/theme/app_theme.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  Widget app(Widget home) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider<ThemeProvider>(
          create: (_) => ThemeProvider(),
        ),
        ChangeNotifierProvider<WebSocketService>(
          create: (_) => WebSocketService(),
        ),
        ChangeNotifierProvider<ClipboardService>(
          create: (_) => ClipboardService(),
        ),
        ChangeNotifierProvider<FileService>(
          create: (_) => FileService(),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.dark(),
        home: home,
      ),
    );
  }

  testWidgets('renders settings sections and app bar', (tester) async {
    await tester.pumpWidget(app(const SettingsScreen()));
    await tester.pump();

    expect(find.text('Settings'), findsOneWidget);
    expect(find.text('DEVICE'), findsOneWidget);
    expect(find.text('This device'), findsOneWidget);
    expect(find.text('Hub Address'), findsOneWidget);
    expect(find.text('APPEARANCE'), findsOneWidget);
    expect(find.text('Theme'), findsOneWidget);
    expect(find.text('Accent color'), findsOneWidget);
    expect(find.byType(SegmentedButton<ThemeMode>), findsOneWidget);
  });
}
