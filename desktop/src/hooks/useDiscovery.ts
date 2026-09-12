import { useState, useEffect, useCallback } from 'react';
import { showError } from '../lib/toast';

interface DiscoveredDevice {
  id: string;
  name: string;
  type: string;
  address: string;
  port: number;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

export function useDiscovery() {
  const [discoveredDevices, setDiscoveredDevices] = useState<DiscoveredDevice[]>([]);
  const [isDiscovering, setIsDiscovering] = useState(false);

  const startDiscovery = useCallback(async () => {
    try {
      await invoke('start_discovery');
      setIsDiscovering(true);
    } catch (err) {
      console.error('Failed to start discovery:', err);
    }
  }, []);

  const stopDiscovery = useCallback(async () => {
    try {
      await invoke('stop_discovery');
      setIsDiscovering(false);
    } catch (err) {
      console.error('Failed to stop discovery:', err);
    }
  }, []);

  // Listen for discovery announcements via WebSocket
  const handleDiscoveryMessage = useCallback((data: Record<string, unknown>) => {
    const action = data.action as string;
    if (action === 'announce') {
      const device: DiscoveredDevice = {
        id: data.device_id as string,
        name: (data.device_name as string) || (data.name as string) || 'Unknown',
        type: data.device_type as string,
        address: data.address as string || '',
        port: (data.port as number) || 9527,
      };
      setDiscoveredDevices((prev) => {
        const exists = prev.find((d) => d.id === device.id);
        if (exists) return prev;
        return [...prev, device];
      });
    }
  }, []);

  const clearDiscovered = useCallback(() => {
    setDiscoveredDevices([]);
  }, []);

  return {
    discoveredDevices,
    isDiscovering,
    startDiscovery,
    stopDiscovery,
    handleDiscoveryMessage,
    clearDiscovered,
  };
}
