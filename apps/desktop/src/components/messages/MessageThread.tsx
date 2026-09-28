import React, { useRef, useEffect } from 'react';
import { SmsThread, SmsMessage } from '../../hooks/useSms';
import MessageInput from './MessageInput';
import { ArrowLeft, Copy } from 'lucide-react';
import { cn } from '../../lib/utils';
import { formatTime } from '../../lib/time';
import ContextMenu from '../ui/ContextMenu';

interface MessageThreadProps {
  thread: SmsThread;
  onSend: (to: string, body: string) => void;
  onBack: () => void;
  timeAgo: (ts: number) => string;
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
    <div className="surf-frost m-4 flex h-[calc(100%_-_2rem)] flex-col overflow-hidden rounded-[24px]">
      {/* Header */}
      <div className="flex items-center gap-3 border-b border-ink-line px-5 py-3.5 shrink-0">
        <button
          className="w-8 h-8 rounded-full bg-fill-1 flex items-center justify-center text-ink-3 hover:text-ink-1 transition-colors md:hidden"
          onClick={onBack}
        >
          <ArrowLeft size={16} />
        </button>
        <div className="w-9 h-9 rounded-full bg-fill-1 border border-line-1 flex items-center justify-center text-[14px] font-semibold text-ink-2 shrink-0">
          {thread.name ? thread.name.charAt(0).toUpperCase() : '?'}
        </div>
        <div>
          <div className="text-[14px] font-medium text-ink-1">{thread.name || thread.address}</div>
          <div className="text-[11px] text-ink-3">{thread.address}</div>
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-5 py-4 space-y-3">
        {thread.messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} timeAgo={timeAgo} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <MessageInput onSend={handleSend} />
    </div>
  );
};

const MessageBubble: React.FC<{ message: SmsMessage; timeAgo: (ts: number) => string }> = ({
  message,
}) => {
  const isOut = message.is_outgoing;
  // `navigator.clipboard` is typed non-null but can be undefined in insecure
  // contexts — widen so the optional chain is type-honest.
  const clipboard = navigator.clipboard as Clipboard | undefined;

  const contextItems = [
    { label: 'Copy', icon: Copy, onClick: () => { void clipboard?.writeText(message.body); } },
  ];

  return (
    <ContextMenu items={contextItems}>
    <div className={cn('flex', isOut ? 'justify-end' : 'justify-start')}>
      <div className={cn(
        'max-w-[70%] px-3.5 py-2.5 rounded-2xl text-[13px] leading-relaxed',
        isOut
          ? 'bg-accent-dim-ink text-ink-1 rounded-br-md border border-[rgba(14,116,144,0.20)]'
          : 'bg-fill-1 text-ink-1 rounded-bl-md border border-line-1'
      )}>
        <p className="mb-1">{message.body}</p>
        <div className="flex gap-1.5 items-center justify-end text-[10px] text-ink-3 opacity-70">
          <span>{formatTime(message.timestamp)}</span>
          {isOut && (
            <span>{message.read ? '✓✓' : '✓'}</span>
          )}
        </div>
      </div>
    </div>
    </ContextMenu>
  );
};

export default MessageThread;
