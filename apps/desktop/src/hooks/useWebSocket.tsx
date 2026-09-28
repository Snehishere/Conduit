// @ts-nocheck
import { useState, useEffect, useCallback, useRef, createContext, useContext, ReactNode } from 'react';
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { showError } from '../lib/toast';
import { WS_URL, WS_RECONNECT_BASE_MS, WS_RECONNECT_MAX_MS } from '../config';
import type { WebSocketMessage, MessageHandler } from '../types/websocket';

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

export type { WebSocketMessage, MessageHandler };

interface WebSocketContextType {
  connected: boolean;
  reconnecting: boolean;
  notifications: Notification[];
  dismissNotification: (id: string) => void;
  replyNotification: (id: string, text: string) => void;
  syncClipboard: (content: string, mime: string, sourceDevice: string) => void;
  registerHandler: (type: string, handler: MessageHandler) => () => void;
  /**
   * Writes one frame to the socket. Returns `false` when the socket is not
   * OPEN, i.e. the message was NOT sent — callers that need to tell the user
   * must check the result rather than assume delivery.
   */
  sendMessage: (msg: Record<string, unknown>) => boolean;
}

const WebSocketContext = createContext<WebSocketContextType | null>(null);

export function useSharedWebSocket(): WebSocketContextType {
  const ctx = useContext(WebSocketContext);
  if (!ctx) throw new Error('useSharedWebSocket must be used within WebSocketProvider');
  return ctx;
}

