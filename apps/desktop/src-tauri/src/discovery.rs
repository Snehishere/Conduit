use log::{error, info, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use tauri::Emitter;

// Use crate-level constants so port/service type are always in sync.
const SERVICE_TYPE: &str = crate::MDNS_SERVICE_TYPE;
const SERVICE_DOMAIN: &str = "local.";

/// How many characters of the device id the registration log line shows.
const LOGGED_ID_CHARS: usize = 8;

/// The first [`LOGGED_ID_CHARS`] characters of an id, cut on a character boundary.
///
/// The id is not generated fresh on every launch — `load_or_create_device_id`
/// reads it verbatim out of `device_id.txt` — so it can be any UTF-8 the user
/// or a sync tool put there. A byte-index slice (`&id[..8.min(id.len())]`)
/// panics the moment that index lands inside a multi-byte character, which would
/// take down the one startup path that must never fail.
fn short_id(device_id: &str) -> &str {
    match device_id.char_indices().nth(LOGGED_ID_CHARS) {
        Some((idx, _)) => &device_id[..idx],
        None => device_id,
    }
}

/// Turn a resolved mDNS service into the payload the frontend consumes, or
/// `None` when the result is not surfaceable.
///
/// This is the whole consumption decision for the mDNS path, kept out of the
/// event loop so it can be exercised without a multicast socket:
///
///  * our own service comes back from the browse loop like any other, and must
///    not be listed as a peer;
///  * a peer that advertises no `device_id` is skipped, because
///    `DiscoveredDevice::device_id` is the identity every consumer keys on —
///    admitting an empty one would collapse every unnamed responder into one
///    unusable entry.
///
/// Nothing here is a trust check and nothing here is advertised: the TXT
/// record written by [`advertised_txt_properties`] is the complete set of
/// properties this service publishes, and it is unchanged.
fn discovered_from_txt(
    my_device_id: &str,
    txt: &HashMap<String, String>,
    hostname: &str,
    address: Option<&str>,
    port: u16,
) -> Option<DiscoveredDevice> {
    let device_id = txt.get("device_id").map(String::as_str).unwrap_or("");
    if device_id.is_empty() || device_id == my_device_id {
        return None;
    }
    Some(DiscoveredDevice {
        device_id: device_id.to_string(),
        name: hostname.to_string(),
        address: address.unwrap_or_default().to_string(),
        port,
        device_type: txt
            .get("device_type")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        version: txt.get("version").cloned().unwrap_or_default(),
    })
}

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
                                short_id(&device_id)
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
                            let txt: HashMap<String, String> =
                                info.get_properties().clone().into_property_map_str();
                            let hostname = info.get_hostname();
                            let address = info.get_addresses().iter().next().map(|a| a.to_string());

                            if let Some(discovered) = discovered_from_txt(
                                &my_device_id,
                                &txt,
                                hostname,
                                address.as_deref(),
                                info.get_port(),
                            ) {
                                info!(
                                    "Discovered device: {} ({})",
                                    discovered.name, discovered.device_id
                                );
                                match &app_handle {
                                    Some(handle) => {
                                        let _ = handle.emit("mdns-device-discovered", &discovered);
                                    }
                                    None => warn!(
                                        "Resolved {} ({}) but no app handle is attached, \
                                         so nothing can receive it — discovery results are \
                                         dropped",
                                        discovered.name, discovered.device_id
                                    ),
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

    // ── short_id: a log line must never be able to panic ──────────────────────

    #[test]
    fn short_id_takes_eight_characters_of_an_ascii_id() {
        assert_eq!(short_id("dev-1234-5678"), "dev-1234");
    }

    #[test]
    fn short_id_returns_short_ids_whole() {
        assert_eq!(short_id("dev"), "dev");
        assert_eq!(short_id(""), "");
        assert_eq!(short_id("12345678"), "12345678");
    }

    #[test]
    fn short_id_never_splits_a_multi_byte_character() {
        // REGRESSION: `&device_id[..8.min(device_id.len())]` sliced by *byte*.
        // This id has 7 ASCII bytes then a 2-byte 'é', so byte index 8 — the
        // one the old expression used — is the second byte of that character
        // and `&id[..8]` panicked. This is reachable: the id comes from
        // `device_id.txt` on disk, not from a freshly generated UUID.
        let id = "aaaaaaaé-bbbb";
        assert_eq!(id.len(), 14, "sanity: the id is longer than 8 bytes");
        assert_eq!(
            id.char_indices().nth(LOGGED_ID_CHARS).map(|(i, _)| i),
            Some(9)
        );
        assert_eq!(short_id(id), "aaaaaaaé");
        assert_eq!(short_id(id).chars().count(), LOGGED_ID_CHARS);
    }

    #[test]
    fn short_id_is_a_prefix_of_its_input_for_every_id() {
        for id in [
            "",
            "a",
            "dev-1234-5678",
            "aaaaaaaé-bbbb",
            "日本語のデバイス",
            "🎛🎛🎛🎛🎛",
            "mixed é文 mixed",
            " exactly-8",
            " exactly-9!",
        ] {
            let prefix = short_id(id);
            assert!(
                id.starts_with(prefix),
                "{prefix:?} is not a prefix of {id:?}"
            );
            assert_eq!(
                prefix.chars().count(),
                LOGGED_ID_CHARS.min(id.chars().count()),
                "wrong length for {id:?}"
            );
            assert!(prefix.len() <= id.len(), "{prefix:?} is longer than {id:?}");
        }
    }

    // ── consumption: resolved mDNS results reach the frontend payload ─────────

    fn txt_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn resolved_peer_becomes_a_payload_the_frontend_can_key_on() {
        let txt = advertised_txt_properties("dev-remote");
        let d = discovered_from_txt("dev-me", &txt, "laptop.local", Some("192.168.1.5"), 9527)
            .expect("a peer advertising a device_id is surfaceable");
        assert_eq!(d.device_id, "dev-remote");
        assert_eq!(d.name, "laptop.local");
        assert_eq!(d.address, "192.168.1.5");
        assert_eq!(d.port, 9527);
        assert_eq!(d.device_type, "desktop");
        assert_eq!(d.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn resolved_peer_payload_serialises_with_the_field_names_the_hook_reads() {
        // useDiscovery.ts reads `device_id`, `name`, `address`, `port`,
        // `device_type`, `version`. If serde ever renames one of these the
        // frontend silently drops the peer, so pin the wire names here.
        let txt = advertised_txt_properties("dev-remote");
        let d =
            discovered_from_txt("dev-me", &txt, "laptop.local", Some("192.168.1.5"), 9527).unwrap();
        let json = serde_json::to_value(&d).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "address",
                "device_id",
                "device_type",
                "name",
                "port",
                "version"
            ]
        );
    }

    #[test]
    fn our_own_service_is_not_reported_back_to_us() {
        let txt = advertised_txt_properties("dev-me");
        assert!(
            discovered_from_txt("dev-me", &txt, "myhost.local", Some("192.168.1.9"), 9527)
                .is_none()
        );
    }

    #[test]
    fn a_peer_that_advertises_no_device_id_is_skipped() {
        for txt in [
            HashMap::new(),
            txt_of(&[("device_type", "desktop"), ("version", "1.0.0")]),
            txt_of(&[("device_id", "")]),
        ] {
            assert!(
                discovered_from_txt("dev-me", &txt, "ghost.local", Some("192.168.1.5"), 9527)
                    .is_none(),
                "a peer with no usable device_id must not enter the device list"
            );
        }
    }

    #[test]
    fn a_peer_with_no_address_still_surfaces_with_an_empty_address() {
        // mDNS resolves the SRV record before the A record on a cold cache.
        // Dropping the peer there loses it for good — the resolved event does
        // not fire again — so the entry is emitted with an empty address and
        // the frontend's existing `payload.address || ''` fallback applies.
        let txt = advertised_txt_properties("dev-remote");
        let d = discovered_from_txt("dev-me", &txt, "laptop.local", None, 9527)
            .expect("an unresolved address must not discard the peer");
        assert_eq!(d.device_id, "dev-remote");
        assert_eq!(d.address, "");
    }

    #[test]
    fn a_peer_with_no_device_type_is_reported_as_unknown() {
        let txt = txt_of(&[("device_id", "dev-remote"), ("version", "1.0.0")]);
        let d =
            discovered_from_txt("dev-me", &txt, "laptop.local", Some("10.0.0.2"), 9527).unwrap();
        assert_eq!(d.device_type, "unknown");
        assert_eq!(d.version, "1.0.0");
    }

    #[test]
    fn every_advertised_property_still_rides_in_the_payload() {
        // Round-trip: the record we publish must be the record we read back,
        // so a peer sees the same identity on both sides of the LAN.
        let txt = advertised_txt_properties("dev-remote");
        let d =
            discovered_from_txt("dev-me", &txt, "laptop.local", Some("10.0.0.2"), 9527).unwrap();
        assert_eq!(d.device_id, txt["device_id"]);
        assert_eq!(d.device_type, txt["device_type"]);
        assert_eq!(d.version, txt["version"]);
    }
}
