import { useState, useEffect, useCallback, useRef, createContext, useContext, ReactNode } from 'react';

interface Notification {
  id: string;
  device_id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  actions?: string;
  dismissed: boolean;
}

type MessageHandler = (data: any) => void;

interface WebSocketContextType {
  connected: boolean;
  notifications: Notification[];
  dismissNotification: (id: string) => void;
  replyNotification: (id: string, text: string) => void;
  syncClipboard: (content: string, mime: string, sourceDevice: string) => void;
  registerHandler: (type: string, handler: MessageHandler) => () => void;
  sendMessage: (msg: Record<string, unknown>) => void;
}

const WebSocketContext = createContext<WebSocketContextType | null>(null);

export function useSharedWebSocket(): WebSocketContextType {
  const ctx = useContext(WebSocketContext);
  if (!ctx) throw new Error('useSharedWebSocket must be used within WebSocketProvider');
  return ctx;
}

export function WebSocketProvider({ children }: { children: ReactNode }) {
  const [connected, setConnected] = useState(false);
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const wsRef = useRef<WebSocket | null>(null);
  const retryCount = useRef(0);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handlersRef = useRef<Map<string, Set<MessageHandler>>>(new Map());

  const loadNotifications = useCallback(async () => {
    try {
      const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
      const stored = await tauriInvoke<Array<{
        id: string; device_id: string; app: string; title: string;
        body: string; timestamp: number; actions?: string; dismissed: boolean;
      }>>('get_notifications', { limit: 50 });
      setNotifications(stored);
    } catch (err) {
      console.error('Failed to load notifications:', err);
    }
  }, []);

  const scheduleReconnect = useCallback(() => {
    if (reconnectTimer.current) return;
    const delay = Math.min(1000 * 2 ** retryCount.current, 30000);
    retryCount.current++;
    reconnectTimer.current = setTimeout(() => {
      reconnectTimer.current = null;
      connectRef.current?.();
    }, delay);
  }, []);

  const connectRef = useRef<(() => void) | null>(null);

  const connect = useCallback(() => {
    // Guard against duplicate connections.
    if (wsRef.current && (wsRef.current.readyState === WebSocket.OPEN || wsRef.current.readyState === WebSocket.CONNECTING)) {
      return;
    }
    try {
      wsRef.current?.close();
    } catch { /* ignore */ }
    try {
      const ws = new WebSocket('ws://localhost:9527');
      wsRef.current = ws;

      ws.onopen = () => {
        setConnected(true);
        retryCount.current = 0;
        loadNotifications();
      };

      ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          if (data.type === 'notification' && data.action === 'post') {
            setNotifications((prev) => [
              { id: data.id, device_id: data.device_id, app: data.app, title: data.title, body: data.body, timestamp: data.timestamp, dismissed: false },
              ...prev,
            ]);
          }
          const typeHandlers = handlersRef.current.get(data.type);
          if (typeHandlers) {
            typeHandlers.forEach(handler => handler(data));
          }
        } catch (err) {
          console.error('WS: failed to parse message', err);
        }
      };

      ws.onclose = () => {
        setConnected(false);
        if (wsRef.current === ws) {
          wsRef.current = null;
        }
        scheduleReconnect();
      };

      ws.onerror = () => {
        try { ws.close(); } catch { /* ignore */ }
      };
    } catch (err) {
      console.error('WS: connection failed', err);
      scheduleReconnect();
    }
  }, [loadNotifications, scheduleReconnect]);

  useEffect(() => {
    connectRef.current = connect;
  }, [connect]);

  useEffect(() => {
    connect();
    return () => {
      if (reconnectTimer.current) {
        clearTimeout(reconnectTimer.current);
        reconnectTimer.current = null;
      }
      try { wsRef.current?.close(); } catch { /* ignore */ }
      wsRef.current = null;
    };
  }, [connect]);

  const dismissNotification = useCallback(async (id: string) => {
    setNotifications((prev) => prev.map((n) => (n.id === id ? { ...n, dismissed: true } : n)));
    try {
      const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
      await tauriInvoke('dismiss_notification', { id });
    } catch (err) {
      console.error('WS: dismiss_notification failed', err);
    }
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({ type: 'notification', action: 'dismiss', id }));
    }
  }, []);

  const replyNotification = useCallback((id: string, text: string) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({ type: 'notification', action: 'reply', id, text }));
    }
  }, []);

  const syncClipboard = useCallback(async (content: string, mime: string, sourceDevice: string) => {
    try {
      const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
      await tauriInvoke('sync_clipboard', { content, mime, sourceDevice });
    } catch (err) {
      console.error('WS: sync_clipboard failed', err);
    }
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({
        type: 'clipboard', action: 'sync', content, mime, source_device: sourceDevice,
        timestamp: Math.floor(Date.now() / 1000),
      }));
    }
  }, []);

  const registerHandler = useCallback((type: string, handler: MessageHandler) => {
    if (!handlersRef.current.has(type)) {
      handlersRef.current.set(type, new Set());
    }
    handlersRef.current.get(type)!.add(handler);
    return () => {
      handlersRef.current.get(type)?.delete(handler);
    };
  }, []);

  const sendMessage = useCallback((msg: Record<string, unknown>) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    }
  }, []);

  return (
    <WebSocketContext.Provider value={{ connected, notifications, dismissNotification, replyNotification, syncClipboard, registerHandler, sendMessage }}>
      {children}
    </WebSocketContext.Provider>
  );
}

// Legacy hook for backward compatibility
export function useWebSocket() {
  const ctx = useSharedWebSocket();
  return {
    connected: ctx.connected,
    notifications: ctx.notifications,
    dismissNotification: ctx.dismissNotification,
    replyNotification: ctx.replyNotification,
    syncClipboard: ctx.syncClipboard,
    sendMessage: ctx.sendMessage,
    registerHandler: ctx.registerHandler,
  };
}
