import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import '../services/clipboard_service.dart';
import '../theme/app_theme.dart';
import '../widgets/empty_state.dart';

class ClipboardScreen extends StatelessWidget {
  const ClipboardScreen({super.key});

  static String _itemCount(int count) => count == 1 ? '1 item' : '$count items';

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Clipboard'),
        actions: [
          Consumer<ClipboardService>(
            builder: (_, svc, _) {
              if (svc.history.isEmpty) return const SizedBox.shrink();
              return IconButton(
                icon: Icon(Icons.delete_sweep, color: colors.text2),
                tooltip: 'Clear history',
                onPressed: () => _showClearDialog(context, svc),
              );
            },
          ),
        ],
      ),
      body: Consumer<ClipboardService>(
        builder: (_, svc, _) {
          return Column(
            children: [
              // Auto-sync status
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
                color: colors.bg2,
                child: Row(
                  children: [
                    Icon(
                      svc.isListening ? Icons.sync : Icons.sync_disabled,
                      size: 16,
                      color: svc.isListening ? colors.accent : colors.text3,
                    ),
                    const SizedBox(width: 8),
                    Text(
                      svc.isListening
                          ? 'Auto-sync on — checked every 2 seconds'
                          : 'Auto-sync paused',
                      style: TextStyle(
                        fontSize: 12,
                        color: svc.isListening ? colors.accent : colors.text3,
                      ),
                    ),
                    const Spacer(),
                    if (svc.history.isNotEmpty)
                      Text(
                        _itemCount(svc.history.length),
                        style: TextStyle(fontSize: 12, color: colors.text3),
                      ),
                  ],
                ),
              ),
              Divider(height: 1, color: colors.border),
              // History list
              Expanded(
                child: svc.history.isEmpty
                    ? const EmptyState(
                        icon: Icons.content_paste,
                        title: 'No clipboard history',
                        description: 'Clipboard changes will appear here\nwhen auto-sync is active',
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
    final colors = Theme.of(context).extension<AppColors>()!;
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text('Clear clipboard history?', style: TextStyle(color: colors.text1)),
        content: Text(
          'This removes ${svc.history.length} '
          '${svc.history.length == 1 ? 'item' : 'items'} from history.',
          style: TextStyle(color: colors.text2, fontSize: 13),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text2)),
          ),
          TextButton(
            onPressed: () {
              svc.clearHistory();
              Navigator.pop(ctx);
            },
            child: Text('Clear', style: TextStyle(color: colors.error)),
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
    final colors = Theme.of(context).extension<AppColors>()!;

    return Card(
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: ListTile(
        leading: Container(
          width: 36,
          height: 36,
          decoration: BoxDecoration(
            color: colors.accent.withValues(alpha: 0.1),
            borderRadius: BorderRadius.circular(8),
          ),
          child: Icon(Icons.content_paste, color: colors.accent, size: 18),
        ),
        title: Text(
          item.content,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontSize: 13, color: colors.text1),
        ),
        subtitle: Text(
          '${item.sourceDevice} · ${_timeAgo(item.timestamp)}',
          style: TextStyle(fontSize: 11, color: colors.text3),
        ),
        trailing: IconButton(
          icon: Icon(Icons.copy, size: 16, color: colors.text3),
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
