use log::info;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ConnectedClient {
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub shared_secret: String,
    pub last_heartbeat: i64,
    pub battery_level: Option<i32>,
}

pub struct SyncEngine {
    pub connected_clients: HashMap<String, ConnectedClient>,
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncEngine {
    pub fn new() -> Self {
        SyncEngine {
            connected_clients: HashMap::new(),
        }
    }

    /// Register a paired device.
    ///
    /// The map is keyed by **stable device id**, never by the per-connection
    /// UUID: several connections can exist for one device over its lifetime,
    /// and every lookup below (`get_client`, `update_heartbeat`,
    /// `remove_client`) is given a device id. Keeping that invariant in one
    /// place is what stopped `handle_status_update` from silently no-op'ing.
    pub fn add_client(&mut self, client: ConnectedClient) {
        info!(
            "Client connected: {} ({})",
            client.device_name, client.device_id
        );
        self.connected_clients
            .insert(client.device_id.clone(), client);
    }

    pub fn remove_client(&mut self, device_id: &str) {
        info!("Client disconnected: {}", device_id);
        self.connected_clients.remove(device_id);
    }

    pub fn get_client(&self, device_id: &str) -> Option<&ConnectedClient> {
        self.connected_clients.get(device_id)
    }

    pub fn get_all_client_ids(&self) -> Vec<String> {
        self.connected_clients.keys().cloned().collect()
    }

    pub fn get_connected_count(&self) -> usize {
        self.connected_clients.len()
    }

    pub fn update_heartbeat(&mut self, device_id: &str, timestamp: i64) {
        if let Some(client) = self.connected_clients.get_mut(device_id) {
            client.last_heartbeat = timestamp;
        }
    }

    /// Record a heartbeat and, optionally, a battery level for a paired device.
    ///
    /// Added so `handle_status_update` no longer has to reach into
    /// `connected_clients` directly (its `get_mut(connection_id)` lookup never
    /// matched, because the map is keyed by device id — so every battery
    /// update was dropped on the floor).
    ///
    /// `battery` is clamped to `0..=100`; a peer reporting `-1` or `900` must
    /// not be able to write an impossible state into the UI.
    ///
    /// Returns the stored device id when the device was found, so the caller
    /// can attribute the automation trigger to a real device.
    pub fn apply_status(
        &mut self,
        device_id: &str,
        timestamp: i64,
        battery: Option<i64>,
    ) -> Option<String> {
        let client = self.connected_clients.get_mut(device_id)?;
        client.last_heartbeat = timestamp;
        if let Some(b) = battery {
            client.battery_level = Some(b.clamp(0, 100) as i32);
        }
        Some(client.device_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: &str) -> ConnectedClient {
        ConnectedClient {
            device_id: id.to_string(),
            device_name: "Peer".to_string(),
            device_type: "phone".to_string(),
            shared_secret: "00".repeat(32),
            last_heartbeat: 0,
            battery_level: None,
        }
    }

    #[test]
    fn clients_are_keyed_by_device_id() {
        let mut engine = SyncEngine::new();
        engine.add_client(client("dev_a"));
        assert!(engine.get_client("dev_a").is_some());
        assert!(
            engine.get_client("ws-uuid-1234").is_none(),
            "a connection id must never resolve to a client"
        );
    }

    #[test]
    fn apply_status_updates_the_matching_device() {
        let mut engine = SyncEngine::new();
        engine.add_client(client("dev_a"));
        assert_eq!(
            engine.apply_status("dev_a", 1_700_000_500, Some(42)),
            Some("dev_a".to_string())
        );
        let c = engine.get_client("dev_a").unwrap();
        assert_eq!(c.battery_level, Some(42));
        assert_eq!(c.last_heartbeat, 1_700_000_500);
    }

    #[test]
    fn apply_status_clamps_an_impossible_battery() {
        let mut engine = SyncEngine::new();
        engine.add_client(client("dev_a"));
        engine.apply_status("dev_a", 1, Some(9_999));
        assert_eq!(engine.get_client("dev_a").unwrap().battery_level, Some(100));
        engine.apply_status("dev_a", 2, Some(-5));
        assert_eq!(engine.get_client("dev_a").unwrap().battery_level, Some(0));
    }

    #[test]
    fn apply_status_reports_an_unknown_device() {
        let mut engine = SyncEngine::new();
        engine.add_client(client("dev_a"));
        assert_eq!(engine.apply_status("dev_ghost", 1, Some(50)), None);
    }

    #[test]
    fn apply_status_without_battery_leaves_the_level_alone() {
        let mut engine = SyncEngine::new();
        engine.add_client(client("dev_a"));
        engine.apply_status("dev_a", 1, Some(77));
        engine.apply_status("dev_a", 2, None);
        assert_eq!(engine.get_client("dev_a").unwrap().battery_level, Some(77));
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(SyncEngine::default().get_connected_count(), 0);
    }
}
