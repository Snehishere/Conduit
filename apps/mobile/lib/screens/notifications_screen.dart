import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/notification_service.dart';
import '../services/websocket_service.dart';
import '../models/conduit_notification.dart';
import '../theme/app_theme.dart';
import '../widgets/empty_state.dart';

class NotificationsScreen extends StatelessWidget {
  final bool embedded;

  const NotificationsScreen({super.key, this.embedded = false});

  @override
  Widget build(BuildContext context) {
    final content = _buildContent(context);
    if (embedded) return content;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Notifications'),
        actions: [
          Consumer<NotificationService>(
            builder: (_, svc, _) {
              if (svc.notifications.isEmpty) return const SizedBox.shrink();
              final colors = Theme.of(context).extension<AppColors>()!;
              return IconButton(
                icon: Icon(Icons.delete_sweep, color: colors.text2),
                tooltip: 'Clear all',
                onPressed: () => _showClearAllDialog(context, svc),
              );
            },
          ),
        ],
      ),
      body: content,
    );
  }

  Widget _buildContent(BuildContext context) {
    return Consumer2<NotificationService, WebSocketService>(
      builder: (_, svc, ws, __) {
        if (svc.notifications.isEmpty) {
          return const EmptyState(
            icon: Icons.notifications_none,
            title: 'No notifications',
            description: 'Notifications from paired devices\nwill appear here',
          );
        }

        return ListView.builder(
          padding: const EdgeInsets.symmetric(vertical: 8),
          itemCount: svc.notifications.length,
          itemBuilder: (_, i) {
            final n = svc.notifications[i];
            return _NotificationCard(
              notification: n,
              onDismiss: () {
                ws.sendNotificationDismiss(n.id);
                svc.dismissNotification(n.id);
              },
              onMarkRead: () {
                ws.sendNotificationMarkRead(n.id);
                svc.dismissNotification(n.id);
              },
            );
          },
        );
      },
    );
  }

  void _showClearAllDialog(BuildContext context, NotificationService svc) {
    final colors = Theme.of(context).extension<AppColors>()!;
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text('Clear all notifications?', style: TextStyle(color: colors.text1)),
        content: Text(
          'This will remove ${svc.notifications.length} notifications.',
          style: TextStyle(color: colors.text2, fontSize: 13),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text2)),
          ),
          TextButton(
            onPressed: () {
              for (final n in List.of(svc.notifications)) {
                svc.dismissNotification(n.id);
              }
              Navigator.pop(ctx);
            },
            child: Text('Clear', style: TextStyle(color: colors.error)),
          ),
        ],
      ),
    );
  }
}

class _NotificationCard extends StatelessWidget {
  final ConduitNotification notification;
  final VoidCallback onDismiss;
  final VoidCallback onMarkRead;

  const _NotificationCard({
    required this.notification,
    required this.onDismiss,
    required this.onMarkRead,
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
    final colors = Theme.of(context).extension<AppColors>()!;

    return Dismissible(
      key: Key(notification.id),
      direction: DismissDirection.horizontal,
      onDismissed: (direction) {
        if (direction == DismissDirection.startToEnd) {
          onDismiss();
        } else {
          onMarkRead();
        }
      },
      background: Container(
        alignment: Alignment.centerLeft,
        padding: const EdgeInsets.only(left: 20),
        color: colors.error,
        child: const Icon(Icons.delete, color: Colors.white),
      ),
      secondaryBackground: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 20),
        color: colors.accent,
        child: const Icon(Icons.check, color: Colors.white),
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
            style: TextStyle(fontSize: 14, fontWeight: FontWeight.w500, color: colors.text1),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          subtitle: Text(
            notification.body,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: 13, color: colors.text2),
          ),
          trailing: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Text(
                notification.app,
                style: TextStyle(fontSize: 11, color: colors.text3),
              ),
              const SizedBox(height: 2),
              Text(
                notification.timeAgo,
                style: TextStyle(fontSize: 11, color: colors.text3),
              ),
            ],
          ),
          onTap: () => _showReplyDialog(context),
        ),
      ),
    );
  }

  void _showReplyDialog(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final controller = TextEditingController();
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text('Reply to ${notification.app}', style: TextStyle(color: colors.text1)),
        content: TextField(
          controller: controller,
          decoration: InputDecoration(
            hintText: 'Type a reply...',
            hintStyle: TextStyle(color: colors.text3),
          ),
          style: TextStyle(color: colors.text1),
          autofocus: true,
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text2)),
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
            child: Text('Send', style: TextStyle(color: colors.accent)),
          ),
        ],
      ),
    );
  }
}
