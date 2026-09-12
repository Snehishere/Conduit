import React, { useState, useEffect } from 'react';
import { IconSettings, IconCheck } from '../icons';

interface SettingsData {
  device_name: string;
  max_devices: number;
  auto_connect: boolean;
  sync_notifications: boolean;
  sync_clipboard: boolean;
  sync_files: boolean;
  notification_apps: string[];
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

const Settings: React.FC = () => {
  const [settings, setSettings] = useState<SettingsData>({
    device_name: 'Desktop',
    max_devices: 5,
    auto_connect: true,
    sync_notifications: true,
    sync_clipboard: true,
    sync_files: true,
    notification_apps: ['WhatsApp', 'Telegram', 'Slack', 'Discord'],
  });
  const [saved, setSaved] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadSettings();
  }, []);

  const loadSettings = async () => {
    try {
      const result = await invoke<SettingsData>('get_settings');
      setSettings(result);
    } catch (err) {
      console.error('Failed to load settings:', err);
    } finally {
      setLoading(false);
    }
  };

  const handleSave = async () => {
    try {
      await invoke('save_settings', { settings });
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (err) {
      console.error('Failed to save settings:', err);
    }
  };

  if (loading) {
    return (
      <div className="settings-panel">
        <div className="settings-header">
          <h2><IconSettings size={20} /> Settings</h2>
        </div>
        <div style={{ display: 'flex', justifyContent: 'center', padding: '40px' }}>
          <div className="spinner" />
        </div>
      </div>
    );
  }

  return (
    <div className="settings-panel">
      <div className="settings-header">
        <h2><IconSettings size={20} /> Settings</h2>
      </div>
      <div className="settings-body">
        <section className="settings-section">
          <h3>Device</h3>
          <div className="settings-field">
            <label>Device Name</label>
            <input
              type="text"
              value={settings.device_name}
              onChange={(e) => setSettings({ ...settings, device_name: e.target.value })}
            />
          </div>
          <div className="settings-field">
            <label>Max Connected Devices</label>
            <input
              type="number"
              min="1"
              max="10"
              value={settings.max_devices}
              onChange={(e) => {
                const parsed = parseInt(e.target.value, 10);
                setSettings({ ...settings, max_devices: Number.isNaN(parsed) ? 1 : Math.min(10, Math.max(1, parsed)) });
              }}
            />
          </div>
        </section>

        <section className="settings-section">
          <h3>Sync</h3>
          <div className="settings-toggle">
            <label>Auto-connect on startup</label>
            <input
              type="checkbox"
              checked={settings.auto_connect}
              onChange={(e) => setSettings({ ...settings, auto_connect: e.target.checked })}
            />
          </div>
          <div className="settings-toggle">
            <label>Sync Notifications</label>
            <input
              type="checkbox"
              checked={settings.sync_notifications}
              onChange={(e) => setSettings({ ...settings, sync_notifications: e.target.checked })}
            />
          </div>
          <div className="settings-toggle">
            <label>Sync Clipboard</label>
            <input
              type="checkbox"
              checked={settings.sync_clipboard}
              onChange={(e) => setSettings({ ...settings, sync_clipboard: e.target.checked })}
            />
          </div>
          <div className="settings-toggle">
            <label>Sync Files</label>
            <input
              type="checkbox"
              checked={settings.sync_files}
              onChange={(e) => setSettings({ ...settings, sync_files: e.target.checked })}
            />
          </div>
        </section>

        <section className="settings-section">
          <h3>Notification Apps</h3>
          <div className="settings-list">
            {settings.notification_apps.map((app, index) => (
              <div key={index} className="settings-list-item">
                <span>{app}</span>
                <button
                  className="remove-btn"
                  onClick={() => {
                    const newApps = settings.notification_apps.filter((_, i) => i !== index);
                    setSettings({ ...settings, notification_apps: newApps });
                  }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </section>

        <button className="save-btn" onClick={handleSave}>
          {saved ? <><IconCheck size={14} /> Saved</> : 'Save Settings'}
        </button>
      </div>
    </div>
  );
};

export default Settings;
