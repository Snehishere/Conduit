import React from 'react';
import { cn } from '../../lib/utils';
import type { AutomationAction } from '../../hooks/useAutomation';
import { ACTION_TYPES, getActionIcon } from './automationMeta';

interface ActionEditorProps {
  selected: AutomationAction['type'];
  onSelect: (id: AutomationAction['type']) => void;
  config: Partial<AutomationAction>;
  onConfigChange: (config: Partial<AutomationAction>) => void;
}

/**
 * Action type picker + per-action configuration fields.
 * Returns a fragment so the parent modal's `space-y-6` spacing is preserved.
 */
const ActionEditor: React.FC<ActionEditorProps> = ({
  selected,
  onSelect,
  config,
  onConfigChange,
}) => (
  <>
    {/* Action Selection */}
    <div role="group" aria-labelledby="action-picker-label">
      <div id="action-picker-label" className="text-[12px] text-ink-2 font-medium block mb-3">Then:</div>
      <div className="grid grid-cols-2 gap-2">
        {ACTION_TYPES.map(action => {
          const AIcon = getActionIcon(action.id);
          return (
            <button
              key={action.id}
              type="button"
              aria-pressed={selected === action.id}
              className={cn(
                'flex items-center gap-2.5 px-3.5 py-2.5 rounded-xl border text-left transition-all duration-200',
                selected === action.id
                  ? 'bg-accent-dim-ink border-[rgba(14,116,144,0.30)] text-accent-ink'
                  : 'bg-fill-1 border-line-1 text-ink-2 hover:border-line-2'
              )}
              onClick={() => { onSelect(action.id); }}
            >
              <AIcon size={14} strokeWidth={1.5} />
              <span className="text-[12px] font-medium">{action.label}</span>
            </button>
          );
        })}
      </div>
    </div>

    {/* Action Config */}
    {selected === 'send_notification' && (
      <div className="space-y-2" role="group" aria-labelledby="action-notification-label">
        <div id="action-notification-label" className="text-[12px] text-ink-2 font-medium block">Notification</div>
        <input
          type="text"
          aria-label="Notification title"
          value={config.title || ''}
          onChange={(e) => { onConfigChange({ ...config, title: e.target.value }); }}
          placeholder="Notification title"
          className="w-full px-4 py-2.5 rounded-xl bg-fill-1 border border-line-1 text-[14px] text-ink-1 placeholder:text-ink-3 focus:border-accent-ink transition-colors"
        />
        <input
          type="text"
          aria-label="Notification body"
          value={config.body || ''}
          onChange={(e) => { onConfigChange({ ...config, body: e.target.value }); }}
          placeholder="Notification body"
          className="w-full px-4 py-2.5 rounded-xl bg-fill-1 border border-line-1 text-[14px] text-ink-1 placeholder:text-ink-3 focus:border-accent-ink transition-colors"
        />
      </div>
    )}
    {selected === 'set_phone_profile' && (
      <div role="group" aria-labelledby="action-profile-label">
        <div id="action-profile-label" className="text-[12px] text-ink-2 font-medium block mb-2">Phone profile</div>
        <div className="flex gap-2">
          {(['silent', 'vibrate', 'ring'] as const).map(profile => (
            <button
              key={profile}
              type="button"
              aria-pressed={config.profile === profile}
              className={cn(
                'flex-1 py-2.5 rounded-xl text-[12px] font-medium border transition-colors capitalize',
                config.profile === profile
                  ? 'bg-accent-dim-ink border-[rgba(14,116,144,0.30)] text-accent-ink'
                  : 'bg-fill-1 border-line-1 text-ink-2 hover:border-line-2'
              )}
              onClick={() => { onConfigChange({ ...config, profile }); }}
            >
              {profile}
            </button>
          ))}
        </div>
      </div>
    )}
    {selected === 'run_shell_command' && (
      <div>
        <label htmlFor="action-command" className="text-[12px] text-ink-2 font-medium block mb-2">Command</label>
        <input
          id="action-command"
          type="text"
          value={config.command || ''}
          onChange={(e) => { onConfigChange({ ...config, command: e.target.value }); }}
          placeholder="Command to run"
          className="w-full px-4 py-2.5 rounded-xl bg-fill-1 border border-line-1 text-[14px] text-ink-1 font-mono placeholder:text-ink-3 focus:border-accent-ink transition-colors"
        />
      </div>
    )}
    {selected === 'open_url' && (
      <div>
        <label htmlFor="action-url" className="text-[12px] text-ink-2 font-medium block mb-2">URL</label>
        <input
          id="action-url"
          type="url"
          value={config.url || ''}
          onChange={(e) => { onConfigChange({ ...config, url: e.target.value }); }}
          placeholder="URL to open"
          className="w-full px-4 py-2.5 rounded-xl bg-fill-1 border border-line-1 text-[14px] text-ink-1 placeholder:text-ink-3 focus:border-accent-ink transition-colors"
        />
      </div>
    )}
  </>
);

export default ActionEditor;
