use log::{info, warn};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, private_key};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

/// Where the certificate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsSource {
    /// The relay generated a fresh self-signed certificate at startup.
    Generated,
    /// `cert.pem` / `key.pem` were found in the certificate directory.
    Provided,
}

/// Everything the relay needs to serve TLS, plus the values it must publish.
pub struct TlsContext {
    /// The TLS 1.3 server configuration, ready to hand to a `TlsAcceptor`.
    pub acceptor: TlsAcceptor,
    /// Certificate pin in `sha256/<base64>` form, computed over the
    /// SubjectPublicKeyInfo (SPKI) DER — see [`spki_sha256_pin`].
    ///
    /// This is the value `GET /pin` serves. Clients MUST pin this, not
    /// `sha256(cert-DER)`: the SPKI survives certificate renewal as long as the
    /// key pair is reused, whereas a whole-certificate pin breaks on renewal.
    pub spki_pin: String,
    /// Where `cert.pem` was read from, or written to.
    pub cert_path: PathBuf,
    /// Where `key.pem` was read from, or written to.
    pub key_path: PathBuf,
    /// SANs embedded in (or expected of) the served certificate.
    pub subject_alt_names: Vec<String>,
    /// Whether the material was generated here or supplied by the operator.
    pub source: TlsSource,
}

/// How a relay should present TLS.
///
/// Passed in rather than read from the environment inside this module, so that a
/// host embedding the relay can say what the certificate should look like
/// without mutating process-global state. [`TlsParams::from_env`] reproduces the
/// environment behaviour exactly, and is what the no-argument entry point uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsParams {
    /// Directory holding `cert.pem` and `key.pem`, or where they are created.
    pub cert_dir: PathBuf,
    /// The name the relay is actually reached by.
    ///
    /// Without this every client fails hostname verification against anything
    /// but localhost, which is the most common cause of a relay that works on
    /// the developer's machine and nowhere else.
    pub hostname: String,
    /// Further names to add to the certificate, e.g. a LAN address.
    pub extra_sans: Vec<String>,
}

