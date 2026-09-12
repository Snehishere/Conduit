import React, { useState } from 'react';

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
    <div className="message-input-container">
      <input
        type="text"
        className="message-input"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Type a message..."
        disabled={disabled}
      />
      <button
        className="message-send-btn"
        onClick={handleSend}
        disabled={!text.trim() || disabled}
      >
        Send
      </button>
    </div>
  );
};

export default MessageInput;
