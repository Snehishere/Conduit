import React, { useCallback, useEffect, useState } from 'react';
import { Trash2, Download, Upload, HardDrive, Plus, X, ShieldAlert } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { showSuccess, showError, showInfo } from '../../lib/toast';
import { SETTINGS_KEY_LIST, type SettingsData } from './settingsTypes';

interface AdvancedSectionProps {
  settings: SettingsData;
  setSettings: React.Dispatch<React.SetStateAction<SettingsData>>;
}

// Rows sit directly on the frost panel: 16px vertical rhythm, ink hairline
// separator (none on :last-child), subtle clear-fill hover.
const ROW_CLS =
  'w-full flex items-center justify-between py-4 text-left border-b border-line-1 last:border-0 transition-colors hover:bg-fill-1';

/**
 * Upper bound on an import file. A settings export is a few KB; anything past
 * this is a file that was not produced by Export Settings, and `readTextFile`
 * would pull the whole thing across the IPC boundary first.
 */
const MAX_IMPORT_BYTES = 1024 * 1024;

/**
 * Read a settings export chosen through the native file dialog.
 *
 * The dialog is a user gesture, but the path it returns is still just a string,
 * so the read is bounded and shape-checked before anything touches app state.
 * (`fs:allow-read-text-file` is deliberately read-only and text-only in
 * `capabilities/default.json`; see the comment there on why it cannot be scoped
 * to "the path the user just picked".)
 */
const readSettingsExport = async (selected: string): Promise<Record<string, unknown>> => {
  if (!selected.toLowerCase().endsWith('.json')) {
    throw new Error('Settings import expects a .json file');
  }
  const { stat, readTextFile } = await import('@tauri-apps/plugin-fs');
  const info = await stat(selected);
  if (info.size > MAX_IMPORT_BYTES) {
    throw new Error(`Settings file is too large (${info.size} bytes)`);
  }
  const parsed: unknown = JSON.parse(await readTextFile(selected));
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('Settings file is not a JSON object');
  }
  return parsed as Record<string, unknown>;
};

