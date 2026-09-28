import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import '../models/conduit_notification.dart';
import 'database_service.dart';

class NotificationService extends ChangeNotifier {
  static const _eventChannel = EventChannel('com.conduit.mobile/events');
  static const _channel = MethodChannel('com.conduit.mobile/native');

  final FlutterLocalNotificationsPlugin _notifications =
      FlutterLocalNotificationsPlugin();
  final List<ConduitNotification> _notificationsList = [];
  bool _initialized = false;
  StreamSubscription? _notificationSubscription;
  DatabaseService? _db;
  void Function(Map<String, dynamic>)? _send;

  List<ConduitNotification> get notifications => List.unmodifiable(_notificationsList);

  /// Inject the database service. Must be called before [initialize].
  void setDatabase(DatabaseService db) {
    _db = db;
  }

  void setSendFunction(void Function(Map<String, dynamic>) send) {
    _send = send;
  }

  Future<void> initialize() async {
    if (_initialized) return;

    const androidSettings = AndroidInitializationSettings('@mipmap/ic_launcher');
    const initSettings = InitializationSettings(android: androidSettings);

    await _notifications.initialize(
      settings: initSettings,
      onDidReceiveNotificationResponse: (details) {},
    );

    // Load persisted notifications from SQLite
    await _loadFromDb();

    _initialized = true;

    // Listen for notifications from native NotificationListenerService
    _notificationSubscription = _eventChannel.receiveBroadcastStream('notifications').listen(
      (event) {
        if (event is Map) {
          final type = event['type'] as String?;
          final action = event['action'] as String?;
          if (type == 'notification') {
            if (action == 'post') {
              final notification = ConduitNotification(
                id: (event['id'] as String?) ?? 'notif_${DateTime.now().millisecondsSinceEpoch}',
                deviceId: (event['device_id'] as String?) ?? 'phone',
                app: (event['app'] as String?) ?? 'unknown',
                title: (event['title'] as String?) ?? '',
                body: (event['body'] as String?) ?? '',
                timestamp: (event['timestamp'] as num?)?.toInt() ?? DateTime.now().millisecondsSinceEpoch ~/ 1000,
                actions: null,
              );
              addNotification(notification);
              if (_send != null) {
                _send!({
                  'type': 'notification',
                  'action': 'post',
                  'id': notification.id,
                  'device_id': notification.deviceId,
                  'app': notification.app,
                  'title': notification.title,
                  'body': notification.body,
                  'timestamp': notification.timestamp,
                });
              }
            } else if (action == 'dismiss') {
              final id = event['id'] as String?;
              if (id != null) dismissNotification(id);
            }
          }
        }
      },
      onError: (error) {
        debugPrint('Notification event stream error: $error');
      },
    );

    // Check if notification listener is enabled
    try {
      final enabled = await _channel.invokeMethod<bool>('isNotificationListenerEnabled');
      if (enabled != true) {
        debugPrint('NotificationListenerService not enabled. Prompting user...');
        await _channel.invokeMethod('openNotificationListenerSettings');
      }
    } catch (e) {
      debugPrint('Failed to check notification listener: $e');
    }
  }

  void addNotification(ConduitNotification notification) {
    _notificationsList.insert(0, notification);
    if (_notificationsList.length > 100) {
      _notificationsList.removeLast();
    }
    notifyListeners();
    _showLocalNotification(notification);

    // Persist to SQLite
    _persistNotification(notification);
  }

  /// Load notifications from SQLite on startup.
  Future<void> _loadFromDb() async {
    if (_db == null) return;
    try {
      final rows = await _db!.getNotifications(limit: 200);
      _notificationsList.clear();
      for (final row in rows) {
        _notificationsList.add(ConduitNotification.fromMap(row));
      }
      if (_notificationsList.isNotEmpty) {
        debugPrint('[NotificationService] Loaded ${_notificationsList.length} notifications from DB');
        notifyListeners();
      }
    } catch (e) {
      debugPrint('[NotificationService] Failed to load from DB: $e');
    }
  }

  /// Persist a single notification to SQLite.
  Future<void> _persistNotification(ConduitNotification notification) async {
    if (_db == null) return;
    try {
      await _db!.insertNotification(notification.toMap());
    } catch (e) {
      debugPrint('[NotificationService] Failed to persist notification: $e');
    }
  }

  Future<void> _showLocalNotification(ConduitNotification notification) async {
    if (!_initialized) return;

    const androidDetails = AndroidNotificationDetails(
      'conduit_sync',
      'Synced notifications',
      channelDescription: 'Notifications forwarded between your devices',
      importance: Importance.high,
      priority: Priority.high,
    );

    await _notifications.show(
      id: notification.id.hashCode,
      title: '${notification.app} - ${notification.title}',
      body: notification.body,
      notificationDetails: const NotificationDetails(android: androidDetails),
    );
  }

  void dismissNotification(String id) {
    _notificationsList.removeWhere((n) => n.id == id);
    notifyListeners();
    // Remove from SQLite
    _db?.deleteNotification(id).catchError((e) {
      debugPrint('[NotificationService] Failed to delete from DB: $e');
    });
  }

  /// Mark a notification as read.
  ///
  /// Not implemented: [ConduitNotification] has no read flag, so there is
  /// nothing to persist. The UI currently marks a notification read by removing
  /// it, and sends the desktop a `mark_read` action over the WebSocket. Add a
  /// read column before this needs to do anything.
  void markReadNotification(String id) {}

  @override
  void dispose() {
    _notificationSubscription?.cancel();
    super.dispose();
  }
}
