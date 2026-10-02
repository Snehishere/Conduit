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
 * The editors (`ActionEditor.tsx`, `AutomationPanel.tsx`) import
 * `AutomationAction`. Until this alias existed the import resolved to nothing,
 * and `npx tsc --noEmit` stayed silent only because those files carried
 * `@ts-nocheck` (W4.32). They no longer do, so the alias is load-bearing: remove
 * it and `tsc` fails with TS2305 on both editors.
 */
export type { AutomationActionPayload as AutomationAction };

/**
 * Loosely-typed inbound automation payload.
 *
 * The live WebSocket path passes already-narrowed `AutomationMessage` values,
 * while direct callers (tests, manual wiring) may pass partial or malformed
 * payloads — every field is therefore validated defensively before use.
 *
 * `rule_id` and `timestamp` are the names this hook has always accepted and are
 * kept as fallbacks, but they are not what the protocol sends: the generated
 * `AutomationTriggeredMessage` carries `id` and `trigger_type` and has no
 * timestamp at all.
 */
interface AutomationMessageInput {
  action?: string;
  rules?: unknown;
  rule?: unknown;
  id?: string;
  rule_id?: string;
  timestamp?: number;
}

/**
 * Coerce one inbound rule into the shape the UI renders, or `null` if it cannot
 * be rendered.
 *
 * `AutomationSyncMessage.rules` is `Array<any>` and carries no counters: the
 * counters are local state this hook owns. Without a default, `RuleCard` prints
 * "Triggered  times" and the first increment produces `NaN`.
 *
 * A rule with no `trigger` or no `action` is dropped rather than half-filled:
 * `RuleCard` dereferences `rule.trigger.type`, and the hub's own parser rejects
 * such a rule anyway (`parse_rule_packet` requires both), so nothing persisted
 * can look like this.
 */
function normalizeRule(incoming: unknown): AutomationRule | null {
  if (!incoming || typeof incoming !== 'object') return null;
  const rule = incoming as Partial<AutomationRule>;
  if (typeof rule.id !== 'string' || rule.id === '') return null;
  if (!rule.trigger || !rule.action) return null;
  return {
    id: rule.id,
    name: rule.name ?? '',
    trigger: rule.trigger,
    action: rule.action,
    enabled: rule.enabled ?? true,
    triggerCount: rule.triggerCount ?? 0,
    lastTriggered: rule.lastTriggered,
  };
}

function normalizeRules(incoming: unknown): AutomationRule[] | null {
  if (!Array.isArray(incoming)) return null;
  return incoming.map(normalizeRule).filter((r): r is AutomationRule => r !== null);
}

/**
 * The frame this hook puts on the wire when it writes a rule.
 *
 * `AutomationRuleMessage` in `types/websocket.ts` describes this frame **flat**
 * — `id`, `name`, `trigger` and `rule_action` at the top level — but the hub
 * parses the **nested** form: `automation.rs::parse_rule_packet` unwraps `rule`
 * first and only falls back to the flat fields. So the generated type describes
 * a frame nobody sends (audit W1.5).
 *
 * This local type states what is actually sent so the three write paths below
 * are checked against it. `sendMessage` takes `Record<string, unknown>`, so
 * nothing enforces the shape today; that is why the drift went unnoticed.
 * `AutomationRule` is a type alias, not an interface, so it carries the implicit
 * index signature `Record<string, unknown>` requires.
 *
 * The generated type underneath is still wrong. Correcting it means editing
 * `packages/protocol/schema.json` and regenerating — W1.5, another agent's
 * file. Do not "fix" `websocket.ts` by hand; it is regenerated.
 */
type AutomationRuleFrame = {
  type: 'automation';
  action: 'rule';
  rule: AutomationRule;
};

function ruleFrame(rule: AutomationRule): AutomationRuleFrame {
  return { type: 'automation', action: 'rule', rule };
}

export function useAutomation() {
  const [rules, setRules] = useState<AutomationRule[]>([]);
  const [isEditing, setIsEditing] = useState(false);
  const [editingRule, setEditingRule] = useState<AutomationRule | null>(null);
  const { registerHandler, sendMessage } = useSharedWebSocket();

  // handleAutomationMessage is kept for direct subscription use (e.g. tests / manual wiring).
  // The live subscription below delegates to it, so there is one implementation
  // of the wire handling rather than two that can drift.
  const handleAutomationMessage = useCallback((data: AutomationMessageInput) => {
    switch (data.action) {
      case 'sync': {
        // A frame with a missing or non-array `rules` leaves the list untouched:
        // the panel maps over it, so storing `undefined` is a crash, not an
        // empty state.
        const incoming = normalizeRules(data.rules);
        if (incoming) setRules(incoming);
        break;
      }
      case 'triggered': {
        const ruleId = data.id ?? data.rule_id;
        // Seconds, which is what `RuleCard` renders. The protocol carries no
        // timestamp on this frame, so fall back to now rather than leaving
        // "last triggered" blank.
        const at = typeof data.timestamp === 'number' ? data.timestamp : Math.floor(Date.now() / 1000);
        if (ruleId) {
          setRules(prev => prev.map(r =>
            r.id === ruleId
              ? { ...r, lastTriggered: at, triggerCount: r.triggerCount + 1 }
              : r
          ));
        }
        break;
      }
      case 'created': {
        const incoming = normalizeRule(data.rule);
        if (incoming) {
          setRules(prev => (prev.some(r => r.id === incoming.id) ? prev : [...prev, incoming]));
        }
        break;
      }
      case 'updated': {
        const incoming = normalizeRule(data.rule);
        if (incoming) {
          setRules(prev => prev.map(r => (r.id === incoming.id ? incoming : r)));
        }
        break;
      }
      case 'deleted': {
        const ruleId = data.rule_id ?? data.id;
        if (ruleId) {
          setRules(prev => prev.filter(r => r.id !== ruleId));
        }
        break;
      }
      default:
        break;
    }
  }, []);

  const addRule = useCallback((rule: AutomationRule) => {
    // Deduplicate by id — server echoes use server UUIDs, optimistic adds use client ids.
    setRules(prev => (prev.some(r => r.id === rule.id) ? prev.map(r => (r.id === rule.id ? rule : r)) : [...prev, rule]));
  }, []);

  const deleteRule = useCallback((ruleId: string) => {
    setRules(prev => prev.filter(r => r.id !== ruleId));
  }, []);

  // Live subscription — declared after its stable useCallback dependencies so
  // the deps array can reference them without hitting the temporal dead zone.
  useEffect(() => {
    return registerHandler('automation', (data) => {
      if (data.type !== 'automation') return;
      handleAutomationMessage(data);
    });
  }, [registerHandler, handleAutomationMessage]);

  const createRule = useCallback((rule: Omit<AutomationRule, 'id' | 'triggerCount'>) => {
    const newRule: AutomationRule = {
      ...rule,
      id: `rule_${Date.now()}`,
      triggerCount: 0,
    };
    sendMessage(ruleFrame(newRule));
    addRule(newRule);
    return newRule;
  }, [addRule, sendMessage]);

  const updateRuleById = useCallback((ruleId: string, updates: Partial<AutomationRule>) => {
    setRules(prev => prev.map(r => {
      if (r.id === ruleId) {
        const updated = { ...r, ...updates };
        sendMessage(ruleFrame(updated));
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
        sendMessage(ruleFrame(updated));
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