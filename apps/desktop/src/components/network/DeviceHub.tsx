import React, { useState } from 'react';
import { Plus, Smartphone } from 'lucide-react';
import DeviceBubbleCanvas from './DeviceBubbleCanvas';
import DeviceDetailPanel from './DeviceDetailPanel';
import StatusBadge from '../ui/StatusBadge';
import EmptyState from '../ui/EmptyState';
import { SkeletonList } from '../ui/Skeleton';

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
  onNavigate?: (view: string) => void;
  /** Called when Shift+clicking a device bubble — triggers audio streaming */
  onShiftClickDevice?: (deviceId: string) => void;
  loading?: boolean;
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
  onShiftClickDevice,
  loading = false,
}) => {
  const [detailDevice, setDetailDevice] = useState<Device | null>(null);
  const connectedDevices = devices.filter((d) => d.status === 'connected');

  const handleBubbleClick = (device: Device) => {
    setDetailDevice(device);
  };

  const handleCloseDetail = () => {
    setDetailDevice(null);
  };

  if (connectedDevices.length === 0) {
    if (loading) {
      return (
        <div className="h-full flex items-center justify-center p-6" style={{ background: 'transparent' }}>
          <SkeletonList count={3} className="w-full max-w-md" />
        </div>
      );
    }
    return (
      <div className="h-full flex items-center justify-center" style={{ background: 'transparent' }}>
        <EmptyState
          icon={Smartphone}
          title="No devices paired"
          description="Connect your first device to start syncing across platforms"
          action={{ label: 'Pair device', onClick: onPairDevice }}
        />
      </div>
    );
  }

  const bubbleData = connectedDevices.map((d) => ({
    id: d.id,
    name: d.name,
    deviceType: d.device_type,
    status: d.status,
    battery: d.battery,
    os: d.os,
  }));

  return (
    <div className="h-full flex flex-col relative" style={{ background: 'transparent' }}>
      <div className="flex-1 relative">
        <DeviceBubbleCanvas
          devices={bubbleData}
          selectedDevice={selectedDevice}
          onSelectDevice={onSelectDevice}
          onBubbleClick={(bd) => {
            const orig = devices.find((d) => d.id === bd.id);
            if (orig) handleBubbleClick(orig);
          }}
          onShiftClick={onShiftClickDevice}
        />

        {/* Header */}
          <div className="absolute top-0 left-0 right-0 px-6 pt-4 pb-8 pointer-events-none z-30"
            style={{ background: 'linear-gradient(180deg, var(--bg-0) 0%, transparent 100%)' }}
          >
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <h1 style={{ fontSize: 14, fontWeight: 500, color: 'var(--text-1)' }}>
                Devices
              </h1>
              <StatusBadge status="connected" label={`${connectedDevices.length} online`} size="sm" />
            </div>
            <button
              onClick={onPairDevice}
              data-tutorial="pair-button"
              className="flex items-center gap-1 pointer-events-auto transition-colors"
              style={{
                padding: '4px 10px',
                borderRadius: 6,
                fontSize: 11,
                fontWeight: 500,
                background: 'var(--bg-2)',
                border: '1px solid var(--border)',
                color: 'var(--text-2)',
              }}
            >
              <Plus size={12} strokeWidth={1.5} /> Pair
            </button>
          </div>
        </div>
      </div>

      {detailDevice && (
        <DeviceDetailPanel
          device={detailDevice}
          onClose={handleCloseDetail}
          onUnpair={onUnpairDevice}
          onSendNotification={onPingDevice}
          onStartScreenMirror={onStartScreenMirror}
          onStartRemoteInput={onStartRemoteInput}
        />
      )}
    </div>
  );
};

export default DeviceHub;
