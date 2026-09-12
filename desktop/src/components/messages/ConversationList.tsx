import React from 'react';
import { SmsThread } from '../../hooks/useSms';
import { IconChat } from '../icons';

interface ConversationListProps {
  threads: SmsThread[];
  selectedThread: string | null;
  onSelectThread: (id: string) => void;
  timeAgo: (ts: number) => string;
}

const ConversationList: React.FC<ConversationListProps> = ({
  threads,
  selectedThread,
  onSelectThread,
  timeAgo,
}) => {
  if (threads.length === 0) {
    return (
      <div className="sms-empty">
        <div className="sms-empty-icon">
          <IconChat size={48} />
        </div>
        <p>No conversations</p>
        <p className="sms-empty-hint">SMS messages from your phone will appear here</p>
      </div>
    );
  }

  return (
    <div className="conversation-list">
      {threads.map((thread) => (
        <div
          key={thread.thread_id}
          className={`conversation-item ${selectedThread === thread.thread_id ? 'active' : ''} ${thread.unread_count > 0 ? 'unread' : ''}`}
          onClick={() => onSelectThread(thread.thread_id)}
        >
          <div className="conversation-avatar">
            {thread.name ? thread.name.charAt(0).toUpperCase() : '?'}
          </div>
          <div className="conversation-info">
            <div className="conversation-header">
              <span className="conversation-name">
                {thread.name || thread.address}
              </span>
              <span className="conversation-time">
                {timeAgo(thread.timestamp)}
              </span>
            </div>
            <div className="conversation-snippet">
              {thread.snippet.length > 50
                ? thread.snippet.slice(0, 50) + '...'
                : thread.snippet}
            </div>
          </div>
          {thread.unread_count > 0 && (
            <span className="conversation-badge">{thread.unread_count}</span>
          )}
        </div>
      ))}
    </div>
  );
};

export default ConversationList;
