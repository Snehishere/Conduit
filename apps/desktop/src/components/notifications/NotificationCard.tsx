import React, { useState } from 'react';
import { formatTime } from '../../lib/utils';
import { Send, Check, BellOff, Copy, Reply, Smartphone, Laptop, Monitor } from 'lucide-react';
import { cn } from '../../lib/utils';
import SwipeableCard from '../ui/SwipeableCard';
import ContextMenu from '../ui/ContextMenu';

interface NotificationCardProps {
  notification: {
    id: string;
    device_id: string;
    app: string;
    title: string;
    body: string;
    timestamp: number;
    dismissed: boolean;
    device_name?: string;
  };
  onDismiss: (id: string) => void;
  onReply: (id: string, text: string) => void;
  onMarkRead?: (id: string) => void;
}

const NotificationCard: React.FC<NotificationCardProps> = ({
  notification,
  onDismiss,
  onReply,
  onMarkRead,
}) => {
  const [replyText, setReplyText] = useState('');
  const [showReply, setShowReply] = useState(false);
  const [swiped, setSwiped] = useState(false);

  const handleReply = () => {
    if (replyText.trim()) {
      onReply(notification.id, replyText);
      setReplyText('');
      setShowReply(false);
    }
  };

  const handleSwipeLeft = () => {
    if (onMarkRead) {
      onMarkRead(notification.id);
    } else {
      onDismiss(notification.id);
    }
  };

  const handleSwipeRight = () => {
    setSwiped(true);
    onDismiss(notification.id);
  };

  // Per-app chip colors. Chips sit on a light frosted card, so glyph/border
  // values must clear ≥3:1 against the tinted chip background (spec §B7) —
  // Discord's old #00f0ff was 1.3:1 on white and is now the ink cyan.
  const getAppColor = (app: string) => {
    const colors: Record<string, { bg: string; border: string; text: string } | undefined> = {
      WhatsApp: { bg: '#047857', border: '#047857', text: '#FFFFFF' },
      Telegram: { bg: '#2563EB', border: '#2563EB', text: '#FFFFFF' },
      Messages: { bg: '#2563EB', border: '#2563EB', text: '#FFFFFF' },
      Gmail: { bg: '#B91C1C', border: '#B91C1C', text: '#FFFFFF' },
      System: { bg: '#B45309', border: '#B45309', text: '#FFFFFF' },
      Slack: { bg: '#6D28D9', border: '#6D28D9', text: '#FFFFFF' },
      Discord: { bg: '#0E7490', border: '#0E7490', text: '#FFFFFF' },
      SMS: { bg: '#15803D', border: '#15803D', text: '#FFFFFF' },
    };
    return colors[app] || { bg: 'var(--fill-2)', border: 'var(--line-2)', text: 'var(--ink-2)' };
  };

  const getDeviceIcon = (deviceName?: string) => {
    if (!deviceName) return Smartphone;
    if (deviceName.toLowerCase().includes('mac') || deviceName.toLowerCase().includes('laptop')) return Laptop;
    if (deviceName.toLowerCase().includes('desktop')) return Monitor;
    return Smartphone;
  };

  if (swiped) return null;

  const appColor = getAppColor(notification.app);
  const DeviceIcon = getDeviceIcon(notification.device_name);
  // Unread = not yet read/dismissed (the panel folds read ids into `dismissed`).
  const unread = !notification.dismissed;
  // `navigator.clipboard` is typed non-null but can be undefined in insecure
  // contexts — widen so the optional chain is type-honest.
  const clipboard = navigator.clipboard as Clipboard | undefined;

  const contextItems = [
    { label: 'Reply', icon: Reply, onClick: () => { setShowReply(true); } },
    { label: 'Copy text', icon: Copy, onClick: () => { void clipboard?.writeText(`${notification.title}\n${notification.body}`); } },
    { type: 'separator' as const },
    { label: 'Dismiss', icon: BellOff, danger: true, onClick: () => { onDismiss(notification.id); } },
  ];

  return (
    <ContextMenu items={contextItems}>
    <SwipeableCard
      onSwipeLeft={handleSwipeLeft}
      onSwipeRight={handleSwipeRight}
      threshold={80}
      leftLabel="Mark Read"
      rightLabel="Dismiss"
      leftColor="rgba(21,128,61,0.12)"
      rightColor="rgba(185,28,28,0.12)"
      leftIcon={<Check size={14} className="text-success-ink" />}
      rightIcon={<BellOff size={14} className="text-danger-ink" />}
    >
      <div className={cn(
        'relative rounded-2xl transition-colors',
        notification.dismissed
          ? 'opacity-50'
          : ''
      )} style={{
        background: unread ? 'rgba(14, 116, 144, 0.06)' : 'var(--card-bg)',
        borderTop: '1px solid var(--line-1)',
        borderRight: '1px solid var(--line-1)',
        borderBottom: '1px solid var(--line-1)',
        borderLeft: `3px solid ${unread ? 'var(--accent-ink)' : appColor.border}`,
      }}>
        <div style={{ padding: 16 }}>
          <div className="flex items-start gap-3">
            {/* App icon */}
            <div
              className="w-10 h-10 rounded-xl flex items-center justify-center shrink-0"
              style={{ background: appColor.bg }}
            >
              {notification.app === 'Gmail' ? (
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke={appColor.text} strokeWidth="2">
                  <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/>
                  <polyline points="22,6 12,13 2,6"/>
                </svg>
              ) : notification.app === 'System' ? (
                <BellOff size={18} style={{ color: appColor.text }} />
              ) : (
                <span className="text-[14px] font-semibold" style={{ color: appColor.text }}>
                  {notification.app.charAt(0).toUpperCase()}
                </span>
              )}
            </div>

            {/* Content */}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2" style={{ marginBottom: 4 }}>
                <span className="text-[14px] font-semibold text-ink-1">
                  {notification.title}
                </span>
                <span
                  className="text-[10px] font-medium rounded-full"
                  style={{
                    background: 'var(--fill-1)',
                    color: 'var(--ink-2)',
                    padding: '2px 8px',
                  }}
                >
                  {notification.device_name || 'Device'}
                </span>
              </div>
              <p className="text-[13px] leading-relaxed line-clamp-2 text-ink-2">
                {notification.body}
              </p>
              <div className="flex items-center gap-2" style={{ marginTop: 8 }}>
                <DeviceIcon size={12} style={{ color: 'var(--ink-3)' }} />
                <span className="text-[11px] text-ink-3">
                  {formatTime(notification.timestamp)}
                </span>
              </div>
            </div>

            {/* Explicit message actions, in addition to the swipe gestures. */}
            <div className="flex items-center gap-1.5 shrink-0">
              {!notification.dismissed && (
                <button
                  className="surf-clear rounded-lg text-[11px] font-medium text-ink-1 transition-colors hover:bg-fill-2!"
                  style={{ padding: '6px 10px', border: '1px solid var(--line-1)' }}
                  onClick={() => { onMarkRead?.(notification.id); }}
                  aria-label="Mark notification as read"
                >
                  Mark
                </button>
              )}
              <button
                className="surf-clear rounded-lg text-[11px] font-medium text-ink-1 transition-colors hover:bg-fill-2!"
                style={{ padding: '6px 12px', border: '1px solid var(--line-1)' }}
                onClick={() => { setShowReply(!showReply); }}
                aria-label="Reply to notification"
              >
                Reply
              </button>
            </div>
          </div>

          {/* Reply input */}
          {showReply && (
            <div className="flex gap-2" style={{ marginTop: 12, marginLeft: 52 }}>
              <input
                type="text"
                value={replyText}
                onChange={(e) => { setReplyText(e.target.value); }}
                placeholder="Type a reply..."
                className="flex-1 rounded-lg text-[13px] transition-colors placeholder:text-ink-3"
                style={{
                  padding: '8px 14px',
                  background: 'var(--fill-1)',
                  border: '1px solid var(--line-1)',
                  color: 'var(--ink-1)',
                }}
                onKeyDown={(e) => { if (e.key === 'Enter') handleReply(); }}
                autoFocus
              />
              <button
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent-dim-ink! text-accent-ink transition-colors hover:bg-accent-ink! hover:text-on-accent"
                aria-label="Send reply"
                onClick={handleReply}
              >
                <Send size={14} />
              </button>
            </div>
          )}
        </div>
      </div>
    </SwipeableCard>
    </ContextMenu>
  );
};

export default NotificationCard;
