import React from 'react';
import { Folder } from 'lucide-react';
import MergedMasonry from './MergedMasonry';

type FilesScreenProps = React.ComponentProps<typeof MergedMasonry> & {
  /** Clear the entire clipboard history (header action). */
  onClearAll?: () => void;
};

/**
 * Files + Clipboard in ONE frosted panel — no tabs, no split lists.
 *
 * The shell owns the panel and the single header row; everything below it is
 * one scroll owner hosting `MergedMasonry`, where clipboard cards and file
 * transfer cards interleave in a content-sized (masonry) flow, so box heights
 * follow the text/file type instead of a fixed grid.
 */
const FilesScreen: React.FC<FilesScreenProps> = ({
  transfers,
  activeTransferId,
  activeProgress,
  onAccept,
  onCancel,
  onResume,
  onOpen,
  formatFileSize,
  getFileIcon,
  loading,
  items,
  onCopy,
  onPin,
  onDelete,
  onClearAll,
}) => {
  const combinedCount = transfers.length + items.length;

  return (
    <div className="flex h-full min-h-0 flex-col" style={{ padding: 28 }}>
      <div className="surf-frost flex min-h-0 flex-1 flex-col overflow-hidden rounded-[24px]">
        {/* One heading row — heading + combined count chip. No tabs. */}
        <div
          className="flex shrink-0 items-center justify-between gap-3"
          style={{ padding: '20px 24px 16px' }}
        >
          <div className="flex items-center gap-3">
            <h1 className="flex items-center gap-2 text-[20px] font-semibold text-ink-1">
              <Folder size={20} strokeWidth={1.8} aria-hidden="true" />
              Files
            </h1>
            {combinedCount > 0 && (
              <span className="rounded-full border border-line-1 bg-fill-1 px-2 py-0.5 text-[11px] text-ink-3">
                {combinedCount} items
              </span>
            )}
          </div>
          {onClearAll !== undefined && items.length > 0 && (
            <button
              type="button"
              onClick={onClearAll}
              className="shrink-0 rounded-lg px-3 py-1.5 text-[12px] font-medium text-accent-ink transition-colors hover:bg-accent-dim-ink"
            >
              Clear clipboard
            </button>
          )}
        </div>

        {/* Single scroll owner for the merged surface */}
        <div id="files-screen-panel" className="min-h-0 flex-1 overflow-y-auto">
          <MergedMasonry
            transfers={transfers}
            activeTransferId={activeTransferId}
            activeProgress={activeProgress}
            onAccept={onAccept}
            onCancel={onCancel}
            onResume={onResume}
            onOpen={onOpen}
            formatFileSize={formatFileSize}
            getFileIcon={getFileIcon}
            loading={loading}
            items={items}
            onCopy={onCopy}
            onPin={onPin}
            onDelete={onDelete}
          />
        </div>
      </div>
    </div>
  );
};

export default FilesScreen;
