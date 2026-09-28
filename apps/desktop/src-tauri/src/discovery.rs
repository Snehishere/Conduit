use log::{error, info, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use tauri::Emitter;

// Use crate-level constants so port/service type are always in sync.
const SERVICE_TYPE: &str = crate::MDNS_SERVICE_TYPE;
const SERVICE_DOMAIN: &str = "local.";

/// The TXT properties advertised alongside the mDNS service.
///
/// `ws_port` / `wss_port` are the *authoritative* source of the peer's ports:
/// the SRV port is the plaintext listener, but a client that needs TLS (which
/// is every mobile client) must read `wss_port` from here.
fn advertised_txt_properties(device_id: &str) -> HashMap<String, String> {
    HashMap::from([
        ("device_id".to_string(), device_id.to_string()),
        ("device_type".to_string(), "desktop".to_string()),
        ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
        ("ws_port".to_string(), crate::WS_PORT.to_string()),
        ("wss_port".to_string(), crate::WSS_PORT.to_string()),
    ])
}

pub struct DiscoveryService {
    device_id: String,
    device_name: String,
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
    pub fn new(device_id: String, device_name: String) -> Self {
        DiscoveryService {
            device_id,
            device_name,
            mdns: None,
            app_handle: None,
        }
    }

    pub fn set_app_handle(&mut self, handle: tauri::AppHandle) {
        self.app_handle = Some(handle);
    }

    pub async fn start(&mut self) {
        let device_id = self.device_id.clone();
        let device_name = self.device_name.clone();
        // mDNS daemon creation, service-info construction, and registration
        // perform blocking I/O — run them on the blocking pool.
        let setup = tokio::task::spawn_blocking(move || -> Option<ServiceDaemon> {
            let my_addr = match get_local_ipv4() {
                Some(addr) => addr,
                None => {
                    warn!(
                        "No IPv4 address found on any interface — mDNS discovery will not work on IPv6-only networks"
                    );
                    warn!("Discovery will still work via WebSocket announcements");
                    return None;
                }
            };

            match ServiceDaemon::new() {
                Ok(mdns) => {


                    let service_info = match ServiceInfo::new(
                        &format!("{}.{}", SERVICE_TYPE, SERVICE_DOMAIN),
                        &device_name,
                        &format!("{}.{}", gethostname::gethostname().to_string_lossy(), SERVICE_DOMAIN),
                        &my_addr,
                        // The SRV record points at the plaintext listener. The
                        // TLS port is advertised in the TXT record below.
                        crate::WS_PORT,
                        advertised_txt_properties(&device_id),
                    ) {
                        Ok(info) => info,
                        Err(e) => {
                            error!("Failed to create mDNS service info: {}", e);
                            return None;
                        }
                    };

                    match mdns.register(service_info) {
                        Ok(_) => {
                            info!(
                                "mDNS service registered on {}:{} (TLS {}; device: {})",
                                my_addr,
                                crate::WS_PORT,
                                crate::WSS_PORT,
                                &device_id[..8.min(device_id.len())]
                            );
                        }
                        Err(e) => {
                            error!("Failed to register mDNS service: {}", e);
                        }
                    }

                    Some(mdns)
                }
                Err(e) => {
                    error!("Failed to create mDNS daemon: {}", e);
                    None
                }
            }
        })
        .await;

        let Some(mdns) = setup.ok().flatten() else {
            return;
        };
        self.mdns = Some(mdns.clone());

        let app_handle = self.app_handle.clone();
        let my_id = self.device_id.clone();
        tokio::spawn(async move {
            Self::browse_devices(mdns, app_handle, my_id).await;
        });
    }

    async fn browse_devices(
        mdns: ServiceDaemon,
        app_handle: Option<tauri::AppHandle>,
        my_device_id: String,
    ) {
        let service_name = format!("{}.{}", SERVICE_TYPE, SERVICE_DOMAIN);
        // `ServiceDaemon::browse` performs blocking socket setup — run it on
        // the blocking pool; the event loop below stays async via recv_async.
        let browse_result = tokio::task::spawn_blocking(move || mdns.browse(&service_name)).await;
        match browse_result {
            Ok(Ok(rx)) => {
                info!("Browsing for Conduit devices...");
                while let Ok(event) = rx.recv_async().await {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let props = info.get_properties();
                            let device_id = props
                                .get_property_val_str("device_id")
                                .unwrap_or("")
                                .to_string();
                            let device_type = props
                                .get_property_val_str("device_type")
                                .unwrap_or("unknown")
                                .to_string();
                            let version = props
                                .get_property_val_str("version")
                                .unwrap_or("")
                                .to_string();

                            if device_id != my_device_id && !device_id.is_empty() {
                                let discovered = DiscoveredDevice {
                                    device_id,
                                    name: info.get_hostname().to_string(),
                                    address: info
                                        .get_addresses()
                                        .iter()
                                        .next()
                                        .map(|a| a.to_string())
                                        .unwrap_or_default(),
                                    port: info.get_port(),
                                    device_type,
                                    version,
                                };
                                info!(
                                    "Discovered device: {} ({})",
                                    discovered.name, discovered.device_id
                                );
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
            Ok(Err(e)) => {
                error!("Failed to browse mDNS: {}", e);
            }
            Err(e) => {
                error!("Failed to browse mDNS task join: {}", e);
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
    if let Ok(ip) = local_ip_address::local_ip()
        && ip.is_ipv4()
    {
        return Some(ip.to_string());
    }
    // Fallback to localhost if no active network interface is found
    // This allows the app to start up and function locally even in airplane mode
    Some("127.0.0.1".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_discovery_service_holds_device_id() {
        let svc = DiscoveryService::new("abc123".to_string(), "Test Device".to_string());
        assert_eq!(svc.device_id, "abc123");
        assert_eq!(svc.device_name, "Test Device");
        assert!(svc.mdns.is_none());
        assert!(svc.app_handle.is_none());
    }

    #[test]
    fn discovered_device_serializes() {
        let d = DiscoveredDevice {
            device_id: "dev1".to_string(),
            name: "Laptop".to_string(),
            address: "192.168.1.5".to_string(),
            port: crate::WS_PORT,
            device_type: "desktop".to_string(),
            version: "0.1.0".to_string(),
        };
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["device_id"], "dev1");
        assert_eq!(json["name"], "Laptop");
        assert_eq!(json["port"], crate::WS_PORT);
        assert_eq!(json["device_type"], "desktop");
        assert_eq!(json["version"], "0.1.0");
    }

    #[test]
    fn discovered_device_clone() {
        let d = DiscoveredDevice {
            device_id: "dev2".to_string(),
            name: "Phone".to_string(),
            address: "10.0.0.1".to_string(),
            port: crate::WS_PORT,
            device_type: "mobile".to_string(),
            version: "1.0.0".to_string(),
        };
        let d2 = d.clone();
        assert_eq!(d.device_id, d2.device_id);
        assert_eq!(d.address, d2.address);
    }

    #[test]
    fn get_local_ipv4_returns_some_on_normal_host() {
        // On most machines with a network connection this returns Some
        let result = get_local_ipv4();
        if let Some(ref ip) = result {
            assert!(!ip.is_empty());
            assert!(ip.parse::<std::net::Ipv4Addr>().is_ok());
        }
    }

    #[test]
    fn stop_without_start_is_noop() {
        let mut svc = DiscoveryService::new("test".to_string(), "Test Device".to_string());
        svc.stop(); // should not panic
    }

    // ── advertised ports ──────────────────────────────────────────────────────

    #[test]
    fn txt_properties_advertise_both_lan_ports() {
        let props = advertised_txt_properties("dev-1234");
        assert_eq!(props.get("ws_port").map(String::as_str), Some("9527"));
        assert_eq!(props.get("wss_port").map(String::as_str), Some("9531"));
    }

    #[test]
    fn txt_wss_port_is_the_port_actually_bound_for_tls() {
        let props = advertised_txt_properties("dev-1234");
        assert_eq!(
            props.get("wss_port").map(String::as_str),
            Some(crate::WSS_PORT.to_string().as_str()),
            "the advertised wss_port must be the port the TLS listener binds"
        );
        assert_eq!(
            props.get("ws_port").map(String::as_str),
            Some(crate::WS_PORT.to_string().as_str()),
            "the advertised ws_port must be the port the plaintext listener binds"
        );
    }

    #[test]
    fn txt_wss_port_differs_from_the_srv_port() {
        // The SRV port is the plaintext listener; if the TXT wss_port equalled
        // it, a client reading either record would be unable to tell the two
        // transports apart — which is the bug that broke mobile pairing.
        let props = advertised_txt_properties("dev-1234");
        assert_ne!(props.get("ws_port"), props.get("wss_port"));
    }

    #[test]
    fn txt_properties_preserve_identity_fields() {
        let props = advertised_txt_properties("dev-abc");
        assert_eq!(props.get("device_id").map(String::as_str), Some("dev-abc"));
        assert_eq!(
            props.get("device_type").map(String::as_str),
            Some("desktop")
        );
        assert_eq!(
            props.get("version").map(String::as_str),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
