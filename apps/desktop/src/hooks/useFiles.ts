// @ts-nocheck
import { useState, useCallback, useEffect } from 'react';
import { showError, showSuccess } from '../lib/toast';
import { invoke, openDownloadedFile } from '../lib/tauri';
import type { WebSocketMessage } from '../types/websocket';

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

/** Placeholder the UI uses for "this device" on inbound transfers. */
const LOCAL_DEVICE = 'local';

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
      console.error('[useFiles] Failed to load file transfers:', err);
      showError('Failed to load file transfers');
    }
  }, []);

  const sendFile = useCallback(async (targetDeviceId: string, filePath: string) => {
    try {
      // The user chose this path in the native file dialog (or dropped the file
      // on the window). Record that choice *before* asking for the transfer:
      // `send_file` refuses any path that is neither inside the configured
      // download folder nor explicitly approved here, so it can never be
      // pointed at, say, ~/.ssh/id_rsa. The backend canonicalises and refuses
      // symlinks.
      await invoke('approve_files_for_send', { paths: [filePath] });

      const result = await invoke<{
        transfer_id: string;
        name: string;
        size: number;
        mime: string;
        total_chunks: number;
      }>('send_file', { targetDeviceId, filePath });

      setActiveTransferId(result.transfer_id);
      setActiveProgress(0);

      // Add to local list immediately, then prune old terminal transfers
      setTransfers((prev) => {
        const updated = [
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
        ];
        // Keep at most 100 transfers to prevent unbounded memory growth
        if (updated.length > 100) {
          return updated.slice(0, 100);
        }
        return updated;
      });

      showSuccess(`File transfer started: ${result.name}`);
      return result.transfer_id;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[useFiles] Failed to send file:', err);
      showError(`Failed to send file: ${message}`);
      return null;
    }
  }, []);

  /**
   * Device ids of the two ends of a transfer, for accept/cancel.
   *
   * The backend refuses to send an accept/cancel frame when it cannot work out
   * a peer rather than falling back to a broadcast. These are candidates only —
   * the backend picks whichever id is *not* this device, so a peer cannot
   * redirect the frame at a third device by lying about `from`.
   */
  const routeHint = useCallback(
    (id: string) => {
      const transfer = transfers.find((t) => t.id === id);
      return {
        fromDevice: transfer?.from_device ?? null,
        toDevice: transfer?.to_device ?? null,
      };
    },
    [transfers],
  );

  const acceptTransfer = useCallback(async (id: string) => {
    try {
      const hint = routeHint(id);
      await invoke('accept_file_transfer', {
        id,
        fromDevice: hint.fromDevice,
        toDevice: hint.toDevice,
      });
      setTransfers((prev) =>
        prev.map((t) => (t.id === id ? { ...t, status: 'transferring' } : t))
      );
      showSuccess('Transfer accepted');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[useFiles] Failed to accept transfer:', err);
      showError(`Failed to accept transfer: ${message}`);
    }
  }, [routeHint]);

  const cancelTransfer = useCallback(async (id: string) => {
    try {
      const hint = routeHint(id);
      await invoke('cancel_file_transfer', {
        id,
        fromDevice: hint.fromDevice,
        toDevice: hint.toDevice,
      });
      setTransfers((prev) =>
        prev.map((t) => (t.id === id ? { ...t, status: 'cancelled' } : t))
      );
      setActiveTransferId((prev) => (prev === id ? null : prev));
      setActiveProgress(0);
      showSuccess('Transfer cancelled');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[useFiles] Failed to cancel transfer:', err);
      showError(`Failed to cancel transfer: ${message}`);
    }
  }, [routeHint]);

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
      showSuccess('Transfer resumed');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[useFiles] Failed to resume transfer:', err);
      showError(`Failed to resume transfer: ${message}`);
    }
  }, [transfers]);

  /**
   * Open a file this device received.
   *
   * Takes whatever the file card has — a `saved_path` — and turns it into a
   * *transfer id*, then calls `open_downloaded_file`. The Rust command looks
   * the path up from local state (the `file_transfers` row and the engine's own
   * record of what it wrote) and refuses anything outside the download folder.
   *
   * The old code called the `shell` plugin's `open(path)` with the string
   * directly. That string arrives from a `file/complete` frame, whose `path`
   * is chosen by the *sending* device, so any paired peer could decide what
   * this machine launches — a `\\attacker\share\file` UNC path discloses the
   * Windows NTLM hash, and whether it succeeded was a side channel the peer
   * could read. There is deliberately no shell capability in
   * `capabilities/default.json` any more.
   */
  const openFile = useCallback(async (savedPathOrId: string) => {
    try {
      const target =
        transfers.find((t) => t.id === savedPathOrId) ??
        transfers.find((t) => t.saved_path === savedPathOrId);
      if (!target) {
        showError('That file is no longer available to open');
        return;
      }
      await openDownloadedFile(target.id);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[useFiles] Failed to open file:', err);
      showError(`Failed to open file: ${message}`);
    }
  }, [transfers]);

  // Listen for WS file messages
  const handleFileMessage = useCallback((msg: WebSocketMessage) => {
    if (msg.type !== 'file') return;

    switch (msg.action) {
      case 'request': {
        const { id, name, size, mime, from } = msg;

        setTransfers((prev) => {
          const updated = [
            {
              id,
              name,
              size,
              mime,
              from_device: from,
              to_device: LOCAL_DEVICE,
              status: 'pending',
              chunks_received: 0,
              total_chunks: Math.ceil(size / (64 * 1024)),
              saved_path: null,
              timestamp: Math.floor(Date.now() / 1000),
            },
            ...prev,
          ];
          if (updated.length > 100) {
            return updated.slice(0, 100);
          }
          return updated;
        });
        break;
      }
      case 'chunk': {
        // Chunks belong to the transfer engine, not to React state. A sender
        // streams 64 KB frames; parsing each one here only produced work for
        // the UI, and because `send_file` used to *broadcast* its own chunk
        // frames the desktop used to parse the payload of every file it sent
        // itself. Nothing to do — but the case has to exist explicitly so a
        // future reader does not "fix" the gap by re-adding shell/open.
        break;
      }
      case 'progress': {
        const { id, percent } = msg;
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
        const { id, path } = msg;
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
        setTimeout(() => { setActiveProgress(0); }, 2000);
        break;
      }
      case 'cancel': {
        const { id } = msg;
        setTransfers((prev) =>
          prev.map((t) => (t.id === id ? { ...t, status: 'cancelled' } : t))
        );
        setActiveTransferId((prev) => (prev === id ? null : prev));
        setActiveProgress(0);
        break;
      }
      case 'resume_ack': {
        const { id, chunks_loaded } = msg;
        setTransfers((prev) =>
          prev.map((t) =>
            t.id === id
              ? { ...t, status: 'transferring', chunks_received: chunks_loaded }
              : t
          )
        );
        break;
      }
    }
  }, []);

  useEffect(() => {
    void loadTransfers();
    const interval = setInterval(loadTransfers, 10000);
    return () => { clearInterval(interval); };
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
