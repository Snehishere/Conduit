import React, { useMemo } from 'react';
import { ArrowLeft, Bell } from 'lucide-react';
import NotificationCard from '../notifications/NotificationCard';
import { useReadNotifications } from '../notifications/useReadNotifications';
import type { NotificationItem } from '../notifications/NotificationPanel';
import MessageThread from '../messages/MessageThread';
import ThreadRow from '../messages/ThreadRow';
import EmptyState from '../ui/EmptyState';
import { SkeletonList } from '../ui/Skeleton';
import { cn } from '../../lib/utils';
import type { SmsThread } from '../../hooks/useSms';

export interface InboxScreenProps {
  /** Unread notifications — half of the combined "N unread" chip. */
  notificationCount: number;
  /** Unread conversations — half of the combined "N unread" chip. */
  messageCount: number;
  /* Notification rows (NotificationCard surface) */
  notifications: NotificationItem[];
  onDismiss: (id: string) => void;
  onReply: (id: string, text: string) => void;
  onMarkRead?: (id: string) => void;
  /* Message rows (ThreadRow surface) */
  threads: SmsThread[];
  selectedThread: string | null;
  onSelectThread: (id: string) => void;
  /* Drill-in: resolved thread shows MessageThread instead of the feed */
  thread: SmsThread | null;
  onSend: (to: string, body: string) => void;
  onBack: () => void;
  timeAgo: (ts: number) => string;
  loading?: boolean;
}

/** One row of the merged feed, tagged by source and pre-sorted by `ts`. */
type FeedItem =
  | { kind: 'notification'; key: string; ts: number; notification: NotificationItem }
  | { kind: 'thread'; key: string; ts: number; thread: SmsThread };

/**
 * ONE unified Inbox — notifications and messages share a single chronological
 * feed inside one frosted panel (revision-3: no tabs, no tablist).
 *
 * The screen owns the panel, its heading row, and the scroll-region contract
 * (fills `<main>`, header + min-h-0 content region — same shell rhythm as
 * CallsNotificationsSplit). Notification rows reuse `NotificationCard` (app
 * colour coding, Mark/Reply, unread state) and message rows use `ThreadRow`;
 * clicking a message row selects the thread, which drills the SAME panel into
 * `MessageThread` — the heading row becomes the "← Inbox" back control, so the
 * feed is always one click away on desktop as well as mobile.
 */
const InboxScreen: React.FC<InboxScreenProps> = ({
  notificationCount,
  messageCount,
  notifications,
  onDismiss,
  onReply,
  onMarkRead,
  threads,
  selectedThread,
  onSelectThread,
  thread,
  onSend,
  onBack,
  timeAgo,
  loading = false,
}) => {
  // Same read-id ledger NotificationPanel uses, so "Mark" updates the card,
  // the combined chip (via the host's count) and the dock badge together.
  const { readIds, markRead } = useReadNotifications(onMarkRead);

  // Merge both sources into one list, newest first.
  const feed = useMemo<FeedItem[]>(() => {
    const items: FeedItem[] = [
      ...notifications.map((notification) => ({
        kind: 'notification' as const,
        key: `notification-${notification.id}`,
        ts: notification.timestamp,
        notification,
      })),
      ...threads.map((feedThread) => ({
        kind: 'thread' as const,
        key: `thread-${feedThread.thread_id}`,
        ts: feedThread.timestamp,
        thread: feedThread,
      })),
    ];
    return items.sort((a, b) => b.ts - a.ts);
  }, [notifications, threads]);

  const unreadTotal = notificationCount + messageCount;

  return (
    <div className="flex h-full min-h-0 flex-col" style={{ padding: 28 }}>
      <div className="surf-frost flex min-h-0 flex-1 flex-col overflow-hidden rounded-[24px]">
        {/* Heading row — one title for the whole screen + combined unread chip.
            While a thread is drilled into it doubles as the desktop back control. */}
        <div className="flex shrink-0 items-center justify-between gap-3" style={{ padding: '20px 24px 16px' }}>
          <h1 className="text-[24px] font-bold text-ink-1">
            {thread ? (
              <button
                type="button"
                onClick={onBack}
                aria-label="Back to inbox"
                className="-ml-1.5 flex items-center gap-1.5 rounded-lg px-1.5 py-0.5 text-ink-2 transition-colors hover:bg-fill-1! hover:text-ink-1"
              >
                <ArrowLeft size={20} aria-hidden="true" />
                Inbox
              </button>
            ) : (
              'Inbox'
            )}
          </h1>
          <span
            aria-label={unreadTotal > 0 ? `${unreadTotal} unread items` : 'No unread items'}
            className={cn(
              'shrink-0 rounded-full px-2.5 py-1 text-[11px] font-semibold leading-none',
              unreadTotal > 0 ? 'bg-accent-ink text-on-accent' : 'bg-fill-2 text-ink-2'
            )}
          >
            {unreadTotal} unread
          </span>
        </div>

        {/* Content region — one scroll owner at a time, never nested */}
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {thread ? (
            /* Drill-in: same panel, thread view. `MessageThread` owns its own
               scroll here; the heading row above is the way back out. */
            <div className="min-h-0 flex-1 overflow-hidden">
              <MessageThread thread={thread} onSend={onSend} onBack={onBack} timeAgo={timeAgo} />
            </div>
          ) : (
            /* Unified feed — notifications + messages, newest first. */
            <div className="min-h-0 flex-1 overflow-y-auto" style={{ padding: '0 24px 24px' }}>
              {loading && feed.length === 0 ? (
                <SkeletonList count={4} />
              ) : feed.length === 0 ? (
                <div className="flex min-h-full w-full items-center justify-center">
                  <EmptyState
                    icon={Bell}
                    title="All caught up"
                    description="Notifications and messages from your paired devices will show up here in real time"
                  />
                </div>
              ) : (
                <ul className="flex flex-col gap-3">
                  {feed.map((item) =>
                    item.kind === 'notification' ? (
                      <li key={item.key}>
                        <NotificationCard
                          notification={{
                            ...item.notification,
                            // Fold locally-marked-read ids into `dismissed`
                            // (NotificationPanel's contract) for unread state.
                            dismissed: item.notification.dismissed || readIds.has(item.notification.id),
                          }}
                          onDismiss={onDismiss}
                          onReply={onReply}
                          onMarkRead={markRead}
                        />
                      </li>
                    ) : (
                      <li key={item.key}>
                        <ThreadRow
                          thread={item.thread}
                          active={selectedThread === item.thread.thread_id}
                          onSelect={onSelectThread}
                          timeAgo={timeAgo}
                        />
                      </li>
                    )
                  )}
                </ul>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default InboxScreen;
