import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

// ─── Mocks ────────────────────────────────────────────────────────────────────

const mockInvoke = vi.fn();

vi.mock('../../lib/tauri', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

/**
 * Every registered listener, keyed by the Tauri event name it was registered
 * for. This is the seam `discovery.rs` reaches through: it emits
 * `mdns-device-discovered` with `serde_json::to_value(DiscoveredDevice)`, and
 * the hook's only job is to turn that into `discoveredDevices`.
 */
const listeners = new Map<string, (event: { payload: unknown }) => void>();

const unlisten = vi.fn();

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(
    async (event: string, handler: (e: { payload: unknown }) => void) => {
      listeners.set(event, handler);
      return unlisten;
    },
  ),
}));

import {
  useDiscovery,
  MDNS_DISCOVERED_EVENT,
  MDNS_REMOVED_EVENT,
} from '../../hooks/useDiscovery';

/**
 * The event names `discovery.rs` emits, spelled out as literals on purpose.
 *
 * `discovery.rs:229` emits `"mdns-device-discovered"` and `discovery.rs:248`
 * emits `"mdns-device-removed"`. Asserting against the exported constants would
 * only prove the hook agrees with itself; comparing against these literals is
 * what pins the Rust → TypeScript contract. Rename either side and the
 * subscription silently stops firing, dropping every peer on the floor.
 */
const RUST_DISCOVERED_EVENT = 'mdns-device-discovered';
const RUST_REMOVED_EVENT = 'mdns-device-removed';

// ─── Fixtures ────────────────────────────────────────────────────────────────

/**
 * Exactly what `discovery::DiscoveredDevice` serialises to — snake_case field
 * names, and no `service_name` (the Rust struct has no such field, so the
 * removal path can only match on the mDNS instance label).
 */
function rustDiscoveredPayload(overrides: Record<string, unknown> = {}) {
  return {
    device_id: 'dev-1',
    name: 'Pixel 7',
    address: '192.168.1.5',
    port: 9527,
    device_type: 'phone',
    version: '1.0.0',
    ...overrides,
  };
}

async function mountHook() {
  const hook = renderHook(() => useDiscovery());
  // Let the two `listen()` promises inside the mount effect resolve.
  await act(async () => {
    await Promise.resolve();
  });
  return hook;
}

function emit(event: string, payload: unknown) {
  const handler = listeners.get(event);
  if (!handler) throw new Error(`nothing is listening for "${event}"`);
  act(() => {
    handler({ payload });
  });
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('useDiscovery — mDNS results reach the caller', () => {
  beforeEach(() => {
    listeners.clear();
    mockInvoke.mockReset();
    unlisten.mockReset();
    vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  it('subscribes to exactly the event names discovery.rs emits', () => {
    expect(MDNS_DISCOVERED_EVENT).toBe(RUST_DISCOVERED_EVENT);
    expect(MDNS_REMOVED_EVENT).toBe(RUST_REMOVED_EVENT);
  });

  it('subscribes to both discovery events on mount', async () => {
    await mountHook();
    expect([...listeners.keys()].sort()).toEqual(
      [RUST_DISCOVERED_EVENT, RUST_REMOVED_EVENT].sort(),
    );
  });

  it('turns an mdns-device-discovered payload into an entry in discoveredDevices', async () => {
    const { result } = await mountHook();
    expect(result.current.discoveredDevices).toEqual([]);

    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload());

    expect(result.current.discoveredDevices).toHaveLength(1);
    expect(result.current.discoveredDevices[0]).toMatchObject({
      id: 'dev-1',
      name: 'Pixel 7',
      type: 'phone',
      address: '192.168.1.5',
      port: 9527,
      source: 'mdns',
    });
  });

  it('surfaces every peer it is told about, not just the first', async () => {
    const { result } = await mountHook();

    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload({ device_id: 'dev-1' }));
    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload({ device_id: 'dev-2', name: 'iPad' }));

    expect(result.current.discoveredDevices.map((d) => d.id)).toEqual(['dev-1', 'dev-2']);
  });

  it('drops a payload with no device_id rather than showing a nameless device', async () => {
    const { result } = await mountHook();
    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload({ device_id: '' }));
    expect(result.current.discoveredDevices).toEqual([]);
  });

  it('keeps the discovered list when a peer has no name, address or type', async () => {
    // discovery.rs falls back to device_type "unknown" and an empty address when
    // the TXT record is thin; the hook must not drop the peer over it.
    const { result } = await mountHook();
    emit(
      RUST_DISCOVERED_EVENT,
      rustDiscoveredPayload({ name: '', address: '', device_type: '', version: '' }),
    );
    expect(result.current.discoveredDevices).toHaveLength(1);
    expect(result.current.discoveredDevices[0]).toMatchObject({
      id: 'dev-1',
      name: 'Unknown',
      type: 'unknown',
      address: '',
    });
  });

  it('removes an entry on mdns-device-removed', async () => {
    const { result } = await mountHook();
    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload());
    expect(result.current.discoveredDevices).toHaveLength(1);

    // Rust emits the mDNS fullname on removal (`handle.emit(..., fullname)`).
    emit(RUST_REMOVED_EVENT, 'Pixel 7._conduit._tcp.local.');

    expect(result.current.discoveredDevices).toEqual([]);
  });

  it('unlistens on unmount', async () => {
    const { unmount } = await mountHook();
    unmount();
    expect(unlisten).toHaveBeenCalled();
  });

  it('startDiscovery invokes the start_discovery command and flips isDiscovering', async () => {
    mockInvoke.mockResolvedValue(undefined);
    const { result } = await mountHook();
    expect(result.current.isDiscovering).toBe(false);

    await act(async () => {
      await result.current.startDiscovery();
    });

    expect(mockInvoke).toHaveBeenCalledWith('start_discovery');
    expect(result.current.isDiscovering).toBe(true);
  });

  it('stopDiscovery invokes stop_discovery and clears isDiscovering', async () => {
    mockInvoke.mockResolvedValue(undefined);
    const { result } = await mountHook();
    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload());

    await act(async () => {
      await result.current.stopDiscovery();
    });

    expect(mockInvoke).toHaveBeenCalledWith('stop_discovery');
    expect(result.current.isDiscovering).toBe(false);
    // Stopping the daemon is not a reset: the peers already on screen stay put
    // until `clearDiscovered` or a removal event removes them.
    expect(result.current.discoveredDevices).toHaveLength(1);
  });

  it('clearDiscovered empties the list without telling the backend', async () => {
    const { result } = await mountHook();
    emit(RUST_DISCOVERED_EVENT, rustDiscoveredPayload());

    act(() => {
      result.current.clearDiscovered();
    });

    expect(result.current.discoveredDevices).toEqual([]);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it('a WebSocket discovery announce also reaches discoveredDevices', async () => {
    const { result } = await mountHook();

    act(() => {
      result.current.handleDiscoveryMessage({
        type: 'discovery',
        action: 'announce',
        device_id: 'dev-ws',
        device_name: 'Work Laptop',
        device_type: 'desktop',
      } as never);
    });

    expect(result.current.discoveredDevices).toHaveLength(1);
    expect(result.current.discoveredDevices[0]).toMatchObject({
      id: 'dev-ws',
      name: 'Work Laptop',
      type: 'desktop',
      source: 'announce',
    });
  });
});
