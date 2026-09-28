import { useState, useCallback } from 'react';

/**
 * Manages pairing dialog visibility state.
 * Extracted from AppInner to isolate pairing flow concerns.
 */
export function usePairing() {
  const [showPairing, setShowPairing] = useState(false);

  const openPairing = useCallback(() => { setShowPairing(true); }, []);
  const closePairing = useCallback(() => { setShowPairing(false); }, []);

  return {
    showPairing,
    setShowPairing,
    openPairing,
    closePairing,
  };
}
