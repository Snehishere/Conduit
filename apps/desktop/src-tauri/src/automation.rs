use chrono::Timelike;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::error::ConduitError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerType {
    DeviceConnect {
        device_id: String,
    },
    DeviceDisconnect {
        device_id: String,
    },
    Time {
        time: String,
    },
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
    SendNotification {
        title: String,
        body: String,
    },
    SetPhoneProfile {
        profile: String,
    },
    RouteAudio {
        #[serde(alias = "audio_device_id")]
        device_id: String,
    },
    RunShellCommand {
        command: String,
    },
    #[serde(rename = "toggle_wifi")]
    ToggleWiFi {
        enabled: bool,
    },
    ToggleBluetooth {
        enabled: bool,
    },
    OpenUrl {
        url: String,
    },
    OpenApp {
        app_package: String,
    },
    SetWindowState {
        state: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: TriggerType,
    pub action: ActionType,
    /// If true, this rule may only be triggered by a device with a confirmed
    /// shared secret (i.e. already paired). Unpaired/anonymous WS connections
    /// cannot fire this rule.
    #[serde(default)]
    pub trusted_source_only: bool,
}

/// Per-instance allowlist of shell commands that automation is permitted to run.
/// An empty allowlist means ALL shell commands are BLOCKED (deny-by-default).
///
/// Commands are matched by **executable token** — only the first token (the
/// executable name) is compared against the allowlist entries.  This prevents
/// bypasses such as `ls; rm -rf /` where a shell metacharacter chains an
/// unauthorised command after an allowed one.
///
/// Shell metacharacters (`;`, `|`, `&`, `$`, `` ` ``, `>`, `<`, newlines)
/// anywhere in the command cause an immediate rejection.
///
/// Absolute paths without `..` segments also match a **bare-name** entry via
/// their final component, so `/usr/bin/ls` is allowed by an `ls` entry
/// (see `executable_matches_entry` for the documented residual risk).
/// Relative paths containing a separator (`./ls`, `../ls`, `bin/ls`) are
/// rejected unless that exact literal string is an entry.
///
/// Add `"*"` as the only entry to allow any command (not recommended).
#[derive(Debug, Clone, Default)]
pub struct CommandAllowlist {
    entries: Vec<String>,
}

impl CommandAllowlist {
    /// Create a new allowlist from a list of permitted command names.
    #[allow(dead_code)]
    pub fn new(entries: Vec<String>) -> Self {
        Self { entries }
    }

    /// Returns `true` if `command` is allowed by this allowlist.
    ///
    /// Matching strategy:
    /// 1. Wildcard `"*"` allows everything.
    /// 2. Reject commands containing shell metacharacters that enable command
    ///    chaining, substitution, or redirection.
    /// 3. Extract the first token (the executable) from the command, properly
    ///    handling quoted strings and backslash escapes.
    /// 4. Match the extracted executable against the allowlist entries using
    ///    `executable_matches_entry` (exact match, plus clean absolute paths
    ///    against bare-name entries via their final component).
    pub fn is_allowed(&self, command: &str) -> bool {
        if self.entries.iter().any(|e| e == "*") {
            return true;
        }

        let cmd_trimmed = command.trim();
        if cmd_trimmed.is_empty() {
            return false;
        }

        // Reject shell metacharacters — these allow chaining, substitution, or
        // redirection and must never appear in an automation command.
        if contains_shell_metacharacters(cmd_trimmed) {
            return false;
        }

        // Extract the executable token (first word, respecting quotes/escapes).
        let executable = match extract_executable(cmd_trimmed) {
            Some(exe) => exe,
            None => return false,
        };

        // Match the executable token (not the full command) against the allowlist.
        self.entries
            .iter()
            .any(|entry| executable_matches_entry(&executable, entry.trim()))
    }

    /// Returns `true` if no entries are configured (fully locked down).
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Returns `true` if `command` contains any shell metacharacter that could
/// enable command chaining, substitution, or redirection.
///
/// Rejected characters:
/// - `;`  — command separator
/// - `|`  — pipe / logical OR
/// - `&`  — background execution / logical AND (`&&`)
/// - `$`  — variable expansion / command substitution (`$()`)
/// - `` ` `` — command substitution
/// - `>`  — output redirection
/// - `<`  — input redirection
/// - `\n` — newline (multiple commands)
fn contains_shell_metacharacters(command: &str) -> bool {
    command
        .chars()
        .any(|c| matches!(c, ';' | '|' | '&' | '$' | '`' | '>' | '<' | '\n'))
}

/// Extracts the first token (executable name) from a command string.
///
/// Handles:
/// - Double-quoted strings: `echo "hello world"` → `echo`
/// - Single-quoted strings: `echo 'hello world'` → `echo`
/// - Backslash escapes: `echo hello\ world` → `echo`
/// - Escaped quotes: `echo \"hello\"` → `echo`
/// - Leading whitespace
///
/// Returns `None` for empty / unparseable commands.
fn extract_executable(command: &str) -> Option<String> {
    let bytes = command.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // Skip leading whitespace.
    while i < len && bytes[i] == b' ' {
        i += 1;
    }
    if i >= len {
        return None;
    }

    let start = i;

    match bytes[i] {
        // Double-quoted token — read until closing `"`.
        b'"' => {
            i += 1; // skip opening quote
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2; // skip escaped character
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1; // skip closing quote
            }
            // The token is everything between the quotes (excluding them).
            // We need to re-scan to build the unquoted content.
            Some(unquote(&command[start..i], '"'))
        }
        // Single-quoted token — read until closing `'` (no escape processing).
        b'\'' => {
            i += 1;
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            Some(unquote(&command[start..i], '\''))
        }
        // Unquoted token — read until whitespace.
        _ => {
            while i < len && bytes[i] != b' ' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2; // skip escaped character
                } else {
                    i += 1;
                }
            }
            Some(unescape(&command[start..i]))
        }
    }
}

/// Remove surrounding `quote` characters and process backslash escapes.
fn unquote(token: &str, quote: char) -> String {
    let inner = token
        .strip_prefix(quote)
        .and_then(|s| s.strip_suffix(quote))
        .unwrap_or(token);
    unescape(inner)
}

