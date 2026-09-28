import React from 'react';
import { ArrowLeft, ArrowRight, Phone } from 'lucide-react';
import NotificationPanel from '../notifications/NotificationPanel';
import CallHistory from './CallHistory';
import IncomingCall from './IncomingCall';
import type { Call } from '../../hooks/useCalls';

interface Device {
  id: string;
  name: string;
  status: string;
}

interface CallsNotificationsSplitProps {
  notifications: React.ComponentProps<typeof NotificationPanel>['notifications'];
  onDismissNotification: (id: string) => void;
  onReplyNotification: (id: string, text: string) => void;
  onMarkReadNotification?: (id: string) => void;
  callHistory: Call[];
  activeCall: Call | null;
  getCallerName: (call: Call) => string;
  getCallDuration: (call: Call) => string;
  timeAgo: (timestamp: number) => string;
  devices: Device[];
  onForward: (callId: string, deviceId: string) => void;
  onAnswer: () => void;
  onReject: () => void;
  loading: boolean;
}

/**
 * Two frosted panels side by side with a 16px starfield gap between them —
 * the gap IS the divider (no borders, spec §B5). Screen root stays transparent.
 */
const CallsNotificationsSplit: React.FC<CallsNotificationsSplitProps> = ({
  notifications,
  onDismissNotification,
  onReplyNotification,
  onMarkReadNotification,
  callHistory,
  activeCall,
  getCallerName,
  getCallDuration,
  timeAgo,
  devices,
  onForward,
  onAnswer,
  onReject,
  loading,
}) => (
  <div
    className="grid h-full min-h-0 grid-cols-1 grid-rows-2 overflow-hidden lg:grid-cols-[minmax(0,1.1fr)_minmax(320px,0.9fr)] lg:grid-rows-1"
    style={{ gap: 16, padding: 28 }}
  >
    <section className="min-h-0 min-w-0 overflow-hidden">
      <NotificationPanel
        notifications={notifications}
        onDismiss={onDismissNotification}
        onReply={onReplyNotification}
        onMarkRead={onMarkReadNotification}
        loading={loading}
        embedded
      />
    </section>
    <section className="surf-frost flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[24px]">
      {/* Right column header — normal document flow, well below the search row */}
      <header
        className="flex shrink-0 items-center justify-between"
        style={{ padding: '20px 24px 16px' }}
      >
        <div className="flex items-center gap-2">
          <Phone size={16} className="text-accent-ink" />
          <h2 className="text-sm text-ink-1" style={{ fontWeight: 650 }}>Calls</h2>
        </div>
        {activeCall?.status === 'ringing' && (
          <span
            className="rounded-full text-[11px] font-medium text-danger-ink"
            style={{ background: 'rgba(185, 28, 28, 0.10)', padding: '4px 10px' }}
          >
            Incoming call
          </span>
        )}
      </header>
      {activeCall && (activeCall.status === 'ringing' || activeCall.status === 'active') ? (
        <div className="min-h-0 flex-1 overflow-hidden" style={{ padding: 16 }}>
          <IncomingCall
            call={activeCall}
            onAnswer={onAnswer}
            onReject={onReject}
            onForward={onForward}
            getCallerName={getCallerName}
            getCallDuration={getCallDuration}
            devices={devices}
            embedded
          />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto" style={{ padding: 20 }}>
          <CallHistory
            history={callHistory}
            getCallerName={getCallerName}
            getCallDuration={getCallDuration}
            timeAgo={timeAgo}
            loading={loading}
            devices={devices}
            onForward={onForward}
          />
          {/* Incoming-call gestures. These apply to the call window that replaces
              this list when a call arrives — not to the history rows above, which
              have no drag handler. The previous wording ("Call popup gestures /
              Swipe left to reject / Swipe right to accept") sat directly under
              the history in two tinted chips that read as per-row actions for rows
              that cannot be swiped, so the copy now says which surface it
              describes. Terminology is aligned with IncomingCall's own labels
              ("Decline" / "Accept"). */}
          <div className="surf-clear" style={{ marginTop: 20, padding: 16, borderRadius: 16 }}>
            <h3 className="text-xs font-semibold text-ink-1">Answering an incoming call</h3>
            <div className="grid grid-cols-2 gap-3 text-[11px] text-ink-2" style={{ marginTop: 12 }}>
              <div
                className="flex items-center gap-2 rounded-lg"
                style={{ background: 'rgba(185, 28, 28, 0.10)', padding: 10 }}
              >
                <ArrowLeft size={15} className="text-danger-ink" />
                <span>Left to decline</span>
              </div>
              <div
                className="flex items-center gap-2 rounded-lg"
                style={{ background: 'rgba(21, 128, 61, 0.10)', padding: 10 }}
              >
                <ArrowRight size={15} className="text-success-ink" />
                <span>Right to accept</span>
              </div>
            </div>
            <p className="text-[10px] text-ink-3" style={{ marginTop: 8 }}>
              When a call arrives, the call window can be declined or accepted by
              dragging it, or with a two-finger trackpad swipe. These gestures do
              not apply to the call history above.
            </p>
          </div>
        </div>
      )}
    </section>
  </div>
);

export default CallsNotificationsSplit;
