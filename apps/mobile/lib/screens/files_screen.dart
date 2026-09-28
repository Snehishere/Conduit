import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:file_picker/file_picker.dart';
import '../services/file_service.dart';
import '../services/websocket_service.dart';
import '../theme/app_theme.dart';
import '../widgets/file_tile.dart';
import '../widgets/empty_state.dart';

class FilesScreen extends StatelessWidget {
  const FilesScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Files'),
        actions: [
          IconButton(
            icon: Icon(Icons.send_outlined, color: colors.accent),
            tooltip: 'Send file',
            onPressed: () => _sendFile(context),
          ),
        ],
      ),
      body: Consumer<FileService>(
        builder: (_, fileService, _) {
          if (fileService.transfers.isEmpty) {
            return const EmptyState(
              icon: Icons.folder_open_outlined,
              title: 'No file transfers yet',
              description: 'Tap the send button to send a file to your desktop',
            );
          }

          return ListView.builder(
            padding: const EdgeInsets.all(8),
            itemCount: fileService.transfers.length,
            itemBuilder: (_, index) {
              final transfer = fileService.transfers[index];
              return FileTile(
                transfer: transfer,
                onTap: transfer.status == TransferStatus.pending
                    ? () => _acceptTransfer(context, transfer.id)
                    : transfer.status == TransferStatus.complete && transfer.savedPath != null
                        ? () => _openFile(context, transfer.savedPath!)
                        : null,
                onCancel:
                    transfer.status == TransferStatus.pending || transfer.status == TransferStatus.transferring
                        ? () => _cancelTransfer(context, transfer.id)
                        : null,
              );
            },
          );
        },
      ),
    );
  }

  Future<void> _sendFile(BuildContext context) async {
    try {
      final result = await FilePicker.pickFiles();
      if (!context.mounted) return;
      if (result.isNotEmpty) {
        final fileService = context.read<FileService>();
        final wsService = context.read<WebSocketService>();

        if (!wsService.isConnected) {
          if (context.mounted) {
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(content: Text('Not connected to a device')),
            );
          }
          return;
        }

        final targetDeviceId = await _pickTargetDevice(context);
        if (targetDeviceId == null) return;

        for (final file in result) {
          if (file.path != null) {
            await fileService.sendFile(targetDeviceId, file.path!);
          }
        }
      }
    } catch (e) {
      debugPrint('Failed to pick file: $e');
    }
  }

  Future<String?> _pickTargetDevice(BuildContext context) async {
    final wsService = context.read<WebSocketService>();
    final colors = Theme.of(context).extension<AppColors>()!;
    final devices = wsService.connectedDevices;

    if (devices.isEmpty) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('No devices connected')),
        );
      }
      return null;
    }

    if (devices.length == 1) {
      return devices.first['device_id'] as String?;
    }

    return showModalBottomSheet<String>(
      context: context,
      backgroundColor: colors.bg1,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.all(16),
              child: Text(
                'Send to',
                style: TextStyle(
                  fontSize: 18,
                  fontWeight: FontWeight.bold,
                  color: colors.text1,
                ),
              ),
            ),
            ...devices.map((d) => ListTile(
                  leading: Icon(
                    d['device_type'] == 'desktop' ? Icons.computer : Icons.phone_android,
                    color: colors.accent,
                  ),
                  title: Text(
                    d['device_name'] as String? ?? 'Unknown',
                    style: TextStyle(color: colors.text1),
                  ),
                  subtitle: Text(
                    d['device_type'] as String? ?? '',
                    style: TextStyle(color: colors.text2, fontSize: 12),
                  ),
                  onTap: () => Navigator.pop(ctx, d['device_id'] as String?),
                )),
            const SizedBox(height: 8),
          ],
        ),
      ),
    );
  }

  void _acceptTransfer(BuildContext context, String id) {
    context.read<FileService>().acceptTransfer(id);
  }

  void _cancelTransfer(BuildContext context, String id) {
    context.read<FileService>().cancelTransfer(id);
  }

  void _openFile(BuildContext context, String path) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('File saved to: $path')),
    );
  }
}
