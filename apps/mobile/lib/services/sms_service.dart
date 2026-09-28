import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:permission_handler/permission_handler.dart';
import 'database_service.dart';

/// SMS thread data matching the shared protocol
class SmsThread {
  final String threadId;
  final String address;
  final String? name;
  final String snippet;
  final int unreadCount;
  final int timestamp;
  final List<SmsMessage> messages;

  SmsThread({
    required this.threadId,
    required this.address,
    this.name,
    required this.snippet,
    required this.unreadCount,
    required this.timestamp,
    required this.messages,
  });
}

/// SMS message data matching the shared protocol
class SmsMessage {
  final String id;
  final String address;
  final String body;
  final int timestamp;
  final bool read;
  final bool isOutgoing;

  SmsMessage({
    required this.id,
    required this.address,
    required this.body,
    required this.timestamp,
    required this.read,
    required this.isOutgoing,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'address': address,
    'body': body,
    'timestamp': timestamp,
    'read': read,
    'is_outgoing': isOutgoing,
  };
}

/// Service for reading and sending SMS on Android.
/// Uses MethodChannel to access Android Telephony API.
class SmsService extends ChangeNotifier {
  static const _channel = MethodChannel('com.conduit.mobile/native');
  static const _eventChannel = EventChannel('com.conduit.mobile/sms_events');

  List<SmsThread> _threads = [];
  bool _hasPermission = false;
  bool _initialized = false;
  void Function(Map<String, dynamic>)? _sendMessage;
  Function(String)? _onError;
  StreamSubscription? _incomingSmsSubscription;
  DatabaseService? _db;

  List<SmsThread> get threads => _threads;
  bool get hasPermission => _hasPermission;

  void onError(Function(String) callback) {
    _onError = callback;
  }

  void _handleError(String message) {
    debugPrint('[SmsService] Error: $message');
    _onError?.call(message);
  }

  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }

  /// Inject the database service and load persisted SMS threads.
  void setDatabase(DatabaseService db) {
    _db = db;
    _loadThreadsFromDb();
  }

  /// Load SMS threads from SQLite on startup.
  /// This provides cached data before the native platform query completes.
  Future<void> _loadThreadsFromDb() async {
    if (_db == null) return;
    try {
      final rows = await _db!.getSmsThreads();
      if (rows.isEmpty) return;

      // Build threads from DB rows (same grouping logic as loadThreads)
      final Map<String, List<SmsMessage>> grouped = {};
      for (final row in rows) {
        final threadId = row['thread_id'] as String;
        final address = row['address'] as String;
        final msg = SmsMessage(
          id: row['msg_id'] as String,
          address: address,
          body: row['msg_body'] as String,
          timestamp: (row['msg_timestamp'] as num?)?.toInt() ?? 0,
          read: ((row['msg_read'] as num?)?.toInt() ?? 1) == 1,
          isOutgoing: ((row['msg_outgoing'] as num?)?.toInt() ?? 0) == 1,
        );
        grouped.putIfAbsent(threadId, () => []).add(msg);
      }

      _threads = grouped.entries.map((e) {
        final msgs = e.value..sort((a, b) => a.timestamp.compareTo(b.timestamp));
        final last = msgs.last;
        return SmsThread(
          threadId: e.key,
          address: msgs.first.address,
          snippet: last.body,
          unreadCount: msgs.where((m) => !m.read).length,
          timestamp: last.timestamp,
          messages: msgs,
        );
      }).toList();

      _threads.sort((a, b) => b.timestamp.compareTo(a.timestamp));
      if (_threads.isNotEmpty) {
        debugPrint('[SmsService] Loaded ${_threads.length} threads from DB');
        notifyListeners();
      }
    } catch (e) {
      debugPrint('[SmsService] Failed to load threads from DB: $e');
    }
  }

  /// Persist all current threads to SQLite (batch upsert).
  Future<void> _persistThreads() async {
    if (_db == null) return;
    try {
      final rows = <Map<String, dynamic>>[];
      for (final thread in _threads) {
        for (final msg in thread.messages) {
          rows.add({
            'thread_id': thread.threadId,
            'address': thread.address,
            'name': thread.name,
            'snippet': thread.snippet,
            'unread_count': thread.unreadCount,
            'timestamp': thread.timestamp,
            'msg_id': msg.id,
            'msg_body': msg.body,
            'msg_read': msg.read ? 1 : 0,
            'msg_outgoing': msg.isOutgoing ? 1 : 0,
            'msg_timestamp': msg.timestamp,
          });
        }
      }
      await _db!.upsertSmsMessagesBatch(rows);
    } catch (e) {
      debugPrint('[SmsService] Failed to persist threads: $e');
    }
  }

