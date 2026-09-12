import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as p;

const int _chunkSize = 64 * 1024; // 64KB

enum TransferStatus { pending, transferring, complete, failed, cancelled }

class FileTransfer {
  final String id;
  final String name;
  final int size;
  final String mime;
  final String fromDevice;
  final String toDevice;
  TransferStatus status;
  int chunksReceived;
  final int totalChunks;
  String? savedPath;
  final int timestamp;

  FileTransfer({
    required this.id,
    required this.name,
    required this.size,
    required this.mime,
    required this.fromDevice,
    required this.toDevice,
    this.status = TransferStatus.pending,
    this.chunksReceived = 0,
    required this.totalChunks,
    this.savedPath,
    required this.timestamp,
  });

  double get progress => totalChunks > 0 ? chunksReceived / totalChunks : 0;

  factory FileTransfer.fromJson(Map<String, dynamic> json) {
    return FileTransfer(
      id: json['id'],
      name: json['name'],
      size: json['size'],
      mime: json['mime'] ?? 'application/octet-stream',
      fromDevice: json['from_device'],
      toDevice: json['to_device'],
      status: TransferStatus.values.firstWhere(
        (e) => e.name == json['status'],
        orElse: () => TransferStatus.pending,
      ),
      chunksReceived: json['chunks_received'] ?? 0,
      totalChunks: json['total_chunks'] ?? 1,
      savedPath: json['saved_path'],
      timestamp: json['timestamp'],
    );
  }
}

class FileService extends ChangeNotifier {
  final List<FileTransfer> _transfers = [];
  String? _deviceId;
  void Function(Map<String, dynamic>)? _sendMessage;

  List<FileTransfer> get transfers => List.unmodifiable(_transfers);

