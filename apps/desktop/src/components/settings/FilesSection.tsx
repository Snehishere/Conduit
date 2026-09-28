import React from 'react';
import { FolderOpen } from 'lucide-react';
import Toggle from './Toggle';
import type { SectionProps } from './settingsTypes';

const FilesSection: React.FC<SectionProps> = ({ settings, updateSetting }) => {
  const handleSelectFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true });
      if (selected) updateSetting('default_download_folder', selected);
    } catch { /* user cancelled or dialog unavailable */ }
  };

  return (
    <div className="space-y-4">
      <Toggle checked={settings.auto_accept_files} onChange={(v) => { updateSetting('auto_accept_files', v); }} label="Auto-accept files" description="Automatically accept incoming file transfers" />
      <div>
        <label htmlFor="settings-download-folder" className="text-[13px] font-medium block mb-1.5 text-ink-1">Default download folder</label>
        <div className="flex items-center gap-2">
          <input
            id="settings-download-folder"
            type="text"
            value={settings.default_download_folder || 'Downloads'}
            readOnly
            className="flex-1 min-w-0 px-3 py-2.5 rounded-[10px] text-[13px] text-ink-2 bg-fill-1 border border-line-1 transition-shadow focus:shadow-[0_0_0_3px_rgba(14,116,144,0.30)]"
          />
          <button
            type="button"
            onClick={handleSelectFolder}
            className="btn-press shrink-0 h-[42px] px-3 rounded-[10px] text-[12px] font-medium text-ink-2 bg-fill-1 border border-line-1 hover:bg-fill-2 hover:text-ink-1 transition-colors"
            aria-label="Browse for download folder"
          >
            <FolderOpen size={14} />
          </button>
        </div>
      </div>
    </div>
  );
};

export default FilesSection;
