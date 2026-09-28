import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../theme/app_theme.dart';
import '../widgets/search_bar.dart';
import '../services/file_service.dart';
import '../services/notification_service.dart';
import '../services/clipboard_service.dart';

/// Full search screen with results grouped by category.
class SearchScreen extends StatefulWidget {
  const SearchScreen({super.key});

  @override
  State<SearchScreen> createState() => _SearchScreenState();
}

class _SearchScreenState extends State<SearchScreen> {
  String _query = '';

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    final fileService = context.watch<FileService>();
    final notifService = context.watch<NotificationService>();
    final clipService = context.watch<ClipboardService>();

    // Filter results
    final fileResults = _query.isEmpty
        ? <dynamic>[]
        : fileService.transfers.where((f) =>
            f.name.toLowerCase().contains(_query.toLowerCase())).toList();
    final notifResults = _query.isEmpty
        ? <dynamic>[]
        : notifService.notifications.where((n) =>
            n.title.toLowerCase().contains(_query.toLowerCase()) ||
            n.body.toLowerCase().contains(_query.toLowerCase())).toList();
    final clipResults = _query.isEmpty
        ? <dynamic>[]
        : clipService.history.where((c) =>
            c.content.toLowerCase().contains(_query.toLowerCase())).toList();

    final hasResults = fileResults.isNotEmpty || notifResults.isNotEmpty || clipResults.isNotEmpty;

    return Scaffold(
      appBar: AppBar(
        title: ConduitSearchBar(
          hint: 'Search files, notifications, clipboard',
          autofocus: true,
          onChanged: (q) => setState(() => _query = q),
        ),
        titleSpacing: 0,
      ),
      body: _query.isEmpty
          ? Center(
              child: Text(
                'Search files, notifications, and clipboard history',
                style: TextStyle(color: colors.text3, fontSize: 14),
              ),
            )
          : !hasResults
              ? Center(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(Icons.search_off, size: 48, color: colors.text3),
                      const SizedBox(height: 12),
                      Text(
                        'No results for "$_query"',
                        style: TextStyle(color: colors.text2, fontSize: 14),
                      ),
                    ],
                  ),
                )
              : ListView(
                  padding: const EdgeInsets.all(16),
                  children: [
                    if (fileResults.isNotEmpty) ...[
                      _SectionHeader(label: 'Files', colors: colors, count: fileResults.length),
                      ...fileResults.map((f) => _ResultTile(
                        icon: Icons.insert_drive_file,
                        title: f.name,
                        subtitle: f.status,
                        colors: colors,
                      )),
                      const SizedBox(height: 16),
                    ],
                    if (notifResults.isNotEmpty) ...[
                      _SectionHeader(label: 'Notifications', colors: colors, count: notifResults.length),
                      ...notifResults.map((n) => _ResultTile(
                        icon: Icons.notifications,
                        title: n.title,
                        subtitle: n.app,
                        colors: colors,
                      )),
                      const SizedBox(height: 16),
                    ],
                    if (clipResults.isNotEmpty) ...[
                      _SectionHeader(label: 'Clipboard', colors: colors, count: clipResults.length),
                      ...clipResults.map((c) => _ResultTile(
                        icon: Icons.content_paste,
                        title: c.content.length > 60
                            ? '${c.content.substring(0, 60)}...'
                            : c.content,
                        subtitle: c.source,
                        colors: colors,
                      )),
                    ],
                  ],
                ),
    );
  }
}

class _SectionHeader extends StatelessWidget {
  final String label;
  final AppColors colors;
  final int count;

  const _SectionHeader({required this.label, required this.colors, required this.count});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        children: [
          Text(
            label,
            style: TextStyle(
              fontSize: 13,
              fontWeight: FontWeight.w600,
              color: colors.text2,
            ),
          ),
          const SizedBox(width: 6),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
            decoration: BoxDecoration(
              color: colors.accent.withValues(alpha: 0.15),
              borderRadius: BorderRadius.circular(8),
            ),
            child: Text(
              '$count',
              style: TextStyle(fontSize: 10, fontWeight: FontWeight.w600, color: colors.accent),
            ),
          ),
        ],
      ),
    );
  }
}

class _ResultTile extends StatelessWidget {
  final IconData icon;
  final String title;
  final String? subtitle;
  final AppColors colors;

  const _ResultTile({
    required this.icon,
    required this.title,
    this.subtitle,
    required this.colors,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(bottom: 4),
      child: ListTile(
        leading: Icon(icon, size: 20, color: colors.accent),
        title: Text(title, style: TextStyle(fontSize: 13, color: colors.text1)),
        subtitle: subtitle != null
            ? Text(subtitle!, style: TextStyle(fontSize: 11, color: colors.text3))
            : null,
        dense: true,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
        tileColor: colors.bg2,
      ),
    );
  }
}
