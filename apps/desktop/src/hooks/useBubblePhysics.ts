import { useRef, useCallback, useEffect, useState } from 'react';

export interface BubbleData {
  id: string;
  name: string;
  deviceType: string;
  status: string;
  battery?: number;
  os: string;
}

export interface BubbleState {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  radius: number;
  mass: number;
  scale: number;
  opacity: number;
  popping: boolean;
  data: BubbleData;
}

const DEVICE_SIZES: Record<string, number> = {
  desktop: 180,
  laptop: 180,
  phone: 160,
  tablet: 150,
  tv: 150,
  watch: 120,
  earbuds: 120,
  headphones: 120,
};
const DEFAULT_SIZE = 140;
const MARGIN = 24;
// Per-frame velocity damping — a thrown bubble bleeds energy and comes to rest
// in a few seconds (0.996 made it creep across the hub for ~25s before the
// rest threshold could trigger).
const DAMPING = 0.99;
// Throw speed ceiling (px per 60fps frame) — fast flicks travel the hub and
// bounce off the walls instead of crawling.
const MAX_SPEED = 4;
// Below this speed a bubble is considered at rest and snaps to a full stop,
// so untouched bubbles are literally motionless.
const REST_SPEED = 0.05;

function getRadius(deviceType: string): number {
  return (DEVICE_SIZES[deviceType] || DEFAULT_SIZE) / 2;
}

function getMass(radius: number): number {
  return Math.PI * radius * radius;
}

function findOpenPosition(
  existing: BubbleState[],
  radius: number,
  width: number,
  height: number,
): { x: number; y: number } {
  const pad = radius + MARGIN;
  for (let i = 0; i < 100; i++) {
    const x = pad + Math.random() * (width - 2 * pad);
    const y = pad + Math.random() * (height - 2 * pad);
    const ok = existing.every((b) => {
      const dx = x - b.x, dy = y - b.y;
      return Math.sqrt(dx * dx + dy * dy) >= radius + b.radius + MARGIN;
    });
    if (ok) return { x, y };
  }
  // Fallback: grid
  const cols = Math.max(1, Math.floor(width / (2 * radius + MARGIN)));
  const idx = existing.length;
  return {
    x: pad + (idx % cols) * (2 * radius + MARGIN),
    y: pad + Math.floor(idx / cols) * (2 * radius + MARGIN),
  };
}

