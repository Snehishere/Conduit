import { useState, useCallback, useEffect } from 'react';
import { showError, showSuccess } from '../lib/toast';

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
  const [error, setError] = useState<string | null>(null);

  const refreshDevices = useCallback(async () => {
    try {
      setError(null);
      const result = await invoke<Device[]>('get_devices');
      setDevices(result);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      console.error('[useDevices] Failed to refresh devices:', err);
      showError(`Failed to refresh devices: ${message}`);
    }
  }, []);

  const removeDevice = useCallback(async (id: string) => {
    try {
      setError(null);
      await invoke('delete_paired_device', { id });
      setDevices((prev) => prev.filter((d) => d.id !== id));
      if (selectedDevice === id) setSelectedDevice(null);
      showSuccess('Device removed successfully');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      console.error('[useDevices] Failed to remove device:', err);
      showError(`Failed to remove device: ${message}`);
    }
  }, [selectedDevice]);

  useEffect(() => {
    refreshDevices();
    const interval = setInterval(refreshDevices, 5000);
    return () => clearInterval(interval);
  }, [refreshDevices]);

  return { devices, selectedDevice, setSelectedDevice, refreshDevices, removeDevice, error };
}
