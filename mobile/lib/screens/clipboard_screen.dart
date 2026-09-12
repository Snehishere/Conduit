import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import '../services/clipboard_service.dart';

class ClipboardScreen extends StatelessWidget {
  const ClipboardScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Clipboard'),
        actions: [
          Consumer<ClipboardService>(
            builder: (_, svc, __) {
              if (svc.history.isEmpty) return const SizedBox.shrink();
              return IconButton(
                icon: const Icon(Icons.delete_sweep),
                tooltip: 'Clear history',
                onPressed: () => _showClearDialog(context, svc),
              );
            },
          ),
        ],
      ),
      body: Consumer<ClipboardService>(
        builder: (_, svc, __) {
          return Column(
            children: [
              // Auto-sync status
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
                color: const Color(0xFF12121A),
                child: Row(
                  children: [
                    Icon(
                      svc.isListening ? Icons.sync : Icons.sync_disabled,
                      size: 16,
                      color: svc.isListening
                          ? const Color(0xFF2DD4BF)
                          : const Color(0xFF5C586A),
                    ),
                    const SizedBox(width: 8),
                    Text(
                      svc.isListening
                          ? 'Auto-syncing — changes detected every 2s'
                          : 'Auto-sync paused',
                      style: TextStyle(
                        fontSize: 12,
                        color: svc.isListening
                            ? const Color(0xFF2DD4BF)
                            : const Color(0xFF5C586A),
                      ),
                    ),
                    const Spacer(),
                    if (svc.history.isNotEmpty)
                      Text(
                        '${svc.history.length} items',
                        style: const TextStyle(
                          fontSize: 12,
                          color: Color(0xFF5C586A),
                        ),
                      ),
                  ],
                ),
              ),
              const Divider(height: 1),
              // History list
              Expanded(
                child: svc.history.isEmpty
                    ? Center(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Icon(Icons.content_paste,
                                size: 64, color: Colors.grey.shade600),
                            const SizedBox(height: 16),
                            Text(
                              'No clipboard history',
                              style: TextStyle(
                                  fontSize: 16, color: Colors.grey.shade400),
                            ),
                            const SizedBox(height: 8),
                            Text(
                              'Clipboard changes will appear here\nwhen auto-sync is active',
                              textAlign: TextAlign.center,
                              style: TextStyle(
                                  fontSize: 13, color: Colors.grey.shade600),
                            ),
                          ],
                        ),
                      )
                    : ListView.builder(
                        padding: const EdgeInsets.symmetric(vertical: 8),
                        itemCount: svc.history.length,
                        itemBuilder: (_, i) {
                          final item = svc.history[i];
                          return _ClipboardItemCard(item: item);
                        },
                      ),
              ),
            ],
          );
        },
      ),
    );
  }

  void _showClearDialog(BuildContext context, ClipboardService svc) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Clear clipboard history?'),
        content: Text(
            'This will remove ${svc.history.length} items from history.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () {
              svc.clearHistory();
              Navigator.pop(ctx);
            },
            child: const Text('Clear'),
          ),
        ],
      ),
    );
  }
}

class _ClipboardItemCard extends StatelessWidget {
  final ClipboardItem item;

  const _ClipboardItemCard({required this.item});

  String _timeAgo(int timestamp) {
    final date = DateTime.fromMillisecondsSinceEpoch(timestamp * 1000);
    final now = DateTime.now();
    final diff = now.difference(date);
    if (diff.inMinutes < 1) return 'just now';
    if (diff.inMinutes < 60) return '${diff.inMinutes}m ago';
    if (diff.inHours < 24) return '${diff.inHours}h ago';
    return '${diff.inDays}d ago';
  }

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: ListTile(
        leading: Container(
          width: 36,
          height: 36,
          decoration: BoxDecoration(
            color: const Color(0xFF2DD4BF).withValues(alpha: 0.1),
            borderRadius: BorderRadius.circular(8),
          ),
          child: const Icon(Icons.content_paste,
              color: Color(0xFF2DD4BF), size: 18),
        ),
        title: Text(
          item.content,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(fontSize: 13),
        ),
        subtitle: Text(
          '${item.sourceDevice} · ${_timeAgo(item.timestamp)}',
          style: const TextStyle(fontSize: 11, color: Color(0xFF5C586A)),
        ),
        trailing: IconButton(
          icon: const Icon(Icons.copy, size: 16, color: Color(0xFF5C586A)),
          tooltip: 'Copy',
          onPressed: () {
            Clipboard.setData(ClipboardData(text: item.content));
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(
                content: Text('Copied to clipboard'),
                duration: Duration(seconds: 1),
              ),
            );
          },
        ),
      ),
    );
  }
}
