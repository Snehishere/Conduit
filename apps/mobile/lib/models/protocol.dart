// To parse this JSON data, do
//
//     final protocol = protocolFromJson(jsonString);

import 'dart:convert';

Protocol protocolFromJson(String str) => Protocol.fromJson(json.decode(str));

String protocolToJson(Protocol data) => json.encode(data.toJson());


///JSON message schema for the Conduit ecosystem (desktop, mobile, relay).
///
///A newly received SMS. Both sender ({from, body, timestamp}) and receiver ({thread_id,
///message}) shapes are accepted; see PROTOCOL.md on the unresolved divergence.
///
///Confirmation of a successfully sent SMS. Both sender ({to, body, timestamp}) and receiver
///({thread_id, message}) shapes are accepted; see PROTOCOL.md on the unresolved divergence.
class Protocol {
    final String? action;
    final String? apnsToken;
    final int? battery;
    final String? deviceId;
    final String? deviceName;
    final DeviceTypeEnum? deviceType;
    final String? os;
    final int? protocolVersion;
    final PurpleType type;
    final String? version;
    final int? wsPort;
    final int? wssPort;
    final DeviceInfo? deviceInfo;
    
    ///X25519 public key, hex-encoded
    final String? publicKey;
    final String? token;
    final String? reason;
    final String? content;
    final String? mime;
    
    ///Sender device ID
    final String? sourceDevice;
    
    ///Unix timestamp (seconds)
    final int? timestamp;
    final List<String>? actions;
    final String? app;
    final String? body;
    
    ///Alias for rule_id
    final String? id;
    final String? title;
    final String? text;
    final String? checksum;
    final String? from;
    final String? name;
    final int? size;
    final String? to;
    
    ///Base64-encoded chunk data
    ///
    ///Base64-encoded PCM-16 LE samples
    ///
    ///Base64-encoded JPEG image
    ///
    ///Hex-encoded ciphertext
    final String? data;
    final int? index;
    final int? total;
    final int? percent;
    final String? path;
    final int? chunksLoaded;
    final int? channels;
    final Format? format;
    final int? sampleRate;
    final Quality? quality;
    final bool? fromDesktop;
    final int? height;
    final int? width;
    final ActionType? actionType;
    final double? x;
    final double? y;
    final String? key;
    final List<Modifier>? modifiers;
    final double? dx;
    final double? dy;
    final Button? button;
    final bool? enabled;
    final AutomationActionPayload? ruleAction;
    final AutomationTrigger? trigger;
    final String? ruleId;
    final bool? fullSync;
    final List<Map<String, dynamic>>? rules;
    final String? triggerType;
    final String? callId;
    final String? number;
    final String? route;
    final String? toDeviceId;
    final String? wifiSsid;
    final String? code;
    final dynamic message;
    final int? serverVersion;
    final String? relayToken;
    
    ///Complete JSON message to forward
    final Map<String, dynamic>? payload;
    
    ///Hex-encoded HMAC-SHA256
    final String? hmac;
    
    ///Hex-encoded XChaCha20-Poly1305 nonce
    final String? nonce;
    final List<SmsThread>? threads;
    final String? threadId;

    Protocol({
        this.action,
        this.apnsToken,
        this.battery,
        this.deviceId,
        this.deviceName,
        this.deviceType,
        this.os,
        this.protocolVersion,
        required this.type,
        this.version,
        this.wsPort,
        this.wssPort,
        this.deviceInfo,
        this.publicKey,
        this.token,
        this.reason,
        this.content,
        this.mime,
        this.sourceDevice,
        this.timestamp,
        this.actions,
        this.app,
        this.body,
        this.id,
        this.title,
        this.text,
        this.checksum,
        this.from,
        this.name,
        this.size,
        this.to,
        this.data,
        this.index,
        this.total,
        this.percent,
        this.path,
        this.chunksLoaded,
        this.channels,
        this.format,
        this.sampleRate,
        this.quality,
        this.fromDesktop,
        this.height,
        this.width,
        this.actionType,
        this.x,
        this.y,
        this.key,
        this.modifiers,
        this.dx,
        this.dy,
        this.button,
        this.enabled,
        this.ruleAction,
        this.trigger,
        this.ruleId,
        this.fullSync,
        this.rules,
        this.triggerType,
        this.callId,
        this.number,
        this.route,
        this.toDeviceId,
        this.wifiSsid,
        this.code,
        this.message,
        this.serverVersion,
        this.relayToken,
        this.payload,
        this.hmac,
        this.nonce,
        this.threads,
        this.threadId,
    });