  /// Persist a single thread's latest state to SQLite.
  Future<void> _persistThread(SmsThread thread) async {
    if (_db == null) return;
    try {
      final rows = thread.messages.map((msg) => {
        'thread_id': thread.threadId,
        'address': thread.address,
        'name': thread.name,
        'snippet': thread.snippet,
        'unread_count': thread.unreadCount,
        'timestamp': thread.timestamp,
        'msg_id': msg.id,
        'msg_body': msg.body,
        'msg_read': msg.read ? 1 : 0,
        'msg_outgoing': msg.isOutgoing ? 1 : 0,
        'msg_timestamp': msg.timestamp,
      }).toList();
      await _db!.upsertSmsMessagesBatch(rows);
    } catch (e) {
      debugPrint('[SmsService] Failed to persist thread: $e');
    }
  }

  /// Initialize SMS permission request and start listening for incoming SMS
  Future<void> initialize() async {
    if (_initialized) return;

    try {
      final status = await Permission.sms.request();
      _hasPermission = status.isGranted;
      notifyListeners();

      if (_hasPermission) {
        await loadThreads();
        _startIncomingSmsListener();
      } else {
        _handleError('SMS permission denied. Please enable SMS access in Settings.');
      }
      _initialized = true;
    } catch (e) {
      _handleError('Failed to initialize SMS: $e');
    }
  }

  /// Listen for incoming SMS on Android via EventChannel.
  /// When a new SMS arrives, forward it to the desktop via WebSocket.
  void _startIncomingSmsListener() {
    if (defaultTargetPlatform != TargetPlatform.android) {
      return;
    }

    try {
      _incomingSmsSubscription?.cancel();
      _incomingSmsSubscription = _eventChannel.receiveBroadcastStream('sms').listen(
        (event) {
          if (event is Map) {
            final type = event['type'] as String?;
            final action = event['action'] as String?;
            if (type == 'sms' && action == 'new') {
              final from = event['from'] as String? ?? '';
              final body = event['body'] as String? ?? '';
              final timestamp = (event['timestamp'] as num?)?.toInt()
                  ?? DateTime.now().millisecondsSinceEpoch ~/ 1000;

              // Forward to desktop via WebSocket
              _sendMessage?.call({
                'type': 'sms',
                'action': 'new',
                'from': from,
                'body': body,
                'timestamp': timestamp,
              });

              // Also update local threads
              _addIncomingToLocal(from, body, timestamp);
            }
          }
        },
        onError: (error) {
          debugPrint('[SmsService] Incoming SMS stream error: $error');
        },
      );
    } catch (e) {
      debugPrint('[SmsService] Failed to start incoming SMS listener: $e');
    }
  }

  /// Add an incoming SMS to local threads
  void _addIncomingToLocal(String from, String body, int timestamp) {
    final message = SmsMessage(
      id: 'in_${DateTime.now().millisecondsSinceEpoch}',
      address: from,
      body: body,
      timestamp: timestamp,
      read: false,
      isOutgoing: false,
    );

    final existingIndex = _threads.indexWhere((t) => t.address == from);
    if (existingIndex >= 0) {
      final existing = _threads[existingIndex];
      _threads[existingIndex] = SmsThread(
        threadId: existing.threadId,
        address: from,
        name: existing.name,
        snippet: body,
        unreadCount: existing.unreadCount + 1,
        timestamp: timestamp,
        messages: [...existing.messages, message],
      );
    } else {
      _threads.insert(0, SmsThread(
        threadId: 't_${DateTime.now().millisecondsSinceEpoch}',
        address: from,
        snippet: body,
        unreadCount: 1,
        timestamp: timestamp,
        messages: [message],
      ));
    }

    _threads.sort((a, b) => b.timestamp.compareTo(a.timestamp));
    notifyListeners();

    // Persist the updated thread to SQLite
    final updatedThread = existingIndex >= 0 ? _threads[existingIndex] : _threads.first;
    _persistThread(updatedThread);
  }

  /// Load all SMS threads from the device via native platform.
  Future<void> loadThreads() async {
    try {
      final result = await _channel.invokeMethod<String>('getSmsThreads');
      if (result == null || result.isEmpty) {
        _threads = [];
        notifyListeners();
        return;
      }

      final List<dynamic> data = jsonDecode(result);
      // Native returns a FLAT list of SMS records (address/body/date/type/read).
      // Group by address into threads; each record becomes one SmsMessage.
      final Map<String, List<SmsMessage>> grouped = {};
      for (final t in data) {
        final m = t as Map<String, dynamic>;
        final address = (m['address'] as String?) ?? 'unknown';
        final msg = SmsMessage(
          id: (m['_id']?.toString() ?? '${address}_${m['date'] ?? DateTime.now().millisecondsSinceEpoch}'),
          address: address,
          body: (m['body'] as String?) ?? '',
          timestamp: ((m['date'] as num?)?.toInt() ?? DateTime.now().millisecondsSinceEpoch) ~/ 1000,
          read: ((m['read'] as num?)?.toInt() ?? 1) == 1,
          isOutgoing: ((m['type'] as num?)?.toInt() ?? 1) == 2,
        );
        grouped.putIfAbsent(address, () => []).add(msg);
      }
      _threads = grouped.entries.map((e) {
        final msgs = e.value..sort((a, b) => a.timestamp.compareTo(b.timestamp));
        final last = msgs.last;
        return SmsThread(
          threadId: e.key,
          address: e.key,
          snippet: last.body,
          unreadCount: msgs.where((m) => !m.read).length,
          timestamp: last.timestamp,
          messages: msgs,
        );
      }).toList();

      _threads.sort((a, b) => b.timestamp.compareTo(a.timestamp));
      notifyListeners();

      // Persist native-loaded threads to SQLite for offline caching
      _persistThreads();
    } catch (e) {
      _handleError('Failed to load SMS threads: $e');
    }
  }

