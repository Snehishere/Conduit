import React from 'react';
import { Call } from '../../hooks/useCalls';
import { IconPhone } from '../icons';

interface CallHistoryProps {
  history: Call[];
  getCallerName: (call: Call) => string;
  getCallDuration: (call: Call) => string;
  timeAgo: (ts: number) => string;
}

function formatTime(timestamp: number): string {
  // Timestamps are Unix seconds — convert to ms for Date.
  const date = new Date(timestamp * 1000);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

const CallHistory: React.FC<CallHistoryProps> = ({
  history,
  getCallerName,
  getCallDuration,
  timeAgo,
}) => {
  if (history.length === 0) {
    return (
      <div className="call-history-empty">
        <div className="call-history-icon">
          <IconPhone size={48} />
        </div>
        <p>No call history</p>
        <p className="call-history-hint">Calls from your devices will appear here</p>
      </div>
    );
  }

  return (
    <div className="call-history">
      {history.map((call) => (
        <div key={call.call_id} className="call-history-item">
          <div className="call-history-avatar">
            {getCallerName(call).charAt(0).toUpperCase()}
          </div>
          <div className="call-history-info">
            <div className="call-history-name">{getCallerName(call)}</div>
            <div className="call-history-meta">
              {call.end_time ? formatTime(call.end_time) : 'In progress'} · {getCallDuration(call)}
            </div>
          </div>
          <div className="call-history-status">
            <IconPhone size={14} />
          </div>
        </div>
      ))}
    </div>
  );
};

export default CallHistory;
