import { useState, useEffect, useCallback } from 'react';

interface EncryptionInfo {
  public_key: string;
  device_id: string;
  name: string;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

export function useEncryption() {
  const [publicKey, setPublicKey] = useState('');
  const [deviceId, setDeviceId] = useState('');
  const [deviceName, setDeviceName] = useState('');
  const [initialized, setInitialized] = useState(false);

  useEffect(() => {
    initEncryption();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const initEncryption = async () => {
    try {
      const info = await invoke<EncryptionInfo>('get_device_info');
      setPublicKey(info.public_key || '');
      setDeviceId(info.device_id || '');
      setDeviceName(info.name || 'Desktop');
      setInitialized(true);
    } catch (err) {
      console.error('Failed to initialize encryption:', err);
    }
  };

  const getPublicKey = useCallback(async (): Promise<string> => {
    if (publicKey) return publicKey;
    const info = await invoke<EncryptionInfo>('get_device_info');
    setPublicKey(info.public_key);
    return info.public_key;
  }, [publicKey]);

  const sendEncrypted = useCallback(async (
    targetDeviceId: string,
    plaintext: string,
  ): Promise<boolean> => {
    try {
      await invoke('send_encrypted_message', {
        targetDeviceId,
        plaintext: plaintext,
      });
      return true;
    } catch (err) {
      console.error('Failed to send encrypted message:', err);
      return false;
    }
  }, []);

  return {
    publicKey,
    deviceId,
    deviceName,
    initialized,
    getPublicKey,
    sendEncrypted,
  };
}
