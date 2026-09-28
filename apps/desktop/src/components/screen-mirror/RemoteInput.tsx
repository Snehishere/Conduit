import React, { useState, useEffect, useRef, useCallback } from 'react';
import { useSharedWebSocket } from '../../hooks/useWebSocket';
import { MousePointer2, Square } from 'lucide-react';

interface RemoteInputProps {
  deviceId: string;
  deviceName: string;
  onStop: () => void;
}

/** Buttons the protocol's `RemoteInputClick.button` enum accepts. */
type RemoteButton = 'left' | 'right' | 'double_left' | 'middle';

/** Modifiers the protocol's `modifiers` enum accepts — note `control`, not `ctrl`. */
type Modifier = 'shift' | 'control' | 'alt' | 'meta';

/** Two clicks within this window (ms) are a double click. */
const DOUBLE_CLICK_MS = 300;

const clamp = (value: number, limit: number) => Math.min(limit, Math.max(-limit, value));

/** DOM wheel delta (in pixels, lines or pages) to scroll steps. */
function wheelSteps(deltaX: number, deltaY: number, deltaMode: number): { dx: number; dy: number } {
  switch (deltaMode) {
    case 1: // DOM_DELTA_LINE
      return { dx: deltaX, dy: deltaY };
    case 2: // DOM_DELTA_PAGE
      return { dx: deltaX * 10, dy: deltaY * 10 };
    default: // DOM_DELTA_PIXEL — one notch is ~100px
      return { dx: deltaX / 100, dy: deltaY / 100 };
  }
}

