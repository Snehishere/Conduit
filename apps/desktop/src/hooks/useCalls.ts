// @ts-nocheck
import { useState, useCallback } from 'react';
import { formatRelativeTime } from '../lib/time';
import type { WebSocketMessage } from '../types/websocket';
import type { SendResult } from './useEncryption';

export type CallStatus = 'idle' | 'ringing' | 'active' | 'ended';

export interface Call {
  call_id: string;
  number: string;
  name: string | null;
  status: CallStatus;
  direction?: 'incoming' | 'outgoing' | 'missed';
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

export interface AudioStreamState {
  /** Device ID currently being streamed to, null if not streaming */
  streamingTarget: string | null;
  /** Whether surround sound mode is active (all devices combined) */
  surroundSoundActive: boolean;
  /** Devices currently playing audio in surround mode */
  surroundDevices: string[];
}

/**
 * @param sendMessage      Plain WebSocket send (broadcast, unencrypted).
 * @param sendSensitive    Encrypted send path. Call control is the only message
 *                         type in the app with a destination device id, so it is
 *                         the only one that can be wrapped in an envelope — see
 *                         `MESSAGE_PROTECTION` in useEncryption.ts.
 */
export function useCalls(
  sendMessage?: (msg: Record<string, unknown>) => void,
  sendSensitive?: (
    targetDeviceId: string | null | undefined,
    msg: Record<string, unknown>,
  ) => Promise<SendResult>,
) {
  const [activeCall, setActiveCall] = useState<Call | null>(null);
  const [callHistory, setCallHistory] = useState<Call[]>([]);
  const [audioDevices, setAudioDevices] = useState<AudioDevice[]>([]);
  const [streamingTarget, setStreamingTarget] = useState<string | null>(null);
  const [surroundSoundActive, setSurroundSoundActive] = useState(false);
  const [surroundDevices, setSurroundDevices] = useState<string[]>([]);

  /**
   * Try the envelope path first, then fall back to the plain socket.
   *
   * The fallback is deliberate and visible: `sendSensitive` has already warned
   * the user and recorded the type as having left in the clear, and the status
   * bar reports it. Dropping the call control outright on an encryption
   * failure would be a functional regression bought with a security claim the
   * app cannot honour anyway — the relay terminates the envelope regardless.
   */
  const deliver = useCallback(
    async (targetDeviceId: string | null, msg: Record<string, unknown>): Promise<boolean> => {
      if (sendSensitive) {
        const result = await sendSensitive(targetDeviceId, msg);
        if (result.encrypted) return true;
      }
      if (!sendMessage) return false;
      sendMessage(msg);
      return true;
    },
    [sendMessage, sendSensitive],
  );

  const handleCallMessage = useCallback((data: WebSocketMessage) => {
    if (data.type !== 'call') return;

    switch (data.action) {
      case 'incoming': {
        const call: Call = {
          call_id: data.call_id,
          number: (data as any).number,
          name: (data as any).name,
          status: 'ringing',
          device_id: (data as any).device_id,
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
        setAudioDevices((data as any).devices);
        break;
      }
      case 'stream_started': {
        setStreamingTarget((data as any).target_device_id || null);
        break;
      }
      case 'stream_stopped' as any: {
        setStreamingTarget(null);
        break;
      }
      case 'surround_started' as any: {
        setSurroundSoundActive(true);
        setSurroundDevices((data as any).device_ids);
        break;
      }
      case 'surround_stopped' as any: {
        setSurroundSoundActive(false);
        setSurroundDevices([]);
        break;
      }
    }
  }, []);

  const handleAudioMessage = useCallback((data: WebSocketMessage) => {
    if (data.type !== 'audio') return;

    switch (data.action) {
      case 'stream_data': {
        // Received audio data from mobile - play it
        // Audio playback is handled by the audio module in Rust backend
        break;
      }
      case 'stream_started': {
        setStreamingTarget((data as any).target_device_id || null);
        break;
      }
      case 'stream_stopped' as any: {
        setStreamingTarget(null);
        break;
      }
      case 'surround_started' as any: {
        setSurroundSoundActive(true);
        setSurroundDevices((data as any).device_ids);
        break;
      }
      case 'surround_stopped' as any: {
        setSurroundSoundActive(false);
        setSurroundDevices([]);
        break;
      }
    }
  }, []);

  const dismissCall = useCallback(() => {
    setActiveCall(null);
  }, []);

  // Answer/reject/forward await the delivery instead of flipping local state
  // optimistically. A local Tauri invoke costs a few ms, and a stuck button is
  // recoverable (the user can still reject) whereas a silently dropped answer
  // is not — the same reason the status bar refuses to guess.
  const answerCall = useCallback(async () => {
    if (!activeCall) return;
    const sent = await deliver(activeCall.device_id, {
      type: 'call',
      action: 'answer',
      call_id: activeCall.call_id,
    });
    if (!sent) return;
    setActiveCall((prev) =>
      prev ? { ...prev, status: 'active', start_time: Math.floor(Date.now() / 1000) } : prev
    );
  }, [activeCall, deliver]);

  const rejectCall = useCallback(async () => {
    if (!activeCall) return;
    const sent = await deliver(activeCall.device_id, {
      type: 'call',
      action: 'reject',
      call_id: activeCall.call_id,
    });
    if (!sent) return;
    setActiveCall(null);
  }, [activeCall, deliver]);

  const forwardCall = useCallback(async (callId: string, toDeviceId: string) => {
    await deliver(toDeviceId, {
      type: 'call',
      action: 'forward',
      call_id: callId,
      to_device_id: toDeviceId,
    });
  }, [deliver]);

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
    streamingTarget,
    surroundSoundActive,
    surroundDevices,
    handleCallMessage,
    handleAudioMessage,
    dismissCall,
    answerCall,
    rejectCall,
    forwardCall,
    getCallerName,
    getCallDuration,
    timeAgo: formatRelativeTime,
  };
}