    factory Protocol.fromJson(Map<String, dynamic> json) => Protocol(
        action: json["action"],
        apnsToken: json["apns_token"],
        battery: json["battery"],
        deviceId: json["device_id"],
        deviceName: json["device_name"],
        deviceType: deviceTypeEnumValues.map[json["device_type"]],
        os: json["os"],
        protocolVersion: json["protocol_version"],
        type: purpleTypeValues.map[json["type"]]!,
        version: json["version"],
        wsPort: json["ws_port"],
        wssPort: json["wss_port"],
        deviceInfo: json["device_info"] == null ? null : DeviceInfo.fromJson(json["device_info"]),
        publicKey: json["public_key"],
        token: json["token"],
        reason: json["reason"],
        content: json["content"],
        mime: json["mime"],
        sourceDevice: json["source_device"],
        timestamp: json["timestamp"],
        actions: json["actions"] == null ? [] : List<String>.from(json["actions"]!.map((x) => x)),
        app: json["app"],
        body: json["body"],
        id: json["id"],
        title: json["title"],
        text: json["text"],
        checksum: json["checksum"],
        from: json["from"],
        name: json["name"],
        size: json["size"],
        to: json["to"],
        data: json["data"],
        index: json["index"],
        total: json["total"],
        percent: json["percent"],
        path: json["path"],
        chunksLoaded: json["chunks_loaded"],
        channels: json["channels"],
        format: formatValues.map[json["format"]],
        sampleRate: json["sample_rate"],
        quality: qualityValues.map[json["quality"]],
        fromDesktop: json["from_desktop"],
        height: json["height"],
        width: json["width"],
        actionType: actionTypeValues.map[json["actionType"]],
        x: json["x"]?.toDouble(),
        y: json["y"]?.toDouble(),
        key: json["key"],
        modifiers: json["modifiers"] == null ? [] : List<Modifier>.from(json["modifiers"]!.map((x) => modifierValues.map[x]!)),
        dx: json["dx"]?.toDouble(),
        dy: json["dy"]?.toDouble(),
        button: buttonValues.map[json["button"]],
        enabled: json["enabled"],
        ruleAction: json["rule_action"] == null ? null : AutomationActionPayload.fromJson(json["rule_action"]),
        trigger: json["trigger"] == null ? null : AutomationTrigger.fromJson(json["trigger"]),
        ruleId: json["rule_id"],
        fullSync: json["full_sync"],
        rules: json["rules"] == null ? [] : List<Map<String, dynamic>>.from(json["rules"]!.map((x) => Map.from(x).map((k, v) => MapEntry<String, dynamic>(k, v)))),
        triggerType: json["trigger_type"],
        callId: json["call_id"],
        number: json["number"],
        route: json["route"],
        toDeviceId: json["to_device_id"],
        wifiSsid: json["wifi_ssid"],
        code: json["code"],
        message: json["message"],
        serverVersion: json["server_version"],
        relayToken: json["relay_token"],
        payload: Map.from(json["payload"]!).map((k, v) => MapEntry<String, dynamic>(k, v)),
        hmac: json["hmac"],
        nonce: json["nonce"],
        threads: json["threads"] == null ? [] : List<SmsThread>.from(json["threads"]!.map((x) => SmsThread.fromJson(x))),
        threadId: json["thread_id"],
    );

