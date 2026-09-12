import { useState, useEffect, useCallback, useRef } from 'react';
import { useSharedWebSocket } from './useWebSocket';

interface WatchDevice {
  id: string;
  name: string;
  type: 'wearos' | 'galaxy_watch' | 'apple_watch';
  battery: number;
  isConnected: boolean;
  lastSeen: number;
}

interface WatchNotification {
  id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  dismissed: boolean;
}

interface WatchCommand {
  type: 'watch';
  action: 'notify' | 'quick_reply' | 'find_phone' | 'music_control' | 'battery_update' | 'sync';
  device_id: string;
  command: string;
  value?: string;
}

export function useWatch() {
  const [watches, setWatches] = useState<WatchDevice[]>([]);
  const [selectedWatch, setSelectedWatch] = useState<WatchDevice | null>(null);
  const [notifications, setNotifications] = useState<WatchNotification[]>([]);
  const [isDiscovering, setIsDiscovering] = useState(false);
  const [currentTrack, setCurrentTrack] = useState<{ title: string; artist: string; isPlaying: boolean } | null>(null);
  const discoverGen = useRef(0);
  const { registerHandler, sendMessage } = useSharedWebSocket();

  useEffect(() => {
    return registerHandler('watch', (data: any) => {
      switch (data.action) {
        case 'discovered':
          addOrUpdateWatch(data.watch);
          break;
        case 'status':
          updateWatchStatus(data.device_id, data);
          break;
        case 'notification':
          addNotification(data.notification);
          break;
        case 'battery':
          updateWatchBattery(data.device_id, data.battery);
          break;
        case 'music':
          updateMusicStatus(data);
          break;
      }
    });
  }, [registerHandler]);

  const addOrUpdateWatch = useCallback((watch: WatchDevice) => {
    setWatches(prev => {
      const index = prev.findIndex(w => w.id === watch.id);
      if (index >= 0) {
        const updated = [...prev];
        updated[index] = watch;
        return updated;
      }
      return [...prev, watch];
    });
  }, []);

  const updateWatchStatus = useCallback((deviceId: string, status: Partial<WatchDevice>) => {
    setWatches(prev => prev.map(watch =>
      watch.id === deviceId ? { ...watch, ...status } : watch
    ));
  }, []);

  const updateWatchBattery = useCallback((deviceId: string, battery: number) => {
    setWatches(prev => prev.map(watch =>
      watch.id === deviceId ? { ...watch, battery } : watch
    ));
  }, []);

  const addNotification = useCallback((notification: WatchNotification) => {
    setNotifications(prev => [notification, ...prev].slice(0, 50));
  }, []);

  const updateMusicStatus = useCallback((data: any) => {
    setCurrentTrack({
      title: data.title,
      artist: data.artist,
      isPlaying: data.isPlaying,
    });
  }, []);

  const discoverWatches = useCallback(() => {
    setIsDiscovering(true);
    sendMessage({ type: 'watch', action: 'discover' });
    // Generation counter so overlapping discovers don't cancel each other early.
    discoverGen.current += 1;
    const gen = discoverGen.current;
    setTimeout(() => {
      if (discoverGen.current === gen) setIsDiscovering(false);
    }, 5000);
  }, [sendMessage]);

  const sendWatchCommand = useCallback((command: WatchCommand) => {
    sendMessage(command as unknown as Record<string, unknown>);
  }, [sendMessage]);

  const sendNotification = useCallback((watchId: string, title: string, body: string) => {
    sendWatchCommand({
      type: 'watch',
      action: 'notify',
      device_id: watchId,
      command: 'send',
      value: JSON.stringify({ title, body }),
    });
  }, [sendWatchCommand]);

  const quickReply = useCallback((watchId: string, notificationId: string, reply: string) => {
    sendWatchCommand({
      type: 'watch',
      action: 'quick_reply',
      device_id: watchId,
      command: 'reply',
      value: JSON.stringify({ notificationId, reply }),
    });
  }, [sendWatchCommand]);

  const findPhone = useCallback((watchId: string) => {
    sendWatchCommand({
      type: 'watch',
      action: 'find_phone',
      device_id: watchId,
      command: 'ring',
    });
  }, [sendWatchCommand]);

  const controlMusic = useCallback((watchId: string, action: 'play' | 'pause' | 'next' | 'prev') => {
    sendWatchCommand({
      type: 'watch',
      action: 'music_control',
      device_id: watchId,
      command: action,
    });
  }, [sendWatchCommand]);

  const dismissNotification = useCallback((notificationId: string) => {
    setNotifications(prev => prev.map(n =>
      n.id === notificationId ? { ...n, dismissed: true } : n
    ));
  }, []);

  return {
    watches,
    selectedWatch,
    setSelectedWatch,
    notifications,
    isDiscovering,
    currentTrack,
    discoverWatches,
    sendNotification,
    quickReply,
    findPhone,
    controlMusic,
    dismissNotification,
  };
}