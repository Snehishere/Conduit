/**
 * Regression tests for the local-capability handshake.
 *
 * Background: `ws_to_device_id` IS the identity store the hub's auth gate reads
 * (see apps/desktop/src-tauri/src/server/mod.rs). An earlier design
 * auto-registered every loopback connection into it, which made any process on
 * the machine authenticated for `automation/rule`, `automation/triggered`,
 * `screen_mirror` and `remote_input` — an unauthenticated RCE. That was removed
 * and replaced with a per-launch capability token the webview must present as
 * the first frame. See docs/decisions/0003-per-launch-local-capability-token.md.
 *
 * These tests pin the client half of that handshake. When the webview was not
 * updated, the socket still opened and ping/pong still worked, so the app LOOKED
 * connected while every privileged message was refused with `not_authenticated`.
 * That is a silent failure mode, which is exactly the kind a test has to catch.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, render, waitFor } from '@testing-library/react';
import { WebSocketProvider, useSharedWebSocket } from '../../hooks/useWebSocket';
import { invoke } from '@tauri-apps/api/core';

vi.mock('../../lib/toast', () => ({
  showError: vi.fn(),
  showSuccess: vi.fn(),
  showInfo: vi.fn(),
  showWarning: vi.fn(),
}));

const TOKEN = 'kQ8vZ2mNp4Xw7Rt1Lb6Yh3Jf0Dc5Sg9A2uE8iO';

/** Captured instances, so a test can reach the handler closures the hook assigns. */
const sockets: FakeSocket[] = [];

class FakeSocket {
  static readonly OPEN = 1;
  static readonly CONNECTING = 0;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readyState = 1;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  sent: string[] = [];
  closed = false;
  /** Order the hook assigned handlers in, so we can fire onopen realistically. */
  constructor() {
    sockets.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.closed = true;
    this.readyState = 3;
  }
  /** Parsed frames, in wire order. */
  frames(): Record<string, unknown>[] {
    return this.sent.map((s) => JSON.parse(s) as Record<string, unknown>);
  }
  open() {
    this.onopen?.();
  }
}

function mountProvider() {
  return render(
    <WebSocketProvider>
      <div data-testid="child" />
    </WebSocketProvider>
  );
}

