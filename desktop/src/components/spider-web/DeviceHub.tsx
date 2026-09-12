import React from 'react';
import SpiderWeb from './SpiderWeb';
import {
  IconPlus,
  IconDevicePhone,
  IconDeviceDesktop,
  IconDeviceEarbuds,
  IconWatch,
  IconShield,
  IconLink,
  IconChat,
  IconVolume,
  IconSignal,
} from '../icons';

interface Device {
  id: string;
  name: string;
  device_type: string;
  os: string;
  battery?: number;
  signal?: string;
  status: string;
}

interface DeviceHubProps {
  devices: Device[];
  selectedDevice: string | null;
  onSelectDevice: (id: string | null) => void;
  onPairDevice: () => void;
  onUnpairDevice?: (id: string) => void;
  onPingDevice?: (id: string) => void;
  onStartScreenMirror?: (deviceId: string, deviceName: string) => void;
  onStartRemoteInput?: (deviceId: string, deviceName: string) => void;
}

const DeviceHub: React.FC<DeviceHubProps> = ({
  devices,
  selectedDevice,
  onSelectDevice,
  onPairDevice,
  onUnpairDevice,
  onPingDevice,
  onStartScreenMirror,
  onStartRemoteInput,
}) => {
  const connectedDevices = devices.filter((d) => d.status === 'connected');
  const hubDevice = devices.find((d) => d.device_type === 'desktop');
  const phoneDevice = devices.find((d) => d.device_type === 'phone');
  const earbudsDevice = devices.find((d) => d.device_type === 'earbuds');
  const watchDevice = devices.find((d) => d.device_type === 'watch');

  return (
    <div className="device-hub-layout">
      <div className="device-hub-web">
        <SpiderWeb
          devices={devices}
          selectedDevice={selectedDevice}
          onSelectDevice={onSelectDevice}
          onPairDevice={onPairDevice}
          onUnpairDevice={onUnpairDevice}
          onPingDevice={onPingDevice}
          onStartScreenMirror={onStartScreenMirror}
          onStartRemoteInput={onStartRemoteInput}
        />
        <div className="ecosystem-status">
          <span className="ecosystem-status-dot" />
          ECOSYSTEM STATUS: {connectedDevices.length} DEVICE{connectedDevices.length !== 1 ? 'S' : ''} ACTIVE
          {hubDevice && ` (Local Hub: ${hubDevice.name})`}
        </div>
      </div>

      <div className="device-hub-panels">
        {/* Add New Device */}
        <div className="add-device-panel" onClick={onPairDevice}>
          <IconPlus size={16} />
          ADD NEW DEVICE
        </div>

        {/* Messaging Sync */}
        <div className="feature-panel">
          <div className="feature-panel-header">
            <div className="feature-panel-title">
              <span className="feature-panel-title-icon"><IconChat size={16} /></span>
              Messaging Sync
            </div>
            <div className="toggle-switch on" />
          </div>
          <div className="feature-panel-body">
            {phoneDevice ? (
              <>
                <div className="sync-status">
                  <span className="sync-dot active" />
                  Synchronizing with {phoneDevice.name}
                </div>
                <div className="feature-panel-meta">
                  Recent messaging thread &mdash; Th...
                </div>
              </>
            ) : (
              <div className="feature-panel-meta">No phone connected</div>
            )}
          </div>
        </div>

        {/* Audio Source & Autoswitch */}
        <div className="feature-panel">
          <div className="feature-panel-header">
            <div className="feature-panel-title">
              <span className="feature-panel-title-icon"><IconVolume size={16} /></span>
              Audio Source & Autoswitch
            </div>
            <div className="toggle-switch on" />
          </div>
          <div className="feature-panel-body">
            <div className="device-icons-row">
              {earbudsDevice && (
                <div className="device-mini-icon"><IconDeviceEarbuds size={16} /></div>
              )}
              {watchDevice && (
                <div className="device-mini-icon"><IconWatch size={16} /></div>
              )}
              {hubDevice && (
                <div className="device-mini-icon"><IconDeviceDesktop size={16} /></div>
              )}
            </div>
            <div className="feature-panel-meta">
              Active Source: Laptop Speakers
            </div>
          </div>
        </div>

        {/* Shared Clipboard */}
        <div className="feature-panel">
          <div className="feature-panel-header">
            <div className="feature-panel-title">
              <span className="feature-panel-title-icon"><IconLink size={16} /></span>
              Shared Clipboard
            </div>
            <div className="toggle-switch on" />
          </div>
          <div className="feature-panel-body">
            <div className="feature-panel-meta">
              Clipboard &mdash; Text, Files, Links Supported
            </div>
          </div>
        </div>

        {/* System Secured */}
        <div className="feature-panel">
          <div className="feature-panel-header">
            <div className="feature-panel-title">
              <span className="secured-icon"><IconShield size={16} /></span>
              System Secured
            </div>
          </div>
          <div className="feature-panel-body">
            <div className="secured-badge">
              <IconShield size={14} />
              E2E Encrypted over Local Network
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default DeviceHub;
