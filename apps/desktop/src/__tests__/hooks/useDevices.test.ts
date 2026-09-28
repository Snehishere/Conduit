import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useDevices } from '../../hooks/useDevices';
import { makeDevice } from '../mocks';

// ─── Mocks ────────────────────────────────────────────────────────────────────

const mockInvoke = vi.fn();

vi.mock('../../lib/tauri', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockShowError = vi.fn();
const mockShowSuccess = vi.fn();

vi.mock('../../lib/toast', () => ({
  showError: (...args: unknown[]) => mockShowError(...args),
  showSuccess: (...args: unknown[]) => mockShowSuccess(...args),
}));

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('useDevices', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    // Default: invoke returns empty list
    mockInvoke.mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // ── Initial state ─────────────────────────────────────────────────────────

  it('starts with an empty devices list', async () => {
    const { result } = renderHook(() => useDevices());
    // Allow the initial useEffect refresh to settle
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.devices).toEqual([]);
  });

  it('starts with selectedDevice null', async () => {
    const { result } = renderHook(() => useDevices());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.selectedDevice).toBeNull();
  });

  it('starts with error null', async () => {
    const { result } = renderHook(() => useDevices());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.error).toBeNull();
  });

  // ── refreshDevices ────────────────────────────────────────────────────────

  it('refreshDevices populates device list', async () => {
    const devices = [
      makeDevice({ id: 'd1', name: 'Pixel 7' }),
      makeDevice({ id: 'd2', name: 'iPhone 15' }),
    ];
    mockInvoke.mockResolvedValue(devices);

    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await result.current.refreshDevices();
    });

    expect(result.current.devices).toHaveLength(2);
    expect(result.current.devices[0].name).toBe('Pixel 7');
    expect(result.current.devices[1].name).toBe('iPhone 15');
  });

  it('refreshDevices calls invoke with get_devices', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await result.current.refreshDevices();
    });

    expect(mockInvoke).toHaveBeenCalledWith('get_devices');
  });

  it('refreshDevices clears previous error on success', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'));

    const { result } = renderHook(() => useDevices());

    // First call fails
    await act(async () => {
      await result.current.refreshDevices();
    });
    expect(result.current.error).toBe('fail');

    // Second call succeeds
    mockInvoke.mockResolvedValueOnce([]);
    await act(async () => {
      await result.current.refreshDevices();
    });
    expect(result.current.error).toBeNull();
  });

  // ── refreshDevices error handling ─────────────────────────────────────────

  it('refreshDevices sets error state on failure', async () => {
    mockInvoke.mockRejectedValue(new Error('Connection refused'));

    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await result.current.refreshDevices();
    });

    expect(result.current.error).toBe('Connection refused');
  });

  it('refreshDevices shows toast on error', async () => {
    mockInvoke.mockRejectedValue(new Error('Timeout'));

    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await result.current.refreshDevices();
    });

    expect(mockShowError).toHaveBeenCalledWith('Failed to refresh devices: Timeout');
  });

  it('refreshDevices handles non-Error thrown values', async () => {
    mockInvoke.mockRejectedValue('string error');

    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await result.current.refreshDevices();
    });

    expect(result.current.error).toBe('string error');
  });

  // ── removeDevice ──────────────────────────────────────────────────────────

  it('removeDevice calls invoke with delete_paired_device', async () => {
    const { result } = renderHook(() => useDevices());

    // Let mount refresh settle (consumes default [] from beforeEach)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Set up devices for the manual refresh
    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' }), makeDevice({ id: 'd2' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    // Set up delete success
    mockInvoke.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.removeDevice('d1');
    });

    expect(mockInvoke).toHaveBeenCalledWith('delete_paired_device', { id: 'd1' });
  });

  it('removeDevice removes device from list', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' }), makeDevice({ id: 'd2' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    expect(result.current.devices).toHaveLength(2);

    mockInvoke.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.removeDevice('d1');
    });

    expect(result.current.devices).toHaveLength(1);
    expect(result.current.devices[0].id).toBe('d2');
  });

  it('removeDevice clears selectedDevice if removed device was selected', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' }), makeDevice({ id: 'd2' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    act(() => {
      result.current.setSelectedDevice('d1');
    });
    expect(result.current.selectedDevice).toBe('d1');

    mockInvoke.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.removeDevice('d1');
    });

    expect(result.current.selectedDevice).toBeNull();
  });

  it('removeDevice does not clear selectedDevice if different device is removed', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' }), makeDevice({ id: 'd2' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    act(() => {
      result.current.setSelectedDevice('d1');
    });

    mockInvoke.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.removeDevice('d2');
    });

    expect(result.current.selectedDevice).toBe('d1');
  });

  it('removeDevice shows success toast', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    mockInvoke.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.removeDevice('d1');
    });

    expect(mockShowSuccess).toHaveBeenCalledWith('Device removed successfully');
  });

  // ── removeDevice error handling ───────────────────────────────────────────

  it('removeDevice sets error on failure', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    mockInvoke.mockRejectedValueOnce(new Error('Delete failed'));
    await act(async () => {
      await result.current.removeDevice('d1');
    });

    expect(result.current.error).toBe('Delete failed');
    // Device should still be in the list (not removed)
    expect(result.current.devices).toHaveLength(1);
  });

  it('removeDevice shows toast on error', async () => {
    const { result } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    mockInvoke.mockResolvedValueOnce([makeDevice({ id: 'd1' })]);
    await act(async () => {
      await result.current.refreshDevices();
    });

    mockInvoke.mockRejectedValueOnce(new Error('Perm denied'));
    await act(async () => {
      await result.current.removeDevice('d1');
    });

    expect(mockShowError).toHaveBeenCalledWith('Failed to remove device: Perm denied');
  });

  // ── Auto-refresh on mount ─────────────────────────────────────────────────

  it('calls refreshDevices automatically on mount', async () => {
    renderHook(() => useDevices());

    // The useEffect runs on mount
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockInvoke).toHaveBeenCalledWith('get_devices');
  });

  it('sets up auto-refresh interval every 5 seconds', async () => {
    renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const callCountBefore = mockInvoke.mock.calls.length;

    // Advance by 5 seconds
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });

    // Should have called invoke again
    expect(mockInvoke.mock.calls.length).toBeGreaterThan(callCountBefore);
  });

  it('clears interval on unmount', async () => {
    const clearIntervalSpy = vi.spyOn(globalThis, 'clearInterval');

    const { unmount } = renderHook(() => useDevices());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    unmount();

    expect(clearIntervalSpy).toHaveBeenCalled();
    clearIntervalSpy.mockRestore();
  });

  // ── setSelectedDevice ─────────────────────────────────────────────────────

  it('setSelectedDevice updates the selected device', async () => {
    const { result } = renderHook(() => useDevices());

    act(() => {
      result.current.setSelectedDevice('some-device');
    });

    expect(result.current.selectedDevice).toBe('some-device');
  });

  it('setSelectedDevice can reset to null', async () => {
    const { result } = renderHook(() => useDevices());

    act(() => {
      result.current.setSelectedDevice('some-device');
    });
    act(() => {
      result.current.setSelectedDevice(null);
    });

    expect(result.current.selectedDevice).toBeNull();
  });
});
