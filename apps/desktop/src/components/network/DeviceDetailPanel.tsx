import React, { useEffect, useRef } from 'react';
import {
  X,
  Monitor,
  Smartphone,
  Tablet,
  Headphones,
  Watch,
  Tv,
  Bell,
  MonitorSpeaker,
  MousePointer2,
  Send,
  Trash2,
  Wifi,
  Battery,
  BatteryLow,
  BatteryWarning,
  Shield,
  Pencil,
  type LucideIcon,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import StatusBadge from '../ui/StatusBadge';
import type { StatusType } from '../ui/StatusBadge';
import ContextMenu from '../ui/ContextMenu';

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

interface DeviceDetailPanelProps {
  device: Device;
  onClose: () => void;
  onUnpair?: (id: string) => void;
  onSendNotification?: (deviceId: string) => void;
  onStartScreenMirror?: (deviceId: string, deviceName: string) => void;
  onStartRemoteInput?: (deviceId: string, deviceName: string) => void;
}

const DEVICE_ICON_MAP: Record<string, LucideIcon | undefined> = {
  desktop: Monitor,
  phone: Smartphone,
  tablet: Tablet,
  earbuds: Headphones,
  headphones: Headphones,
  watch: Watch,
  tv: Tv,
};

const DEVICE_ICON_COLOR: Record<string, string | undefined> = {
  desktop: 'text-ink-2',
  phone: 'text-ink-2',
  tablet: 'text-ink-2',
  earbuds: 'text-ink-2',
  headphones: 'text-ink-2',
  watch: 'text-ink-2',
  tv: 'text-ink-2',
};

const getStatusBadgeType = (status: string): StatusType => {
  switch (status) {
    case 'connected': return 'connected';
    case 'paired': return 'pairing';
    default: return 'offline';
  }
};

const getBatteryIcon = (level?: number): LucideIcon => {
  if (level === undefined) return Battery;
  if (level <= 20) return BatteryWarning;
  if (level <= 50) return BatteryLow;
  return Battery;
};

const getBatteryColor = (level?: number) => {
  if (level === undefined) return 'text-ink-3';
  if (level <= 20) return 'text-danger-ink';
  if (level <= 50) return 'text-warning-ink';
  return 'text-success-ink';
};

const DeviceDetailPanel: React.FC<DeviceDetailPanelProps> = ({
  device,
  onClose,
  onUnpair,
  onSendNotification,
  onStartScreenMirror,
  onStartRemoteInput,
}) => {
  const Icon = DEVICE_ICON_MAP[device.device_type] || Smartphone;
  const iconColor = DEVICE_ICON_COLOR[device.device_type] || 'text-gray-400';
  const badgeType = getStatusBadgeType(device.status);
  const BatteryIcon = getBatteryIcon(device.battery);
  const batteryColor = getBatteryColor(device.battery);
  const isHub = device.device_type === 'desktop';
  const isMobile = device.device_type === 'phone' || device.device_type === 'tablet';

  const panelRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  // Focus management: move focus into the panel on open, restore on close.
  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    closeButtonRef.current?.focus();
    return () => {
      previousFocusRef.current?.focus();
    };
  }, []);

  // Escape closes the panel; Tab cycles focus within it (document level so it
  // works even if focus lands outside the panel element).
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === 'Tab' && panelRef.current) {
        const focusableElements = panelRef.current.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
        );
        if (focusableElements.length === 0) return;
        const firstElement = focusableElements[0];
        const lastElement = focusableElements[focusableElements.length - 1];
        const active = document.activeElement;
        const focusInside = panelRef.current.contains(active);

        if (e.shiftKey) {
          if (!focusInside || active === firstElement) {
            e.preventDefault();
            lastElement.focus();
          }
        } else {
          if (!focusInside || active === lastElement) {
            e.preventDefault();
            firstElement.focus();
          }
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => { document.removeEventListener('keydown', handleKeyDown); };
  }, [onClose]);

  const formatLastSeen = (timestamp?: number) => {
    if (!timestamp) return 'Unknown';
    const diff = Date.now() / 1000 - timestamp;
    if (diff < 60) return 'Just now';
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  };

  return (
    <>
      {/* Backdrop — real scrim: 0.45 black + blur, click to close (spec §B5) */}
      <div
        className="scrim fixed inset-0 z-[200] animate-fade-in"
        style={{ background: 'rgba(0, 0, 0, 0.45)' }}
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Panel — white frosted glass, ink tokens (spec §B4/B5) */}
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="device-detail-title"
        className="surf-frost fixed right-0 top-0 bottom-0 w-[380px] z-50 flex flex-col animate-slide-in-right"
        style={{
          borderRadius: '24px 0 0 24px',
          boxShadow: '-24px 0 64px -24px rgba(0, 0, 0, 0.85)',
        }}
      >
        {/* Header */}
        <ContextMenu items={[
          { label: 'Rename', icon: Pencil, shortcut: 'F2', onClick: () => {} },
          { label: 'Ping device', icon: Bell, shortcut: 'Ctrl+P', onClick: () => onSendNotification?.(device.id), disabled: device.status !== 'connected' || isHub },
          { type: 'separator' as const },
          { label: 'Send file', icon: Send, shortcut: 'Ctrl+S', disabled: device.status !== 'connected', onClick: () => {} },
          ...(isMobile ? [
            { label: 'Screen mirror', icon: Monitor, shortcut: 'Ctrl+M', disabled: device.status !== 'connected', onClick: () => onStartScreenMirror?.(device.id, device.name) },
          ] : []),
          { type: 'separator' as const },
          { label: 'Unpair device', icon: Trash2, danger: true, onClick: () => onUnpair?.(device.id) },
        ]}>
        <div className="p-6 pb-4 border-b border-ink-line">
          <div className="flex items-start justify-between mb-4">
            <div className="flex items-center gap-3">
              <div
                className={cn('w-12 h-12 rounded-2xl bg-fill-1 flex items-center justify-center', iconColor)}
              >
                <Icon size={24} strokeWidth={1.5} />
              </div>
              <div>
                <h3 id="device-detail-title" className="text-[16px] font-semibold text-ink-1">{device.name}</h3>
                <div className="flex items-center gap-2 mt-0.5">
                  <span className="text-[11px] text-ink-3 uppercase tracking-wider font-medium">
                    {device.os}
                  </span>
                  <span className="text-ink-3">·</span>
                  <span className="text-[11px] text-ink-3 uppercase tracking-wider font-medium">
                    {device.device_type}
                  </span>
                </div>
              </div>
            </div>
            <button
              ref={closeButtonRef}
              onClick={onClose}
              aria-label={`Close details for ${device.name}`}
              className="w-8 h-8 rounded-full bg-fill-1 flex items-center justify-center text-ink-3 hover:text-ink-1 hover:bg-fill-2 transition-colors"
            >
              <X size={16} />
            </button>
          </div>

          {/* Status row */}
          <div className="flex items-center gap-3">
            <StatusBadge status={badgeType} size="md" />
            {device.signal && (
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-full surf-clear text-ink-2">
                <Wifi size={12} />
                <span className="text-[12px]">{device.signal}</span>
              </div>
            )}
          </div>
        </div>
        </ContextMenu>

        {/* Body */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {/* Battery */}
          {device.battery !== undefined && (
            <div>
              <div className="text-[11px] text-ink-3 uppercase tracking-wider font-semibold mb-3">
                Battery
              </div>
              <div className="flex items-center gap-3">
                <div
                  className="flex-1 h-2 surf-clear rounded-full overflow-hidden"
                  role="progressbar"
                  aria-label="Battery level"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={device.battery}
                >
                  <div
                    className={cn('h-full rounded-full transition-all duration-500', {
                      'bg-danger-ink': device.battery <= 20,
                      'bg-warning-ink': device.battery > 20 && device.battery <= 50,
                      'bg-success-ink': device.battery > 50,
                    })}
                    style={{ width: `${device.battery}%` }}
                  />
                </div>
                <div className={cn('flex items-center gap-1 text-[13px] font-medium', batteryColor)}>
                  <BatteryIcon size={14} />
                  {device.battery}%
                </div>
              </div>
            </div>
          )}

          {/* Device Info */}
          <div>
            <div className="text-[11px] text-ink-3 uppercase tracking-wider font-semibold mb-3">
              Device Info
            </div>
            <div className="space-y-2">
              <div className="flex justify-between items-center py-2 border-b border-ink-line">
                <span className="text-[13px] text-ink-3">Device ID</span>
                <span className="text-[12px] text-ink-2 font-mono">{device.id.slice(0, 12)}...</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-ink-line">
                <span className="text-[13px] text-ink-3">Last Seen</span>
                <span className="text-[13px] text-ink-2">{formatLastSeen(device.last_seen)}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-ink-line">
                <span className="text-[13px] text-ink-3">Encryption</span>
                {/* Not "E2E". No message type in this build is end-to-end
                    encrypted — only call control uses the envelope, and the relay
                    terminates even that (see MESSAGE_PROTECTION in
                    useEncryption.ts). This row used to render a green "E2E" chip,
                    which contradicted the "Not E2E" status-bar chip built from the
                    same evidence. The tone is warning rather than success for the
                    same reason. */}
                <span
                  className="text-[13px] text-warning-ink flex items-center gap-1"
                  title="Traffic to this device is not end-to-end encrypted. The status bar carries the full breakdown."
                >
                  <Shield size={12} /> Not E2E
                </span>
              </div>
            </div>
          </div>

          {/* Capabilities */}
          {!isHub && device.status === 'connected' && (
            <div>
              <div className="text-[11px] text-ink-3 uppercase tracking-wider font-semibold mb-3">
                Capabilities
              </div>
              <div className="grid grid-cols-3 gap-2">
                <CapabilityPill icon={Bell} label="Ping" />
                <CapabilityPill icon={Monitor} label="Files" />
                <CapabilityPill icon={MonitorSpeaker} label="Clipboard" />
                {isMobile && <CapabilityPill icon={Monitor} label="Screen" />}
                {isMobile && <CapabilityPill icon={MousePointer2} label="Remote" />}
              </div>
            </div>
          )}
        </div>

        {/* Actions Footer */}
        {!isHub && (
          <div className="p-6 pt-4 border-t border-ink-line space-y-2">
            {onSendNotification && device.status === 'connected' && (
              <button
                onClick={() => { onSendNotification(device.id); }}
                className="w-full py-2.5 rounded-full surf-clear border border-ink-line text-ink-1 text-[13px] font-medium flex items-center justify-center gap-2 hover:bg-fill-2 transition-colors"
              >
                <Bell size={14} /> Ping Device
              </button>
            )}
            {onStartScreenMirror && device.status === 'connected' && isMobile && (
              <button
                onClick={() => { onStartScreenMirror(device.id, device.name); }}
                className="w-full py-2.5 rounded-full surf-clear border border-ink-line text-ink-1 text-[13px] font-medium flex items-center justify-center gap-2 hover:bg-fill-2 transition-colors"
              >
                <Monitor size={14} /> Screen Mirror
              </button>
            )}
            {onStartRemoteInput && device.status === 'connected' && isMobile && (
              <button
                onClick={() => { onStartRemoteInput(device.id, device.name); }}
                className="w-full py-2.5 rounded-full surf-clear border border-ink-line text-ink-1 text-[13px] font-medium flex items-center justify-center gap-2 hover:bg-fill-2 transition-colors"
              >
                <MousePointer2 size={14} /> Remote Input
              </button>
            )}
            {onStartScreenMirror && device.status === 'connected' && (
              <button
                onClick={() => {}}
                className="w-full py-2.5 rounded-full surf-clear border border-ink-line text-ink-1 text-[13px] font-medium flex items-center justify-center gap-2 hover:bg-fill-2 transition-colors"
              >
                <Send size={14} /> Send File
              </button>
            )}
            {onUnpair && (
              <button
                onClick={() => { onUnpair(device.id); }}
                className="w-full py-2.5 rounded-full bg-[rgba(185,28,28,0.06)] border border-[rgba(185,28,28,0.22)] text-danger-ink text-[13px] font-medium flex items-center justify-center gap-2 hover:bg-[rgba(185,28,28,0.10)] transition-colors"
              >
                <Trash2 size={14} /> Unpair Device
              </button>
            )}
          </div>
        )}
      </div>
    </>
  );
};

// Capability pill sub-component — .surf-clear chip on the frost panel (spec §B5)
const CapabilityPill: React.FC<{ icon: LucideIcon; label: string }> = ({ icon: CapIcon, label }) => (
  <div className="flex flex-col items-center gap-1.5 py-2.5 px-2 rounded-xl surf-clear text-ink-3">
    <CapIcon size={16} strokeWidth={1.5} />
    <span className="text-[10px] font-medium">{label}</span>
  </div>
);

export default DeviceDetailPanel;
