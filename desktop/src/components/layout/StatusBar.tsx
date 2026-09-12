import React, { useState, useEffect } from 'react';
import { IconWeb, IconLock, IconSignal, IconZap } from '../icons';

interface StatusBarProps {
  connected: boolean;
  deviceCount: number;
  encryptionEnabled: boolean;
}

async function invoke<T>(cmd: string): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd);
}

const StatusBar: React.FC<StatusBarProps> = ({
  connected,
  deviceCount,
  encryptionEnabled,
}) => {
  const [ramMb, setRamMb] = useState<number | null>(null);

  useEffect(() => {
    const fetchRam = async () => {
      try {
        const info = await invoke<{ ram_used_mb: number }>('get_system_info');
        setRamMb(info.ram_used_mb);
      } catch {}
    };
    fetchRam();
    const interval = setInterval(fetchRam, 5000);
    return () => clearInterval(interval);
  }, []);

  return (
    <footer className="status-bar">
      <div className="status-left">
        <span className="status-item">
          <IconWeb size={12} /> {deviceCount} device{deviceCount !== 1 ? 's' : ''}
        </span>
        <span className="status-separator">·</span>
        <span className={`status-item ${connected ? 'status-connected' : 'status-disconnected'}`}>
          <IconSignal size={12} /> {connected ? 'Strong' : 'Disconnected'}
        </span>
        <span className="status-separator">·</span>
        <span className="status-item">
          <IconLock size={12} /> {encryptionEnabled ? 'E2E' : 'No encryption'}
        </span>
      </div>
      <div className="status-right">
        <span className="status-item">
          <IconZap size={12} /> {ramMb !== null ? `${ramMb} MB` : '...'}
        </span>
      </div>
    </footer>
  );
};

export default StatusBar;
