use std::collections::HashMap;
use log::info;

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

impl SyncEngine {
    pub fn new() -> Self {
        SyncEngine {
            connected_clients: HashMap::new(),
        }
    }

    pub fn add_client(&mut self, client: ConnectedClient) {
        info!("Client connected: {} ({})", client.device_name, client.device_id);
        self.connected_clients.insert(client.device_id.clone(), client);
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
}
