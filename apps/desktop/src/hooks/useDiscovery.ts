import { useState, useCallback, useEffect, useReducer } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '../lib/tauri';
import { WS_PORT } from '../config';
import type { WebSocketMessage } from '../types/websocket';

// ─── Tauri event names (src-tauri/src/discovery.rs) ───────────────────────────

/** Emitted by `DiscoveryService::browse_devices` on `ServiceResolved`. */
export const MDNS_DISCOVERED_EVENT = 'mdns-device-discovered';

/**
 * Emitted by `DiscoveryService::browse_devices` on `ServiceRemoved`.
 *
 * The payload is the mDNS **fullname** (e.g. `Conduit._conduit._tcp.local.`),
 * not a device id — see {@link removeDeviceByService} for how removal is keyed.
 */
export const MDNS_REMOVED_EVENT = 'mdns-device-removed';

// ─── Payload shapes ───────────────────────────────────────────────────────────

/**
 * Payload of {@link MDNS_DISCOVERED_EVENT}, mirroring the Rust
 * `discovery::DiscoveredDevice` struct (discovery.rs:17-25).
 */
export interface MdnsDiscoveredPayload {
  device_id: string;
  name: string;
  address: string;
  port: number;
  device_type: string;
  version: string;
  /**
   * mDNS fullname for the resolved service.
   *
   * NOT part of the serialised Rust struct today: `discovery.rs:148-156` builds
   * the payload from `info.get_hostname()` and never serialises
   * `info.get_fullname()`. Modelled as optional and captured when present, so
   * the removal path starts working the moment the Rust side adds it — no
   * further frontend change required.
   */
  service_name?: string;
}

/** A device seen on the LAN, from either the mDNS events or a WS announcement. */
export interface DiscoveredDevice {
  /** Stable identity: the announced `device_id`. */
  id: string;
  name: string;
  type: string;
  /** mDNS-resolved address. Empty when unknown (WS announcements carry none). */
  address: string;
  port: number;
  /**
   * mDNS fullname captured at discovery time — the only identifier
   * {@link MDNS_REMOVED_EVENT} is able to carry. `null` when the backend did
   * not supply one.
   */
  serviceName: string | null;
  source: 'mdns' | 'announce';
}

// ─── Event reducer (pure — unit tested directly) ──────────────────────────────

/** Discriminated input to {@link reduceDiscovery}. */
export type DiscoveryEvent =
  | { kind: 'discovered'; device: DiscoveredDevice }
  | { kind: 'removed'; serviceName: string }
  | { kind: 'cleared' };

/**
 * mDNS fullnames are compared case-insensitively and with a trailing-dot
 * difference tolerated: `Conduit._conduit._tcp.local` and
 * `Conduit._conduit._tcp.local.` name the same service.
 */
function normaliseServiceName(serviceName: string): string {
  return serviceName.trim().toLowerCase().replace(/\.+$/, '');
}

/**
 * Removes the device identified by a removal event, or returns the list
 * unchanged when nothing matches.
 *
 * `mdns-device-removed` carries a service fullname, not a device id, so removal
 * is keyed on the fullname captured at discovery. Two fallbacks keep the list
 * from going stale while that capture is unavailable:
 *
 *  1. the removal payload equals a device's `device_id` — which is what the
 *     WebSocket `discovery/remove` message carries, and what the mDNS event
 *     would carry if discovery.rs:173-178 were fixed to include it;
 *  2. the first label of the fullname (the mDNS *instance* name) equals the
 *     device name — true for any device registered as
 *     `ServiceInfo::new(type, device_name, …)`, which is what discovery.rs:62-67
 *     does. Deliberately not attempted once a fullname was captured, so a
 *     precise match is never second-guessed by a fuzzy one.
 *
 * An unmatched removal is a no-op, not a wipe: guessing wrong would hide live
 * devices, and the caller has `clearDiscovered` for an explicit reset.
 */
export function removeDeviceByService(
  devices: readonly DiscoveredDevice[],
  serviceName: string,
): readonly DiscoveredDevice[] {
  const target = normaliseServiceName(serviceName);
  if (!target) return devices;

  const exact = devices.find(
    (d) => d.serviceName !== null && normaliseServiceName(d.serviceName) === target,
  );
  if (exact) return devices.filter((d) => d.id !== exact.id);

  const byDeviceId = devices.find((d) => normaliseServiceName(d.id) === target);
  if (byDeviceId) return devices.filter((d) => d.id !== byDeviceId.id);

  const instanceLabel = target.split('.')[0];
  const byInstance = devices.find(
    (d) => d.serviceName === null && normaliseServiceName(d.name) === instanceLabel,
  );
  if (byInstance) return devices.filter((d) => d.id !== byInstance.id);

  return devices;
}

