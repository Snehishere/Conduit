import React, { useState, useCallback } from 'react';
import { Upload } from 'lucide-react';
import { cn } from '../../lib/utils';

interface FileDropZoneProps {
  visible: boolean;
  targetDeviceId: string | null;
  targetDeviceName: string;
  onDrop: (files: FileList, targetDeviceId: string) => void;
  onClose: () => void;
}

const FileDropZone: React.FC<FileDropZoneProps> = ({
  visible,
  targetDeviceId,
  targetDeviceName,
  onDrop,
  onClose,
}) => {
  const [isDragOver, setIsDragOver] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDragOver(false);

      if (e.dataTransfer.files.length > 0 && targetDeviceId) {
        onDrop(e.dataTransfer.files, targetDeviceId);
      }
      onClose();
    },
    [onDrop, targetDeviceId, onClose]
  );

  if (!visible) return null;

  return (
    <div
      className="scrim fixed inset-0 z-[200] flex items-center justify-center transition-colors"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      onClick={onClose}
    >
      {/* Drag-over wash (separate layer so the .scrim backdrop keeps its fill) */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 -z-10 transition-colors duration-200"
        style={{ background: isDragOver ? 'rgba(0, 240, 255, 0.10)' : 'rgba(0, 240, 255, 0)' }}
      />
      <div
        className={cn(
          'surf-frost relative min-w-[320px] rounded-[24px] border-2 border-dashed px-12 py-12 text-center transition-all duration-200',
          isDragOver
            ? 'scale-105 border-success-ink'
            : 'border-line-2',
        )}
        style={{
          boxShadow:
            '0 32px 80px -24px rgba(0, 0, 0, 0.9), 0 0 0 0.5px rgba(0, 0, 0, 0.16)',
        }}
        onClick={(e) => { e.stopPropagation(); }}
      >
        <div className={cn('mb-4 transition-colors', isDragOver ? 'text-success-ink' : 'text-accent-ink')}>
          <Upload size={48} strokeWidth={1.2} className="mx-auto" />
        </div>
        <h3 className="mb-2 text-[18px] font-semibold text-ink-1">
          Send to {targetDeviceName}
        </h3>
        <p className="text-[14px] text-ink-2">Drop files here to send</p>
      </div>
    </div>
  );
};

export default FileDropZone;
