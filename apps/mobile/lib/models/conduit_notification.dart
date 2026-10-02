class ConduitNotification {
  final String id;
  final String deviceId;
  final String app;
  final String title;
  final String body;
  final int timestamp;
  final List<String>? actions;

  /// Whether the user has read this. Distinct from "dismissed": dismissing
  /// deletes the row, reading must not.
  final bool read;

  ConduitNotification({
    required this.id,
    required this.deviceId,
    required this.app,
    required this.title,
    required this.body,
    required this.timestamp,
    this.actions,
    this.read = false,
  });

  ConduitNotification copyWith({
    String? id,
    String? deviceId,
    String? app,
    String? title,
    String? body,
    int? timestamp,
    List<String>? actions,
    bool? read,
  }) {
    return ConduitNotification(
      id: id ?? this.id,
      deviceId: deviceId ?? this.deviceId,
      app: app ?? this.app,
      title: title ?? this.title,
      body: body ?? this.body,
      timestamp: timestamp ?? this.timestamp,
      actions: actions ?? this.actions,
      read: read ?? this.read,
    );
  }

  /// Create from an SQLite row (maps to notification_history table columns).
  factory ConduitNotification.fromMap(Map<String, dynamic> map) {
    return ConduitNotification(
      id: (map['id'] as String?) ?? '',
      deviceId: (map['device_id'] as String?) ?? '',
      app: (map['app'] as String?) ?? 'unknown',
      title: (map['title'] as String?) ?? '',
      body: (map['body'] as String?) ?? '',
      timestamp: (map['timestamp'] as num?)?.toInt() ?? 0,
      actions: (map['actions'] as String?)?.split(',').where((s) => s.isNotEmpty).toList(),
      read: ((map['read'] as num?)?.toInt() ?? 0) == 1,
    );
  }

  /// Convert to an SQLite row for the notification_history table.
  Map<String, dynamic> toMap() {
    return {
      'id': id,
      'device_id': deviceId,
      'app': app,
      'title': title,
      'body': body,
      'timestamp': timestamp,
      'actions': actions?.join(','),
      'read': read ? 1 : 0,
      'created_at': DateTime.now().millisecondsSinceEpoch,
    };
  }

  factory ConduitNotification.fromJson(Map<String, dynamic> json) {
    return ConduitNotification(
      id: (json['id'] as String?) ?? '',
      deviceId: (json['device_id'] as String?) ?? '',
      app: (json['app'] as String?) ?? 'unknown',
      title: (json['title'] as String?) ?? '',
      body: (json['body'] as String?) ?? '',
      timestamp: (json['timestamp'] as num?)?.toInt() ?? 0,
      actions: (json['actions'] as List?)?.cast<String>(),
      read: (json['read'] as bool?) ?? ((json['read'] as num?)?.toInt() == 1),
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'device_id': deviceId,
      'app': app,
      'title': title,
      'body': body,
      'timestamp': timestamp,
      'actions': actions,
      'read': read,
    };
  }

  String get timeAgo {
    final diff = DateTime.now().millisecondsSinceEpoch - timestamp * 1000;
    if (diff <= 0) return 'just now';
    final minutes = diff ~/ 60000;
    if (minutes < 1) return 'just now';
    if (minutes < 60) return '${minutes}m ago';
    final hours = minutes ~/ 60;
    if (hours < 24) return '${hours}h ago';
    return '${hours ~/ 24}d ago';
  }
}
