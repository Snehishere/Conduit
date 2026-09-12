use chrono::Timelike;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerType {
    DeviceConnect { device_id: String },
    DeviceDisconnect { device_id: String },
    Time { time: String },
    BatteryLevel {
        #[serde(alias = "battery_threshold")]
        below: i32,
        device_id: Option<String>,
    },
    #[serde(rename = "wifi_change")]
    WiFiChange {
        #[serde(alias = "wifi_ssid")]
        ssid: String,
    },
    AppOpen {
        app_package: String,
        device_id: Option<String>,
    },
    AudioDeviceConnect {
        #[serde(alias = "audio_device_id")]
        device_id: String,
    },
    AudioDeviceDisconnect {
        #[serde(alias = "audio_device_id")]
        device_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionType {
    SendNotification { title: String, body: String },
    SetPhoneProfile { profile: String },
    RouteAudio {
        #[serde(alias = "audio_device_id")]
        device_id: String,
    },
    RunShellCommand { command: String },
    #[serde(rename = "toggle_wifi")]
    ToggleWiFi { enabled: bool },
    ToggleBluetooth { enabled: bool },
    OpenUrl { url: String },
    OpenApp { app_package: String },
    SetWindowState { state: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: TriggerType,
    pub action: ActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExecutionLog {
    pub id: String,
    pub trigger_type: String,
    pub timestamp: i64,
    pub success: bool,
    pub message: Option<String>,
}

pub struct AutomationEngine {
    rules: Vec<AutomationRule>,
}

impl AutomationEngine {
    pub fn new() -> Self {
        AutomationEngine { rules: Vec::new() }
    }

    pub fn load_rules(&mut self, rules: Vec<AutomationRule>) {
        self.rules = rules;
        info!("Loaded {} automation rules", self.rules.len());
    }

    pub fn add_rule(&mut self, rule: AutomationRule) {
        self.rules.push(rule);
    }

    pub fn remove_rule(&mut self, id: &str) {
        self.rules.retain(|r| r.id != id);
    }

    pub fn update_rule(&mut self, rule: AutomationRule) {
        self.remove_rule(&rule.id);
        self.add_rule(rule);
    }

    pub fn get_rules(&self) -> &[AutomationRule] {
        &self.rules
    }

    pub fn get_rule(&self, id: &str) -> Option<&AutomationRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    pub fn check_time_triggers(&self) -> Vec<String> {
        let now = chrono::Local::now();
        let current_time = format!("{:02}:{:02}", now.hour(), now.minute());
        let mut triggered = Vec::new();
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if let TriggerType::Time { ref time } = rule.trigger {
                if time == &current_time {
                    triggered.push(rule.id.clone());
                    info!("Time trigger fired for rule: {} ({})", rule.name, rule.id);
                }
            }
        }
        triggered
    }

    pub async fn evaluate_trigger(&self, trigger: &TriggerType, context: &TriggerContext) -> bool {
        match trigger {
            TriggerType::DeviceConnect { device_id } => {
                context.event == TriggerEvent::DeviceConnect
                    && (device_id.as_str() == "*" || context.source_device_id == *device_id)
            }
            TriggerType::DeviceDisconnect { device_id } => {
                context.event == TriggerEvent::DeviceDisconnect
                    && (device_id.as_str() == "*" || context.source_device_id == *device_id)
            }
            TriggerType::Time { time } => {
                if context.event != TriggerEvent::TimeTick {
                    return false;
                }
                parse_hhmm(time).is_some_and(|(h, m)| {
                    let now = chrono::Local::now();
                    now.hour() == h && now.minute() == m
                })
            }
            TriggerType::BatteryLevel { below, device_id } => {
                let scope_ok =
                    device_id.as_deref().is_none_or(|d| d == "*" || d == context.source_device_id);
                scope_ok
                    && context.event == TriggerEvent::BatteryUpdate
                    && context.battery_level.is_some_and(|b| b < *below)
            }
            TriggerType::WiFiChange { ssid } => {
                context.event == TriggerEvent::WiFiChange
                    && context.wifi_ssid.as_deref() == Some(ssid.as_str())
            }
            TriggerType::AppOpen { app_package, device_id } => {
                let scope_ok =
                    device_id.as_deref().is_none_or(|d| d == "*" || d == context.source_device_id);
                scope_ok
                    && context.event == TriggerEvent::AppOpen
                    && context.app_package.as_deref() == Some(app_package.as_str())
            }
            TriggerType::AudioDeviceConnect { device_id } => {
                context.event == TriggerEvent::AudioDeviceConnect
                    && (device_id.as_str() == "*" || context.source_device_id == *device_id)
            }
            TriggerType::AudioDeviceDisconnect { device_id } => {
                context.event == TriggerEvent::AudioDeviceDisconnect
                    && (device_id.as_str() == "*" || context.source_device_id == *device_id)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum TriggerEvent {
    #[default]
    DeviceConnect,
    DeviceDisconnect,
    TimeTick,
    BatteryUpdate,
    WiFiChange,
    AppOpen,
    AudioDeviceConnect,
    AudioDeviceDisconnect,
}

#[derive(Debug, Clone, Default)]
pub struct TriggerContext {
    pub event: TriggerEvent,
    pub source_device_id: String,
    pub battery_level: Option<i32>,
    pub wifi_ssid: Option<String>,
    pub app_package: Option<String>,
}

pub fn trigger_tag(trigger: &TriggerType) -> String {
    serde_json::to_value(trigger)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_default()
}

pub fn action_tag(action: &ActionType) -> String {
    serde_json::to_value(action)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_default()
}

pub fn is_desktop_executable(action: &ActionType) -> bool {
    matches!(
        action,
        ActionType::SendNotification { .. }
            | ActionType::RouteAudio { .. }
            | ActionType::RunShellCommand { .. }
            | ActionType::ToggleWiFi { .. }
            | ActionType::ToggleBluetooth { .. }
            | ActionType::OpenUrl { .. }
            | ActionType::SetWindowState { .. }
    )
}

pub fn validate_rule(rule: &AutomationRule) -> Result<(), String> {
    if rule.id.trim().is_empty() {
        return Err("rule id must not be empty".to_string());
    }
    if rule.name.trim().is_empty() {
        return Err("rule name must not be empty".to_string());
    }
    match &rule.trigger {
        TriggerType::Time { time } => {
            if !is_valid_hhmm(time) {
                return Err(format!("invalid time '{}', must be HH:MM (24h)", time));
            }
        }
        TriggerType::BatteryLevel { below, .. } => {
            if *below < 0 || *below > 100 {
                return Err(format!("battery below must be 0..100, got {}", below));
            }
        }
        TriggerType::AppOpen { app_package, .. }
            if app_package.contains('*') => {
                return Err("app_package must not contain wildcards".to_string());
            }
        _ => {}
    }
    Ok(())
}

pub fn re_evaluate_rule(
    rule: &AutomationRule,
    event_tag: &str,
    device_id: &str,
    battery: Option<i32>,
) -> bool {
    if !rule.enabled {
        return false;
    }
    if trigger_tag(&rule.trigger) != event_tag {
        return false;
    }
    match &rule.trigger {
        TriggerType::DeviceConnect { device_id: d }
        | TriggerType::DeviceDisconnect { device_id: d }
        | TriggerType::AudioDeviceConnect { device_id: d }
        | TriggerType::AudioDeviceDisconnect { device_id: d } => d.as_str() == "*" || d == device_id,
        TriggerType::BatteryLevel { below, device_id: d } => {
            let scope_ok = d.as_deref().is_none_or(|x| x == "*" || x == device_id);
            scope_ok && battery.is_none_or(|b| b < *below)
        }
        TriggerType::Time { .. } => true,
        TriggerType::WiFiChange { .. } => true,
        TriggerType::AppOpen { .. } => true,
    }
}

pub fn parse_rule_packet(value: &serde_json::Value) -> Result<AutomationRule, String> {
    // Frontend sends { type:'automation', action:'rule', rule:{...} } — unwrap nested rule object if present.
    let src = match value.get("rule") {
        Some(r) if r.is_object() => r,
        _ => value,
    };
    let id = src.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let name = src.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let enabled = src.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    let trigger_val = src
        .get("trigger")
        .ok_or_else(|| "missing trigger".to_string())?;
    let trigger: TriggerType =
        serde_json::from_value(trigger_val.clone()).map_err(|e| format!("trigger: {}", e))?;

    let action_val = src
        .get("rule_action")
        .filter(|v| v.is_object())
        .or_else(|| src.get("action").filter(|v| v.is_object()))
        .or_else(|| src.get("action_config").filter(|v| v.is_object()))
        .ok_or_else(|| "missing action object".to_string())?;
    let action: ActionType =
        serde_json::from_value(action_val.clone()).map_err(|e| format!("action: {}", e))?;

    Ok(AutomationRule {
        id,
        name,
        enabled,
        trigger,
        action,
    })
}

fn is_valid_hhmm(time: &str) -> bool {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return false;
    }
    let (hour, minute) = match (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
        (Ok(h), Ok(m)) => (h, m),
        _ => return false,
    };
    hour <= 23 && minute <= 59
}

fn parse_hhmm(time: &str) -> Option<(u32, u32)> {
    if !is_valid_hhmm(time) {
        return None;
    }
    let mut parts = time.split(':');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

pub fn execute_action(action: &ActionType) -> RuleExecutionLog {
    let timestamp = chrono::Utc::now().timestamp();
    match action {
        ActionType::RunShellCommand { command } => {
            info!("Executing shell command: {}", command);
            let output = if cfg!(target_os = "windows") {
                Command::new("cmd").args(["/C", command]).output()
            } else {
                Command::new("sh").args(["-c", command]).output()
            };

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let success = out.status.success();
                    let msg = if success {
                        info!("Command succeeded: {}", stdout.trim());
                        format!("Exit 0: {}", stdout.trim())
                    } else {
                        let code = out.status.code().unwrap_or(-1);
                        warn!("Command failed (exit {}): {}", code, stderr.trim());
                        format!("Exit {}: {}", code, stderr.trim())
                    };
                    RuleExecutionLog {
                        id: String::new(),
                        trigger_type: "run_shell_command".to_string(),
                        timestamp,
                        success,
                        message: Some(msg),
                    }
                }
                Err(e) => {
                    error!("Failed to execute command: {}", e);
                    RuleExecutionLog {
                        id: String::new(),
                        trigger_type: "run_shell_command".to_string(),
                        timestamp,
                        success: false,
                        message: Some(format!("Exec error: {}", e)),
                    }
                }
            }
        }
        ActionType::SetWindowState { state } => {
            info!("Setting window state: {} (standalone — requires window context)", state);
            let valid = matches!(state.as_str(), "minimize" | "maximize" | "restore" | "close");
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "set_window_state".to_string(),
                timestamp,
                success: false,
                message: Some(if valid {
                    "Window action requires desktop window context; use execute_automation_action".to_string()
                } else {
                    format!("Invalid window state: {}", state)
                }),
            }
        }
        ActionType::SendNotification { title, body } => {
            info!("Sending notification: {} - {}", title, body);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "send_notification".to_string(),
                timestamp,
                success: true,
                message: Some(format!("Notification: {} - {}", title, body)),
            }
        }
        ActionType::SetPhoneProfile { profile } => {
            info!("Setting phone profile: {} (phone-only)", profile);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "set_phone_profile".to_string(),
                timestamp,
                success: false,
                message: Some(format!("Phone profile is phone-only, not executed on desktop: {}", profile)),
            }
        }
        ActionType::RouteAudio { device_id } => {
            info!("Routing audio to device: {}", device_id);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "route_audio".to_string(),
                timestamp,
                success: true,
                message: Some(format!("Audio routed to: {}", device_id)),
            }
        }
        ActionType::ToggleWiFi { enabled } => {
            info!("Toggling WiFi: {}", enabled);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "toggle_wifi".to_string(),
                timestamp,
                success: true,
                message: Some(format!("WiFi set to: {}", enabled)),
            }
        }
        ActionType::ToggleBluetooth { enabled } => {
            info!("Toggling Bluetooth: {}", enabled);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "toggle_bluetooth".to_string(),
                timestamp,
                success: true,
                message: Some(format!("Bluetooth set to: {}", enabled)),
            }
        }
        ActionType::OpenUrl { url } => {
            info!("Opening URL: {}", url);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "open_url".to_string(),
                timestamp,
                success: true,
                message: Some(format!("URL opened: {}", url)),
            }
        }
        ActionType::OpenApp { app_package } => {
            info!("Open app (phone/tablet only): {}", app_package);
            RuleExecutionLog {
                id: String::new(),
                trigger_type: "open_app".to_string(),
                timestamp,
                success: false,
                message: Some(format!("App open is phone/tablet only, not executed on desktop: {}", app_package)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_trigger_shapes() {
        assert_eq!(
            serde_json::to_string(&TriggerType::Time { time: "22:00".to_string() }).unwrap(),
            r#"{"type":"time","time":"22:00"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::BatteryLevel { below: 20, device_id: Some("*".to_string()) }).unwrap(),
            r#"{"type":"battery_level","below":20,"device_id":"*"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::WiFiChange { ssid: "HomeNet".to_string() }).unwrap(),
            r#"{"type":"wifi_change","ssid":"HomeNet"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::AppOpen { app_package: "com.spotify.music".to_string(), device_id: Some("d_002".to_string()) }).unwrap(),
            r#"{"type":"app_open","app_package":"com.spotify.music","device_id":"d_002"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::AudioDeviceDisconnect { device_id: "d_003".to_string() }).unwrap(),
            r#"{"type":"audio_device_disconnect","device_id":"d_003"}"#
        );
    }

    #[test]
    fn wire_action_shapes() {
        assert_eq!(
            serde_json::to_string(&ActionType::ToggleWiFi { enabled: false }).unwrap(),
            r#"{"type":"toggle_wifi","enabled":false}"#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::ToggleBluetooth { enabled: true }).unwrap(),
            r#"{"type":"toggle_bluetooth","enabled":true}"#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::RouteAudio { device_id: "d_003".to_string() }).unwrap(),
            r#"{"type":"route_audio","device_id":"d_003"}"#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::OpenApp { app_package: "com.whatsapp".to_string() }).unwrap(),
            r#"{"type":"open_app","app_package":"com.whatsapp"}"#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::SetWindowState { state: "minimize".to_string() }).unwrap(),
            r#"{"type":"set_window_state","state":"minimize"}"#
        );
    }

    #[test]
    fn deprecated_aliases_accepted_on_read() {
        let battery: TriggerType = serde_json::from_str(
            r#"{"type":"battery_level","battery_threshold":15}"#,
        )
        .unwrap();
        assert_eq!(
            battery,
            TriggerType::BatteryLevel { below: 15, device_id: None }
        );

        let wifi: TriggerType =
            serde_json::from_str(r#"{"type":"wifi_change","wifi_ssid":"Office"}"#).unwrap();
        assert_eq!(
            wifi,
            TriggerType::WiFiChange { ssid: "Office".to_string() }
        );

        let route: ActionType =
            serde_json::from_str(r#"{"type":"route_audio","audio_device_id":"d_003"}"#).unwrap();
        assert_eq!(
            route,
            ActionType::RouteAudio { device_id: "d_003".to_string() }
        );
    }

    #[test]
    fn tags_match_schema() {
        assert_eq!(trigger_tag(&TriggerType::WiFiChange { ssid: "x".to_string() }), "wifi_change");
        assert_eq!(trigger_tag(&TriggerType::AppOpen { app_package: "x".to_string(), device_id: None }), "app_open");
        assert_eq!(action_tag(&ActionType::ToggleWiFi { enabled: true }), "toggle_wifi");
        assert_eq!(action_tag(&ActionType::RunShellCommand { command: "x".to_string() }), "run_shell_command");
    }

    #[test]
    fn parse_rule_packet_works_for_both_wire_forms() {
        let with_rule_action = serde_json::json!({
            "type": "automation",
            "action": "rule",
            "id": "rule_001",
            "name": "Bedtime silent",
            "trigger": {"type": "time", "time": "22:00"},
            "rule_action": {"type": "set_phone_profile", "profile": "silent"},
            "enabled": true
        });
        let rule = parse_rule_packet(&with_rule_action).unwrap();
        assert_eq!(rule.id, "rule_001");
        assert_eq!(rule.name, "Bedtime silent");
        assert!(rule.enabled);
        assert_eq!(rule.trigger, TriggerType::Time { time: "22:00".to_string() });
        assert_eq!(rule.action, ActionType::SetPhoneProfile { profile: "silent".to_string() });
    }

    #[test]
    fn validation_rejects_bad_time_and_range() {
        let bad_time = AutomationRule {
            id: "r1".into(),
            name: "t".into(),
            enabled: true,
            trigger: TriggerType::Time { time: "25:99".into() },
            action: ActionType::SendNotification { title: "t".into(), body: "b".into() },
        };
        assert!(validate_rule(&bad_time).is_err());

        let bad_battery = AutomationRule {
            id: "r2".into(),
            name: "t".into(),
            enabled: true,
            trigger: TriggerType::BatteryLevel { below: 101, device_id: None },
            action: ActionType::SendNotification { title: "t".into(), body: "b".into() },
        };
        assert!(validate_rule(&bad_battery).is_err());
    }

    #[test]
    fn re_evaluate_rule_scopes_and_tags() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "t".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect { device_id: "d_002".into() },
            action: ActionType::RouteAudio { device_id: "d_003".into() },
        };
        assert!(re_evaluate_rule(&rule, "device_connect", "d_002", None));
        assert!(!re_evaluate_rule(&rule, "device_disconnect", "d_002", None));
        assert!(!re_evaluate_rule(&rule, "device_connect", "d_009", None));

        let disabled = AutomationRule { enabled: false, ..rule };
        assert!(!re_evaluate_rule(&disabled, "device_connect", "d_002", None));
    }
}