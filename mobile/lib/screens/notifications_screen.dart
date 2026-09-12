import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/notification_service.dart';
import '../services/websocket_service.dart';
import '../models/conduit_notification.dart';

class NotificationsScreen extends StatelessWidget {
  const NotificationsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Notifications'),
        actions: [
          Consumer<NotificationService>(
            builder: (_, svc, __) {
              if (svc.notifications.isEmpty) return const SizedBox.shrink();
              return IconButton(
                icon: const Icon(Icons.delete_sweep),
                tooltip: 'Clear all',
                onPressed: () => _showClearAllDialog(context, svc),
              );
            },
          ),
        ],
      ),
      body: Consumer<NotificationService>(
        builder: (_, svc, __) {
          if (svc.notifications.isEmpty) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(Icons.notifications_none, size: 64, color: Colors.grey.shade600),
                  const SizedBox(height: 16),
                  Text(
                    'No notifications',
                    style: TextStyle(fontSize: 16, color: Colors.grey.shade400),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Notifications from paired devices\nwill appear here',
                    textAlign: TextAlign.center,
                    style: TextStyle(fontSize: 13, color: Colors.grey.shade600),
                  ),
                ],
              ),
            );
          }

          return ListView.builder(
            padding: const EdgeInsets.symmetric(vertical: 8),
            itemCount: svc.notifications.length,
            itemBuilder: (_, i) {
              final n = svc.notifications[i];
              return _NotificationCard(
                notification: n,
                onDismiss: () => svc.dismissNotification(n.id),
              );
            },
          );
        },
      ),
    );
  }

  void _showClearAllDialog(BuildContext context, NotificationService svc) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Clear all notifications?'),
        content: Text('This will remove ${svc.notifications.length} notifications.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () {
              for (final n in List.of(svc.notifications)) {
                svc.dismissNotification(n.id);
              }
              Navigator.pop(ctx);
            },
            child: const Text('Clear'),
          ),
        ],
      ),
    );
  }
}

class _NotificationCard extends StatelessWidget {
  final ConduitNotification notification;
  final VoidCallback onDismiss;

  const _NotificationCard({
    required this.notification,
    required this.onDismiss,
  });

  IconData _getAppIcon(String app) {
    switch (app.toLowerCase()) {
      case 'whatsapp': return Icons.chat;
      case 'telegram': return Icons.telegram;
      case 'gmail': case 'mail': return Icons.email;
      case 'messages': case 'sms': return Icons.message;
      case 'phone': case 'call': return Icons.phone;
      case 'instagram': return Icons.camera_alt;
      case 'twitter': case 'x': return Icons.alternate_email;
      case 'slack': return Icons.work;
      case 'discord': return Icons.headset_mic;
      default: return Icons.notifications;
    }
  }

  Color _getAppColor(String app) {
    switch (app.toLowerCase()) {
      case 'whatsapp': return const Color(0xFF25D366);
      case 'telegram': return const Color(0xFF0088CC);
      case 'gmail': case 'mail': return const Color(0xFFEA4335);
      case 'messages': case 'sms': return const Color(0xFF34C759);
      case 'phone': case 'call': return const Color(0xFF007AFF);
      case 'instagram': return const Color(0xFFE4405F);
      default: return const Color(0xFF6366F1);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Dismissible(
      key: Key(notification.id),
      direction: DismissDirection.endToStart,
      onDismissed: (_) => onDismiss(),
      background: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 20),
        color: const Color(0xFFEF4444),
        child: const Icon(Icons.delete, color: Colors.white),
      ),
      child: Card(
        margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
        child: ListTile(
          leading: Container(
            width: 40,
            height: 40,
            decoration: BoxDecoration(
              color: _getAppColor(notification.app).withValues(alpha: 0.15),
              borderRadius: BorderRadius.circular(10),
            ),
            child: Icon(
              _getAppIcon(notification.app),
              color: _getAppColor(notification.app),
              size: 20,
            ),
          ),
          title: Text(
            notification.title,
            style: const TextStyle(fontSize: 14, fontWeight: FontWeight.w500),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          subtitle: Text(
            notification.body,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: 13, color: Colors.grey.shade400),
          ),
          trailing: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Text(
                notification.app,
                style: TextStyle(fontSize: 11, color: Colors.grey.shade500),
              ),
              const SizedBox(height: 2),
              Text(
                notification.timeAgo,
                style: TextStyle(fontSize: 11, color: Colors.grey.shade600),
              ),
            ],
          ),
          onTap: () => _showReplyDialog(context),
        ),
      ),
    );
  }

  void _showReplyDialog(BuildContext context) {
    final controller = TextEditingController();
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('Reply to ${notification.app}'),
        content: TextField(
          controller: controller,
          decoration: const InputDecoration(hintText: 'Type a reply...'),
          autofocus: true,
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () {
              final text = controller.text.trim();
              if (text.isNotEmpty) {
                context.read<WebSocketService>().sendNotificationReply(
                  notification.id,
                  text,
                );
              }
              Navigator.pop(ctx);
            },
            child: const Text('Send'),
          ),
        ],
      ),
    );
  }
}
