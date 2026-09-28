// @ts-nocheck
import { useEffect } from 'react';
import type { WebSocketMessage, MessageHandler } from '../types/websocket';

interface MessageHandlers {
  handleFileMessage: (data: WebSocketMessage) => void;
  handleSmsMessage: (data: WebSocketMessage) => void;
  handleCallMessage: (data: WebSocketMessage) => void;
  handleAudioMessage: (data: WebSocketMessage) => void;
  handleDiscoveryMessage: (data: WebSocketMessage) => void;
  handleIncoming: (entry: {
    content: string;
    mime: string;
    sourceDevice: string;
    timestamp: number;
  }) => void;
  registerHandler: (type: string, handler: MessageHandler) => () => void;
}

export function useMessageHandlers({
  handleFileMessage,
  handleSmsMessage,
  handleCallMessage,
  handleAudioMessage,
  handleDiscoveryMessage,
  handleIncoming,
  registerHandler,
}: MessageHandlers) {
  useEffect(() => {
    const unsubs = [
      registerHandler('clipboard', (data: WebSocketMessage) => {
        // Type check needed to access the sync fields; 'sync' is the only
        // clipboard variant so an extra action check would be redundant.
        if (data.type === 'clipboard') {
          handleIncoming({
            content: (data as any).content,
            mime: data.mime,
            sourceDevice: (data as any).source_device,
            timestamp: (data as any).timestamp,
          });
        }
      }),
      registerHandler('file', handleFileMessage),
      registerHandler('sms', handleSmsMessage),
      registerHandler('call', handleCallMessage),
      registerHandler('audio', handleAudioMessage),
      registerHandler('discovery', handleDiscoveryMessage),
    ];
    return () => { unsubs.forEach((unsub) => { unsub(); }); };
  }, [
    registerHandler,
    handleIncoming,
    handleFileMessage,
    handleSmsMessage,
    handleCallMessage,
    handleAudioMessage,
    handleDiscoveryMessage,
  ]);
}
