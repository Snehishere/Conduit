import 'package:flutter/material.dart';
import '../services/file_service.dart';
import '../theme/app_theme.dart';

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
    final colors = Theme.of(context).extension<AppColors>()!;

    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: ListTile(
        leading: _buildIcon(colors),
        title: Text(
          transfer.name,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontWeight: FontWeight.w500, color: colors.text1),
        ),
        subtitle: _buildSubtitle(colors),
        trailing: _buildTrailing(colors),
        onTap: onTap,
      ),
    );
  }

  Widget _buildIcon(AppColors colors) {
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
        color: colors.bg2,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: colors.border),
      ),
      child: Icon(iconData, color: colors.accent, size: 22),
    );
  }

  Widget _buildSubtitle(AppColors colors) {
    final sizeStr = _formatSize(transfer.size);
    final statusStr = _statusText();

    if (transfer.status == TransferStatus.transferring) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 4),
          LinearProgressIndicator(
            value: transfer.progress,
            backgroundColor: colors.bg2,
            valueColor: AlwaysStoppedAnimation<Color>(colors.accent),
            minHeight: 3,
          ),
          const SizedBox(height: 4),
          Text(
            '$sizeStr • $statusStr (${(transfer.progress * 100).toInt()}%)',
            style: TextStyle(fontSize: 12, color: colors.text2),
          ),
        ],
      );
    }

    return Text(
      '$sizeStr • $statusStr',
      style: TextStyle(fontSize: 12, color: colors.text2),
    );
  }

  Widget? _buildTrailing(AppColors colors) {
    if (transfer.status == TransferStatus.pending) {
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          IconButton(
            icon: Icon(Icons.check_outlined, color: colors.accent),
            onPressed: onTap,
            tooltip: 'Accept',
          ),
          IconButton(
            icon: Icon(Icons.close_outlined, color: colors.error),
            onPressed: onCancel,
            tooltip: 'Decline',
          ),
        ],
      );
    }

    if (transfer.status == TransferStatus.transferring && onCancel != null) {
      return IconButton(
        icon: Icon(Icons.close_outlined, color: colors.error),
        onPressed: onCancel,
        tooltip: 'Cancel',
      );
    }

    if (transfer.status == TransferStatus.complete) {
      return Icon(Icons.check_circle_outlined, color: colors.accent);
    }

    if (transfer.status == TransferStatus.failed) {
      return Icon(Icons.error, color: colors.error);
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
