import { useState, useCallback } from 'react';
import { showError } from '../lib/toast';

export type CallStatus = 'idle' | 'ringing' | 'active' | 'ended';

export interface Call {
  call_id: string;
  number: string;
  name: string | null;
  status: CallStatus;
  device_id: string;
  start_time?: number;
  end_time?: number;
}

export interface AudioDevice {
  id: string;
  name: string;
  type: 'speaker' | 'headphones' | 'bluetooth' | 'earpiece';
  active: boolean;
}

function timeAgo(timestamp: number): string {
  const minutes = Math.floor((Date.now() / 1000 - timestamp) / 60);
  if (minutes < 1) return 'now';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

export function useCalls(sendMessage?: (msg: Record<string, unknown>) => void) {
  const [activeCall, setActiveCall] = useState<Call | null>(null);
  const [callHistory, setCallHistory] = useState<Call[]>([]);
  const [audioDevices, setAudioDevices] = useState<AudioDevice[]>([]);
  const [currentRoute, setCurrentRoute] = useState<'phone' | 'desktop' | 'bluetooth'>('phone');
  const [isStreaming, setIsStreaming] = useState(false);

  const handleCallMessage = useCallback((data: Record<string, unknown>) => {
    const action = data.action as string;

    switch (action) {
      case 'incoming': {
        const call: Call = {
          call_id: data.call_id as string,
          number: data.number as string,
          name: data.name as string | null,
          status: 'ringing',
          device_id: data.device_id as string,
        };
        setActiveCall(call);
        break;
      }
      case 'answered': {
        setActiveCall((prev) =>
          prev
            ? { ...prev, status: 'active', start_time: Math.floor(Date.now() / 1000) }
            : prev
        );
        break;
      }
      case 'ended': {
        setActiveCall((prev) => {
          if (prev) {
            const ended = { ...prev, status: 'ended' as CallStatus, end_time: Math.floor(Date.now() / 1000) };
            setCallHistory((h) => [ended, ...h].slice(0, 50));
            return null;
          }
          return prev;
        });
        break;
      }
      case 'devices': {
        setAudioDevices(data.devices as AudioDevice[]);
        break;
      }
      case 'route': {
        setCurrentRoute(data.route as 'phone' | 'desktop' | 'bluetooth');
        break;
      }
      case 'stream_started': {
        setIsStreaming(true);
        break;
      }
      case 'stream_stop': {
        setIsStreaming(false);
        break;
      }
    }
  }, []);

  const handleAudioMessage = useCallback((data: Record<string, unknown>) => {
    const action = data.action as string;

    switch (action) {
      case 'stream_data': {
        // Received audio data from mobile - play it
        // Audio playback is handled by the audio module in Rust backend
        break;
      }
      case 'stream_started': {
        setIsStreaming(true);
        break;
      }
      case 'stream_stop': {
        setIsStreaming(false);
        break;
      }
    }
  }, []);

  const answerCall = useCallback(() => {
    if (activeCall) {
      if (!sendMessage) {
        showError('Cannot answer call: Not connected to device');
        return;
      }
      sendMessage({ type: 'call', action: 'answer', call_id: activeCall.call_id });
      setActiveCall((prev) =>
        prev ? { ...prev, status: 'active', start_time: Math.floor(Date.now() / 1000) } : prev
      );
    }
  }, [activeCall, sendMessage]);

  const rejectCall = useCallback(() => {
    if (activeCall) {
      if (!sendMessage) {
        showError('Cannot reject call: Not connected to device');
        return;
      }
      sendMessage({ type: 'call', action: 'reject', call_id: activeCall.call_id });
      setActiveCall(null);
    }
  }, [activeCall, sendMessage]);

  const endCall = useCallback(() => {
    if (activeCall) {
      if (!sendMessage) {
        showError('Cannot end call: Not connected to device');
        return;
      }
      sendMessage({ type: 'call', action: 'end', call_id: activeCall.call_id });
      const ended = { ...activeCall, status: 'ended' as CallStatus, end_time: Math.floor(Date.now() / 1000) };
      setCallHistory((h) => [ended, ...h].slice(0, 50));
      setActiveCall(null);
    }
  }, [activeCall, sendMessage]);

  const forwardCall = useCallback((toDeviceId: string) => {
    if (activeCall) {
      if (!sendMessage) {
        showError('Cannot forward call: Not connected to device');
        return;
      }
      sendMessage({ type: 'call', action: 'forward', call_id: activeCall.call_id, to_device_id: toDeviceId });
    }
  }, [activeCall, sendMessage]);

  const setAudioRoute = useCallback((route: 'phone' | 'desktop' | 'bluetooth') => {
    if (!sendMessage) {
      showError('Cannot change audio route: Not connected');
      return;
    }
    setCurrentRoute(route);
    sendMessage({ type: 'audio', action: 'route', route });
  }, [sendMessage]);

  const startAudioStream = useCallback(() => {
    if (!sendMessage) {
      showError('Cannot start audio stream: Not connected');
      return;
    }
    sendMessage({ type: 'audio', action: 'stream_start' });
    setIsStreaming(true);
  }, [sendMessage]);

  const stopAudioStream = useCallback(() => {
    if (!sendMessage) {
      showError('Cannot stop audio stream: Not connected');
      return;
    }
    sendMessage({ type: 'audio', action: 'stream_stop' });
    setIsStreaming(false);
  }, [sendMessage]);

  const getCallerName = useCallback((call: Call) => {
    return call.name || call.number || 'Unknown';
  }, []);

  const getCallDuration = useCallback((call: Call) => {
    if (!call.start_time) return '0:00';
    const end = call.end_time || Math.floor(Date.now() / 1000);
    const seconds = end - call.start_time;
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  }, []);

  return {
    activeCall,
    callHistory,
    audioDevices,
    currentRoute,
    isStreaming,
    handleCallMessage,
    handleAudioMessage,
    answerCall,
    rejectCall,
    endCall,
    forwardCall,
    setAudioRoute,
    startAudioStream,
    stopAudioStream,
    getCallerName,
    getCallDuration,
    timeAgo,
  };
}
