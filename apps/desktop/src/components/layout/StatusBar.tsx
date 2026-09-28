import { useState, useEffect, type CSSProperties } from 'react';
import { Wifi, Shield, ShieldOff, Cpu } from 'lucide-react';
import StatusBadge, { type StatusType } from '../ui/StatusBadge';
import { invoke } from '../../lib/tauri';
import type { EncryptionStatus } from '../../hooks/useEncryption';

interface StatusBarProps {
  connected: boolean;
  deviceCount: number;
  /**
   * Encryption state, supplied by `useEncryption`. Deliberately an object
   * rather than a boolean: the old `encryptionEnabled` prop was passed as a
   * bare identifier, which evaluated to the function reference and was
   * therefore always truthy — the "E2E" chip could never turn off.
   */
  encryption: EncryptionStatus;
  /** App version from `get_current_version`; hidden until fetched. */
  version?: string;
}

/* The strip is frosted glass, but StatusBadge is a shared component that emits
   dark-surface tokens (var(--success) #22c55e, var(--text-3), var(--accent)).
   Custom properties inherit, so scoping them here keeps the strip ink-safe
   without editing a file owned by another agent:
     success  → --success-ink (#15803D, 4.5:1 on white)
     offline  → --ink-2      (#46505F, 7.4:1 on white)
     cyan     → --accent-ink (never #00F0FF on this white strip) */
const STATUS_BADGE_SCOPE = {
  '--success': 'var(--success-ink)',
  '--text-3': 'var(--ink-2)',
  '--accent': 'var(--accent-ink)',
} as CSSProperties;

const SEPARATOR_COLOR = 'var(--ink-3)';

const StatusBar = ({
  connected,
  deviceCount,
  encryption,
  version,
}: StatusBarProps) => {
  const [ramMb, setRamMb] = useState<number | null>(null);

  useEffect(() => {
    const fetchRam = async () => {
      try {
        const info = await invoke<{ ram_used_mb: number }>('get_system_info');
        setRamMb(info.ram_used_mb);
      } catch {
        /* RAM display is best-effort */
      }
    };
    void fetchRam();
    const interval = setInterval(fetchRam, 5000);
    return () => { clearInterval(interval); };
  }, []);

  const connectionStatus: StatusType = connected ? 'connected' : 'offline';

  return (
    <footer
      data-tutorial="status-bar"
      className="flex items-center justify-between px-3 select-none"
      style={{
        height: 26,
        /* Frost strip (spec §B5) — ink text, never cyan on a light strip */
        backgroundColor: 'var(--frost-1)',
        backgroundImage: 'var(--frost-noise), linear-gradient(180deg, var(--frost-1), var(--frost-2))',
        backgroundSize: '160px 160px, auto',
        backdropFilter: 'blur(20px) saturate(160%)',
        WebkitBackdropFilter: 'blur(20px) saturate(160%)',
        borderTop: '1px solid var(--frost-border)',
        boxShadow: '0 -1px 0 rgba(0,0,0,0.14)',
        fontSize: 11,
        color: 'var(--ink-2)',
      }}
    >
      <div className="flex gap-2.5 items-center">
        <span className="flex items-center gap-1">
          <Wifi size={11} strokeWidth={1.5} /> {deviceCount} device{deviceCount !== 1 ? 's' : ''}
        </span>
        <span style={{ color: SEPARATOR_COLOR }}>·</span>
        <span style={STATUS_BADGE_SCOPE}>
          <StatusBadge status={connectionStatus} size="sm" label={connected ? 'Connected' : 'Offline'} />
        </span>
        <span style={{ color: SEPARATOR_COLOR }}>·</span>
        {/* Never says "E2E": nothing in this build is end-to-end encrypted —
            the relay terminates the envelope. The `title` carries the full
            reason and `aria-label` repeats it, since `title` alone is not
            reliably announced. */}
        <span
          className="flex items-center gap-1"
          data-testid="encryption-chip"
          data-encryption-level={encryption.level}
          title={encryption.detail}
          aria-label={`Encryption: ${encryption.label}. ${encryption.detail}`}
        >
          {encryption.level === 'no-key' ? (
            <ShieldOff size={11} strokeWidth={1.5} aria-hidden="true" />
          ) : (
            <Shield size={11} strokeWidth={1.5} aria-hidden="true" />
          )}
          {encryption.label}
        </span>
      </div>
      <div className="flex gap-3 items-center">
        {version && (
          <>
            <span aria-label={`Conduit version ${version}`}>v{version}</span>
            <span style={{ color: SEPARATOR_COLOR }}>·</span>
          </>
        )}
        <span className="flex items-center gap-1">
          <Cpu size={11} strokeWidth={1.5} />
          {ramMb !== null ? `${ramMb} MB` : '—'}
        </span>
      </div>
    </footer>
  );
};

export default StatusBar;