  /// Send an SMS message via native platform.
  Future<bool> sendSms(String to, String body) async {
    try {
      final result = await _channel.invokeMethod<bool>('sendSms', {
        'to': to,
        'body': body,
      });

      if (result == true) {
        final message = SmsMessage(
          id: 'out_${DateTime.now().millisecondsSinceEpoch}',
          address: to,
          body: body,
          timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
          read: true,
          isOutgoing: true,
        );

        // Relay sent SMS to desktop via WebSocket
        _sendMessage?.call({
          'type': 'sms',
          'action': 'sent',
          'to': to,
          'body': body,
          'timestamp': message.timestamp,
        });

        // Find or create thread locally
        final existingIndex = _threads.indexWhere((t) => t.address == to);
        if (existingIndex >= 0) {
          _threads[existingIndex] = SmsThread(
            threadId: _threads[existingIndex].threadId,
            address: to,
            name: _threads[existingIndex].name,
            snippet: body,
            unreadCount: 0,
            timestamp: message.timestamp,
            messages: [..._threads[existingIndex].messages, message],
          );
        } else {
          _threads.insert(0, SmsThread(
            threadId: 't_${DateTime.now().millisecondsSinceEpoch}',
            address: to,
            snippet: body,
            unreadCount: 0,
            timestamp: message.timestamp,
            messages: [message],
          ));
        }

        notifyListeners();
        _persistThread(
          existingIndex >= 0 ? _threads[existingIndex] : _threads.first,
        );
        return true;
      }
      return false;
    } catch (e) {
      _handleError('Failed to send SMS: $e');
      return false;
    }
  }

  /// Convert threads to JSON for WebSocket sync
  List<Map<String, dynamic>> threadsToJson() {
    return _threads.map((t) => {
      'thread_id': t.threadId,
      'address': t.address,
      'name': t.name,
      'snippet': t.snippet,
      'unread_count': t.unreadCount,
      'timestamp': t.timestamp,
      'messages': t.messages.map((m) => m.toJson()).toList(),
    }).toList();
  }

  /// Send SMS sync to desktop via WebSocket
  void syncToDesktop() {
    if (_threads.isEmpty) return;
    _sendMessage?.call({
      'type': 'sms',
      'action': 'sync',
      'threads': threadsToJson(),
    });
  }

  /// Handle incoming SMS sync from another device
  void handleSync(List<dynamic> threadData) {
    for (final t in threadData) {
      final thread = SmsThread(
        threadId: t['thread_id'] as String,
        address: t['address'] as String,
        name: t['name'] as String?,
        snippet: t['snippet'] as String,
        unreadCount: (t['unread_count'] as num).toInt(),
        timestamp: (t['timestamp'] as num).toInt(),
        messages: (t['messages'] as List).map((m) => SmsMessage(
          id: m['id'] as String,
          address: m['address'] as String,
          body: m['body'] as String,
          timestamp: (m['timestamp'] as num).toInt(),
          read: m['read'] as bool,
          isOutgoing: m['is_outgoing'] as bool,
        )).toList(),
      );

      final existingIndex = _threads.indexWhere((t) => t.threadId == thread.threadId);
      if (existingIndex >= 0) {
        _threads[existingIndex] = thread;
      } else {
        _threads.add(thread);
      }
    }

    _threads.sort((a, b) => b.timestamp.compareTo(a.timestamp));
    notifyListeners();
    _persistThreads();
  }

  /// Handle a new incoming SMS from another device
  void handleNewMessage(String threadId, Map<String, dynamic> messageData) {
    final message = SmsMessage(
      id: messageData['id'] as String,
      address: messageData['address'] as String,
      body: messageData['body'] as String,
      timestamp: (messageData['timestamp'] as num).toInt(),
      read: messageData['read'] as bool,
      isOutgoing: messageData['is_outgoing'] as bool,
    );

    final existingIndex = _threads.indexWhere((t) => t.threadId == threadId);
    if (existingIndex >= 0) {
      final existing = _threads[existingIndex];
      _threads[existingIndex] = SmsThread(
        threadId: threadId,
        address: existing.address,
        name: existing.name,
        snippet: message.body,
        unreadCount: message.read ? existing.unreadCount : existing.unreadCount + 1,
        timestamp: message.timestamp,
        messages: [...existing.messages, message],
      );
      _threads.sort((a, b) => b.timestamp.compareTo(a.timestamp));
      notifyListeners();
      _persistThread(_threads[existingIndex]);
    }
  }

  @override
  void dispose() {
    _incomingSmsSubscription?.cancel();
    super.dispose();
  }
}
