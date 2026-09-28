# Relay TLS runbook

Everything about the relay's certificate, hostname and certificate pinning.

- [1. The two things that must agree](#1-the-two-things-that-must-agree)
- [2. Pin algorithm](#2-pin-algorithm)
- [3. Default deployment (self-signed)](#3-default-deployment-self-signed)
- [4. Bring your own certificate](#4-bring-your-own-certificate)
- [5. Let's Encrypt / ACME](#5-lets-encrypt--acme)
- [6. Renewal](#6-renewal)
- [7. Distributing the pin to clients](#7-distributing-the-pin-to-clients)
- [8. Troubleshooting](#8-troubleshooting)

---

## 1. The two things that must agree

Two independent pieces of configuration decide whether a client can connect:

| Concern | Relay setting | Client setting |
|---|---|---|
| Name in the certificate | `RELAY_TLS_HOSTNAME` (+ `RELAY_TLS_EXTRA_SANS`) | the hostname in the relay URL |
| Certificate trust | `RELAY_CERT_DIR` | the pin from `GET /pin` |

If the first pair disagrees, the client fails **hostname verification**. If the
second pair disagrees, the client fails **pin verification**. The two failures
look identical from the client ("certificate error"), so check both.

---

## 2. Pin algorithm

**`sha256/` + base64( SHA-256( DER SubjectPublicKeyInfo ) )**

The pin covers the certificate's **public key only**, not the whole
certificate. That is deliberate:

- The pin survives certificate renewal as long as the key pair is reused. A
  whole-certificate pin must be redistributed at every renewal, which is how
  teams end up disabling pinning.
- `sha256(SPKI-DER) != sha256(cert-DER)`. These are **different values**. An
  earlier version of this repo documented an SPKI pin while the only client
  that implemented pinning hashed `cert.der` — the documented pin could never
  match, and every connection was rejected.

The relay publishes its own pin at `GET /pin` (see §7). `scripts/generate-cert-pin.sh`
prefers that endpoint so there is no chance of computing a pin over the wrong
certificate.

> **Note:** the mobile client (`apps/mobile/lib/services/websocket_service.dart`)
> still hashes the whole DER certificate. It must be changed to hash the SPKI
> before mobile pinning works. The desktop app has no pin configuration target
> at all — `VITE_RELAY_CERT_SHA256` was deliberately removed from the project.

---

## 3. Default deployment (self-signed)

On first boot the relay generates a self-signed certificate containing these SANs:

- `RELAY_TLS_HOSTNAME` (default `localhost`)
- `127.0.0.1`, `::1`
- everything in `RELAY_TLS_EXTRA_SANS` (comma/space separated)

```bash
# .env
RELAY_TLS_HOSTNAME=relay.example.com
RELAY_TLS_EXTRA_SANS=relay2.example.com,10.0.0.5
```

If you do not set `RELAY_TLS_HOSTNAME`, the certificate is only valid for
`localhost` and **every** client fails hostname verification against a real
deployment name. That is the single most common cause of "it works on my
machine" relay TLS failures.

Self-signed is fine for a private deployment where clients pin the SPKI. It is
not fine if you want to put the relay behind a normal TLS-terminating load
balancer — in that case terminate TLS upstream and run the relay's plain WS
listener, or use §4/§5.

---

## 4. Bring your own certificate

Put `cert.pem` (full chain, leaf first) and `key.pem` in `RELAY_CERT_DIR`:

```bash
mkdir -p secrets/certs
cp fullchain.pem secrets/certs/cert.pem
cp privkey.pem   secrets/certs/key.pem
chmod 600 secrets/certs/key.pem

# .env
RELAY_CERT_DIR=/data/certs
```

```yaml
volumes:
  - ./secrets/certs:/data/certs:ro
```

Accepted private-key formats — **all three are auto-detected**:

| Format | PEM header | Typical producer |
|---|---|---|
| PKCS#8 | `-----BEGIN PRIVATE KEY-----` | `openssl genpkey`, certbot (modern) |
| PKCS#1 | `-----BEGIN RSA PRIVATE KEY-----` | `openssl rsa -traditional`, older certbot, Windows exports |
| SEC1 | `-----BEGIN EC PRIVATE KEY-----` | `openssl ecparam`, older tooling |

> The relay previously used `rustls_pemfile::pkcs8_private_keys`, which silently
> yielded **zero** keys for PKCS#1 and SEC1 files and then reported the
> misleading *"No private keys found in key file"* — text that `openssl
> rsa -traditional`, older certbot and Windows exports all produce by default.
> The current message names the three accepted armours and counts what it found,
> so a genuinely keyless file is now distinguishable from an unsupported one.

The relay tightens `key.pem` to `0600` on startup (unix only) and **fails
closed** if the key and certificate do not belong together.

---

## 5. Let's Encrypt / ACME

**ACME/Let's Encrypt integration is deliberately not built in.** Reasoning:

- An HTTP-01 challenge needs port 80 and a publicly reachable address; the
  relay's only listener is the WebSocket port. Supporting it properly means
  embedding a challenge responder in the relay, which is a meaningful new
  network-exposed surface.
- An `rustls-acme`-style in-process solution has to renew in the background and
  hot-swap the certificate. The relay deliberately loads its certificate once at
  startup (§6), so an in-process client would need a new reload path too.
- Running certbot (or any external ACME client) in a sidecar against the same
  `RELAY_CERT_DIR` volume is simpler, is what operators already know, and
  fails loudly in a place they can see.

Recommended ACME layout without touching the relay:

```
                 ┌────────────────────────────┐
   :80 / :443 ──►│ nginx / caddy              │
                 │  - terminates TLS           │
                 │  - serves /.well-known/     │
                 │  - proxies /  → relay:9528  │
                 └────────────────────────────┘
                 sidecar: certbot renew → writes into relay-certs volume
```

If your client pins the relay's own certificate, note that a terminating proxy
presents **its own** certificate — so pin the proxy's certificate, not the
relay's. Decide before you set up pinning which hop is authoritative.

---

## 6. Renewal

The certificate is read **once, at startup**. There is no hot reload.

```bash
# After renewing cert.pem / key.pem:
docker compose restart relay
```

The pin is stable across a renewal **as long as the key pair is reused**. If you
issue a new key, the pin changes and every client must be updated — check
`GET /pin` after any renewal.

---

## 7. Distributing the pin to clients

The relay serves the pin from the certificate it actually loaded:

```bash
curl -s http://127.0.0.1:9530/pin
# {"sha256":"sha256/0Q2m…","algorithm":"spki-sha256"}
```

`/pin` is unauthenticated, like `/healthz`, and is only published on the
loopback-mapped health port.

Or use the helper, which prefers `/pin` and falls back to openssl (and refuses
to print a pin when `openssl x509 -checkhost` says the certificate is not valid
for the requested name):

```bash
scripts/generate-cert-pin.sh relay.example.com 9529
scripts/generate-cert-pin.sh relay.example.com 9529 --health-port 9530
```

Then:

- **mobile** — Settings → pinned certificate, or
  `websocketService.setPinnedCertificate('sha256/…')`.
  **The client code must hash the SPKI, not `cert.der`** (§2).
- **desktop** — no configuration target exists today; the project removed
  `VITE_RELAY_CERT_SHA256`. Until one is added, re-check `/pin` after every
  renewal.

---

## 8. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Client rejects with a hostname/certificate error | `RELAY_TLS_HOSTNAME` not set, so the cert is only valid for `localhost` | set `RELAY_TLS_HOSTNAME`, delete `certs/` and restart to regenerate |
| Client rejects a pin it should accept | client hashes `cert.der`, pin is over the SPKI | client must hash the SPKI (§2) |
| Pin changed unexpectedly | certificate was reissued with a **new key** | reuse the existing key, or redistribute the pin |
| `No private key found in key PEM — expected one of …` | the file contains no recognised `PRIVATE KEY` block | check the file actually contains a key |
| `Failed to parse private key as RSA, ECDSA, or EdDSA` | PEM armour is recognised but the body is not a valid key | re-export with `openssl pkcs8 -topk8 -nocrypt` |
| `TLS config error` at startup | cert and key do not belong together | re-issue both from the same key |
| Pin helper exits 2 | `openssl x509 -checkhost` failed — something other than your relay answered | do **not** pin; fix DNS/hostname first |

Relevant source: `services/relay/src/tls.rs`,
`scripts/generate-cert-pin.sh`, `docker-compose.yml`, `.env.example`.