  void setDeviceId(String id) {
    _deviceId = id;
  }

  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }

  FileTransfer? _findTransfer(String id) {
    for (final t in _transfers) {
      if (t.id == id) return t;
    }
    return null;
  }

  Future<void> sendFile(String targetDeviceId, String filePath) async {
    final file = File(filePath);
    if (!await file.exists()) {
      debugPrint('File does not exist: $filePath');
      return;
    }

    final bytes = await file.readAsBytes();
    final name = p.basename(filePath);
    final size = bytes.length;
    final mime = _guessMime(name);
    final totalChunks = (size / _chunkSize).ceil();
    final transferId = '${DateTime.now().millisecondsSinceEpoch}_$name';

    final transfer = FileTransfer(
      id: transferId,
      name: name,
      size: size,
      mime: mime,
      fromDevice: _deviceId ?? 'mobile',
      toDevice: targetDeviceId,
      status: TransferStatus.transferring,
      totalChunks: totalChunks,
      timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    _transfers.insert(0, transfer);
    notifyListeners();

    // Send file request
    debugPrint('Sending file request: $name ($size bytes, $totalChunks chunks)');

    // Send file request via WebSocket
    _sendMessage?.call({
      'type': 'file',
      'action': 'request',
      'id': transferId,
      'name': name,
      'size': size,
      'mime': mime,
      'from': _deviceId ?? 'mobile',
      'to': targetDeviceId,
    });

    // Wait for the receiver to accept before streaming chunks.
    // The accept arrives via handleFileAccepted(); poll the transfer status
    // with a timeout so we never blast chunks at an unwilling receiver.
    const maxWaitMs = 30000;
    const pollMs = 200;
    var waited = 0;
    while (waited < maxWaitMs) {
      final current = _transfers.where((t) => t.id == transferId).firstOrNull;
      if (current == null) return; // cancelled
      if (current.status == TransferStatus.transferring || current.status == TransferStatus.complete) break;
      // 'pending' + explicit accept flips status via handleFileAccepted.
      // Also accept an out-of-band accept that flips to transferring.
      await Future.delayed(const Duration(milliseconds: pollMs));
      waited += pollMs;
    }
    final ready = _transfers.where((t) => t.id == transferId).firstOrNull;
    if (ready == null || ready.status == TransferStatus.cancelled || ready.status == TransferStatus.failed) {
      return;
    }

    // Send file chunks
    _sendChunks(transferId, bytes, targetDeviceId);
  }

  Future<void> _sendChunks(String transferId, Uint8List bytes, String targetDeviceId) async {
    final totalChunks = (bytes.length / _chunkSize).ceil();
    for (var i = 0; i < totalChunks; i++) {
      final start = i * _chunkSize;
      final end = (start + _chunkSize).clamp(0, bytes.length);
      final chunk = bytes.sublist(start, end);
      final dataB64 = base64Encode(chunk);

      _sendMessage?.call({
        'type': 'file',
        'action': 'chunk',
        'id': transferId,
        'index': i,
        'data': dataB64,
      });

      // Small delay between chunks to avoid overwhelming the connection
      await Future.delayed(const Duration(milliseconds: 10));
    }

    // Send completion notification
    _sendMessage?.call({
      'type': 'file',
      'action': 'complete',
      'id': transferId,
    });

    // Update local transfer status
    final transfer = _findTransfer(transferId);
    if (transfer == null) return;
    transfer.status = TransferStatus.complete;
    notifyListeners();
  }

  void handleFileAccepted(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.status = TransferStatus.transferring;
    notifyListeners();
  }

  void handleFileCancelled(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.status = TransferStatus.cancelled;
    notifyListeners();
  }

  void handleFileProgress(String id, int chunksReceived) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.chunksReceived = chunksReceived;
    notifyListeners();
  }

  void handleFileRequest(Map<String, dynamic> msg) {
    final id = msg['id'] as String;
    final name = msg['name'] as String;
    final size = msg['size'] as int;
    final mime = msg['mime'] as String? ?? 'application/octet-stream';
    final from = msg['from'] as String;
    final totalChunks = (size / _chunkSize).ceil();

    final transfer = FileTransfer(
      id: id,
      name: name,
      size: size,
      mime: mime,
      fromDevice: from,
      toDevice: _deviceId ?? 'mobile',
      totalChunks: totalChunks,
      timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    _transfers.insert(0, transfer);
    notifyListeners();
  }

  Map<String, dynamic> acceptTransfer(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return {'error': 'Transfer not found'};
    transfer.status = TransferStatus.transferring;
    notifyListeners();
    _sendMessage?.call({'type': 'file', 'action': 'accept', 'id': id});
    return {'type': 'file', 'action': 'accept', 'id': id};
  }

  Future<void> receiveChunk(String id, int index, String dataB64) async {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    final data = base64Decode(dataB64);

    // Save chunks to temp directory for reassembly
    final tempDir = await getTemporaryDirectory();
    final chunkFile = File('${tempDir.path}/conduit_${id}_chunk_$index');
    await chunkFile.writeAsBytes(data);

    transfer.chunksReceived++;
    notifyListeners();

    // Relay progress to sender
    _sendMessage?.call({
      'type': 'file',
      'action': 'progress',
      'id': id,
      'chunks_received': transfer.chunksReceived,
    });

    if (transfer.chunksReceived >= transfer.totalChunks) {
      await _reassemble(transfer);
    }
  }

  Future<void> _reassemble(FileTransfer transfer) async {
    try {
      final tempDir = await getTemporaryDirectory();
      final outputDir = await getDownloadsDirectory();
      final outputPath = '${outputDir?.path ?? tempDir.path}/${transfer.name}';

      final sink = File(outputPath).openWrite();
      for (var i = 0; i < transfer.totalChunks; i++) {
        final chunkFile = File('${tempDir.path}/conduit_${transfer.id}_chunk_$i');
        if (await chunkFile.exists()) {
          final bytes = await chunkFile.readAsBytes();
          sink.add(bytes);
          await chunkFile.delete();
        }
      }
      await sink.flush();
      await sink.close();

      transfer.status = TransferStatus.complete;
      transfer.savedPath = outputPath;
      notifyListeners();
      debugPrint('File saved: $outputPath');
    } catch (e) {
      transfer.status = TransferStatus.failed;
      notifyListeners();
      debugPrint('Failed to reassemble file: $e');
    }
  }

  void cancelTransfer(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.status = TransferStatus.cancelled;
    notifyListeners();
    _sendMessage?.call({'type': 'file', 'action': 'cancel', 'id': id});
  }

  void handleResumeAck(String id, int chunksLoaded) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.chunksReceived = chunksLoaded;
    transfer.status = TransferStatus.transferring;
    notifyListeners();
    debugPrint('Resume ack: $id ($chunksLoaded chunks loaded)');
  }

  void resumeTransfer(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    _sendMessage?.call({
      'type': 'file',
      'action': 'resume',
      'id': id,
      'name': transfer.name,
      'size': transfer.size,
      'mime': transfer.mime,
      'from': transfer.fromDevice,
    });
    transfer.status = TransferStatus.transferring;
    notifyListeners();
  }

  String _guessMime(String name) {
    final ext = p.extension(name).toLowerCase();
    switch (ext) {
      case '.jpg':
      case '.jpeg':
        return 'image/jpeg';
      case '.png':
        return 'image/png';
      case '.gif':
        return 'image/gif';
      case '.mp4':
        return 'video/mp4';
      case '.mp3':
        return 'audio/mpeg';
      case '.pdf':
        return 'application/pdf';
      case '.zip':
        return 'application/zip';
      case '.txt':
        return 'text/plain';
      default:
        return 'application/octet-stream';
    }
  }
}
