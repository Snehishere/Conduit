import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/sms_service.dart';
import '../widgets/message_bubble.dart';

class MessagesScreen extends StatelessWidget {
  const MessagesScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Messages'),
        actions: [
          IconButton(
            icon: const Icon(Icons.message_outlined),
            tooltip: 'New message',
            onPressed: () => _showNewMessageDialog(context),
          ),
        ],
      ),
      body: Consumer<SmsService>(
        builder: (_, smsService, __) {
          if (!smsService.hasPermission) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(Icons.sms_outlined, size: 64, color: const Color(0xFF55515E)),
                  const SizedBox(height: 16),
                  Text(
                    'SMS permission required',
                    style: TextStyle(fontSize: 16, color: Colors.grey.shade400),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Allow Conduit to access your messages',
                    style: TextStyle(fontSize: 13, color: Colors.grey.shade600),
                  ),
                  const SizedBox(height: 24),
                  ElevatedButton(
                    onPressed: () => smsService.initialize(),
                    child: const Text('Grant Permission'),
                  ),
                ],
              ),
            );
          }

          if (smsService.threads.isEmpty) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(Icons.chat_bubble_outline, size: 64, color: Colors.grey.shade600),
                  const SizedBox(height: 16),
                  Text(
                    'No messages yet',
                    style: TextStyle(fontSize: 16, color: Colors.grey.shade400),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Your SMS conversations will appear here',
                    style: TextStyle(fontSize: 13, color: Colors.grey.shade600),
                  ),
                ],
              ),
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
                    backgroundColor: const Color(0xFF16161E),
                    child: Text(
                      thread.name?.isNotEmpty == true
                          ? thread.name![0].toUpperCase()
                          : thread.address.length > 2
                              ? thread.address.substring(thread.address.length - 2)
                              : '?',
                      style: const TextStyle(color: Color(0xFF2DD4BF), fontWeight: FontWeight.bold),
                    ),
                  ),
                  title: Text(
                    thread.name ?? thread.address,
                    style: TextStyle(
                      fontWeight: thread.unreadCount > 0 ? FontWeight.bold : FontWeight.normal,
                    ),
                  ),
                  subtitle: Text(
                    thread.snippet,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: Colors.grey.shade400,
                      fontWeight: thread.unreadCount > 0 ? FontWeight.w500 : FontWeight.normal,
                    ),
                  ),
                  trailing: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.end,
                    children: [
                      Text(
                        _formatTime(thread.timestamp),
                        style: TextStyle(fontSize: 11, color: Colors.grey.shade500),
                      ),
                      if (thread.unreadCount > 0) ...[
                        const SizedBox(height: 4),
                        Container(
                          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                          decoration: BoxDecoration(
                            color: const Color(0xFF1A2E2B),
                            borderRadius: BorderRadius.circular(10),
                          ),
                          child: Text(
                            '${thread.unreadCount}',
                            style: const TextStyle(color: Color(0xFF2DD4BF), fontSize: 10, fontWeight: FontWeight.bold),
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
    final toController = TextEditingController();
    final bodyController = TextEditingController();

    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('New Message'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextField(
              controller: toController,
              decoration: const InputDecoration(
                labelText: 'To',
                hintText: '+1 234 567 8900',
              ),
              keyboardType: TextInputType.phone,
            ),
            const SizedBox(height: 12),
            TextField(
              controller: bodyController,
              decoration: const InputDecoration(
                labelText: 'Message',
                hintText: 'Type a message...',
              ),
              maxLines: 3,
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
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
            child: const Text('Send'),
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
              style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
            ),
            Text(
              widget.thread.address,
              style: const TextStyle(fontSize: 12, color: Color(0xFF808090)),
            ),
          ],
        ),
      ),
      body: Column(
        children: [
          Expanded(
            child: Consumer<SmsService>(
              builder: (_, smsService, __) {
                final thread = smsService.threads.firstWhere(
                  (t) => t.threadId == widget.thread.threadId,
                  orElse: () => widget.thread,
                );

                if (thread.messages.isEmpty) {
                  return const Center(
                    child: Text('No messages', style: TextStyle(color: Color(0xFF808090))),
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
            decoration: const BoxDecoration(
              color: Color(0xFF0E0E14),
              border: Border(top: BorderSide(color: Color(0xFF1A1A24))),
            ),
            child: SafeArea(
              child: Row(
                children: [
                  Expanded(
                    child: TextField(
                      controller: _messageController,
                      decoration: InputDecoration(
                        hintText: 'Type a message...',
                        hintStyle: const TextStyle(color: Color(0xFF55515E)),
                        filled: true,
                        fillColor: const Color(0xFF16161E),
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(20),
                          borderSide: BorderSide.none,
                        ),
                        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
                      ),
                      style: const TextStyle(color: Color(0xFFD4D0DC)),
                      maxLines: null,
                      textInputAction: TextInputAction.send,
                      onSubmitted: (_) => _sendMessage(),
                    ),
                  ),
                  const SizedBox(width: 8),
                  IconButton(
                    icon: const Icon(Icons.send_outlined, color: Color(0xFF2DD4BF)),
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
