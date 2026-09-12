import React from 'react';
import { getDeviceIconComponent } from '../../lib/utils';
import {
  IconX,
  IconSignal,
  IconBell,
  IconMonitor,
  IconCursor,
  IconTrash,
} from '../icons';

interface Device {
  id: string;
  name: string;
  device_type: string;
  os: string;
  battery?: number;
  signal?: string;
  status: string;
  last_seen?: number;
}

interface DeviceDetailProps {
  device: Device;
  onClose: () => void;
  onUnpair?: (id: string) => void;
  onSendNotification?: (deviceId: string) => void;
  onStartScreenMirror?: (deviceId: string, deviceName: string) => void;
  onStartRemoteInput?: (deviceId: string, deviceName: string) => void;
}

const DeviceDetail: React.FC<DeviceDetailProps> = ({
  device,
  onClose,
  onUnpair,
  onSendNotification,
  onStartScreenMirror,
  onStartRemoteInput,
}) => {
  const isHub = device.device_type === 'desktop';
  const DeviceIcon = getDeviceIconComponent(device.device_type);

  return (
    <div className="device-detail-card" onClick={(e) => e.stopPropagation()}>
      <div className="device-detail-header">
        <div className="device-detail-title">
          <span className="device-detail-icon">
            <DeviceIcon size={24} />
            {device.status === 'connected' && (
              <span
                style={{
                  position: 'absolute',
                  bottom: 2,
                  right: 2,
                  width: 8,
                  height: 8,
                  borderRadius: '50%',
                  background: 'var(--online-dot)',
                  border: '2px solid var(--bg-secondary)',
                }}
              />
            )}
          </span>
          <div>
            <h3>{device.name}</h3>
            <span className="device-detail-sub">
              {device.os.toUpperCase()} · {device.device_type}
            </span>
          </div>
        </div>
        <button className="device-detail-close" onClick={onClose}>
          <IconX size={16} />
        </button>
      </div>

      <div className="device-detail-body">
        <div className="device-stat-row">
          <span className="stat-label">Status</span>
          <span className={`stat-value status-badge status-${device.status}`}>
            <span className="status-dot" />
            {device.status.charAt(0).toUpperCase() + device.status.slice(1)}
          </span>
        </div>

        {device.battery !== undefined && (
          <div className="device-stat-row">
            <span className="stat-label">Battery</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div className="battery-bar-container">
                <div
                  className="battery-bar-fill"
                  style={{
                    width: `${device.battery}%`,
                  }}
                />
              </div>
              <span className="battery-text" style={{ position: 'static' }}>{device.battery}%</span>
            </div>
          </div>
        )}

        {device.signal && (
          <div className="device-stat-row">
            <span className="stat-label">Signal</span>
            <span className="stat-value">
              <IconSignal size={14} /> {device.signal}
            </span>
          </div>
        )}

        <div className="device-stat-row">
          <span className="stat-label">Device ID</span>
          <span className="stat-value code-font">{device.id.slice(0, 16)}...</span>
        </div>
      </div>

      {!isHub && (
        <div className="device-detail-actions">
          {onSendNotification && device.status === 'connected' && (
            <button
              className="action-btn primary"
              onClick={() => onSendNotification(device.id)}
            >
              <IconBell size={14} /> Ping
            </button>
          )}
          {onStartScreenMirror && device.status === 'connected' && (device.device_type === 'phone' || device.device_type === 'tablet') && (
            <button
              className="action-btn primary"
              onClick={() => onStartScreenMirror(device.id, device.name)}
            >
              <IconMonitor size={14} /> Screen Mirror
            </button>
          )}
          {onStartRemoteInput && device.status === 'connected' && (device.device_type === 'phone' || device.device_type === 'tablet') && (
            <button
              className="action-btn primary"
              onClick={() => onStartRemoteInput(device.id, device.name)}
            >
              <IconCursor size={14} /> Remote Input
            </button>
          )}
          {onUnpair && (
            <button
              className="action-btn danger"
              onClick={() => onUnpair(device.id)}
            >
              <IconTrash size={14} /> Unpair
            </button>
          )}
        </div>
      )}
    </div>
  );
};

export default DeviceDetail;
