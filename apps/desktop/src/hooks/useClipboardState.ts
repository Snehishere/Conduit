import { useState, useCallback, useEffect } from 'react';
import { invoke } from '../lib/tauri';

export interface ClipboardItem {
  id: number;
  content: string;
  mime: string;
  source_device: string;
  timestamp: number;
  pinned: boolean;
}

export function useClipboardState(activeView: string) {
  const [clipboardItems, setClipboardItems] = useState<ClipboardItem[]>([]);

  const refreshClipboard = useCallback(() => {
    invoke<ClipboardItem[]>('get_clipboard_history', { limit: 100 })
      .then((items) => { setClipboardItems(items); })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (activeView === 'files' || activeView === 'clipboard') refreshClipboard();
  }, [activeView, refreshClipboard]);

  const copyItem = useCallback(
    (id: number) => {
      const item = clipboardItems.find((i) => i.id === id);
      // Hoisted so `navigator.clipboard` can be optional-chained (non-null ref type).
      const clipboard = navigator.clipboard as Clipboard | undefined;
      if (item) void clipboard?.writeText(item.content);
    },
    [clipboardItems]
  );

  const pinItem = useCallback(
    async (id: number) => {
      await invoke('toggle_clipboard_pin', { id });
      refreshClipboard();
    },
    [refreshClipboard]
  );

  const deleteItem = useCallback(
    async (id: number) => {
      await invoke('delete_clipboard_entry', { id });
      refreshClipboard();
    },
    [refreshClipboard]
  );

  const clearAll = useCallback(async () => {
    await invoke('clear_clipboard_history');
    refreshClipboard();
  }, [refreshClipboard]);

  return {
    clipboardItems,
    refreshClipboard,
    copyItem,
    pinItem,
    deleteItem,
    clearAll,
  };
}
