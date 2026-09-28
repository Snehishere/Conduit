import React, { useRef } from 'react';
import { motion } from 'motion/react';
import {
  Home,
  Bell,
  Phone,
  Folder,
  Monitor,
  MousePointer2,
  Zap,
  Settings,
  Radio,
  Plus,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { cn } from '../../lib/utils';

type ActiveView = 'web' | 'notifications' | 'messages' | 'calls' | 'files' | 'clipboard' | 'settings' | 'screen_mirror' | 'remote_input' | 'devices' | 'automation';

interface FloatingDockProps {
  activeView: ActiveView;
  onViewChange: (view: ActiveView) => void;
  onPairDevice: () => void;
  notificationCount: number;
  deviceCount: number;
  fileCount: number;
  messageCount: number;
  clipboardCount: number;
  callActive: boolean;
  /** Whether surround sound mode is active */
  surroundSoundActive?: boolean;
  /** Toggle surround sound mode */
  onToggleSurroundSound?: () => void;
}

interface NavItem {
  id: ActiveView;
  label: string;
  icon: LucideIcon;
  shortcut?: string;
}

/* Revision 3: ONE non-expanding dock row — no flyout tray. Every destination
   sits inline: Home, Inbox, Calls, Files, Mirror, Remote, Automation,
   Settings, then the optional Surround toggle and the Pair "+" button last.
   The merged screens mean Inbox covers notifications+messages and Files
   covers files+clipboard. */
const PRIMARY_NAV: NavItem[] = [
  { id: 'web', label: 'Home', icon: Home },
  { id: 'notifications', label: 'Inbox', icon: Bell, shortcut: 'Ctrl+M' },
  { id: 'calls', label: 'Calls', icon: Phone },
  { id: 'files', label: 'Files', icon: Folder, shortcut: 'Ctrl+T' },
];

const SECONDARY_NAV: NavItem[] = [
  { id: 'screen_mirror', label: 'Mirror', icon: Monitor },
  { id: 'remote_input', label: 'Remote', icon: MousePointer2 },
  { id: 'automation', label: 'Automation', icon: Zap },
  { id: 'settings', label: 'Settings', icon: Settings, shortcut: 'Ctrl+,' },
];

/* ── Smoked-glass icon scheme (spec §B3, revision 2) ────────────────────────
   Chrome (glass gradient, blur, radius, shadow, z-index) comes from the
   `.dock` class in globals.css — no inline glass styles here.
   Default icon : --on-glass-muted @ 1.5 stroke
   Hover/focus  : --on-glass + --fill-1 wash (bg-fill-1)
   Active item  : --accent-ink icon on the --accent-dim-ink pill (the old
                  near-black pill was invisible on a dark smoked dock)
   Active dot   : --accent-ink core with a --dot-ring halo
   Note: the `!` on hover/focus fills keeps them ahead of the base-layer
   `button { background: none }` reset in globals.css. */
const BTN_BASE =
  'relative flex cursor-pointer items-center justify-center transition-colors duration-150';

const BTN_IDLE =
  'text-on-glass-muted hover:text-on-glass hover:bg-fill-1! focus-visible:text-on-glass focus-visible:bg-fill-1!';

const BTN_ACTIVE = 'text-accent-ink bg-accent-dim-ink!';

/* Shared tooltip chrome: frosted + on-glass text (`.dock-tip`), position and
   show/hide motion kept from the original. */
const TIP_BASE =
  'dock-tip absolute bottom-full left-1/2 z-[110] mb-3 -translate-x-1/2 whitespace-nowrap opacity-0 transition-opacity duration-150 group-focus-within:opacity-100 group-hover:opacity-100';

/* The dock collapses merged views into their primary group: Inbox lights up
   for notifications AND messages; Files for files AND clipboard. */
const isItemActive = (item: ActiveView, activeView: ActiveView): boolean => {
  if (item === 'notifications') return activeView === 'notifications' || activeView === 'messages';
  if (item === 'files') return activeView === 'files' || activeView === 'clipboard';
  return activeView === item;
};

/* Decorative group separator inside the single dock row. */
const Divider: React.FC = () => (
  <div
    aria-hidden="true"
    style={{ width: 1, height: 22, background: 'var(--line-2)', margin: '0 6px', borderRadius: 1 }}
  />
);

const FloatingDock: React.FC<FloatingDockProps> = ({
  activeView,
  onViewChange,
  onPairDevice,
  notificationCount,
  fileCount,
  messageCount,
  clipboardCount,
  callActive,
  surroundSoundActive = false,
  onToggleSurroundSound,
}) => {
  const dockRef = useRef<HTMLDivElement>(null);

  const getBadge = (id: ActiveView): number | string | null => {
    if (id === 'notifications') {
      const total = notificationCount + messageCount;
      return total > 0 ? total : null;
    }
    if (id === 'calls') return callActive ? '•' : null;
    if (id === 'files') {
      const total = fileCount + clipboardCount;
      return total > 0 ? total : null;
    }
    return null;
  };

  const renderItem = (item: NavItem) => {
    const Icon = item.icon;
    const isActive = isItemActive(item.id, activeView);
    const badge = getBadge(item.id);
    const tooltipId = `tooltip-${item.id}`;
    const tooltipText = item.shortcut ? `${item.label} (${item.shortcut})` : item.label;

    let accessibleLabel = item.label;
    if (badge !== null && typeof badge === 'number') {
      accessibleLabel = `${item.label}, ${badge} unread`;
    } else if (badge === '•') {
      accessibleLabel = `${item.label}, active call`;
    }

    return (
      <div key={item.id} className="relative group">
        <motion.button
          onClick={() => { onViewChange(item.id); }}
          aria-label={accessibleLabel}
          aria-describedby={tooltipId}
          aria-current={isActive ? 'page' : undefined}
          whileTap={{ scale: 0.88 }}
          whileHover={{ scale: 1.1 }}
          transition={{ type: 'spring', stiffness: 500, damping: 25 }}
          className={cn(BTN_BASE, isActive ? BTN_ACTIVE : BTN_IDLE)}
          style={{ width: 40, height: 40, borderRadius: 10 }}
        >
          {/* Active indicator dot — accent core + theme ring (readable on smoked glass) */}
          {isActive && (
            <motion.span
              layoutId="dock-active-dot"
              className="absolute -bottom-2.5 left-1/2 h-1 w-1 -translate-x-1/2 rounded-full"
              style={{
                background: 'var(--accent-ink)',
                boxShadow: '0 0 0 2px var(--dot-ring)',
              }}
              transition={{ type: 'spring', stiffness: 500, damping: 30 }}
            />
          )}
          <Icon size={18} strokeWidth={isActive ? 2 : 1.5} />
          {badge !== null && (
            <motion.span
              initial={{ scale: 0 }}
              animate={{ scale: 1 }}
              transition={{ type: 'spring', stiffness: 600, damping: 20 }}
              aria-hidden="true"
              style={{
                position: 'absolute',
                top: 2,
                right: 2,
                minWidth: 14,
                height: 14,
                borderRadius: 7,
                fontSize: 9,
                fontWeight: 700,
                padding: '0 4px',
                background: item.id === 'calls' ? 'var(--warning-ink)' : 'var(--accent-ink)',
                color: 'var(--on-accent)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                boxShadow: '0 0 0 2px var(--dot-ring)',
              }}
            >
              {badge}
            </motion.span>
          )}
        </motion.button>
        {/* Tooltip */}
        <div id={tooltipId} role="tooltip" className={TIP_BASE}>
          {tooltipText}
        </div>
      </div>
    );
  };

  return (
    <div
      ref={dockRef}
      data-tutorial="dock"
      role="navigation"
      aria-label="Main navigation"
      className="dock flex items-center"
    >
      {PRIMARY_NAV.map((item) => renderItem(item))}

      <Divider />

      {SECONDARY_NAV.map((item) => renderItem(item))}

      {/* Surround sound toggle — inline in the single dock row.

          Currently dead UI: no call site passes `onToggleSurroundSound`, so this
          block never renders, and `surroundSoundActive` in useCalls is not
          surfaced anywhere else. Left in place rather than deleted because wiring
          the toggle is a behaviour change outside the copy remit.

          The old aria-label ended "— combine all devices", which promises
          simultaneous multi-device audio mixing. Nothing in the frontend mixes
          audio; `useCalls` only flips a boolean from an inbound
          `surround_started` frame. The unsupported clause is dropped rather than
          restated. */}
      {onToggleSurroundSound && (
        <>
          <Divider />
          <div className="relative group">
            <motion.button
              onClick={onToggleSurroundSound}
              aria-label={surroundSoundActive ? 'Disable surround sound' : 'Enable surround sound'}
              aria-pressed={surroundSoundActive}
              aria-describedby="tooltip-surround"
              whileTap={{ scale: 0.88 }}
              whileHover={{ scale: 1.1 }}
              transition={{ type: 'spring', stiffness: 500, damping: 25 }}
              className={cn(BTN_BASE, surroundSoundActive ? BTN_ACTIVE : BTN_IDLE)}
              style={{ width: 40, height: 40, borderRadius: 10 }}
            >
              <Radio size={18} strokeWidth={surroundSoundActive ? 2 : 1.5} />
              {surroundSoundActive && (
                <span
                  aria-hidden="true"
                  style={{
                    position: 'absolute',
                    top: 2,
                    right: 2,
                    width: 6,
                    height: 6,
                    borderRadius: 3,
                    background: 'var(--accent-ink)',
                    boxShadow: '0 0 0 2px var(--dot-ring)',
                  }}
                />
              )}
            </motion.button>
            <div id="tooltip-surround" role="tooltip" className={TIP_BASE}>
              {surroundSoundActive ? 'Surround sound on' : 'Surround sound'}
            </div>
          </div>
        </>
      )}

      <Divider />

      {/* Pair device — always the last control in the dock */}
      <div className="relative group">
        <motion.button
          onClick={() => { onPairDevice(); }}
          aria-label="Pair device"
          aria-describedby="tooltip-pair"
          whileTap={{ scale: 0.88 }}
          whileHover={{ scale: 1.1 }}
          transition={{ type: 'spring', stiffness: 500, damping: 25 }}
          className={cn(BTN_BASE, BTN_IDLE)}
          style={{ width: 40, height: 40, borderRadius: 10 }}
        >
          <Plus size={18} strokeWidth={1.75} />
        </motion.button>
        <div id="tooltip-pair" role="tooltip" className={TIP_BASE}>
          Pair (Ctrl+N)
        </div>
      </div>
    </div>
  );
};

export default FloatingDock;