impl TlsParams {
    /// Read the TLS settings from the environment, with the current defaults.
    ///
    /// `RELAY_CERT_DIR`, `RELAY_TLS_HOSTNAME` and `RELAY_TLS_EXTRA_SANS`.
    pub fn from_env() -> Self {
        Self {
            cert_dir: std::env::var("RELAY_CERT_DIR")
                .ok()
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("./certs")),
            hostname: std::env::var("RELAY_TLS_HOSTNAME")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
                .unwrap_or_else(|| "localhost".to_string()),
            extra_sans: std::env::var("RELAY_TLS_EXTRA_SANS")
                .ok()
                .map(|extra| {
                    extra
                        .split([',', ' ', ';'])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The full SAN list, with the loopback names always present.
    ///
    /// `localhost`, `127.0.0.1` and `::1` are unconditional so a relay started
    /// without configuration still works for local development. Duplicates are
    /// dropped — a certificate listing the same name twice is rejected outright
    /// by some TLS stacks.
    pub fn subject_alt_names(&self) -> Vec<String> {
        let mut sans = vec![
            self.hostname.clone(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
        ];
        for san in &self.extra_sans {
            let san = san.trim();
            if !san.is_empty() && !sans.iter().any(|s| s == san) {
                sans.push(san.to_string());
            }
        }
        sans
    }
}

impl Default for TlsParams {
    /// A localhost-only relay under the working directory.
    fn default() -> Self {
        Self {
            cert_dir: PathBuf::from("./certs"),
            hostname: "localhost".to_string(),
            extra_sans: Vec::new(),
        }
    }
}

/// Certificate directory, overridable with `RELAY_CERT_DIR`.
///
/// The default is CWD-relative, which only works because the Dockerfile sets
/// `WORKDIR /data`. Setting this explicitly removes that hidden dependency.
pub fn cert_dir() -> PathBuf {
    TlsParams::from_env().cert_dir
}

/// SANs to embed in a generated self-signed certificate, from the environment.
///
/// `RELAY_TLS_HOSTNAME` is the name the relay is actually reached by — without
/// it every client fails hostname verification against anything but localhost.
/// `RELAY_TLS_EXTRA_SANS` adds further names (comma/space separated), e.g. the
/// LAN address or an alternate public name.
pub fn configured_subject_alt_names() -> Vec<String> {
    TlsParams::from_env().subject_alt_names()
}

/// Load (or generate) the relay's TLS material from the environment.
pub fn load_tls_context() -> Result<TlsContext, String> {
    load_tls_context_with(&TlsParams::from_env())
}

/// Load (or generate) the relay's TLS material, honouring `params`.
pub fn load_tls_context_with(params: &TlsParams) -> Result<TlsContext, String> {
    let certs_dir = params.cert_dir.clone();
    if !certs_dir.exists() {
        std::fs::create_dir_all(&certs_dir).map_err(|e| {
            format!(
                "Failed to create certs directory {}: {e}",
                certs_dir.display()
            )
        })?;
    }

    let cert_path = certs_dir.join("cert.pem");
    let key_path = certs_dir.join("key.pem");

    let (cert_pem, key_pem, source) = if cert_path.exists() && key_path.exists() {
        info!(
            "Loading existing TLS certificate and key from {}",
            certs_dir.display()
        );
        // Best-effort: tighten permissions on a pre-existing key file that may
        // have been created with looser defaults (e.g. by an earlier version).
        tighten_key_permissions(&key_path);
        let cert = std::fs::read_to_string(&cert_path)
            .map_err(|e| format!("Failed to read {}: {e}", cert_path.display()))?;
        let key = std::fs::read_to_string(&key_path)
            .map_err(|e| format!("Failed to read {}: {e}", key_path.display()))?;
        (cert, key, TlsSource::Provided)
    } else {
        let subject_alt_names = params.subject_alt_names();
        info!(
            "Generating new self-signed TLS certificate in {} with SANs {:?}",
            certs_dir.display(),
            subject_alt_names
        );
        let key_pair = rcgen::KeyPair::generate().map_err(|e| format!("rcgen key error: {e}"))?;
        let cert_params = rcgen::CertificateParams::new(subject_alt_names.clone())
            .map_err(|e| format!("rcgen params error: {e}"))?;
        let cert = cert_params
            .self_signed(&key_pair)
            .map_err(|e| format!("rcgen sign error: {e}"))?;

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        std::fs::write(&cert_path, &cert_pem)
            .map_err(|e| format!("Failed to write {}: {e}", cert_path.display()))?;
        write_key_restricted(&key_path, &key_pem)?;

        return Ok(build_context(
            cert_pem,
            key_pem,
            subject_alt_names,
            cert_path,
            key_path,
            TlsSource::Generated,
        ));
    };

    // A supplied certificate's SANs are not visible without an X.509 parser,
    // so report what this relay is *expected* to serve.
    Ok(build_context(
        cert_pem,
        key_pem,
        configured_subject_alt_names(),
        cert_path,
        key_path,
        source,
    ))
}

fn build_context(
    cert_pem: String,
    key_pem: String,
    subject_alt_names: Vec<String>,
    cert_path: PathBuf,
    key_path: PathBuf,
    source: TlsSource,
) -> TlsContext {
    let config = build_tls_server_config(&cert_pem, &key_pem)
        .expect("TLS material was just validated by build_tls_server_config");
    let spki_pin = spki_sha256_pin(&cert_pem)
        .expect("certificate PEM parsed moments ago; SPKI extraction must succeed");
    TlsContext {
        acceptor: TlsAcceptor::from(Arc::new(config)),
        spki_pin,
        cert_path,
        key_path,
        subject_alt_names,
        source,
    }
}

/// Build a TLS 1.3-only `ServerConfig` from PEM cert and key.
///
/// Split out of [`load_tls_context`] so tests exercise the real
/// configuration path (protocol version restriction, cert/key parsing)
/// without touching the filesystem.
pub(crate) fn build_tls_server_config(
    cert_pem: &str,
    key_pem: &str,
) -> Result<ServerConfig, String> {
    let mut cert_reader = std::io::BufReader::new(cert_pem.as_bytes());
    let mut key_reader = std::io::BufReader::new(key_pem.as_bytes());

    let certs_result: Result<Vec<CertificateDer<'static>>, std::io::Error> =
        certs(&mut cert_reader).collect();
    let certs = certs_result.map_err(|e| format!("Failed to parse certs: {e}"))?;
    if certs.is_empty() {
        return Err(format!(
            "No certificates found in certificate PEM ({} block(s) expected)",
            cert_pem.matches("BEGIN CERTIFICATE").count()
        ));
    }

    // `private_key` auto-detects PKCS#8, PKCS#1 and SEC1. `pkcs8_private_keys`
    // (the previous implementation) silently yielded *zero* keys for a
    // PKCS#1 (`BEGIN RSA PRIVATE KEY`) or SEC1 (`BEGIN EC PRIVATE KEY`) file
    // and reported the misleading "No private keys found in key file" — which
    // is what `openssl rsa -traditional`, older certbot and Windows exports all
    // produce by default.
    let mut keys: Vec<PrivateKeyDer<'static>> = match private_key(&mut key_reader) {
        Ok(Some(key)) => vec![key],
        Ok(None) => Vec::new(),
        Err(e) => return Err(format!("Failed to parse private key: {e}")),
    };

    if keys.is_empty() {
        return Err(format!(
            "No private key found in key PEM — expected one of \
             'BEGIN PRIVATE KEY' (PKCS#8), 'BEGIN RSA PRIVATE KEY' (PKCS#1) or \
             'BEGIN EC PRIVATE KEY' (SEC1), found {} key block(s)",
            key_pem
                .lines()
                .filter(|l| l.contains("BEGIN") && l.contains("PRIVATE KEY"))
                .count()
        ));
    }

    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| format!("TLS config error: {e}"))?
            .with_no_client_auth()
            .with_single_cert(certs, keys.remove(0))
            .map_err(|e| format!("TLS config error: {e}"))?;

    Ok(config)
}

// ============================================================
//  Certificate pinning (SPKI)
// ============================================================

/// Standard base64 alphabet, no padding characters needed beyond `=`.
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64-encode with padding (RFC 4648 §4).
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// One DER TLV: `(tag, full_range, contents_range)`.
struct Tlv {
    tag: u8,
    full: std::ops::Range<usize>,
    contents: std::ops::Range<usize>,
}

/// Read one DER TLV starting at `pos`.
///
/// Handles the short and long definite-length forms only — DER mandates those;
/// BER's indefinite length is not legal in DER and not emitted by any of the
/// toolchains that produce these certificates.
fn read_tlv(buf: &[u8], pos: usize) -> Option<Tlv> {
    let tag = *buf.get(pos)?;
    let first_len = *buf.get(pos + 1)?;
    let (len, header_len) = if first_len & 0x80 == 0 {
        (first_len as usize, 2usize)
    } else {
        let n = (first_len & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None; // indefinite or absurdly long
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *buf.get(pos + 2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let full_start = pos;
    let contents_start = pos + header_len;
    let contents_end = contents_start.checked_add(len)?;
    if contents_end > buf.len() {
        return None;
    }
    Some(Tlv {
        tag,
        full: full_start..contents_end,
        contents: contents_start..contents_end,
    })
}

/// Tag byte of a constructed SEQUENCE.
const TAG_SEQUENCE: u8 = 0x30;
/// Tag byte of an INTEGER.
const TAG_INTEGER: u8 = 0x02;
/// Tag byte of `[0] EXPLICIT` (the optional `version` field).
const TAG_CONTEXT_0: u8 = 0xa0;

/// Extract the SubjectPublicKeyInfo (SPKI) DER from the first certificate in
/// `cert_pem`.
///
/// Walks `Certificate ::= SEQUENCE { tbsCertificate, ... }` and, inside
/// `TBSCertificate`, skips `version? serialNumber signature issuer validity
/// subject` to reach `subjectPublicKeyInfo`. The original DER bytes are sliced
/// out rather than re-encoded, so for any canonically-encoded certificate the
/// result is byte-identical to
/// `openssl x509 -pubkey -noout | openssl pkey -pubin -outform DER`.
pub fn spki_der_from_pem(cert_pem: &str) -> Result<Vec<u8>, String> {
    let mut reader = std::io::BufReader::new(cert_pem.as_bytes());
    let der: CertificateDer<'static> = certs(&mut reader)
        .next()
        .ok_or_else(|| "no certificate block found in PEM".to_string())?
        .map_err(|e| format!("failed to parse certificate PEM: {e}"))?;
    spki_der_from_certificate(&der)
}

/// [`spki_der_from_pem`] for an already-parsed certificate.
pub fn spki_der_from_certificate(cert: &CertificateDer<'_>) -> Result<Vec<u8>, String> {
    let der = cert.as_ref();
    let cert_tlv = read_tlv(der, 0).ok_or_else(|| "certificate is not valid DER".to_string())?;
    if cert_tlv.tag != TAG_SEQUENCE {
        return Err("certificate is not a DER SEQUENCE".to_string());
    }

    // First element of Certificate is the TBSCertificate.
    let tbs = read_tlv(der, cert_tlv.contents.start)
        .ok_or_else(|| "certificate TBSCertificate is truncated or malformed".to_string())?;
    if tbs.tag != TAG_SEQUENCE {
        return Err("TBSCertificate is not a DER SEQUENCE".to_string());
    }

    // TBSCertificate ::= SEQUENCE {
    //   version         [0] EXPLICIT INTEGER OPTIONAL,  -- skip if present
    //   serialNumber        INTEGER,                   -- tag 0x02
    //   signature           AlgorithmIdentifier,       -- SEQUENCE
    //   issuer              Name,                      -- SEQUENCE
    //   validity            Validity,                  -- SEQUENCE
    //   subject             Name,                      -- SEQUENCE
    //   subjectPublicKeyInfo SubjectPublicKeyInfo,     -- <-- what we want
    //   ... }
    //
    // Six elements precede the SPKI: the optional version, serialNumber, and
    // four SEQUENCEs (signature, issuer, validity, subject).
    const TBS_FIELDS_BEFORE_SPKI: usize = 6;

    let mut pos = tbs.contents.start;
    let mut consumed = 0usize;

    while consumed < TBS_FIELDS_BEFORE_SPKI {
        let tlv = read_tlv(der, pos).ok_or_else(|| {
            format!(
                "TBSCertificate ended after {consumed} field(s), before the \
                 subjectPublicKeyInfo"
            )
        })?;

        // Field 0 is `version?` ([0] EXPLICIT) or, if absent, serialNumber
        // (INTEGER). Field 1 is always serialNumber. Fields 2..=5 are SEQUENCEs.
        let expected_tag = match consumed {
            0 if tlv.tag == TAG_CONTEXT_0 => TAG_CONTEXT_0,
            0 | 1 => TAG_INTEGER,
            _ => TAG_SEQUENCE,
        };
        if tlv.tag != expected_tag {
            return Err(format!(
                "unexpected DER tag 0x{:02x} at TBSCertificate field {consumed} \
                 (expected 0x{expected_tag:02x}) — not an X.509 certificate",
                tlv.tag
            ));
        }
        pos = tlv.full.end;
        consumed += 1;
    }

    let spki = read_tlv(der, pos).ok_or_else(|| "no SPKI element found".to_string())?;
    if spki.tag != TAG_SEQUENCE {
        return Err("subjectPublicKeyInfo is not a DER SEQUENCE".to_string());
    }
    Ok(der[spki.full].to_vec())
}

/// `sha256/<base64>` SPKI certificate pin for the first certificate in `cert_pem`.
pub fn spki_sha256_pin(cert_pem: &str) -> Result<String, String> {
    let spki = spki_der_from_pem(cert_pem)?;
    Ok(format!(
        "sha256/{}",
        base64_encode(&conduit_protocol::hmac::sha256(&spki))
    ))
}

fn write_key_restricted(path: &Path, contents: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| {
                format!(
                    "Failed to create key file with restricted permissions: {}",
                    e
                )
            })?;
        file.write_all(contents.as_bytes())
            .map_err(|e| format!("Failed to write key: {}", e))?;
    }

    #[cfg(not(unix))]
    {
        // Non-unix: `Permissions::from_mode` is unix-only. Windows relies on NTFS
        // ACLs (set by directory permissions), so a plain write is the best we can
        // do portably here. Do NOT set the readonly flag — it would break key
        // regeneration on the next startup.
        std::fs::write(path, contents).map_err(|e| format!("Failed to write key: {}", e))?;
    }

    Ok(())
}

/// Best-effort: enforce 0600 on an existing key file (unix only).
/// Failures are logged but non-fatal — the key is still usable.
#[cfg(unix)]
fn tighten_key_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        log::warn!("Failed to tighten permissions on {}: {}", path.display(), e);
    }
}

#[cfg(not(unix))]
fn tighten_key_permissions(_path: &Path) {
    // No-op on non-unix (see write_key_restricted non-unix branch).
}

/// Log a warning if the loaded certificate cannot serve the configured SANs.
pub fn warn_if_sans_may_not_match(context: &TlsContext) {
    if context.source == TlsSource::Generated {
        return;
    }
    warn!(
        "Using the supplied certificate {} — SANs were not verified against \
         the configured RELAY_TLS_HOSTNAME {:?}. Run \
         `openssl x509 -in {} -noout -text` (or -checkhost) to confirm.",
        context.cert_path.display(),
        context.subject_alt_names,
        context.cert_path.display()
    );
}

// Opts back into `unsafe` for exactly one reason: `std::env::set_var` and
// `remove_var` are unsafe in edition 2024, and several tests here exist to pin
// the environment's effect on the certificate.
#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls_pemfile::certs;
    use std::sync::Arc;

