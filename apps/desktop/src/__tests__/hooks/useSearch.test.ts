import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useSearch } from '../../hooks/useSearch';
import { makeDevice, makeNotification } from '../mocks';

// ─── Mocks ────────────────────────────────────────────────────────────────────

const mockSetSelectedDevice = vi.fn();
let mockDevices: ReturnType<typeof makeDevice>[] = [];
let mockNotifications: ReturnType<typeof makeNotification>[] = [];

vi.mock('../../hooks/useDevices', () => ({
  useDevices: () => ({
    devices: mockDevices,
    selectedDevice: null,
    setSelectedDevice: mockSetSelectedDevice,
    refreshDevices: vi.fn(),
    removeDevice: vi.fn(),
    error: null,
  }),
}));

vi.mock('../../hooks/useWebSocket', () => ({
  useWebSocket: () => ({
    connected: true,
    reconnecting: false,
    notifications: mockNotifications,
    dismissNotification: vi.fn(),
    replyNotification: vi.fn(),
    syncClipboard: vi.fn(),
    sendMessage: vi.fn(),
    registerHandler: vi.fn(() => vi.fn()),
  }),
}));

const mockSetActiveView = vi.fn();

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('useSearch', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockDevices = [];
    mockNotifications = [];
  });

  // ── Initial state ─────────────────────────────────────────────────────────

  it('starts with an empty search query', () => {
    const { result } = renderHook(() => useSearch(mockSetActiveView));
    expect(result.current.searchQuery).toBe('');
  });

  it('starts with empty search results', () => {
    const { result } = renderHook(() => useSearch(mockSetActiveView));
    expect(result.current.searchResults).toEqual([]);
  });

  it('starts with searchFocused false', () => {
    const { result } = renderHook(() => useSearch(mockSetActiveView));
    expect(result.current.searchFocused).toBe(false);
  });

  // ── Empty query returns empty results ─────────────────────────────────────

  it('returns empty results when query is empty even if devices exist', () => {
    mockDevices = [makeDevice({ id: 'd1', name: 'Pixel' })];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    expect(result.current.searchQuery).toBe('');
    expect(result.current.searchResults).toEqual([]);
  });

  // ── Device search ─────────────────────────────────────────────────────────

  it('finds devices by name (case-insensitive)', () => {
    mockDevices = [
      makeDevice({ id: 'd1', name: 'Pixel 7', device_type: 'phone', os: 'Android 14' }),
      makeDevice({ id: 'd2', name: 'MacBook Pro', device_type: 'laptop', os: 'macOS 14' }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('pixel');
    });

    expect(result.current.searchResults).toHaveLength(1);
    expect(result.current.searchResults[0].title).toBe('Pixel 7');
    expect(result.current.searchResults[0].category).toBe('device');
  });

  it('finds devices by device_type', () => {
    mockDevices = [
      makeDevice({ id: 'd1', name: 'Galaxy S24', device_type: 'phone', os: 'Android 14' }),
      makeDevice({ id: 'd2', name: 'iPad Air', device_type: 'tablet', os: 'iPadOS 17' }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('tablet');
    });

    expect(result.current.searchResults).toHaveLength(1);
    expect(result.current.searchResults[0].title).toBe('iPad Air');
  });

  it('returns all matching devices for a broad query', () => {
    mockDevices = [
      makeDevice({ id: 'd1', name: 'Phone Alpha', device_type: 'phone', os: 'Android' }),
      makeDevice({ id: 'd2', name: 'Phone Beta', device_type: 'phone', os: 'iOS' }),
      makeDevice({ id: 'd3', name: 'Desktop', device_type: 'desktop', os: 'Windows' }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('phone');
    });

    expect(result.current.searchResults).toHaveLength(2);
  });

  // ── Notification search ───────────────────────────────────────────────────

  it('finds notifications by title', () => {
    mockNotifications = [
      makeNotification({ id: 'n1', title: 'New message from Alice', body: 'Hey there' }),
      makeNotification({ id: 'n2', title: 'Calendar reminder', body: 'Meeting at 3pm' }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('alice');
    });

    expect(result.current.searchResults).toHaveLength(1);
    expect(result.current.searchResults[0].title).toBe('New message from Alice');
    expect(result.current.searchResults[0].category).toBe('notification');
  });

  it('finds notifications by body', () => {
    mockNotifications = [
      makeNotification({ id: 'n1', title: 'Reminder', body: 'Buy groceries' }),
      makeNotification({ id: 'n2', title: 'Alert', body: 'Server is down' }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('groceries');
    });

    expect(result.current.searchResults).toHaveLength(1);
    expect(result.current.searchResults[0].id).toBe('notif-n1');
  });

  it('excludes dismissed notifications from results', () => {
    mockNotifications = [
      makeNotification({ id: 'n1', title: 'Active notif', body: 'visible', dismissed: false }),
      makeNotification({ id: 'n2', title: 'Dismissed notif', body: 'visible', dismissed: true }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('visible');
    });

    expect(result.current.searchResults).toHaveLength(1);
    expect(result.current.searchResults[0].title).toBe('Active notif');
  });

  // ── Case insensitivity ────────────────────────────────────────────────────

  it('search is case-insensitive for devices', () => {
    mockDevices = [makeDevice({ id: 'd1', name: 'Pixel Phone', device_type: 'phone', os: 'Android' })];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('PIXEL');
    });

    expect(result.current.searchResults).toHaveLength(1);
  });

  it('search is case-insensitive for notifications', () => {
    mockNotifications = [
      makeNotification({ id: 'n1', title: 'Urgent Alert', body: 'Critical issue', dismissed: false }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('URGENT');
    });

    expect(result.current.searchResults).toHaveLength(1);
  });

  // ── Combined results ──────────────────────────────────────────────────────

  it('returns both device and notification results', () => {
    mockDevices = [makeDevice({ id: 'd1', name: 'Alpha Phone', device_type: 'phone', os: 'Android' })];
    mockNotifications = [
      makeNotification({ id: 'n1', title: 'Alpha Update', body: 'New version available', dismissed: false }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('alpha');
    });

    expect(result.current.searchResults).toHaveLength(2);
    const categories = result.current.searchResults.map((r) => r.category);
    expect(categories).toContain('device');
    expect(categories).toContain('notification');
  });

  // ── No matches ────────────────────────────────────────────────────────────

  it('returns empty results for non-matching query', () => {
    mockDevices = [makeDevice({ id: 'd1', name: 'Pixel', device_type: 'phone', os: 'Android' })];
    mockNotifications = [
      makeNotification({ id: 'n1', title: 'Test', body: 'Body', dismissed: false }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('zzzznonexistent');
    });

    expect(result.current.searchResults).toEqual([]);
  });

  // ── searchRef ─────────────────────────────────────────────────────────────

  it('provides a searchRef for the input element', () => {
    const { result } = renderHook(() => useSearch(mockSetActiveView));
    expect(result.current.searchRef).toBeDefined();
    expect(result.current.searchRef.current).toBeNull(); // no DOM element yet
  });

  // ── onClick handlers ──────────────────────────────────────────────────────

  it('device result onClick calls setActiveView and setSelectedDevice', () => {
    mockDevices = [makeDevice({ id: 'd-abc', name: 'Test Device', device_type: 'phone', os: 'Android' })];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('test device');
    });

    expect(result.current.searchResults).toHaveLength(1);

    act(() => {
      result.current.searchResults[0].onClick();
    });

    expect(mockSetActiveView).toHaveBeenCalledWith('web');
    expect(mockSetSelectedDevice).toHaveBeenCalledWith('d-abc');
  });

  it('notification result onClick calls setActiveView notifications', () => {
    mockNotifications = [
      makeNotification({ id: 'n-test', title: 'Alert', body: 'Something happened', dismissed: false }),
    ];

    const { result } = renderHook(() => useSearch(mockSetActiveView));

    act(() => {
      result.current.setSearchQuery('alert');
    });

    expect(result.current.searchResults).toHaveLength(1);

    act(() => {
      result.current.searchResults[0].onClick();
    });

    expect(mockSetActiveView).toHaveBeenCalledWith('notifications');
  });
});
