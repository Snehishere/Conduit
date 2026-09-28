import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useAutomation } from '../../hooks/useAutomation';
import { makeAutomationRule } from '../mocks';

// ─── Mocks ────────────────────────────────────────────────────────────────────

const mockSendMessage = vi.fn();
const mockRegisterHandler = vi.fn(() => vi.fn()); // returns unregister fn

vi.mock('../../hooks/useWebSocket', () => ({
  useSharedWebSocket: () => ({
    connected: true,
    reconnecting: false,
    notifications: [],
    dismissNotification: vi.fn(),
    replyNotification: vi.fn(),
    syncClipboard: vi.fn(),
    registerHandler: mockRegisterHandler,
    sendMessage: mockSendMessage,
  }),
}));

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('useAutomation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ── Initial state ─────────────────────────────────────────────────────────

  it('starts with an empty rules list', () => {
    const { result } = renderHook(() => useAutomation());
    expect(result.current.rules).toEqual([]);
  });

  it('starts with isEditing false', () => {
    const { result } = renderHook(() => useAutomation());
    expect(result.current.isEditing).toBe(false);
  });

  it('starts with editingRule null', () => {
    const { result } = renderHook(() => useAutomation());
    expect(result.current.editingRule).toBeNull();
  });

  it('registers an automation message handler on mount', () => {
    renderHook(() => useAutomation());
    expect(mockRegisterHandler).toHaveBeenCalledWith('automation', expect.any(Function));
  });

  // ── handleAutomationMessage ───────────────────────────────────────────────

  describe('handleAutomationMessage', () => {
    it('"sync" action replaces the rules list', () => {
      const { result } = renderHook(() => useAutomation());
      const rules = [
        makeAutomationRule({ id: 'r1', name: 'Rule 1' }),
        makeAutomationRule({ id: 'r2', name: 'Rule 2' }),
      ];

      act(() => {
        result.current.handleAutomationMessage({ action: 'sync', rules });
      });

      expect(result.current.rules).toHaveLength(2);
      expect(result.current.rules[0].name).toBe('Rule 1');
    });

    it('"sync" ignores non-array rules', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({ action: 'sync', rules: 'not-an-array' });
      });

      expect(result.current.rules).toEqual([]);
    });

    it('"created" action adds a new rule', () => {
      const { result } = renderHook(() => useAutomation());
      const rule = makeAutomationRule({ id: 'new-1', name: 'New Rule' });

      act(() => {
        result.current.handleAutomationMessage({ action: 'created', rule });
      });

      expect(result.current.rules).toHaveLength(1);
      expect(result.current.rules[0].id).toBe('new-1');
    });

    it('"created" action deduplicates by id (ignores duplicate)', () => {
      const { result } = renderHook(() => useAutomation());
      const rule = makeAutomationRule({ id: 'dup-1', name: 'First' });

      act(() => {
        result.current.handleAutomationMessage({ action: 'created', rule });
      });
      act(() => {
        result.current.handleAutomationMessage({ action: 'created', rule: { ...rule, name: 'Second' } });
      });

      // handleAutomationMessage dedup: if id exists, keep original (no update)
      expect(result.current.rules).toHaveLength(1);
      expect(result.current.rules[0].name).toBe('First');
    });

    it('"created" ignores message without rule', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({ action: 'created' });
      });

      expect(result.current.rules).toEqual([]);
    });

    it('"updated" action replaces matching rule', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({
          action: 'created',
          rule: makeAutomationRule({ id: 'u1', name: 'Original' }),
        });
      });

      act(() => {
        result.current.handleAutomationMessage({
          action: 'updated',
          rule: makeAutomationRule({ id: 'u1', name: 'Updated' }),
        });
      });

      expect(result.current.rules[0].name).toBe('Updated');
    });

    it('"updated" ignores message without rule', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({ action: 'updated' });
      });

      // No crash, no state change
      expect(result.current.rules).toEqual([]);
    });

    it('"deleted" action removes rule by id', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({
          action: 'created',
          rule: makeAutomationRule({ id: 'del-1', name: 'To Delete' }),
        });
      });
      expect(result.current.rules).toHaveLength(1);

      act(() => {
        result.current.handleAutomationMessage({ action: 'deleted', rule_id: 'del-1' });
      });

      expect(result.current.rules).toHaveLength(0);
    });

    it('"deleted" ignores message without rule_id', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({
          action: 'created',
          rule: makeAutomationRule({ id: 'keep-1' }),
        });
      });

      act(() => {
        result.current.handleAutomationMessage({ action: 'deleted' });
      });

      expect(result.current.rules).toHaveLength(1);
    });

    it('"triggered" action increments triggerCount and sets lastTriggered', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({
          action: 'created',
          rule: makeAutomationRule({ id: 't1', triggerCount: 5 }),
        });
      });

      act(() => {
        result.current.handleAutomationMessage({
          action: 'triggered',
          rule_id: 't1',
          timestamp: 9999,
        });
      });

      expect(result.current.rules[0].triggerCount).toBe(6);
      expect(result.current.rules[0].lastTriggered).toBe(9999);
    });

    it('"triggered" ignores message without rule_id', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({
          action: 'created',
          rule: makeAutomationRule({ id: 't2', triggerCount: 0 }),
        });
      });

      act(() => {
        result.current.handleAutomationMessage({ action: 'triggered', timestamp: 100 });
      });

      expect(result.current.rules[0].triggerCount).toBe(0);
    });

    it('unknown action is a no-op', () => {
      const { result } = renderHook(() => useAutomation());

      act(() => {
        result.current.handleAutomationMessage({ action: 'unknown_action' });
      });

      expect(result.current.rules).toEqual([]);
    });
  });

  // ── createRule ────────────────────────────────────────────────────────────

  it('createRule adds a rule with generated id and triggerCount=0', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.createRule({
        name: 'Test Rule',
        trigger: { type: 'device_connect' },
        action: { type: 'send_notification', title: 'Hi' },
        enabled: true,
      });
    });

    // The rule should be in the rules list with auto-generated id and triggerCount=0
    expect(result.current.rules).toHaveLength(1);
    expect(result.current.rules[0].id).toMatch(/^rule_/);
    expect(result.current.rules[0].triggerCount).toBe(0);
    expect(result.current.rules[0].name).toBe('Test Rule');
  });

  it('createRule sends message via WebSocket', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.createRule({
        name: 'WS Rule',
        trigger: { type: 'time', time: '08:00' },
        action: { type: 'open_url', url: 'https://example.com' },
        enabled: false,
      });
    });

    expect(mockSendMessage).toHaveBeenCalledTimes(1);
    const sent = mockSendMessage.mock.calls[0][0];
    expect(sent).toMatchObject({
      type: 'automation',
      action: 'rule',
      rule: expect.objectContaining({ name: 'WS Rule' }),
    });
  });

  // ── toggleRule ────────────────────────────────────────────────────────────

  it('toggleRule flips enabled from true to false', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'tog-1', enabled: true }),
      });
    });

    act(() => {
      result.current.toggleRule('tog-1');
    });

    expect(result.current.rules[0].enabled).toBe(false);
  });

  it('toggleRule flips enabled from false to true', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'tog-2', enabled: false }),
      });
    });

    act(() => {
      result.current.toggleRule('tog-2');
    });

    expect(result.current.rules[0].enabled).toBe(true);
  });

  it('toggleRule sends updated rule via WebSocket', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'tog-3', enabled: true }),
      });
    });

    mockSendMessage.mockClear();

    act(() => {
      result.current.toggleRule('tog-3');
    });

    expect(mockSendMessage).toHaveBeenCalledTimes(1);
    expect(mockSendMessage.mock.calls[0][0].rule.enabled).toBe(false);
  });

  it('toggleRule does nothing for non-existent id', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'exists', enabled: true }),
      });
    });

    mockSendMessage.mockClear();

    act(() => {
      result.current.toggleRule('does-not-exist');
    });

    // No crash, original rule unchanged
    expect(result.current.rules[0].enabled).toBe(true);
    expect(mockSendMessage).not.toHaveBeenCalled();
  });

  // ── updateRuleById ────────────────────────────────────────────────────────

  it('updateRuleById applies partial updates', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'upd-1', name: 'Original' }),
      });
    });

    act(() => {
      result.current.updateRuleById('upd-1', { name: 'Changed' });
    });

    expect(result.current.rules[0].name).toBe('Changed');
  });

  it('updateRuleById sends message via WebSocket', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'upd-2' }),
      });
    });

    mockSendMessage.mockClear();

    act(() => {
      result.current.updateRuleById('upd-2', { enabled: false });
    });

    expect(mockSendMessage).toHaveBeenCalledTimes(1);
    expect(mockSendMessage.mock.calls[0][0].rule.enabled).toBe(false);
  });

  // ── deleteRuleById ────────────────────────────────────────────────────────

  it('deleteRuleById removes rule from list', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'del-2' }),
      });
    });
    expect(result.current.rules).toHaveLength(1);

    act(() => {
      result.current.deleteRuleById('del-2');
    });

    expect(result.current.rules).toHaveLength(0);
  });

  it('deleteRuleById sends delete message via WebSocket', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.handleAutomationMessage({
        action: 'created',
        rule: makeAutomationRule({ id: 'del-3' }),
      });
    });

    mockSendMessage.mockClear();

    act(() => {
      result.current.deleteRuleById('del-3');
    });

    expect(mockSendMessage).toHaveBeenCalledTimes(1);
    expect(mockSendMessage.mock.calls[0][0]).toMatchObject({
      type: 'automation',
      action: 'delete',
      rule_id: 'del-3',
    });
  });

  // ── startEditing / stopEditing ────────────────────────────────────────────

  it('startEditing without rule sets isEditing true and editingRule null', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.startEditing();
    });

    expect(result.current.isEditing).toBe(true);
    expect(result.current.editingRule).toBeNull();
  });

  it('startEditing with rule sets editingRule', () => {
    const { result } = renderHook(() => useAutomation());
    const rule = makeAutomationRule({ id: 'edit-1' });

    act(() => {
      result.current.startEditing(rule as any);
    });

    expect(result.current.isEditing).toBe(true);
    expect(result.current.editingRule).toEqual(rule);
  });

  it('stopEditing resets isEditing and editingRule', () => {
    const { result } = renderHook(() => useAutomation());

    act(() => {
      result.current.startEditing(makeAutomationRule() as any);
    });
    act(() => {
      result.current.stopEditing();
    });

    expect(result.current.isEditing).toBe(false);
    expect(result.current.editingRule).toBeNull();
  });
});
