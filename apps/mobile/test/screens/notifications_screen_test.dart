import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/screens/notifications_screen.dart';
import 'package:conduit/services/notification_service.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/models/conduit_notification.dart';
import 'package:conduit/theme/app_theme.dart';

Widget _app(NotificationService svc) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<NotificationService>.value(value: svc),
      ChangeNotifierProvider<WebSocketService>(
        create: (_) => WebSocketService(),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const NotificationsScreen(),
    ),
  );
}

void main() {
  testWidgets('shows empty state when there are no notifications',
      (tester) async {
    await tester.pumpWidget(_app(NotificationService()));
    await tester.pump();

    expect(find.text('Notifications'), findsOneWidget);
    expect(find.text('No notifications'), findsOneWidget);
    expect(
      find.text('Notifications from paired devices\nwill appear here'),
      findsOneWidget,
    );
  });

  testWidgets('renders seeded notification card', (tester) async {
    final svc = NotificationService();
    svc.addNotification(
      ConduitNotification(
        id: 'n1',
        deviceId: 'desktop-1',
        app: 'WhatsApp',
        title: 'New message',
        body: 'Hello from desktop',
        timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
      ),
    );

    await tester.pumpWidget(_app(svc));
    await tester.pump();

    expect(find.text('New message'), findsOneWidget);
    expect(find.text('Hello from desktop'), findsOneWidget);
    expect(find.text('WhatsApp'), findsOneWidget);
  });
}
