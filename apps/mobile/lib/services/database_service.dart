import 'dart:async';
import 'dart:math';
import 'package:flutter/foundation.dart';
import 'package:path/path.dart' as p;
import 'package:sqflite_sqlcipher/sqflite.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Centralized SQLite persistence for the mobile app.
///
/// Tables:
///   - notification_history  (last 200 entries)
///   - clipboard_history     (last 500 entries)
///   - file_transfers        (last 100 entries)
///   - sms_threads           (no auto-prune — native is source of truth)
class DatabaseService {
  static const _dbName = 'conduit_mobile.db';
  static const _dbVersion = 1;

  Database? _db;

  Database get db {
    if (_db == null) throw StateError('DatabaseService not initialized. Call init() first.');
    return _db!;
  }

  // ── Lifecycle ───────────────────────────────────────────────────────

  Future<void> init() async {
    const storage = FlutterSecureStorage();
    var dbKey = await storage.read(key: 'database_encryption_key');

    if (dbKey == null || dbKey.isEmpty) {
      // Generate a new 256-bit key with a CSPRNG and persist it in
      // flutter_secure_storage (Keychain / Android Keystore backed).
      final random = Random.secure();
      final bytes = List<int>.generate(32, (_) => random.nextInt(256));
      dbKey = bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
      await storage.write(key: 'database_encryption_key', value: dbKey);
    }

    final dbPath = await getDatabasesPath();
    final path = p.join(dbPath, _dbName);
    _db = await openDatabase(
      path,
      version: _dbVersion,
      onCreate: _onCreate,
      onUpgrade: _onUpgrade,
      password: dbKey,
    );
    debugPrint('[DatabaseService] Opened encrypted $path (v$_dbVersion)');
  }

  Future<void> _onCreate(Database db, int version) async {
    await db.execute('''
      CREATE TABLE notification_history (
        id          TEXT PRIMARY KEY,
        device_id   TEXT NOT NULL,
        app         TEXT NOT NULL,
        title       TEXT NOT NULL,
        body        TEXT NOT NULL,
        timestamp   INTEGER NOT NULL,
        actions     TEXT,
        created_at  INTEGER NOT NULL
      )
    ''');

    await db.execute('''
      CREATE TABLE clipboard_history (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        content      TEXT NOT NULL,
        mime         TEXT NOT NULL,
        source_device TEXT NOT NULL,
        timestamp    INTEGER NOT NULL,
        created_at   INTEGER NOT NULL
      )
    ''');

    await db.execute('''
      CREATE TABLE file_transfers (
        id               TEXT PRIMARY KEY,
        name             TEXT NOT NULL,
        size             INTEGER NOT NULL,
        mime             TEXT NOT NULL,
        from_device      TEXT NOT NULL,
        to_device        TEXT NOT NULL,
        status           TEXT NOT NULL,
        chunks_received  INTEGER NOT NULL DEFAULT 0,
        total_chunks     INTEGER NOT NULL,
        saved_path       TEXT,
        expected_checksum TEXT,
        timestamp        INTEGER NOT NULL,
        created_at       INTEGER NOT NULL
      )
    ''');

    await db.execute('''
      CREATE TABLE sms_threads (
        thread_id    TEXT NOT NULL,
        address      TEXT NOT NULL,
        name         TEXT,
        snippet      TEXT NOT NULL,
        unread_count INTEGER NOT NULL DEFAULT 0,
        timestamp    INTEGER NOT NULL,
        msg_id       TEXT NOT NULL,
        msg_body     TEXT NOT NULL,
        msg_read     INTEGER NOT NULL DEFAULT 1,
        msg_outgoing INTEGER NOT NULL DEFAULT 0,
        msg_timestamp INTEGER NOT NULL,
        PRIMARY KEY (thread_id, msg_id)
      )
    ''');
  }

  Future<void> _onUpgrade(Database db, int oldVersion, int newVersion) async {
    // Future migrations go here
  }

  // ── Notifications ───────────────────────────────────────────────────

  static const _maxNotifications = 200;

