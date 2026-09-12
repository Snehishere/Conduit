import React, { useState } from 'react';
import NotificationCard from './NotificationCard';
import { IconBell, IconSend } from '../icons';

interface Notification {
  id: string;
  device_id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  actions?: string;
  dismissed: boolean;
}

interface NotificationPanelProps {
  notifications: Notification[];
  onDismiss: (id: string) => void;
  onReply: (id: string, text: string) => void;
}

const NotificationPanel: React.FC<NotificationPanelProps> = ({
  notifications,
  onDismiss,
  onReply,
}) => {
  const [filter, setFilter] = useState<'all' | 'unread'>('all');

  const filteredNotifications = notifications.filter((n) => {
    if (filter === 'unread') return !n.dismissed;
    return true;
  });

  const markAllRead = () => {
    notifications.forEach((n) => {
      if (!n.dismissed) onDismiss(n.id);
    });
  };

  return (
    <div className="notification-panel">
      <div className="notification-header">
        <h2>
          <IconBell size={20} /> Notifications
        </h2>
        <div className="notification-actions">
          <button
            className={`filter-btn ${filter === 'all' ? 'active' : ''}`}
            onClick={() => setFilter('all')}
          >
            All
          </button>
          <button
            className={`filter-btn ${filter === 'unread' ? 'active' : ''}`}
            onClick={() => setFilter('unread')}
          >
            Unread
          </button>
          <button className="mark-all-btn" onClick={markAllRead}>
            Mark All
          </button>
        </div>
      </div>
      <div className="notification-list">
        {filteredNotifications.length === 0 ? (
          <div className="notification-empty">
            <div className="notification-empty-icon">
              <IconBell size={48} />
            </div>
            <p>No notifications</p>
          </div>
        ) : (
          filteredNotifications.map((notification) => (
            <NotificationCard
              key={notification.id}
              notification={notification}
              onDismiss={onDismiss}
              onReply={onReply}
            />
          ))
        )}
      </div>
    </div>
  );
};

export default NotificationPanel;
