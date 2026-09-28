/**
 * Conduit Desktop — Centralized Frontend Configuration
 *
 * All magic strings and environment-dependent values live here.
 * Never scatter raw port numbers or URLs across component files.
 */

// ─── WebSocket ────────────────────────────────────────────────────────────────

/** Port the Rust backend WebSocket server listens on. Must match WS_PORT in main.rs. */
export const WS_PORT = 9_527;

/** Port the Rust backend WSS server listens on for LAN TLS connections. Must match WSS_PORT in main.rs. */
export const WSS_PORT = 9_531;

/**
 * Base WebSocket URL for the local Conduit backend.
 * Uses `ws://` (plain) for local loopback connections.
 */
export const WS_URL = `ws://127.0.0.1:${WS_PORT}`;

// ─── App identity ─────────────────────────────────────────────────────────────

/** Reverse-DNS bundle identifier — must match tauri.conf.json > identifier. */
export const APP_ID = 'com.conduit.app';

/** Human-facing product name. */
export const APP_NAME = 'Conduit';

// ─── Reconnect tuning ─────────────────────────────────────────────────────────

/** Initial WebSocket reconnect delay in milliseconds (doubles on each failure). */
export const WS_RECONNECT_BASE_MS = 1_000;

/** Maximum WebSocket reconnect delay in milliseconds. */
export const WS_RECONNECT_MAX_MS = 30_000;

// ─── File transfer ────────────────────────────────────────────────────────────

/** Chunk size in bytes — must match CHUNK_SIZE in file_transfer.rs. */
export const FILE_CHUNK_SIZE_BYTES = 64 * 1_024; // 64 KB
