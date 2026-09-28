# Build from the REPOSITORY ROOT context (render.yaml / compose / CI all pass
# "." as the context): the relay is a member of the root Cargo workspace and
# depends on the sibling crate packages/protocol, neither of which is visible
# from a services/relay-only context.
#   docker build -f services/relay/Dockerfile -t conduit-relay .
FROM rust:1.98-alpine AS builder
WORKDIR /workspace

# Workspace manifests. apps/desktop/src-tauri/Cargo.toml is required so the
# virtual workspace resolves; the desktop app itself is never compiled here
# (its placeholder src/ below only satisfies target auto-discovery).
COPY Cargo.toml Cargo.lock ./
COPY services/relay/Cargo.toml services/relay/
COPY packages/protocol/Cargo.toml packages/protocol/
COPY apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/

# Warm dependency artifacts with placeholder sources, then swap in the real
# relay/protocol sources so only those two crates recompile afterwards.
RUN mkdir -p services/relay/src packages/protocol/src apps/desktop/src-tauri/src \
    && printf 'fn main() {}\n' > services/relay/src/main.rs \
    && printf '' > packages/protocol/src/lib.rs \
    && printf 'fn main() {}\n' > apps/desktop/src-tauri/src/main.rs \
    && cargo build --release --locked -p relay \
    && rm -rf services/relay/src packages/protocol/src target/release/relay

COPY services/relay/src services/relay/src
COPY packages/protocol/src packages/protocol/src
RUN cargo build --release --locked -p relay

FROM alpine:3.20
RUN apk add --no-cache libgcc ca-certificates
RUN adduser -D -u 1001 relay
# Workspace builds place the binary in the ROOT target/, not services/relay/target.
COPY --from=builder /workspace/target/release/relay /usr/local/bin/relay
# Persistent state under /data, owned by relay. EVERY path the relay writes to
# must be a volume in docker-compose.yml or the state is lost on recreate:
#   /data/secrets/hmac_secret  master secret
#   /data/nonces.json          replay-protection cache (flushed every 30 s)
#   /data/certs/{cert,key}.pem TLS material
RUN mkdir -p /data/secrets /data/certs \
    && chown -R relay:relay /data
# Default paths, overridable via env. RELAY_CERT_DIR is explicit so the relay
# no longer depends on WORKDIR being /data (it used CWD-relative "./certs",
# which only worked by accident).
ENV HMAC_SECRET_FILE=/data/secrets/hmac_secret \
    RELAY_NONCE_FILE=/data/nonces.json \
    RELAY_CERT_DIR=/data/certs
WORKDIR /data
USER relay
EXPOSE 9528 9529 9530
# start-time is longer than interval because first boot generates a self-signed
# certificate (and possibly the secret file) before the health listener binds.
HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 \
    CMD wget -q --spider http://localhost:9530/healthz || exit 1
CMD ["relay"]
