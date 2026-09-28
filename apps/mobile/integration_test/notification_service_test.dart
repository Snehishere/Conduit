import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:conduit/services/database_service.dart';
import 'package:conduit/services/notification_service.dart';
import 'package:conduit/models/conduit_notification.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  // ── Notification In-Memory Management ─────────────────────────────

  group('NotificationService in-memory flows', () {
    testWidgets('addNotification adds to the notifications list', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      final notification = ConduitNotification(
        id: 'n1',
        deviceId: 'desktop_1',
        app: 'Slack',
        title: 'New message',
        body: 'Hello!',
        timestamp: 1700000000,
      );

      service.addNotification(notification);

      expect(service.notifications.length, 1);
      expect(service.notifications[0].id, 'n1');
      expect(service.notifications[0].app, 'Slack');
      expect(service.notifications[0].title, 'New message');

      await db.close();
    });

    testWidgets('addNotification prepends (newest first)', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'n1',
        deviceId: 'd1',
        app: 'A',
        title: 'First',
        body: '',
        timestamp: 100,
      ));
      service.addNotification(ConduitNotification(
        id: 'n2',
        deviceId: 'd1',
        app: 'B',
        title: 'Second',
        body: '',
        timestamp: 200,
      ));

      expect(service.notifications.length, 2);
      expect(service.notifications[0].id, 'n2'); // newest first
      expect(service.notifications[1].id, 'n1');

      await db.close();
    });

    testWidgets('addNotification caps at 100 entries', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      for (var i = 0; i < 110; i++) {
        service.addNotification(ConduitNotification(
          id: 'n_$i',
          deviceId: 'd1',
          app: 'Test',
          title: 'Title $i',
          body: 'Body $i',
          timestamp: 1700000000 + i,
        ));
      }

      expect(service.notifications.length, 100);
      // Oldest should be n_10 (indices 0-9 pruned)
      expect(service.notifications.last.id, 'n_10');
      // Newest should be n_109
      expect(service.notifications.first.id, 'n_109');

      await db.close();
    });

    testWidgets('dismissNotification removes from in-memory list', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'n_del',
        deviceId: 'd1',
        app: 'Test',
        title: 'Delete me',
        body: '',
        timestamp: 100,
      ));
      service.addNotification(ConduitNotification(
        id: 'n_keep',
        deviceId: 'd1',
        app: 'Test',
        title: 'Keep me',
        body: '',
        timestamp: 200,
      ));

      expect(service.notifications.length, 2);

      service.dismissNotification('n_del');

      expect(service.notifications.length, 1);
      expect(service.notifications[0].id, 'n_keep');

      await db.close();
    });

    testWidgets('dismissNotification is safe for non-existent id', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'existing',
        deviceId: 'd1',
        app: 'Test',
        title: 'Exists',
        body: '',
        timestamp: 100,
      ));

      service.dismissNotification('nonexistent');
      expect(service.notifications.length, 1); // unchanged

      await db.close();
    });

    testWidgets('notifications list is unmodifiable', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'n1',
        deviceId: 'd1',
        app: 'Test',
        title: 'Test',
        body: '',
        timestamp: 100,
      ));

      final list = service.notifications;
      expect(() => list.add(ConduitNotification(
            id: 'x',
            deviceId: 'x',
            app: 'x',
            title: 'x',
            body: 'x',
            timestamp: 0,
          )), throwsA(anything));

      await db.close();
    });
  });

  // ── Persistence Integration ──────────────────────────────────────

  group('NotificationService persistence via DatabaseService', () {
    testWidgets('notifications are persisted to SQLite on add', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'persist_1',
        deviceId: 'desktop_1',
        app: 'Mail',
        title: 'New email',
        body: 'You have mail',
        timestamp: 1700000000,
      ));

      // Verify directly in the database
      final rows = await db.getNotifications();
      expect(rows.length, 1);
      expect(rows[0]['id'], 'persist_1');
      expect(rows[0]['app'], 'Mail');
      expect(rows[0]['title'], 'New email');

      await db.close();
    });

    testWidgets('dismissNotification removes from SQLite', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'del_persist',
        deviceId: 'd1',
        app: 'Test',
        title: 'Delete from DB',
        body: '',
        timestamp: 1700000000,
      ));

      // Confirm it's in DB
      var rows = await db.getNotifications();
      expect(rows.length, 1);

      service.dismissNotification('del_persist');

      // Wait for async DB delete
      await Future.delayed(const Duration(milliseconds: 200));

      rows = await db.getNotifications();
      expect(rows, isEmpty);

      await db.close();
    });

    testWidgets('multiple notifications persisted correctly', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      for (var i = 0; i < 5; i++) {
        service.addNotification(ConduitNotification(
          id: 'bulk_$i',
          deviceId: 'd1',
          app: 'App_$i',
          title: 'Title $i',
          body: 'Body $i',
          timestamp: 1700000000 + i,
        ));
      }

      final rows = await db.getNotifications();
      expect(rows.length, 5);

      // Verify ordering (newest first in DB query)
      expect(rows[0]['id'], 'bulk_4');
      expect(rows[4]['id'], 'bulk_0');

      await db.close();
    });
  });

  // ── Notification Loading from DB ─────────────────────────────────

  group('Notification history loading from DB', () {
    testWidgets('setDatabase and initialize loads persisted notifications',
        (tester) async {
      // First session: add notifications
      final db = DatabaseService();
      await db.init();

      await db.insertNotification(ConduitNotification(
        id: 'loaded_1',
        deviceId: 'd1',
        app: 'App',
        title: 'Persisted',
        body: 'Should load on init',
        timestamp: 1700000000,
      ).toMap());

      await db.insertNotification(ConduitNotification(
        id: 'loaded_2',
        deviceId: 'd1',
        app: 'App2',
        title: 'Also persisted',
        body: 'Second one',
        timestamp: 1700000001,
      ).toMap());

      // Second session: new NotificationService should load from DB
      final service2 = NotificationService();
      service2.setDatabase(db);
      await service2.initialize();

      expect(service2.notifications.length, 2);
      expect(service2.notifications[0].id, 'loaded_2'); // newest first
      expect(service2.notifications[1].id, 'loaded_1');

      await db.close();
    });

    testWidgets('initialize with empty DB results in empty notifications',
        (tester) async {
      final db = DatabaseService();
      await db.init();

      final service = NotificationService();
      service.setDatabase(db);
      await service.initialize();

      expect(service.notifications, isEmpty);

      await db.close();
    });

    testWidgets('ConduitNotification toMap/fromMap roundtrip', (tester) async {
      final original = ConduitNotification(
        id: 'rt_1',
        deviceId: 'device_x',
        app: 'TestApp',
        title: 'Roundtrip',
        body: 'Body text',
        timestamp: 1700000042,
        actions: ['Reply', 'Dismiss'],
      );

      final map = original.toMap();
      final restored = ConduitNotification.fromMap(map);

      expect(restored.id, original.id);
      expect(restored.deviceId, original.deviceId);
      expect(restored.app, original.app);
      expect(restored.title, original.title);
      expect(restored.body, original.body);
      expect(restored.timestamp, original.timestamp);
      expect(restored.actions, ['Reply', 'Dismiss']);
    });

    testWidgets('ConduitNotification fromMap handles null optional fields',
        (tester) async {
      final map = {
        'id': 'minimal',
        'device_id': 'd1',
        'app': 'App',
        'title': 'T',
        'body': 'B',
        'timestamp': 100,
        'actions': null,
      };

      final notif = ConduitNotification.fromMap(map);
      expect(notif.actions, isNull);
    });

    testWidgets('ConduitNotification timeAgo calculation', (tester) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;

      final justNow = ConduitNotification(
        id: 'x',
        deviceId: 'd',
        app: 'A',
        title: 'T',
        body: 'B',
        timestamp: now,
      );
      expect(justNow.timeAgo, 'just now');

      final fiveMinAgo = ConduitNotification(
        id: 'x',
        deviceId: 'd',
        app: 'A',
        title: 'T',
        body: 'B',
        timestamp: now - 300, // 5 minutes
      );
      expect(fiveMinAgo.timeAgo, '5m ago');

      final twoHoursAgo = ConduitNotification(
        id: 'x',
        deviceId: 'd',
        app: 'A',
        title: 'T',
        body: 'B',
        timestamp: now - 7200, // 2 hours
      );
      expect(twoHoursAgo.timeAgo, '2h ago');

      final threeDaysAgo = ConduitNotification(
        id: 'x',
        deviceId: 'd',
        app: 'A',
        title: 'T',
        body: 'B',
        timestamp: now - 259200, // 3 days
      );
      expect(threeDaysAgo.timeAgo, '3d ago');
    });
  });

  // ── Listener Notification ────────────────────────────────────────

  group('ChangeNotifier integration', () {
    testWidgets('addNotification notifies listeners', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      var notified = false;
      service.addListener(() {
        notified = true;
      });

      service.addNotification(ConduitNotification(
        id: 'listen_1',
        deviceId: 'd1',
        app: 'Test',
        title: 'Listen',
        body: '',
        timestamp: 100,
      ));

      expect(notified, isTrue);

      await db.close();
    });

    testWidgets('dismissNotification notifies listeners', (tester) async {
      final service = NotificationService();
      final db = DatabaseService();
      await db.init();
      service.setDatabase(db);

      service.addNotification(ConduitNotification(
        id: 'listen_del',
        deviceId: 'd1',
        app: 'Test',
        title: 'Del',
        body: '',
        timestamp: 100,
      ));

      var notified = false;
      service.addListener(() {
        notified = true;
      });

      service.dismissNotification('listen_del');
      expect(notified, isTrue);

      await db.close();
    });
  });
}
