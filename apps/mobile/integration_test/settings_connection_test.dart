import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:conduit/screens/home_screen.dart';
import 'package:conduit/screens/settings_screen.dart';
import 'package:conduit/services/call_service.dart';
import 'package:conduit/services/clipboard_service.dart';
import 'package:conduit/services/file_service.dart';
import 'package:conduit/services/notification_service.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/theme/app_theme.dart';
import 'package:conduit/theme/theme_provider.dart';

/// Providers required to pump HomeScreen / SettingsScreen without the full app.
class _TestHarness {
  final ThemeProvider themeProvider = ThemeProvider();
  final WebSocketService ws = WebSocketService();
  final NotificationService notifications = NotificationService();
  final ClipboardService clipboard = ClipboardService();
  final FileService files = FileService();
  final CallService calls = CallService();

  Widget wrap(Widget child) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider<ThemeProvider>.value(value: themeProvider),
        ChangeNotifierProvider<WebSocketService>.value(value: ws),
        ChangeNotifierProvider<NotificationService>.value(value: notifications),
        ChangeNotifierProvider<ClipboardService>.value(value: clipboard),
        ChangeNotifierProvider<FileService>.value(value: files),
        ChangeNotifierProvider<CallService>.value(value: calls),
      ],
      child: MaterialApp(
        // SettingsScreen asserts Theme.of(context).extension<AppColors>()!
        theme: AppTheme.dark(themeProvider.accentColor),
        darkTheme: AppTheme.dark(themeProvider.accentColor),
        themeMode: ThemeMode.dark,
        home: child,
      ),
    );
  }

  void dispose() {
    ws.dispose();
  }
}

/// Pump enough frames for entrance/float animations to settle without
/// pumpAndSettle (DeviceHub's float controller repeats forever).
Future<void> pumpFrames(WidgetTester tester, {int frames = 20}) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  group('Settings navigation from Home', () {
    late _TestHarness harness;

    setUp(() {
      SharedPreferences.setMockInitialValues({});
      harness = _TestHarness();
    });

    tearDown(() {
      harness.dispose();
    });

    testWidgets(
      'bottom-nav Settings pushes SettingsScreen with Sync Notifications toggle',
      (tester) async {
        await tester.pumpWidget(harness.wrap(const HomeScreen()));
        await pumpFrames(tester);

        // Home is visible
        expect(find.byType(HomeScreen), findsOneWidget);

        // Tap the Settings bottom-nav item (index 3)
        await tester.tap(find.byIcon(Icons.settings_outlined).last);
        // HomeScreen pushes Settings as a full-screen route — bounded pumps.
        await pumpFrames(tester, frames: 30);

        // SettingsScreen is now on the stack
        expect(find.byType(SettingsScreen), findsOneWidget);
        expect(find.text('Settings'), findsWidgets);

        // Key settings section is visible
        expect(find.text('Sync Notifications'), findsOneWidget);
        expect(find.text('Hub Address'), findsOneWidget);
        expect(find.text('This device'), findsOneWidget);

        // Connection status reflects disconnected WebSocket
        expect(find.text('Disconnected'), findsOneWidget);
        expect(find.text('Not connected'), findsOneWidget);

        // Navigate back to Home
        final back = find.byType(BackButton);
        expect(back, findsOneWidget);
        await tester.tap(back);
        await pumpFrames(tester, frames: 30);
        expect(find.byType(SettingsScreen), findsNothing);
        expect(find.byType(HomeScreen), findsOneWidget);
      },
    );
  });

  group('Settings connection status display', () {
    testWidgets(
      'shows Disconnected then Connected when WebSocket connects',
      (tester) async {
        SharedPreferences.setMockInitialValues({});
        final harness = _TestHarness();

        // Local WebSocket echo server (same pattern as websocket_service_test)
        final server = await HttpServer.bind('localhost', 0);
        final port = server.port;
        server.transform(WebSocketTransformer()).listen((socket) {
          socket.listen((_) {});
        });
        // The address the app must display: the real peer endpoint it dialed.
        // It previously hardcoded `ws://localhost:9527`, which is a different
        // socket than the one actually connected to (and the wrong port for
        // the TLS listener the desktop really serves LAN WSS on).
        final connectedUrl = 'ws://localhost:$port';

        // embedded: false (default) — embedded mode returns a bare ListView
        // with no Material ancestor, which crashes ListTile.
        await tester.pumpWidget(harness.wrap(const SettingsScreen()));
        await pumpFrames(tester);

        // Initial: disconnected
        expect(find.byType(SettingsScreen), findsOneWidget);
        expect(find.text('Disconnected'), findsOneWidget);
        expect(find.text('Not connected'), findsOneWidget);
        expect(find.text('Connected'), findsNothing);
        // No placeholder address may be shown while disconnected.
        expect(find.textContaining('ws://'), findsNothing);
        expect(find.textContaining('wss://'), findsNothing);

        // Connect the shared WebSocketService
        await harness.ws.connect(connectedUrl);
        await pumpFrames(tester, frames: 10);

        // UI reflects the connected state
        expect(find.text('Connected'), findsOneWidget);
        expect(find.text('Disconnected'), findsNothing);
        // The real address/scheme/port that was dialled, not a hardcoded one.
        expect(find.text(connectedUrl), findsOneWidget);
        expect(find.text('Not connected'), findsNothing);
        // The TLS LAN port must never be claimed on a plaintext connection.
        expect(find.textContaining('9531'), findsNothing);
        expect(find.textContaining('9527'), findsNothing);

        // Disconnect → back to Disconnected
        harness.ws.dispose();
        await pumpFrames(tester, frames: 10);
        expect(find.text('Disconnected'), findsOneWidget);
        expect(find.text('Not connected'), findsOneWidget);
        expect(find.text(connectedUrl), findsNothing);

        await server.close(force: true);
      },
    );
  });
}