    Map<String, dynamic> toJson() => {
        "action": action,
        "apns_token": apnsToken,
        "battery": battery,
        "device_id": deviceId,
        "device_name": deviceName,
        "device_type": deviceTypeEnumValues.reverse[deviceType],
        "os": os,
        "protocol_version": protocolVersion,
        "type": purpleTypeValues.reverse[type],
        "version": version,
        "ws_port": wsPort,
        "wss_port": wssPort,
        "device_info": deviceInfo?.toJson(),
        "public_key": publicKey,
        "token": token,
        "reason": reason,
        "content": content,
        "mime": mime,
        "source_device": sourceDevice,
        "timestamp": timestamp,
        "actions": actions == null ? [] : List<dynamic>.from(actions!.map((x) => x)),
        "app": app,
        "body": body,
        "id": id,
        "title": title,
        "text": text,
        "checksum": checksum,
        "from": from,
        "name": name,
        "size": size,
        "to": to,
        "data": data,
        "index": index,
        "total": total,
        "percent": percent,
        "path": path,
        "chunks_loaded": chunksLoaded,
        "channels": channels,
        "format": formatValues.reverse[format],
        "sample_rate": sampleRate,
        "quality": qualityValues.reverse[quality],
        "from_desktop": fromDesktop,
        "height": height,
        "width": width,
        "actionType": actionTypeValues.reverse[actionType],
        "x": x,
        "y": y,
        "key": key,
        "modifiers": modifiers == null ? [] : List<dynamic>.from(modifiers!.map((x) => modifierValues.reverse[x])),
        "dx": dx,
        "dy": dy,
        "button": buttonValues.reverse[button],
        "enabled": enabled,
        "rule_action": ruleAction?.toJson(),
        "trigger": trigger?.toJson(),
        "rule_id": ruleId,
        "full_sync": fullSync,
        "rules": rules == null ? [] : List<dynamic>.from(rules!.map((x) => Map.from(x).map((k, v) => MapEntry<String, dynamic>(k, v)))),
        "trigger_type": triggerType,
        "call_id": callId,
        "number": number,
        "route": route,
        "to_device_id": toDeviceId,
        "wifi_ssid": wifiSsid,
        "code": code,
        "message": message,
        "server_version": serverVersion,
        "relay_token": relayToken,
        "payload": Map.from(payload!).map((k, v) => MapEntry<String, dynamic>(k, v)),
        "hmac": hmac,
        "nonce": nonce,
        "threads": threads == null ? [] : List<dynamic>.from(threads!.map((x) => x.toJson())),
        "thread_id": threadId,
    };
}

enum ActionType {
    DOUBLE_TAP,
    LONG_PRESS,
    RIGHT_CLICK,
    TAP
}

final actionTypeValues = EnumValues({
    "double_tap": ActionType.DOUBLE_TAP,
    "long_press": ActionType.LONG_PRESS,
    "right_click": ActionType.RIGHT_CLICK,
    "tap": ActionType.TAP
});

enum Button {
    DOUBLE_LEFT,
    LEFT,
    MIDDLE,
    RIGHT
}

final buttonValues = EnumValues({
    "double_left": Button.DOUBLE_LEFT,
    "left": Button.LEFT,
    "middle": Button.MIDDLE,
    "right": Button.RIGHT
});


///Device metadata exchanged during pairing and discovery.
class DeviceInfo {
    final int? battery;
    final String name;
    final Os? os;
    final DeviceTypeEnum type;

    DeviceInfo({
        this.battery,
        required this.name,
        this.os,
        required this.type,
    });

    factory DeviceInfo.fromJson(Map<String, dynamic> json) => DeviceInfo(
        battery: json["battery"],
        name: json["name"],
        os: osValues.map[json["os"]],
        type: deviceTypeEnumValues.map[json["type"]]!,
    );

    Map<String, dynamic> toJson() => {
        "battery": battery,
        "name": name,
        "os": osValues.reverse[os],
        "type": deviceTypeEnumValues.reverse[type],
    };
}

enum Os {
    ANDROID,
    IOS,
    LINUX,
    MACOS,
    UNKNOWN,
    WINDOWS
}

final osValues = EnumValues({
    "android": Os.ANDROID,
    "ios": Os.IOS,
    "linux": Os.LINUX,
    "macos": Os.MACOS,
    "unknown": Os.UNKNOWN,
    "windows": Os.WINDOWS
});

enum DeviceTypeEnum {
    DESKTOP,
    PHONE,
    TABLET
}

final deviceTypeEnumValues = EnumValues({
    "desktop": DeviceTypeEnum.DESKTOP,
    "phone": DeviceTypeEnum.PHONE,
    "tablet": DeviceTypeEnum.TABLET
});

enum Format {
    JPEG,
    PCM16
}

final formatValues = EnumValues({
    "jpeg": Format.JPEG,
    "pcm16": Format.PCM16
});


///A single SMS record as produced by the phone.
class SmsMessage {
    final String address;
    final String body;
    
    ///Stable record ID; prefixed in_ for received, out_ for sent.
    final String id;
    final bool isOutgoing;
    final bool read;
    
    ///Unix timestamp (seconds)
    final int timestamp;

    SmsMessage({
        required this.address,
        required this.body,
        required this.id,
        required this.isOutgoing,
        required this.read,
        required this.timestamp,
    });

