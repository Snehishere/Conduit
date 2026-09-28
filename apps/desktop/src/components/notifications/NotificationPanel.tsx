import React, { useState } from 'react';
import { Bell, Check } from 'lucide-react';
import NotificationCard from './NotificationCard';
import { useReadNotifications } from './useReadNotifications';
import { cn } from '../../lib/utils';
import EmptyState from '../ui/EmptyState';
import { SkeletonList } from '../ui/Skeleton';

export interface NotificationItem {
  id: string;
  device_id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  actions?: string;
  dismissed: boolean;
  device_name?: string;
}

interface NotificationPanelProps {
  notifications: NotificationItem[];
  onDismiss: (id: string) => void;
  onReply: (id: string, text: string) => void;
  onMarkRead?: (id: string) => void;
  loading?: boolean;
  /**
   * Render flush inside a host panel (Calls split view) — drops the screen
   * gutter so the host controls panel spacing.
   */
  embedded?: boolean;
  /**
   * Render content-only inside a host panel — no own surface,
   * no gutter; the host owns the frosted panel, padding, and scroll region.
   * (Originally the tabbed Inbox host; the unified Inbox feed now renders
   * `NotificationCard` directly, so the prop is kept for any other host.)
   */
  bare?: boolean;
}

const NotificationPanel: React.FC<NotificationPanelProps> = ({
  notifications,
  onDismiss,
  onReply,
  onMarkRead,
  loading = false,
  embedded = false,
  bare = false,
}) => {
  const [filter, setFilter] = useState<'all' | 'unread'>('all');
  // Read-id ledger lives in a shared hook (same storage/event contract the
  // unified Inbox feed uses) — behaviour is unchanged from the inline version.
  const { readIds, markRead } = useReadNotifications(onMarkRead);

  const filteredNotifications = notifications.filter((n) => {
    if (filter === 'unread') return !n.dismissed && !readIds.has(n.id);
    return true;
  });

  // Group by app — a Map's `get()` honestly returns `| undefined`, so the
  // first-occurrence branch is type-honest (Record index access isn't, without
  // `noUncheckedIndexedAccess`, and would trip no-unnecessary-condition).
  const grouped = new Map<string, NotificationItem[]>();
  for (const n of filteredNotifications) {
    const key = n.app || 'Other';
    const bucket = grouped.get(key);
    if (bucket) {
      bucket.push(n);
    } else {
      grouped.set(key, [n]);
    }
  }

  const sortedGroups = [...grouped.entries()].sort((a, b) => {
    const latestA = Math.max(...a[1].map((n) => n.timestamp));
    const latestB = Math.max(...b[1].map((n) => n.timestamp));
    return latestB - latestA;
  });

  const markAllRead = () => {
    notifications.forEach((n) => {
      if (!n.dismissed) markRead(n.id);
    });
  };

  const unreadCount = notifications.filter((n) => !n.dismissed && !readIds.has(n.id)).length;

  // Segmented control (spec §B5) — the same chrome PanelTabs promotes for the
  // merged screens: surf-clear tray + accent-dim/accent-ink selected pill.
  // (The old near-black pill + white label was invisible on smoked glass.)
  const renderSegment = (value: 'all' | 'unread', label: string) => {
    const active = filter === value;
    return (
      <button
        type="button"
        aria-pressed={active}
        className={cn(
          'rounded-lg text-[12px] font-medium transition-colors',
          active
            ? 'bg-accent-dim-ink! text-accent-ink'
            : 'text-ink-2 hover:bg-fill-1! hover:text-ink-1'
        )}
        style={{ padding: '6px 14px' }}
        onClick={() => { setFilter(value); }}
      >
        {label}
      </button>
    );
  };

  const content = (
    <>
      {/* Header — 24px to content */}
      <div className="flex shrink-0 items-center justify-between" style={{ marginBottom: 24 }}>
        <div>
          <h1 className="text-[24px] font-bold text-ink-1">
            Notifications
          </h1>
          <p className="text-[13px] text-ink-2" style={{ marginTop: 4 }}>
            Notifications from your paired devices, grouped by app.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <div
            className="surf-clear flex items-center gap-0.5 rounded-[10px]"
            style={{ padding: 3 }}
            role="group"
            aria-label="Filter notifications"
          >
            {renderSegment('all', 'All')}
            {renderSegment('unread', 'Unread')}
          </div>
          {unreadCount > 0 && (
            <button
              type="button"
              className="surf-clear flex items-center gap-1.5 rounded-lg text-[12px] font-medium text-ink-1 transition-colors hover:bg-fill-2!"
              style={{ padding: '6px 12px' }}
              onClick={markAllRead}
            >
              <Check size={14} />
              Mark all read
            </button>
          )}
        </div>
      </div>

      {/* Notification List */}
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
        {loading && sortedGroups.length === 0 ? (
          <SkeletonList count={4} />
        ) : sortedGroups.length === 0 ? (
          <div className="flex min-h-full w-full items-center justify-center">
            <EmptyState
              icon={Bell}
              title="All caught up"
              description="Notifications from your paired devices will show up here in real time"
            />
          </div>
        ) : (
          sortedGroups.map(([app, items]) => (
            <div key={app} className="flex flex-col gap-3">
              <span className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-3">
                {app}
              </span>
              {items.map((notification) => (
                <NotificationCard
                  key={notification.id}
                  notification={{ ...notification, dismissed: notification.dismissed || readIds.has(notification.id) }}
                  onDismiss={onDismiss}
                  onMarkRead={markRead}
                  onReply={onReply}
                />
              ))}
            </div>
          ))
        )}
      </div>
    </>
  );

  // Content-only mode — the host owns the surface, padding, and scroll contract.
  if (bare) {
    return (
      <div className="flex h-full flex-col" style={{ padding: '16px 32px 28px' }}>
        {content}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col" style={embedded ? undefined : { padding: 28 }}>
      {/* One frosted panel wrapping the whole notification surface (spec §B5) */}
      <div
        className="surf-frost flex min-h-0 flex-1 flex-col overflow-hidden rounded-[24px]"
        style={{ padding: '28px 32px' }}
      >
        {content}
      </div>
    </div>
  );
};

export default NotificationPanel;
