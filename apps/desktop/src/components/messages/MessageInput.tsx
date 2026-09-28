import React, { useState } from 'react';
import { Send } from 'lucide-react';
import { cn } from '../../lib/utils';

interface MessageInputProps {
  onSend: (body: string) => void;
  disabled?: boolean;
}

const MessageInput: React.FC<MessageInputProps> = ({ onSend, disabled }) => {
  const [text, setText] = useState('');

  const handleSend = () => {
    if (text.trim()) {
      onSend(text.trim());
      setText('');
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="flex gap-2.5 border-t border-ink-line px-5 py-3.5 shrink-0">
      <input
        type="text"
        className={cn(
          'flex-1 px-4 py-2.5 rounded-full bg-fill-1 border border-line-1 text-[13px] text-ink-1',
          'placeholder:text-ink-3 focus:border-accent-ink transition-colors'
        )}
        value={text}
        onChange={(e) => { setText(e.target.value); }}
        onKeyDown={handleKeyDown}
        placeholder="Type a message..."
        disabled={disabled}
      />
      <button
        className={cn(
          'w-9 h-9 rounded-full flex items-center justify-center transition-colors shrink-0',
          text.trim()
            ? 'btn-solid'
            : 'bg-fill-1 text-ink-3 cursor-not-allowed'
        )}
        onClick={handleSend}
        disabled={!text.trim() || disabled}
      >
        <Send size={15} />
      </button>
    </div>
  );
};

export default MessageInput;
