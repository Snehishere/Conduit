// @ts-nocheck
import { useState, useEffect, useRef } from 'react';
import {
  useAutomation,
  type AutomationRule,
  type AutomationTrigger,
  type AutomationAction,
} from '../../hooks/useAutomation';
import { Zap, Plus, X } from 'lucide-react';
import { cn } from '../../lib/utils';
import EmptyState from '../ui/EmptyState';
import TriggerEditor from './TriggerEditor';
import ActionEditor from './ActionEditor';
import RuleCard from './RuleCard';

interface AutomationPanelProps {
  onRuleCreated?: (rule: AutomationRule) => void;
  onRuleDeleted?: (ruleId: string) => void;
}

export default function AutomationPanel({ onRuleCreated, onRuleDeleted }: AutomationPanelProps) {
  const {
    rules,
    isEditing,
    editingRule,
    createRule,
    updateRuleById,
    deleteRuleById,
    toggleRule,
    startEditing,
    stopEditing,
  } = useAutomation();

  const [ruleName, setRuleName] = useState('');
  const [selectedTrigger, setSelectedTrigger] = useState<AutomationTrigger['type']>('device_connect');
  const [selectedAction, setSelectedAction] = useState<AutomationAction['type']>('send_notification');
  const [triggerConfig, setTriggerConfig] = useState<Partial<AutomationTrigger>>({});
  const [actionConfig, setActionConfig] = useState<Partial<AutomationAction>>({});

  const dialogRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  // Focus management: move focus into the dialog on open, restore on close.
  useEffect(() => {
    if (!isEditing) return;
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    closeButtonRef.current?.focus();
    return () => {
      previousFocusRef.current?.focus();
    };
  }, [isEditing]);

  // Escape closes the dialog; Tab cycles focus within it (document level so it
  // works even if focus lands outside the dialog element).
  useEffect(() => {
    if (!isEditing) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        stopEditing();
        return;
      }
      if (e.key === 'Tab' && dialogRef.current) {
        const focusableElements = dialogRef.current.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
        );
        if (focusableElements.length === 0) return;
        const firstElement = focusableElements[0];
        const lastElement = focusableElements[focusableElements.length - 1];
        const active = document.activeElement;
        const focusInside = dialogRef.current.contains(active);

        if (e.shiftKey) {
          if (!focusInside || active === firstElement) {
            e.preventDefault();
            lastElement.focus();
          }
        } else {
          if (!focusInside || active === lastElement) {
            e.preventDefault();
            firstElement.focus();
          }
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => { document.removeEventListener('keydown', handleKeyDown); };
  }, [isEditing, stopEditing]);

  const resetEditor = () => {
    setRuleName('');
    setSelectedTrigger('device_connect');
    setSelectedAction('send_notification');
    setTriggerConfig({});
    setActionConfig({});
  };

  const handleSave = () => {
    const rule = {
      name: ruleName,
      trigger: { ...triggerConfig, type: selectedTrigger },
      action: { ...actionConfig, type: selectedAction },
      enabled: true,
    };

    if (editingRule) {
      updateRuleById(editingRule.id, rule);
    } else {
      const newRule = createRule(rule);
      onRuleCreated?.(newRule);
    }

    stopEditing();
    resetEditor();
  };

  const handleEdit = (rule: AutomationRule) => {
    setRuleName(rule.name);
    setSelectedTrigger(rule.trigger.type);
    setSelectedAction(rule.action.type);
    setTriggerConfig(rule.trigger);
    setActionConfig(rule.action);
    startEditing(rule);
  };

  const handleTriggerSelect = (id: AutomationTrigger['type']) => {
    setSelectedTrigger(id);
    setTriggerConfig({});
  };

  const handleActionSelect = (id: AutomationAction['type']) => {
    setSelectedAction(id);
    setActionConfig({});
  };

  return (
    <div className="flex h-full flex-col p-4">
      {/* Content panel — kept OUTSIDE any backdrop-filtered ancestor so the
          fixed modal below stays viewport-positioned. */}
      <div className="surf-frost flex min-h-0 flex-1 flex-col overflow-hidden rounded-[24px] p-7">
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-[20px] font-semibold text-ink-1 flex items-center gap-2">
            <Zap size={20} strokeWidth={1.8} />
            Automation
          </h1>
          <button
            onClick={() => { startEditing(); }}
            className="flex h-9 items-center gap-2 rounded-full btn-solid px-4 text-[13px] font-semibold transition-colors"
          >
            <Plus size={14} strokeWidth={2} />
            New rule
          </button>
        </div>

        {rules.length === 0 ? (
          <EmptyState
            icon={Zap}
            title="No automation rules"
            description="Create rules to automate tasks between your devices"
          />
        ) : (
          <div className="flex-1 overflow-y-auto space-y-3">
            {rules.map(rule => (
              <RuleCard
                key={rule.id}
                rule={rule}
                onToggle={toggleRule}
                onEdit={handleEdit}
                onDelete={(ruleId) => {
                  deleteRuleById(ruleId);
                  onRuleDeleted?.(ruleId);
                }}
              />
            ))}
          </div>
        )}
      </div>

      {/* Rule Editor Modal */}
      {isEditing && (
        <div className="scrim fixed inset-0 z-[200] flex items-center justify-center" onClick={stopEditing}>
          <div
            ref={dialogRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="rule-editor-title"
            onClick={(e) => { e.stopPropagation(); }}
            className="surf-frost flex max-h-[85vh] w-[520px] flex-col overflow-hidden rounded-[24px]"
            style={{
              boxShadow:
                '0 32px 80px -24px rgba(0, 0, 0, 0.9), 0 0 0 0.5px rgba(0, 0, 0, 0.16)',
            }}
          >
            {/* Modal Header */}
            <div className="flex items-center justify-between border-b border-ink-line px-6 py-4">
              <h2 id="rule-editor-title" className="text-[16px] font-semibold text-ink-1">
                {editingRule ? 'Edit rule' : 'Create rule'}
              </h2>
              <button
                ref={closeButtonRef}
                onClick={stopEditing}
                className="flex w-8 h-8 items-center justify-center rounded-lg bg-fill-1 text-ink-3 transition-colors hover:bg-fill-2 hover:text-ink-1"
                aria-label="Close rule editor"
              >
                <X size={16} strokeWidth={1.5} />
              </button>
            </div>

            {/* Modal Content */}
            <div className="flex-1 overflow-y-auto px-6 py-5 space-y-6">
              {/* Rule Name */}
              <div>
                <label htmlFor="rule-name" className="text-[12px] text-ink-2 font-medium block mb-2">Rule name</label>
                <input
                  id="rule-name"
                  type="text"
                  value={ruleName}
                  onChange={(e) => { setRuleName(e.target.value); }}
                  placeholder="Enter rule name"
                  className="w-full rounded-xl border border-line-1 bg-fill-1 px-4 py-2.5 text-[14px] text-ink-1 transition-colors placeholder:text-ink-3 focus:border-accent-ink"
                />
              </div>

              <TriggerEditor
                selected={selectedTrigger}
                onSelect={handleTriggerSelect}
                config={triggerConfig}
                onConfigChange={setTriggerConfig}
              />

              <ActionEditor
                selected={selectedAction}
                onSelect={handleActionSelect}
                config={actionConfig}
                onConfigChange={setActionConfig}
              />
            </div>

            {/* Modal Footer */}
            <div className="flex items-center justify-end gap-3 border-t border-ink-line px-6 py-4">
              <button
                onClick={stopEditing}
                className="rounded-full px-4 py-2 text-[13px] font-medium text-ink-3 transition-colors hover:text-ink-1"
              >
                Cancel
              </button>
              <button
                onClick={handleSave}
                disabled={!ruleName}
                className={cn(
                  'rounded-full px-5 py-2 text-[13px] font-semibold transition-colors',
                  ruleName
                    ? 'btn-solid'
                    : 'surf-clear text-ink-3 opacity-50 cursor-not-allowed'
                )}
              >
                Save rule
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