    factory SmsMessage.fromJson(Map<String, dynamic> json) => SmsMessage(
        address: json["address"],
        body: json["body"],
        id: json["id"],
        isOutgoing: json["is_outgoing"],
        read: json["read"],
        timestamp: json["timestamp"],
    );

    Map<String, dynamic> toJson() => {
        "address": address,
        "body": body,
        "id": id,
        "is_outgoing": isOutgoing,
        "read": read,
        "timestamp": timestamp,
    };
}

enum Modifier {
    ALT,
    CONTROL,
    META,
    SHIFT
}

final modifierValues = EnumValues({
    "alt": Modifier.ALT,
    "control": Modifier.CONTROL,
    "meta": Modifier.META,
    "shift": Modifier.SHIFT
});

enum Quality {
    HIGH,
    LOW,
    MEDIUM
}

final qualityValues = EnumValues({
    "high": Quality.HIGH,
    "low": Quality.LOW,
    "medium": Quality.MEDIUM
});

class AutomationActionPayload {
    final String? appPackage;
    final String? body;
    final String? command;
    final String? deviceId;
    final bool? enabled;
    final Profile? profile;
    final State? state;
    final String? title;
    final RuleActionType type;
    final String? url;

    AutomationActionPayload({
        this.appPackage,
        this.body,
        this.command,
        this.deviceId,
        this.enabled,
        this.profile,
        this.state,
        this.title,
        required this.type,
        this.url,
    });

    factory AutomationActionPayload.fromJson(Map<String, dynamic> json) => AutomationActionPayload(
        appPackage: json["app_package"],
        body: json["body"],
        command: json["command"],
        deviceId: json["device_id"],
        enabled: json["enabled"],
        profile: profileValues.map[json["profile"]],
        state: stateValues.map[json["state"]],
        title: json["title"],
        type: ruleActionTypeValues.map[json["type"]]!,
        url: json["url"],
    );

    Map<String, dynamic> toJson() => {
        "app_package": appPackage,
        "body": body,
        "command": command,
        "device_id": deviceId,
        "enabled": enabled,
        "profile": profileValues.reverse[profile],
        "state": stateValues.reverse[state],
        "title": title,
        "type": ruleActionTypeValues.reverse[type],
        "url": url,
    };
}

enum Profile {
    RING,
    SILENT,
    VIBRATE
}

final profileValues = EnumValues({
    "ring": Profile.RING,
    "silent": Profile.SILENT,
    "vibrate": Profile.VIBRATE
});

enum State {
    CLOSE,
    MAXIMIZE,
    MINIMIZE,
    RESTORE
}

final stateValues = EnumValues({
    "close": State.CLOSE,
    "maximize": State.MAXIMIZE,
    "minimize": State.MINIMIZE,
    "restore": State.RESTORE
});

enum RuleActionType {
    OPEN_APP,
    OPEN_URL,
    ROUTE_AUDIO,
    RUN_SHELL_COMMAND,
    SEND_NOTIFICATION,
    SET_PHONE_PROFILE,
    SET_WINDOW_STATE,
    TOGGLE_BLUETOOTH,
    TOGGLE_WIFI
}

final ruleActionTypeValues = EnumValues({
    "open_app": RuleActionType.OPEN_APP,
    "open_url": RuleActionType.OPEN_URL,
    "route_audio": RuleActionType.ROUTE_AUDIO,
    "run_shell_command": RuleActionType.RUN_SHELL_COMMAND,
    "send_notification": RuleActionType.SEND_NOTIFICATION,
    "set_phone_profile": RuleActionType.SET_PHONE_PROFILE,
    "set_window_state": RuleActionType.SET_WINDOW_STATE,
    "toggle_bluetooth": RuleActionType.TOGGLE_BLUETOOTH,
    "toggle_wifi": RuleActionType.TOGGLE_WIFI
});


///One conversation thread within an SMS Sync snapshot.
class SmsThread {
    final String address;
    final List<SmsMessage> messages;
    
    ///Address-book display name, null when unresolved.
    final String? name;
    final String snippet;
    
    ///Stable thread ID; prefixed t_ on the originating phone.
    final String threadId;
    
    ///Unix timestamp (seconds) of the most recent message.
    final int timestamp;
    final int unreadCount;

    SmsThread({
        required this.address,
        required this.messages,
        this.name,
        required this.snippet,
        required this.threadId,
        required this.timestamp,
        required this.unreadCount,
    });