/// Process backslash escapes in a token.  `\"` → `"`, `\\` → `\`, `\x` → `x`.
fn unescape(token: &str) -> String {
    let bytes = token.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 1; // skip the backslash, push the next character as-is
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Returns `true` if the extracted `executable` token is permitted by a single
/// allowlist `entry`.
///
/// Matching rules:
/// - **Exact match**: the token equals the entry (`ls` ↔ `ls`).
/// - **Clean absolute path** (POSIX `/...`, UNC `\\...`, or drive-letter
///   `C:\...` / `C:/...`) with **no `..` segments**: the entry may also be a
///   bare executable name matching the final path component, so `/usr/bin/ls`
///   is allowed by an `ls` entry. Entries that themselves contain a path
///   separator only ever match the exact full token (an entry of
///   `/usr/bin/ls` does **not** match `ls` or `/tmp/ls`).
/// - **Relative paths containing a separator** (`./ls`, `../ls`, `bin/ls`)
///   are rejected unless the entry is that exact literal string — CWD-relative
///   and traversal-based resolution is never interpreted.
///
/// Residual risk (accepted & documented): an attacker who already has a
/// *separate* file write + execute-capable vector could plant a binary at an
/// attacker-chosen absolute path with an allowlisted basename (e.g.
/// `/tmp/ls`) and have it matched. Exploiting that requires capabilities
/// outside this control: the command string itself cannot create the file,
/// because shell metacharacters (redirection `$()` substitution, chaining,
/// pipes) are rejected upstream by [`contains_shell_metacharacters`] before
/// this function is ever reached. Traversal in the path is additionally
/// rejected here.
fn executable_matches_entry(executable: &str, entry: &str) -> bool {
    if executable == entry {
        return true;
    }
    if !is_absolute_path(executable) || has_parent_segment(executable) {
        return false;
    }
    // Only bare-name entries may match a clean absolute path's final component.
    if entry.contains('/') || entry.contains('\\') {
        return false;
    }
    basename(executable).is_some_and(|base| base == entry)
}

/// Absolute path: POSIX root (`/...`), Windows UNC (`\\...`), or drive letter
/// with separator (`C:\...` / `C:/...`). Drive-relative `C:ls` is NOT absolute.
fn is_absolute_path(path: &str) -> bool {
    let b = path.as_bytes();
    if b.first() == Some(&b'/') {
        return true;
    }
    if path.starts_with("\\\\") {
        return true;
    }
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

/// True if any `/`- or `\`-separated path segment is `..`.
fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "..")
}