beforeEach(() => {
  sockets.length = 0;
  vi.stubGlobal('WebSocket', FakeSocket as unknown as typeof WebSocket);
  vi.mocked(invoke).mockReset();
  // Only the capability command is stubbed; `get_notifications` must resolve to
  // an array or the hook's own error path fires and pollutes the assertions.
  vi.mocked(invoke).mockImplementation(async (cmd: string) => {
    if (cmd === 'get_local_ws_token') return TOKEN;
    if (cmd === 'get_notifications') return [];
    return undefined;
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('local capability handshake', () => {
  it('sends pairing/local_auth as the very first frame', async () => {
    mountProvider();
    await waitFor(() => expect(sockets).toHaveLength(1));
    act(() => { sockets[0].open(); });

    const frames = sockets[0].frames();
    expect(frames.length).toBeGreaterThan(0);
    // First on the wire, before the heartbeat or anything else. If this ever
    // moves, the hub refuses everything sent before it and the app goes quiet
    // rather than broken.
    expect(frames[0]).toEqual({
      type: 'pairing',
      action: 'local_auth',
      token: TOKEN,
    });
  });

  it('fetches the capability before the socket is created', async () => {
    mountProvider();
    await waitFor(() => expect(sockets).toHaveLength(1));
    // A socket existing at all implies the token command already resolved,
    // because `connect` awaits it before `new WebSocket(...)`. This is the
    // ordering guarantee that removes the pre-auth race window.
    expect(invoke).toHaveBeenCalledWith('get_local_ws_token');
    act(() => { sockets[0].open(); });
    expect(sockets[0].frames()[0]?.action).toBe('local_auth');
  });

  it('fetches the capability only once across reconnects', async () => {
    mountProvider();
    await waitFor(() => expect(sockets).toHaveLength(1));
    act(() => { sockets[0].open(); });

    // Simulate a drop; the hook schedules a reconnect with backoff.
    act(() => { sockets[0].onclose?.(); });

    await waitFor(() => expect(sockets.length).toBeGreaterThan(1), { timeout: 5000 });
    act(() => { sockets[1].open(); });

    // The capability is minted once per process launch, so re-invoking per
    // reconnect would be pure IPC churn.
    const tokenCalls = vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'get_local_ws_token');
    expect(tokenCalls).toHaveLength(1);
    expect(sockets[1].frames()[0]).toMatchObject({ action: 'local_auth', token: TOKEN });
  });

  it('still connects when no capability is available', async () => {
    // The Playwright e2e suite runs the React shell in a plain browser with no
    // Tauri IPC, so `get_local_ws_token` rejects. The socket must still open —
    // otherwise the browser-shell tests cannot run at all. It is simply
    // unpaired, which is why "connected" is not a proxy for "authenticated".
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_notifications') return [];
      throw new Error('no IPC outside Tauri');
    });

    mountProvider();
    await waitFor(() => expect(sockets).toHaveLength(1));
    act(() => { sockets[0].open(); });

    // No local_auth frame — and no fabricated one either.
    const authFrames = sockets[0].frames().filter((f) => f.action === 'local_auth');
    expect(authFrames).toHaveLength(0);
  });
});

describe('notification frame handling', () => {
  /** Renders the provider and exposes its context value for assertions. */
  async function mountAndObserve() {
    let observed: ReturnType<typeof useSharedWebSocket> | null = null;
    function Probe() {
      observed = useSharedWebSocket();
      return null;
    }
    render(
      <WebSocketProvider>
        <Probe />
      </WebSocketProvider>
    );
    await waitFor(() => expect(sockets).toHaveLength(1));
    act(() => { sockets[0].open(); });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('get_notifications', { limit: 50 }));
    return {
      socket: sockets[0],
      get notifications() {
        return observed?.notifications ?? [];
      },
    };
  }

  const POST = {
    type: 'notification',
    action: 'post',
    id: 'notif-1',
    device_id: 'dev-1',
    app: 'Messages',
    title: 'New message',
    body: 'Are you free tonight?',
    timestamp: 1_700_000_000,
  };

  it('adds a notification for a post frame', async () => {
    const obs = await mountAndObserve();
    expect(obs.notifications).toHaveLength(0);

    act(() => { obs.socket.onmessage?.({ data: JSON.stringify(POST) }); });

    expect(obs.notifications).toHaveLength(1);
    expect(obs.notifications[0]).toMatchObject({
      id: 'notif-1',
      app: 'Messages',
      title: 'New message',
    });
  });

  it('ignores its own dismiss frame instead of adding a blank notification', async () => {
    // Regression: the webview is itself a registered client of the local
    // server, so `dismiss_notification` broadcasts `notification/dismiss` and
    // the sender receives it back. Treating every `notification` frame as a
    // post inserted a blank notification with undefined app/title/body on
    // every dismissal.
    const obs = await mountAndObserve();
    act(() => { obs.socket.onmessage?.({ data: JSON.stringify(POST) }); });
    expect(obs.notifications).toHaveLength(1);

    act(() => {
      obs.socket.onmessage?.({
        data: JSON.stringify({
          type: 'notification',
          action: 'dismiss',
          id: 'notif-1',
          device_id: 'dev-1',
        }),
      });
    });

    // Still one — the dismiss frame must not have created a second entry, and
    // must have marked the existing one dismissed.
    expect(obs.notifications).toHaveLength(1);
    expect(obs.notifications[0]?.dismissed).toBe(true);
  });

  it('ignores a reply frame, which carries no notification to display', async () => {
    const obs = await mountAndObserve();
    act(() => { obs.socket.onmessage?.({ data: JSON.stringify(POST) }); });
    expect(obs.notifications).toHaveLength(1);

    act(() => {
      obs.socket.onmessage?.({
        data: JSON.stringify({
          type: 'notification',
          action: 'reply',
          id: 'notif-1',
          text: 'On my way',
        }),
      });
    });

    expect(obs.notifications).toHaveLength(1);
  });
});
