import { cn } from '../../lib/utils';

export type StatusType = 'connected' | 'syncing' | 'offline' | 'error' | 'pairing';

interface StatusBadgeProps {
  status: StatusType;
  label?: string;
  className?: string;
  size?: 'sm' | 'md';
}

/* Ink-safe semantic colors (spec §B6/B7): the badge renders as a clear chip so
   it stays legible both on white frosted panels (StatusBar, detail drawer)
   and over the dark starfield (DeviceHub header). Bright cyan / neon variants
   fail contrast on white — the ink variants read ≥4.5:1 on both. */
const statusConfig: Record<StatusType, { color: string; pulse: boolean }> = {
  connected: { color: 'var(--success-ink)', pulse: false },
  syncing: { color: 'var(--accent-ink)', pulse: true },
  offline: { color: 'var(--ink-2)', pulse: false },
  error: { color: 'var(--danger-ink)', pulse: false },
  pairing: { color: 'var(--warning-ink)', pulse: true },
};

const StatusBadge: React.FC<StatusBadgeProps> = ({ status, label, className, size = 'sm' }) => {
  const config = statusConfig[status];

  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 font-medium rounded-full',
        size === 'sm' ? 'px-2 py-0.5 text-[10px]' : 'px-2.5 py-1 text-[11px]',
        className,
      )}
      style={{
        background: 'var(--fill-1)',
        boxShadow: 'inset 0 0 0 1px var(--line-1)',
        color: config.color,
      }}
    >
      <span
        className={cn(
          'rounded-full shrink-0',
          size === 'sm' ? 'w-1.5 h-1.5' : 'w-2 h-2',
          config.pulse && 'animate-pulse-dot',
        )}
        style={{ background: config.color }}
      />
      {label || status.charAt(0).toUpperCase() + status.slice(1)}
    </span>
  );
};

export default StatusBadge;
