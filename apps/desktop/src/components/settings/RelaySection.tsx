import React, { useCallback, useEffect, useState } from 'react';
import { Copy, Check, Globe } from 'lucide-react';
import Toggle from './Toggle';
import type { SectionProps } from './settingsTypes';
import { invoke } from '../../lib/tauri';
import { showError } from '../../lib/toast';

/** Mirrors `RelayStatusView` in `commands/settings.rs`. */
interface RelayStatus {
  running: boolean;
  error: string | null;
  port: number | null;
  local_port: number | null;
  tls_pin: string | null;
  active_connections: number;
  registered_devices: number;
}

/**
 * How often the status is refreshed.
 *
 * The counters move on their own as devices connect and disconnect, so this is
 * a poll rather than a subscription. Four seconds is fast enough that the number
 * does not look stuck and slow enough that an idle settings panel is not
 * crossing the IPC boundary forever.
 */
const POLL_MS = 4000;

const INPUT_CLS =
  'w-full px-3 py-2.5 rounded-[10px] text-[13px] text-ink-1 bg-fill-1 border border-line-1 transition-shadow focus:shadow-[0_0_0_3px_rgba(14,116,144,0.30)]';

/**
 * Status of the relay this app hosts, and the two knobs that matter.
 *
 * Deliberately does not expose a port field as if it were free configuration:
 * the relay is a background part of this app, so the common case is that a phone
 * on another network finds it already running. The port is shown read-only for
 * the one case that does need it — setting up a port-forward rule.
 */
const RelaySection: React.FC<SectionProps> = ({ settings, updateSetting }) => {
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [copied, setCopied] = useState(false);

  const poll = useCallback(async () => {
    try {
      setStatus(await invoke<RelayStatus>('get_relay_status'));
    } catch (err) {
      // Not worth a toast: the panel shows the state as unknown and the next
      // poll may well succeed. A relay that is merely not running is a normal
      // state, not something the user did wrong.
      console.error('Failed to read relay status:', err);
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void poll();
    const timer = setInterval(() => { void poll(); }, POLL_MS);
    return () => clearInterval(timer);
  }, [poll]);

  const copy = async (text: string, what: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      showError(`Could not copy the ${what}`);
    }
  };

  return (
    <div className="space-y-5">
      <Toggle
        checked={settings.relay_enabled}
        onChange={(v) => { updateSetting('relay_enabled', v); }}
        label="Reach my devices from other networks"
        description="Runs a relay in the background so a phone on mobile data can still reach this computer. Devices on your home network never need it."
      />

      <div className="text-[11px] text-ink-3 leading-relaxed">
        <p>
          The relay is part of this app — there is nothing else to install, deploy or
          keep running. It only ever forwards encrypted envelopes; it cannot read
          your messages or files.
        </p>
      </div>

      {settings.relay_enabled && (
        <div className="rounded-[12px] border border-line-1 bg-fill-1 p-4 space-y-2.5">
          {status === null && (
            <p className="text-[12px] text-ink-3">Checking…</p>
          )}

          {status !== null && !status.running && (
            <p className="text-[12px] text-ink-2">
              {status.error
                ? `Not running — ${status.error}`
                : 'Not running. Saving your changes restarts it.'}
            </p>
          )}

          {status !== null && status.running && (
            <>
              <div className="flex items-center gap-2">
                <span
                  className="inline-block h-2 w-2 rounded-full bg-emerald-500"
                  aria-hidden="true"
                />
                <p className="text-[12px] text-ink-1">Running</p>
              </div>
              <p className="text-[12px] text-ink-3">
                {status.active_connections} device{status.active_connections === 1 ? '' : 's'} connected
                · {status.registered_devices} known
              </p>

              {status.port !== null && (
                <div className="flex items-center justify-between gap-3 pt-1">
                  <span className="text-[12px] text-ink-2">
                    Listening on port {status.port}
                  </span>
                  <button
                    type="button"
                    onClick={() => { void copy(String(status.port), 'port'); }}
                    className="flex items-center gap-1.5 text-[11px] text-ink-3 hover:text-ink-1 transition-colors"
                  >
                    {copied ? <Check size={12} /> : <Copy size={12} />}
                    {copied ? 'Copied' : 'Copy'}
                  </button>
                </div>
              )}

              {status.tls_pin !== null && (
                <div className="flex items-center justify-between gap-3">
                  <span className="text-[12px] text-ink-2">Certificate pin</span>
                  <button
                    type="button"
                    onClick={() => { void copy(status.tls_pin as string, 'certificate pin'); }}
                    className="flex items-center gap-1.5 text-[11px] text-ink-3 hover:text-ink-1 transition-colors"
                  >
                    <Copy size={12} />
                    Copy
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {settings.relay_enabled && (
        <div className="border-t border-line-1 pt-5 space-y-5">
          <div>
            <label htmlFor="settings-relay-hostname" className="text-[13px] font-medium block mb-1.5 text-ink-1">
              Public hostname
            </label>
            <input
              id="settings-relay-hostname"
              type="text"
              value={settings.relay_hostname}
              onChange={(e) => { updateSetting('relay_hostname', e.target.value); }}
              className={INPUT_CLS}
              aria-describedby="settings-relay-hostname-hint"
            />
            <p id="settings-relay-hostname-hint" className="text-[11px] mt-1.5 text-ink-3">
              Leave this empty unless your relay is published under a real name. It is
              what the certificate is issued for, so a phone checks it before trusting
              the connection.
            </p>
          </div>

          <div className="flex items-start gap-2.5 text-[11px] text-ink-3 leading-relaxed">
            <Globe size={14} className="mt-0.5 shrink-0" aria-hidden="true" />
            <p>
              Reaching this computer from another network also needs this port open
              in your router or a tunnel pointed at it. Conduit does not change your
              router, and it will not open a port on its own.
            </p>
          </div>
        </div>
      )}
    </div>
  );
};

export default RelaySection;
