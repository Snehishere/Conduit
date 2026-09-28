import React from 'react';
import { motion } from 'motion/react';
import {
  FileText, Image, Music, Film, Archive, Code,
  Check, Loader2, X, Play, RotateCcw, FolderOpen, Copy,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { timeAgo } from '../../lib/time';
import ContextMenu, { ContextMenuItem } from '../ui/ContextMenu';

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

interface FileCardProps {
  transfer: FileTransfer;
  activeTransferId: string | null;
  activeProgress: number;
  onAccept: (id: string) => void;
  onCancel: (id: string) => void;
  onResume: (id: string) => void;
  onOpen: (path: string) => void;
  formatFileSize: (bytes: number) => string;
  getFileIcon: (mime: string) => string;
}

function getFileCategory(mime: string): { icon: typeof FileText; color: string; bg: string } {
  if (mime.startsWith('image/')) return { icon: Image, color: 'text-violet-400', bg: 'bg-violet-500/8' };
  if (mime.startsWith('audio/')) return { icon: Music, color: 'text-amber-400', bg: 'bg-amber-500/8' };
  if (mime.startsWith('video/')) return { icon: Film, color: 'text-rose-400', bg: 'bg-rose-500/8' };
  if (mime.includes('zip') || mime.includes('archive') || mime.includes('tar') || mime.includes('gz')) return { icon: Archive, color: 'text-blue-400', bg: 'bg-blue-500/8' };
  if (mime.includes('json') || mime.includes('javascript') || mime.includes('python') || mime.includes('typescript')) return { icon: Code, color: 'text-amber-400', bg: 'bg-amber-500/8' };
  if (mime === 'application/pdf') return { icon: FileText, color: 'text-red-400', bg: 'bg-red-500/8' };
  if (mime.includes('spreadsheet') || mime.includes('csv') || mime.includes('excel') || mime.includes('xlsx') || mime.includes('xls')) return { icon: FileText, color: 'text-green-400', bg: 'bg-green-500/8' };
  if (mime.includes('presentation') || mime.includes('pptx') || mime.includes('ppt')) return { icon: FileText, color: 'text-orange-400', bg: 'bg-orange-500/8' };
  if (mime.includes('document') || mime.includes('msword') || mime.includes('docx') || mime.includes('doc')) return { icon: FileText, color: 'text-blue-300', bg: 'bg-blue-500/8' };
  if (mime.includes('executable') || mime.includes('x-dosexec') || mime.includes('x-apple') || mime.includes('.exe') || mime.includes('.dmg') || mime.includes('.app')) return { icon: FileText, color: 'text-purple-400', bg: 'bg-purple-500/8' };
  return { icon: FileText, color: 'text-blue-400', bg: 'bg-blue-500/8' };
}

const FileCard: React.FC<FileCardProps> = ({
  transfer,
  activeTransferId,
  activeProgress,
  onAccept,
  onCancel,
  onResume,
  onOpen,
  formatFileSize,
}) => {
  const { icon: FileIcon, color, bg } = getFileCategory(transfer.mime);
  const isImage = transfer.mime.startsWith('image/');
  const isVideo = transfer.mime.startsWith('video/');
  const isAudio = transfer.mime.startsWith('audio/');
  // Captured for narrowing inside closures (transfer.saved_path is `string | null`).
  const savedPath = transfer.status === 'complete' ? transfer.saved_path : null;
  // `navigator.clipboard` is typed non-null but can be undefined in insecure
  // contexts — widen so the optional chain is type-honest.
  const clipboard = navigator.clipboard as Clipboard | undefined;

  const contextItems: ContextMenuItem[] = [
    ...(savedPath
      ? [
          { label: 'Open', icon: Play, shortcut: 'Enter', onClick: () => { onOpen(savedPath); } },
          { label: 'Show in folder', icon: FolderOpen, onClick: () => { onOpen(savedPath); } },
        ]
      : []),
    ...(transfer.status === 'pending'
      ? [
          { label: 'Accept', icon: Play, onClick: () => { onAccept(transfer.id); } },
          { label: 'Decline', icon: X, danger: true, onClick: () => { onCancel(transfer.id); } },
        ]
      : []),
    ...(transfer.status === 'transferring'
      ? [{ label: 'Cancel', icon: X, danger: true, onClick: () => { onCancel(transfer.id); } }]
      : []),
    ...(transfer.status === 'cancelled'
      ? [{ label: 'Resume', icon: RotateCcw, onClick: () => { onResume(transfer.id); } }]
      : []),
    { type: 'separator' as const },
    { label: 'Copy file name', icon: Copy, onClick: () => { void clipboard?.writeText(transfer.name); } },
  ];

  const progress =
    transfer.status === 'complete' ? 100
    : transfer.status === 'transferring' && activeTransferId === transfer.id ? activeProgress
    : transfer.total_chunks > 0
    ? (transfer.chunks_received / transfer.total_chunks) * 100
    : 0;

  const statusColor =
    transfer.status === 'complete' ? 'text-success-ink'
    : transfer.status === 'transferring' ? 'text-accent-ink'
    : transfer.status === 'cancelled' ? 'text-danger-ink'
    : 'text-ink-3';

  return (
    <ContextMenu items={contextItems}>
    <motion.div
      className={cn(
        'break-inside-avoid mb-3 rounded-2xl border p-4 transition-all cursor-pointer group',
        'texture-metal surf-clear border-line-1 hover:bg-fill-2 hover:border-line-2',
      )}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25 }}
      onClick={() => {
        if (transfer.status === 'complete' && transfer.saved_path) {
          onOpen(transfer.saved_path);
        }
      }}
    >
      {/* Media gets a larger type-specific preview area; documents stay compact. */}
      {isImage || isVideo ? (
        <div className={cn(
          'mb-3 flex items-center justify-center rounded-xl border border-line-1',
          isImage ? 'aspect-[4/3]' : 'aspect-video',
          bg,
        )} aria-label={`${isImage ? 'Image' : 'Video'} file preview`}>
          <FileIcon size={30} strokeWidth={1.4} className={color} />
        </div>
      ) : isAudio ? (
        <div className={cn('mb-3 h-14 rounded-xl flex items-center gap-1 px-4', bg)} aria-label="Audio file">
          <FileIcon size={20} strokeWidth={1.5} className={cn(color, 'mr-2')} />
          {[18, 30, 22, 38, 26, 34, 16, 28, 20, 36, 24, 14].map((height, index) => (
            <span key={index} className={cn('w-1 rounded-full opacity-70', color)} style={{ height, backgroundColor: 'currentColor' }} />
          ))}
        </div>
      ) : (
        <div className={cn('w-10 h-10 rounded-xl flex items-center justify-center mb-3', bg)}>
          <FileIcon size={20} strokeWidth={1.5} className={color} />
        </div>
      )}

      {/* Filename */}
        <h3 className="text-[13px] font-medium text-ink-1 whitespace-normal break-words line-clamp-3 mb-1">
          {transfer.name}
        </h3>

      {/* Meta */}
      <div className="flex items-center gap-2 text-[11px] text-ink-3 mb-3">
        <span>{formatFileSize(transfer.size)}</span>
        <span className="w-0.5 h-0.5 rounded-full bg-ink-3" />
        <span>{timeAgo(transfer.timestamp)}</span>
      </div>

      {/* Progress bar (for active transfers) */}
      {(transfer.status === 'transferring' || transfer.status === 'pending') && (
        <div className="w-full h-1 rounded-full bg-fill-2 mb-3 overflow-hidden">
          <motion.div
            className="h-full rounded-full bg-accent"
            style={{ width: `${progress}%`, background: 'var(--accent-ink)' }}
            transition={{ duration: 0.3 }}
          />
        </div>
      )}

      {/* Footer */}
      <div className="flex items-center justify-between">
        <span className={cn('text-[11px] font-medium capitalize', statusColor)}>
          {transfer.status === 'transferring' && (
            <Loader2 size={10} className="inline animate-spin mr-1" />
          )}
          {transfer.status === 'complete' && (
            <Check size={10} className="inline mr-1" />
          )}
          {transfer.status}
        </span>

        {/* Actions */}
        <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
          {transfer.status === 'pending' && (
            <>
              <button
                onClick={(e) => { e.stopPropagation(); onAccept(transfer.id); }}
                className="p-1.5 rounded-lg bg-success-dim text-success-ink hover:bg-success/20 transition-colors"
              >
                <Play size={11} />
              </button>
              <button
                onClick={(e) => { e.stopPropagation(); onCancel(transfer.id); }}
                className="p-1.5 rounded-lg bg-danger-dim text-danger-ink hover:bg-danger/20 transition-colors"
              >
                <X size={11} />
              </button>
            </>
          )}
          {transfer.status === 'transferring' && (
            <button
              onClick={(e) => { e.stopPropagation(); onCancel(transfer.id); }}
              className="p-1.5 rounded-lg bg-danger-dim text-danger-ink hover:bg-danger/20 transition-colors"
            >
              <X size={11} />
            </button>
          )}
          {transfer.status === 'cancelled' && (
            <button
              onClick={(e) => { e.stopPropagation(); onResume(transfer.id); }}
              className="p-1.5 rounded-lg bg-accent-dim-ink text-accent-ink hover:bg-accent-ink/20 transition-colors"
            >
              <RotateCcw size={11} />
            </button>
          )}
        </div>
      </div>
    </motion.div>
    </ContextMenu>
  );
};

export default FileCard;
