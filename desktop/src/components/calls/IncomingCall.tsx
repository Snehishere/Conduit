import React from 'react';
import { Call } from '../../hooks/useCalls';
import { IconPhone, IconX } from '../icons';

interface IncomingCallProps {
  call: Call;
  onAnswer: () => void;
  onReject: () => void;
  onForward: () => void;
  getCallerName: (call: Call) => string;
}

const IncomingCall: React.FC<IncomingCallProps> = ({
  call,
  onAnswer,
  onReject,
  onForward,
  getCallerName,
}) => {
  return (
    <div className="incoming-call-overlay">
      <div className="incoming-call-modal">
        <div className="incoming-call-avatar">
          {getCallerName(call).charAt(0).toUpperCase()}
        </div>
        <div className="incoming-call-info">
          <h2>{getCallerName(call)}</h2>
          <p>{call.number}</p>
          <p className="incoming-call-status">Incoming call...</p>
        </div>
        <div className="incoming-call-actions">
          <button className="call-btn call-reject" onClick={onReject}>
            <IconX size={24} />
          </button>
          <button className="call-btn call-answer" onClick={onAnswer}>
            <IconPhone size={24} />
          </button>
        </div>
        <button className="call-forward-btn" onClick={onForward}>
          Forward to Desktop
        </button>
      </div>
    </div>
  );
};

export default IncomingCall;
