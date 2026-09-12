use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::commands::ConduitSettings;
use crate::error::{ConduitError, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub os: String,
    pub public_key: String,
    pub shared_secret: String,
    pub paired_at: i64,
    pub last_seen: i64,
    pub battery: Option<i32>,
    pub signal: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredNotification {
    pub id: String,
    pub device_id: String,
    pub app: String,
    pub title: String,
    pub body: String,
    pub timestamp: i64,
    pub actions: Option<String>,
    pub dismissed: bool,
}

pub struct NotificationParams<'a> {
    pub id: &'a str,
    pub device_id: &'a str,
    pub app: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub timestamp: i64,
    pub actions: Option<&'a str>,
}

pub struct FileTransferParams<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub size: i64,
    pub mime: &'a str,
    pub from_device: &'a str,
    pub to_device: &'a str,
    pub status: &'a str,
    pub chunks_received: i32,
    pub total_chunks: i32,
    pub saved_path: Option<&'a str>,
    pub timestamp: i64,
}

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS devices (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                device_type TEXT NOT NULL,
                os TEXT NOT NULL,
                public_key TEXT NOT NULL,
                shared_secret TEXT NOT NULL,
                paired_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                battery INTEGER,
                signal TEXT,
                status TEXT DEFAULT 'paired'
            );

            CREATE TABLE IF NOT EXISTS notifications (
                id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                app TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                actions TEXT,
                dismissed INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS clipboard_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                mime TEXT NOT NULL,
                source_device TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_transfers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                size INTEGER NOT NULL,
                mime TEXT NOT NULL,
                from_device TEXT NOT NULL,
                to_device TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                chunks_received INTEGER NOT NULL DEFAULT 0,
                total_chunks INTEGER NOT NULL DEFAULT 0,
                saved_path TEXT,
                timestamp INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS automation_rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                trigger_type TEXT NOT NULL,
                trigger_config TEXT NOT NULL,
                action_type TEXT NOT NULL,
                action_config TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_triggered INTEGER
            );

            CREATE TABLE IF NOT EXISTS automation_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                rule_id TEXT NOT NULL,
                trigger_type TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                success INTEGER NOT NULL,
                message TEXT
            );",
        )?;

        Ok(Storage {
            conn: Mutex::new(conn),
        })
    }

    fn get_db_path() -> PathBuf {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("conduit");
        data_dir.join("conduit.db")
    }

    pub fn save_device(&self, device: &StoredDevice) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO devices (id, name, device_type, os, public_key, shared_secret, paired_at, last_seen, battery, signal, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                device.id,
                device.name,
                device.device_type,
                device.os,
                device.public_key,
                device.shared_secret,
                device.paired_at,
                device.last_seen,
                device.battery,
                device.signal,
                device.status,
            ],
        )?;
        Ok(())
    }

    pub fn get_device(&self, id: &str) -> Result<Option<StoredDevice>> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, device_type, os, public_key, shared_secret, paired_at, last_seen, battery, signal, status
             FROM devices WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(StoredDevice {
                id: row.get(0)?,
                name: row.get(1)?,
                device_type: row.get(2)?,
                os: row.get(3)?,
                public_key: row.get(4)?,
                shared_secret: row.get(5)?,
                paired_at: row.get(6)?,
                last_seen: row.get(7)?,
                battery: row.get(8)?,
                signal: row.get(9)?,
                status: row.get(10)?,
            })
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn get_all_devices(&self) -> Result<Vec<StoredDevice>> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, device_type, os, public_key, shared_secret, paired_at, last_seen, battery, signal, status
             FROM devices ORDER BY last_seen DESC",
        )?;

        let devices = stmt
            .query_map([], |row| {
                Ok(StoredDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    device_type: row.get(2)?,
                    os: row.get(3)?,
                    public_key: row.get(4)?,
                    shared_secret: row.get(5)?,
                    paired_at: row.get(6)?,
                    last_seen: row.get(7)?,
                    battery: row.get(8)?,
                    signal: row.get(9)?,
                    status: row.get(10)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(devices)
    }

    pub fn delete_device(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute("DELETE FROM devices WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn save_notification(&self, p: &NotificationParams<'_>) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO notifications (id, device_id, app, title, body, timestamp, actions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![p.id, p.device_id, p.app, p.title, p.body, p.timestamp, p.actions],
        )?;
        Ok(())
    }

    pub fn get_notifications(&self, limit: i64) -> Result<Vec<StoredNotification>> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let mut stmt = conn.prepare(
            "SELECT id, device_id, app, title, body, timestamp, actions, dismissed
             FROM notifications ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let notifications = stmt
            .query_map(params![limit], |row| {
                Ok(StoredNotification {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    app: row.get(2)?,
                    title: row.get(3)?,
                    body: row.get(4)?,
                    timestamp: row.get(5)?,
                    actions: row.get(6)?,
                    dismissed: row.get::<_, i64>(7)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(notifications)
    }

    pub fn dismiss_notification(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "UPDATE notifications SET dismissed = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn save_clipboard(
        &self,
        content: &str,
        mime: &str,
        source_device: &str,
        timestamp: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "INSERT INTO clipboard_history (content, mime, source_device, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![content, mime, source_device, timestamp],
        )?;
        Ok(())
    }

    fn get_setting_value(&self, conn: &Connection, key: &str, default: &str) -> String {
        let result = conn
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .ok()
            .and_then(|mut stmt| {
                let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0)).ok()?;
                rows.next().and_then(|r| r.ok())
            });
        result.unwrap_or_else(|| default.to_string())
    }

    pub fn get_settings(&self) -> Result<ConduitSettings> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let default_apps = serde_json::to_string(&vec![
            "WhatsApp".to_string(),
            "Telegram".to_string(),
            "Slack".to_string(),
            "Discord".to_string(),
        ])
        .unwrap_or_default();

        Ok(ConduitSettings {
            device_name: self.get_setting_value(&conn, "device_name", &gethostname::gethostname().to_string_lossy()),
            max_devices: self.get_setting_value(&conn, "max_devices", "5").parse().unwrap_or(5),
            auto_connect: self.get_setting_value(&conn, "auto_connect", "true") == "true",
            sync_notifications: self.get_setting_value(&conn, "sync_notifications", "true") == "true",
            sync_clipboard: self.get_setting_value(&conn, "sync_clipboard", "true") == "true",
            sync_files: self.get_setting_value(&conn, "sync_files", "true") == "true",
            notification_apps: serde_json::from_str(
                &self.get_setting_value(&conn, "notification_apps", &default_apps),
            )
            .unwrap_or_default(),
        })
    }

    pub fn save_settings(&self, settings: &ConduitSettings) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let upsert = "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)";

        conn.execute(upsert, params!["device_name", settings.device_name])?;
        conn.execute(upsert, params!["max_devices", settings.max_devices.to_string()])?;
        conn.execute(upsert, params!["auto_connect", settings.auto_connect.to_string()])?;
        conn.execute(upsert, params!["sync_notifications", settings.sync_notifications.to_string()])?;
        conn.execute(upsert, params!["sync_clipboard", settings.sync_clipboard.to_string()])?;
        conn.execute(upsert, params!["sync_files", settings.sync_files.to_string()])?;
        conn.execute(
            upsert,
            params![
                "notification_apps",
                serde_json::to_string(&settings.notification_apps).unwrap_or_default()
            ],
        )?;
        Ok(())
    }

    pub fn save_file_transfer(&self, p: &FileTransferParams<'_>) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO file_transfers (id, name, size, mime, from_device, to_device, status, chunks_received, total_chunks, saved_path, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![p.id, p.name, p.size, p.mime, p.from_device, p.to_device, p.status, p.chunks_received, p.total_chunks, p.saved_path, p.timestamp],
        )?;
        Ok(())
    }

    pub fn update_file_transfer_progress(
        &self,
        id: &str,
        status: &str,
        chunks_received: i32,
        saved_path: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "UPDATE file_transfers SET status = ?2, chunks_received = ?3, saved_path = COALESCE(?4, saved_path) WHERE id = ?1",
            params![id, status, chunks_received, saved_path],
        )?;
        Ok(())
    }

    pub fn save_automation_rule(
        &self,
        rule: &crate::automation::AutomationRule,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let trigger_json = serde_json::to_string(&rule.trigger).unwrap_or_default();
        let action_json = serde_json::to_string(&rule.action).unwrap_or_default();
        let trigger_tag = crate::automation::trigger_tag(&rule.trigger);
        let action_tag = crate::automation::action_tag(&rule.action);
        conn.execute(
            "INSERT INTO automation_rules (id, name, enabled, trigger_type, trigger_config, action_type, action_config, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                enabled = excluded.enabled,
                trigger_type = excluded.trigger_type,
                trigger_config = excluded.trigger_config,
                action_type = excluded.action_type,
                action_config = excluded.action_config",
            params![
                rule.id,
                rule.name,
                rule.enabled as i32,
                trigger_tag,
                trigger_json,
                action_tag,
                action_json,
                chrono::Utc::now().timestamp(),
            ],
        )?;
        Ok(())
    }

    pub fn get_all_automation_rules(&self) -> Result<Vec<crate::automation::AutomationRule>> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, enabled, trigger_type, trigger_config, action_type, action_config
             FROM automation_rules ORDER BY created_at DESC",
        )?;

        let rules = stmt
            .query_map([], |row| {
                let trigger_json: String = row.get(4)?;
                let action_json: String = row.get(6)?;
                let trigger: crate::automation::TriggerType = serde_json::from_str(&trigger_json)
                    .unwrap_or(crate::automation::TriggerType::DeviceConnect { device_id: "*".to_string() });
                let action: crate::automation::ActionType = serde_json::from_str(&action_json).unwrap_or(
                    crate::automation::ActionType::SendNotification { title: String::new(), body: String::new() },
                );
                Ok(crate::automation::AutomationRule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    enabled: row.get::<_, i32>(2)? != 0,
                    trigger,
                    action,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rules)
    }

    pub fn delete_automation_rule(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute("DELETE FROM automation_rules WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM automation_logs WHERE rule_id = ?1", params![id])?;
        Ok(())
    }

    pub fn log_automation_execution(
        &self,
        rule_id: &str,
        trigger_type: &str,
        timestamp: i64,
        success: bool,
        message: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        conn.execute(
            "INSERT INTO automation_logs (rule_id, trigger_type, timestamp, success, message)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![rule_id, trigger_type, timestamp, success as i32, message],
        )?;
        Ok(())
    }

    pub fn get_automation_logs(&self, limit: i64) -> Result<Vec<crate::automation::RuleExecutionLog>> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let mut stmt = conn.prepare(
            "SELECT rule_id AS id, trigger_type, timestamp, success, message
             FROM automation_logs ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let logs = stmt
            .query_map(params![limit], |row| {
                Ok(crate::automation::RuleExecutionLog {
                    id: row.get(0)?,
                    trigger_type: row.get(1)?,
                    timestamp: row.get(2)?,
                    success: row.get::<_, i32>(3)? != 0,
                    message: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(logs)
    }

    pub fn get_file_transfers(&self, limit: i64) -> Result<Vec<crate::file_transfer::FileTransferInfo>> {
        let conn = self.conn.lock().map_err(|e| ConduitError::LockPoisoned(format!("Storage lock poisoned: {}", e)))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, size, mime, from_device, to_device, status, chunks_received, total_chunks, saved_path, timestamp
             FROM file_transfers ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let transfers = stmt
            .query_map(params![limit], |row| {
                Ok(crate::file_transfer::FileTransferInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    size: row.get(2)?,
                    mime: row.get(3)?,
                    from_device: row.get(4)?,
                    to_device: row.get(5)?,
                    status: row.get(6)?,
                    chunks_received: row.get(7)?,
                    total_chunks: row.get(8)?,
                    saved_path: row.get(9)?,
                    timestamp: row.get(10)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(transfers)
    }
}
