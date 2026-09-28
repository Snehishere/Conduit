import React, { useState, useEffect, useCallback } from 'react';
import {
  Settings as SettingsIcon, Check,
  FolderOpen, Bell, Palette, Info, Wrench, Keyboard,
} from 'lucide-react';
import { useTheme } from '../../lib/theme';
import type { Theme } from '../../lib/theme';
import { cn } from '../../lib/utils';
import { invoke, getCurrentVersion } from '../../lib/tauri';
import { showError, showSuccess } from '../../lib/toast';
import GeneralSection from './GeneralSection';
import AppearanceSection from './AppearanceSection';
import NotificationsSection from './NotificationsSection';
import FilesSection from './FilesSection';
import AdvancedSection from './AdvancedSection';
import AboutSection from './AboutSection';
import { DEFAULT_SETTINGS } from './settingsTypes';
import type { SettingsCategory, SettingsData, UpdateSetting } from './settingsTypes';

const CATEGORIES: { id: SettingsCategory; label: string; icon: React.ElementType }[] = [
  { id: 'general', label: 'General', icon: SettingsIcon },
  { id: 'appearance', label: 'Appearance', icon: Palette },
  { id: 'notifications', label: 'Notifications', icon: Bell },
  { id: 'files', label: 'File Transfers', icon: FolderOpen },
  { id: 'advanced', label: 'Advanced', icon: Wrench },
  { id: 'about', label: 'About', icon: Info },
];

// Frosted scrollbar thumb — the global scrollbar token is tuned for dark
// surfaces and is invisible on white frost (globals.css can't be edited here).
const SCROLLBAR_THUMB = '[&::-webkit-scrollbar-thumb]:bg-line-2!';