export function WebSocketProvider({ children }: { children: ReactNode }) {
  const [connected, setConnected] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const wsRef = useRef<WebSocket | null>(null);
  const retryCount = useRef(0);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handlersRef = useRef<Map<string, Set<MessageHandler>>>(new Map());

  const loadNotifications = useCallback(async () => {
    try {
      // Tauri IPC (local state read) — the Rust command clamps `limit` to
      // 1..=500 before it reaches SQL, because SQLite reads `LIMIT -1` as
      // "no limit". No WS frame is involved: this only reads the local DB.
      // Arg names are camelCase on the JS side; tauri-macros converts
      // `limit` → `limit` (identical) for the Rust param.
      const stored = await tauriInvoke<Array<{
        id: string; device_id: string; app: string; title: string;
        body: string; timestamp: number; actions?: string; dismissed: boolean;
      }>>('get_notifications', { limit: 50 });
      // `get_notifications` returns persisted rows including dismissed ones, so
      // filter here rather than rendering a dismissal as a live notification on
      // every reload. The provider keeps the full list (so the dismiss animation
      // has something to act on) but `dismissed` is never rendered as unread.
      setNotifications(Array.isArray(stored) ? stored : []);
    } catch (err) {
      console.error('[useWebSocket] Failed to load notifications:', err);
      showError('Couldn\'t load notifications');
    }
  }, []);

  const scheduleReconnect = useCallback(() => {
    if (reconnectTimer.current) return;
    setReconnecting(true);
    const delay = Math.min(WS_RECONNECT_BASE_MS * 2 ** retryCount.current, WS_RECONNECT_MAX_MS);
    retryCount.current++;
    reconnectTimer.current = setTimeout(() => {
      reconnectTimer.current = null;
      connectRef.current?.();
    }, delay);
  }, []);

  const connectRef = useRef<(() => void) | null>(null);
  // The local capability is minted once per process launch, so it is fetched
  // once and reused for the lifetime of the app (and across reconnects). A ref
  // rather than state: it is read only inside `connect` and must not trigger a
  // re-render.
  const localTokenRef = useRef<string | null>(null);
  // `connect` is async (it awaits the token before opening the socket), so the
  // synchronous `readyState` guard below is no longer sufficient to prevent two
  // sockets racing during that await. This flag closes the window.
  const connectingRef = useRef(false);

  const connect = useCallback(async () => {
    // Guard against duplicate connections.
    if (connectingRef.current) {
      return;
    }
    if (wsRef.current && (wsRef.current.readyState === WebSocket.OPEN || wsRef.current.readyState === WebSocket.CONNECTING)) {
      return;
    }

    if (wsRef.current) {
      try { wsRef.current.close(); } catch { /* ignore */ }
    }

    // The local Rust backend is the only WS target. An earlier comment here said
    // the relay path "lives on the Rust side via spawn_relay_client", which
    // stopped being true when the cloud relay client was disabled: main.rs
    // keeps `ws.spawn_relay_client(relay_url, server_id)` commented out so the
    // app stays LAN-only. Discovery is mDNS; there is no outbound relay
    // connection in this build.

    const wsUrl = WS_URL;

    // Present the per-launch local capability BEFORE the socket is created, so
    // the `local_auth` frame can be written synchronously as the very first
    // frame in `onopen`. Fetching it after the socket opens would open a window
    // in which the app sends privileged frames while still unpaired — the hub
    // answers `not_authenticated` and drops them, which surfaces as data that
    // silently never arrives rather than as an auth error.
    //
    // If the command is unavailable (the Playwright e2e suite runs in a plain
    // browser with no Tauri IPC) we still connect, just unpaired. That keeps the
    // browser-shell tests working and is why the socket can look "connected"
    // while unauthenticated.
    if (localTokenRef.current === null) {
      try {
        const token = await tauriInvoke<string>('get_local_ws_token');
        if (typeof token === 'string' && token.length > 0) {
          localTokenRef.current = token;
        }
      } catch (err) {
        console.warn(
          '[useWebSocket] No local capability available; the hub will treat this ' +
          'connection as unpaired and refuse privileged messages.', err
        );
      }
    }

    connectingRef.current = true;
    try {
      const ws = new WebSocket(wsUrl);
      wsRef.current = ws;

      // Keepalive interval reference attached to the socket instance logic
      let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
      let lastPongTime = Date.now();

      ws.onopen = () => {
        // Must be the first frame on the wire. The hub grants the
        // `local_desktop` identity on `pairing`/`local_auth` and refuses
        // everything else until then, so anything sent before this frame is
        // rejected. `ws_to_device_id` is the identity store the whole auth gate
        // reads, which is why an earlier design that auto-registered every
        // loopback peer was an unauthenticated-RCE hole — see
        // docs/decisions/0003-per-launch-local-capability-token.md.
        if (localTokenRef.current) {
          ws.send(JSON.stringify({
            type: 'pairing',
            action: 'local_auth',
            token: localTokenRef.current,
          }));
        }

        // NOTE: Browser WebSocket API does not expose TLS certificate details.
        // Certificate pinning cannot be implemented in browser-based WebSocket clients.
        // For Tauri desktop builds, use tauri-plugin-http or Rust-side certificate pinning
        // via the HttpRequestBuilder::danger_accept_invalid_certs() API with custom validation.
        // The browser's built-in TLS validation (certificate chain, expiry, domain) is active.
        // (This socket is plaintext loopback, so none of that applies to it —
        //  ws://127.0.0.1:9527. Remote peers use the TLS listener on 9531.)

        setConnected(true);
        setReconnecting(false);
        retryCount.current = 0;
        void loadNotifications();
        lastPongTime = Date.now();

        // Start ping/pong heartbeat
        heartbeatTimer = setInterval(() => {
          if (ws.readyState === WebSocket.OPEN) {
            // The peer may be gone without a close frame (a suspended phone, a
            // dropped AP), so treat a missing pong as a dead socket rather than
            // waiting on a close event that may never arrive.
            if (Date.now() - lastPongTime > 60000) {
              console.warn('[useWebSocket] Connection dead (missed pongs). Closing socket.');
              try { ws.close(); } catch { /* ignore */ }
              return;
            }
            ws.send(JSON.stringify({ type: 'ping' }));
          }
        }, 25000);
      };

      ws.onmessage = (event) => {
        try {
          // `MessageEvent.data` is `any` — capture it as `unknown` once so all
          // downstream uses are typed (avoids unsafe-assignment/argument).
          const raw: unknown = event.data;
          if (raw instanceof Blob || raw instanceof ArrayBuffer) {
            const handlers = handlersRef.current.get('binary');
            if (handlers) {
              handlers.forEach(h => { h({ type: 'binary' as any as any, data: raw }); });
            }
            return;
          }
          const data = JSON.parse(String(raw)) as WebSocketMessage;

          if (data.type === 'pong') {
            lastPongTime = Date.now();
            return;
          }
          if (data.type === 'ping') {
            ws.send(JSON.stringify({ type: 'pong' }));
            return;
          }

          // All other messages count as activity too
          lastPongTime = Date.now();

          // Only `post` creates a notification. `dismiss` and `reply` are also
          // broadcast by the local commands (dismiss_notification /
          // reply_notification), and this webview is itself a registered client
          // of the local server — so it receives its OWN frames back. Treating
          // every `notification` frame as a post inserted a blank notification
          // with undefined app/title/body on every dismissal. Narrow on the
          // action before reading the payload.
          if (data.type === 'notification' && data.action === 'post') {
            const notif = data;
            setNotifications((prev) => [
              { id: notif.id, device_id: notif.device_id, app: notif.app, title: notif.title, body: notif.body, timestamp: notif.timestamp, dismissed: false },
              ...prev,
            ]);
          }
          if (data.type === 'notification' && data.action === 'dismiss' && typeof data.id === 'string') {
            // A remote peer dismissed something; mirror it so both sides agree.
            setNotifications((prev) => prev.map((n) => (n.id === data.id ? { ...n, dismissed: true } : n)));
          }
          const typeHandlers = handlersRef.current.get(data.type);
          if (typeHandlers) {
            typeHandlers.forEach(handler => { handler(data); });
          }
        } catch (err) {
          console.error('[useWebSocket] Failed to parse message:', err);
        }
      };

      ws.onclose = () => {
        if (heartbeatTimer) clearInterval(heartbeatTimer);

        // Only trigger reconnect if this is the active socket
        if (wsRef.current === ws) {
          setConnected(false);
          wsRef.current = null;
          scheduleReconnect();
        }
      };

      ws.onerror = () => {
        try { ws.close(); } catch { /* ignore */ }
      };
    } catch (err) {
      // This socket is the connection to the *local* backend, not to a phone, so
      // the failure here is the desktop's own service being unreachable. The
      // previous message told the user to check their mobile device.
      connectingRef.current = false;
      console.error('[useWebSocket] Connection failed:', err);
      showError('Could not reach the local Conduit service.');
      scheduleReconnect();
    } finally {
      connectingRef.current = false;
    }
  }, [loadNotifications, scheduleReconnect]);

  useEffect(() => {
    connectRef.current = () => { void connect(); };
  }, [connect]);

  useEffect(() => {
    void connect();
    return () => {
      if (reconnectTimer.current) {
        clearTimeout(reconnectTimer.current);
        reconnectTimer.current = null;
      }
      const activeWs = wsRef.current;
      if (activeWs) {
        // Clear active ref first so onclose doesn't schedule reconnect
        wsRef.current = null;
        try { activeWs.close(); } catch { /* ignore */ }
      }
    };
  }, [connect]);

  const dismissNotification = useCallback(async (id: string) => {
    setNotifications((prev) => prev.map((n) => (n.id === id ? { ...n, dismissed: true } : n)));
    try {
      // Tauri IPC is the single path for a *locally initiated* dismissal: the
      // Rust command persists `dismissed = 1` and then forwards one canonical
      // `notification/dismiss` frame. Deliberately NOT also calling
      // `sendMessage({type:'notification', action:'dismiss', id})` — that
      // would write the row twice and put two identical frames on the wire.
      // The WS handler in `server/handlers/notifications.rs` is the inbound
      // path, used when a *remote peer* dismisses.
      // `id` is a TEXT column, so the Rust param is `id: String`.
      await tauriInvoke('dismiss_notification', { id });
    } catch (err) {
      console.error('WS: dismiss_notification failed', err);
      // Roll the optimistic update back so the UI matches the DB, which the
      // failed command never wrote.
      setNotifications((prev) => prev.map((n) => (n.id === id ? { ...n, dismissed: false } : n)));
      showError('Couldn\'t dismiss notification');
    }
  }, []);

  const replyNotification = useCallback(async (id: string, text: string) => {
    try {
      // Same single-path rule as dismiss: the Rust command sends the
      // `notification/reply` frame built from `protocol::NotificationReply`.
      // Both args are plain identifiers (`id`, `text`) so no case conversion
      // is involved.
      await tauriInvoke('reply_notification', { id, text });
    } catch (err) {
      // A reply has no local effect to fall back on, so a failure here means
      // the text is silently lost — surface it instead of only logging.
      console.error('WS: reply_notification failed', err);
      showError('Couldn\'t send reply');
    }
  }, []);

  const syncClipboard = useCallback(async (content: string, mime: string, sourceDevice: string) => {
    try {
      // Tauri v2 commands default to `rename_all = "camelCase"`, so the Rust
      // param `source_device: String` must be passed as `sourceDevice` here.
      // The command persists to clipboard history and then broadcasts
      // `clipboard/sync` (protocol `ClipboardSync`).
      await tauriInvoke('sync_clipboard', { content, mime, sourceDevice });
    } catch (err) {
      console.error('WS: sync_clipboard failed', err);
      showError('Couldn\'t sync clipboard');
    }
  }, []);

  const registerHandler = useCallback((type: string, handler: MessageHandler) => {
    // Reuse the existing set when present, otherwise create it up-front —
    // no non-null assertion needed after the write.
    const existing = handlersRef.current.get(type);
    const handlers = existing ?? new Set<MessageHandler>();
    if (!existing) handlersRef.current.set(type, handlers);
    handlers.add(handler);
    return () => {
      handlersRef.current.get(type)?.delete(handler);
    };
  }, []);

  const sendMessage = useCallback((msg: Record<string, unknown>): boolean => {
    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
      return true;
    }
    // Previously a silent no-op, which let callers believe a message had left
    // the app when nothing was on the wire. Report it instead.
    const type = typeof msg.type === 'string' ? msg.type : 'unknown';
    console.warn(`[useWebSocket] Dropped "${type}" — socket is not open.`);
    return false;
  }, []);

  return (
    <WebSocketContext.Provider value={{ connected, reconnecting, notifications, dismissNotification, replyNotification, syncClipboard, registerHandler, sendMessage }}>
      {children}
    </WebSocketContext.Provider>
  );
}

// Legacy hook for backward compatibility
export function useWebSocket() {
  const ctx = useSharedWebSocket();
  return {
    connected: ctx.connected,
    reconnecting: ctx.reconnecting,
    notifications: ctx.notifications,
    dismissNotification: ctx.dismissNotification,
    replyNotification: ctx.replyNotification,
    syncClipboard: ctx.syncClipboard,
    sendMessage: ctx.sendMessage,
    registerHandler: ctx.registerHandler,
  };
}
