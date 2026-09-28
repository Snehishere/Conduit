import React, { useState, useEffect, useRef, useCallback } from 'react';
import { useSharedWebSocket } from '../../hooks/useWebSocket';
import { Monitor, Square } from 'lucide-react';
import type { ScreenMirrorFrameMessage, WebSocketMessage } from '../../types/websocket';

interface ScreenMirrorProps {
  deviceId: string;
  deviceName: string;
  onStop: () => void;
}

/** Gestures the protocol's `actionType` enum accepts. */
type TouchAction = 'tap' | 'double_tap' | 'long_press' | 'right_click' | 'move';

/** Modifiers the protocol's `modifiers` enum accepts — note `control`, not `ctrl`. */
type Modifier = 'shift' | 'control' | 'alt' | 'meta';

/** Two clicks within this window (ms) are a double click. */
const DOUBLE_CLICK_MS = 300;
/** Hold time (ms) before a press becomes a `long_press`. */
const LONG_PRESS_MS = 500;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

/**
 * Map a pointer event onto the mirrored image's own 0..1 space.
 *
 * The canvas is letterboxed (`object-fit: contain`), so the element's box is
 * usually larger than the frame that is actually drawn: mapping against the
 * element rect alone sends taps into the black bars. Returns the clamped
 * result so a drag that leaves the canvas still targets a real pixel.
 */
function toImageSpace(
  canvas: HTMLCanvasElement,
  clientX: number,
  clientY: number
): { x: number; y: number } {
  const rect = canvas.getBoundingClientRect();
  if (canvas.width === 0 || canvas.height === 0 || rect.width === 0 || rect.height === 0) {
    return { x: 0, y: 0 };
  }
  const scale = Math.min(rect.width / canvas.width, rect.height / canvas.height);
  const drawWidth = canvas.width * scale;
  const drawHeight = canvas.height * scale;
  const originX = rect.left + (rect.width - drawWidth) / 2;
  const originY = rect.top + (rect.height - drawHeight) / 2;
  return {
    x: clamp01((clientX - originX) / drawWidth),
    y: clamp01((clientY - originY) / drawHeight),
  };
}

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