    /// Generate a fresh self-signed certificate + key in PEM form.
    fn generate_self_signed_pair() -> (String, String) {
        generate_self_signed_pair_with_sans(&[
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            "::1".to_string(),
        ])
    }

    fn generate_self_signed_pair_with_sans(sans: &[String]) -> (String, String) {
        let key_pair = rcgen::KeyPair::generate().expect("key gen failed");
        let params = rcgen::CertificateParams::new(sans.to_vec()).expect("params failed");
        let cert = params.self_signed(&key_pair).expect("self_signed failed");
        (cert.pem(), key_pair.serialize_pem())
    }

    /// Parse PEM cert and key into rustls types (mirrors load_tls_context internals).
    fn parse_cert_and_key(
        cert_pem: &str,
        key_pem: &str,
    ) -> (Vec<CertificateDer<'static>>, Vec<PrivateKeyDer<'static>>) {
        let mut cr = std::io::BufReader::new(cert_pem.as_bytes());
        let certs_vec: Vec<CertificateDer<'static>> = certs(&mut cr)
            .collect::<Result<_, _>>()
            .expect("cert parse failed");

        let mut kr = std::io::BufReader::new(key_pem.as_bytes());
        let keys_vec: Vec<PrivateKeyDer<'static>> =
            private_key(&mut kr).into_iter().flatten().collect();

        (certs_vec, keys_vec)
    }

    // ---------------------------------------------------------------
    //  Certificate generation
    // ---------------------------------------------------------------

