import 'package:flutter/material.dart';
import '../services/file_service.dart';

class FileTile extends StatelessWidget {
  final FileTransfer transfer;
  final VoidCallback? onTap;
  final VoidCallback? onCancel;

  const FileTile({
    super.key,
    required this.transfer,
    this.onTap,
    this.onCancel,
  });

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: ListTile(
        leading: _buildIcon(),
        title: Text(
          transfer.name,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(fontWeight: FontWeight.w500),
        ),
        subtitle: _buildSubtitle(),
        trailing: _buildTrailing(),
        onTap: onTap,
      ),
    );
  }

  Widget _buildIcon() {
    IconData iconData;

    if (transfer.mime.startsWith('image/')) {
      iconData = Icons.image_outlined;
    } else if (transfer.mime.startsWith('video/')) {
      iconData = Icons.videocam_outlined;
    } else if (transfer.mime.startsWith('audio/')) {
      iconData = Icons.music_note_outlined;
    } else if (transfer.mime.contains('pdf')) {
      iconData = Icons.picture_as_pdf_outlined;
    } else if (transfer.mime.contains('zip') || transfer.mime.contains('tar')) {
      iconData = Icons.archive_outlined;
    } else {
      iconData = Icons.insert_drive_file_outlined;
    }

    return Container(
      width: 40,
      height: 40,
      decoration: BoxDecoration(
        color: const Color(0xFF16161E),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: const Color(0xFF1E1E28)),
      ),
      child: Icon(iconData, color: const Color(0xFF2DD4BF), size: 22),
    );
  }

  Widget _buildSubtitle() {
    final sizeStr = _formatSize(transfer.size);
    final statusStr = _statusText();

    if (transfer.status == TransferStatus.transferring) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 4),
          LinearProgressIndicator(
            value: transfer.progress,
            backgroundColor: const Color(0xFF16161E),
            valueColor: const AlwaysStoppedAnimation<Color>(Color(0xFF2DD4BF)),
            minHeight: 3,
          ),
          const SizedBox(height: 4),
          Text('$sizeStr • $statusStr (${(transfer.progress * 100).toInt()}%)',
              style: const TextStyle(fontSize: 12)),
        ],
      );
    }

    return Text('$sizeStr • $statusStr', style: const TextStyle(fontSize: 12));
  }

  Widget? _buildTrailing() {
    if (transfer.status == TransferStatus.pending) {
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          IconButton(
            icon: const Icon(Icons.check_outlined, color: Color(0xFF2DD4BF)),
            onPressed: onTap,
            tooltip: 'Accept',
          ),
          IconButton(
            icon: const Icon(Icons.close_outlined, color: Color(0xFFEF4444)),
            onPressed: onCancel,
            tooltip: 'Decline',
          ),
        ],
      );
    }

    if (transfer.status == TransferStatus.transferring && onCancel != null) {
      return IconButton(
        icon: const Icon(Icons.close_outlined, color: Color(0xFFEF4444)),
        onPressed: onCancel,
        tooltip: 'Cancel',
      );
    }

    if (transfer.status == TransferStatus.complete) {
      return const Icon(Icons.check_circle_outlined, color: Color(0xFF2DD4BF));
    }

    if (transfer.status == TransferStatus.failed) {
      return const Icon(Icons.error, color: Color(0xFFEF4444));
    }

    return null;
  }

  String _statusText() {
    switch (transfer.status) {
      case TransferStatus.pending:
        return 'Waiting to accept';
      case TransferStatus.transferring:
        return 'Transferring';
      case TransferStatus.complete:
        return 'Complete';
      case TransferStatus.failed:
        return 'Failed';
      case TransferStatus.cancelled:
        return 'Cancelled';
    }
  }

  String _formatSize(int bytes) {
    if (bytes <= 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    var size = bytes.toDouble();
    var i = 0;
    while (size >= k && i < sizes.length - 1) {
      size /= k;
      i++;
    }
    return '${size.toStringAsFixed(1)} ${sizes[i]}';
  }
}
