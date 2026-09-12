import React, { useRef, useEffect } from 'react';
import { SmsThread, SmsMessage } from '../../hooks/useSms';
import MessageInput from './MessageInput';
import { IconChevronLeft } from '../icons';

interface MessageThreadProps {
  thread: SmsThread;
  onSend: (to: string, body: string) => void;
  onBack: () => void;
  timeAgo: (ts: number) => string;
}

function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

const MessageThread: React.FC<MessageThreadProps> = ({
  thread,
  onSend,
  onBack,
  timeAgo,
}) => {
  const messagesEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [thread.messages]);

  const handleSend = (body: string) => {
    onSend(thread.address, body);
  };

  return (
    <div className="message-thread">
      <div className="thread-header">
        <button className="thread-back" onClick={onBack}>
          <IconChevronLeft size={18} />
        </button>
        <div className="thread-contact">
          <div className="thread-avatar">
            {thread.name ? thread.name.charAt(0).toUpperCase() : '?'}
          </div>
          <div>
            <div className="thread-name">{thread.name || thread.address}</div>
            <div className="thread-address">{thread.address}</div>
          </div>
        </div>
      </div>

      <div className="thread-messages">
        {thread.messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} timeAgo={timeAgo} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      <MessageInput onSend={handleSend} />
    </div>
  );
};

const MessageBubble: React.FC<{ message: SmsMessage; timeAgo: (ts: number) => string }> = ({
  message,
  timeAgo,
}) => {
  return (
    <div className={`message-bubble ${message.is_outgoing ? 'outgoing' : 'incoming'}`}>
      <div className="message-body">{message.body}</div>
      <div className="message-meta">
        <span className="message-time">{formatTime(message.timestamp)}</span>
        {message.is_outgoing && (
          <span className="message-status">
            {message.read ? '✓✓' : '✓'}
          </span>
        )}
      </div>
    </div>
  );
};

export default MessageThread;
