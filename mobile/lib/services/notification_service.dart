import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import '../models/conduit_notification.dart';

class NotificationService extends ChangeNotifier {
  static const _eventChannel = EventChannel('com.conduit.mobile/events');
  static const _channel = MethodChannel('com.conduit.mobile/native');

  final FlutterLocalNotificationsPlugin _notifications =
      FlutterLocalNotificationsPlugin();
  final List<ConduitNotification> _notificationsList = [];
  bool _initialized = false;
  StreamSubscription? _notificationSubscription;

  List<ConduitNotification> get notifications => List.unmodifiable(_notificationsList);

  Future<void> initialize() async {
    if (_initialized) return;

    const androidSettings = AndroidInitializationSettings('@mipmap/ic_launcher');
    const initSettings = InitializationSettings(android: androidSettings);

    await _notifications.initialize(
      initSettings,
      onDidReceiveNotificationResponse: (details) {},
    );

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
  }

  Future<void> _showLocalNotification(ConduitNotification notification) async {
    if (!_initialized) return;

    const androidDetails = AndroidNotificationDetails(
      'conduit_sync',
      'Conduit Sync',
      channelDescription: 'Notifications synced from other devices',
      importance: Importance.high,
      priority: Priority.high,
    );

    await _notifications.show(
      notification.id.hashCode,
      '${notification.app} - ${notification.title}',
      notification.body,
      const NotificationDetails(android: androidDetails),
    );
  }

  void dismissNotification(String id) {
    _notificationsList.removeWhere((n) => n.id == id);
    notifyListeners();
  }

  @override
  void dispose() {
    _notificationSubscription?.cancel();
    super.dispose();
  }
}
