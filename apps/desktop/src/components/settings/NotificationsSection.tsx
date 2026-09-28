import React, { useId, useState } from 'react';
import { Plus, X } from 'lucide-react';
import Toggle from './Toggle';
import type { SectionProps } from './settingsTypes';

// Rows are contiguous: each Toggle supplies its own 16px padding and ink
// hairline separator (none on :last-child).
const NotificationsSection: React.FC<SectionProps> = ({ settings, updateSetting }) => {
  const inputId = useId();
  const [draft, setDraft] = useState('');
  const apps = settings.notification_apps;

  const addApp = () => {
    const name = draft.trim();
    setDraft('');
    if (!name) return;
    // The backend matches case-insensitively, so reject an obvious duplicate here
    // rather than storing two entries that behave as one.
    if (apps.some((a) => a.trim().toLowerCase() === name.toLowerCase())) return;
    updateSetting('notification_apps', [...apps, name]);
  };

  const removeApp = (name: string) => {
    updateSetting('notification_apps', apps.filter((a) => a !== name));
  };

  return (
    <div>
      <Toggle
        checked={settings.notifications_enabled}
        onChange={(v) => { updateSetting('notifications_enabled', v); }}
        label="Enable notifications"
        description="Receive notifications from connected devices"
      />

      <div className="py-4 border-b border-line-1">
        <label htmlFor={inputId} className="text-[13px] font-medium block text-ink-1">
          Mirrored apps
        </label>
        <p id={`${inputId}-desc`} className="text-[11px] mt-0.5 mb-3 text-ink-3">
          Only notifications from these apps are mirrored to your other devices. Names
          are matched case-insensitively against the app the device reports; an Android
          package name matches its last segment (com.whatsapp matches WhatsApp).
        </p>

        {apps.length === 0 ? (
          <p className="text-[12px] mb-3 text-danger-ink" role="status">
            No apps listed — notifications will not be mirrored.
          </p>
        ) : (
          <ul className="flex flex-wrap gap-1.5 mb-3" aria-label="Apps whose notifications are mirrored">
            {apps.map((app) => (
              <li key={app}>
                <span className="inline-flex items-center gap-1 pl-2.5 pr-1 py-1 rounded-[999px] text-[12px] text-ink-1 bg-fill-1 border border-line-1">
                  {app}
                  <button
                    type="button"
                    onClick={() => { removeApp(app); }}
                    aria-label={`Remove ${app}`}
                    className="btn-press inline-flex items-center justify-center w-5 h-5 rounded-full text-ink-3 hover:text-ink-1 hover:bg-fill-2 transition-colors"
                  >
                    <X size={11} aria-hidden="true" />
                  </button>
                </span>
              </li>
            ))}
          </ul>
        )}

        <div className="flex items-center gap-2">
          <input
            id={inputId}
            type="text"
            value={draft}
            placeholder="Add an app name"
            aria-describedby={`${inputId}-desc`}
            onChange={(e) => { setDraft(e.target.value); }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                addApp();
              }
            }}
            className="flex-1 min-w-0 px-3 py-2 rounded-[10px] text-[13px] text-ink-1 bg-fill-1 border border-line-1 transition-shadow focus:shadow-[0_0_0_3px_rgba(14,116,144,0.30)]"
          />
          <button
            type="button"
            onClick={addApp}
            disabled={draft.trim().length === 0}
            aria-label="Add app"
            className="btn-press shrink-0 inline-flex items-center gap-1.5 h-[38px] px-3 rounded-[10px] text-[12px] font-medium text-ink-2 bg-fill-1 border border-line-1 hover:bg-fill-2 hover:text-ink-1 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            <Plus size={13} aria-hidden="true" />
            Add
          </button>
        </div>
      </div>
    </div>
  );
};

export default NotificationsSection;