const AdvancedSection: React.FC<AdvancedSectionProps> = ({ settings, setSettings }) => {
  // ── Shell-command allowlist ──────────────────────────────────────────────
  //
  // Not part of `SettingsData`: `settingsTypes.ts` is the hand-maintained
  // mirror of the Rust `ConduitSettings` struct and is pinned to it by
  // `frontend_settings_type_key_set_matches_conduit_settings`, so adding a
  // field on the Rust side without the matching TS field turns that test red.
  // `allowed_commands` is therefore read and written through its own pair of
  // Tauri commands and keeps its state here, until both sides can be changed in
  // one pass.
  const [allowedCommands, setAllowedCommands] = useState<string[]>([]);
  const [commandDraft, setCommandDraft] = useState('');
  const [commandsLoaded, setCommandsLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<string[]>('get_allowed_commands')
      .then((list) => {
        if (!cancelled) setAllowedCommands(Array.isArray(list) ? list : []);
      })
      .catch(() => {
        // Fail closed in the UI too: show an empty list rather than an
        // optimistic "everything is allowed" one.
        if (!cancelled) setAllowedCommands([]);
      })
      .finally(() => {
        if (!cancelled) setCommandsLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const addCommand = useCallback(async () => {
    const name = commandDraft.trim();
    if (!name) return;
    const next = [...allowedCommands, name];
    try {
      const stored = await invoke<string[]>('set_allowed_commands', { commands: next });
      setAllowedCommands(Array.isArray(stored) ? stored : next);
      setCommandDraft('');
    } catch (err) {
      showError(
        `Could not allow "${name}": ${err instanceof Error ? err.message : String(err)}`
      );
    }
  }, [allowedCommands, commandDraft]);

  const removeCommand = useCallback(
    async (name: string) => {
      const next = allowedCommands.filter((c) => c !== name);
      try {
        const stored = await invoke<string[]>('set_allowed_commands', { commands: next });
        setAllowedCommands(Array.isArray(stored) ? stored : next);
      } catch (err) {
        showError(
          `Could not remove "${name}": ${err instanceof Error ? err.message : String(err)}`
        );
      }
    },
    [allowedCommands]
  );

  const handleClearCache = () => {
    try {
      // Clear localStorage cache items
      const keysToKeep = ['conduit-theme', 'conduit-accent', 'conduit_onboarded', 'conduit_search_history'];
      const keys = Object.keys(localStorage);
      keys.forEach((key) => {
        if (!keysToKeep.includes(key)) {
          localStorage.removeItem(key);
        }
      });
      showInfo('Cache cleared');
    } catch {
      showInfo('Cache cleared');
    }
  };

  const handleExport = () => {
    try {
      const settingsJson = JSON.stringify(settings, null, 2);
      const blob = new Blob([settingsJson], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'conduit-settings.json';
      a.click();
      URL.revokeObjectURL(url);
      showSuccess('Settings exported');
    } catch {
      showError('Could not export settings');
    }
  };

  const handleImport = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        multiple: false,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (selected) {
        const parsed = await readSettingsExport(selected);
        // Only keep keys the backend still knows about: an older export can carry
        // settings that have since been removed, and re-injecting them would put
        // dead fields back into the payload sent to `save_settings`.
        const known = new Set<string>(SETTINGS_KEY_LIST);
        const imported: Partial<SettingsData> = {};
        for (const [key, value] of Object.entries(parsed)) {
          if (known.has(key)) {
            (imported as Record<string, unknown>)[key] = value;
          }
        }
        setSettings((s) => ({ ...s, ...imported }));
        showSuccess('Settings imported — click Save to apply');
      }
    } catch (err) {
      // The message can carry our own validation text (e.g. "expects a .json
      // file"); fall back to a generic one for anything unrecognised.
      const message = err instanceof Error ? err.message : String(err);
      showError(message && message !== '[object Object]' ? `Could not import settings: ${message}` : 'Could not import settings');
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between py-4 border-b border-line-1">
        <div>
          <div className="text-[13px] font-medium text-ink-1">Storage Usage</div>
          <div className="text-[11px] mt-0.5 text-ink-3">Local database and cache</div>
        </div>
        <div className="flex items-center gap-1.5 text-[12px] text-ink-2">
          <HardDrive size={13} />
          <span>~2.1 MB</span>
        </div>
      </div>
      <button
        type="button"
        className={`btn-press ${ROW_CLS}`}
        onClick={handleClearCache}
      >
        <div>
          <div className="text-[13px] font-medium text-ink-1">Clear Cache</div>
          <div className="text-[11px] mt-0.5 text-ink-3">Remove cached app data stored on this device</div>
        </div>
        <Trash2 size={14} className="text-ink-3" />
      </button>
      <button
        type="button"
        className={`btn-press ${ROW_CLS}`}
        onClick={handleExport}
      >
        <div>
          <div className="text-[13px] font-medium text-ink-1">Export Settings</div>
          <div className="text-[11px] mt-0.5 text-ink-3">Save settings to file</div>
        </div>
        <Download size={14} className="text-ink-3" />
      </button>
      <button
        type="button"
        className={`btn-press ${ROW_CLS}`}
        onClick={handleImport}
      >
        <div>
          <div className="text-[13px] font-medium text-ink-1">Import Settings</div>
          <div className="text-[11px] mt-0.5 text-ink-3">Load settings from file</div>
        </div>
        <Upload size={14} className="text-ink-3" />
      </button>

      {/* Read-only, deliberately not a toggle: the relay client is compiled out of
          this build (main.rs keeps `spawn_relay_client` commented out so Conduit
          stays LAN-only), so there is nothing here to switch on. The endpoint is
          still persisted and exported so the value survives until the relay
          client is re-enabled. Re-enabling it is a one-line uncomment in
          main.rs:305. */}
      <div className="flex items-center justify-between py-4 border-b border-line-1">
        <div className="pr-4">
          <div className="text-[13px] font-medium text-ink-1">Relay endpoint</div>
          <div className="text-[11px] mt-0.5 text-ink-3">
            Saved, but the cloud relay client is disabled in this build.
          </div>
        </div>
        <code className="text-[11px] text-ink-2 truncate max-w-[180px]" title={settings.relay_url}>
          {settings.relay_url}
        </code>
      </div>

      {/* ── Shell-command allowlist ───────────────────────────────────────────
          Automation rules can run a shell command. Until this list was
          consulted, every production call site passed "no allowlist", so a rule
          pushed by a phone (or by any process that could open the loopback
          socket) executed unchecked — that was the remote-code-execution half of
          the vulnerability.

          The list is deny-by-default: while it is empty, no automation rule can
          run a shell command at all, and a rule asking to run one is refused
          rather than stored. Entries are matched on the executable name only,
          and any command containing a shell metacharacter (`;`, `|`, `&`, `$`,
          backtick, `>`, `<`, newline) is always rejected — so listing `ls` can
          never authorise `ls; rm -rf /`.

          Changes are applied immediately (the backend swaps the live allowlist
          on save); there is no separate "apply" step and no restart. */}
      <div className="py-4 border-b border-line-1">
        <div className="flex items-start justify-between gap-4">
          <div className="pr-4">
            <div className="text-[13px] font-medium text-ink-1 flex items-center gap-1.5">
              <ShieldAlert size={13} className="text-ink-3" />
              Allowed shell commands
            </div>
            <div className="text-[11px] mt-0.5 text-ink-3">
              {allowedCommands.length === 0
                ? 'None. Automation cannot run shell commands.'
                : `${allowedCommands.length} permitted for automation rules.`}
            </div>
          </div>
        </div>

        {allowedCommands.length > 0 && (
          <ul className="mt-3 flex flex-wrap gap-1.5" data-testid="allowed-commands-list">
            {allowedCommands.map((name) => (
              <li
                key={name}
                className="flex items-center gap-1 rounded-md bg-fill-1 px-2 py-1 text-[11px] font-mono text-ink-1"
              >
                {name}
                <button
                  type="button"
                  aria-label={`Remove ${name}`}
                  className="btn-press text-ink-3 hover:text-ink-1"
                  onClick={() => void removeCommand(name)}
                >
                  <X size={11} />
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="mt-3 flex items-center gap-1.5">
          <input
            type="text"
            value={commandDraft}
            spellCheck={false}
            autoComplete="off"
            placeholder="e.g. notify-send"
            aria-label="Command to allow"
            disabled={!commandsLoaded}
            onChange={(e) => setCommandDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                void addCommand();
              }
            }}
            className="flex-1 rounded-md bg-fill-1 px-2.5 py-1.5 text-[12px] font-mono text-ink-1 placeholder:text-ink-3 outline-none focus:border-accent"
          />
          <button
            type="button"
            className="btn-press flex items-center gap-1 rounded-md bg-fill-1 px-2.5 py-1.5 text-[12px] text-ink-1 disabled:opacity-40"
            disabled={!commandsLoaded || commandDraft.trim().length === 0}
            onClick={() => void addCommand()}
          >
            <Plus size={12} />
            Allow
          </button>
        </div>
        <div className="text-[11px] mt-1.5 text-ink-3">
          Executable name only — arguments are not matched. Listing{' '}
          <code className="font-mono">*</code> allows every command and is not
          recommended.
        </div>
      </div>
    </div>
  );
};

export default AdvancedSection;