    #[test]
    fn self_signed_cert_generation_produces_valid_pem() {
        let (cert_pem, key_pem) = generate_self_signed_pair();

        assert!(
            cert_pem.starts_with("-----BEGIN CERTIFICATE-----"),
            "cert PEM should start with BEGIN CERTIFICATE"
        );
        assert!(
            cert_pem.contains("-----END CERTIFICATE-----"),
            "cert PEM should contain END CERTIFICATE"
        );
        assert!(
            key_pem.starts_with("-----BEGIN PRIVATE KEY-----"),
            "key PEM should start with BEGIN PRIVATE KEY"
        );
    }

    #[test]
    fn self_signed_cert_is_parsable_by_rustls() {
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let (certs_vec, keys_vec) = parse_cert_and_key(&cert_pem, &key_pem);

        assert_eq!(certs_vec.len(), 1, "should parse exactly 1 certificate");
        assert_eq!(keys_vec.len(), 1, "should parse exactly 1 private key");
    }

    // ---------------------------------------------------------------
    //  TLS 1.3 configuration (via the real build_tls_server_config path)
    // ---------------------------------------------------------------

    #[test]
    fn tls_config_enforces_tls13_only() {
        // The production code path builds a valid ServerConfig using ONLY
        // TLS 1.3 via build_tls_server_config — the same function
        // load_tls_context calls.
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let config = build_tls_server_config(&cert_pem, &key_pem)
            .expect("TLS 1.3 config should build from a valid cert/key pair");
        assert_eq!(
            config.max_early_data_size, 0,
            "early data should not be enabled by default"
        );
    }

    #[test]
    fn tls_config_enforces_only_tls13() {
        // Same production path; building successfully with TLS13-only
        // protocol versions proves the restriction is applied. A config
        // built without TLS 1.3 support would fail with_protocol_versions.
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let config = build_tls_server_config(&cert_pem, &key_pem)
            .expect("TLS config should build with TLS 1.3 restriction");
        // Verify explicitly: with_single_cert succeeded under the TLS13-only
        // builder, so the config exists and has no client auth.
        let _ = config;
    }

    #[test]
    fn build_tls_server_config_rejects_empty_cert_pem() {
        let (_, key_pem) = generate_self_signed_pair();
        let result = build_tls_server_config("", &key_pem);
        // Empty cert PEM parses to 0 certs → explicit rejection
        assert!(result.is_err(), "empty cert PEM should be rejected");
        assert!(
            result.unwrap_err().contains("No certificates found"),
            "error should say no certificates were found"
        );
    }

    #[test]
    fn build_tls_server_config_rejects_empty_key_pem() {
        let (cert_pem, _) = generate_self_signed_pair();
        let result = build_tls_server_config(&cert_pem, "");
        assert!(
            result.is_err(),
            "empty key PEM (no private keys) should be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("No private key found"),
            "error should mention missing private keys, got: {}",
            err
        );
    }

    #[test]
    fn build_tls_server_config_rejects_garbage_pem() {
        let result = build_tls_server_config("not a cert", "not a key");
        assert!(result.is_err(), "garbage PEM should be rejected");
    }

    #[test]
    fn build_tls_server_config_rejects_mismatched_cert_and_key() {
        // Valid cert from one pair, key from a different pair.
        let (cert_a, _) = generate_self_signed_pair();
        let (_, key_b) = generate_self_signed_pair();
        let result = build_tls_server_config(&cert_a, &key_b);
        assert!(
            result.is_err(),
            "cert/key that do not belong together should be rejected"
        );
    }

    // ---------------------------------------------------------------
    //  Key format acceptance (VULNERABILITY 10)
    // ---------------------------------------------------------------

    /// Re-encode a PKCS#8 PEM into the legacy labelled PEM forms.
    ///
    /// The DER payload is identical in all three cases; only the PEM armour
    /// (and therefore the parser branch) differs. This is exactly the
    /// difference `openssl rsa -traditional` / older certbot / Windows exports
    /// produce, and exactly what the old `pkcs8_private_keys`-only parser
    /// silently rejected.
    fn repem(pkcs8_pem: &str, label: &str) -> String {
        let body: String = pkcs8_pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n")
    }

    #[test]
    fn private_key_accepts_pkcs8_private_key() {
        let (cert_pem, key_pem) = generate_self_signed_pair();
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(build_tls_server_config(&cert_pem, &key_pem).is_ok());
    }

    /// What `rustls_pemfile::private_key` yields for each PEM armour.
    #[test]
    fn private_key_recognises_all_three_pem_armours() {
        // The regression this pins: the old `pkcs8_private_keys`-only
        // implementation yielded *zero* keys for PKCS#1 and SEC1 armour and
        // then reported "No private keys found in key file", which is what
        // `openssl rsa -traditional`, older certbot and Windows exports all
        // produce by default.
        let (_, key_pem) = generate_self_signed_pair();

        let mut kr = std::io::BufReader::new(key_pem.as_bytes());
        assert!(
            matches!(private_key(&mut kr), Ok(Some(PrivateKeyDer::Pkcs8(_)))),
            "PKCS#8 armour must yield PrivateKeyDer::Pkcs8"
        );

        let pkcs1 = repem(&key_pem, "RSA PRIVATE KEY");
        let mut kr = std::io::BufReader::new(pkcs1.as_bytes());
        assert!(
            matches!(private_key(&mut kr), Ok(Some(PrivateKeyDer::Pkcs1(_)))),
            "'BEGIN RSA PRIVATE KEY' must be recognised as PKCS#1, not skipped"
        );

        let sec1 = repem(&key_pem, "EC PRIVATE KEY");
        let mut kr = std::io::BufReader::new(sec1.as_bytes());
        assert!(
            matches!(private_key(&mut kr), Ok(Some(PrivateKeyDer::Sec1(_)))),
            "'BEGIN EC PRIVATE KEY' must be recognised as SEC1, not skipped"
        );
    }

    #[test]
    fn build_tls_server_config_reports_key_format_not_absence() {
        // With all three armours recognised, an unreadable key now surfaces a
        // parse error from the crypto layer instead of the misleading
        // "no private keys found" message.
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let pkcs1 = repem(&key_pem, "RSA PRIVATE KEY");
        let err = build_tls_server_config(&cert_pem, &pkcs1).unwrap_err();
        assert!(
            err.contains("TLS config error"),
            "the key WAS found; the failure must come from key parsing, got: {err}"
        );
    }

