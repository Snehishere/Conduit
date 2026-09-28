#![allow(dead_code)]

use log::info;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

use crate::error::ConduitError;

pub fn get_tls_acceptor() -> Result<TlsAcceptor, ConduitError> {
    use keyring::Entry;
    let cert_entry = Entry::new("conduit_app", "tls_cert").unwrap();
    let key_entry = Entry::new("conduit_app", "tls_key").unwrap();

    let (cert_pem, key_pem) = match (cert_entry.get_password(), key_entry.get_password()) {
        (Ok(cert), Ok(key)) => (cert, key),
        _ => {
            let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
            let cert = rcgen::generate_simple_self_signed(subject_alt_names)
                .map_err(|e| ConduitError::Encryption(format!("rcgen error: {}", e)))?;

            let cert_pem = cert.cert.pem();
            let key_pem = cert.signing_key.serialize_pem();

            let _ = cert_entry.set_password(&cert_pem);
            let _ = key_entry.set_password(&key_pem);

            (cert_pem, key_pem)
        }
    };

    let mut cert_reader = std::io::BufReader::new(cert_pem.as_bytes());
    let mut key_reader = std::io::BufReader::new(key_pem.as_bytes());

    let certs_result: Result<Vec<CertificateDer<'static>>, std::io::Error> =
        certs(&mut cert_reader).collect();
    let certs = certs_result?;

    let keys_result: Result<Vec<PrivateKeyDer<'static>>, std::io::Error> =
        pkcs8_private_keys(&mut key_reader)
            .map(|res| res.map(PrivateKeyDer::Pkcs8))
            .collect();
    let mut keys = keys_result?;

    if keys.is_empty() {
        return Err(ConduitError::Encryption(
            "No private keys found".to_string(),
        ));
    }

    // TLS 1.3 only — pin the protocol version; never negotiate down to 1.2 or earlier.
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| ConduitError::Encryption(format!("{}", e)))?
            .with_no_client_auth()
            .with_single_cert(certs, keys.remove(0))
            .map_err(|e| ConduitError::Encryption(format!("{}", e)))?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Generate a self-signed TLS acceptor for the WSS (LAN WebSocket Secure) server.
/// The certificate includes localhost and all LAN IP addresses as SANs so mobile
/// clients can connect over WSS without certificate errors.
pub fn get_wss_acceptor() -> Result<TlsAcceptor, ConduitError> {
    let mut subject_alt_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];

    if let Ok(addrs) = local_ip_address::list_afinet_netifas() {
        for (_name, ip) in addrs {
            if !ip.is_loopback() {
                let ip_str = ip.to_string();
                if !subject_alt_names.contains(&ip_str) {
                    subject_alt_names.push(ip_str);
                }
            }
        }
    }

    info!("WSS certificate SANs: {:?}", subject_alt_names);

    let key_pair = rcgen::KeyPair::generate()
        .map_err(|e| ConduitError::Encryption(format!("rcgen key error: {}", e)))?;
    let cert_params = rcgen::CertificateParams::new(subject_alt_names)
        .map_err(|e| ConduitError::Encryption(format!("rcgen params error: {}", e)))?;
    let cert = cert_params
        .self_signed(&key_pair)
        .map_err(|e| ConduitError::Encryption(format!("rcgen sign error: {}", e)))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let mut cert_reader = std::io::BufReader::new(cert_pem.as_bytes());
    let mut key_reader = std::io::BufReader::new(key_pem.as_bytes());

    let certs_result: Result<Vec<CertificateDer<'static>>, std::io::Error> =
        certs(&mut cert_reader).collect();
    let certs = certs_result
        .map_err(|e| ConduitError::Encryption(format!("Failed to parse WSS certs: {}", e)))?;

    let keys_result: Result<Vec<PrivateKeyDer<'static>>, std::io::Error> =
        pkcs8_private_keys(&mut key_reader)
            .map(|res| res.map(PrivateKeyDer::Pkcs8))
            .collect();
    let mut keys = keys_result
        .map_err(|e| ConduitError::Encryption(format!("Failed to parse WSS keys: {}", e)))?;

    if keys.is_empty() {
        return Err(ConduitError::Encryption(
            "No private keys found for WSS".to_string(),
        ));
    }

    // TLS 1.3 only — the WSS WebSocket path must not fall back to TLS 1.2 or earlier.
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| ConduitError::Encryption(format!("WSS TLS config error: {}", e)))?
            .with_no_client_auth()
            .with_single_cert(certs, keys.remove(0))
            .map_err(|e| ConduitError::Encryption(format!("WSS TLS config error: {}", e)))?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}
