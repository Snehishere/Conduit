use log::{error, info, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use tauri::Emitter;

const SERVICE_TYPE: &str = "_conduit._tcp";
const SERVICE_DOMAIN: &str = "local.";

pub struct DiscoveryService {
    device_id: String,
    mdns: Option<ServiceDaemon>,
    app_handle: Option<tauri::AppHandle>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub device_type: String,
    pub version: String,
}

impl DiscoveryService {
    pub fn new(device_id: String) -> Self {
        DiscoveryService {
            device_id,
            mdns: None,
            app_handle: None,
        }
    }

    pub fn set_app_handle(&mut self, handle: tauri::AppHandle) {
        self.app_handle = Some(handle);
    }

    pub async fn start(&mut self) {
        let my_addr = match get_local_ipv4() {
            Some(addr) => addr,
            None => {
                warn!("No IPv4 address found on any interface — mDNS discovery will not work on IPv6-only networks");
                warn!("Discovery will still work via WebSocket announcements");
                return;
            }
        };

        match ServiceDaemon::new() {
            Ok(mdns) => {
                self.mdns = Some(mdns.clone());

                let hostname = gethostname::gethostname()
                    .to_string_lossy()
                    .to_string();

                let service_info = ServiceInfo::new(
                    &format!("{}.{}", SERVICE_TYPE, SERVICE_DOMAIN),
                    &format!("conduit-{}", &self.device_id[..8.min(self.device_id.len())]),
                    &format!("{}.{}", hostname, SERVICE_DOMAIN),
                    &my_addr,
                    9527,
                    HashMap::from([
                        ("device_id".to_string(), self.device_id.clone()),
                        ("device_type".to_string(), "desktop".to_string()),
                        ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
                    ]),
                )
                .expect("Failed to create service info");

                match mdns.register(service_info) {
                    Ok(_) => {
                        info!("mDNS service registered on {}:{} (device: {})", my_addr, 9527, &self.device_id[..8.min(self.device_id.len())]);
                    }
                    Err(e) => {
                        error!("Failed to register mDNS service: {}", e);
                    }
                }

                let mdns_clone = mdns.clone();
                let app_handle = self.app_handle.clone();
                let my_id = self.device_id.clone();
                tokio::spawn(async move {
                    Self::browse_devices(mdns_clone, app_handle, my_id).await;
                });
            }
            Err(e) => {
                error!("Failed to create mDNS daemon: {}", e);
            }
        }
    }

    async fn browse_devices(mdns: ServiceDaemon, app_handle: Option<tauri::AppHandle>, my_device_id: String) {
        let service_name = format!("{}.{}", SERVICE_TYPE, SERVICE_DOMAIN);
        match mdns.browse(&service_name) {
            Ok(rx) => {
                info!("Browsing for Conduit devices...");
                while let Ok(event) = rx.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let props = info.get_properties();
                            let device_id = props.get_property_val_str("device_id")
                                .unwrap_or("").to_string();
                            let device_type = props.get_property_val_str("device_type")
                                .unwrap_or("unknown").to_string();
                            let version = props.get_property_val_str("version")
                                .unwrap_or("").to_string();

                            if device_id != my_device_id && !device_id.is_empty() {
                                let discovered = DiscoveredDevice {
                                    device_id,
                                    name: info.get_hostname().to_string(),
                                    address: info.get_addresses().iter().next()
                                        .map(|a| a.to_string())
                                        .unwrap_or_default(),
                                    port: info.get_port(),
                                    device_type,
                                    version,
                                };
                                info!("Discovered device: {} ({})", discovered.name, discovered.device_id);
                                if let Some(ref handle) = app_handle {
                                    let _ = handle.emit("mdns-device-discovered", &discovered);
                                }
                            }
                        }
                        ServiceEvent::ServiceFound(service_type, fullname) => {
                            info!("mDNS service found: {} ({})", service_type, fullname);
                        }
                        ServiceEvent::ServiceRemoved(service_type, fullname) => {
                            info!("mDNS service removed: {} ({})", service_type, fullname);
                            if let Some(ref handle) = app_handle {
                                let _ = handle.emit("mdns-device-removed", fullname);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                error!("Failed to browse mDNS: {}", e);
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(mdns) = self.mdns.take() {
            let _ = mdns.shutdown();
            info!("mDNS service stopped");
        }
    }
}

fn get_local_ipv4() -> Option<String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip();
    if ip.is_ipv4() {
        Some(ip.to_string())
    } else {
        None
    }
}
