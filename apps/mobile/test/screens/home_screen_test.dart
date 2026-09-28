import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/home_screen.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/services/call_service.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(Widget home) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<WebSocketService>(
        create: (_) => WebSocketService(),
      ),
      ChangeNotifierProvider<CallService>(create: (_) => CallService()),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: home,
    ),
  );
}

void main() {
  // DeviceHub runs a repeating animation — use pump() instead of pumpAndSettle.
  testWidgets('renders app bar, device hub, and bottom navigation',
      (tester) async {
    await tester.pumpWidget(_app(const HomeScreen()));
    await tester.pump();

    expect(find.text('Conduit'), findsOneWidget);
    expect(find.text('Quick access'), findsOneWidget);
    expect(find.text('No devices connected'), findsOneWidget);
    expect(find.text('Offline'), findsOneWidget);
    expect(find.text('Home'), findsOneWidget);
    expect(find.text('Notifications'), findsOneWidget);
    expect(find.text('Add'), findsOneWidget);
    expect(find.text('Settings'), findsOneWidget);
    expect(find.byType(BottomNavigationBar), findsOneWidget);
  });

  testWidgets('shows quick access cards for clipboard and files',
      (tester) async {
    await tester.pumpWidget(_app(const HomeScreen()));
    await tester.pump();

    expect(find.text('Clipboard'), findsOneWidget);
    expect(find.text('View synced items'), findsOneWidget);
    expect(find.text('Files'), findsOneWidget);
    expect(find.text('View transfers'), findsOneWidget);
  });
}
