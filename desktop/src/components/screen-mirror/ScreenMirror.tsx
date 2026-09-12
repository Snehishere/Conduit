import React, { useState, useEffect, useRef, useCallback } from 'react';
import { useSharedWebSocket } from '../../hooks/useWebSocket';

interface ScreenMirrorProps {
  deviceId: string;
  deviceName: string;
  onStop: () => void;
}

export default function ScreenMirror({ deviceId, deviceName, onStop }: ScreenMirrorProps) {
  const [isStreaming, setIsStreaming] = useState(false);
  const [quality, setQuality] = useState<'low' | 'medium' | 'high'>('medium');
  const [fps, setFps] = useState(15);
  const [resolution, setResolution] = useState({ width: 0, height: 0 });
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { registerHandler, sendMessage, connected } = useSharedWebSocket();

  useEffect(() => {
    const unregister = registerHandler('screen_mirror', (data: any) => {
      if (data.action === 'frame') {
        handleFrame(data);
      }
    });
    return unregister;
  }, [registerHandler]);

  // Start/stop lifecycle tied to connection + device only.
  // Quality/FPS changes send a configure update without tearing down the stream.
  const connectedRef = useRef(connected);
  connectedRef.current = connected;

  useEffect(() => {
    if (connected) {
      sendMessage({
        type: 'screen_mirror',
        action: 'start',
        device_id: deviceId,
        quality,
        fps,
      });
      setIsStreaming(true);
    }
    return () => {
      if (connectedRef.current) {
        sendMessage({
          type: 'screen_mirror',
          action: 'stop',
          device_id: deviceId,
        });
      }
      setIsStreaming(false);
    };
  }, [connected, deviceId, sendMessage]);

  useEffect(() => {
    if (connected && isStreaming) {
      sendMessage({
        type: 'screen_mirror',
        action: 'configure',
        device_id: deviceId,
        quality,
        fps,
      });
    }
  }, [quality, fps, connected, isStreaming, deviceId, sendMessage]);

  const handleFrame = useCallback((data: any) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    if (canvas.width !== data.width || canvas.height !== data.height) {
      canvas.width = data.width;
      canvas.height = data.height;
      setResolution({ width: data.width, height: data.height });
    }

    const img = new Image();
    img.onload = () => {
      ctx.drawImage(img, 0, 0);
    };
    img.src = `data:image/jpeg;base64,${data.data}`;
  }, []);

  const handleStop = () => {
    sendMessage({
      type: 'screen_mirror',
      action: 'stop',
      device_id: deviceId,
    });
    setIsStreaming(false);
    onStop();
  };

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width;
    const y = (e.clientY - rect.top) / rect.height;

    sendMessage({
      type: 'screen_mirror',
      action: 'touch',
      device_id: deviceId,
      x,
      y,
      action_type: 'tap',
    });
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    const modifiers: string[] = [];
    if (e.ctrlKey) modifiers.push('ctrl');
    if (e.shiftKey) modifiers.push('shift');
    if (e.altKey) modifiers.push('alt');
    if (e.metaKey) modifiers.push('meta');

    sendMessage({
      type: 'screen_mirror',
      action: 'key',
      device_id: deviceId,
      key: e.key,
      modifiers,
    });
  };

  return (
    <div className="screen-mirror">
      <div className="screen-mirror-header">
        <h3>Screen Mirror: {deviceName}</h3>
        <div className="screen-mirror-controls">
          <select
            value={quality}
            onChange={(e) => setQuality(e.target.value as any)}
            disabled={isStreaming}
          >
            <option value="low">Low (480p)</option>
            <option value="medium">Medium (720p)</option>
            <option value="high">High (1080p)</option>
          </select>
          <select
            value={fps}
            onChange={(e) => setFps(parseInt(e.target.value))}
            disabled={isStreaming}
          >
            <option value="15">15 FPS</option>
            <option value="30">30 FPS</option>
            <option value="60">60 FPS</option>
          </select>
          <button onClick={handleStop} className="stop-button">
            Stop Mirroring
          </button>
        </div>
      </div>
      <div className="screen-mirror-content">
        <canvas
          ref={canvasRef}
          onClick={handleCanvasClick}
          onKeyDown={handleKeyDown}
          tabIndex={0}
          style={{
            maxWidth: '100%',
            maxHeight: '100%',
            objectFit: 'contain',
            cursor: 'pointer',
          }}
        />
        {resolution.width > 0 && (
          <div className="resolution-info">
            {resolution.width} x {resolution.height}
          </div>
        )}
      </div>
    </div>
  );
}
