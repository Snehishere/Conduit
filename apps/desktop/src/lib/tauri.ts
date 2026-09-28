import { invoke as tauriInvoke } from '@tauri-apps/api/core';

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

export async function invokeWithTimeout<T>(cmd: string, args?: Record<string, unknown>, timeoutMs = 30000): Promise<T> {
  return Promise.race([
    tauriInvoke<T>(cmd, args),
    new Promise<T>((_, reject) => setTimeout(() => { reject(new Error(`Command ${cmd} timed out after ${timeoutMs}ms`)); }, timeoutMs)),
  ]);
}

/**
 * Typed wrapper for the `get_current_version` Tauri command.
 *
 * Returns the desktop app's semantic version, sourced from the Rust
 * crate (`env!("CARGO_PKG_VERSION")` in `src-tauri/src/commands/system.rs`).
 * Failures are expected to be handled by the caller — the version display
 * is non-critical UI.
 */
export function getCurrentVersion(): Promise<string> {
  return invoke<string>('get_current_version');
}

/**
 * Open a file this device received, by **transfer id**.
 *
 * There is deliberately no frontend helper that opens a caller-supplied path.
 * The `file/complete` protocol frame carries a `path` chosen by the *sending*
 * device, so passing that string to an `open()`-style API let any paired peer
 * choose what this machine launched — including a `\\attacker\share\file` UNC
 * path, which on Windows makes the shell make an outbound SMB connection and
 * leak the local NTLM hash, and whose success or failure the peer could read
 * back as a side channel.
 *
 * `open_downloaded_file` takes an id, resolves the path from local state
 * (the `file_transfers` row and the transfer engine's own record of what it
 * wrote), and refuses anything that does not resolve to a regular file inside
 * the configured download folder. That is why `shell:allow-open` is no longer
 * in `capabilities/default.json`.
 */
export function openDownloadedFile(transferId: string): Promise<void> {
  return invoke<void>('open_downloaded_file', { id: transferId });
}

