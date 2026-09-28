import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/sms_service.dart';
import '../theme/app_theme.dart';
import '../widgets/message_bubble.dart';
import '../widgets/empty_state.dart';

class MessagesScreen extends StatelessWidget {
  const MessagesScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Messages'),
        actions: [
          IconButton(
            icon: Icon(Icons.message_outlined, color: colors.accent),
            tooltip: 'New message',
            onPressed: () => _showNewMessageDialog(context),
          ),
        ],
      ),
      body: Consumer<SmsService>(
        builder: (_, smsService, _) {
          if (!smsService.hasPermission) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(Icons.sms_outlined, size: 64, color: colors.text3),
                  const SizedBox(height: 16),
                  Text(
                    'SMS permission required',
                    style: TextStyle(fontSize: 16, color: colors.text2),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Allow Conduit to read your SMS messages so they '
                    'can be shared with your desktop',
                    style: TextStyle(fontSize: 13, color: colors.text3),
                  ),
                  const SizedBox(height: 24),
                  FilledButton(
                    onPressed: () => smsService.initialize(),
                    style: FilledButton.styleFrom(backgroundColor: colors.accent),
                    child: const Text('Grant permission'),
                  ),
                ],
              ),
            );
          }

          if (smsService.threads.isEmpty) {
            return const EmptyState(
              icon: Icons.chat_bubble_outline,
              title: 'No messages yet',
              description: 'Your SMS conversations will appear here',
            );
          }

          return ListView.builder(
            padding: const EdgeInsets.all(8),
            itemCount: smsService.threads.length,
            itemBuilder: (_, index) {
              final thread = smsService.threads[index];
              return Card(
                margin: const EdgeInsets.only(bottom: 4),
                child: ListTile(
                  leading: CircleAvatar(
                    backgroundColor: colors.bg2,
                    child: Text(
                      thread.name?.isNotEmpty == true
                          ? thread.name![0].toUpperCase()
                          : thread.address.length > 2
                              ? thread.address.substring(thread.address.length - 2)
                              : '?',
                      style: TextStyle(color: colors.accent, fontWeight: FontWeight.bold),
                    ),
                  ),
                  title: Text(
                    thread.name ?? thread.address,
                    style: TextStyle(
                      fontWeight: thread.unreadCount > 0 ? FontWeight.bold : FontWeight.normal,
                      color: colors.text1,
                    ),
                  ),
                  subtitle: Text(
                    thread.snippet,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: colors.text2,
                      fontWeight: thread.unreadCount > 0 ? FontWeight.w500 : FontWeight.normal,
                    ),
                  ),
                  trailing: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.end,
                    children: [
                      Text(
                        _formatTime(thread.timestamp),
                        style: TextStyle(fontSize: 11, color: colors.text3),
                      ),
                      if (thread.unreadCount > 0) ...[
                        const SizedBox(height: 4),
                        Container(
                          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                          decoration: BoxDecoration(
                            color: colors.accent.withValues(alpha: 0.15),
                            borderRadius: BorderRadius.circular(10),
                          ),
                          child: Text(
                            '${thread.unreadCount}',
                            style: TextStyle(color: colors.accent, fontSize: 10, fontWeight: FontWeight.bold),
                          ),
                        ),
                      ],
                    ],
                  ),
                  onTap: () => _openThread(context, thread),
                ),
              );
            },
          );
        },
      ),
    );
  }

  String _formatTime(int timestamp) {
    final date = DateTime.fromMillisecondsSinceEpoch(timestamp * 1000);
    final now = DateTime.now();
    if (date.day == now.day && date.month == now.month && date.year == now.year) {
      return '${date.hour.toString().padLeft(2, '0')}:${date.minute.toString().padLeft(2, '0')}';
    }
    return '${date.month}/${date.day}';
  }

  void _openThread(BuildContext context, SmsThread thread) {
    Navigator.push(
      context,
      MaterialPageRoute(
        builder: (_) => _ThreadDetailScreen(thread: thread),
      ),
    );
  }

  void _showNewMessageDialog(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final toController = TextEditingController();
    final bodyController = TextEditingController();

    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text('New message', style: TextStyle(color: colors.text1)),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextField(
              controller: toController,
              decoration: InputDecoration(
                labelText: 'To',
                labelStyle: TextStyle(color: colors.text3),
                hintText: '+1 234 567 900',
                hintStyle: TextStyle(color: colors.text3),
              ),
              style: TextStyle(color: colors.text1),
              keyboardType: TextInputType.phone,
            ),
            const SizedBox(height: 12),
            TextField(
              controller: bodyController,
              decoration: InputDecoration(
                labelText: 'Message',
                labelStyle: TextStyle(color: colors.text3),
                hintText: 'Type a message...',
                hintStyle: TextStyle(color: colors.text3),
              ),
              style: TextStyle(color: colors.text1),
              maxLines: 3,
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text2)),
          ),
          TextButton(
            onPressed: () async {
              final to = toController.text.trim();
              final body = bodyController.text.trim();
              if (to.isNotEmpty && body.isNotEmpty) {
                final smsService = context.read<SmsService>();
                await smsService.sendSms(to, body);
                if (ctx.mounted) Navigator.pop(ctx);
              }
            },
            child: Text('Send', style: TextStyle(color: colors.accent)),
          ),
        ],
      ),
    );
  }
}

