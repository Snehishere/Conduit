import React from 'react';
import { cn } from '../../lib/utils';
import type { AutomationTrigger } from '../../hooks/useAutomation';
import { TRIGGER_TYPES, getTriggerIcon } from './automationMeta';

interface TriggerEditorProps {
  selected: AutomationTrigger['type'];
  onSelect: (id: AutomationTrigger['type']) => void;
  config: Partial<AutomationTrigger>;
  onConfigChange: (config: Partial<AutomationTrigger>) => void;
}

/**
 * Trigger type picker + per-trigger configuration fields.
 * Returns a fragment so the parent modal's `space-y-6` spacing is preserved.
 */
const TriggerEditor: React.FC<TriggerEditorProps> = ({
  selected,
  onSelect,
  config,
  onConfigChange,
}) => (
  <>
    {/* Trigger Selection */}
    <div role="group" aria-labelledby="trigger-picker-label">
      <div id="trigger-picker-label" className="text-[12px] text-ink-2 font-medium block mb-3">When:</div>
      <div className="grid grid-cols-2 gap-2">
        {TRIGGER_TYPES.map(trigger => {
          const TIcon = getTriggerIcon(trigger.id);
          return (
            <button
              key={trigger.id}
              type="button"
              aria-pressed={selected === trigger.id}
              className={cn(
                'flex items-center gap-2.5 px-3.5 py-2.5 rounded-xl border text-left transition-all duration-200',
                selected === trigger.id
                  ? 'bg-accent-dim-ink border-[rgba(14,116,144,0.30)] text-accent-ink'
                  : 'bg-fill-1 border-line-1 text-ink-2 hover:border-line-2'
              )}
              onClick={() => { onSelect(trigger.id); }}
            >
              <TIcon size={14} strokeWidth={1.5} />
              <span className="text-[12px] font-medium">{trigger.label}</span>
            </button>
          );
        })}
      </div>
    </div>

    {/* Trigger Config */}
    {selected === 'time' && (
      <div>
        <label htmlFor="trigger-time" className="text-[12px] text-ink-2 font-medium block mb-2">Time</label>
        <input
          id="trigger-time"
          type="time"
          value={config.time || ''}
          onChange={(e) => { onConfigChange({ ...config, time: e.target.value }); }}
          className="w-full px-4 py-2.5 rounded-xl bg-fill-1 border border-line-1 text-[14px] text-ink-1 placeholder:text-ink-3 focus:border-accent-ink transition-colors"
        />
      </div>
    )}
    {selected === 'battery_level' && (
      <div>
        <label htmlFor="trigger-battery-threshold" className="text-[12px] text-ink-2 font-medium block mb-2">
          Battery below {config.below || 20}%
        </label>
        <input
          id="trigger-battery-threshold"
          type="range"
          min="0"
          max="100"
          value={config.below || 20}
          onChange={(e) => { onConfigChange({
            ...config,
            below: parseInt(e.target.value),
          }); }}
          className="w-full h-1.5 bg-fill-2 rounded-full appearance-none cursor-pointer accent-accent-ink"
        />
      </div>
    )}
    {selected === 'wifi_change' && (
      <div>
        <label htmlFor="trigger-wifi-ssid" className="text-[12px] text-ink-2 font-medium block mb-2">Network name</label>
        <input
          id="trigger-wifi-ssid"
          type="text"
          value={config.ssid || ''}
          onChange={(e) => { onConfigChange({ ...config, ssid: e.target.value }); }}
          placeholder="Enter WiFi network name"
          className="w-full px-4 py-2.5 rounded-xl bg-fill-1 border border-line-1 text-[14px] text-ink-1 placeholder:text-ink-3 focus:border-accent-ink transition-colors"
        />
      </div>
    )}
  </>
);

export default TriggerEditor;
