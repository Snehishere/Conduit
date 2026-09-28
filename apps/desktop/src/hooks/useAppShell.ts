import { useState, useEffect, useCallback } from 'react';
import { invoke, getCurrentVersion } from '../lib/tauri';

interface AppShellState {
  showSplash: boolean;
  showOnboarding: boolean;
  showWhatsNew: boolean;
  showTutorial: boolean;
  showShortcuts: boolean;
  currentVersion: string;
}

export function useAppShell() {
  const [state, setState] = useState<AppShellState>({
    showSplash: true,
    showOnboarding: !localStorage.getItem('conduit_onboarded'),
    showWhatsNew: false,
    showTutorial: false,
    showShortcuts: false,
    currentVersion: '',
  });

  const dismissSplash = useCallback(() => {
    setState((s) => ({ ...s, showSplash: false }));
  }, []);

  const completeOnboarding = useCallback(() => {
    localStorage.setItem('conduit_onboarded', 'true');
    setState((s) => ({ ...s, showOnboarding: false }));
  }, []);

  const dismissWhatsNew = useCallback(() => {
    setState((s) => ({ ...s, showWhatsNew: false }));
  }, []);

  const dismissTutorial = useCallback(() => {
    setState((s) => ({ ...s, showTutorial: false }));
  }, []);

  const toggleShortcuts = useCallback(() => {
    setState((s) => ({ ...s, showShortcuts: !s.showShortcuts }));
  }, []);

  const dismissShortcuts = useCallback(() => {
    setState((s) => ({ ...s, showShortcuts: false }));
  }, []);

  const setShowTutorial = useCallback((show: boolean) => {
    setState((s) => ({ ...s, showTutorial: show }));
  }, []);

  // Show window after splash
  useEffect(() => {
    const showWindow = async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        await win.show();
      } catch {
        /* showing the window is best-effort (fails outside Tauri, e.g. tests) */
      }
    };
    const timer = setTimeout(() => {
      dismissSplash();
      void showWindow();
    }, 1500);
    return () => { clearTimeout(timer); };
  }, [dismissSplash]);

  // Check for first run after update (What's New)
  useEffect(() => {
    const checkVersion = async () => {
      try {
        const settings = await invoke<{ last_version?: string }>('get_settings');
        const version = await getCurrentVersion();
        setState((s) => ({ ...s, currentVersion: version }));
        if (settings.last_version && settings.last_version !== version) {
          setState((s) => ({ ...s, showWhatsNew: true }));
        }
        await invoke('save_settings_field', { key: 'last_version', value: version });
      } catch {
        /* version gating is non-critical — skip What's New on failure */
      }
    };
    void checkVersion();
  }, []);

  return {
    ...state,
    dismissSplash,
    completeOnboarding,
    dismissWhatsNew,
    dismissTutorial,
    toggleShortcuts,
    dismissShortcuts,
    setShowTutorial,
  };
}
