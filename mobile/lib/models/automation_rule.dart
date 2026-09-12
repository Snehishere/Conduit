enum TriggerType {
  deviceConnect,
  deviceDisconnect,
  time,
  batteryLevel,
  wifiChange,
  appOpen,
  audioDeviceConnect,
  audioDeviceDisconnect,
}

enum ActionType {
  sendNotification,
  setPhoneProfile,
  routeAudio,
  runShellCommand,
  toggleWifi,
  toggleBluetooth,
  openUrl,
  openApp,
  setWindowState,
}

enum PhoneProfile { silent, vibrate, ring }

enum WindowState { minimize, maximize, restore, close }

extension TriggerTypeExt on TriggerType {
  String get label {
    switch (this) {
      case TriggerType.deviceConnect:
        return 'Device Connected';
      case TriggerType.deviceDisconnect:
        return 'Device Disconnected';
      case TriggerType.time:
        return 'Time-Based';
      case TriggerType.batteryLevel:
        return 'Battery Level';
      case TriggerType.wifiChange:
        return 'WiFi Change';
      case TriggerType.appOpen:
        return 'App Opened';
      case TriggerType.audioDeviceConnect:
        return 'Audio Connected';
      case TriggerType.audioDeviceDisconnect:
        return 'Audio Disconnected';
    }
  }

  String get icon {
    switch (this) {
      case TriggerType.deviceConnect:
        return '🔗';
      case TriggerType.deviceDisconnect:
        return '🔌';
      case TriggerType.time:
        return '⏰';
      case TriggerType.batteryLevel:
        return '🪫';
      case TriggerType.wifiChange:
        return '📶';
      case TriggerType.appOpen:
        return '📱';
      case TriggerType.audioDeviceConnect:
        return '🎧';
      case TriggerType.audioDeviceDisconnect:
        return '🔇';
    }
  }

  /// Wire name: snake_case tag sent on the protocol.
  String get wireType => switch (this) {
        TriggerType.deviceConnect => 'device_connect',
        TriggerType.deviceDisconnect => 'device_disconnect',
        TriggerType.time => 'time',
        TriggerType.batteryLevel => 'battery_level',
        TriggerType.wifiChange => 'wifi_change',
        TriggerType.appOpen => 'app_open',
        TriggerType.audioDeviceConnect => 'audio_device_connect',
        TriggerType.audioDeviceDisconnect => 'audio_device_disconnect',
      };
}

extension ActionTypeExt on ActionType {
  String get label {
    switch (this) {
      case ActionType.sendNotification:
        return 'Send Notification';
      case ActionType.setPhoneProfile:
        return 'Set Phone Profile';
      case ActionType.routeAudio:
        return 'Route Audio';
      case ActionType.runShellCommand:
        return 'Run Command';
      case ActionType.toggleWifi:
        return 'Toggle WiFi';
      case ActionType.toggleBluetooth:
        return 'Toggle Bluetooth';
      case ActionType.openUrl:
        return 'Open URL';
      case ActionType.openApp:
        return 'Open App';
      case ActionType.setWindowState:
        return 'Set Window State';
    }
  }

  String get icon {
    switch (this) {
      case ActionType.sendNotification:
        return '🔔';
      case ActionType.setPhoneProfile:
        return '📳';
      case ActionType.routeAudio:
        return '🔊';
      case ActionType.runShellCommand:
        return '💻';
      case ActionType.toggleWifi:
        return '📶';
      case ActionType.toggleBluetooth:
        return '🔷';
      case ActionType.openUrl:
        return '🌐';
      case ActionType.openApp:
        return '📲';
      case ActionType.setWindowState:
        return '🪟';
    }
  }

  /// Wire name: snake_case tag sent on the protocol.
  String get wireType => switch (this) {
        ActionType.sendNotification => 'send_notification',
        ActionType.setPhoneProfile => 'set_phone_profile',
        ActionType.routeAudio => 'route_audio',
        ActionType.runShellCommand => 'run_shell_command',
        ActionType.toggleWifi => 'toggle_wifi',
        ActionType.toggleBluetooth => 'toggle_bluetooth',
        ActionType.openUrl => 'open_url',
        ActionType.openApp => 'open_app',
        ActionType.setWindowState => 'set_window_state',
      };
}

// ---------------------------------------------------------------------------
// Trigger — flat, internally tagged
// ---------------------------------------------------------------------------

class Trigger {
  final TriggerType type;
  final String? deviceId;
  final String? time; // "HH:MM" 24h string
  final int? below; // 0..100 for battery_level
  final String? ssid; // for wifi_change
  final String? appPackage; // for app_open

  const Trigger({
    required this.type,
    this.deviceId,
    this.time,
    this.below,
    this.ssid,
    this.appPackage,
  });

  /// Parse from wire JSON. Throws [FormatException] on unknown trigger type.
  factory Trigger.fromJson(Map<String, dynamic> json) {
    final tag = json['type'] as String;
    final type = _parseTriggerType(tag);
    return Trigger(
      type: type,
      deviceId: json['device_id'] as String?,
      time: json['time'] as String?,
      below: (json['below'] as num?)?.toInt(),
      ssid: json['ssid'] as String?,
      appPackage: json['app_package'] as String?,
    );
  }

