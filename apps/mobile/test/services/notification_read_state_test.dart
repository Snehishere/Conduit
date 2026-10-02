import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:conduit/models/conduit_notification.dart';
import 'package:conduit/screens/notifications_screen.dart';
import 'package:conduit/services/database_service.dart';
import 'package:conduit/services/notification_service.dart';
import 'package:conduit/services/websocket_service.dart';
import 'package:conduit/theme/app_theme.dart';

ConduitNotification _seed({bool read = false}) => ConduitNotification(
      id: 'n1',
      deviceId: 'desktop-1',
      app: 'WhatsApp',
      title: 'New message',
      body: 'Hello from desktop',
      timestamp: 1700000000,
      read: read,
    );

Widget _app(NotificationService svc) {
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<NotificationService>.value(value: svc),
      ChangeNotifierProvider<WebSocketService>(create: (_) => WebSocketService()),
    ],
    child: MaterialApp(
      theme: AppTheme.dark(),
      home: const NotificationsScreen(),
    ),
  );
}

void main() {
  test('read state survives the SQLite round-trip', () {
    expect(ConduitNotification.fromMap(_seed(read: true).toMap()).read, isTrue);
    expect(ConduitNotification.fromMap(_seed().toMap()).read, isFalse);
    expect(ConduitNotification.fromJson(_seed(read: true).toJson()).read, isTrue);
  });

  test('a row written before the read column existed reads as unread', () {
    final legacy = _seed().toMap()..remove('read');
    expect(ConduitNotification.fromMap(legacy).read, isFalse);
  });

  test('the notification_history schema has somewhere to put read', () {
    expect(DatabaseService.notificationTableDdl, contains('read'));
    expect(DatabaseService.notificationReadColumnUpgrade,
        contains('ALTER TABLE notification_history'));
    expect(DatabaseService.notificationReadColumnUpgrade, contains('read'));
  });

  test('markReadNotification marks the entry read instead of dropping it', () {
    final svc = NotificationService();
    svc.addNotification(_seed());
    var notified = 0;
    svc.addListener(() => notified++);

    svc.markReadNotification('n1');

    expect(svc.notifications, hasLength(1),
        reason: 'marking read must not delete the notification');
    expect(svc.notifications.single.read, isTrue);
    expect(notified, greaterThan(0));
  });

  test('markReadNotification on an unknown id does nothing', () {
    final svc = NotificationService();
    svc.addNotification(_seed());
    var notified = 0;
    svc.addListener(() => notified++);

    svc.markReadNotification('nope');

    expect(svc.notifications.single.read, isFalse);
    expect(notified, 0);
  });

  test('marking read twice is idempotent', () {
    final svc = NotificationService();
    svc.addNotification(_seed());
    svc.markReadNotification('n1');
    var notified = 0;
    svc.addListener(() => notified++);

    svc.markReadNotification('n1');

    expect(svc.notifications.single.read, isTrue);
    expect(notified, 0, reason: 'no state change, so no notification');
  });

  testWidgets('swiping a card to mark it read keeps it in the list',
      (tester) async {
    final svc = NotificationService();
    svc.addNotification(_seed());

    await tester.pumpWidget(_app(svc));
    await tester.pump();

    expect(find.text('New message'), findsOneWidget);

    await tester.drag(find.byType(Card).first, const Offset(-500, 0));
    await tester.pumpAndSettle();

    expect(find.text('New message'), findsOneWidget,
        reason: 'a read notification is still a notification');
    expect(svc.notifications, hasLength(1));
    expect(svc.notifications.single.read, isTrue);
  });

  testWidgets('swiping the other way still dismisses it', (tester) async {
    final svc = NotificationService();
    svc.addNotification(_seed());

    await tester.pumpWidget(_app(svc));
    await tester.pump();

    await tester.drag(find.byType(Card).first, const Offset(500, 0));
    await tester.pumpAndSettle();

    expect(svc.notifications, isEmpty);
  });
}