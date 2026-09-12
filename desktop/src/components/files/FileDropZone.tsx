import React, { useState, useCallback } from 'react';
import { IconFolder } from '../icons';

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
      className={`file-drop-overlay ${isDragOver ? 'drag-over' : ''}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      onClick={onClose}
    >
      <div className="file-drop-content" onClick={(e) => e.stopPropagation()}>
        <div className={`file-drop-icon ${isDragOver ? 'bounce' : ''}`}>
          <IconFolder size={64} />
        </div>
        <h3>Send to {targetDeviceName}</h3>
        <p>Drop files here to send</p>
      </div>
    </div>
  );
};

export default FileDropZone;
