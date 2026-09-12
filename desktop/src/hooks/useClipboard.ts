import { useState, useEffect, useCallback, useRef } from 'react';

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

interface ClipboardEntry {
  content: string;
  mime: string;
  sourceDevice: string;
  timestamp: number;
}

interface UseClipboardOptions {
  sendMessage?: (msg: Record<string, unknown>) => void;
  deviceId: string;
  onSynced?: (entry: ClipboardEntry) => void;
}

export function useClipboard({ sendMessage, deviceId, onSynced }: UseClipboardOptions) {
  const [history, setHistory] = useState<ClipboardEntry[]>([]);
  const [isMonitoring, setIsMonitoring] = useState(false);
  const lastContent = useRef('');
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const readClipboard = useCallback(async (): Promise<string | null> => {
    try {
      const { readText } = await import('@tauri-apps/plugin-clipboard-manager');
      return await readText();
    } catch {
      // Fallback: try web clipboard API
      try {
        return await navigator.clipboard.readText();
      } catch {
        return null;
      }
    }
  }, []);

  const copyToClipboard = useCallback(async (text: string) => {
    try {
      const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
      await writeText(text);
    } catch {
      await navigator.clipboard.writeText(text);
    }
  }, []);

  const sendSync = useCallback((content: string, mime: string) => {
    sendMessage?.({
      type: 'clipboard',
      action: 'sync',
      content,
      mime,
      source_device: deviceId,
      timestamp: Math.floor(Date.now() / 1000),
    });
  }, [sendMessage, deviceId]);

  const startMonitoring = useCallback(() => {
    // Guard with ref to survive React 18 double-invoke / rapid calls.
    if (pollRef.current) return;
    setIsMonitoring(true);

    pollRef.current = setInterval(async () => {
      const content = await readClipboard();
      if (content && content !== lastContent.current && content.length < 10 * 1024 * 1024) {
        lastContent.current = content;
        const mime = content.startsWith('data:') ? 'image/png' : 'text/plain';
        sendSync(content, mime);
      }
    }, 1000);
  }, [readClipboard, sendSync]);

  const stopMonitoring = useCallback(() => {
    setIsMonitoring(false);
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  const handleIncoming = useCallback((entry: ClipboardEntry) => {
    if (entry.sourceDevice === deviceId) return;
    lastContent.current = entry.content;
    setHistory((prev) => [entry, ...prev].slice(0, 50));
    copyToClipboard(entry.content);
    onSynced?.(entry);
  }, [deviceId, copyToClipboard, onSynced]);

  useEffect(() => {
    return () => { if (pollRef.current) clearInterval(pollRef.current); };
  }, []);

  return {
    history,
    isMonitoring,
    startMonitoring,
    stopMonitoring,
    handleIncoming,
    copyToClipboard,
    sendSync,
  };
}