/**
 * Pure state transition for the discovered-device list.
 *
 * - `discovered` upserts by `device_id`, so a re-resolve (address or port
 *   change, periodic mDNS refresh) updates in place instead of duplicating.
 * - `removed` drops the device matching the removal key — see
 *   {@link removeDeviceByService}.
 * - `cleared` empties the list.
 */
export function reduceDiscovery(
  state: readonly DiscoveredDevice[],
  event: DiscoveryEvent,
): readonly DiscoveredDevice[] {
  switch (event.kind) {
    case 'discovered': {
      const incoming = event.device;
      const index = state.findIndex((d) => d.id === incoming.id);
      if (index === -1) return [...state, incoming];
      const next = state.slice();
      next[index] = incoming;
      return next;
    }
    case 'removed':
      return removeDeviceByService(state, event.serviceName);
    case 'cleared':
      return [];
  }
}

// ─── Hook ─────────────────────────────────────────────────────────────────────

export function useDiscovery() {
  const [discoveredDevices, dispatch] = useReducer(reduceDiscovery, []);
  const [isDiscovering, setIsDiscovering] = useState(false);

  const startDiscovery = useCallback(async () => {
    try {
      await invoke('start_discovery');
      setIsDiscovering(true);
    } catch (err) {
      console.error('Failed to start discovery:', err);
    }
  }, []);

  const stopDiscovery = useCallback(async () => {
    try {
      await invoke('stop_discovery');
      setIsDiscovering(false);
    } catch (err) {
      console.error('Failed to stop discovery:', err);
    }
  }, []);

  /**
   * Subscribe to the mDNS events emitted by the Rust discovery service.
   *
   * This is the only transport `discovery.rs` uses — before this existed the
   * frontend never called `listen()` at all, so every discovered device was
   * dropped on the floor. Both unlisten functions run on unmount, and the
   * `disposed` guard covers a subscription that resolves *after* unmount
   * (Tauri resolves listeners asynchronously).
   */
  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];

    const subscribe = async () => {
      try {
        const [unlistenDiscovered, unlistenRemoved] = await Promise.all([
          listen<MdnsDiscoveredPayload>(MDNS_DISCOVERED_EVENT, (event) => {
            const payload = event.payload;
            if (!payload.device_id) return;
            dispatch({
              kind: 'discovered',
              device: {
                id: payload.device_id,
                name: payload.name || 'Unknown',
                type: payload.device_type || 'unknown',
                address: payload.address || '',
                // Rust sends a u16; fall back to the protocol default so the UI
                // never renders "port 0".
                port: payload.port || WS_PORT,
                serviceName: payload.service_name ?? null,
                source: 'mdns',
              },
            });
          }),
          listen<string | { device_id?: string }>(MDNS_REMOVED_EVENT, (event) => {
            const payload = event.payload;
            // Today a bare fullname string; tolerate an object payload so the
            // reducer works unchanged if the Rust event is corrected to carry
            // a device_id.
            const key = typeof payload === 'string' ? payload : payload.device_id ?? '';
            if (!key) return;
            dispatch({ kind: 'removed', serviceName: key });
          }),
        ]);

        if (disposed) {
          unlistenDiscovered();
          unlistenRemoved();
          return;
        }
        unlisteners.push(unlistenDiscovered, unlistenRemoved);
      } catch (err) {
        // Outside a Tauri window (tests, a plain browser) the event plugin is
        // unavailable. Discovery is best-effort; do not crash the app.
        console.error('[useDiscovery] Failed to subscribe to mDNS events:', err);
      }
    };

    void subscribe();

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        try { unlisten(); } catch { /* the listener is already gone */ }
      }
      unlisteners.length = 0;
    };
  }, []);

  /**
   * Discovery announcements that arrive over the WebSocket instead of mDNS.
   *
   * A real second transport — `websocket_service.dart:378` announces on every
   * connect — so it stays. Fields come from the generated
   * `DiscoveryAnnounceMessage`; the old `as any` casts for `device_name`,
   * `name`, `device_type`, `address` and `port` were covering either fields
   * the schema already declares or one (`address`) that it does not have.
   */
  const handleDiscoveryMessage = useCallback((data: WebSocketMessage) => {
    if (data.type !== 'discovery') return;

    if (data.action === 'remove') {
      dispatch({ kind: 'removed', serviceName: data.device_id });
      return;
    }

    // Narrowed to DiscoveryAnnounceMessage: this variant has no `name` and no
    // `address` on the wire.
    dispatch({
      kind: 'discovered',
      device: {
        id: data.device_id,
        name: data.device_name || 'Unknown',
        type: data.device_type,
        address: '',
        port: data.ws_port || WS_PORT,
        serviceName: null,
        source: 'announce',
      },
    });
  }, []);

  const clearDiscovered = useCallback(() => {
    dispatch({ kind: 'cleared' });
  }, []);

  return {
    discoveredDevices,
    isDiscovering,
    startDiscovery,
    stopDiscovery,
    handleDiscoveryMessage,
    clearDiscovered,
  };
}
