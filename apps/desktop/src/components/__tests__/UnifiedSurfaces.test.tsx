import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeAll } from 'vitest';

// jsdom has no scrollIntoView (MessageThread autoscroll).
beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn();
});
import InboxScreen from '../inbox/InboxScreen';
import FilesScreen from '../files/FilesScreen';
import type { NotificationItem } from '../notifications/NotificationPanel';
import type { SmsThread } from '../../hooks/useSms';

vi.mock('framer-motion', () => ({
  motion: {
    div: (props: any) => <div {...props} />,
    button: (props: any) => <button {...props} />,
    li: (props: any) => <li {...props} />,
  },
  AnimatePresence: ({ children }: any) => <>{children}</>,
}));

const notification: NotificationItem = {
  id: 'n1',
  device_id: 'dev-1',
  app: 'slack',
  title: 'Slack mention',
  body: 'you were mentioned',
  timestamp: 1000,
  dismissed: false,
};

const thread: SmsThread = {
  thread_id: 't1',
  address: '+15550001',
  name: 'Alice',
  snippet: 'see you soon',
  unread_count: 1,
  timestamp: 2000,
  messages: [
    { id: 'm1', address: '+15550001', body: 'see you soon', timestamp: 2000, read: false, is_outgoing: false },
  ],
};

const inboxProps = {
  notificationCount: 1,
  messageCount: 1,
  notifications: [notification],
  onDismiss: vi.fn(),
  onReply: vi.fn(),
  onMarkRead: vi.fn(),
  threads: [thread],
  selectedThread: null,
  onSelectThread: vi.fn(),
  thread: null,
  onSend: vi.fn(),
  onBack: vi.fn(),
  timeAgo: (ts: number) => `t${ts}`,
};

describe('InboxScreen — one unified feed (no tabs)', () => {
  it('renders notification and thread rows inside the SAME single list', () => {
    render(<InboxScreen {...inboxProps} />);

    const notifRow = screen.getByText('Slack mention').closest('li');
    const threadRow = screen.getByText(/Alice/).closest('li');
    expect(notifRow).toBeTruthy();
    expect(threadRow).toBeTruthy();

    const feedList = notifRow!.closest('ul');
    expect(feedList).toBeTruthy();
    expect(threadRow!.closest('ul')).toBe(feedList);
  });

  it('sorts the merged feed newest-first (thread above older notification)', () => {
    render(<InboxScreen {...inboxProps} />);

    const notifRow = screen.getByText('Slack mention').closest('li')!;
    const threadRow = screen.getByText(/Alice/).closest('li')!;
    const position = threadRow.compareDocumentPosition(notifRow);
    expect(position & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('has no tablist/tab/tabpanel at all', () => {
    const { container } = render(<InboxScreen {...inboxProps} />);
    expect(container.querySelectorAll('[role="tab"], [role="tablist"], [role="tabpanel"]')).toHaveLength(0);
  });

  it('clicking a thread row drills into that thread', () => {
    const onSelectThread = vi.fn();
    render(<InboxScreen {...inboxProps} onSelectThread={onSelectThread} />);
    fireEvent.click(screen.getByRole('button', { name: /Open conversation with Alice/ }));
    expect(onSelectThread).toHaveBeenCalledWith('t1');
  });

  it('shows MessageThread with a working back control when a thread is selected', () => {
    const onBack = vi.fn();
    render(<InboxScreen {...inboxProps} thread={thread} onBack={onBack} />);
    const back = screen.getByRole('button', { name: 'Back to inbox' });
    fireEvent.click(back);
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it('keeps the combined unread chip', () => {
    render(<InboxScreen {...inboxProps} />);
    expect(screen.getByLabelText('2 unread items')).toBeTruthy();
  });
});

const transfer = {
  id: 'tr-1',
  name: 'photo.png',
  size: 1024,
  mime: 'image/png',
  from_device: 'phone',
  to_device: 'desktop',
  status: 'complete',
  chunks_received: 1,
  total_chunks: 1,
  saved_path: '/tmp/photo.png',
  timestamp: 3000,
};

const clipboardItem = {
  id: 7,
  content: 'a very long clipboard snippet that makes this card tall',
  mime: 'text/plain',
  source_device: 'phone',
  timestamp: 4000,
  pinned: false,
};

const filesProps = {
  transfers: [transfer],
  activeTransferId: null,
  activeProgress: 0,
  onAccept: vi.fn(),
  onCancel: vi.fn(),
  onResume: vi.fn(),
  onOpen: vi.fn(),
  formatFileSize: (b: number) => `${b} B`,
  getFileIcon: () => '📄',
  items: [clipboardItem],
  onCopy: vi.fn(),
  onPin: vi.fn(),
  onDelete: vi.fn(),
  onClearAll: vi.fn(),
};

describe('FilesScreen — one merged masonry (no tabs)', () => {
  it('interleaves transfers and clipboard items in a single multi-column flow', () => {
    const { container } = render(<FilesScreen {...filesProps} />);
    const flows = container.querySelectorAll('[class*="columns-"]');
    expect(flows).toHaveLength(1);
    const flow = flows[0];
    expect(flow.textContent).toContain('photo.png');
    expect(flow.textContent).toContain(clipboardItem.content);
    expect(container.querySelectorAll('[role="tab"], [role="tabpanel"]')).toHaveLength(0);
  });

  it('shows both per-source empty states inside the flow when lists are empty', () => {
    const { container } = render(
      <FilesScreen {...filesProps} transfers={[]} items={[]} />
    );
    const statuses = [...container.querySelectorAll('[role="status"]')].map(
      (el) => el.getAttribute('aria-label') ?? ''
    );
    expect(statuses.some((s) => s.startsWith('Your clipboard is empty'))).toBe(true);
    expect(statuses.some((s) => s.startsWith('No file transfers yet'))).toBe(true);
    const flows = container.querySelectorAll('[class*="columns-"]');
    expect(flows).toHaveLength(1);
    for (const status of container.querySelectorAll('[role="status"]')) {
      expect(status.closest('[class*="columns-"]')).toBe(flows[0]);
    }
  });
});
