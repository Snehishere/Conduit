import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as p;
import 'package:cryptography/cryptography.dart';
import 'encryption_service.dart';
import 'database_service.dart';
import '../models/persisted_transfer.dart';

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
  String? expectedChecksum;

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
    this.expectedChecksum,
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
  void Function(Uint8List)? _sendBinaryMessage;
  String? Function()? getSharedSecret;
  Function(String)? _onError;
  DatabaseService? _db;

  List<FileTransfer> get transfers => List.unmodifiable(_transfers);

  /// Inject the database service and load persisted transfer history.
  void setDatabase(DatabaseService db) {
    _db = db;
    _loadTransfersFromDb();
  }

  /// Load file transfer history from SQLite on startup.
  Future<void> _loadTransfersFromDb() async {
    if (_db == null) return;
    try {
      final rows = await _db!.getFileTransfers(limit: 100);
      for (final row in rows) {
        final persisted = PersistedTransfer.fromMap(row);
        // Skip transfers still in-progress (stale from a previous session)
        if (persisted.status == 'transferring' || persisted.status == 'pending') continue;

        _transfers.add(FileTransfer(
          id: persisted.id,
          name: persisted.name,
          size: persisted.size,
          mime: persisted.mime,
          fromDevice: persisted.fromDevice,
          toDevice: persisted.toDevice,
          status: TransferStatus.values.firstWhere(
            (e) => e.name == persisted.status,
            orElse: () => TransferStatus.pending,
          ),
          chunksReceived: persisted.chunksReceived,
          totalChunks: persisted.totalChunks,
          savedPath: persisted.savedPath,
          timestamp: persisted.timestamp,
          expectedChecksum: persisted.expectedChecksum,
        ));
      }
      if (_transfers.isNotEmpty) {
        debugPrint('[FileService] Loaded ${_transfers.length} transfers from DB');
        notifyListeners();
      }
    } catch (e) {
      debugPrint('[FileService] Failed to load transfers from DB: $e');
    }
  }

  /// Persist a new file transfer to SQLite.
  Future<void> _persistTransfer(FileTransfer transfer) async {
    if (_db == null) return;
    try {
      await _db!.insertFileTransfer(PersistedTransfer(
        id: transfer.id,
        name: transfer.name,
        size: transfer.size,
        mime: transfer.mime,
        fromDevice: transfer.fromDevice,
        toDevice: transfer.toDevice,
        status: transfer.status.name,
        chunksReceived: transfer.chunksReceived,
        totalChunks: transfer.totalChunks,
        savedPath: transfer.savedPath,
        expectedChecksum: transfer.expectedChecksum,
        timestamp: transfer.timestamp,
        createdAt: DateTime.now().millisecondsSinceEpoch,
      ).toMap());
    } catch (e) {
      debugPrint('[FileService] Failed to persist transfer: $e');
    }
  }

  /// Update an existing file transfer record in SQLite.
  Future<void> _updateTransferInDb(String id, Map<String, dynamic> values) async {
    if (_db == null) return;
    try {
      await _db!.updateFileTransfer(id, values);
    } catch (e) {
      debugPrint('[FileService] Failed to update transfer in DB: $e');
    }
  }

  /// Compute SHA-256 hex digest of bytes.
  static Future<String> _computeSha256(List<int> bytes) async {
    final algorithm = Sha256();
    final digest = await algorithm.hash(bytes);
    return digest.bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
  }

  void onError(Function(String) callback) {
    _onError = callback;
  }

  void _handleError(String message) {
    debugPrint('[FileService] Error: $message');
    _onError?.call(message);
  }

  void setDeviceId(String id) {
    _deviceId = id;
  }

  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }

  void setSendBinaryFunction(void Function(Uint8List) sendFn) {
    _sendBinaryMessage = sendFn;
  }

  void setGetSharedSecret(String? Function() fn) {
    getSharedSecret = fn;
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
      _handleError('File does not exist: $filePath');
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
    _pruneOldTransfers();
    notifyListeners();
    _persistTransfer(transfer);

    // Compute SHA-256 checksum for integrity verification
    final checksum = await _computeSha256(bytes);

    // Send file request
    debugPrint('Sending file request: $name ($size bytes, $totalChunks chunks, checksum: ${checksum.substring(0, 16)}...)');

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
      'checksum': checksum,
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
    final sharedSecretHex = getSharedSecret?.call();
    if (sharedSecretHex == null) {
      _handleError('Cannot send file chunks: missing shared secret');
      return;
    }
    final secretBytes = EncryptionService.hexToBytes(sharedSecretHex);
    final totalChunks = (bytes.length / _chunkSize).ceil();

    for (var i = 0; i < totalChunks; i++) {
      final start = i * _chunkSize;
      final end = (start + _chunkSize).clamp(0, bytes.length);
      final chunk = bytes.sublist(start, end);
      
      final metadataStr = jsonEncode({
        'id': transferId,
        'index': i,
        'total': totalChunks
      });
      final metadataBytes = utf8.encode(metadataStr);
      final byteData = ByteData(4)..setUint32(0, metadataBytes.length, Endian.little);
      
      final (nonce, encryptedChunkWithMac) = await EncryptionService().encryptBinary(secretBytes, chunk);
      
      final frame = BytesBuilder()
        ..add(nonce)
        ..add(byteData.buffer.asUint8List())
        ..add(metadataBytes)
        ..add(encryptedChunkWithMac);
        
      _sendBinaryMessage?.call(frame.toBytes());

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
    _updateTransferInDb(transferId, {'status': 'complete'});
    notifyListeners();
  }

  void handleFileAccepted(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.status = TransferStatus.transferring;
    _updateTransferInDb(id, {'status': 'transferring'});
    notifyListeners();
  }

  void handleFileCancelled(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.status = TransferStatus.cancelled;
    _updateTransferInDb(id, {'status': 'cancelled'});
    notifyListeners();
  }

  void handleFileProgress(String id, int chunksReceived) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.chunksReceived = chunksReceived;
    _updateTransferInDb(id, {'chunks_received': chunksReceived});
    notifyListeners();
  }

  void handleFileRequest(Map<String, dynamic> msg) {
    final id = msg['id'] as String;
    final name = msg['name'] as String;
    final size = msg['size'] as int;
    final mime = msg['mime'] as String? ?? 'application/octet-stream';
    final from = msg['from'] as String;
    final checksum = msg['checksum'] as String?;
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
      expectedChecksum: checksum,
    );
    _transfers.insert(0, transfer);
    _pruneOldTransfers();
    _persistTransfer(transfer);
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

  Future<void> receiveChunk(String id, int index, List<int> data) async {
    final transfer = _findTransfer(id);
    if (transfer == null) return;

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
    IOSink? sink;
    try {
      final tempDir = await getTemporaryDirectory();
      final outputDir = await getDownloadsDirectory();

      // Sanitize filename to prevent path traversal
      final sanitized = p.basename(transfer.name);
      if (sanitized.contains('..') || sanitized.isEmpty) {
        throw Exception('Invalid filename: ${transfer.name}');
      }
      final downloadDir = outputDir?.path ?? tempDir.path;
      final outputPath = p.join(downloadDir, sanitized);

      sink = File(outputPath).openWrite();
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
      sink = null;

      // Verify file integrity via SHA-256 if a checksum was provided
      if (transfer.expectedChecksum != null) {
        final fileBytes = await File(outputPath).readAsBytes();
        final actualChecksum = await _computeSha256(fileBytes);
        if (actualChecksum != transfer.expectedChecksum) {
          debugPrint('[FileService] File integrity check FAILED for ${transfer.name}: expected ${transfer.expectedChecksum}, got $actualChecksum');
          transfer.status = TransferStatus.failed;
          _updateTransferInDb(transfer.id, {'status': 'failed'});
          notifyListeners();
          _handleError('File integrity check failed: checksum mismatch for ${transfer.name}');
          // Clean up the corrupted file
          await File(outputPath).delete();
          return;
        }
        debugPrint('File integrity verified for ${transfer.name}: ${actualChecksum.substring(0, 16)}');
      }

      transfer.status = TransferStatus.complete;
      transfer.savedPath = outputPath;
      _updateTransferInDb(transfer.id, {
        'status': 'complete',
        'saved_path': outputPath,
      });
      notifyListeners();
      debugPrint('File saved: $outputPath');
    } catch (e) {
      transfer.status = TransferStatus.failed;
      _updateTransferInDb(transfer.id, {'status': 'failed'});
      notifyListeners();
      _handleError('Failed to save file: $e');
    } finally {
      if (sink != null) {
        try { await sink.close(); } catch (_) {}
      }
    }
  }

  void cancelTransfer(String id) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.status = TransferStatus.cancelled;
    _updateTransferInDb(id, {'status': 'cancelled'});
    notifyListeners();
    _cleanupChunks(id);
    _sendMessage?.call({'type': 'file', 'action': 'cancel', 'id': id});
  }

  /// Clean up temp chunk files for a transfer to prevent disk/memory leaks.
  Future<void> _cleanupChunks(String transferId) async {
    try {
      final tempDir = await getTemporaryDirectory();
      for (var i = 0; i < 10000; i++) {
        final chunkFile = File('${tempDir.path}/conduit_${transferId}_chunk_$i');
        if (!await chunkFile.exists()) break;
        await chunkFile.delete();
      }
    } catch (e) {
      debugPrint('[FileService] Chunk cleanup error: $e');
    }
  }

  /// Remove old completed/cancelled/failed transfers to prevent unbounded memory growth.
  /// Keeps the last 50 transfers.
  void _pruneOldTransfers() {
    const maxKept = 50;
    if (_transfers.length <= maxKept) return;
    _transfers.removeWhere((t) =>
        t.status == TransferStatus.complete ||
        t.status == TransferStatus.cancelled ||
        t.status == TransferStatus.failed);
    // If still over limit after pruning terminal states, trim oldest
    if (_transfers.length > maxKept) {
      _transfers.removeRange(maxKept, _transfers.length);
    }
  }

  void handleResumeAck(String id, int chunksLoaded) {
    final transfer = _findTransfer(id);
    if (transfer == null) return;
    transfer.chunksReceived = chunksLoaded;
    transfer.status = TransferStatus.transferring;
    _updateTransferInDb(id, {
      'chunks_received': chunksLoaded,
      'status': 'transferring',
    });
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
      if (transfer.expectedChecksum != null) 'checksum': transfer.expectedChecksum,
    });
    transfer.status = TransferStatus.transferring;
    _updateTransferInDb(id, {'status': 'transferring'});
    notifyListeners();
  }

  /// Clear all received file cache: delete on-disk files, purge DB records, and reset state.
  Future<void> clearCache() async {
    try {
      // Delete on-disk files for completed transfers
      for (final transfer in _transfers) {
        if (transfer.savedPath != null) {
          final file = File(transfer.savedPath!);
          if (await file.exists()) {
            await file.delete();
          }
        }
        // Also clean up any leftover chunk files
        await _cleanupChunks(transfer.id);
      }

      // Purge all file transfer records from the database
      if (_db != null) {
        await _db!.deleteAllFileTransfers();
      }

      // Reset in-memory state
      _transfers.clear();
      notifyListeners();

      debugPrint('[FileService] Cache cleared');
    } catch (e) {
      debugPrint('[FileService] Cache clear error: $e');
    }
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
