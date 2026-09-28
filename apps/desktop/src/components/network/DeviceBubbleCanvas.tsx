import React, { useRef, useState, useEffect, useCallback } from 'react';
import { AnimatePresence } from 'motion/react';
import { useBubblePhysics, type BubbleData } from '../../hooks/useBubblePhysics';
import DeviceBubble from './DeviceBubble';
import DeviceBubbleParticles from './DeviceBubbleParticles';

interface DeviceBubbleCanvasProps {
  devices: BubbleData[];
  selectedDevice: string | null;
  onSelectDevice: (id: string | null) => void;
  onBubbleClick: (device: BubbleData) => void;
  /** Called when Shift+clicking a bubble — used for audio streaming */
  onShiftClick?: (deviceId: string) => void;
}

const DeviceBubbleCanvas: React.FC<DeviceBubbleCanvasProps> = ({
  devices,
  selectedDevice,
  onSelectDevice,
  onBubbleClick,
  onShiftClick,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [poppedBubbles, setPoppedBubbles] = useState<
    { id: string; x: number; y: number; color: string; radius: number }[]
  >([]);

  const dragIdRef = useRef<string | null>(null);
  const dragStartedRef = useRef(false);
  const pointerDownRef = useRef<{ id: string; x: number; y: number } | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize({ width, height });
    });
    observer.observe(el);
    return () => { observer.disconnect(); };
  }, []);

  const { renderStates, startDrag, moveDrag, endDrag, findBubbleAt } = useBubblePhysics(devices, size);

  const getCanvasCoords = useCallback((e: React.PointerEvent | PointerEvent): { x: number; y: number } | null => {
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect) return null;
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const coords = getCanvasCoords(e);
    if (!coords) return;

    const hitId = findBubbleAt(coords.x, coords.y);
    if (hitId) {
      pointerDownRef.current = { id: hitId, x: coords.x, y: coords.y };
      dragStartedRef.current = false;
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      e.preventDefault();
    } else {
      pointerDownRef.current = null;
      onSelectDevice(null);
    }
  }, [findBubbleAt, getCanvasCoords, onSelectDevice]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    const coords = getCanvasCoords(e);
    if (!coords) return;

    const down = pointerDownRef.current;
    if (down) {
      const dx = coords.x - down.x;
      const dy = coords.y - down.y;

      if (!dragStartedRef.current) {
        if (Math.abs(dx) > 4 || Math.abs(dy) > 4) {
          dragStartedRef.current = true;
          dragIdRef.current = down.id;
          setDraggingId(down.id);
          startDrag(down.id, coords.x, coords.y);
        }
      }

      if (dragStartedRef.current) {
        moveDrag(coords.x, coords.y);
      }
    }
  }, [getCanvasCoords, startDrag, moveDrag]);

  const handlePointerUp = useCallback((e: React.PointerEvent) => {
    const down = pointerDownRef.current;

    if (dragStartedRef.current && down) {
      endDrag();
      dragIdRef.current = null;
      setDraggingId(null);
    } else if (down) {
      const state = renderStates.get(down.id);
      if (state) {
        // Shift+click triggers audio streaming to this device
        if (e.shiftKey && onShiftClick) {
          onShiftClick(down.id);
        } else {
          onSelectDevice(down.id);
          onBubbleClick(state.data);
        }
      }
    }

    pointerDownRef.current = null;
    dragStartedRef.current = false;
  }, [endDrag, renderStates, onSelectDevice, onBubbleClick, onShiftClick]);

  const handlePointerLeave = useCallback(() => {
    if (dragStartedRef.current) {
      endDrag();
      dragIdRef.current = null;
      setDraggingId(null);
    }
    pointerDownRef.current = null;
    dragStartedRef.current = false;
  }, [endDrag]);

  const prevIdsRef = useRef(new Set<string>());
  useEffect(() => {
    const currentIds = new Set(devices.map((d) => d.id));
    const prevIds = prevIdsRef.current;

    for (const id of prevIds) {
      if (!currentIds.has(id)) {
        const state = renderStates.get(id);
        if (state) {
          setPoppedBubbles((prev) => [
            ...prev,
            {
              id: `${id}-${Date.now()}`,
              x: state.x,
              y: state.y,
              color: '#8e8e93',
              radius: state.radius,
            },
          ]);
        }
      }
    }
    prevIdsRef.current = currentIds;
  }, [devices, renderStates]);

  useEffect(() => {
    if (poppedBubbles.length === 0) return;
    const timer = setTimeout(() => {
      setPoppedBubbles((prev) => prev.slice(1));
    }, 700);
    return () => { clearTimeout(timer); };
  }, [poppedBubbles]);

  return (
    <div
      ref={containerRef}
      className="relative w-full h-full overflow-hidden"
      style={{
        // Transparent: reveals the body .starfield (spec §B1)
        cursor: draggingId ? 'grabbing' : 'default',
        touchAction: 'none',
      }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerLeave}
    >
      {Array.from(renderStates.entries()).map(([id, state]) => (
        <DeviceBubble
          key={id}
          id={id}
          x={state.x}
          y={state.y}
          radius={state.radius}
          scale={state.scale}
          opacity={state.opacity}
          popping={state.popping}
          name={state.data.name}
          deviceType={state.data.deviceType}
          os={state.data.os}
          battery={state.data.battery}
          status={state.data.status}
          isSelected={selectedDevice === id}
          isDragging={draggingId === id}
        />
      ))}

      <AnimatePresence>
        {poppedBubbles.map((p) => (
          <DeviceBubbleParticles
            key={p.id}
            x={p.x}
            y={p.y}
            color={p.color}
            radius={p.radius}
          />
        ))}
      </AnimatePresence>
    </div>
  );
};

export default DeviceBubbleCanvas;
