import React from 'react';
import TransferProgress from './TransferProgress';
import { IconFolder } from '../icons';

interface FileTransfer {
  id: string;
  name: string;
  size: number;
  mime: string;
  from_device: string;
  to_device: string;
  status: string;
  chunks_received: number;
  total_chunks: number;
  saved_path: string | null;
  timestamp: number;
}

interface FileExplorerProps {
  transfers: FileTransfer[];
  activeTransferId: string | null;
  activeProgress: number;
  onAccept: (id: string) => void;
  onCancel: (id: string) => void;
  onResume: (id: string) => void;
  onOpen: (path: string) => void;
  formatFileSize: (bytes: number) => string;
  getFileIcon: (mime: string) => string;
}

const FileExplorer: React.FC<FileExplorerProps> = ({
  transfers,
  activeTransferId,
  activeProgress,
  onAccept,
  onCancel,
  onResume,
  onOpen,
  formatFileSize,
  getFileIcon,
}) => {
  const [filter, setFilter] = React.useState<'all' | 'active' | 'completed'>('all');

  const filtered = transfers.filter((t) => {
    if (filter === 'active') return t.status === 'pending' || t.status === 'transferring';
    if (filter === 'completed') return t.status === 'complete';
    return true;
  });

  const activeCount = transfers.filter(
    (t) => t.status === 'pending' || t.status === 'transferring'
  ).length;

  return (
    <div className="file-explorer">
      <div className="file-header">
        <h2>
          <IconFolder size={20} /> Files
        </h2>
        <div className="file-actions">
          <button
            className={`filter-btn ${filter === 'all' ? 'active' : ''}`}
            onClick={() => setFilter('all')}
          >
            All ({transfers.length})
          </button>
          <button
            className={`filter-btn ${filter === 'active' ? 'active' : ''}`}
            onClick={() => setFilter('active')}
          >
            Active ({activeCount})
          </button>
          <button
            className={`filter-btn ${filter === 'completed' ? 'active' : ''}`}
            onClick={() => setFilter('completed')}
          >
            Done
          </button>
        </div>
      </div>

      <div className="file-list">
        {filtered.length === 0 ? (
          <div className="file-empty">
            <div className="file-empty-icon">
              <IconFolder size={48} />
            </div>
            <p>No file transfers yet</p>
            <p className="file-empty-hint">Drag files onto a device in the spider web to send them</p>
          </div>
        ) : (
          filtered.map((transfer) => (
            <TransferProgress
              key={transfer.id}
              name={transfer.name}
              size={transfer.size}
              status={transfer.status}
              progress={
                transfer.status === 'complete'
                  ? 100
                  : transfer.status === 'transferring' && activeTransferId === transfer.id
                  ? activeProgress
                  : transfer.total_chunks > 0
                  ? (transfer.chunks_received / transfer.total_chunks) * 100
                  : 0
              }
              mime={transfer.mime}
              onAccept={
                transfer.status === 'pending'
                  ? () => onAccept(transfer.id)
                  : undefined
              }
              onCancel={
                transfer.status === 'pending' || transfer.status === 'transferring'
                  ? () => onCancel(transfer.id)
                  : undefined
              }
              onResume={
                transfer.status === 'cancelled'
                  ? () => onResume(transfer.id)
                  : undefined
              }
              onOpen={
                transfer.status === 'complete' && transfer.saved_path
                  ? () => onOpen(transfer.saved_path!)
                  : undefined
              }
            />
          ))
        )}
      </div>
    </div>
  );
};

export default FileExplorer;