const Settings: React.FC = () => {
  // The pre-load state is the full `SettingsData` (mirroring the Rust defaults) so
  // `save_settings` is never handed a partial object.
  const [settings, setSettings] = useState<SettingsData>(DEFAULT_SETTINGS);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeCategory, setActiveCategory] = useState<SettingsCategory>('general');
  const [appVersion, setAppVersion] = useState('');
  const { setTheme } = useTheme();

  // Memoized so the mount effect below can safely list it as a dependency
  // without re-running on every render (which would loop network fetches).
  const loadSettings = useCallback(async () => {
    try {
      const result = await invoke<SettingsData>('get_settings');
      setSettings(result);
      if (result.theme) setTheme(result.theme as Theme);
      const version = await getCurrentVersion();
      setAppVersion(version);
    } catch (err) {
      console.error('Failed to load settings:', err);
      showError('Could not load settings — showing defaults');
    } finally {
      setLoading(false);
    }
  }, [setTheme]);

  useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  const updateSetting: UpdateSetting = (key, value) => {
    setSettings((s) => ({ ...s, [key]: value }));
  };

  const handleSave = async () => {
    setSaveError(null);
    try {
      await invoke('save_settings', { settings });
      setSaved(true);
      showSuccess('Settings saved');
      setTimeout(() => { setSaved(false); }, 2000);
    } catch (err) {
      // A failed save used to be swallowed into the console, so the button looked
      // broken with no explanation. Surface it.
      const message = err instanceof Error ? err.message : String(err);
      console.error('Failed to save settings:', err);
      setSaveError(message);
      showError(`Could not save settings: ${message}`);
    }
  };

  if (loading) {
    return (
      <div className="h-full w-full p-4">
        <div
          className="surf-frost h-full w-full rounded-[24px] flex flex-col items-center justify-center gap-4"
          role="status"
          aria-live="polite"
        >
          <div
            className="w-8 h-8 border-2 border-line-2 border-t-accent-ink rounded-full animate-spin"
            aria-hidden="true"
          />
          <span className="text-[13px] text-ink-3">Loading settings...</span>
        </div>
      </div>
    );
  }

  const renderCategory = () => {
    switch (activeCategory) {
      case 'general':
        return <GeneralSection settings={settings} updateSetting={updateSetting} />;
      case 'appearance':
        return <AppearanceSection settings={settings} updateSetting={updateSetting} />;
      case 'notifications':
        return <NotificationsSection settings={settings} updateSetting={updateSetting} />;
      case 'files':
        return <FilesSection settings={settings} updateSetting={updateSetting} />;
      case 'advanced':
        return <AdvancedSection settings={settings} setSettings={setSettings} />;
      case 'about':
        return <AboutSection appVersion={appVersion} />;
    }
  };

  return (
    <div className="h-full w-full p-4">
      {/* One frosted panel for the whole screen, inset from the starfield */}
      <div className="surf-frost h-full w-full rounded-[24px] overflow-hidden flex">
        {/* Left sidebar */}
        <div
          className={`w-[200px] shrink-0 py-6 px-3 overflow-y-auto ${SCROLLBAR_THUMB}`}
          style={{
            background: 'var(--fill-1)',
            borderRight: '1px solid var(--line-1)',
          }}
        >
          <div className="flex items-center gap-2 px-3 mb-5">
            <SettingsIcon size={16} className="text-accent-ink" />
            <span className="text-[14px] font-semibold text-ink-1">Settings</span>
          </div>
          <nav aria-label="Settings categories" className="space-y-0.5">
            {CATEGORIES.map((cat) => {
              const Icon = cat.icon;
              const active = activeCategory === cat.id;
              return (
                <button
                  key={cat.id}
                  type="button"
                  onClick={() => { setActiveCategory(cat.id); }}
                  aria-current={active ? 'page' : undefined}
                  className={cn(
                    'btn-press w-full flex items-center gap-2.5 px-3 py-[9px] rounded-[10px] text-[12px] [font-weight:550] transition-colors text-left',
                    active
                      ? 'bg-accent-dim-ink text-accent-ink shadow-[inset_2px_0_0_var(--accent-ink)]'
                      : 'text-ink-2 hover:bg-fill-1'
                  )}
                >
                  <Icon size={14} strokeWidth={1.5} />
                  {cat.label}
                </button>
              );
            })}
          </nav>
        </div>

        {/* Right content */}
        <div className={`flex-1 min-w-0 overflow-y-auto ${SCROLLBAR_THUMB}`}>
          <div className="max-w-[520px] px-8 py-6">
            <h2 className="text-[20px] font-bold mb-2 text-ink-1">
              {CATEGORIES.find((c) => c.id === activeCategory)?.label}
            </h2>
            <p className="text-[13px] mb-6 text-ink-3">
              Manage preferences, notifications, and desktop behaviors.
            </p>

            {/* Rows sit directly on the frost panel — ink hairline separators */}
            <div>{renderCategory()}</div>

            {/* Replays the Onboarding dialog, not the (unreachable) tutorial: the
                handler clears `conduit_onboarded`, which is the flag useAppShell
                reads to decide `showOnboarding`. The old label said "tutorial",
                which sent the user looking for InteractiveTutorial. */}
            <div className="mt-4">
              <button
                type="button"
                onClick={() => {
                  localStorage.removeItem('conduit_onboarded');
                  window.location.reload();
                }}
                className="btn-press inline-flex items-center gap-2 h-10 px-3 rounded-[10px] text-[12px] font-medium text-ink-3 hover:text-ink-1 hover:bg-fill-1 transition-colors"
              >
                <Keyboard size={14} />
                Show onboarding again
              </button>
            </div>

            {/* Save */}
            <div
              className="mt-6 pt-5"
              style={{ borderTop: '1px solid var(--line-1)' }}
            >
              {/* Live region so screen readers hear save confirmation */}
              <div className="sr-only" role="status" aria-live="polite">
                {saved ? 'Settings saved successfully.' : ''}
              </div>
              {/* Visible failure feedback: the save used to fail silently. */}
              {saveError && (
                <p role="alert" className="mb-3 text-[12px] text-danger-ink">
                  {saveError}
                </p>
              )}
              <button
                type="button"
                onClick={handleSave}
                className={cn(
                  'btn-press w-full h-11 rounded-[12px] text-[13px] font-semibold flex items-center justify-center gap-2 transition-colors',
                  saved
                    ? 'bg-success-ink text-on-accent'
                    : 'btn-solid'
                )}
              >
                {saved ? <><Check size={14} /> Saved</> : 'Save settings'}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Settings;
