import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import '../models/conduit_notification.dart';
import 'database_service.dart';

class NotificationService extends ChangeNotifier {
  /// The channel the native `NotificationListenerService` bridge publishes on.
  ///
  /// One stream, one channel. `EventChannel.receiveBroadcastStream` registers
  /// its Dart handler by channel *name* only, and the native side keeps one sink
  /// per name, so sharing a name between streams left every subscriber but the
  /// last one permanently dead. Do not merge this with another stream's name.
  static const _eventChannel = EventChannel(
    'com.conduit.mobile/notification_events',
  );

  static const _channel = MethodChannel('com.conduit.mobile/native');

  final FlutterLocalNotificationsPlugin _notifications =
      FlutterLocalNotificationsPlugin();
  final List<ConduitNotification> _notificationsList = [];
  bool _initialized = false;
  StreamSubscription? _notificationSubscription;
  DatabaseService? _db;
  void Function(Map<String, dynamic>)? _send;

  List<ConduitNotification> get notifications =>
      List.unmodifiable(_notificationsList);

  /// Inject the database service. Must be called before [initialize].
  void setDatabase(DatabaseService db) {
    _db = db;
  }

  void setSendFunction(void Function(Map<String, dynamic>) send) {
    _send = send;
  }

  Future<void> initialize() async {
    if (_initialized) return;

    const androidSettings = AndroidInitializationSettings(
      '@mipmap/ic_launcher',
    );
    const initSettings = InitializationSettings(android: androidSettings);

    await _notifications.initialize(
      settings: initSettings,
      onDidReceiveNotificationResponse: (details) {},
    );

    // Load persisted notifications from SQLite
    await _loadFromDb();

    _initialized = true;

    // Listen for notifications from native NotificationListenerService
    _notificationSubscription = _eventChannel.receiveBroadcastStream().listen(
      (event) {
        if (event is Map) {
          final type = event['type'] as String?;
          final action = event['action'] as String?;
          if (type == 'notification') {
            if (action == 'post') {
              final notification = ConduitNotification(
                id:
                    (event['id'] as String?) ??
                    'notif_${DateTime.now().millisecondsSinceEpoch}',
                deviceId: (event['device_id'] as String?) ?? 'phone',
                app: (event['app'] as String?) ?? 'unknown',
                title: (event['title'] as String?) ?? '',
                body: (event['body'] as String?) ?? '',
                timestamp:
                    (event['timestamp'] as num?)?.toInt() ??
                    DateTime.now().millisecondsSinceEpoch ~/ 1000,
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
      final enabled = await _channel.invokeMethod<bool>(
        'isNotificationListenerEnabled',
      );
      if (enabled != true) {
        debugPrint(
          'NotificationListenerService not enabled. Prompting user...',
        );
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
        debugPrint(
          '[NotificationService] Loaded ${_notificationsList.length} notifications from DB',
        );
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
  /// The entry stays in the list and on disk — only the flag moves. Deleting
  /// the row here (what the UI used to do) destroyed the notification on the
  /// phone, so the desktop was told "read" while the phone's copy vanished.
  Future<void> markReadNotification(String id) async {
    final index = _notificationsList.indexWhere((n) => n.id == id);
    if (index < 0 || _notificationsList[index].read) return;

    _notificationsList[index] = _notificationsList[index].copyWith(read: true);
    notifyListeners();

    if (_db == null) return;
    try {
      await _db!.markNotificationRead(id);
    } catch (e) {
      debugPrint('[NotificationService] Failed to mark $id read: $e');
    }
  }

  @override
  void dispose() {
    _notificationSubscription?.cancel();
    super.dispose();
  }
}
