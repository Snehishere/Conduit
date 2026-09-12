import { useState, useCallback } from 'react';

export interface SmsMessage {
  id: string;
  address: string;
  body: string;
  timestamp: number;
  read: boolean;
  is_outgoing: boolean;
}

export interface SmsThread {
  thread_id: string;
  address: string;
  name: string | null;
  snippet: string;
  unread_count: number;
  timestamp: number;
  messages: SmsMessage[];
}

function timeAgo(timestamp: number): string {
  const minutes = Math.floor((Date.now() / 1000 - timestamp) / 60);
  if (minutes < 1) return 'now';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

export function useSms(sendMessage?: (msg: Record<string, unknown>) => void) {
  const [threads, setThreads] = useState<SmsThread[]>([]);
  const [selectedThread, setSelectedThread] = useState<string | null>(null);

  const handleSmsMessage = useCallback((data: Record<string, unknown>) => {
    const action = data.action as string;

    switch (action) {
      case 'sync': {
        const threadList = data.threads as SmsThread[];
        setThreads(threadList.sort((a, b) => b.timestamp - a.timestamp));
        break;
      }
      case 'new': {
        const threadId = data.thread_id as string;
        const message = data.message as SmsMessage;
        setThreads((prev) => {
          const existing = prev.find((t) => t.thread_id === threadId);
          if (existing) {
            return prev.map((t) =>
              t.thread_id === threadId
                ? {
                    ...t,
                    messages: [...t.messages, message],
                    snippet: message.body,
                    timestamp: message.timestamp,
                    unread_count: message.read ? t.unread_count : t.unread_count + 1,
                  }
                : t
            ).sort((a, b) => b.timestamp - a.timestamp);
          }
          // Create a new thread instead of dropping the message.
          const newThread: SmsThread = {
            thread_id: threadId,
            address: (data.address as string) || message.address,
            name: (data.name as string) || null,
            snippet: message.body,
            unread_count: message.read ? 0 : 1,
            timestamp: message.timestamp,
            messages: [message],
          };
          return [...prev, newThread].sort((a, b) => b.timestamp - a.timestamp);
        });
        break;
      }
      case 'sent': {
        const threadId = data.thread_id as string;
        const message = data.message as SmsMessage;
        setThreads((prev) =>
          prev.map((t) =>
            t.thread_id === threadId
              ? {
                  ...t,
                  messages: [...t.messages, message],
                  snippet: message.body,
                  timestamp: message.timestamp,
                }
              : t
          ).sort((a, b) => b.timestamp - a.timestamp)
        );
        break;
      }
    }
  }, []);

  const sendMessageFn = useCallback((to: string, body: string) => {
    sendMessage?.({ type: 'sms', action: 'send', to, body });
  }, [sendMessage]);

  const markRead = useCallback((threadId: string) => {
    setThreads((prev) =>
      prev.map((t) =>
        t.thread_id === threadId ? { ...t, unread_count: 0 } : t
      )
    );
  }, []);

  const getSelectedThread = useCallback(() => {
    return threads.find((t) => t.thread_id === selectedThread) || null;
  }, [threads, selectedThread]);

  const totalUnread = threads.reduce((sum, t) => sum + t.unread_count, 0);

  return {
    threads,
    selectedThread,
    setSelectedThread,
    handleSmsMessage,
    sendMessage: sendMessageFn,
    markRead,
    getSelectedThread,
    totalUnread,
    timeAgo,
  };
}
