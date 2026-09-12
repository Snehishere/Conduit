import React, { useState, useEffect, useRef, useCallback } from 'react';
import { useSharedWebSocket } from '../../hooks/useWebSocket';

interface RemoteInputProps {
  deviceId: string;
  deviceName: string;
  onStop: () => void;
}

export default function RemoteInput({ deviceId, deviceName, onStop }: RemoteInputProps) {
  const [isConnected, setIsConnected] = useState(false);
  const [lastEvent, setLastEvent] = useState<string>('');
  const containerRef = useRef<HTMLDivElement>(null);
  const { sendMessage, connected } = useSharedWebSocket();

  const connectedRef = useRef(connected);
  connectedRef.current = connected;

  useEffect(() => {
    if (connected) {
      sendMessage({
        type: 'remote_input',
        action: 'start',
        device_id: deviceId,
      });
      setIsConnected(true);
    }
    return () => {
      if (connectedRef.current) {
        sendMessage({
          type: 'remote_input',
          action: 'stop',
          device_id: deviceId,
        });
      }
      setIsConnected(false);
    };
  }, [connected, deviceId, sendMessage]);

  interface RemoteInputEvent {
    event_type: string;
    x?: number;
    y?: number;
    key?: string;
    button?: string;
    delta?: number;
    modifiers?: string[];
  }

  const sendEvent = useCallback((event: RemoteInputEvent) => {
    if (!connected) return;
    sendMessage({
      type: 'remote_input',
      action: 'event',
      device_id: deviceId,
      ...event,
    });
  }, [connected, deviceId, sendMessage]);

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    const container = containerRef.current;
    if (!container) return;

    const rect = container.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width;
    const y = (e.clientY - rect.top) / rect.height;

    sendEvent({
      event_type: 'mouse_move',
      x,
      y,
    });
    setLastEvent(`Move: ${Math.round(x * 100)}%, ${Math.round(y * 100)}%`);
  }, [sendEvent]);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    let button: 'left' | 'right' | 'middle' = 'left';
    if (e.button === 1) button = 'middle';
    if (e.button === 2) button = 'right';

    sendEvent({
      event_type: 'mouse_click',
      button,
      x: e.nativeEvent.offsetX,
      y: e.nativeEvent.offsetY,
    });
    setLastEvent(`Click: ${button}`);
  }, [sendEvent]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    const modifiers: string[] = [];
    if (e.ctrlKey) modifiers.push('ctrl');
    if (e.shiftKey) modifiers.push('shift');
    if (e.altKey) modifiers.push('alt');
    if (e.metaKey) modifiers.push('meta');

    sendEvent({
      event_type: 'key_press',
      key: e.key,
      modifiers,
    });
    setLastEvent(`Key: ${e.key}`);
  }, [sendEvent]);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    sendEvent({
      event_type: 'scroll',
      delta: e.deltaY,
    });
    setLastEvent(`Scroll: ${e.deltaY > 0 ? 'down' : 'up'}`);
  }, [sendEvent]);

  const handleStop = () => {
    sendMessage({
      type: 'remote_input',
      action: 'stop',
      device_id: deviceId,
    });
    setIsConnected(false);
    onStop();
  };

  return (
    <div className="remote-input">
      <div className="remote-input-header">
        <h3>Remote Input: {deviceName}</h3>
        <div className="remote-input-controls">
          <span className={`status ${isConnected ? 'connected' : 'disconnected'}`}>
            {isConnected ? 'Connected' : 'Disconnected'}
          </span>
          <button onClick={handleStop} className="stop-button">
            Stop Remote Input
          </button>
        </div>
      </div>
      <div
        ref={containerRef}
        className="remote-input-content"
        onMouseMove={handleMouseMove}
        onMouseDown={handleMouseDown}
        onKeyDown={handleKeyDown}
        onWheel={handleWheel}
        tabIndex={0}
        style={{
          width: '100%',
          height: '400px',
          backgroundColor: '#1a1a2e',
          border: '1px solid #333',
          borderRadius: '8px',
          outline: 'none',
          cursor: 'crosshair',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: '#666',
          fontSize: '14px',
        }}
      >
        Click here to send input events to {deviceName}
      </div>
      {lastEvent && (
        <div className="remote-input-status">
          Last event: {lastEvent}
        </div>
      )}
    </div>
  );
}
