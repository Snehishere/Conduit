import { useState, useEffect, useCallback } from 'react';
import { useSharedWebSocket } from './useWebSocket';

interface AutomationTrigger {
  type: 'device_connect' | 'device_disconnect' | 'time' | 'battery_level' | 'wifi_change' | 'app_open' | 'audio_device_connect';
  device_id?: string;
  time?: string;
  battery_threshold?: number;
  wifi_ssid?: string;
  app_package?: string;
  audio_device_id?: string;
}

interface AutomationAction {
  type: 'send_notification' | 'set_phone_profile' | 'route_audio' | 'run_shell_command' | 'toggle_wifi' | 'toggle_bluetooth' | 'open_url' | 'open_app' | 'set_window_state';
  title?: string;
  body?: string;
  profile?: 'silent' | 'vibrate' | 'ring';
  device_id?: string;
  command?: string;
  enabled?: boolean;
  url?: string;
  app_package?: string;
  state?: string;
}

interface AutomationRule {
  id: string;
  name: string;
  trigger: AutomationTrigger;
  action: AutomationAction;
  enabled: boolean;
  lastTriggered?: number;
  triggerCount: number;
}

export function useAutomation() {
  const [rules, setRules] = useState<AutomationRule[]>([]);
  const [isEditing, setIsEditing] = useState(false);
  const [editingRule, setEditingRule] = useState<AutomationRule | null>(null);
  const { registerHandler, sendMessage } = useSharedWebSocket();

  useEffect(() => {
    return registerHandler('automation', (data: any) => {
      switch (data.action) {
        case 'sync':
          setRules(data.rules);
          break;
        case 'triggered':
          updateRuleTriggered(data.rule_id, data.timestamp);
          break;
        case 'created':
          addRule(data.rule);
          break;
        case 'updated':
          updateRule(data.rule);
          break;
        case 'deleted':
          deleteRule(data.rule_id);
          break;
      }
    });
  }, [registerHandler]);

  // handleAutomationMessage is kept for direct subscription use (e.g. tests / manual wiring).
  // The live subscription above already routes automation messages.
  const handleAutomationMessage = useCallback((data: any) => {
    switch (data?.action) {
      case 'sync':
        if (Array.isArray(data.rules)) setRules(data.rules);
        break;
      case 'triggered':
        if (data.rule_id) {
          setRules(prev => prev.map(r =>
            r.id === data.rule_id
              ? { ...r, lastTriggered: data.timestamp, triggerCount: r.triggerCount + 1 }
              : r
          ));
        }
        break;
      case 'created':
        if (data.rule) {
          setRules(prev => (prev.some(r => r.id === data.rule.id) ? prev : [...prev, data.rule]));
        }
        break;
      case 'updated':
        if (data.rule) {
          setRules(prev => prev.map(r => (r.id === data.rule.id ? data.rule : r)));
        }
        break;
      case 'deleted':
        if (data.rule_id) {
          setRules(prev => prev.filter(r => r.id !== data.rule_id));
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