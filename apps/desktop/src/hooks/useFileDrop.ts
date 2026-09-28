import { useState, useCallback, useRef } from 'react';

/**
 * Manages file drop zone visibility and drag-over counter state.
 * The dragCounter tracks nested dragenter/dragleave events to prevent
 * flickering when dragging over child elements.
 */
export function useFileDrop() {
  const [showDropZone, setShowDropZone] = useState(false);
  const dragCounter = useRef(0);

  const onDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    dragCounter.current += 1;
    if (dragCounter.current === 1) {
      setShowDropZone(true);
    }
  }, []);

  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    dragCounter.current -= 1;
    if (dragCounter.current <= 0) {
      dragCounter.current = 0;
      setShowDropZone(false);
    }
  }, []);

  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
  }, []);

  const openDropZone = useCallback(() => { setShowDropZone(true); }, []);
  const closeDropZone = useCallback(() => {
    dragCounter.current = 0;
    setShowDropZone(false);
  }, []);

  return {
    showDropZone,
    setShowDropZone,
    dragCounter: dragCounter.current,
    onDragEnter,
    onDragLeave,
    onDragOver,
    openDropZone,
    closeDropZone,
  };
}
