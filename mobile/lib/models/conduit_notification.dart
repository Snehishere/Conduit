class ConduitNotification {
  final String id;
  final String deviceId;
  final String app;
  final String title;
  final String body;
  final int timestamp;
  final List<String>? actions;

  ConduitNotification({
    required this.id,
    required this.deviceId,
    required this.app,
    required this.title,
    required this.body,
    required this.timestamp,
    this.actions,
  });

  factory ConduitNotification.fromJson(Map<String, dynamic> json) {
    return ConduitNotification(
      id: (json['id'] as String?) ?? '',
      deviceId: (json['device_id'] as String?) ?? '',
      app: (json['app'] as String?) ?? 'unknown',
      title: (json['title'] as String?) ?? '',
      body: (json['body'] as String?) ?? '',
      timestamp: (json['timestamp'] as num?)?.toInt() ?? 0,
      actions: (json['actions'] as List?)?.cast<String>(),
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
