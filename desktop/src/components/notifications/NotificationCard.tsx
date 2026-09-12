import React, { useState } from 'react';
import { formatTime } from '../../lib/utils';
import { IconSend } from '../icons';

interface NotificationCardProps {
  notification: {
    id: string;
    device_id: string;
    app: string;
    title: string;
    body: string;
    timestamp: number;
    dismissed: boolean;
  };
  onDismiss: (id: string) => void;
  onReply: (id: string, text: string) => void;
}

const NotificationCard: React.FC<NotificationCardProps> = ({ notification, onDismiss, onReply }) => {
  const [replyText, setReplyText] = useState('');
  const [showReply, setShowReply] = useState(false);

  const handleReply = () => {
    if (replyText.trim()) {
      onReply(notification.id, replyText);
      setReplyText('');
      setShowReply(false);
    }
  };

  return (
    <div className={`notification-card ${notification.dismissed ? 'dismissed' : ''}`}>
      <div className="notification-card-header">
        <span className="notification-app">{notification.app}</span>
        <span className="notification-time">{formatTime(notification.timestamp)}</span>
      </div>
      <div className="notification-card-title">{notification.title}</div>
      <div className="notification-card-body">{notification.body}</div>
      <div className="notification-card-actions">
        <button className="notification-action-btn" onClick={() => setShowReply(!showReply)}>Reply</button>
        <button className="notification-action-btn dismiss" onClick={() => onDismiss(notification.id)}>Dismiss</button>
      </div>
      {showReply && (
        <div className="notification-reply">
          <input
            type="text"
            value={replyText}
            onChange={(e) => setReplyText(e.target.value)}
            placeholder="Type a reply..."
            onKeyDown={(e) => e.key === 'Enter' && handleReply()}
          />
          <button className="send-btn" onClick={handleReply}>
            <IconSend size={14} />
          </button>
        </div>
      )}
    </div>
  );
};

export default NotificationCard;
