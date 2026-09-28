import React from 'react';
import { SmsThread } from '../../hooks/useSms';
import { cn } from '../../lib/utils';
import ContextMenu from '../ui/ContextMenu';
import { Copy } from 'lucide-react';

interface ThreadRowProps {
  thread: SmsThread;
  /** Row mirrors the current selection (only possible in edge states). */
  active: boolean;
  onSelect: (id: string) => void;
  timeAgo: (ts: number) => string;
}

/**
 * One conversation in the unified Inbox feed — the message-side counterpart
 * to `NotificationCard`. Keyboard-reachable (a real `<button>`), token-only
 * visual language (`bg-fill-1` / `border-line-1`, accent selection), unread
 * affordances via the accent count chip + semibold sender.
 */
const ThreadRow: React.FC<ThreadRowProps> = ({ thread, active, onSelect, timeAgo }) => {
  // `navigator.clipboard` is typed non-null but can be undefined in insecure
  // contexts — widen so the optional chain is type-honest.
  const clipboard = navigator.clipboard as Clipboard | undefined;

  const name = thread.name || thread.address;
  const unread = thread.unread_count > 0;
  const preview = thread.snippet.length > 50 ? `${thread.snippet.slice(0, 50)}...` : thread.snippet;

  const contextItems = [
    { label: 'Copy', icon: Copy, onClick: () => { void clipboard?.writeText(name); } },
  ];

  return (
    <ContextMenu items={contextItems}>
      <button
        type="button"
        onClick={() => { onSelect(thread.thread_id); }}
        aria-label={
          unread
            ? `Open conversation with ${name}, ${thread.unread_count} unread: ${preview}`
            : `Open conversation with ${name}: ${preview}`
        }
        className={cn(
          'flex w-full items-center gap-3 rounded-2xl border px-4 py-3.5 text-left transition-colors',
          active
            ? 'border-accent-ink bg-accent-dim-ink'
            : 'border-line-1 bg-fill-1 hover:bg-fill-2!'
        )}
      >
        {/* Avatar */}
        <div
          className={cn(
            'flex h-11 w-11 shrink-0 items-center justify-center rounded-full border text-[15px] font-semibold',
            active
              ? 'border-accent-ink/30 bg-accent-dim-ink text-accent-ink'
              : 'border-line-1 bg-fill-2 text-ink-2'
          )}
          aria-hidden="true"
        >
          {thread.name ? thread.name.charAt(0).toUpperCase() : '?'}
        </div>

        {/* Sender + preview */}
        <div className="min-w-0 flex-1">
          <div className="mb-0.5 flex items-center justify-between gap-2">
            <span
              className={cn(
                'truncate text-[14px] text-ink-1',
                unread ? 'font-semibold' : 'font-medium'
              )}
            >
              {name}
            </span>
            <span className="shrink-0 text-[11px] text-ink-3">
              {timeAgo(thread.timestamp)}
            </span>
          </div>
          <p className={cn('truncate text-[12px]', unread ? 'text-ink-2' : 'text-ink-3')}>
            {preview}
          </p>
        </div>

        {/* Unread badge */}
        {unread && (
          <span
            aria-hidden="true"
            className="flex h-[18px] min-w-[18px] shrink-0 items-center justify-center rounded-full bg-accent-ink px-1 text-[10px] font-semibold text-on-accent"
          >
            {thread.unread_count > 99 ? '99+' : thread.unread_count}
          </span>
        )}
      </button>
    </ContextMenu>
  );
};

export default ThreadRow;
