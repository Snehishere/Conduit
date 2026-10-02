import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useSms } from '../../hooks/useSms';
import type { WebSocketMessage } from '../../types/websocket';

vi.mock('../../lib/toast', () => ({
  showError: vi.fn(),
  showSuccess: vi.fn(),
}));

// A thread as the phone relays it: two unread messages.
const syncMessage = {
  type: 'sms',
  action: 'sync',
  threads: [
    {
      thread_id: '+15550001111',
      address: '+15550001111',
      name: null,
      snippet: 'test body',
      unread_count: 2,
      timestamp: 1700000000,
      messages: [
        {
          id: 'sms_1',
          address: '+15550001111',
          body: 'test body',
          timestamp: 1700000000,
          read: false,
          is_outgoing: false,
        },
      ],
    },
  ],
} as unknown as WebSocketMessage;

describe('useSms read state', () => {
  let sent: Record<string, unknown>[];

  beforeEach(() => {
    sent = [];
  });

  const setup = () => {
    const hook = renderHook(() => useSms((m) => sent.push(m)));
    act(() => hook.result.current.handleSmsMessage(syncMessage));
    return hook;
  };

  it('a synced thread reports its unread count', () => {
    const { result } = setup();
    expect(result.current.totalUnread).toBe(2);
  });

  it('selecting a thread clears its unread count', () => {
    // W4.22: `markRead` existed but nothing called it, so opening a thread
    // left the badge at 2 forever.
    const { result } = setup();

    act(() => result.current.setSelectedThread('+15550001111'));

    expect(result.current.threads[0].unread_count).toBe(0);
    expect(result.current.totalUnread).toBe(0);
  });

  it('selecting a thread marks its messages read locally', () => {
    const { result } = setup();

    act(() => result.current.setSelectedThread('+15550001111'));

    expect(result.current.threads[0].messages.every((m) => m.read)).toBe(true);
  });

  it('does not clear the unread count of a different thread', () => {
    const { result } = setup();

    act(() => result.current.setSelectedThread('+15550002222'));

    expect(result.current.totalUnread).toBe(2);
  });

  it('tells the phone which thread was read', () => {
    const { result } = setup();

    act(() => result.current.setSelectedThread('+15550001111'));

    expect(sent).toContainEqual({
      type: 'sms',
      action: 'mark_read',
      thread_id: '+15550001111',
    });
  });

  it('does not send anything when the selection is cleared or unchanged', () => {
    const { result } = setup();

    act(() => result.current.setSelectedThread('+15550001111'));
    expect(sent).toHaveLength(1);

    act(() => result.current.setSelectedThread('+15550001111'));
    act(() => result.current.setSelectedThread(null));
    expect(sent).toHaveLength(1);
  });

  it('works with no transport attached', () => {
    const { result } = renderHook(() => useSms());
    act(() => result.current.handleSmsMessage(syncMessage));

    act(() => result.current.setSelectedThread('+15550001111'));

    expect(result.current.totalUnread).toBe(0);
  });

  it('still exposes markRead for callers that want it', () => {
    const { result } = setup();

    act(() => result.current.markRead('+15550001111'));

    expect(result.current.totalUnread).toBe(0);
    expect(sent).toContainEqual({
      type: 'sms',
      action: 'mark_read',
      thread_id: '+15550001111',
    });
  });
});