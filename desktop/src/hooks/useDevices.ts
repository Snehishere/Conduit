import { useState, useCallback, useEffect } from 'react';

interface Device {
  id: string;
  name: string;
  device_type: string;
  os: string;
  battery?: number;
  signal?: string;
  status: string;
  last_seen: number;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

export function useDevices() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);

  const refreshDevices = useCallback(async () => {
    try {
      const result = await invoke<Device[]>('get_devices');
      setDevices(result);
    } catch (err) {
      console.error('Failed to refresh devices:', err);
    }
  }, []);

  const removeDevice = useCallback(async (id: string) => {
    try {
      await invoke('delete_paired_device', { id });
      setDevices((prev) => prev.filter((d) => d.id !== id));
      if (selectedDevice === id) setSelectedDevice(null);
    } catch (err) {
      console.error('Failed to remove device:', err);
    }
  }, [selectedDevice]);

  useEffect(() => {
    refreshDevices();
    const interval = setInterval(refreshDevices, 5000);
    return () => clearInterval(interval);
  }, [refreshDevices]);

  return { devices, selectedDevice, setSelectedDevice, refreshDevices, removeDevice };
}