/// Final component of a path (splits on `/` and `\`). `None` if empty.
fn basename(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty())
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
            if let TriggerType::Time { ref time } = rule.trigger
                && time == &current_time
            {
                triggered.push(rule.id.clone());
                info!("Time trigger fired for rule: {} ({})", rule.name, rule.id);
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
                let scope_ok = device_id
                    .as_deref()
                    .is_none_or(|d| d == "*" || d == context.source_device_id);
                scope_ok
                    && context.event == TriggerEvent::BatteryUpdate
                    && context.battery_level.is_some_and(|b| b < *below)
            }
            TriggerType::WiFiChange { ssid } => {
                context.event == TriggerEvent::WiFiChange
                    && context.wifi_ssid.as_deref() == Some(ssid.as_str())
            }
            TriggerType::AppOpen {
                app_package,
                device_id,
            } => {
                let scope_ok = device_id
                    .as_deref()
                    .is_none_or(|d| d == "*" || d == context.source_device_id);
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

/// Does the **desktop hub** execute this action, rather than a paired phone?
///
/// This is a routing predicate, not a capability predicate: it answers "which
/// side runs this", and therefore which side's execution path — and which log
/// row — a rule produces. It is deliberately **not** narrowed to the actions the
/// hub has an implementation for.
///
/// Narrowing it would be actively harmful. `handlers/auto_rules.rs` uses it to
/// choose between executing the action and taking the "this executes on the
/// target phone/tablet" branch, and that branch writes `success = 1` to
/// `automation_logs` without sending anything to any device (the desktop never
/// emits `automation/triggered` outbound — see W1.16). Dropping the five
/// unimplemented actions out of this set would therefore trade five false
/// successes for seven, and would misattribute them to a peer that never
/// received the frame. The truthful report comes from
/// [`execute_action_with_allowlist`] returning `success: false`, which is where
/// W3.1 was fixed.
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

pub fn validate_rule(rule: &AutomationRule) -> Result<(), ConduitError> {
    if rule.id.trim().is_empty() {
        return Err(ConduitError::Protocol(
            "rule id must not be empty".to_string(),
        ));
    }
    if rule.name.trim().is_empty() {
        return Err(ConduitError::Protocol(
            "rule name must not be empty".to_string(),
        ));
    }
    match &rule.trigger {
        TriggerType::Time { time } => {
            if !is_valid_hhmm(time) {
                return Err(ConduitError::Protocol(format!(
                    "invalid time '{}', must be HH:MM (24h)",
                    time
                )));
            }
        }
        TriggerType::BatteryLevel { below, .. } => {
            if *below < 0 || *below > 100 {
                return Err(ConduitError::Protocol(format!(
                    "battery below must be 0..100, got {}",
                    below
                )));
            }
        }
        TriggerType::AppOpen { app_package, .. } if app_package.contains('*') => {
            return Err(ConduitError::Protocol(
                "app_package must not contain wildcards".to_string(),
            ));
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
        | TriggerType::AudioDeviceDisconnect { device_id: d } => {
            d.as_str() == "*" || d == device_id
        }
        TriggerType::BatteryLevel {
            below,
            device_id: d,
        } => {
            let scope_ok = d.as_deref().is_none_or(|x| x == "*" || x == device_id);
            scope_ok && battery.is_none_or(|b| b < *below)
        }
        TriggerType::Time { .. } => true,
        TriggerType::WiFiChange { .. } => true,
        TriggerType::AppOpen { .. } => true,
    }
}

pub fn parse_rule_packet(value: &serde_json::Value) -> Result<AutomationRule, ConduitError> {
    // Frontend sends { type:'automation', action:'rule', rule:{...} } — unwrap nested rule object if present.
    let src = match value.get("rule") {
        Some(r) if r.is_object() => r,
        _ => value,
    };
    let id = src
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = src
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let enabled = src.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    let trigger_val = src
        .get("trigger")
        .ok_or_else(|| ConduitError::Protocol("missing trigger".to_string()))?;
    let trigger: TriggerType = serde_json::from_value(trigger_val.clone())
        .map_err(|e| ConduitError::Protocol(format!("trigger: {}", e)))?;

    let action_val = src
        .get("rule_action")
        .filter(|v| v.is_object())
        .or_else(|| src.get("action").filter(|v| v.is_object()))
        .or_else(|| src.get("action_config").filter(|v| v.is_object()))
        .ok_or_else(|| ConduitError::Protocol("missing action object".to_string()))?;
    let action: ActionType = serde_json::from_value(action_val.clone())
        .map_err(|e| ConduitError::Protocol(format!("action: {}", e)))?;

    let trusted_source_only = src
        .get("trusted_source_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Ok(AutomationRule {
        id,
        name,
        enabled,
        trigger,
        action,
        trusted_source_only,
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

/// The desktop hub has no implementation for `action_tag`, so nothing happened.
///
/// Every arm that reaches this helper used to `info!()` its parameters and
/// return `success: true`. The caller writes that boolean to
/// `automation_logs.success`, the Automation panel renders it green, and the
/// user believes their machine sent a notification, routed audio, toggled
/// Wi-Fi, toggled Bluetooth or opened a URL. None of those five had an
/// implementation: there is no notification emitter, no Wi-Fi/Bluetooth
/// control and no URL opener anywhere in this crate.
///
/// Reporting failure is the whole fix here. Implementing the actions is a
/// separate piece of work and is deliberately not attempted here.
fn not_executed(action_tag: &str, timestamp: i64, detail: impl Into<String>) -> RuleExecutionLog {
    let detail = detail.into();
    warn!(
        "Automation action {} is not implemented on the desktop hub: {}",
        action_tag, detail
    );
    RuleExecutionLog {
        id: String::new(),
        trigger_type: action_tag.to_string(),
        timestamp,
        success: false,
        message: Some(format!("Not executed on the desktop hub: {}", detail)),
    }
}

/// Execute an action with no allowlist enforcement (use only for internal/desktop-initiated actions).
pub fn execute_action(action: &ActionType) -> RuleExecutionLog {
    execute_action_with_allowlist(action, None)
}

/// Execute an action, enforcing the provided shell-command allowlist.
/// Pass `Some(allowlist)` for externally-triggered actions (from mobile devices).
/// Pass `None` to skip allowlist checks (e.g. user-initiated from desktop UI).
pub fn execute_action_with_allowlist(
    action: &ActionType,
    allowlist: Option<&CommandAllowlist>,
) -> RuleExecutionLog {
    let timestamp = chrono::Utc::now().timestamp();
    match action {
        ActionType::RunShellCommand { command } => {
            // Allowlist enforcement: if an allowlist is provided, the command MUST match.
            if let Some(list) = allowlist
                && !list.is_allowed(command)
            {
                warn!(
                    "Shell command blocked by allowlist (not in permitted list): {}",
                    command
                );
                return RuleExecutionLog {
                    id: String::new(),
                    trigger_type: "run_shell_command".to_string(),
                    timestamp,
                    success: false,
                    message: Some(format!(
                        "Blocked: command '{}' is not in the shell command allowlist. \
                             Add it to Settings → Automation → Allowed Commands.",
                        command
                    )),
                };
            }

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
            let valid = matches!(
                state.as_str(),
                "minimize" | "maximize" | "restore" | "close"
            );
            if !valid {
                return RuleExecutionLog {
                    id: String::new(),
                    trigger_type: "set_window_state".to_string(),
                    timestamp,
                    success: false,
                    message: Some(format!("Invalid window state: {}", state)),
                };
            }
            not_executed(
                "set_window_state",
                timestamp,
                format!(
                    "a window action requires desktop window context (state '{}'); \
                     use execute_automation_action",
                    state
                ),
            )
        }
        ActionType::SendNotification { title, body } => not_executed(
            "send_notification",
            timestamp,
            format!(
                "sending a notification is not implemented ({}: {})",
                title, body
            ),
        ),
        ActionType::SetPhoneProfile { profile } => not_executed(
            "set_phone_profile",
            timestamp,
            format!(
                "setting the phone profile is a phone-only action ({})",
                profile
            ),
        ),
        ActionType::RouteAudio { device_id } => not_executed(
            "route_audio",
            timestamp,
            format!(
                "routing audio is not implemented ({} was not touched)",
                device_id
            ),
        ),
        ActionType::ToggleWiFi { enabled } => not_executed(
            "toggle_wifi",
            timestamp,
            format!("toggling Wi-Fi is not implemented (requested {})", enabled),
        ),
        ActionType::ToggleBluetooth { enabled } => not_executed(
            "toggle_bluetooth",
            timestamp,
            format!(
                "toggling Bluetooth is not implemented (requested {})",
                enabled
            ),
        ),
        ActionType::OpenUrl { url } => not_executed(
            "open_url",
            timestamp,
            format!("opening a URL is not implemented ({})", url),
        ),
        ActionType::OpenApp { app_package } => not_executed(
            "open_app",
            timestamp,
            format!(
                "opening an app is a phone/tablet only action ({})",
                app_package
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_allowlist_empty_blocks_all() {
        let list = super::CommandAllowlist::new(vec![]);
        assert!(!list.is_allowed("echo hello"));
        assert!(!list.is_allowed("ls"));
        assert!(list.is_empty());
    }

    #[test]
    fn command_allowlist_token_matching() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string(), "ls".to_string()]);
        assert!(list.is_allowed("echo hello"));
        assert!(list.is_allowed("echo"));
        assert!(list.is_allowed("ls -la"));
        assert!(list.is_allowed("ls -la /tmp"));
        assert!(!list.is_allowed("rm -rf /"));
        assert!(!list.is_allowed("curl http://evil.com"));
    }

    #[test]
    fn command_allowlist_wildcard_allows_all() {
        let list = super::CommandAllowlist::new(vec!["*".to_string()]);
        assert!(list.is_allowed("rm -rf /"));
        assert!(list.is_allowed("any_command"));
    }

    // --- Shell metacharacter rejection tests (CRITICAL security fix) ---

    #[test]
    fn command_allowlist_rejects_semicolon_chaining() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls; rm -rf /"));
        assert!(!list.is_allowed("ls;echo pwned"));
        assert!(!list.is_allowed("ls ; whoami"));
    }

    #[test]
    fn command_allowlist_rejects_pipe() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls | grep etc"));
        assert!(!list.is_allowed("ls|cat /etc/passwd"));
    }

    #[test]
    fn command_allowlist_rejects_logical_operators() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls && rm -rf /"));
        assert!(!list.is_allowed("ls || rm -rf /"));
    }

    #[test]
    fn command_allowlist_rejects_command_substitution() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string()]);
        assert!(!list.is_allowed("echo $(whoami)"));
        assert!(!list.is_allowed("echo `whoami`"));
        assert!(!list.is_allowed("echo $USER"));
    }

    #[test]
    fn command_allowlist_rejects_redirection() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string()]);
        assert!(!list.is_allowed("echo hi > /tmp/x"));
        assert!(!list.is_allowed("echo hi >> /tmp/x"));
        assert!(!list.is_allowed("cat < /etc/passwd"));
    }

    #[test]
    fn command_allowlist_rejects_background() {
        let list = super::CommandAllowlist::new(vec!["sleep".to_string()]);
        assert!(!list.is_allowed("sleep 10 &"));
    }

    #[test]
    fn command_allowlist_rejects_newline() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls\nrm -rf /"));
    }

    // --- Executable extraction tests ---

    #[test]
    fn command_allowlist_handles_quoted_executable() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(list.is_allowed("\"ls\" -la"));
        assert!(list.is_allowed("'ls' -la"));
    }

    #[test]
    fn command_allowlist_handles_escaped_characters() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string()]);
        assert!(list.is_allowed("echo hello\\ world"));
    }

    #[test]
    fn command_allowlist_rejects_quoted_metacharacters() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string()]);
        // Even inside quotes, metacharacters indicate shell interpretation
        assert!(!list.is_allowed("echo \"; rm -rf /\""));
        assert!(!list.is_allowed("echo '| cat /etc/passwd'"));
    }

    #[test]
    fn command_allowlist_empty_command_blocked() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed(""));
        assert!(!list.is_allowed("   "));
    }

    #[test]
    fn command_allowlist_exact_match() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(list.is_allowed("ls"));
    }

    #[test]
    fn command_allowlist_no_prefix_confusion() {
        // "lsc" must NOT match when only "ls" is allowed
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("lsc"));
        assert!(!list.is_allowed("lsync"));
    }

    // --- Acceptance criteria: CRITICAL shell allowlist prefix-bypass fix ---

    #[test]
    fn acceptance_ls_passes_when_allowlisted() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(list.is_allowed("ls"));
        assert!(list.is_allowed("ls -la"));
    }

    #[test]
    fn acceptance_absolute_path_passes() {
        // Bare-name entry matches the final component of a clean absolute path.
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(list.is_allowed("/usr/bin/ls"));
        assert!(list.is_allowed("/usr/bin/ls -la"));
        assert!(list.is_allowed("/bin/ls"));

        // Full-path entries match ONLY that exact path.
        let exact = super::CommandAllowlist::new(vec!["/usr/bin/ls".to_string()]);
        assert!(exact.is_allowed("/usr/bin/ls"));
        assert!(!exact.is_allowed("/tmp/ls")); // different directory, same name
        assert!(!exact.is_allowed("ls")); // bare name is not the allowlisted path
    }

    #[test]
    fn acceptance_semicolon_chain_rejected() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls; rm -rf /"));
    }

    #[test]
    fn acceptance_pipe_rejected() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls | grep foo"));
    }

    #[test]
    fn acceptance_logical_and_rejected() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("ls && whoami"));
    }

    #[test]
    fn acceptance_command_substitution_rejected() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("$(whoami)"));
        assert!(!list.is_allowed("`whoami`"));
    }

    #[test]
    fn acceptance_cat_only_passes_when_allowlisted() {
        let no_cat = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!no_cat.is_allowed("cat file.txt"));

        let with_cat = super::CommandAllowlist::new(vec!["cat".to_string()]);
        assert!(with_cat.is_allowed("cat file.txt"));
    }

    #[test]
    fn acceptance_relative_paths_with_separators_rejected() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(!list.is_allowed("./ls"));
        assert!(!list.is_allowed("../bin/ls"));
        assert!(!list.is_allowed("bin/ls"));
        // ".." is rejected even when it would normalize to an allowlisted path.
        assert!(!list.is_allowed("/usr/bin/../bin/ls"));
    }

    #[test]
    fn execute_action_with_allowlist_blocks_disallowed_command() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string()]);
        let action = super::ActionType::RunShellCommand {
            command: "rm -rf /tmp/test".to_string(),
        };
        let log = super::execute_action_with_allowlist(&action, Some(&list));
        assert!(!log.success);
        assert!(log.message.as_deref().unwrap_or("").contains("Blocked"));
    }

    #[test]
    fn execute_action_with_allowlist_allows_permitted_command() {
        let list = super::CommandAllowlist::new(vec!["echo".to_string()]);
        let action = super::ActionType::RunShellCommand {
            command: "echo hello world".to_string(),
        };
        let log = super::execute_action_with_allowlist(&action, Some(&list));
        // Success depends on system echo being available, but command should NOT be blocked
        assert!(!log.message.as_deref().unwrap_or("").contains("Blocked"));
    }

    #[test]
    fn trusted_source_only_field_defaults_false() {
        let rule = super::AutomationRule {
            id: "r1".into(),
            name: "test".into(),
            enabled: true,
            trigger: super::TriggerType::Time {
                time: "08:00".into(),
            },
            action: super::ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        let json = serde_json::to_string(&rule).unwrap();
        // trusted_source_only should be present in wire format
        let roundtripped: super::AutomationRule = serde_json::from_str(&json).unwrap();
        assert!(!roundtripped.trusted_source_only);
    }

    #[test]
    fn trusted_source_only_roundtrips() {
        let rule = super::AutomationRule {
            id: "r2".into(),
            name: "secure".into(),
            enabled: true,
            trusted_source_only: true,
            trigger: super::TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: super::ActionType::RunShellCommand {
                command: "echo connected".into(),
            },
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("trusted_source_only"));
        let rt: super::AutomationRule = serde_json::from_str(&json).unwrap();
        assert!(rt.trusted_source_only);
    }

    use super::*;

    // --- Existing wire-format tests ---

    #[test]
    fn wire_trigger_shapes() {
        assert_eq!(
            serde_json::to_string(&TriggerType::Time {
                time: "22:00".to_string()
            })
            .unwrap(),
            r#"{"type":"time","time":"22:00"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::BatteryLevel {
                below: 20,
                device_id: Some("*".to_string())
            })
            .unwrap(),
            r#"{"type":"battery_level","below":20,"device_id":"*"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::WiFiChange {
                ssid: "HomeNet".to_string()
            })
            .unwrap(),
            r#"{"type":"wifi_change","ssid":"HomeNet"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::AppOpen {
                app_package: "com.spotify.music".to_string(),
                device_id: Some("d_002".to_string())
            })
            .unwrap(),
            r#"{"type":"app_open","app_package":"com.spotify.music","device_id":"d_002"}"#
        );
        assert_eq!(
            serde_json::to_string(&TriggerType::AudioDeviceDisconnect {
                device_id: "d_003".to_string()
            })
            .unwrap(),
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
            serde_json::to_string(&ActionType::RouteAudio {
                device_id: "d_003".to_string()
            })
            .unwrap(),
            r#"{"type":"route_audio","device_id":"d_003"}"#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::OpenApp {
                app_package: "com.whatsapp".to_string()
            })
            .unwrap(),
            r#"{"type":"open_app","app_package":"com.whatsapp"}"#
        );
        assert_eq!(
            serde_json::to_string(&ActionType::SetWindowState {
                state: "minimize".to_string()
            })
            .unwrap(),
            r#"{"type":"set_window_state","state":"minimize"}"#
        );
    }

    #[test]
    fn deprecated_aliases_accepted_on_read() {
        let battery: TriggerType =
            serde_json::from_str(r#"{"type":"battery_level","battery_threshold":15}"#).unwrap();
        assert_eq!(
            battery,
            TriggerType::BatteryLevel {
                below: 15,
                device_id: None
            }
        );

        let wifi: TriggerType =
            serde_json::from_str(r#"{"type":"wifi_change","wifi_ssid":"Office"}"#).unwrap();
        assert_eq!(
            wifi,
            TriggerType::WiFiChange {
                ssid: "Office".to_string()
            }
        );

        let route: ActionType =
            serde_json::from_str(r#"{"type":"route_audio","audio_device_id":"d_003"}"#).unwrap();
        assert_eq!(
            route,
            ActionType::RouteAudio {
                device_id: "d_003".to_string()
            }
        );
    }

    #[test]
    fn tags_match_schema() {
        assert_eq!(
            trigger_tag(&TriggerType::WiFiChange {
                ssid: "x".to_string()
            }),
            "wifi_change"
        );
        assert_eq!(
            trigger_tag(&TriggerType::AppOpen {
                app_package: "x".to_string(),
                device_id: None
            }),
            "app_open"
        );
        assert_eq!(
            action_tag(&ActionType::ToggleWiFi { enabled: true }),
            "toggle_wifi"
        );
        assert_eq!(
            action_tag(&ActionType::RunShellCommand {
                command: "x".to_string()
            }),
            "run_shell_command"
        );
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
        assert_eq!(
            rule.trigger,
            TriggerType::Time {
                time: "22:00".to_string()
            }
        );
        assert_eq!(
            rule.action,
            ActionType::SetPhoneProfile {
                profile: "silent".to_string()
            }
        );
    }

    #[test]
    fn validation_rejects_bad_time_and_range() {
        let bad_time = AutomationRule {
            id: "r1".into(),
            name: "t".into(),
            enabled: true,
            trigger: TriggerType::Time {
                time: "25:99".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&bad_time).is_err());

        let bad_battery = AutomationRule {
            id: "r2".into(),
            name: "t".into(),
            enabled: true,
            trigger: TriggerType::BatteryLevel {
                below: 101,
                device_id: None,
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&bad_battery).is_err());
    }

    #[test]
    fn re_evaluate_rule_scopes_and_tags() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "t".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "d_002".into(),
            },
            action: ActionType::RouteAudio {
                device_id: "d_003".into(),
            },
            trusted_source_only: false,
        };
        assert!(re_evaluate_rule(&rule, "device_connect", "d_002", None));
        assert!(!re_evaluate_rule(&rule, "device_disconnect", "d_002", None));
        assert!(!re_evaluate_rule(&rule, "device_connect", "d_009", None));

        let disabled = AutomationRule {
            enabled: false,
            ..rule
        };
        assert!(!re_evaluate_rule(
            &disabled,
            "device_connect",
            "d_002",
            None
        ));
    }

    // --- AutomationEngine CRUD tests ---

    #[test]
    fn engine_new_is_empty() {
        let engine = AutomationEngine::new();
        assert!(engine.get_rules().is_empty());
    }

    #[test]
    fn engine_add_and_get_rule() {
        let mut engine = AutomationEngine::new();
        let rule = AutomationRule {
            id: "r1".into(),
            name: "Test".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "Hi".into(),
                body: "Connected".into(),
            },
            trusted_source_only: false,
        };
        engine.add_rule(rule.clone());
        assert_eq!(engine.get_rules().len(), 1);
        assert!(engine.get_rule("r1").is_some());
        assert!(engine.get_rule("missing").is_none());
    }

    #[test]
    fn engine_remove_rule() {
        let mut engine = AutomationEngine::new();
        engine.add_rule(AutomationRule {
            id: "r1".into(),
            name: "A".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        });
        engine.remove_rule("r1");
        assert!(engine.get_rules().is_empty());
    }

    #[test]
    fn engine_remove_nonexistent_is_noop() {
        let mut engine = AutomationEngine::new();
        engine.remove_rule("ghost");
    }

    #[test]
    fn engine_update_rule_replaces() {
        let mut engine = AutomationEngine::new();
        let r1 = AutomationRule {
            id: "r1".into(),
            name: "Old".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        engine.add_rule(r1);
        let r1_updated = AutomationRule {
            name: "New".into(),
            enabled: false,
            ..engine.get_rule("r1").unwrap().clone()
        };
        engine.update_rule(r1_updated);
        let r = engine.get_rule("r1").unwrap();
        assert_eq!(r.name, "New");
        assert!(!r.enabled);
    }

    #[test]
    fn engine_load_rules_replaces_all() {
        let mut engine = AutomationEngine::new();
        engine.add_rule(AutomationRule {
            id: "old".into(),
            name: "Old".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        });
        let new_rules = vec![
            AutomationRule {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                trigger: TriggerType::DeviceConnect {
                    device_id: "*".into(),
                },
                action: ActionType::SendNotification {
                    title: "t".into(),
                    body: "b".into(),
                },
                trusted_source_only: false,
            },
            AutomationRule {
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                trigger: TriggerType::DeviceConnect {
                    device_id: "*".into(),
                },
                action: ActionType::SendNotification {
                    title: "t".into(),
                    body: "b".into(),
                },
                trusted_source_only: false,
            },
        ];

        engine.load_rules(new_rules);
        assert_eq!(engine.get_rules().len(), 2);
        assert!(engine.get_rule("old").is_none());
    }

    // --- Validation edge cases ---

    #[test]
    fn validate_rejects_empty_id() {
        let rule = AutomationRule {
            id: "".into(),
            name: "Valid".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_err());
    }

    #[test]
    fn validate_rejects_whitespace_only_id() {
        let rule = AutomationRule {
            id: "   ".into(),
            name: "Valid".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_err());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_err());
    }

    #[test]
    fn validate_rejects_negative_battery() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "neg".into(),
            enabled: true,
            trigger: TriggerType::BatteryLevel {
                below: -1,
                device_id: None,
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_err());
    }

    #[test]
    fn validate_rejects_app_package_with_wildcard() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "wild".into(),
            enabled: true,
            trigger: TriggerType::AppOpen {
                app_package: "com.*".into(),
                device_id: None,
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_err());
    }

    #[test]
    fn validate_accepts_valid_rule() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "Good".into(),
            enabled: true,
            trigger: TriggerType::Time {
                time: "08:30".into(),
            },
            action: ActionType::OpenUrl {
                url: "https://example.com".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_ok());
    }

    #[test]
    fn validate_accepts_zero_battery() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "zero".into(),
            enabled: true,
            trigger: TriggerType::BatteryLevel {
                below: 0,
                device_id: None,
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_ok());
    }

    #[test]
    fn validate_accepts_100_battery() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "full".into(),
            enabled: true,
            trigger: TriggerType::BatteryLevel {
                below: 100,
                device_id: None,
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(validate_rule(&rule).is_ok());
    }

    // --- is_valid_hhmm / parse_hhmm ---

    #[test]
    fn valid_hhmm_cases() {
        assert!(is_valid_hhmm("00:00"));
        assert!(is_valid_hhmm("23:59"));
        assert!(is_valid_hhmm("12:30"));
    }

    #[test]
    fn invalid_hhmm_cases() {
        assert!(!is_valid_hhmm("25:00"));
        assert!(!is_valid_hhmm("12:60"));
        assert!(!is_valid_hhmm("abc"));
        assert!(!is_valid_hhmm(""));
        assert!(!is_valid_hhmm("12"));
        assert!(!is_valid_hhmm("12:30:00"));
    }

    #[test]
    fn parse_hhmm_round_trip() {
        assert_eq!(parse_hhmm("12:30"), Some((12, 30)));
        assert_eq!(parse_hhmm("00:00"), Some((0, 0)));
        assert_eq!(parse_hhmm("bad"), None);
    }

    // --- trigger_tag / action_tag more cases ---

    #[test]
    fn all_trigger_tags() {
        assert_eq!(
            trigger_tag(&TriggerType::DeviceConnect {
                device_id: "*".into()
            }),
            "device_connect"
        );
        assert_eq!(
            trigger_tag(&TriggerType::DeviceDisconnect {
                device_id: "*".into()
            }),
            "device_disconnect"
        );
        assert_eq!(
            trigger_tag(&TriggerType::Time {
                time: "09:00".into()
            }),
            "time"
        );
        assert_eq!(
            trigger_tag(&TriggerType::BatteryLevel {
                below: 20,
                device_id: None
            }),
            "battery_level"
        );
        assert_eq!(
            trigger_tag(&TriggerType::WiFiChange { ssid: "x".into() }),
            "wifi_change"
        );
        assert_eq!(
            trigger_tag(&TriggerType::AppOpen {
                app_package: "x".into(),
                device_id: None
            }),
            "app_open"
        );
        assert_eq!(
            trigger_tag(&TriggerType::AudioDeviceConnect {
                device_id: "x".into()
            }),
            "audio_device_connect"
        );
        assert_eq!(
            trigger_tag(&TriggerType::AudioDeviceDisconnect {
                device_id: "x".into()
            }),
            "audio_device_disconnect"
        );
    }

    #[test]
    fn all_action_tags() {
        assert_eq!(
            action_tag(&ActionType::SendNotification {
                title: "t".into(),
                body: "b".into()
            }),
            "send_notification"
        );
        assert_eq!(
            action_tag(&ActionType::SetPhoneProfile {
                profile: "silent".into()
            }),
            "set_phone_profile"
        );
        assert_eq!(
            action_tag(&ActionType::RouteAudio {
                device_id: "d".into()
            }),
            "route_audio"
        );
        assert_eq!(
            action_tag(&ActionType::RunShellCommand {
                command: "ls".into()
            }),
            "run_shell_command"
        );
        assert_eq!(
            action_tag(&ActionType::ToggleWiFi { enabled: true }),
            "toggle_wifi"
        );
        assert_eq!(
            action_tag(&ActionType::ToggleBluetooth { enabled: true }),
            "toggle_bluetooth"
        );
        assert_eq!(
            action_tag(&ActionType::OpenUrl { url: "x".into() }),
            "open_url"
        );
        assert_eq!(
            action_tag(&ActionType::OpenApp {
                app_package: "x".into()
            }),
            "open_app"
        );
        assert_eq!(
            action_tag(&ActionType::SetWindowState {
                state: "minimize".into()
            }),
            "set_window_state"
        );
    }

    // --- is_desktop_executable ---

    #[test]
    fn desktop_executable_actions() {
        assert!(is_desktop_executable(&ActionType::SendNotification {
            title: "t".into(),
            body: "b".into()
        }));
        assert!(is_desktop_executable(&ActionType::RouteAudio {
            device_id: "d".into()
        }));
        assert!(is_desktop_executable(&ActionType::RunShellCommand {
            command: "ls".into()
        }));
        assert!(is_desktop_executable(&ActionType::ToggleWiFi {
            enabled: true
        }));
        assert!(is_desktop_executable(&ActionType::ToggleBluetooth {
            enabled: true
        }));
        assert!(is_desktop_executable(&ActionType::OpenUrl {
            url: "x".into()
        }));
        assert!(is_desktop_executable(&ActionType::SetWindowState {
            state: "min".into()
        }));
    }

    #[test]
    fn non_desktop_actions() {
        assert!(!is_desktop_executable(&ActionType::SetPhoneProfile {
            profile: "silent".into()
        }));
        assert!(!is_desktop_executable(&ActionType::OpenApp {
            app_package: "x".into()
        }));
    }

    // --- evaluate_trigger ---

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_device_connect_specific() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::DeviceConnect {
            device_id: "d1".into(),
        };
        let ctx_match = TriggerContext {
            event: TriggerEvent::DeviceConnect,
            source_device_id: "d1".into(),
            ..Default::default()
        };
        let ctx_no_match = TriggerContext {
            event: TriggerEvent::DeviceConnect,
            source_device_id: "d2".into(),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx_match).await);
        assert!(!engine.evaluate_trigger(&trigger, &ctx_no_match).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_device_connect_wildcard() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::DeviceConnect {
            device_id: "*".into(),
        };
        let ctx = TriggerContext {
            event: TriggerEvent::DeviceConnect,
            source_device_id: "anything".into(),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_device_disconnect() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::DeviceDisconnect {
            device_id: "d1".into(),
        };
        let ctx = TriggerContext {
            event: TriggerEvent::DeviceDisconnect,
            source_device_id: "d1".into(),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx).await);
        let wrong_event = TriggerContext {
            event: TriggerEvent::DeviceConnect,
            source_device_id: "d1".into(),
            ..Default::default()
        };
        assert!(!engine.evaluate_trigger(&trigger, &wrong_event).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_battery_level_trigger() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::BatteryLevel {
            below: 20,
            device_id: Some("d1".into()),
        };
        let ctx_below = TriggerContext {
            event: TriggerEvent::BatteryUpdate,
            source_device_id: "d1".into(),
            battery_level: Some(15),
            ..Default::default()
        };
        let ctx_above = TriggerContext {
            event: TriggerEvent::BatteryUpdate,
            source_device_id: "d1".into(),
            battery_level: Some(50),
            ..Default::default()
        };
        let ctx_wrong_device = TriggerContext {
            event: TriggerEvent::BatteryUpdate,
            source_device_id: "d2".into(),
            battery_level: Some(10),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx_below).await);
        assert!(!engine.evaluate_trigger(&trigger, &ctx_above).await);
        assert!(!engine.evaluate_trigger(&trigger, &ctx_wrong_device).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_battery_level_wildcard_device() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::BatteryLevel {
            below: 30,
            device_id: Some("*".into()),
        };
        let ctx = TriggerContext {
            event: TriggerEvent::BatteryUpdate,
            source_device_id: "any".into(),
            battery_level: Some(20),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_wifi_change_trigger() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::WiFiChange {
            ssid: "Office".into(),
        };
        let ctx_match = TriggerContext {
            event: TriggerEvent::WiFiChange,
            wifi_ssid: Some("Office".into()),
            ..Default::default()
        };
        let ctx_no_match = TriggerContext {
            event: TriggerEvent::WiFiChange,
            wifi_ssid: Some("Home".into()),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx_match).await);
        assert!(!engine.evaluate_trigger(&trigger, &ctx_no_match).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_app_open_trigger() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::AppOpen {
            app_package: "com.spotify".into(),
            device_id: Some("d1".into()),
        };
        let ctx_match = TriggerContext {
            event: TriggerEvent::AppOpen,
            source_device_id: "d1".into(),
            app_package: Some("com.spotify".into()),
            ..Default::default()
        };
        let ctx_wrong_app = TriggerContext {
            event: TriggerEvent::AppOpen,
            source_device_id: "d1".into(),
            app_package: Some("com.other".into()),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx_match).await);
        assert!(!engine.evaluate_trigger(&trigger, &ctx_wrong_app).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_audio_device_connect() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::AudioDeviceConnect {
            device_id: "headphones".into(),
        };
        let ctx = TriggerContext {
            event: TriggerEvent::AudioDeviceConnect,
            source_device_id: "headphones".into(),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx).await);
        let ctx_wrong = TriggerContext {
            event: TriggerEvent::AudioDeviceDisconnect,
            source_device_id: "headphones".into(),
            ..Default::default()
        };
        assert!(!engine.evaluate_trigger(&trigger, &ctx_wrong).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluate_audio_device_disconnect() {
        let engine = AutomationEngine::new();
        let trigger = TriggerType::AudioDeviceDisconnect {
            device_id: "speaker".into(),
        };
        let ctx = TriggerContext {
            event: TriggerEvent::AudioDeviceDisconnect,
            source_device_id: "speaker".into(),
            ..Default::default()
        };
        assert!(engine.evaluate_trigger(&trigger, &ctx).await);
    }

    // --- re_evaluate_rule more cases ---

    #[test]
    fn re_evaluate_wildcard_match() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "wild".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(re_evaluate_rule(
            &rule,
            "device_connect",
            "any_device",
            None
        ));
    }

    #[test]
    fn re_evaluate_battery_level() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "batt".into(),
            enabled: true,
            trigger: TriggerType::BatteryLevel {
                below: 25,
                device_id: Some("d1".into()),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(re_evaluate_rule(&rule, "battery_level", "d1", Some(20)));
        assert!(!re_evaluate_rule(&rule, "battery_level", "d1", Some(30)));
        assert!(!re_evaluate_rule(&rule, "battery_level", "d2", Some(20)));
    }

    #[test]
    fn re_evaluate_disabled_rule_never_fires() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "off".into(),
            enabled: false,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(!re_evaluate_rule(&rule, "device_connect", "d1", None));
    }

    #[test]
    fn re_evaluate_wrong_event_tag() {
        let rule = AutomationRule {
            id: "r1".into(),
            name: "mismatch".into(),
            enabled: true,
            trigger: TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            trusted_source_only: false,
        };
        assert!(!re_evaluate_rule(&rule, "device_disconnect", "d1", None));
    }

    // --- parse_rule_packet edge cases ---

    #[test]
    fn parse_rule_packet_with_nested_rule_key() {
        let val = serde_json::json!({
            "rule": {
                "id": "r1",
                "name": "nested",
                "enabled": true,
                "trigger": {"type": "time", "time": "07:00"},
                "action": {"type": "send_notification", "title": "Wake", "body": "Up"}
            }
        });
        let rule = parse_rule_packet(&val).unwrap();
        assert_eq!(rule.id, "r1");
        assert_eq!(rule.name, "nested");
    }

    #[test]
    fn parse_rule_packet_missing_trigger() {
        let val = serde_json::json!({
            "id": "r1",
            "name": "bad",
            "action": {"type": "send_notification", "title": "t", "body": "b"}
        });
        assert!(parse_rule_packet(&val).is_err());
    }

    #[test]
    fn parse_rule_packet_missing_action() {
        let val = serde_json::json!({
            "id": "r1",
            "name": "bad",
            "trigger": {"type": "time", "time": "10:00"}
        });
        assert!(parse_rule_packet(&val).is_err());
    }

    // --- execute_action ---

    // ── W3.1: an action that did nothing must not report `success: true` ────
    //
    // `SendNotification`, `RouteAudio`, `ToggleWiFi`, `ToggleBluetooth` and
    // `OpenUrl` each used to `info!()` and return `success: true` without
    // touching the machine at all. A rule therefore wrote `success = 1` to
    // `automation_logs` and the UI showed green for something that never
    // happened. There is no notification emitter, no Wi-Fi/Bluetooth control
    // and no URL opener in this crate, so the honest report is a failure.

    #[test]
    fn execute_send_notification_reports_not_executed() {
        let action = ActionType::SendNotification {
            title: "Test".into(),
            body: "Hello".into(),
        };
        let log = execute_action(&action);
        assert!(!log.success, "nothing emitted a notification");
        assert_eq!(log.trigger_type, "send_notification");
        assert!(log.message.unwrap().contains("Test"));
    }

    #[test]
    fn execute_toggle_wifi_reports_not_executed() {
        let log = execute_action(&ActionType::ToggleWiFi { enabled: true });
        assert!(!log.success, "no Wi-Fi control exists in the desktop crate");
        assert_eq!(log.trigger_type, "toggle_wifi");
    }

    #[test]
    fn execute_toggle_bluetooth_reports_not_executed() {
        let log = execute_action(&ActionType::ToggleBluetooth { enabled: false });
        assert!(
            !log.success,
            "no Bluetooth control exists in the desktop crate"
        );
        assert_eq!(log.trigger_type, "toggle_bluetooth");
    }

    #[test]
    fn execute_route_audio_reports_not_executed() {
        let log = execute_action(&ActionType::RouteAudio {
            device_id: "d1".into(),
        });
        assert!(!log.success, "the audio stream is never touched");
        assert_eq!(log.trigger_type, "route_audio");
    }

    #[test]
    fn execute_open_url_reports_not_executed() {
        let log = execute_action(&ActionType::OpenUrl {
            url: "https://example.com".into(),
        });
        assert!(!log.success, "no URL opener is a dependency of this crate");
        assert_eq!(log.trigger_type, "open_url");
    }

    /// Every action the desktop hub cannot perform must report failure, and must
    /// say in the log *why*. `RunShellCommand` is excluded because it really
    /// does execute, and reports the process's own exit status.
    #[test]
    fn every_action_without_an_implementation_reports_failure() {
        let unimplemented = vec![
            ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            ActionType::SetPhoneProfile {
                profile: "silent".into(),
            },
            ActionType::RouteAudio {
                device_id: "d1".into(),
            },
            ActionType::ToggleWiFi { enabled: true },
            ActionType::ToggleBluetooth { enabled: true },
            ActionType::OpenUrl {
                url: "https://example.com".into(),
            },
            ActionType::OpenApp {
                app_package: "com.example".into(),
            },
            ActionType::SetWindowState {
                state: "minimize".into(),
            },
        ];

        for action in unimplemented {
            let tag = action_tag(&action);
            let log = execute_action(&action);
            assert!(
                !log.success,
                "{tag} reports success, but the desktop hub never performs it"
            );
            assert_eq!(log.trigger_type, tag);
            let message = log.message.unwrap_or_default();
            assert!(!message.is_empty(), "{tag} must explain itself");
            assert!(
                message.to_lowercase().contains("not executed"),
                "{tag} must record that nothing was executed, got: {message}"
            );
        }
    }

    /// The one action the hub really does perform reports the process's own
    /// exit status, in both directions — so the failure above is not just a
    /// blanket `false`.
    #[test]
    fn run_shell_command_reports_the_real_exit_status() {
        let ok = execute_action(&ActionType::RunShellCommand {
            command: "echo hello".into(),
        });
        assert!(ok.success, "a command that exits 0 must report success");

        let failed = execute_action(&ActionType::RunShellCommand {
            command: "exit 1".into(),
        });
        assert!(
            !failed.success,
            "a command that exits 1 must report failure"
        );
        assert_eq!(failed.trigger_type, "run_shell_command");
    }

    #[test]
    fn execute_set_phone_profile_returns_not_executed() {
        let log = execute_action(&ActionType::SetPhoneProfile {
            profile: "silent".into(),
        });
        assert!(!log.success);
        assert!(log.message.unwrap().contains("phone-only"));
    }

    #[test]
    fn execute_open_app_returns_not_executed() {
        let log = execute_action(&ActionType::OpenApp {
            app_package: "com.test".into(),
        });
        assert!(!log.success);
        assert!(log.message.unwrap().contains("phone/tablet only"));
    }

    #[test]
    fn execute_set_window_state_invalid() {
        let log = execute_action(&ActionType::SetWindowState {
            state: "invalid_state".into(),
        });
        assert!(!log.success);
        assert!(log.message.unwrap().contains("Invalid window state"));
    }

    #[test]
    fn execute_set_window_state_valid_requires_context() {
        let log = execute_action(&ActionType::SetWindowState {
            state: "minimize".into(),
        });
        assert!(!log.success);
        assert!(
            log.message
                .unwrap()
                .contains("requires desktop window context")
        );
    }

    // --- TriggerContext Default ---

    #[test]
    fn trigger_context_default() {
        let ctx = TriggerContext::default();
        assert_eq!(ctx.event, TriggerEvent::DeviceConnect);
        assert!(ctx.source_device_id.is_empty());
        assert!(ctx.battery_level.is_none());
        assert!(ctx.wifi_ssid.is_none());
        assert!(ctx.app_package.is_none());
    }

    // --- TriggerType / ActionType Clone and Debug ---

    #[test]
    fn trigger_type_clone_debug() {
        let t = TriggerType::Time {
            time: "10:00".into(),
        };
        let t2 = t.clone();
        assert_eq!(t, t2);
        let _ = format!("{:?}", t); // debug should not panic
    }

    #[test]
    fn action_type_clone_debug() {
        let a = ActionType::OpenUrl { url: "x".into() };
        let a2 = a.clone();
        assert_eq!(a, a2);
        let _ = format!("{:?}", a);
    }

    // --- contains_shell_metacharacters unit tests ---

    #[test]
    fn metacharacter_detection_semicolon() {
        assert!(super::contains_shell_metacharacters("ls; rm"));
        assert!(super::contains_shell_metacharacters("echo hello;whoami"));
    }

    #[test]
    fn metacharacter_detection_pipe() {
        assert!(super::contains_shell_metacharacters("ls | grep x"));
        assert!(super::contains_shell_metacharacters("cat /etc/passwd|head"));
    }

    #[test]
    fn metacharacter_detection_ampersand() {
        assert!(super::contains_shell_metacharacters("cmd1 && cmd2"));
        assert!(super::contains_shell_metacharacters("cmd1 || cmd2"));
        assert!(super::contains_shell_metacharacters("sleep 10 &"));
    }

    #[test]
    fn metacharacter_detection_dollar_and_backtick() {
        assert!(super::contains_shell_metacharacters("echo $USER"));
        assert!(super::contains_shell_metacharacters("echo $(whoami)"));
        assert!(super::contains_shell_metacharacters("echo `whoami`"));
    }

    #[test]
    fn metacharacter_detection_redirection() {
        assert!(super::contains_shell_metacharacters("echo x > /tmp/f"));
        assert!(super::contains_shell_metacharacters("echo x >> /tmp/f"));
        assert!(super::contains_shell_metacharacters("cat < /etc/passwd"));
    }

    #[test]
    fn metacharacter_detection_newline() {
        assert!(super::contains_shell_metacharacters("ls\nrm -rf /"));
    }

    #[test]
    fn metacharacter_clean_commands_pass() {
        assert!(!super::contains_shell_metacharacters("ls -la /tmp"));
        assert!(!super::contains_shell_metacharacters("echo hello world"));
        assert!(!super::contains_shell_metacharacters("cat /etc/hostname"));
    }

    // --- extract_executable unit tests ---

    #[test]
    fn extract_executable_simple() {
        assert_eq!(super::extract_executable("ls"), Some("ls".to_string()));
        assert_eq!(super::extract_executable("ls -la"), Some("ls".to_string()));
        assert_eq!(
            super::extract_executable("  echo  hello"),
            Some("echo".to_string())
        );
    }

    #[test]
    fn extract_executable_double_quoted() {
        assert_eq!(
            super::extract_executable("\"ls\" -la"),
            Some("ls".to_string())
        );
        assert_eq!(
            super::extract_executable("\"echo\" hello"),
            Some("echo".to_string())
        );
    }

    #[test]
    fn extract_executable_single_quoted() {
        assert_eq!(
            super::extract_executable("'ls' -la"),
            Some("ls".to_string())
        );
    }

    #[test]
    fn extract_executable_escaped_chars() {
        assert_eq!(
            super::extract_executable("ls\\ -la"),
            Some("ls -la".to_string())
        );
    }

    #[test]
    fn extract_executable_empty() {
        assert_eq!(super::extract_executable(""), None);
        assert_eq!(super::extract_executable("   "), None);
    }

    // --- unquote / unescape unit tests ---

    #[test]
    fn unquote_double() {
        assert_eq!(super::unquote("\"hello world\"", '"'), "hello world");
    }

    #[test]
    fn unquote_single() {
        assert_eq!(super::unquote("'hello world'", '\''), "hello world");
    }

    #[test]
    fn unescape_removes_backslashes() {
        assert_eq!(super::unescape(r#"hello\ world"#), "hello world");
        assert_eq!(super::unescape(r#"a\"b"#), "a\"b");
        assert_eq!(super::unescape(r#"a\\b"#), "a\\b");
    }

    // --- Integration: execute_action_with_allowlist with token matching ---

    #[test]
    fn execute_action_blocks_command_with_semicolon() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        let action = super::ActionType::RunShellCommand {
            command: "ls; rm -rf /".to_string(),
        };
        let log = super::execute_action_with_allowlist(&action, Some(&list));
        assert!(!log.success);
        assert!(log.message.as_deref().unwrap_or("").contains("Blocked"));
    }

    #[test]
    fn execute_action_blocks_command_with_pipe() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        let action = super::ActionType::RunShellCommand {
            command: "ls | grep etc".to_string(),
        };
        let log = super::execute_action_with_allowlist(&action, Some(&list));
        assert!(!log.success);
        assert!(log.message.as_deref().unwrap_or("").contains("Blocked"));
    }

    #[test]
    fn execute_action_allows_clean_command() {
        let list = super::CommandAllowlist::new(vec!["ls".to_string()]);
        let action = super::ActionType::RunShellCommand {
            command: "ls -la /tmp".to_string(),
        };
        let log = super::execute_action_with_allowlist(&action, Some(&list));
        assert!(!log.message.as_deref().unwrap_or("").contains("Blocked"));
    }
}