  /// Serialize to flat wire JSON.
  Map<String, dynamic> toJson() {
    final map = <String, dynamic>{'type': type.wireType};
    if (deviceId != null) map['device_id'] = deviceId;
    if (time != null) map['time'] = time;
    if (below != null) map['below'] = below;
    if (ssid != null) map['ssid'] = ssid;
    if (appPackage != null) map['app_package'] = appPackage;
    return map;
  }

  static TriggerType _parseTriggerType(String tag) {
    return TriggerType.values.firstWhere(
      (e) => e.wireType == tag,
      orElse: () => throw FormatException('Unknown trigger type: $tag'),
    );
  }
}

// ---------------------------------------------------------------------------
// AutomationAction — flat, internally tagged
// ---------------------------------------------------------------------------

class AutomationAction {
  final ActionType type;
  final String? title; // send_notification
  final String? body; // send_notification
  final String? profile; // set_phone_profile: silent|vibrate|ring
  final String? deviceId; // route_audio
  final String? command; // run_shell_command
  final bool? enabled; // toggle_wifi, toggle_bluetooth
  final String? url; // open_url
  final String? appPackage; // open_app
  final String? state; // set_window_state: minimize|maximize|restore|close

  const AutomationAction({
    required this.type,
    this.title,
    this.body,
    this.profile,
    this.deviceId,
    this.command,
    this.enabled,
    this.url,
    this.appPackage,
    this.state,
  });

  /// Parse from wire JSON. Throws [FormatException] on unknown action type.
  factory AutomationAction.fromJson(Map<String, dynamic> json) {
    final tag = json['type'] as String;
    final type = _parseActionType(tag);
    return AutomationAction(
      type: type,
      title: json['title'] as String?,
      body: json['body'] as String?,
      profile: json['profile'] as String?,
      deviceId: json['device_id'] as String?,
      command: json['command'] as String?,
      enabled: json['enabled'] as bool?,
      url: json['url'] as String?,
      appPackage: json['app_package'] as String?,
      state: json['state'] as String?,
    );
  }

  /// Serialize to flat wire JSON.
  Map<String, dynamic> toJson() {
    final map = <String, dynamic>{'type': type.wireType};
    if (title != null) map['title'] = title;
    if (body != null) map['body'] = body;
    if (profile != null) map['profile'] = profile;
    if (deviceId != null) map['device_id'] = deviceId;
    if (command != null) map['command'] = command;
    if (enabled != null) map['enabled'] = enabled;
    if (url != null) map['url'] = url;
    if (appPackage != null) map['app_package'] = appPackage;
    if (state != null) map['state'] = state;
    return map;
  }

  static ActionType _parseActionType(String tag) {
    return ActionType.values.firstWhere(
      (e) => e.wireType == tag,
      orElse: () => throw FormatException('Unknown action type: $tag'),
    );
  }
}

// ---------------------------------------------------------------------------
// AutomationRule — top-level wire packet
// ---------------------------------------------------------------------------

class AutomationRule {
  final String id;
  final String name;
  final Trigger trigger;
  final AutomationAction action;
  final bool enabled;

  const AutomationRule({
    required this.id,
    required this.name,
    required this.trigger,
    required this.action,
    this.enabled = true,
  });

  AutomationRule copyWith({
    String? id,
    String? name,
    Trigger? trigger,
    AutomationAction? action,
    bool? enabled,
  }) {
    return AutomationRule(
      id: id ?? this.id,
      name: name ?? this.name,
      trigger: trigger ?? this.trigger,
      action: action ?? this.action,
      enabled: enabled ?? this.enabled,
    );
  }

  /// Parse from wire JSON (§4a). Throws [FormatException] on bad data.
  /// Accepts both `action` (wire) and `action_config` (local storage) keys.
  factory AutomationRule.fromJson(Map<String, dynamic> json) {
    final id = json['id'] as String?;
    if (id == null || id.isEmpty) {
      throw FormatException('Rule "id" must be a non-empty string');
    }
    final name = json['name'] as String?;
    if (name == null || name.isEmpty) {
      throw FormatException('Rule "name" must be a non-empty string');
    }
    final trigger =
        Trigger.fromJson(json['trigger'] as Map<String, dynamic>);
    // Wire uses 'action' for the payload; storage may use 'rule_action' or 'action_config'.
    final actionJson = (json['rule_action'] ?? json['action_config'] ?? json['action'])
        as Map<String, dynamic>;
    return AutomationRule(
      id: id,
      name: name,
      trigger: trigger,
      action: AutomationAction.fromJson(actionJson),
      enabled: json['enabled'] as bool? ?? true,
    );
  }

  /// Serialize to wire JSON (§4a). No created_at/last_triggered on wire.
  /// `rule_action` avoids collision with the top-level `action: 'rule'` tag.
  Map<String, dynamic> toJson() {
    return {
      'type': 'automation',
      'action': 'rule',
      'id': id,
      'name': name,
      'trigger': trigger.toJson(),
      'rule_action': action.toJson(),
      'enabled': enabled,
    };
  }
}