export function useBubblePhysics(
  bubbles: BubbleData[],
  container: { width: number; height: number },
) {
  const statesRef = useRef<Map<string, BubbleState>>(new Map());
  const rafRef = useRef(0);
  const lastTimeRef = useRef(0);
  // Drag state: id, offset from pointer to bubble center, velocity history
  const dragRef = useRef<{
    id: string;
    ox: number;
    oy: number;
  } | null>(null);
  const velHistoryRef = useRef<{ x: number; y: number; t: number }[]>([]);
  const [renderStates, setRenderStates] = useState<Map<string, BubbleState>>(new Map());

  // prefers-reduced-motion: no ambient motion exists any more, but the media
  // query still gates the throw-on-release impulse and the speed ceiling
  // (maxSpeed 0 ⇒ velocity stays at 0), so bubbles are guaranteed stationary.
  const reducedMotionRef = useRef(false);
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    const apply = () => { reducedMotionRef.current = mq.matches; };
    apply();
    mq.addEventListener('change', apply);
    return () => { mq.removeEventListener('change', apply); };
  }, []);

  // Sync device list → physics states
  useEffect(() => {
    const { width, height } = container;
    if (!width || !height) return;

    const alive = new Set(bubbles.map((b) => b.id));

    // Mark removed as popping
    statesRef.current.forEach((s, id) => {
      if (!alive.has(id) && !s.popping) {
        s.popping = true;
        s.scale = 1.15;
        setTimeout(() => statesRef.current.delete(id), 600);
      }
    });

    // Add / update
    for (const bubble of bubbles) {
      const existing = statesRef.current.get(bubble.id);
      if (existing) {
        existing.data = bubble;
        continue;
      }
      const radius = getRadius(bubble.deviceType);
      const pos = findOpenPosition(
        Array.from(statesRef.current.values()),
        radius, width, height,
      );
      statesRef.current.set(bubble.id, {
        id: bubble.id,
        x: pos.x,
        y: pos.y,
        // Spawn at rest — bubbles only ever move because the user throws them
        vx: 0,
        vy: 0,
        radius,
        mass: getMass(radius),
        scale: 0,
        opacity: 1,
        popping: false,
        data: bubble,
      });
    }
  }, [bubbles, container]);

  // Physics loop
  useEffect(() => {
    const { width, height } = container;
    if (!width || !height) return;

    let frame = 0;

    const tick = (time: number) => {
      if (!lastTimeRef.current) lastTimeRef.current = time;
      const rawDt = (time - lastTimeRef.current) / 16.67;
      const dt = Math.min(rawDt, 3);
      lastTimeRef.current = time;

      const all = Array.from(statesRef.current.values());
      const reduced = reducedMotionRef.current;
      // 0 under prefers-reduced-motion ⇒ any velocity is zeroed immediately
      const maxSpeed = reduced ? 0 : MAX_SPEED;

      for (const s of all) {
        if (s.popping) {
          s.scale = Math.min(s.scale + 0.08 * dt, 1.2);
          s.opacity = Math.max(s.opacity - 0.04 * dt, 0);
          continue;
        }

        // Entry spring
        if (s.scale < 1) {
          s.scale = Math.min(s.scale + 0.05 * dt, 1);
        }

        // Skip physics for dragged bubble
        if (dragRef.current?.id === s.id) continue;

        // Speed clamp (throw ceiling / reduced-motion stop)
        const spd = Math.sqrt(s.vx * s.vx + s.vy * s.vy);
        if (spd > maxSpeed) {
          s.vx = (s.vx / spd) * maxSpeed;
          s.vy = (s.vy / spd) * maxSpeed;
        } else if (spd < REST_SPEED) {
          // Damped to a full stop — no minimum-speed floor, so they settle
          s.vx = 0;
          s.vy = 0;
        }

        s.x += s.vx * dt;
        s.y += s.vy * dt;

        // Walls — bounce with restitution, position clamped inside
        const m = s.radius + 8;
        if (s.x - m < 0) { s.x = m; s.vx = Math.abs(s.vx) * 0.7; }
        else if (s.x + m > width) { s.x = width - m; s.vx = -Math.abs(s.vx) * 0.7; }
        if (s.y - m < 0) { s.y = m; s.vy = Math.abs(s.vy) * 0.7; }
        else if (s.y + m > height) { s.y = height - m; s.vy = -Math.abs(s.vy) * 0.7; }

        // dt-corrected damping: decay rate is per 60fps-frame regardless of
        // the actual frame rate (a plain *= DAMPING stalled for ~11s at 30fps)
        const damp = Math.pow(DAMPING, dt);
        s.vx *= damp;
        s.vy *= damp;
      }

      // Bubble-bubble collisions
      for (let i = 0; i < all.length; i++) {
        for (let j = i + 1; j < all.length; j++) {
          const a = all[i], b = all[j];
          if (a.popping || b.popping) continue;
          if (dragRef.current?.id === a.id || dragRef.current?.id === b.id) continue;

          const dx = b.x - a.x, dy = b.y - a.y;
          const dist = Math.sqrt(dx * dx + dy * dy);
          const minD = a.radius + b.radius;

          if (dist < minD && dist > 0.01) {
            const nx = dx / dist, ny = dy / dist;
            const dvx = a.vx - b.vx, dvy = a.vy - b.vy;
            const dvn = dvx * nx + dvy * ny;

            if (dvn > 0) {
              const imp = (2 * dvn) / (a.mass + b.mass) * 0.9;
              a.vx -= imp * b.mass * nx;
              a.vy -= imp * b.mass * ny;
              b.vx += imp * a.mass * nx;
              b.vy += imp * a.mass * ny;
            }

            // Separation
            const overlap = minD - dist;
            const tm = a.mass + b.mass;
            a.x -= (overlap * b.mass / tm) * nx;
            a.y -= (overlap * b.mass / tm) * ny;
            b.x += (overlap * a.mass / tm) * nx;
            b.y += (overlap * a.mass / tm) * ny;
          }
        }
      }

      frame++;
      if (frame % 1 === 0) {
        setRenderStates(new Map(statesRef.current));
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => { cancelAnimationFrame(rafRef.current); };
  }, [container]);

  // --- Drag API: called by canvas, NOT by individual bubbles ---
  const startDrag = useCallback((id: string, pointerX: number, pointerY: number) => {
    const s = statesRef.current.get(id);
    if (!s || s.popping) return;
    // Grabbing cancels any residual motion — the throw on release is the only
    // thing that can set velocity again
    s.vx = 0;
    s.vy = 0;
    // ox/oy = pointer offset from bubble center (in canvas coords)
    dragRef.current = { id, ox: pointerX - s.x, oy: pointerY - s.y };
    velHistoryRef.current = [{ x: pointerX, y: pointerY, t: performance.now() }];
  }, []);

  const moveDrag = useCallback((pointerX: number, pointerY: number) => {
    const d = dragRef.current;
    if (!d) return;
    const s = statesRef.current.get(d.id);
    if (!s) return;

    s.x = pointerX - d.ox;
    s.y = pointerY - d.oy;

    // Track pointer history for the throw velocity computed on release
    const now = performance.now();
    velHistoryRef.current.push({ x: pointerX, y: pointerY, t: now });
    if (velHistoryRef.current.length > 8) velHistoryRef.current.shift();
  }, []);

  const endDrag = useCallback(() => {
    const d = dragRef.current;
    if (!d) return;
    const s = statesRef.current.get(d.id);
    const hist = velHistoryRef.current;
    if (s && hist.length >= 2) {
      // Use last few entries for fling velocity
      const recent = hist.slice(-4);
      const first = recent[0];
      const last = recent[recent.length - 1];
      const dt = (last.t - first.t) / 16.67;
      // Throw velocity is suppressed under prefers-reduced-motion
      if (dt > 0 && !reducedMotionRef.current) {
        s.vx = ((last.x - first.x) / dt) * 0.35;
        s.vy = ((last.y - first.y) / dt) * 0.35;
      }
    }
    dragRef.current = null;
    velHistoryRef.current = [];
  }, []);

  const findBubbleAt = useCallback((pointerX: number, pointerY: number): string | null => {
    // Find topmost bubble under pointer (check in reverse render order)
    const all = Array.from(statesRef.current.values());
    for (let i = all.length - 1; i >= 0; i--) {
      const s = all[i];
      if (s.popping) continue;
      const dx = pointerX - s.x;
      const dy = pointerY - s.y;
      if (dx * dx + dy * dy <= s.radius * s.radius) {
        return s.id;
      }
    }
    return null;
  }, []);

  return { renderStates, startDrag, moveDrag, endDrag, findBubbleAt };
}
