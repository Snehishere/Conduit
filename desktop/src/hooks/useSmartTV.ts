import { useState, useEffect, useCallback, useRef } from 'react';
import { useSharedWebSocket } from './useWebSocket';

interface SmartTV {
  id: string;
  name: string;
  ip: string;
  port: number;
  manufacturer: string;
  model: string;
  isOnline: boolean;
  isPlaying: boolean;
  currentApp?: string;
  volume: number;
  isMuted: boolean;
}

interface TVCommand {
  type: 'tv';
  action: 'power' | 'volume' | 'mute' | 'input' | 'app' | 'media' | 'remote';
  device_id: string;
  command: string;
  value?: string | number;
}

export function useSmartTV() {
  const [tvs, setTVs] = useState<SmartTV[]>([]);
  const [selectedTV, setSelectedTV] = useState<SmartTV | null>(null);
  const [isDiscovering, setIsDiscovering] = useState(false);
  const { registerHandler, sendMessage } = useSharedWebSocket();
  const discoverGen = useRef(0);

  useEffect(() => {
    return registerHandler('tv', (data: any) => {
      switch (data.action) {
        case 'discovered':
          addOrUpdateTV(data.tv);
          break;
        case 'status':
          updateTVStatus(data.device_id, data);
          break;
        case 'power':
          updateTVPower(data.device_id, data.powered);
          break;
        case 'volume':
          updateTVVolume(data.device_id, data.volume, data.muted);
          break;
      }
    });
  }, [registerHandler]);

  const addOrUpdateTV = useCallback((tv: SmartTV) => {
    setTVs(prev => {
      const index = prev.findIndex(t => t.id === tv.id);
      if (index >= 0) {
        const updated = [...prev];
        updated[index] = tv;
        return updated;
      }
      return [...prev, tv];
    });
  }, []);

  const updateTVStatus = useCallback((deviceId: string, status: Partial<SmartTV>) => {
    setTVs(prev => prev.map(tv =>
      tv.id === deviceId ? { ...tv, ...status } : tv
    ));
  }, []);

  const updateTVPower = useCallback((deviceId: string, powered: boolean) => {
    setTVs(prev => prev.map(tv =>
      tv.id === deviceId ? { ...tv, isOnline: powered } : tv
    ));
  }, []);

  const updateTVVolume = useCallback((deviceId: string, volume: number, muted: boolean) => {
    setTVs(prev => prev.map(tv =>
      tv.id === deviceId ? { ...tv, volume, isMuted: muted } : tv
    ));
  }, []);

  const discoverTVs = useCallback(() => {
    setIsDiscovering(true);
    sendMessage({ type: 'tv', action: 'discover' });
    discoverGen.current += 1;
    const gen = discoverGen.current;
    setTimeout(() => {
      if (discoverGen.current === gen) setIsDiscovering(false);
    }, 5000);
  }, [sendMessage]);

  const sendTVCommand = useCallback((command: TVCommand) => {
    sendMessage(command as unknown as Record<string, unknown>);
  }, [sendMessage]);

  const powerOn = useCallback((tvId: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'power',
      device_id: tvId,
      command: 'on',
    });
  }, [sendTVCommand]);

  const powerOff = useCallback((tvId: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'power',
      device_id: tvId,
      command: 'off',
    });
  }, [sendTVCommand]);

  const setVolume = useCallback((tvId: string, volume: number) => {
    sendTVCommand({
      type: 'tv',
      action: 'volume',
      device_id: tvId,
      command: 'set',
      value: volume,
    });
  }, [sendTVCommand]);

  const toggleMute = useCallback((tvId: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'mute',
      device_id: tvId,
      command: 'toggle',
    });
  }, [sendTVCommand]);

  const setInput = useCallback((tvId: string, input: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'input',
      device_id: tvId,
      command: 'switch',
      value: input,
    });
  }, [sendTVCommand]);

  const launchApp = useCallback((tvId: string, appId: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'app',
      device_id: tvId,
      command: 'launch',
      value: appId,
    });
  }, [sendTVCommand]);

  const sendRemoteCommand = useCallback((tvId: string, key: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'remote',
      device_id: tvId,
      command: key,
    });
  }, [sendTVCommand]);

  const castScreen = useCallback((tvId: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'media',
      device_id: tvId,
      command: 'cast_screen',
    });
  }, [sendTVCommand]);

  const castMedia = useCallback((tvId: string, url: string) => {
    sendTVCommand({
      type: 'tv',
      action: 'media',
      device_id: tvId,
      command: 'play',
      value: url,
    });
  }, [sendTVCommand]);

  return {
    tvs,
    selectedTV,
    setSelectedTV,
    isDiscovering,
    discoverTVs,
    powerOn,
    powerOff,
    setVolume,
    toggleMute,
    setInput,
    launchApp,
    sendRemoteCommand,
    castScreen,
    castMedia,
  };
}