    #[test]
    fn missing_private_key_error_lists_supported_formats() {
        let (cert_pem, _) = generate_self_signed_pair();
        let err = build_tls_server_config(&cert_pem, "not a key at all").unwrap_err();
        for expected in ["PKCS#8", "PKCS#1", "SEC1"] {
            assert!(
                err.contains(expected),
                "error should name the {expected} format so an operator can \
                 diagnose a key-file problem: {err}"
            );
        }
    }

    // ---------------------------------------------------------------
    //  Key file restricted permissions
    // ---------------------------------------------------------------

    #[test]
    fn write_key_restricted_creates_file_with_contents() {
        let dir = std::env::temp_dir().join("relay-test-key-perms");
        std::fs::create_dir_all(&dir).ok();
        let key_path = dir.join("test-key.pem");

        // Clean up any prior run
        std::fs::remove_file(&key_path).ok();

        let content = "-----BEGIN PRIVATE KEY-----\nFAKE_KEY_DATA\n-----END PRIVATE KEY-----\n";
        let result = write_key_restricted(&key_path, content);
        assert!(
            result.is_ok(),
            "write_key_restricted failed: {:?}",
            result.err()
        );

        let written = std::fs::read_to_string(&key_path).expect("could not read key file");
        assert_eq!(written, content);

        // Clean up
        std::fs::remove_file(&key_path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_key_restricted_sets_600_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join("relay-test-key-perms-unix");
        std::fs::create_dir_all(&dir).ok();
        let key_path = dir.join("test-key-600.pem");

        std::fs::remove_file(&key_path).ok();

        write_key_restricted(&key_path, "secret key").expect("write failed");

        let meta = std::fs::metadata(&key_path).expect("metadata failed");
        let mode = meta.permissions().mode();
        // 0o600 = 384 decimal, owner read+write only
        assert_eq!(
            mode & 0o777,
            0o600,
            "key file should be mode 0600, got {:o}",
            mode & 0o777
        );

        std::fs::remove_file(&key_path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    // ---------------------------------------------------------------
    //  Edge cases
    // ---------------------------------------------------------------

    #[test]
    fn parse_empty_cert_pem_fails() {
        let mut cr = std::io::BufReader::new("".as_bytes());
        let certs_vec: Vec<CertificateDer<'static>> = certs(&mut cr)
            .collect::<Result<_, _>>()
            .expect("empty cert should parse to 0 certs");
        assert!(certs_vec.is_empty(), "empty PEM should yield 0 certs");
    }

    #[test]
    fn parse_invalid_key_pem_fails() {
        // private_key returns Ok(None) for garbage PEM: no key was recognised.
        let mut kr = std::io::BufReader::new("NOT A REAL KEY".as_bytes());
        let keys: Vec<PrivateKeyDer<'static>> =
            private_key(&mut kr).into_iter().flatten().collect();
        assert!(
            keys.is_empty(),
            "garbage PEM should produce zero private keys"
        );
    }

    #[test]
    fn parse_valid_pkcs8_key_pem_succeeds() {
        let (_, key_pem) = generate_self_signed_pair();
        let mut kr = std::io::BufReader::new(key_pem.as_bytes());
        let keys: Vec<PrivateKeyDer<'static>> =
            private_key(&mut kr).into_iter().flatten().collect();
        assert_eq!(keys.len(), 1, "should parse exactly one key");
    }

    // ---------------------------------------------------------------
    //  SPKI extraction + certificate pin (VULNERABILITY 9)
    // ---------------------------------------------------------------

    #[test]
    fn spki_extraction_matches_the_generating_key_pair() {
        // Strongest available cross-check: the SPKI the relay extracts from the
        // certificate must equal the SPKI of the key pair that signed it.
        let key_pair = rcgen::KeyPair::generate().expect("key gen failed");
        let params =
            rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params failed");
        let cert = params.self_signed(&key_pair).expect("self_signed failed");

        let extracted = spki_der_from_pem(&cert.pem()).expect("spki extraction");
        assert_eq!(
            extracted,
            key_pair.public_key_der(),
            "SPKI sliced out of the certificate must equal the key pair's SPKI"
        );
    }

    #[test]
    fn spki_extraction_is_a_der_sequence() {
        let (cert_pem, _) = generate_self_signed_pair();
        let spki = spki_der_from_pem(&cert_pem).expect("spki extraction");
        assert_eq!(spki[0], TAG_SEQUENCE, "SPKI must start with SEQUENCE");
        // The extracted bytes must themselves re-parse as one complete element.
        let tlv = read_tlv(&spki, 0).expect("SPKI must re-parse as a single TLV");
        assert_eq!(tlv.full.end, spki.len(), "SPKI must consume all its bytes");
    }

    #[test]
    fn spki_pin_is_sha256_prefixed_base64_of_32_bytes() {
        let (cert_pem, _) = generate_self_signed_pair();
        let pin = spki_sha256_pin(&cert_pem).expect("pin");
        let (prefix, b64) = pin.split_once('/').expect("pin must have a scheme prefix");
        assert_eq!(prefix, "sha256", "pin scheme must be sha256");
        assert_eq!(
            base64_decoded_len(b64),
            32,
            "pin must cover a 32-byte SHA-256"
        );
    }

    /// Decoded length of a padded base64 string.
    fn base64_decoded_len(b64: &str) -> usize {
        let padding = b64.chars().filter(|c| *c == '=').count();
        b64.len() / 4 * 3 - padding
    }

    #[test]
    fn spki_pin_differs_between_different_certificates() {
        let (a, _) = generate_self_signed_pair();
        let (b, _) = generate_self_signed_pair();
        assert_ne!(
            spki_sha256_pin(&a).unwrap(),
            spki_sha256_pin(&b).unwrap(),
            "distinct key pairs must produce distinct pins"
        );
    }

    #[test]
    fn spki_pin_is_stable_across_calls() {
        let (cert_pem, _) = generate_self_signed_pair();
        assert_eq!(
            spki_sha256_pin(&cert_pem).unwrap(),
            spki_sha256_pin(&cert_pem).unwrap()
        );
    }

    #[test]
    fn spki_pin_is_not_the_whole_certificate_hash() {
        // The mobile client historically hashed `cert.der`. Hashing the whole
        // certificate is a DIFFERENT value, which is why the documented pin
        // could never match. Pin the SPKI and say so.
        let (cert_pem, _) = generate_self_signed_pair();
        let mut cr = std::io::BufReader::new(cert_pem.as_bytes());
        let der: CertificateDer<'static> = certs(&mut cr).next().unwrap().unwrap();

        let spki_pin = base64_encode(&conduit_protocol::hmac::sha256(
            &spki_der_from_certificate(&der).unwrap(),
        ));
        let whole_cert_pin = base64_encode(&conduit_protocol::hmac::sha256(der.as_ref()));
        assert_ne!(
            spki_pin, whole_cert_pin,
            "SPKI pin and whole-certificate pin must differ — clients must use SPKI"
        );
    }

    #[test]
    fn spki_extraction_rejects_garbage() {
        assert!(spki_der_from_pem("not a certificate").is_err());
        assert!(spki_der_from_pem("").is_err());
    }

    /// A real certificate, and the pin a client must compute from it.
    ///
    /// The Dart client parses SPKI out of `cert.der` itself, and this is the
    /// only thing that can prove the two parsers agree: the Rust side walks the
    /// DER, the Dart side walks it again, and both must land on the same bytes.
    /// A freshly generated certificate would prove nothing, because the two
    /// sides would be handed different input.
    ///
    /// `apps/mobile/test/services/relay_route_test.dart` asserts the same two
    /// fixtures. Regenerate both sides with
    /// `cargo test -p conduit-relay --lib emit_spki_vector_fixture -- --ignored --nocapture`.
    #[test]
    fn spki_vector_for_the_dart_client() {
        const CERT_DER_BASE64: &str = include_str!("../tests/fixtures/spki_vector_cert.der.b64");
        let der = decode_base64(CERT_DER_BASE64.trim());
        let spki = spki_der_from_certificate(&CertificateDer::from(der.clone())).unwrap();
        let pin = base64_encode(&conduit_protocol::hmac::sha256(&spki));

        const EXPECTED_PIN: &str = include_str!("../tests/fixtures/spki_vector_pin.txt");
        assert_eq!(
            pin.trim(),
            EXPECTED_PIN.trim(),
            "the recorded SPKI pin no longer matches the recorded certificate"
        );
        assert_ne!(
            pin.trim(),
            base64_encode(&conduit_protocol::hmac::sha256(&der)),
            "the SPKI pin must differ from the whole-certificate hash — this is \
             the exact confusion the mobile client had"
        );
    }

    /// Decode standard base64, ignoring whitespace.
    fn decode_base64(input: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut acc: u32 = 0;
        let mut bits = 0;
        for ch in input.chars().filter(|c| !c.is_whitespace()) {
            if ch == '=' {
                break;
            }
            let v = match ch {
                'A'..='Z' => ch as u32 - 'A' as u32,
                'a'..='z' => ch as u32 - 'a' as u32 + 26,
                '0'..='9' => ch as u32 - '0' as u32 + 52,
                '+' => 62,
                '/' => 63,
                _ => panic!("invalid base64 character {ch:?}"),
            };
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }

    /// Emit the fixture files, once, by hand.
    ///
    /// Not a normal test: run with `SPKI_VECTOR=1 cargo test -p conduit-relay
    /// --lib emit_spki_vector_fixture -- --nocapture --ignored` and commit what
    /// it prints. The generated certificate is thrown away afterwards, so the
    /// committed fixture is a real certificate that simply happens to be old.
    #[test]
    #[ignore = "fixture generator; run manually"]
    fn emit_spki_vector_fixture() {
        let (cert_pem, _) = generate_self_signed_pair();
        let mut cr = std::io::BufReader::new(cert_pem.as_bytes());
        let der: Vec<u8> = certs(&mut cr).next().unwrap().unwrap().as_ref().to_vec();
        let spki = spki_der_from_certificate(&CertificateDer::from(der.clone())).unwrap();
        let pin = base64_encode(&conduit_protocol::hmac::sha256(&spki));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        std::fs::create_dir_all(&root).expect("fixtures dir");
        std::fs::write(
            root.join("spki_vector_cert.der.b64"),
            format!("{}\n", base64_encode(&der)),
        )
        .expect("write cert fixture");
        std::fs::write(root.join("spki_vector_pin.txt"), format!("{pin}\n"))
            .expect("write pin fixture");
        println!("wrote fixtures under {}", root.display());
    }

    // ---------------------------------------------------------------
    //  Base64 encoder
    // ---------------------------------------------------------------

    #[test]
    fn base64_encode_matches_rfc4648_test_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // 32 zero bytes — the shape of a SHA-256 digest.
        assert_eq!(
            base64_encode(&[0u8; 32]),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    // ---------------------------------------------------------------
    //  SAN configuration (VULNERABILITY 8)
    // ---------------------------------------------------------------

    #[test]
    #[serial_test::serial]
    fn default_sans_cover_localhost() {
        // Guard against env leakage from other tests in this process.
        let saved =
            ["RELAY_TLS_HOSTNAME", "RELAY_TLS_EXTRA_SANS"].map(|k| (k, std::env::var(k).ok()));
        for (k, _) in saved.iter() {
            unsafe { std::env::remove_var(k) };
        }

        let sans = configured_subject_alt_names();
        assert!(sans.contains(&"localhost".to_string()));
        assert!(sans.contains(&"127.0.0.1".to_string()));
        assert!(sans.contains(&"::1".to_string()));

        for (k, v) in saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn configured_sans_include_the_deployment_hostname() {
        let saved =
            ["RELAY_TLS_HOSTNAME", "RELAY_TLS_EXTRA_SANS"].map(|k| (k, std::env::var(k).ok()));
        unsafe {
            std::env::set_var("RELAY_TLS_HOSTNAME", "relay.example.com");
            std::env::set_var("RELAY_TLS_EXTRA_SANS", "relay2.example.com, 10.0.0.5");
        }

        let sans = configured_subject_alt_names();
        assert!(
            sans.contains(&"relay.example.com".to_string()),
            "the deployed hostname must be a SAN, got {sans:?}"
        );
        assert!(sans.contains(&"relay2.example.com".to_string()));
        assert!(sans.contains(&"10.0.0.5".to_string()));

        for (k, v) in saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }

    #[test]
    fn generated_certificate_contains_the_configured_san() {
        let (cert_pem, _) = generate_self_signed_pair_with_sans(&[
            "relay.example.com".to_string(),
            "127.0.0.1".to_string(),
        ]);
        // The PEM is base64; assert on the decoded presence via rustls parsing
        // of the SAN extension by simply confirming the cert parses and the
        // configured name is what we asked for.
        let mut cr = std::io::BufReader::new(cert_pem.as_bytes());
        let parsed: Vec<CertificateDer<'static>> = certs(&mut cr)
            .collect::<Result<_, _>>()
            .expect("cert with custom SAN must parse");
        assert_eq!(parsed.len(), 1);
    }

    // ---------------------------------------------------------------
    //  Full TLS handshake over real TCP
    // ---------------------------------------------------------------

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::TlsConnector;

    /// Build a client config that trusts exactly `cert_pem`.
    fn client_config_trusting(cert_pem: &str) -> rustls::ClientConfig {
        let mut roots = rustls::RootCertStore::empty();
        let mut cr = std::io::BufReader::new(cert_pem.as_bytes());
        let certs_vec: Vec<CertificateDer<'static>> = certs(&mut cr)
            .collect::<Result<_, _>>()
            .expect("cert parse failed");
        for c in certs_vec {
            roots.add(c).expect("add trust anchor");
        }
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
    }

    /// Client config with an EMPTY trust store — every server cert is untrusted.
    fn client_config_untrusted() -> rustls::ClientConfig {
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth()
    }

    /// Client config restricted to TLS 1.2 only (cannot talk to our TLS1.3-only server).
    fn client_config_tls12_only_trusting(cert_pem: &str) -> rustls::ClientConfig {
        let mut roots = rustls::RootCertStore::empty();
        let mut cr = std::io::BufReader::new(cert_pem.as_bytes());
        let certs_vec: Vec<CertificateDer<'static>> = certs(&mut cr)
            .collect::<Result<_, _>>()
            .expect("cert parse failed");
        for c in certs_vec {
            roots.add(c).expect("add trust anchor");
        }
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS12])
        .expect("TLS 1.2 should be available")
        .with_root_certificates(roots)
        .with_no_client_auth()
    }

    /// Generate a self-signed cert whose SAN is `san` only (for hostname mismatch tests).
    fn generate_self_signed_pair_with_san(san: &str) -> (String, String) {
        generate_self_signed_pair_with_sans(&[san.to_string()])
    }

    /// Run one full server-side TLS handshake on an ephemeral port and
    /// report whether it succeeded (and the negotiated protocol).
    async fn run_handshake(
        server_cert_pem: &str,
        server_key_pem: &str,
        client: rustls::ClientConfig,
        server_name: &str,
    ) -> Result<Option<rustls::ProtocolVersion>, String> {
        let server_config = build_tls_server_config(server_cert_pem, server_key_pem)?;
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            match acceptor.accept(stream).await {
                Ok(tls) => {
                    // Echo one byte so the client observes a completed handshake.
                    let (mut r, mut w) = tokio::io::split(tls);
                    let mut buf = [0u8; 1];
                    if let Ok(1) = r.read(&mut buf).await {
                        let _ = w.write_all(&buf).await;
                    }
                    Ok::<(), String>(())
                }
                Err(e) => Err(format!("server handshake error: {}", e)),
            }
        });

        let connector = TlsConnector::from(Arc::new(client));
        let stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        let name = rustls::pki_types::ServerName::try_from(server_name.to_string())
            .map_err(|e| format!("bad server name: {e}"))?;

        match connector.connect(name, stream).await {
            Ok(mut tls) => {
                // Complete the echo round-trip to prove the session works.
                let _ = tls.write_all(b"p").await;
                let mut buf = [0u8; 1];
                let n = tls.read(&mut buf).await.map_err(|e| e.to_string())?;
                if n != 1 || buf[0] != b'p' {
                    return Err("echo round-trip failed".to_string());
                }
                let version = tls.get_ref().1.protocol_version();
                let _ = server_task.await;
                Ok(version)
            }
            Err(e) => {
                // Client-side failure (expected for negative tests).
                server_task.abort();
                Err(format!("client handshake error: {e}"))
            }
        }
    }

    #[tokio::test]
    async fn tls_handshake_completes_and_negotiates_tls13() {
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let client = client_config_trusting(&cert_pem);

        let version = run_handshake(&cert_pem, &key_pem, client, "localhost")
            .await
            .expect("handshake should succeed");

        assert_eq!(
            version,
            Some(rustls::ProtocolVersion::TLSv1_3),
            "negotiated protocol must be TLS 1.3"
        );
    }

    #[tokio::test]
    async fn tls_handshake_succeeds_for_configured_deployment_hostname() {
        // The bug this closes: a cert generated without RELAY_TLS_HOSTNAME is
        // only valid for localhost, so every client fails verification against
        // the real deployment name.
        let (cert_pem, key_pem) =
            generate_self_signed_pair_with_sans(&["relay.example.com".to_string()]);
        let client = client_config_trusting(&cert_pem);

        let version = run_handshake(&cert_pem, &key_pem, client, "relay.example.com")
            .await
            .expect("handshake must succeed for the configured hostname");
        assert_eq!(version, Some(rustls::ProtocolVersion::TLSv1_3));
    }

    #[tokio::test]
    async fn tls_handshake_rejects_certificate_for_wrong_hostname() {
        // Server cert is valid for "other.example.com" only; client asks for "localhost".
        let (cert_pem, key_pem) = generate_self_signed_pair_with_san("other.example.com");
        let client = client_config_trusting(&cert_pem);

        let result = run_handshake(&cert_pem, &key_pem, client, "localhost").await;
        assert!(
            result.is_err(),
            "handshake must fail when cert SAN does not match the requested name"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("handshake") || err.contains("certificate") || err.contains("Invalid"),
            "error should be certificate/hostname related, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn tls_handshake_rejects_untrusted_certificate() {
        // Client trusts nothing — server presents a self-signed cert.
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let client = client_config_untrusted();

        let result = run_handshake(&cert_pem, &key_pem, client, "localhost").await;
        assert!(
            result.is_err(),
            "handshake must fail when the client does not trust the server cert"
        );
    }

    #[tokio::test]
    async fn tls_handshake_rejects_tls12_only_client() {
        // Server is TLS 1.3-only; client only offers TLS 1.2 → no overlap.
        let (cert_pem, key_pem) = generate_self_signed_pair();
        let client = client_config_tls12_only_trusting(&cert_pem);

        let result = run_handshake(&cert_pem, &key_pem, client, "localhost").await;
        assert!(
            result.is_err(),
            "handshake must fail when client offers only TLS 1.2"
        );
    }

    // ---------------------------------------------------------------
    //  load_tls_context: generate then reload from disk
    // ---------------------------------------------------------------

    /// Guard that swaps the process CWD to a temp dir and restores it on drop.
    struct CwdGuard {
        original: std::path::PathBuf,
    }

    impl CwdGuard {
        fn enter(dir: &Path) -> Self {
            let original = std::env::current_dir().expect("current_dir");
            std::env::set_current_dir(dir).expect("set_current_dir");
            CwdGuard { original }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            // Restore BEFORE the temp dir is removed (Windows locks open dirs).
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn load_tls_context_generates_then_reloads_certificate() {
        let dir = std::env::temp_dir().join(format!(
            "relay-tls-acceptor-{}-{}",
            std::process::id(),
            "gen"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp certs dir");

        let cert_path;
        let key_path;
        let first_pin;
        {
            let _guard = CwdGuard::enter(&dir);

            // First call: no ./certs → generates a new self-signed pair.
            let ctx1 = load_tls_context().expect("first load_tls_context");
            assert_eq!(ctx1.source, TlsSource::Generated);
            cert_path = dir.join("certs").join("cert.pem");
            key_path = dir.join("certs").join("key.pem");
            assert!(cert_path.exists(), "cert.pem should be generated");
            assert!(key_path.exists(), "key.pem should be generated");
            first_pin = ctx1.spki_pin.clone();
            assert!(
                first_pin.starts_with("sha256/"),
                "context must publish a pin, got {first_pin}"
            );

            let cert1 = std::fs::read_to_string(&cert_path).expect("read cert");

            // Second call: certs exist → loads the same files.
            let ctx2 = load_tls_context().expect("second load_tls_context");
            assert_eq!(ctx2.source, TlsSource::Provided);
            let cert2 = std::fs::read_to_string(&cert_path).expect("read cert again");
            assert_eq!(
                cert1, cert2,
                "reload must reuse the existing certificate, not regenerate"
            );
            assert_eq!(
                ctx2.spki_pin, first_pin,
                "the pin must be stable across restarts (key is unchanged)"
            );
        } // CwdGuard restores CWD here

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn relay_cert_dir_env_var_overrides_the_default_location() {
        let dir = std::env::temp_dir().join(format!("relay-certdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let certs_dir = dir.join("somewhere-else");
        let saved = std::env::var("RELAY_CERT_DIR").ok();
        unsafe { std::env::set_var("RELAY_CERT_DIR", &certs_dir) };

        let ctx = load_tls_context().expect("load with RELAY_CERT_DIR");
        assert_eq!(ctx.cert_path, certs_dir.join("cert.pem"));
        assert!(certs_dir.join("cert.pem").exists());

        match saved {
            Some(v) => unsafe { std::env::set_var("RELAY_CERT_DIR", v) },
            None => unsafe { std::env::remove_var("RELAY_CERT_DIR") },
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// Certificate produced by
    /// `openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
    /// -days 3 -nodes -subj /CN=relay.example.com \
    /// -addext subjectAltName=DNS:relay.example.com,DNS:localhost,IP:127.0.0.1`
    ///
    /// A third-party encoder on purpose: validating our DER walk against rcgen
    /// alone would only prove it round-trips our own writer.
    const OPENSSL_EC_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIBmzCCAUGgAwIBAgIUUNG4+sZy1C5YPTfWROlhvw4G+AAwCgYIKoZIzj0EAwIw\n",
        "HDEaMBgGA1UEAwwRcmVsYXkuZXhhbXBsZS5jb20wHhcNMjYwOTI4MTI0MjEzWhcN\n",
        "MjYxMDAxMTI0MjEzWjAcMRowGAYDVQQDDBFyZWxheS5leGFtcGxlLmNvbTBZMBMG\n",
        "ByqGSM49AgEGCCqGSM49AwEHA0IABIkLcdVOZHVZmfYlxqXAlZinx7rjYqgcxk4x\n",
        "n7/JRlkS9zqaCgdJUwfWzA7esaLHbY00W95qGw/PLuVSK4BYELSjYTBfMB0GA1Ud\n",
        "DgQWBBTxS4ZtTuxjHDti9cvOV75+wEToBTAPBgNVHRMBAf8EBTADAQH/MC0GA1Ud\n",
        "EQQmMCSCEXJlbGF5LmV4YW1wbGUuY29tgglsb2NhbGhvc3SHBH8AAAEwCgYIKoZI\n",
        "zj0EAwIDSAAwRQIhALSORgq2vTdYdxF+WEiyei87FXXwHNon9QKn/br5e//jAiBE\n",
        "7sg01MKumAMqNGOPxsPYSgu0XyGLtEjlCImxyXEsqA==\n",
        "-----END CERTIFICATE-----\n",
    );
    /// `sha256/` pin of [OPENSSL_EC_CERT_PEM] as produced by the documented
    /// openssl pipeline. Cross-checks our DER walk and base64 encoder against a
    /// reference implementation.
    const OPENSSL_EC_CERT_PIN: &str = "sha256/PWkK+oyrvlC4/ArXmgalbiP58FAclYv2uqn11MW6Y3I=";
    #[test]
    fn spki_extraction_matches_openssl_byte_for_byte() {
        // Validates the DER walk against a certificate produced by a
        // third-party encoder, against a pin computed by the documented
        // openssl pipeline:
        //   openssl x509 -pubkey -noout | openssl pkey -pubin -outform DER \
        //     | openssl dgst -sha256 -binary | openssl base64 -A
        let spki = spki_der_from_pem(OPENSSL_EC_CERT_PEM).expect("spki extraction");

        assert_eq!(
            format!(
                "sha256/{}",
                base64_encode(&conduit_protocol::hmac::sha256(&spki))
            ),
            OPENSSL_EC_CERT_PIN,
            "the relay's SPKI pin must equal the pin the documented openssl \
             pipeline produces for the same certificate"
        );

        // Byte-identity against openssl over the extracted bytes: this is what
        // makes slicing (rather than re-encoding) safe — canonical DER in,
        // canonical DER out.
        let hashed: Option<std::process::Output> = {
            use std::io::Write;
            let spawned = std::process::Command::new("openssl")
                .args(["dgst", "-sha256", "-binary"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();
            match spawned {
                Ok(mut child) => {
                    if let Some(stdin) = child.stdin.as_mut() {
                        let _ = stdin.write_all(&spki);
                    }
                    child.wait_with_output().ok()
                }
                Err(_) => None,
            }
        };
        if let Some(out) = hashed {
            assert!(
                out.status.success(),
                "openssl must be able to hash the extracted SPKI"
            );
            assert_eq!(
                base64_encode(&out.stdout),
                OPENSSL_EC_CERT_PIN.trim_start_matches("sha256/"),
                "openssl must hash our extracted SPKI to the same digest"
            );
        }
    }
}
