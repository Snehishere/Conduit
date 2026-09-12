import React from 'react';
import { IconFolder, IconDevicePhone, IconDeviceHeadphones, IconDeviceTV, IconDeviceDesktop } from '../icons';

interface TransferProgressProps {
  name: string;
  size: number;
  status: string;
  progress: number;
  mime: string;
  onAccept?: () => void;
  onCancel?: () => void;
  onResume?: () => void;
  onOpen?: () => void;
}

function getFileIconComponent(mime: string) {
  if (mime.startsWith('image/')) return IconDevicePhone;
  if (mime.startsWith('video/')) return IconDeviceTV;
  if (mime.startsWith('audio/')) return IconDeviceHeadphones;
  if (mime.includes('pdf')) return IconDeviceDesktop;
  return IconFolder;
}

const TransferProgress: React.FC<TransferProgressProps> = ({
  name,
  size,
  status,
  progress,
  mime,
  onAccept,
  onCancel,
  onResume,
  onOpen,
}) => {
  const formatSize = (bytes: number): string => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
  };

  const FileIcon = getFileIconComponent(mime);

  return (
    <div className={`transfer-item transfer-${status}`}>
      <div className="transfer-icon">
        <FileIcon size={20} />
      </div>
      <div className="transfer-info">
        <div className="transfer-name">{name}</div>
        <div className="transfer-meta">{formatSize(size)} · {status}</div>
        {status !== 'complete' && status !== 'cancelled' && (
          <div className="transfer-bar">
            <div className="transfer-bar-fill" style={{ width: `${progress}%` }} />
          </div>
        )}
      </div>
      <div className="transfer-actions">
        {status === 'complete' && onOpen && (
          <button className="transfer-action-btn open" onClick={onOpen}>Open</button>
        )}
        {status === 'pending' && onAccept && (
          <button className="transfer-action-btn accept" onClick={onAccept}>Accept</button>
        )}
        {status === 'cancelled' && onResume && (
          <button className="transfer-action-btn resume" onClick={onResume}>Resume</button>
        )}
        {(status === 'transferring' || status === 'pending') && onCancel && (
          <button className="transfer-action-btn cancel" onClick={onCancel}>Cancel</button>
        )}
      </div>
    </div>
  );
};

export default TransferProgress;
