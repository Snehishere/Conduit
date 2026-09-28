import { useCallback, useState } from 'react';

const STORAGE_KEY = 'conduit-read-notifications';
const CHANGE_EVENT = 'conduit-notifications-read-change';

function loadReadIds(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]') as string[]);
  } catch {
    // Storage unavailable (private mode / disabled) — treat everything as unread.
    return new Set();
  }
}

/**
 * Shared "read" ledger for notifications.
 *
 * A notification counts as unread until its id lands in this set, which is
 * persisted to `localStorage` and broadcast on `window` so the app shell
 * (`App.tsx` → dock badge) and every mounted notification surface stay in
 * sync. Extracted from `NotificationPanel` so the unified Inbox feed can fold
 * the same ids into `NotificationCard`'s `dismissed` flag without duplicating
 * the contract.
 *
 * @param onMarkRead Host callback (e.g. sends `notification.mark_read` over
 * the websocket) — invoked once per id that transitions to read.
 */
export function useReadNotifications(onMarkRead?: (id: string) => void) {
  const [readIds, setReadIds] = useState<Set<string>>(loadReadIds);

  const markRead = useCallback((id: string) => {
    setReadIds((current) => {
      if (current.has(id)) return current;
      const next = new Set(current);
      next.add(id);
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify([...next]));
      } catch {
        /* storage may be unavailable */
      }
      window.dispatchEvent(new Event(CHANGE_EVENT));
      onMarkRead?.(id);
      return next;
    });
  }, [onMarkRead]);

  return { readIds, markRead };
}

export default useReadNotifications;
