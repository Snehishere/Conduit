import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:permission_handler/permission_handler.dart';

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

  List<SmsThread> _threads = [];
  bool _hasPermission = false;
  bool _initialized = false;
  void Function(Map<String, dynamic>)? _sendMessage;

  List<SmsThread> get threads => _threads;
  bool get hasPermission => _hasPermission;

  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }

  /// Initialize SMS permission request
  Future<void> initialize() async {
    if (_initialized) return;

    try {
      final status = await Permission.sms.request();
      _hasPermission = status.isGranted;
      notifyListeners();

      if (_hasPermission) {
        await loadThreads();
      }
      _initialized = true;
    } catch (e) {
      debugPrint('Failed to initialize SMS: $e');
    }
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
    } catch (e) {
      debugPrint('Failed to load SMS threads: $e');
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
        return true;
      }
      return false;
    } catch (e) {
      debugPrint('Failed to send SMS: $e');
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
    }
  }
}
