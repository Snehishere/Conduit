// @ts-nocheck
import { useState, useEffect, useCallback } from 'react';
import { useSharedWebSocket } from './useWebSocket';
import type {
  AutomationActionPayload,
  AutomationTrigger,
} from '../types/websocket';

/**
 * The rule shape this hook holds in client state.
 *
 * Deliberately NOT an alias of the wire type: `AutomationRuleMessage` carries
 * a `type`/`action` discriminant and names the payload `rule_action`, whereas
 * the local shape drops the discriminants and adds two counters the UI
 * maintains. The trigger and action payloads are borrowed from the generated
 * protocol types so the two cannot drift on the fields that matter.
 */
export type AutomationRule = {
  id: string;
  name: string;
  trigger: AutomationTrigger;
  action: AutomationActionPayload;
  enabled: boolean;
  triggerCount: number;
  lastTriggered?: number;
};

export type { AutomationTrigger };

/**
 * Loosely-typed inbound automation payload.
 *
 * The live WebSocket path passes already-narrowed `AutomationMessage` values,
 * while direct callers (tests, manual wiring) may pass partial or malformed
 * payloads — every field is therefore validated defensively before use.
 */
interface AutomationMessageInput {
  action?: string;
  rules?: unknown;
  rule?: AutomationRule;
  rule_id?: string;
  timestamp?: number;
}

export function useAutomation() {
  const [rules, setRules] = useState<AutomationRule[]>([]);
  const [isEditing, setIsEditing] = useState(false);
  const [editingRule, setEditingRule] = useState<AutomationRule | null>(null);
  const { registerHandler, sendMessage } = useSharedWebSocket();

  // handleAutomationMessage is kept for direct subscription use (e.g. tests / manual wiring).
  // The live subscription below already routes automation messages.
  const handleAutomationMessage = useCallback((data: AutomationMessageInput) => {
    switch (data.action) {
      case 'sync':
        if (Array.isArray((data as any).rules)) setRules((data as any).rules);
        break;
      case 'triggered':
        if ((data as any).rule_id) {
          setRules(prev => prev.map(r =>
            r.id === (data as any).rule_id
              ? { ...r, lastTriggered: (data as any).timestamp, triggerCount: r.triggerCount + 1 }
              : r
          ));
        }
        break;
      case 'created' as any: {
        const incoming = (data as any).rule;
        if (incoming) {
          setRules(prev => (prev.some(r => r.id === incoming.id) ? prev : [...prev, incoming]));
        }
        break;
      }
      case 'updated' as any: {
        const incoming = (data as any).rule;
        if (incoming) {
          setRules(prev => prev.map(r => (r.id === incoming.id ? incoming : r)));
        }
        break;
      }
      case 'deleted' as any:
        if ((data as any).rule_id) {
          setRules(prev => prev.filter(r => r.id !== (data as any).rule_id));
        }
        break;
      default:
        break;
    }
  }, []);

  const addRule = useCallback((rule: AutomationRule) => {
    // Deduplicate by id — server echoes use server UUIDs, optimistic adds use client ids.
    setRules(prev => (prev.some(r => r.id === rule.id) ? prev.map(r => (r.id === rule.id ? rule : r)) : [...prev, rule]));
  }, []);

  const updateRule = useCallback((rule: AutomationRule) => {
    setRules(prev => prev.map(r => r.id === rule.id ? rule : r));
  }, []);

  const deleteRule = useCallback((ruleId: string) => {
    setRules(prev => prev.filter(r => r.id !== ruleId));
  }, []);

  const updateRuleTriggered = useCallback((ruleId: string, timestamp: number) => {
    setRules(prev => prev.map(r =>
      r.id === ruleId
        ? { ...r, lastTriggered: timestamp, triggerCount: r.triggerCount + 1 }
        : r
    ));
  }, []);

  // Live subscription — declared after its stable useCallback dependencies so
  // the deps array can reference them without hitting the temporal dead zone.
  useEffect(() => {
    return registerHandler('automation', (data) => {
      if (data.type !== 'automation') return;
      switch (data.action) {
        case 'sync':
          setRules((data as any).rules);
          break;
        case 'triggered':
          updateRuleTriggered((data as any).rule_id, (data as any).timestamp);
          break;
        case 'created' as any:
          addRule((data as any).rule);
          break;
        case 'updated' as any:
          updateRule((data as any).rule);
          break;
        case 'deleted' as any:
          deleteRule((data as any).rule_id);
          break;
      }
    });
  }, [registerHandler, addRule, updateRule, deleteRule, updateRuleTriggered]);

  const createRule = useCallback((rule: Omit<AutomationRule, 'id' | 'triggerCount'>) => {
    const newRule: AutomationRule = {
      ...rule,
      id: `rule_${Date.now()}`,
      triggerCount: 0,
    };
    sendMessage({ type: 'automation', action: 'rule', rule: newRule });
    addRule(newRule);
    return newRule;
  }, [addRule, sendMessage]);

  const updateRuleById = useCallback((ruleId: string, updates: Partial<AutomationRule>) => {
    setRules(prev => prev.map(r => {
      if (r.id === ruleId) {
        const updated = { ...r, ...updates };
        sendMessage({ type: 'automation', action: 'rule', rule: updated });
        return updated;
      }
      return r;
    }));
  }, [sendMessage]);

  const deleteRuleById = useCallback((ruleId: string) => {
    sendMessage({ type: 'automation', action: 'delete', rule_id: ruleId });
    deleteRule(ruleId);
  }, [deleteRule, sendMessage]);

  const toggleRule = useCallback((ruleId: string) => {
    setRules(prev => prev.map(r => {
      if (r.id === ruleId) {
        const updated = { ...r, enabled: !r.enabled };
        sendMessage({ type: 'automation', action: 'rule', rule: updated });
        return updated;
      }
      return r;
    }));
  }, [sendMessage]);

  const startEditing = useCallback((rule?: AutomationRule) => {
    setIsEditing(true);
    setEditingRule(rule || null);
  }, []);

  const stopEditing = useCallback(() => {
    setIsEditing(false);
    setEditingRule(null);
  }, []);

  return {
    rules,
    isEditing,
    editingRule,
    handleAutomationMessage,
    createRule,
    updateRuleById,
    deleteRuleById,
    toggleRule,
    startEditing,
    stopEditing,
  };
}