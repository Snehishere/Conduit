import React from 'react';
import FileCard from './FileCard';
import ClipboardCard, { type ClipboardItem } from '../clipboard/ClipboardCard';
import EmptyState from '../ui/EmptyState';
import { Clipboard, Upload } from 'lucide-react';

export interface FileTransfer {
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

interface MergedMasonryProps {
  /* File transfers */
  transfers: FileTransfer[];
  activeTransferId: string | null;
  activeProgress: number;
  onAccept: (id: string) => void;
  onCancel: (id: string) => void;
  onResume: (id: string) => void;
  onOpen: (path: string) => void;
  formatFileSize: (bytes: number) => string;
  getFileIcon: (mime: string) => string;
  loading?: boolean;
  /* Clipboard history */
  items: ClipboardItem[];
  onCopy?: (id: number) => void;
  onPin?: (id: number) => void;
  onDelete?: (id: number) => void;
}

/** One card in the merged stream — ordered by `timestamp`, rendered by `node`. */
interface StreamEntry {
  timestamp: number;
  node: React.ReactNode;
}

/**
 * The single adaptive surface the "Clipboard and files window" sketch asks
 * for: clipboard cards and file-transfer cards share ONE flow, so boxes
 * interleave instead of living in two lists.
 *
 * Sizing: a CSS multi-column container (`columns-2 xl:columns-3`) with
 * `break-inside-avoid` cards. Nothing is put on a fixed row height — each box
 * is as tall as its own content (a long text snippet runs tall, an image
 * preview card grows with its preview, a document card stays compact) and the
 * browser packs the uneven heights into balanced columns.
 *
 * Ordering: transfers and clipboard items are merged into one timeline sorted
 * by `timestamp` desc, so no column reads as "the files list" or "the
 * clipboard list". Each source keeps its previous per-card ordering for ties
 * (Array.prototype.sort is stable).
 */
const MergedMasonry: React.FC<MergedMasonryProps> = ({
  transfers,
  activeTransferId,
  activeProgress,
  onAccept,
  onCancel,
  onResume,
  onOpen,
  formatFileSize,
  getFileIcon,
  loading = false,
  items,
  onCopy,
  onPin,
  onDelete,
}) => {
  const entries: StreamEntry[] = [
    ...transfers.map((transfer) => ({
      timestamp: transfer.timestamp,
      node: (
        <FileCard
          key={`transfer-${transfer.id}`}
          transfer={transfer}
          activeTransferId={activeTransferId}
          activeProgress={activeProgress}
          onAccept={onAccept}
          onCancel={onCancel}
          onResume={onResume}
          onOpen={onOpen}
          formatFileSize={formatFileSize}
          getFileIcon={getFileIcon}
        />
      ),
    })),
    ...items.map((clip, clipIndex) => ({
      timestamp: clip.timestamp,
      node: (
        <ClipboardCard
          key={`clip-${clip.id}`}
          item={clip}
          index={clipIndex}
          onCopy={onCopy}
          onPin={onPin}
          onDelete={onDelete}
        />
      ),
    })),
  ].sort((a, b) => b.timestamp - a.timestamp);

  // Empty states ride the masonry as ordinary placeholder cards (no separate
  // section, no second list): one for each side that has nothing to show.
  const placeholders: React.ReactNode[] = [];
  if (transfers.length === 0) {
    placeholders.push(
      <div key="empty-transfers" className="break-inside-avoid mb-3">
        <EmptyState
          icon={Upload}
          title="No file transfers yet"
          description="Drag files onto a device to send them"
          className="w-full max-w-none"
        />
      </div>,
    );
  }
  if (items.length === 0) {
    placeholders.push(
      <div key="empty-clipboard" className="break-inside-avoid mb-3">
        <EmptyState
          icon={Clipboard}
          title="Your clipboard is empty"
          description="Copy text or files on any connected device to see them here"
          className="w-full max-w-none"
        />
      </div>,
    );
  }

  return (
    // `loading` maps to aria-busy: offline/first-sync still renders the
    // empty-state placeholders (they are accurate, not "chatter").
    <div className="columns-2 gap-4 px-6 pb-6" aria-busy={loading}>
      {placeholders}
      {entries.map((entry) => entry.node)}
    </div>
  );
};

export default MergedMasonry;