export default function RemoteInput({ deviceId, deviceName, onStop }: RemoteInputProps) {
  const [lastEvent, setLastEvent] = useState<string>('');
  const containerRef = useRef<HTMLDivElement>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const lastClickRef = useRef<{ button: RemoteButton; at: number } | null>(null);
  const { sendMessage, connected } = useSharedWebSocket();

  // No `start` / `stop` frames: the protocol has no remote_input session
  // message, and the receiver injects each frame as it arrives. Sending them
  // only produced "Unknown remote_input action" warnings.
  const send = useCallback(
    (message: Record<string, unknown>) => {
      if (!connected) return;
      sendMessage({ type: 'remote_input', device_id: deviceId, ...message });
    },
    [connected, deviceId, sendMessage]
  );

  // Keyboard events only reach the handler while the pad has focus, and a
  // remote-control pad that swallows every key press is useless until clicked.
  useEffect(() => {
    containerRef.current?.focus();
  }, []);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (!containerRef.current) return;

      const previous = lastPointerRef.current;
      lastPointerRef.current = { x: e.clientX, y: e.clientY };
      // The wire format is a *relative* move (dx/dy), so the first pointer
      // position only seeds the origin — sending it would jump the remote
      // cursor to a random spot.
      if (!previous) return;

      const dx = Math.round(clamp(e.clientX - previous.x, 4096));
      const dy = Math.round(clamp(e.clientY - previous.y, 4096));
      if (dx === 0 && dy === 0) return;

      send({ action: 'move', dx, dy });
      setLastEvent(`Move: ${dx >= 0 ? '+' : ''}${dx}, ${dy >= 0 ? '+' : ''}${dy}`);
    },
    [send]
  );

  const handleMouseLeave = useCallback(() => {
    // Re-entering the pad must not produce one huge delta.
    lastPointerRef.current = null;
  }, []);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      const container = containerRef.current;
      if (!container) return;
      container.focus();

      let button: RemoteButton = 'left';
      if (e.button === 1) button = 'middle';
      if (e.button === 2) button = 'right';

      const now = Date.now();
      const previous = lastClickRef.current;
      lastClickRef.current = { button, at: now };
      if (previous && previous.button === button && now - previous.at < DOUBLE_CLICK_MS) {
        // The first click of a double click already went out; the pair is
        // reported as `double_left` so the receiver is not asked to click
        // three times.
        lastClickRef.current = null;
        send({ action: 'click', button: 'double_left' });
        setLastEvent('Click: double');
        return;
      }

      send({ action: 'click', button });
      setLastEvent(`Click: ${button}`);
    },
    [send]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const modifiers: Modifier[] = [];
      if (e.ctrlKey) modifiers.push('control');
      if (e.shiftKey) modifiers.push('shift');
      if (e.altKey) modifiers.push('alt');
      if (e.metaKey) modifiers.push('meta');

      send({ action: 'key', key: e.key, modifiers });
      setLastEvent(`Key: ${modifiers.length ? `${modifiers.join('+')}+` : ''}${e.key}`);
      // The pad is a remote keyboard: the key press belongs to the remote
      // machine, not to the browser.
      e.preventDefault();
    },
    [send]
  );

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      const { dx, dy } = wheelSteps(e.deltaX, e.deltaY, e.deltaMode);
      if (dx === 0 && dy === 0) return;
      send({ action: 'scroll', dx, dy });
      setLastEvent(`Scroll: ${dy > 0 ? 'down' : dy < 0 ? 'up' : 'sideways'}`);
    },
    [send]
  );

  return (
    <div className="flex flex-col h-full animate-fade-in">
      {/* Header */}
      <div
        className="surf-glass flex items-center justify-between px-4 py-3 shrink-0"
        style={{
          borderTop: 'none',
          borderLeft: 'none',
          borderRight: 'none',
          borderBottom: '1px solid var(--line-1)',
        }}
      >
        <div className="flex items-center gap-3">
          <div
            className="w-8 h-8 rounded-lg flex items-center justify-center shrink-0"
            style={{ background: 'var(--accent-dim)' }}
          >
            <MousePointer2 size={16} style={{ color: 'var(--accent)' }} />
          </div>
          <div>
            <h3
              className="text-[14px] font-semibold leading-tight"
              style={{ color: 'var(--text-1)', fontFamily: 'var(--font-display)' }}
            >
              Remote Input
            </h3>
            <span className="text-[11px]" style={{ color: 'var(--text-3)' }}>
              {deviceName}
            </span>
          </div>
        </div>

        <div className="flex items-center gap-3">
          {/* Status indicator */}
          <span
            className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-medium"
            style={{
              background: connected ? 'var(--success-dim)' : 'var(--bg-3)',
              color: connected ? 'var(--success)' : 'var(--text-3)',
            }}
          >
            <span
              className={`w-1.5 h-1.5 rounded-full shrink-0 ${connected ? 'animate-pulse-dot' : ''}`}
              style={{ background: connected ? 'var(--success)' : 'var(--text-3)' }}
            />
            {connected ? 'Connected' : 'Disconnected'}
          </span>

          <button
            onClick={onStop}
            className="btn-press flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] font-medium transition-colors border border-[rgba(239,68,68,0.25)] bg-danger/15 text-danger hover:bg-[#DC2626] hover:text-white"
          >
            <Square size={10} fill="currentColor" />
            Stop
          </button>
        </div>
      </div>

      {/* Trackpad / Touch Area */}
      <div
        ref={containerRef}
        className="surf-glass flex-1 relative flex items-center justify-center overflow-hidden cursor-crosshair"
        onMouseMove={handleMouseMove}
        onMouseDown={handleMouseDown}
        onMouseLeave={handleMouseLeave}
        onKeyDown={handleKeyDown}
        onWheel={handleWheel}
        onContextMenu={(e) => e.preventDefault()}
        tabIndex={0}
        role="application"
        aria-label={`Remote control trackpad for ${deviceName}. Move the pointer to move the remote cursor, click to click, and type to send keys.`}
        style={{
          border: 'none',
          outline: 'none',
        }}
      >
        <div className="flex flex-col items-center gap-3 pointer-events-none select-none">
          <div
            className="w-14 h-14 rounded-2xl flex items-center justify-center"
            style={{ background: 'var(--bg-3)' }}
          >
            <MousePointer2 size={24} style={{ color: 'var(--text-3)' }} />
          </div>
          <p className="text-[13px]" style={{ color: 'var(--text-3)' }}>
            Move and click to control {deviceName}
          </p>
          <p className="text-[11px]" style={{ color: 'var(--text-3)' }}>
            Mouse, keyboard, and scroll events are forwarded
          </p>
        </div>
      </div>

      {/* Status Bar */}
      {lastEvent && (
        <div
          className="surf-glass flex items-center gap-2 px-4 py-2 shrink-0"
          style={{
            borderBottom: 'none',
            borderLeft: 'none',
            borderRight: 'none',
            borderTop: '1px solid var(--line-1)',
          }}
        >
          <span
            className="w-1.5 h-1.5 rounded-full shrink-0"
            style={{ background: 'var(--accent)' }}
          />
          <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-3)' }}>
            Last event: <span style={{ color: 'var(--text-2)' }}>{lastEvent}</span>
          </span>
        </div>
      )}
    </div>
  );
}