    factory SmsThread.fromJson(Map<String, dynamic> json) => SmsThread(
        address: json["address"],
        messages: List<SmsMessage>.from(json["messages"].map((x) => SmsMessage.fromJson(x))),
        name: json["name"],
        snippet: json["snippet"],
        threadId: json["thread_id"],
        timestamp: json["timestamp"],
        unreadCount: json["unread_count"],
    );

    Map<String, dynamic> toJson() => {
        "address": address,
        "messages": List<dynamic>.from(messages.map((x) => x.toJson())),
        "name": name,
        "snippet": snippet,
        "thread_id": threadId,
        "timestamp": timestamp,
        "unread_count": unreadCount,
    };
}

class AutomationTrigger {
    final String? appPackage;
    final int? below;
    final String? deviceId;
    final String? ssid;
    
    ///HH:MM 24h format
    final String? time;
    final TriggerType type;

    AutomationTrigger({
        this.appPackage,
        this.below,
        this.deviceId,
        this.ssid,
        this.time,
        required this.type,
    });

    factory AutomationTrigger.fromJson(Map<String, dynamic> json) => AutomationTrigger(
        appPackage: json["app_package"],
        below: json["below"],
        deviceId: json["device_id"],
        ssid: json["ssid"],
        time: json["time"],
        type: triggerTypeValues.map[json["type"]]!,
    );

    Map<String, dynamic> toJson() => {
        "app_package": appPackage,
        "below": below,
        "device_id": deviceId,
        "ssid": ssid,
        "time": time,
        "type": triggerTypeValues.reverse[type],
    };
}

enum TriggerType {
    APP_OPEN,
    AUDIO_DEVICE_CONNECT,
    AUDIO_DEVICE_DISCONNECT,
    BATTERY_LEVEL,
    DEVICE_CONNECT,
    DEVICE_DISCONNECT,
    TIME,
    WIFI_CHANGE
}

final triggerTypeValues = EnumValues({
    "app_open": TriggerType.APP_OPEN,
    "audio_device_connect": TriggerType.AUDIO_DEVICE_CONNECT,
    "audio_device_disconnect": TriggerType.AUDIO_DEVICE_DISCONNECT,
    "battery_level": TriggerType.BATTERY_LEVEL,
    "device_connect": TriggerType.DEVICE_CONNECT,
    "device_disconnect": TriggerType.DEVICE_DISCONNECT,
    "time": TriggerType.TIME,
    "wifi_change": TriggerType.WIFI_CHANGE
});

enum PurpleType {
    AUDIO,
    AUTOMATION,
    CALL,
    CLIPBOARD,
    DISCOVERY,
    ENCRYPTED,
    ERROR,
    FILE,
    NOTIFICATION,
    PAIRING,
    PING,
    PONG,
    RELAY_AUTH,
    RELAY_AUTH_OK,
    RELAY_AUTH_REJECTED,
    RELAY_ROUTE,
    REMOTE_INPUT,
    SCREEN_MIRROR,
    SMS,
    STATUS
}

final purpleTypeValues = EnumValues({
    "audio": PurpleType.AUDIO,
    "automation": PurpleType.AUTOMATION,
    "call": PurpleType.CALL,
    "clipboard": PurpleType.CLIPBOARD,
    "discovery": PurpleType.DISCOVERY,
    "encrypted": PurpleType.ENCRYPTED,
    "error": PurpleType.ERROR,
    "file": PurpleType.FILE,
    "notification": PurpleType.NOTIFICATION,
    "pairing": PurpleType.PAIRING,
    "ping": PurpleType.PING,
    "pong": PurpleType.PONG,
    "relay_auth": PurpleType.RELAY_AUTH,
    "relay_auth_ok": PurpleType.RELAY_AUTH_OK,
    "relay_auth_rejected": PurpleType.RELAY_AUTH_REJECTED,
    "relay_route": PurpleType.RELAY_ROUTE,
    "remote_input": PurpleType.REMOTE_INPUT,
    "screen_mirror": PurpleType.SCREEN_MIRROR,
    "sms": PurpleType.SMS,
    "status": PurpleType.STATUS
});

class EnumValues<T> {
    Map<String, T> map;
    late Map<T, String> reverseMap;

    EnumValues(this.map);

    Map<T, String> get reverse {
            reverseMap = map.map((k, v) => MapEntry(v, k));
            return reverseMap;
    }
}