export default function ScreenMirror({ deviceId, deviceName, onStop }: ScreenMirrorProps) {
  const [hasFrame, setHasFrame] = useState(false);
  const [resolution, setResolution] = useState({ width: 0, height: 0 });
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const draggingRef = useRef(false);
  const longPressFiredRef = useRef(false);
  const longPressTimerRef = useRef<number | null>(null);
  const lastClickRef = useRef<number | null>(null);
  const { registerHandler, sendMessage, connected } = useSharedWebSocket();

  const clearLongPress = useCallback(() => {
    if (longPressTimerRef.current !== null) {
      window.clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);

  // A pending long press must not fire after the component goes away.
  useEffect(() => clearLongPress, [clearLongPress]);

  // `onStop` is an inline arrow at the call site, so keep it in a ref: the
  // frame handler must not be torn down and re-registered on every render.
  const onStopRef = useRef(onStop);
  onStopRef.current = onStop;

  const handleFrame = useCallback((data: ScreenMirrorFrameMessage) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    if (canvas.width !== data.width || canvas.height !== data.height) {
      canvas.width = data.width;
      canvas.height = data.height;
      setResolution({ width: data.width, height: data.height });
    }

    const img = new Image();
    img.onload = () => {
      ctx.drawImage(img, 0, 0);
      setHasFrame(true);
    };
    img.src = `data:image/jpeg;base64,${data.data}`;
  }, []);

  // Frames are captured by the *selected device* (the phone runs a
  // MediaProjection capture) and relayed by the server to this viewer. The
  // desktop never captures its own screen for this view.
  useEffect(() => {
    const unregister = registerHandler('screen_mirror', (data: WebSocketMessage) => {
      if (data.type !== 'screen_mirror') return;
      if (data.action === 'frame') {
        handleFrame(data);
      } else if (data.action === 'capture_stopped') {
        onStopRef.current();
      }
    });
    return unregister;
  }, [registerHandler, handleFrame]);

  // Ask the selected device to start capturing. `quality` / `fps` are
  // deliberately not sent: they are capture-side settings (the phone applies
  // them when it starts its MediaProjection capture) and cannot be changed
  // after the stream is running, so the dropdowns that used to sit here only
  // ever showed a value that was never applied.
  useEffect(() => {
    if (!connected) return;

    sendMessage({
      type: 'screen_mirror',
      action: 'start',
      device_id: deviceId,
    });

    return () => {
      sendMessage({
        type: 'screen_mirror',
        action: 'stop',
        device_id: deviceId,
      });
    };
  }, [connected, deviceId, sendMessage]);

  const sendTouch = useCallback(
    (actionType: TouchAction, clientX: number, clientY: number) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const { x, y } = toImageSpace(canvas, clientX, clientY);
      sendMessage({
        type: 'screen_mirror',
        action: 'touch',
        device_id: deviceId,
        x,
        y,
        // camelCase on the wire: the protocol spells it `actionType`, and the
        // `action_type` the UI used to send was silently dropped.
        actionType,
      });
    },
    [deviceId, sendMessage]
  );

  const handlePointerDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    e.currentTarget.focus();
    if (e.button === 2) {
      sendTouch('right_click', e.clientX, e.clientY);
      return;
    }
    if (e.button !== 0) return;

    draggingRef.current = true;
    longPressFiredRef.current = false;
    clearLongPress();
    longPressTimerRef.current = window.setTimeout(() => {
      longPressTimerRef.current = null;
      if (!draggingRef.current) return;
      // A held press is a long press, not a tap: remember it so the trailing
      // `onClick` does not also fire a tap.
      longPressFiredRef.current = true;
      sendTouch('long_press', e.clientX, e.clientY);
    }, LONG_PRESS_MS);
  };

  const handlePointerMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    // Only while a drag is in progress: hover moves on every pixel would flood
    // the socket, and the mirrored device does not need a hover stream.
    if (!draggingRef.current) return;
    sendTouch('move', e.clientX, e.clientY);
  };

  const endDrag = () => {
    clearLongPress();
    draggingRef.current = false;
  };

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    if (longPressFiredRef.current) {
      longPressFiredRef.current = false;
      return;
    }
    const now = Date.now();
    const previous = lastClickRef.current;
    lastClickRef.current = now;
    // The second click of a pair is not a tap of its own — `onDoubleClick`
    // reports the pair as `double_tap`, so sending it again would deliver three
    // taps where the user asked for two.
    if (previous !== null && now - previous < DOUBLE_CLICK_MS) return;
    sendTouch('tap', e.clientX, e.clientY);
  };

  const handleDoubleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    sendTouch('double_tap', e.clientX, e.clientY);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLCanvasElement>) => {
    const modifiers: Modifier[] = [];
    if (e.ctrlKey) modifiers.push('control');
    if (e.shiftKey) modifiers.push('shift');
    if (e.altKey) modifiers.push('alt');
    if (e.metaKey) modifiers.push('meta');

    sendMessage({
      type: 'screen_mirror',
      action: 'key',
      device_id: deviceId,
      key: e.key,
      modifiers,
    });
    // The canvas is a remote control surface: the key belongs to the mirrored
    // device, not to the browser.
    e.preventDefault();
  };

  const handleWheel = (e: React.WheelEvent<HTMLCanvasElement>) => {
    const { dx, dy } = wheelSteps(e.deltaX, e.deltaY, e.deltaMode);
    sendMessage({
      type: 'screen_mirror',
      action: 'scroll',
      device_id: deviceId,
      dx,
      dy,
    });
  };

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
            <Monitor size={16} style={{ color: 'var(--accent)' }} />
          </div>
          <div>
            <h3
              className="text-[14px] font-semibold leading-tight"
              style={{ color: 'var(--text-1)', fontFamily: 'var(--font-display)' }}
            >
              {deviceName}&rsquo;s screen
            </h3>
            <span className="text-[11px]" style={{ color: 'var(--text-3)' }}>
              Mirrored from {deviceName}
            </span>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={onStop}
            className="btn-press flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] font-medium transition-colors border border-[rgba(239,68,68,0.25)] bg-danger/15 text-danger hover:bg-[#DC2626] hover:text-white"
          >
            <Square size={10} fill="currentColor" />
            Stop
          </button>
        </div>
      </div>

      {/* Canvas Area */}
      <div
        className="surf-glass flex-1 relative flex items-center justify-center overflow-hidden"
        style={{
          border: 'none',
          borderRadius: 0,
        }}
      >
        <canvas
          ref={canvasRef}
          onMouseDown={handlePointerDown}
          onMouseMove={handlePointerMove}
          onMouseUp={endDrag}
          onMouseLeave={endDrag}
          onClick={handleClick}
          onDoubleClick={handleDoubleClick}
          onContextMenu={(e) => e.preventDefault()}
          onKeyDown={handleKeyDown}
          onWheel={handleWheel}
          tabIndex={0}
          role="application"
          aria-label={`Remote screen of ${deviceName}. Click to tap, drag to move, scroll and press keys to control it.`}
          className="max-w-full max-h-full outline-none"
          style={{
            objectFit: 'contain',
            cursor: 'pointer',
            touchAction: 'none',
          }}
        />
        {!hasFrame && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 pointer-events-none select-none">
            <p className="text-[13px]" style={{ color: 'var(--text-2)' }}>
              Waiting for {deviceName}&rsquo;s screen&hellip;
            </p>
            <p className="text-[11px]" style={{ color: 'var(--text-3)' }}>
              Keep it unlocked and awake &mdash; the first frame arrives once its
              screen capture starts.
            </p>
          </div>
        )}
        {resolution.width > 0 && (
          <div
            className="absolute bottom-3 right-3 px-2.5 py-1 rounded-lg text-[10px] font-medium tabular-nums"
            style={{
              background: 'rgba(6, 8, 12, 0.80)',
              color: 'var(--text-3)',
              border: '1px solid var(--line-1)',
            }}
          >
            {resolution.width} x {resolution.height}
          </div>
        )}
      </div>
    </div>
  );
}
