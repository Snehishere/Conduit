import React from 'react';
import { Pencil, Trash2, ArrowRight } from 'lucide-react';
import { cn } from '../../lib/utils';
import type { AutomationRule } from '../../hooks/useAutomation';
import {
  getTriggerLabel,
  getActionLabel,
  getTriggerIcon,
  getActionIcon,
} from './automationMeta';

interface RuleCardProps {
  rule: AutomationRule;
  onToggle: (ruleId: string) => void;
  onEdit: (rule: AutomationRule) => void;
  onDelete: (ruleId: string) => void;
}

const RuleCard: React.FC<RuleCardProps> = ({ rule, onToggle, onEdit, onDelete }) => {
  const TriggerIcon = getTriggerIcon(rule.trigger.type);
  const ActionIcon = getActionIcon(rule.action.type);

  return (
    <div
      className={cn(
        'surf-clear border rounded-2xl p-5 transition-all duration-200',
        rule.enabled
          ? 'border-line-1 hover:border-line-2'
          : 'border-line-1 opacity-60'
      )}
    >
      <div className="flex items-start justify-between mb-3">
        <div className="text-[15px] font-medium text-ink-1">{rule.name}</div>
        <div className="flex items-center gap-2">
          {/* Toggle */}
          <button
            onClick={() => { onToggle(rule.id); }}
            role="switch"
            aria-checked={rule.enabled}
            aria-label={`Toggle ${rule.name}`}
            className={cn(
              'relative w-10 h-5 rounded-full transition-colors duration-200',
              rule.enabled ? 'bg-accent-ink' : 'bg-[#CBD2DC]'
            )}
          >
            <div className={cn(
              'absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform duration-200',
              rule.enabled ? 'translate-x-5' : 'translate-x-0.5'
            )} />
          </button>
          <button
            onClick={() => { onEdit(rule); }}
            className="w-7 h-7 rounded-lg bg-fill-1 flex items-center justify-center text-ink-3 hover:text-ink-1 transition-colors"
            aria-label={`Edit ${rule.name}`}
          >
            <Pencil size={12} strokeWidth={1.5} />
          </button>
          <button
            onClick={() => { onDelete(rule.id); }}
            className="w-7 h-7 rounded-lg bg-fill-1 flex items-center justify-center text-ink-3 hover:text-danger-ink hover:bg-[rgba(185,28,28,0.10)] transition-colors"
            aria-label={`Delete ${rule.name}`}
          >
            <Trash2 size={12} strokeWidth={1.5} />
          </button>
        </div>
      </div>

      {/* Trigger → Action flow */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2 rounded-full border border-line-1 bg-fill-1 px-3 py-1.5">
          <TriggerIcon size={12} strokeWidth={1.5} className="text-accent-ink" />
          <span className="text-[11px] text-ink-2 font-medium">{getTriggerLabel(rule.trigger.type)}</span>
        </div>
        <ArrowRight size={14} className="text-ink-3 shrink-0" strokeWidth={1.5} />
        <div className="flex items-center gap-2 rounded-full border border-line-1 bg-fill-1 px-3 py-1.5">
          <ActionIcon size={12} strokeWidth={1.5} className="text-accent-ink" />
          <span className="text-[11px] text-ink-2 font-medium">{getActionLabel(rule.action.type)}</span>
        </div>
      </div>

      {/* Stats */}
      <div className="flex items-center gap-3 mt-3 text-[11px] text-ink-3">
        <span>Triggered {rule.triggerCount} times</span>
        {rule.lastTriggered && (
          <>
            <span className="text-ink-3">·</span>
            <span>Last: {new Date(rule.lastTriggered * 1000).toLocaleString()}</span>
          </>
        )}
      </div>
    </div>
  );
};

export default RuleCard;
