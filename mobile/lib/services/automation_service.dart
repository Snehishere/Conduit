import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/automation_rule.dart';
import 'websocket_service.dart';

class AutomationService extends ChangeNotifier {
  static const MethodChannel _channel = MethodChannel('com.conduit.mobile/native');

  final List<AutomationRule> _rules = [];
  WebSocketService? _ws;
  Timer? _triggerTimer;
  String? _lastWifiName;
  int? _lastBatteryLevel;
  List<String> _lastRunningApps = [];

  List<AutomationRule> get rules => List.unmodifiable(_rules);
  List<AutomationRule> get activeRules => _rules.where((r) => r.enabled).toList();

  void attachWebSocket(WebSocketService ws) {
    _ws = ws;
  }

  Future<void> loadRules() async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString('automation_rules');
    if (raw != null) {
      final List<dynamic> list = jsonDecode(raw);
      _rules.clear();
      _rules.addAll(list.map((e) => AutomationRule.fromJson(e)));
      notifyListeners();
    }
  }

  Future<void> _persist() async {
    final prefs = await SharedPreferences.getInstance();
    final json = _rules.map((r) => r.toJson()).toList();
    await prefs.setString('automation_rules', jsonEncode(json));
  }

  void addRule(AutomationRule rule) {
    _rules.add(rule);
    _persist();
    _sendRuleToHub(rule);
    notifyListeners();
  }

  void removeRule(String id) {
    _rules.removeWhere((r) => r.id == id);
    _persist();
    _notifyHubRuleRemoved(id);
    notifyListeners();
  }

  void toggleRule(String id) {
    final idx = _rules.indexWhere((r) => r.id == id);
    if (idx == -1) return;
    _rules[idx] = _rules[idx].copyWith(enabled: !_rules[idx].enabled);
    _persist();
    _sendRuleToHub(_rules[idx]);
    notifyListeners();
  }

  void updateRule(AutomationRule rule) {
    final idx = _rules.indexWhere((r) => r.id == rule.id);
    if (idx == -1) return;
    _rules[idx] = rule;
    _persist();
    _sendRuleToHub(rule);
    notifyListeners();
  }

  /// Start polling for trigger conditions (battery, WiFi, apps)
  void startTriggerDetection() {
    _triggerTimer?.cancel();
    _triggerTimer = Timer.periodic(const Duration(seconds: 30), (_) => _checkTriggers());
    _checkTriggers();
  }

  void stopTriggerDetection() {
    _triggerTimer?.cancel();
    _triggerTimer = null;
  }

  Future<void> _checkTriggers() async {
    try {
      // Check battery level
      final batteryResult = await _channel.invokeMethod<int>('getBatteryLevel');
      if (batteryResult != null && batteryResult != _lastBatteryLevel) {
        _lastBatteryLevel = batteryResult;
        _evaluateTriggers('battery_level', batteryResult);
      }

      // Check WiFi name
      final wifiResult = await _channel.invokeMethod<String>('getWifiName');
      if (wifiResult != null && wifiResult != _lastWifiName) {
        final oldWifi = _lastWifiName;
        _lastWifiName = wifiResult;
        if (oldWifi != null) {
          _evaluateTriggers('wifi_change', {'from': oldWifi, 'to': wifiResult});
        }
      }

      // Check running apps
      final appsResult = await _channel.invokeMethod<String>('getRunningApps');
      if (appsResult != null) {
        final List<dynamic> apps = jsonDecode(appsResult);
        final currentApps = apps.cast<String>();
        final newApps = currentApps.where((a) => !_lastRunningApps.contains(a)).toList();
        _lastRunningApps = currentApps;
        for (final app in newApps) {
          _evaluateTriggers('app_open', app);
        }
      }
    } catch (e) {
      debugPrint('[Automation] Trigger check error: $e');
    }
  }

  void _evaluateTriggers(String triggerType, dynamic value) {
    for (final rule in activeRules) {
      if (rule.trigger.type.wireType != triggerType) continue;
      bool matched = false;
      switch (triggerType) {
        case 'battery_level':
          final threshold = rule.trigger.below ?? 20;
          matched = (value as int) <= threshold;
          break;
        case 'wifi_change':
          final targetSsid = rule.trigger.ssid;
          matched = targetSsid != null && (value as Map)['to'] == targetSsid;
          break;
        case 'app_open':
          final targetApp = rule.trigger.appPackage;
          matched = targetApp != null && value == targetApp;
          break;
        default:
          break;
      }
      if (matched) {
        debugPrint('[Automation] Rule triggered: ${rule.id} (${rule.name})');
        _ws?.sendMessage({
          'type': 'automation',
          'action': 'triggered',
          'id': rule.id,
          'trigger_type': triggerType,
          'device_id': _ws?.deviceId ?? 'mobile',
        });
      }
    }
  }

  void _sendRuleToHub(AutomationRule rule) {
    _ws?.sendMessage(rule.toJson());
  }

  void _notifyHubRuleRemoved(String ruleId) {
    _ws?.sendMessage({
      'type': 'automation',
      'action': 'rule_remove',
      'id': ruleId,
    });
  }

  void handleRuleTriggered(Map<String, dynamic> data) {
    final ruleId = data['id'] as String?;
    if (ruleId != null) {
      debugPrint('[Automation] Rule triggered from hub: $ruleId');
    }
  }

  @override
  void dispose() {
    stopTriggerDetection();
    super.dispose();
  }
}
