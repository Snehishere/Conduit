/// Model representing a file transfer record for SQLite persistence.
///
/// Maps to the `file_transfers` table in `conduit_mobile.db`.
/// Used by [FileService] to persist transfer history across app restarts.
class PersistedTransfer {
  final String id;
  final String name;
  final int size;
  final String mime;
  final String fromDevice;
  final String toDevice;
  final String status;
  final int chunksReceived;
  final int totalChunks;
  final String? savedPath;
  final String? expectedChecksum;
  final int timestamp;
  final int createdAt;

  PersistedTransfer({
    required this.id,
    required this.name,
    required this.size,
    required this.mime,
    required this.fromDevice,
    required this.toDevice,
    required this.status,
    this.chunksReceived = 0,
    required this.totalChunks,
    this.savedPath,
    this.expectedChecksum,
    required this.timestamp,
    required this.createdAt,
  });

  /// Create from an SQLite row (maps to file_transfers table columns).
  factory PersistedTransfer.fromMap(Map<String, dynamic> map) {
    return PersistedTransfer(
      id: (map['id'] as String?) ?? '',
      name: (map['name'] as String?) ?? '',
      size: (map['size'] as num?)?.toInt() ?? 0,
      mime: (map['mime'] as String?) ?? 'application/octet-stream',
      fromDevice: (map['from_device'] as String?) ?? '',
      toDevice: (map['to_device'] as String?) ?? '',
      status: (map['status'] as String?) ?? 'pending',
      chunksReceived: (map['chunks_received'] as num?)?.toInt() ?? 0,
      totalChunks: (map['total_chunks'] as num?)?.toInt() ?? 0,
      savedPath: map['saved_path'] as String?,
      expectedChecksum: map['expected_checksum'] as String?,
      timestamp: (map['timestamp'] as num?)?.toInt() ?? 0,
      createdAt: (map['created_at'] as num?)?.toInt() ?? 0,
    );
  }

  /// Convert to an SQLite row for the file_transfers table.
  Map<String, dynamic> toMap() {
    return {
      'id': id,
      'name': name,
      'size': size,
      'mime': mime,
      'from_device': fromDevice,
      'to_device': toDevice,
      'status': status,
      'chunks_received': chunksReceived,
      'total_chunks': totalChunks,
      'saved_path': savedPath,
      'expected_checksum': expectedChecksum,
      'timestamp': timestamp,
      'created_at': createdAt,
    };
  }
}