  Future<void> insertNotification(Map<String, dynamic> row) async {
    await db.insert('notification_history', row, conflictAlgorithm: ConflictAlgorithm.replace);
    await _pruneTable('notification_history', _maxNotifications);
  }

  Future<List<Map<String, dynamic>>> getNotifications({int limit = 200}) async {
    return db.query('notification_history', orderBy: 'timestamp DESC', limit: limit);
  }

  Future<void> deleteNotification(String id) async {
    await db.delete('notification_history', where: 'id = ?', whereArgs: [id]);
  }

  // ── Clipboard ───────────────────────────────────────────────────────

  static const _maxClipboard = 500;

  Future<void> insertClipboardEntry(Map<String, dynamic> row) async {
    await db.insert('clipboard_history', row);
    await _pruneTable('clipboard_history', _maxClipboard);
  }

  Future<List<Map<String, dynamic>>> getClipboardHistory({int limit = 500}) async {
    return db.query('clipboard_history', orderBy: 'timestamp DESC', limit: limit);
  }

  Future<void> clearClipboardHistory() async {
    await db.delete('clipboard_history');
  }

  // ── File Transfers ──────────────────────────────────────────────────

  static const _maxFileTransfers = 100;

  Future<void> insertFileTransfer(Map<String, dynamic> row) async {
    await db.insert('file_transfers', row, conflictAlgorithm: ConflictAlgorithm.replace);
    await _pruneTable('file_transfers', _maxFileTransfers);
  }

  Future<void> updateFileTransfer(String id, Map<String, dynamic> values) async {
    await db.update('file_transfers', values, where: 'id = ?', whereArgs: [id]);
  }

  Future<List<Map<String, dynamic>>> getFileTransfers({int limit = 100}) async {
    return db.query('file_transfers', orderBy: 'timestamp DESC', limit: limit);
  }

  Future<void> deleteAllFileTransfers() async {
    await db.delete('file_transfers');
  }

  // ── SMS Threads ─────────────────────────────────────────────────────

  Future<void> upsertSmsMessage(Map<String, dynamic> row) async {
    await db.insert('sms_threads', row, conflictAlgorithm: ConflictAlgorithm.replace);
  }

  Future<void> upsertSmsMessagesBatch(List<Map<String, dynamic>> rows) async {
    final batch = db.batch();
    for (final row in rows) {
      batch.insert('sms_threads', row, conflictAlgorithm: ConflictAlgorithm.replace);
    }
    await batch.commit(noResult: true);
  }

  Future<List<Map<String, dynamic>>> getSmsThreads() async {
    // Distinct threads by thread_id — return the latest message per thread
    // Grouping is done in the caller since SQLite GROUP BY with full rows is awkward.
    // Instead we return all rows ordered and let SmsService group them.
    return db.query('sms_threads', orderBy: 'msg_timestamp DESC');
  }

  Future<void> clearSmsThreads() async {
    await db.delete('sms_threads');
  }

  // ── Helpers ─────────────────────────────────────────────────────────

  Future<void> _pruneTable(String table, int maxRows) async {
    final count = Sqflite.firstIntValue(
      await db.rawQuery('SELECT COUNT(*) FROM $table'),
    );
    if (count != null && count > maxRows) {
      // Keep the newest rows; delete the oldest beyond the limit.
      // For tables with INTEGER PRIMARY KEY AUTOINCREMENT, delete by rowid.
      // For others, delete by a subquery on created_at or timestamp.
      if (table == 'notification_history') {
        await db.rawDelete('''
          DELETE FROM $table WHERE id NOT IN (
            SELECT id FROM $table ORDER BY timestamp DESC LIMIT $maxRows
          )
        ''');
      } else {
        await db.rawDelete('''
          DELETE FROM $table WHERE rowid NOT IN (
            SELECT rowid FROM $table ORDER BY timestamp DESC LIMIT $maxRows
          )
        ''');
      }
    }
  }

  Future<void> close() async {
    await _db?.close();
    _db = null;
  }
}
