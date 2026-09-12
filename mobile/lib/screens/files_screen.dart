import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:file_picker/file_picker.dart';
import '../services/file_service.dart';
import '../services/websocket_service.dart';
import '../widgets/file_tile.dart';

class FilesScreen extends StatelessWidget {
  const FilesScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Files'),
        actions: [
          IconButton(
            icon: const Icon(Icons.send_outlined),
            tooltip: 'Send file',
            onPressed: () => _sendFile(context),
          ),
        ],
      ),
      body: Consumer<FileService>(
        builder: (_, fileService, __) {
          if (fileService.transfers.isEmpty) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(Icons.folder_open_outlined, size: 64, color: const Color(0xFF55515E)),
                  const SizedBox(height: 16),
                  Text(
                    'No file transfers yet',
                    style: TextStyle(
                      fontSize: 16,
                      color: Colors.grey.shade400,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Tap the send button to share files',
                    style: TextStyle(
                      fontSize: 13,
                      color: Colors.grey.shade600,
                    ),
                  ),
                ],
              ),
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
      final result = await FilePicker.platform.pickFiles(allowMultiple: true);
      if (result != null && result.files.isNotEmpty) {
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

        // Show device picker
        final targetDeviceId = await _pickTargetDevice(context);
        if (targetDeviceId == null) return;

        for (final file in result.files) {
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
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Padding(
              padding: EdgeInsets.all(16),
              child: Text('Send to', style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
            ),
            ...devices.map((d) => ListTile(
                  leading: Icon(
                    d['device_type'] == 'desktop' ? Icons.computer : Icons.phone_android,
                    color: const Color(0xFF2dd4bf),
                  ),
                  title: Text(d['device_name'] as String? ?? 'Unknown'),
                  subtitle: Text(d['device_type'] as String? ?? ''),
                  onTap: () => Navigator.pop(ctx, d['device_id'] as String?),
                )),
            const SizedBox(height: 8),
          ],
        ),
      ),
    );
  }

  void _acceptTransfer(BuildContext context, String id) {
    final fileService = context.read<FileService>();
    fileService.acceptTransfer(id);
  }

  void _cancelTransfer(BuildContext context, String id) {
    final fileService = context.read<FileService>();
    fileService.cancelTransfer(id);
  }

  Future<void> _openFile(BuildContext context, String path) async {
    try {
      // No open_file dependency — surface the path so the user can open it manually.
      if (!context.mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('File saved to: $path')),
      );
    } catch (e) {
      debugPrint('Failed to open file: $e');
    }
  }
}
