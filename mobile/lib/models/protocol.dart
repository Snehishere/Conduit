class ProtocolMessage {
  final String type;
  final String? action;
  final Map<String, dynamic> data;

  ProtocolMessage({
    required this.type,
    this.action,
    this.data = const {},
  });

  factory ProtocolMessage.fromJson(Map<String, dynamic> json) {
    return ProtocolMessage(
      type: (json['type'] as String?) ?? 'unknown',
      action: json['action'] as String?,
      data: Map.from(json)..remove('type')..remove('action'),
    );
  }

  Map<String, dynamic> toJson() {
    final Map<String, dynamic> json = {'type': type};
    if (action != null) json['action'] = action!;
    json.addAll(data);
    return json;
  }

  // Discovery messages
  static ProtocolMessage discoveryAnnounce({
    required String deviceName,
    required String deviceType,
    required String deviceId,
  }) {
    return ProtocolMessage(
      type: 'discovery',
      action: 'announce',
      data: {
        'device_name': deviceName,
        'device_type': deviceType,
        'device_id': deviceId,
      },
    );
  }

  // Pairing messages
  static ProtocolMessage pairingRequest({
    required String publicKey,
    required String token,
    required Map<String, dynamic> deviceInfo,
  }) {
    return ProtocolMessage(
      type: 'pairing',
      action: 'request',
      data: {
        'public_key': publicKey,
        'token': token,
        'device_info': deviceInfo,
      },
    );
  }

  // Notification messages
  static ProtocolMessage notificationDismiss({required String id}) {
    return ProtocolMessage(
      type: 'notification',
      action: 'dismiss',
      data: {'id': id},
    );
  }

  static ProtocolMessage notificationReply({
    required String id,
    required String text,
  }) {
    return ProtocolMessage(
      type: 'notification',
      action: 'reply',
      data: {'id': id, 'text': text},
    );
  }

  // Clipboard messages
  static ProtocolMessage clipboardSync({
    required String content,
    required String mime,
    required String sourceDevice,
  }) {
    return ProtocolMessage(
      type: 'clipboard',
      action: 'sync',
      data: {
        'content': content,
        'mime': mime,
        'source_device': sourceDevice,
        'timestamp': DateTime.now().millisecondsSinceEpoch ~/ 1000,
      },
    );
  }

  // Screen mirroring messages
  static ProtocolMessage screenMirrorStart({
    required String deviceId,
    required String quality,
    required int fps,
  }) {
    return ProtocolMessage(
      type: 'screen_mirror',
      action: 'start',
      data: {
        'device_id': deviceId,
        'quality': quality,
        'fps': fps,
      },
    );
  }

  static ProtocolMessage screenMirrorStop({required String deviceId}) {
    return ProtocolMessage(
      type: 'screen_mirror',
      action: 'stop',
      data: {'device_id': deviceId},
    );
  }

  static ProtocolMessage screenMirrorFrame({
    required String deviceId,
    required String data,
    required int width,
    required int height,
  }) {
    return ProtocolMessage(
      type: 'screen_mirror',
      action: 'frame',
      data: {
        'device_id': deviceId,
        'data': data,
        'width': width,
        'height': height,
        'timestamp': DateTime.now().millisecondsSinceEpoch ~/ 1000,
      },
    );
  }

  static ProtocolMessage screenMirrorTouch({
    required String deviceId,
    required double x,
    required double y,
    required String actionType,
    String? direction,
  }) {
    return ProtocolMessage(
      type: 'screen_mirror',
      action: 'touch',
      data: {
        'device_id': deviceId,
        'x': x,
        'y': y,
        'action_type': actionType,
        if (direction != null) 'direction': direction,
      },
    );
  }

  static ProtocolMessage screenMirrorKey({
    required String deviceId,
    required String key,
    List<String>? modifiers,
  }) {
    return ProtocolMessage(
      type: 'screen_mirror',
      action: 'key',
      data: {
        'device_id': deviceId,
        'key': key,
        if (modifiers != null) 'modifiers': modifiers,
      },
    );
  }
}
