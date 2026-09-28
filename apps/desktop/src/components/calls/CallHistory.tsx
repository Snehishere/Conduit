import React, { useState } from 'react';
import { Call } from '../../hooks/useCalls';
import { Phone, PhoneIncoming, PhoneOutgoing, PhoneMissed, Forward } from 'lucide-react';
import { cn } from '../../lib/utils';
import { formatTime } from '../../lib/time';
import EmptyState from '../ui/EmptyState';
import { SkeletonList } from '../ui/Skeleton';

interface Device {
  id: string;
  name: string;
  status: string;
}

interface CallHistoryProps {
  history: Call[];
  getCallerName: (call: Call) => string;
  getCallDuration: (call: Call) => string;
  timeAgo: (ts: number) => string;
  loading?: boolean;
  devices: Device[];
  onForward: (callId: string, toDeviceId: string) => void;
}

// Ink-safe semantic colors for rows sitting on the white frost panel (spec §B7).
const getCallTypeInfo = (call: Call) => {
  if (call.direction === 'incoming') {
    return {
      icon: PhoneIncoming,
      color: 'text-success-ink',
      bg: 'bg-success-ink/10',
      label: 'Incoming',
    };
  }
  if (call.direction === 'outgoing') {
    return {
      icon: PhoneOutgoing,
      color: 'text-info-ink',
      bg: 'bg-info-ink/10',
      label: 'Outgoing',
    };
  }
  return {
    icon: PhoneMissed,
    color: 'text-danger-ink',
    bg: 'bg-danger-ink/10',
    label: 'Missed',
  };
};

const CallHistory: React.FC<CallHistoryProps> = ({
  history,
  getCallerName,
  getCallDuration,
  loading = false,
  devices,
  onForward,
}) => {
  const [forwardingCallId, setForwardingCallId] = useState<string | null>(null);

  const connectedDevices = devices.filter((d) => d.status === 'connected');

  if (history.length === 0) {
    if (loading) {
      return (
        <div className="h-full flex flex-col items-center justify-center gap-4" style={{ padding: 24 }}>
          <SkeletonList count={3} className="w-full max-w-lg" />
        </div>
      );
    }
    return (
      <div className="h-full flex flex-col items-center justify-center gap-4">
        <EmptyState
          icon={Phone}
          title="No call history"
          description="Calls from your devices will appear here"
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col">
      {history.map((call, index) => {
        const typeInfo = getCallTypeInfo(call);
        const TypeIcon = typeInfo.icon;

        return (
          <div
            key={call.call_id}
            className="group flex items-center gap-3.5 rounded-xl transition-colors hover:bg-fill-1"
            style={{
              padding: '12px 20px',
              ...(index < history.length - 1
                ? { borderBottom: '1px solid var(--line-1)' }
                : null),
            }}
          >
            {/* Avatar */}
            <div className={cn(
              'w-10 h-10 rounded-full flex items-center justify-center text-[14px] font-semibold shrink-0 border',
              typeInfo.bg, typeInfo.color, 'border-transparent'
            )}>
              {getCallerName(call).charAt(0).toUpperCase()}
            </div>

            {/* Info */}
            <div className="flex-1 min-w-0">
              <div className="text-[14px] font-medium text-ink-1 truncate">
                {getCallerName(call)}
              </div>
              <div className="flex items-center gap-1.5 text-[12px] text-ink-3">
                <TypeIcon size={11} className={typeInfo.color} />
                <span>{typeInfo.label}</span>
                <span>·</span>
                <span>{call.end_time ? formatTime(call.end_time) : 'In progress'}</span>
              </div>
            </div>

            {/* Duration */}
            <div className="text-[13px] text-ink-2 font-mono shrink-0">
              {getCallDuration(call)}
            </div>

            {/* Forward button for missed calls */}
            {call.direction === 'missed' && connectedDevices.length > 0 && (
              <div className="relative shrink-0">
                <button
                  onClick={() => { setForwardingCallId(forwardingCallId === call.call_id ? null : call.call_id); }}
                  className="rounded-lg text-ink-3 transition-colors hover:bg-fill-2! hover:text-ink-1"
                  style={{ padding: 6 }}
                  title="Forward to phone"
                  aria-label="Forward missed call to phone"
                >
                  <Forward size={14} />
                </button>
                {forwardingCallId === call.call_id && (
                  <div className="surf-frost absolute right-0 top-full z-10 min-w-[160px] overflow-hidden rounded-xl" style={{ marginTop: 4 }}>
                    {connectedDevices.map((device) => (
                      <button
                        key={device.id}
                        onClick={() => {
                          onForward(call.call_id, device.id);
                          setForwardingCallId(null);
                        }}
                        className="flex w-full items-center gap-2 rounded-lg text-left text-[12px] text-ink-1 transition-colors hover:bg-fill-1!"
                        style={{ padding: '8px 12px' }}
                      >
                        <Phone size={10} className="text-accent-ink" />
                        {device.name}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
};

export default CallHistory;
