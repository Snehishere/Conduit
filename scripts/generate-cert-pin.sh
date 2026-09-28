#!/usr/bin/env bash
# ============================================================================
#  generate-cert-pin.sh — print the Conduit relay's certificate pin.
#
#  PIN ALGORITHM: sha256 over the certificate's SubjectPublicKeyInfo (SPKI),
#  base64-encoded, prefixed with "sha256/".
#
#  Why SPKI and not the whole certificate:
#    * The SPKI covers only the public key, so the pin SURVIVES certificate
#      renewal as long as the key pair is reused. A whole-certificate pin has
#      to be redistributed on every renewal, which trains operators to disable
#      pinning.
#    * `sha256(SPKI-DER) != sha256(cert-DER)`, so the two are not
#      interchangeable. An earlier version of this script printed the SPKI hash
#      while the only client that implemented pinning hashed `cert.der` — the
#      documented pin could therefore never match, and every connection was
#      rejected. Pick one; this is the one.
#
#  The relay itself publishes the same value at GET /pin, computed from the
#  certificate it actually loaded. This script prefers that endpoint and falls
#  back to openssl only if the endpoint is unreachable.
#
#  Usage:
#    scripts/generate-cert-pin.sh <relay-host> [port] [--health-port N] [--health-token T]
#
#  Examples:
#    scripts/generate-cert-pin.sh relay.example.com
#    scripts/generate-cert-pin.sh relay.example.com 9529
#    scripts/generate-cert-pin.sh localhost 9529 --health-port 9530
#    scripts/generate-cert-pin.sh localhost 9529 --health-token "$RELAY_HEALTH_TOKEN"
# ============================================================================
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
log()   { echo -e "${GREEN}[cert-pin]${NC} $1"; }
warn()  { echo -e "${YELLOW}[warn]${NC} $1"; }
error() { echo -e "${RED}[error]${NC} $1" >&2; exit 1; }

usage() {
    sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
}

HOST=""
PORT="9529"
HEALTH_PORT=""
HEALTH_TOKEN=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --health-port)  HEALTH_PORT="${2:?--health-port needs a value}"; shift 2 ;;
        --health-token) HEALTH_TOKEN="${2:?--health-token needs a value}"; shift 2 ;;
        -h|--help)      usage ;;
        -*)             error "Unknown option: $1 (try --help)" ;;
        *)
            if [[ -z "$HOST" ]]; then HOST="$1"
            elif [[ "$PORT" == "9529" ]]; then PORT="$1"
            else error "Too many positional arguments (host and port only)"
            fi
            shift ;;
    esac
done

[[ -n "$HOST" ]] || usage
[[ -n "$HEALTH_PORT" ]] || HEALTH_PORT="9530"

command -v openssl >/dev/null 2>&1 || error "openssl is required"

# ----------------------------------------------------------------------------
#  1. Prefer the relay's own /pin endpoint.
# ----------------------------------------------------------------------------
PIN=""
if [[ -n "$HEALTH_PORT" ]]; then
    AUTH_ARGS=()
    [[ -n "$HEALTH_TOKEN" ]] && AUTH_ARGS=(-H "Authorization: Bearer $HEALTH_TOKEN")

    if command -v curl >/dev/null 2>&1; then
        log "Querying the relay's /pin endpoint on 127.0.0.1:$HEALTH_PORT ..."
        BODY=$(curl -fsS --max-time 5 "${AUTH_ARGS[@]}" \
                 "http://127.0.0.1:$HEALTH_PORT/pin" 2>/dev/null || true)
        if [[ -n "$BODY" ]]; then
            PIN=$(printf '%s' "$BODY" \
                  | sed -n 's/.*"sha256"[[:space:]]*:[[:space:]]*"\(sha256\/[A-Za-z0-9+/=]*\)".*/\1/p' \
                  | head -n1)
            if [[ -z "$PIN" ]]; then
                warn "/pin responded but carried no sha256 value; falling back to openssl"
            else
                log "Using the pin the relay reports for its loaded certificate."
            fi
        fi
    fi
fi

# ----------------------------------------------------------------------------
#  2. openssl fallback: connect, extract, hash the SPKI.
# ----------------------------------------------------------------------------
CERT_FILE=""
if [[ -z "$PIN" ]]; then
    command -v openssl >/dev/null 2>&1 || error "openssl is required"
    log "Fetching the certificate from $HOST:$PORT ..."
    CERT_FILE=$(mktemp)
    trap 'rm -f "$CERT_FILE"' EXIT
    # `openssl s_client` is the right tool here: the relay speaks WebSocket, not
    # HTTP, so `curl` cannot retrieve its certificate. -showcerts prints the
    # chain; the relay's leaf is the first certificate.
    if ! echo | openssl s_client -connect "$HOST:$PORT" -servername "$HOST" \
            -showcerts 2>/dev/null \
         | openssl x509 -outform PEM > "$CERT_FILE" 2>/dev/null; then
        error "Failed to fetch a certificate from $HOST:$PORT (is WSS enabled on that port?)"
    fi
    if ! grep -q 'BEGIN CERTIFICATE' "$CERT_FILE"; then
        error "No certificate returned by $HOST:$PORT — wrong port, or TLS is not listening"
    fi
fi

# ----------------------------------------------------------------------------
#  3. Hostname validation — refuse to print a pin for a MITM-presented cert.
# ----------------------------------------------------------------------------
if [[ -n "$CERT_FILE" ]]; then
    if openssl x509 -in "$CERT_FILE" -noout -checkhost "$HOST" >/dev/null 2>&1; then
        log "openssl x509 -checkhost $HOST: OK"
    else
        warn "openssl x509 -checkhost $HOST FAILED — this certificate is NOT valid"
        warn "for $HOST. Pinning it would lock clients to whatever presented it."
        warn "Resolve the name mismatch (RELAY_TLS_HOSTNAME / certificate SANs) before pinning."
        exit 2
    fi

    if ! openssl x509 -in "$CERT_FILE" -noout -checkend 0 >/dev/null 2>&1; then
        warn "the certificate is expired or not yet valid"
    fi

    PIN=$(openssl x509 -in "$CERT_FILE" -pubkey -noout \
          | openssl pkey -pubin -outform DER 2>/dev/null \
          | openssl dgst -sha256 -binary \
          | openssl base64 -A)
    [[ -n "$PIN" ]] || error "Failed to compute the SPKI hash"
    PIN="sha256/$PIN"
    log "Computed the SPKI hash locally (the /pin endpoint was not reachable)."
fi

[[ "$PIN" =~ ^sha256/[A-Za-z0-9+/=]+$ ]] || error "Refusing to print a malformed pin: $PIN"

log "Certificate pin (sha256 over SubjectPublicKeyInfo):"
echo ""
echo -e "${GREEN}${PIN}${NC}"
echo ""
cat <<EOF
Add this to your client configuration:

  mobile  (Dart)   setPinnedCertificate('${PIN}')
                   -> websocket_service.dart must hash the certificate's
                      SubjectPublicKeyInfo, NOT the whole DER certificate.
                      sha256(SPKI) != sha256(cert.der); hashing cert.der
                      produces a different value that never matches.

  desktop (Rust)   no config target exists yet for the pin: the project
                   deliberately removed VITE_RELAY_CERT_SHA256 (see
                   apps/desktop/src/vite-env.d.ts). Until one is added,
                   treat the mobile pin as authoritative and re-check
                   GET /pin after every certificate renewal.

Verify later with:
  curl -s http://127.0.0.1:${HEALTH_PORT}/pin
EOF
