import { useState, useCallback } from 'react';
import { showError } from '../lib/toast';
import { formatRelativeTime } from '../lib/time';
import type { WebSocketMessage } from '../types/websocket';
import type { SendResult } from './useEncryption';

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

/**
 * @param sendMessage   Plain WebSocket send.
 * @param sendSensitive Encrypted send path, consulted for every outbound SMS.
 */
export function useSms(
  sendMessage?: (msg: Record<string, unknown>) => void,
  sendSensitive?: (
    targetDeviceId: string | null | undefined,
    msg: Record<string, unknown>,
  ) => Promise<SendResult>,
) {
  const [threads, setThreads] = useState<SmsThread[]>([]);
  const [selectedThread, setSelectedThread] = useState<string | null>(null);

  const handleSmsMessage = useCallback((data: WebSocketMessage) => {
    if (data.type !== 'sms') return;

    switch (data.action) {
      case 'sync' as any: {
        const threadList = (data as any).threads;
        setThreads(threadList.sort((a: any, b: any) => b.timestamp - a.timestamp));
        break;
      }
      case 'new' as any: {
        const threadId = (data as any).thread_id;
        const message = (data as any).message;
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
            address: (data as any).address || message.address,
            name: (data as any).name || null,
            snippet: message.body,
            unread_count: message.read ? 0 : 1,
            timestamp: message.timestamp,
            messages: [message],
          };
          return [...prev, newThread].sort((a, b) => b.timestamp - a.timestamp);
        });
        break;
      }
      case 'sent' as any: {
        const threadId = (data as any).thread_id;
        const message = (data as any).message;
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

  /**
   * `sms/send` has no destination device: the server broadcasts it to every
   * other peer (`server/mod.rs:644-646`) and the recipient is a phone number,
   * not a device id. There is therefore no single shared secret to pick, so
   * this message type can never be wrapped — it is always sent in the clear.
   *
   * It is still routed through `sendSensitive` with a null target so the
   * status bar records the fact from a real send rather than from a hard-coded
   * table, and so a future protocol that carries a device id needs no change
   * here.
   */
  const sendMessageFn = useCallback(async (to: string, body: string) => {
    const msg = { type: 'sms', action: 'send', to, body };

    if (sendSensitive) {
      const result = await sendSensitive(null, msg);
      if (result.encrypted) return;
    }

    if (!sendMessage) {
      showError('Cannot send SMS: not connected');
      return;
    }
    sendMessage(msg);
  }, [sendMessage, sendSensitive]);

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
    timeAgo: formatRelativeTime,
  };
}
