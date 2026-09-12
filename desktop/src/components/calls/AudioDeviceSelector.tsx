import React from 'react';
import { AudioDevice } from '../../hooks/useCalls';
import { IconDevicePhone, IconDeviceHeadphones, IconBluetooth, IconVolume } from '../icons';

interface AudioDeviceSelectorProps {
  devices: AudioDevice[];
  currentRoute: 'phone' | 'desktop' | 'bluetooth';
  onSelectRoute: (route: 'phone' | 'desktop' | 'bluetooth') => void;
  isStreaming?: boolean;
  onStartStream?: () => void;
  onStopStream?: () => void;
}

function getAudioDeviceIcon(type: AudioDevice['type']) {
  switch (type) {
    case 'speaker': return IconVolume;
    case 'headphones': return IconDeviceHeadphones;
    case 'bluetooth': return IconBluetooth;
    case 'earpiece': return IconDevicePhone;
    default: return IconVolume;
  }
}

const AudioDeviceSelector: React.FC<AudioDeviceSelectorProps> = ({
  devices,
  currentRoute,
  onSelectRoute,
  isStreaming = false,
  onStartStream,
  onStopStream,
}) => {
  const routes: { id: 'phone' | 'desktop' | 'bluetooth'; label: string; icon: React.FC<{ size?: number }> }[] = [
    { id: 'phone', label: 'Phone', icon: IconDevicePhone },
    { id: 'desktop', label: 'Desktop', icon: IconDeviceHeadphones },
    { id: 'bluetooth', label: 'Bluetooth', icon: IconBluetooth },
  ];

  return (
    <div className="audio-device-selector">
      <h3 className="audio-selector-title">Audio Output</h3>
      <div className="audio-routes">
        {routes.map((route) => {
          const RouteIcon = route.icon;
          return (
            <button
              key={route.id}
              className={`audio-route-btn ${currentRoute === route.id ? 'active' : ''}`}
              onClick={() => onSelectRoute(route.id)}
            >
              <span className="audio-route-icon">
                <RouteIcon size={20} />
              </span>
              <span className="audio-route-label">{route.label}</span>
            </button>
          );
        })}
      </div>

      <h3 className="audio-selector-title">Audio Streaming</h3>
      <div className="audio-stream-controls">
        <button
          className={`audio-stream-btn ${isStreaming ? 'active' : ''}`}
          onClick={() => isStreaming ? onStopStream?.() : onStartStream?.()}
        >
          <span className="audio-route-icon">
            {isStreaming ? '⏹' : '🎙️'}
          </span>
          <span className="audio-route-label">{isStreaming ? 'Stop Stream' : 'Start Stream'}</span>
        </button>
      </div>

      {devices.length > 0 && (
        <>
          <h3 className="audio-devices-title">Available Devices</h3>
          <div className="audio-devices-list">
            {devices.map((device) => {
              const DeviceIcon = getAudioDeviceIcon(device.type);
              return (
                <div
                  key={device.id}
                  className={`audio-device-item ${device.active ? 'active' : ''}`}
                >
                  <span className="audio-device-icon">
                    <DeviceIcon size={16} />
                  </span>
                  <span className="audio-device-name">{device.name}</span>
                  {device.active && <span className="audio-device-badge">Active</span>}
                </div>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
};

export default AudioDeviceSelector;