class _ThreadDetailScreen extends StatefulWidget {
  final SmsThread thread;

  const _ThreadDetailScreen({required this.thread});

  @override
  State<_ThreadDetailScreen> createState() => _ThreadDetailScreenState();
}

class _ThreadDetailScreenState extends State<_ThreadDetailScreen> {
  final TextEditingController _messageController = TextEditingController();
  final ScrollController _scrollController = ScrollController();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _scrollToBottom());
  }

  void _scrollToBottom() {
    if (_scrollController.hasClients) {
      _scrollController.animateTo(
        _scrollController.position.maxScrollExtent,
        duration: const Duration(milliseconds: 200),
        curve: Curves.easeOut,
      );
    }
  }

  void _sendMessage() {
    final body = _messageController.text.trim();
    if (body.isEmpty) return;

    final smsService = context.read<SmsService>();
    smsService.sendSms(widget.thread.address, body);
    _messageController.clear();

    WidgetsBinding.instance.addPostFrameCallback((_) => _scrollToBottom());
  }

  @override
  void dispose() {
    _messageController.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(Icons.arrow_back),
          onPressed: () => Navigator.pop(context),
        ),
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              widget.thread.name ?? widget.thread.address,
              style: TextStyle(fontSize: 16, fontWeight: FontWeight.w600, color: colors.text1),
            ),
            Text(
              widget.thread.address,
              style: TextStyle(fontSize: 12, color: colors.text3),
            ),
          ],
        ),
      ),
      body: Column(
        children: [
          Expanded(
            child: Consumer<SmsService>(
              builder: (_, smsService, _) {
                final thread = smsService.threads.firstWhere(
                  (t) => t.threadId == widget.thread.threadId,
                  orElse: () => widget.thread,
                );

                if (thread.messages.isEmpty) {
                  return Center(
                    child: Text('No messages', style: TextStyle(color: colors.text3)),
                  );
                }

                return ListView.builder(
                  controller: _scrollController,
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  itemCount: thread.messages.length,
                  itemBuilder: (_, index) {
                    final msg = thread.messages[index];
                    return MessageBubble(
                      body: msg.body,
                      timestamp: msg.timestamp,
                      isOutgoing: msg.isOutgoing,
                      read: msg.read,
                    );
                  },
                );
              },
            ),
          ),
          Container(
            padding: const EdgeInsets.all(8),
            decoration: BoxDecoration(
              color: colors.bg0,
              border: Border(top: BorderSide(color: colors.border)),
            ),
            child: SafeArea(
              child: Row(
                children: [
                  Expanded(
                    child: TextField(
                      controller: _messageController,
                      decoration: InputDecoration(
                        hintText: 'Type a message...',
                        hintStyle: TextStyle(color: colors.text3),
                        filled: true,
                        fillColor: colors.bg2,
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(20),
                          borderSide: BorderSide.none,
                        ),
                        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
                      ),
                      style: TextStyle(color: colors.text1),
                      maxLines: null,
                      textInputAction: TextInputAction.send,
                      onSubmitted: (_) => _sendMessage(),
                    ),
                  ),
                  const SizedBox(width: 8),
                  IconButton(
                    icon: Icon(Icons.send_outlined, color: colors.accent),
                    onPressed: _sendMessage,
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
