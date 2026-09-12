import { useState, useCallback, useEffect } from 'react';

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

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

function formatFileSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

function getFileIcon(mime: string): string {
  if (mime.startsWith('image/')) return '🖼️';
  if (mime.startsWith('video/')) return '🎬';
  if (mime.startsWith('audio/')) return '🎵';
  if (mime.includes('pdf')) return '📄';
  if (mime.includes('zip') || mime.includes('rar') || mime.includes('tar')) return '📦';
  if (mime.includes('text') || mime.includes('json')) return '📝';
  return '📁';
}

export function useFiles() {
  const [transfers, setTransfers] = useState<FileTransfer[]>([]);
  const [activeTransferId, setActiveTransferId] = useState<string | null>(null);
  const [activeProgress, setActiveProgress] = useState(0);

  const loadTransfers = useCallback(async () => {
    try {
      const result = await invoke<FileTransfer[]>('get_file_transfers', { limit: 50 });
      setTransfers(result);
    } catch (err) {
      console.error('Failed to load file transfers:', err);
    }
  }, []);

  const sendFile = useCallback(async (targetDeviceId: string, filePath: string) => {
    try {
      const result = await invoke<{
        transfer_id: string;
        name: string;
        size: number;
        mime: string;
        total_chunks: number;
      }>('send_file', { targetDeviceId, filePath });

      setActiveTransferId(result.transfer_id);
      setActiveProgress(0);

      // Add to local list immediately
      setTransfers((prev) => [
        {
          id: result.transfer_id,
          name: result.name,
          size: result.size,
          mime: result.mime,
          from_device: 'local',
          to_device: targetDeviceId,
          status: 'transferring',
          chunks_received: 0,
          total_chunks: result.total_chunks,
          saved_path: null,
          timestamp: Math.floor(Date.now() / 1000),
        },
        ...prev,
      ]);

      return result.transfer_id;
    } catch (err) {
      console.error('Failed to send file:', err);
      return null;
    }
  }, []);

  const acceptTransfer = useCallback(async (id: string) => {
    try {
      await invoke('accept_file_transfer', { id });
      setTransfers((prev) =>
        prev.map((t) => (t.id === id ? { ...t, status: 'transferring' } : t))
      );
    } catch (err) {
      console.error('Failed to accept transfer:', err);
    }
  }, []);

  const cancelTransfer = useCallback(async (id: string) => {
    try {
      await invoke('cancel_file_transfer', { id });
      setTransfers((prev) =>
        prev.map((t) => (t.id === id ? { ...t, status: 'cancelled' } : t))
      );
      setActiveTransferId((prev) => (prev === id ? null : prev));
      setActiveProgress(0);
    } catch (err) {
      console.error('Failed to cancel transfer:', err);
    }
  }, []);

  const resumeTransfer = useCallback(async (id: string) => {
    try {
      const transfer = transfers.find((t) => t.id === id);
      if (!transfer) return;
      const result = await invoke<{ transfer_id: string; chunks_loaded: number }>(
        'resume_file_transfer',
        {
          id,
          name: transfer.name,
          size: transfer.size,
          mime: transfer.mime,
          fromDevice: transfer.from_device,
        }
      );
      setTransfers((prev) =>
        prev.map((t) => (t.id === id ? { ...t, status: 'transferring', chunks_received: result.chunks_loaded } : t))
      );
    } catch (err) {
      console.error('Failed to resume transfer:', err);
    }
  }, [transfers]);

  const openFile = useCallback(async (path: string) => {
    try {
      const { open } = await import('@tauri-apps/plugin-shell');
      await open(path);
    } catch (err) {
      console.error('Failed to open file:', err);
    }
  }, []);

  // Listen for WS file messages
  const handleFileMessage = useCallback((data: Record<string, unknown>) => {
    const action = data.action as string;
    const id = data.id as string;

    switch (action) {
      case 'request': {
        const name = data.name as string;
        const size = data.size as number;
        const mime = data.mime as string;
        const from = data.from as string;

        setTransfers((prev) => [
          {
            id,
            name,
            size,
            mime,
            from_device: from,
            to_device: 'local',
            status: 'pending',
            chunks_received: 0,
            total_chunks: Math.ceil(size / (64 * 1024)),
            saved_path: null,
            timestamp: Math.floor(Date.now() / 1000),
          },
          ...prev,
        ]);
        break;
      }
      case 'progress': {
        const percent = data.percent as number;
        setActiveTransferId(id);
        setActiveProgress(percent);
        setTransfers((prev) =>
          prev.map((t) =>
            t.id === id
              ? { ...t, status: 'transferring', chunks_received: Math.floor((percent / 100) * t.total_chunks) }
              : t
          )
        );
        break;
      }
      case 'complete': {
        const path = data.path as string;
        setTransfers((prev) =>
          prev.map((t) =>
            t.id === id
              ? { ...t, status: 'complete', saved_path: path, chunks_received: t.total_chunks }
              : t
          )
        );
        setActiveTransferId((prev) => (prev === id ? null : prev));
        setActiveProgress(100);
        // Reset progress bar for the next transfer.
        setTimeout(() => setActiveProgress(0), 2000);
        break;
      }
      case 'cancel': {
        setTransfers((prev) =>
          prev.map((t) => (t.id === id ? { ...t, status: 'cancelled' } : t))
        );
        setActiveTransferId((prev) => (prev === id ? null : prev));
        setActiveProgress(0);
        break;
      }
      case 'resume_ack': {
        const chunksLoaded = data.chunks_loaded as number;
        setTransfers((prev) =>
          prev.map((t) =>
            t.id === id
              ? { ...t, status: 'transferring', chunks_received: chunksLoaded }
              : t
          )
        );
        break;
      }
    }
  }, []);

  useEffect(() => {
    loadTransfers();
    const interval = setInterval(loadTransfers, 10000);
    return () => clearInterval(interval);
  }, [loadTransfers]);

  return {
    transfers,
    activeTransferId,
    activeProgress,
    sendFile,
    acceptTransfer,
    cancelTransfer,
    resumeTransfer,
    openFile,
    handleFileMessage,
    formatFileSize,
    getFileIcon,
  };
}
