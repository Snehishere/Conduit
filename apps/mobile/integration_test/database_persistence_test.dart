import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:conduit/services/database_service.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late DatabaseService db;

  setUp(() async {
    db = DatabaseService();
    await db.init();
  });

  tearDown(() async {
    await db.close();
  });

  // ── Notification CRUD ─────────────────────────────────────────────

  group('Notification persistence', () {
    testWidgets('insert and query notifications', (tester) async {
      final row = {
        'id': 'notif_1',
        'device_id': 'device_a',
        'app': 'Slack',
        'title': 'New message',
        'body': 'Hello from desktop',
        'timestamp': 1700000000,
        'actions': null,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      };

      await db.insertNotification(row);
      final results = await db.getNotifications();

      expect(results.length, 1);
      expect(results[0]['id'], 'notif_1');
      expect(results[0]['app'], 'Slack');
      expect(results[0]['title'], 'New message');
      expect(results[0]['body'], 'Hello from desktop');
      expect(results[0]['device_id'], 'device_a');
      expect(results[0]['timestamp'], 1700000000);
    });

    testWidgets('query returns newest first', (tester) async {
      final older = {
        'id': 'notif_old',
        'device_id': 'device_a',
        'app': 'Mail',
        'title': 'Old',
        'body': 'Old message',
        'timestamp': 1600000000,
        'created_at': 1600000000000,
      };
      final newer = {
        'id': 'notif_new',
        'device_id': 'device_a',
        'app': 'Mail',
        'title': 'New',
        'body': 'New message',
        'timestamp': 1700000000,
        'created_at': 1700000000000,
      };

      await db.insertNotification(older);
      await db.insertNotification(newer);

      final results = await db.getNotifications();
      expect(results.length, 2);
      expect(results[0]['id'], 'notif_new'); // newest first
      expect(results[1]['id'], 'notif_old');
    });

    testWidgets('deleteNotification removes the row', (tester) async {
      await db.insertNotification({
        'id': 'notif_del',
        'device_id': 'device_a',
        'app': 'Test',
        'title': 'Delete me',
        'body': 'Body',
        'timestamp': 1700000000,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      });

      await db.deleteNotification('notif_del');
      final results = await db.getNotifications();
      expect(results, isEmpty);
    });

    testWidgets('upsert replaces existing notification', (tester) async {
      final v1 = {
        'id': 'notif_upsert',
        'device_id': 'device_a',
        'app': 'Slack',
        'title': 'V1',
        'body': 'Version 1',
        'timestamp': 1700000000,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      };
      await db.insertNotification(v1);

      final v2 = {
        'id': 'notif_upsert',
        'device_id': 'device_a',
        'app': 'Slack',
        'title': 'V2',
        'body': 'Version 2',
        'timestamp': 1700000001,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      };
      await db.insertNotification(v2);

      final results = await db.getNotifications();
      expect(results.length, 1);
      expect(results[0]['title'], 'V2');
    });

    testWidgets('auto-prune keeps only 200 notifications', (tester) async {
      // Insert 210 notifications with sequential timestamps
      for (var i = 0; i < 210; i++) {
        await db.insertNotification({
          'id': 'notif_$i',
          'device_id': 'device_a',
          'app': 'Test',
          'title': 'Notification $i',
          'body': 'Body $i',
          'timestamp': 1700000000 + i,
          'created_at': DateTime.now().millisecondsSinceEpoch + i,
        });
      }

      final results = await db.getNotifications();
      expect(results.length, 200);

      // The oldest entries (0-9) should be pruned; newest (10-209) kept
      expect(results.last['id'], 'notif_10');
      expect(results.first['id'], 'notif_209');
    });

    testWidgets('getNotifications respects limit parameter', (tester) async {
      for (var i = 0; i < 5; i++) {
        await db.insertNotification({
          'id': 'notif_limit_$i',
          'device_id': 'device_a',
          'app': 'Test',
          'title': 'Title $i',
          'body': 'Body',
          'timestamp': 1700000000 + i,
          'created_at': DateTime.now().millisecondsSinceEpoch + i,
        });
      }

      final results = await db.getNotifications(limit: 3);
      expect(results.length, 3);
    });
  });

  // ── Clipboard History ─────────────────────────────────────────────

  group('Clipboard history persistence', () {
    testWidgets('insert and query clipboard entries', (tester) async {
      final row = {
        'content': 'https://example.com',
        'mime': 'text/plain',
        'source_device': 'desktop_1',
        'timestamp': 1700000000,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      };

      await db.insertClipboardEntry(row);
      final results = await db.getClipboardHistory();

      expect(results.length, 1);
      expect(results[0]['content'], 'https://example.com');
      expect(results[0]['mime'], 'text/plain');
      expect(results[0]['source_device'], 'desktop_1');
    });

    testWidgets('clipboard entries ordered by timestamp DESC', (tester) async {
      await db.insertClipboardEntry({
        'content': 'first',
        'mime': 'text/plain',
        'source_device': 'd1',
        'timestamp': 100,
        'created_at': 100,
      });
      await db.insertClipboardEntry({
        'content': 'second',
        'mime': 'text/plain',
        'source_device': 'd1',
        'timestamp': 200,
        'created_at': 200,
      });

      final results = await db.getClipboardHistory();
      expect(results[0]['content'], 'second');
      expect(results[1]['content'], 'first');
    });

    testWidgets('clearClipboardHistory removes all entries', (tester) async {
      for (var i = 0; i < 5; i++) {
        await db.insertClipboardEntry({
          'content': 'clip_$i',
          'mime': 'text/plain',
          'source_device': 'd1',
          'timestamp': 1700000000 + i,
          'created_at': DateTime.now().millisecondsSinceEpoch + i,
        });
      }

      await db.clearClipboardHistory();
      final results = await db.getClipboardHistory();
      expect(results, isEmpty);
    });

    testWidgets('auto-prune keeps only 500 clipboard entries', (tester) async {
      for (var i = 0; i < 510; i++) {
        await db.insertClipboardEntry({
          'content': 'clip_$i',
          'mime': 'text/plain',
          'source_device': 'd1',
          'timestamp': 1700000000 + i,
          'created_at': DateTime.now().millisecondsSinceEpoch + i,
        });
      }

      final results = await db.getClipboardHistory();
      expect(results.length, 500);
    });
  });

  // ── File Transfers ────────────────────────────────────────────────

  group('File transfer CRUD', () {
    testWidgets('insert and query file transfers', (tester) async {
      final row = {
        'id': 'file_1',
        'name': 'photo.jpg',
        'size': 1024000,
        'mime': 'image/jpeg',
        'from_device': 'desktop_1',
        'to_device': 'mobile_1',
        'status': 'completed',
        'chunks_received': 10,
        'total_chunks': 10,
        'saved_path': '/storage/emulated/0/photo.jpg',
        'expected_checksum': 'abc123',
        'timestamp': 1700000000,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      };

      await db.insertFileTransfer(row);
      final results = await db.getFileTransfers();

      expect(results.length, 1);
      expect(results[0]['id'], 'file_1');
      expect(results[0]['name'], 'photo.jpg');
      expect(results[0]['size'], 1024000);
      expect(results[0]['status'], 'completed');
      expect(results[0]['chunks_received'], 10);
      expect(results[0]['saved_path'], '/storage/emulated/0/photo.jpg');
    });

    testWidgets('updateFileTransfer modifies fields', (tester) async {
      await db.insertFileTransfer({
        'id': 'file_upd',
        'name': 'doc.pdf',
        'size': 500000,
        'mime': 'application/pdf',
        'from_device': 'desktop_1',
        'to_device': 'mobile_1',
        'status': 'transferring',
        'chunks_received': 3,
        'total_chunks': 10,
        'saved_path': null,
        'expected_checksum': null,
        'timestamp': 1700000000,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      });

      await db.updateFileTransfer('file_upd', {
        'status': 'completed',
        'chunks_received': 10,
        'saved_path': '/storage/doc.pdf',
      });

      final results = await db.getFileTransfers();
      expect(results.length, 1);
      expect(results[0]['status'], 'completed');
      expect(results[0]['chunks_received'], 10);
      expect(results[0]['saved_path'], '/storage/doc.pdf');
    });

    testWidgets('upsert replaces existing file transfer', (tester) async {
      await db.insertFileTransfer({
        'id': 'file_dup',
        'name': 'v1.txt',
        'size': 100,
        'mime': 'text/plain',
        'from_device': 'd1',
        'to_device': 'd2',
        'status': 'pending',
        'chunks_received': 0,
        'total_chunks': 1,
        'saved_path': null,
        'expected_checksum': null,
        'timestamp': 1700000000,
        'created_at': 1600000000000,
      });

      await db.insertFileTransfer({
        'id': 'file_dup',
        'name': 'v2.txt',
        'size': 200,
        'mime': 'text/plain',
        'from_device': 'd1',
        'to_device': 'd2',
        'status': 'completed',
        'chunks_received': 1,
        'total_chunks': 1,
        'saved_path': '/path/v2.txt',
        'expected_checksum': 'def456',
        'timestamp': 1700000001,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      });

      final results = await db.getFileTransfers();
      expect(results.length, 1);
      expect(results[0]['name'], 'v2.txt');
      expect(results[0]['status'], 'completed');
      expect(results[0]['size'], 200);
    });

    testWidgets('file transfers ordered by timestamp DESC', (tester) async {
      await db.insertFileTransfer({
        'id': 'file_a',
        'name': 'a.txt',
        'size': 10,
        'mime': 'text/plain',
        'from_device': 'd1',
        'to_device': 'd2',
        'status': 'completed',
        'chunks_received': 1,
        'total_chunks': 1,
        'saved_path': null,
        'expected_checksum': null,
        'timestamp': 100,
        'created_at': 100,
      });
      await db.insertFileTransfer({
        'id': 'file_b',
        'name': 'b.txt',
        'size': 20,
        'mime': 'text/plain',
        'from_device': 'd1',
        'to_device': 'd2',
        'status': 'completed',
        'chunks_received': 1,
        'total_chunks': 1,
        'saved_path': null,
        'expected_checksum': null,
        'timestamp': 200,
        'created_at': 200,
      });

      final results = await db.getFileTransfers();
      expect(results[0]['id'], 'file_b');
      expect(results[1]['id'], 'file_a');
    });

    testWidgets('auto-prune keeps only 100 file transfers', (tester) async {
      for (var i = 0; i < 110; i++) {
        await db.insertFileTransfer({
          'id': 'file_$i',
          'name': 'file_$i.txt',
          'size': i * 100,
          'mime': 'text/plain',
          'from_device': 'd1',
          'to_device': 'd2',
          'status': 'completed',
          'chunks_received': 1,
          'total_chunks': 1,
          'saved_path': null,
          'expected_checksum': null,
          'timestamp': 1700000000 + i,
          'created_at': DateTime.now().millisecondsSinceEpoch + i,
        });
      }

      final results = await db.getFileTransfers();
      expect(results.length, 100);
      // Oldest 10 should be pruned
      expect(results.last['id'], 'file_10');
    });
  });

  // ── Database Lifecycle ────────────────────────────────────────────

  group('Database lifecycle', () {
    testWidgets('double init is safe', (tester) async {
      await db.init(); // second init on already-initialized db
      // Should not throw — sqflite opens existing file
      final results = await db.getNotifications();
      expect(results, isA<List>());
    });

    testWidgets('close and re-open preserves data', (tester) async {
      await db.insertNotification({
        'id': 'persist_test',
        'device_id': 'd1',
        'app': 'Test',
        'title': 'Persist',
        'body': 'Should survive',
        'timestamp': 1700000000,
        'created_at': DateTime.now().millisecondsSinceEpoch,
      });

      await db.close();

      final db2 = DatabaseService();
      await db2.init();
      final results = await db2.getNotifications();
      expect(results.length, 1);
      expect(results[0]['id'], 'persist_test');
      await db2.close();
    });
  });
}
