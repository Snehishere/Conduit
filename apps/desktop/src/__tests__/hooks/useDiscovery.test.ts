import { describe, it, expect } from 'vitest';
import {
  reduceDiscovery,
  removeDeviceByService,
  type DiscoveredDevice,
  type DiscoveryEvent,
} from '../../hooks/useDiscovery';

// ─── Fixtures ────────────────────────────────────────────────────────────────

function makeDevice(overrides: Partial<DiscoveredDevice> = {}): DiscoveredDevice {
  return {
    id: 'dev-1',
    name: 'Pixel 7',
    type: 'phone',
    address: '192.168.1.5',
    port: 9527,
    serviceName: 'Pixel 7._conduit._tcp.local.',
    source: 'mdns',
    ...overrides,
  };
}

function discovered(device: DiscoveredDevice): DiscoveryEvent {
  return { kind: 'discovered', device };
}

const apply = (state: readonly DiscoveredDevice[], ...events: DiscoveryEvent[]) =>
  events.reduce<readonly DiscoveredDevice[]>(reduceDiscovery, state);

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('reduceDiscovery', () => {
  // ── discovered ─────────────────────────────────────────────────────────────

  it('appends a newly seen device', () => {
    const next = reduceDiscovery([], discovered(makeDevice()));
    expect(next).toHaveLength(1);
    expect(next[0].id).toBe('dev-1');
    expect(next[0].serviceName).toBe('Pixel 7._conduit._tcp.local.');
  });

  it('preserves the order devices were first seen in', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'a' })),
      discovered(makeDevice({ id: 'b' })),
      discovered(makeDevice({ id: 'c' })),
    );
    expect(next.map((d) => d.id)).toEqual(['a', 'b', 'c']);
  });

  it('upserts by device_id instead of duplicating on re-resolve', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'dev-1', address: '192.168.1.5' })),
      discovered(makeDevice({ id: 'dev-1', address: '192.168.1.99' })),
    );
    expect(next).toHaveLength(1);
    expect(next[0].address).toBe('192.168.1.99');
  });

  it('keeps the original position when a device re-announces', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'a' })),
      discovered(makeDevice({ id: 'b' })),
      discovered(makeDevice({ id: 'a', name: 'A renamed' })),
    );
    expect(next.map((d) => d.id)).toEqual(['a', 'b']);
    expect(next[0].name).toBe('A renamed');
  });

  it('returns a new array rather than mutating the previous state', () => {
    const before: DiscoveredDevice[] = [makeDevice()];
    const after = reduceDiscovery(before, discovered(makeDevice({ id: 'dev-2' })));
    expect(after).not.toBe(before);
    expect(before).toHaveLength(1);
  });

  // ── removed: keyed on the mDNS service name ───────────────────────────────

  it('removes a device by its captured mDNS service name', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'a', serviceName: 'Pixel._conduit._tcp.local.' })),
      discovered(makeDevice({ id: 'b', serviceName: 'Tablet._conduit._tcp.local.' })),
      { kind: 'removed', serviceName: 'Pixel._conduit._tcp.local.' },
    );
    expect(next.map((d) => d.id)).toEqual(['b']);
  });

  it('matches the service name case-insensitively', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'a', serviceName: 'Pixel._conduit._tcp.local.' })),
      { kind: 'removed', serviceName: 'pixel._CONDUIT._tcp.local' },
    );
    expect(next).toHaveLength(0);
  });

  it('tolerates a trailing-dot difference in the service name', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'a', serviceName: 'Pixel._conduit._tcp.local.' })),
      { kind: 'removed', serviceName: 'Pixel._conduit._tcp.local' },
    );
    expect(next).toHaveLength(0);
  });

  // ── removed: fallbacks ────────────────────────────────────────────────────

  it('falls back to matching the mDNS instance label against the device name', () => {
    // The Rust payload carries no fullname today, so removal arrives with only
    // the service name. The instance label is the device name.
    const next = apply(
      [],
      discovered(makeDevice({ id: 'a', name: 'Pixel 7', serviceName: null })),
      { kind: 'removed', serviceName: 'Pixel 7._conduit._tcp.local.' },
    );
    expect(next).toHaveLength(0);
  });

  it('removes by device_id, which is what the WebSocket remove message carries', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'dev-42', serviceName: null })),
      { kind: 'removed', serviceName: 'dev-42' },
    );
    expect(next).toHaveLength(0);
  });

  it('prefers the captured service name over the instance-label heuristic', () => {
    const withName = makeDevice({ id: 'a', name: 'b', serviceName: 'a._conduit._tcp.local.' });
    const other = makeDevice({ id: 'b', name: 'b', serviceName: null });
    const next = apply([], discovered(withName), discovered(other), {
      kind: 'removed',
      serviceName: 'a._conduit._tcp.local.',
    });
    expect(next.map((d) => d.id)).toEqual(['b']);
  });

  it('leaves the list untouched when the removal matches nothing', () => {
    const state = apply([], discovered(makeDevice({ id: 'a' })));
    const next = reduceDiscovery(state, { kind: 'removed', serviceName: 'ghost._conduit._tcp.local.' });
    expect(next).toHaveLength(1);
    expect(next[0].id).toBe('a');
  });

  it('ignores an empty removal key instead of clearing the list', () => {
    const state = apply([], discovered(makeDevice({ id: 'a' })), discovered(makeDevice({ id: 'b' })));
    expect(reduceDiscovery(state, { kind: 'removed', serviceName: '   ' })).toHaveLength(2);
  });

  // ── cleared ───────────────────────────────────────────────────────────────

  it('cleared empties the list', () => {
    const state = apply([], discovered(makeDevice({ id: 'a' })), discovered(makeDevice({ id: 'b' })));
    expect(reduceDiscovery(state, { kind: 'cleared' })).toEqual([]);
  });

  // ── announce-sourced devices ──────────────────────────────────────────────

  it('stores announce-sourced devices with no service name and no address', () => {
    const next = reduceDiscovery([], discovered(makeDevice({
      source: 'announce',
      serviceName: null,
      address: '',
      port: 9527,
    })));
    expect(next[0].source).toBe('announce');
    expect(next[0].serviceName).toBeNull();
    expect(next[0].address).toBe('');
  });

  it('removes an announce-sourced device by device_id', () => {
    const next = apply(
      [],
      discovered(makeDevice({ id: 'mobile_1', source: 'announce', serviceName: null })),
      { kind: 'removed', serviceName: 'mobile_1' },
    );
    expect(next).toHaveLength(0);
  });
});

describe('removeDeviceByService', () => {
  it('returns the same array reference when nothing matches', () => {
    const devices = [makeDevice({ id: 'a' })];
    expect(removeDeviceByService(devices, 'nope._conduit._tcp.local.')).toBe(devices);
  });

  it('does not throw on an empty device list', () => {
    expect(removeDeviceByService([], 'anything._conduit._tcp.local.')).toEqual([]);
  });
});
