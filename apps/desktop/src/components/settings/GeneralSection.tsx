import React from 'react';
import Toggle from './Toggle';
import type { SectionProps } from './settingsTypes';

// White-ish input well on the frost panel (spec §B5): clear ink fill,
// hairline border, ink text, teal focus ring.
const INPUT_CLS =
  'w-full px-3 py-2.5 rounded-[10px] text-[13px] text-ink-1 bg-fill-1 border border-line-1 transition-shadow focus:shadow-[0_0_0_3px_rgba(14,116,144,0.30)]';

const GeneralSection: React.FC<SectionProps> = ({ settings, updateSetting }) => (
  <div className="space-y-5">
    <div>
      <label htmlFor="settings-device-name" className="text-[13px] font-medium block mb-1.5 text-ink-1">Device name</label>
      <input
        id="settings-device-name"
        type="text"
        value={settings.device_name}
        onChange={(e) => { updateSetting('device_name', e.target.value); }}
        className={INPUT_CLS}
      />
    </div>
    <div>
      <label htmlFor="settings-max-devices" className="text-[13px] font-medium block mb-1.5 text-ink-1">Max connected devices</label>
      <input
        id="settings-max-devices"
        type="number" min="1" max="10"
        value={settings.max_devices}
        onChange={(e) => {
          const v = parseInt(e.target.value, 10);
          updateSetting('max_devices', Number.isNaN(v) ? 1 : Math.min(10, Math.max(1, v)));
        }}
        className={INPUT_CLS}
        aria-describedby="settings-max-devices-hint"
      />
      <p id="settings-max-devices-hint" className="text-[11px] mt-1.5 text-ink-3">
        No new pairing token is issued once this many devices are paired.
      </p>
    </div>
    {/* Preference rows: contiguous list, 16px vertical rhythm, ink hairline separators.
        Every toggle here is enforced in the Rust backend:
        max_devices → generate_pairing_token, sync_* → the WS dispatch gate,
        minimize_to_tray → the window CloseRequested handler. */}
    <div className="border-t border-line-1">
      <Toggle checked={settings.sync_notifications} onChange={(v) => { updateSetting('sync_notifications', v); }} label="Sync notifications" description="Mirror notifications from your devices" />
      <Toggle checked={settings.sync_clipboard} onChange={(v) => { updateSetting('sync_clipboard', v); }} label="Sync clipboard" description="Share clipboard content across devices" />
      <Toggle checked={settings.sync_files} onChange={(v) => { updateSetting('sync_files', v); }} label="Sync files" description="Allow file transfers between devices" />
      <Toggle checked={settings.minimize_to_tray} onChange={(v) => { updateSetting('minimize_to_tray', v); }} label="Minimize to tray" description="Keep Conduit running in system tray when closed" />
    </div>
  </div>
);

export default GeneralSection;
