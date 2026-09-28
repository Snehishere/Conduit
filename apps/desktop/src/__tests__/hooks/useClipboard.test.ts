import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useClipboard } from '../../hooks/useClipboard';

// ─── Mocks ────────────────────────────────────────────────────────────────────

vi.mock('../../lib/tauri', () => ({ invoke: vi.fn() }));

// Mock the dynamic import of the Tauri clipboard plugin (will throw in jsdom → falls back to navigator.clipboard)
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  readText: vi.fn().mockRejectedValue(new Error('not in tauri')),
  writeText: vi.fn().mockRejectedValue(new Error('not in tauri')),
}));

const baseOptions = () => ({
  sendMessage: vi.fn(),
  deviceId: 'desktop-1',
  onSynced: vi.fn(),
});

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('useClipboard', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Provide navigator.clipboard mock for the fallback path
    Object.defineProperty(navigator, 'clipboard', {
      value: {
        readText: vi.fn().mockResolvedValue(''),
        writeText: vi.fn().mockResolvedValue(undefined),
      },
      writable: true,
      configurable: true,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  // ── Initial state ─────────────────────────────────────────────────────────

  it('starts with an empty history', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));
    expect(result.current.history).toEqual([]);
  });

  it('starts with isMonitoring false', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));
    expect(result.current.isMonitoring).toBe(false);
  });

  // ── handleIncoming ────────────────────────────────────────────────────────

  it('handleIncoming adds entry to history', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    act(() => {
      result.current.handleIncoming({
        content: 'hello world',
        mime: 'text/plain',
        sourceDevice: 'phone-1',
        timestamp: 1000,
      });
    });

    expect(result.current.history).toHaveLength(1);
    expect(result.current.history[0].content).toBe('hello world');
    expect(result.current.history[0].sourceDevice).toBe('phone-1');
  });

  it('handleIncoming prepends entries (newest first)', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    act(() => {
      result.current.handleIncoming({
        content: 'first',
        mime: 'text/plain',
        sourceDevice: 'phone-1',
        timestamp: 1000,
      });
    });
    act(() => {
      result.current.handleIncoming({
        content: 'second',
        mime: 'text/plain',
        sourceDevice: 'phone-1',
        timestamp: 2000,
      });
    });

    expect(result.current.history[0].content).toBe('second');
    expect(result.current.history[1].content).toBe('first');
  });

  it('handleIncoming ignores entries from the same device', () => {
    const opts = baseOptions();
    const { result } = renderHook(() => useClipboard(opts));

    act(() => {
      result.current.handleIncoming({
        content: 'self-loop',
        mime: 'text/plain',
        sourceDevice: 'desktop-1', // same as deviceId
        timestamp: 1000,
      });
    });

    expect(result.current.history).toHaveLength(0);
    // copyToClipboard should NOT be called for same-device entries
    expect(navigator.clipboard.writeText).not.toHaveBeenCalled();
  });

  it('handleIncoming calls onSynced callback', () => {
    const opts = baseOptions();
    const { result } = renderHook(() => useClipboard(opts));

    const entry = {
      content: 'synced',
      mime: 'text/plain',
      sourceDevice: 'phone-1',
      timestamp: 1000,
    };

    act(() => {
      result.current.handleIncoming(entry);
    });

    expect(opts.onSynced).toHaveBeenCalledWith(entry);
  });

  it('handleIncoming calls navigator.clipboard.writeText (copyToClipboard)', async () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    await act(async () => {
      result.current.handleIncoming({
        content: 'paste-me',
        mime: 'text/plain',
        sourceDevice: 'phone-1',
        timestamp: 1000,
      });
    });

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('paste-me');
  });

  // ── Max history limit ─────────────────────────────────────────────────────

  it('caps history at 50 entries', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    // Add 55 entries
    for (let i = 0; i < 55; i++) {
      act(() => {
        result.current.handleIncoming({
          content: `entry-${i}`,
          mime: 'text/plain',
          sourceDevice: 'phone-1',
          timestamp: i,
        });
      });
    }

    expect(result.current.history).toHaveLength(50);
    // Most recent entry (i=54) should be first
    expect(result.current.history[0].content).toBe('entry-54');
    // Oldest kept entry (i=5) should be last
    expect(result.current.history[49].content).toBe('entry-5');
  });

  // ── sendSync ──────────────────────────────────────────────────────────────

  it('sendSync sends correctly formatted message via sendMessage', () => {
    const opts = baseOptions();
    const { result } = renderHook(() => useClipboard(opts));

    act(() => {
      result.current.sendSync('clipboard content', 'text/plain');
    });

    expect(opts.sendMessage).toHaveBeenCalledTimes(1);
    const sent = opts.sendMessage.mock.calls[0][0];
    expect(sent).toMatchObject({
      type: 'clipboard',
      action: 'sync',
      content: 'clipboard content',
      mime: 'text/plain',
      source_device: 'desktop-1',
    });
    expect(typeof sent.timestamp).toBe('number');
  });

  it('sendSync handles undefined sendMessage gracefully', () => {
    const { result } = renderHook(() =>
      useClipboard({ sendMessage: undefined, deviceId: 'desktop-1' })
    );

    // Should not throw
    expect(() => {
      act(() => {
        result.current.sendSync('content', 'text/plain');
      });
    }).not.toThrow();
  });

  // ── startMonitoring / stopMonitoring ──────────────────────────────────────

  it('startMonitoring sets isMonitoring to true', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    act(() => {
      result.current.startMonitoring();
    });

    expect(result.current.isMonitoring).toBe(true);
  });

  it('stopMonitoring sets isMonitoring to false', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    act(() => {
      result.current.startMonitoring();
    });
    act(() => {
      result.current.stopMonitoring();
    });

    expect(result.current.isMonitoring).toBe(false);
  });

  it('startMonitoring is idempotent (double-call guard)', () => {
    const { result } = renderHook(() => useClipboard(baseOptions()));

    act(() => {
      result.current.startMonitoring();
    });
    act(() => {
      result.current.startMonitoring(); // second call should be no-op
    });

    expect(result.current.isMonitoring).toBe(true);
  });

  // ── Cleanup ───────────────────────────────────────────────────────────────

  it('cleans up polling interval on unmount', () => {
    const clearIntervalSpy = vi.spyOn(globalThis, 'clearInterval');

    const { result, unmount } = renderHook(() => useClipboard(baseOptions()));

    act(() => {
      result.current.startMonitoring();
    });

    unmount();

    expect(clearIntervalSpy).toHaveBeenCalled();
    clearIntervalSpy.mockRestore();
  });
});
