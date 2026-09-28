import { X, Keyboard } from 'lucide-react';

interface ShortcutGroup {
  title: string;
  shortcuts: { keys: string; description: string }[];
}

const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: 'Navigation',
    shortcuts: [
      { keys: 'Ctrl + N', description: 'Pair new device' },
      { keys: 'Ctrl + T', description: 'Open Files' },
      { keys: 'Ctrl + M', description: 'Open Messages' },
      { keys: 'Ctrl + ,', description: 'Open Settings' },
      { keys: 'Ctrl + R', description: 'Refresh devices' },
      { keys: 'Ctrl + F', description: 'Focus search bar' },
    ],
  },
  {
    title: 'General',
    shortcuts: [
      { keys: 'Ctrl + ?', description: 'Toggle shortcuts overlay' },
      { keys: 'Ctrl + W', description: 'Close dialog / pairing' },
      { keys: 'Esc', description: 'Close dialog / cancel' },
    ],
  },
];

interface ShortcutsOverlayProps {
  open: boolean;
  onClose: () => void;
}

const ShortcutsOverlay: React.FC<ShortcutsOverlayProps> = ({ open, onClose }) => {
  if (!open) return null;

  return (
    <div
      className="scrim fixed inset-0 z-[300] flex items-center justify-center"
      onClick={onClose}
    >
      <div
        className="surf-frost w-[460px] max-w-[90vw] animate-fade-in rounded-[24px]"
        style={{
          boxShadow:
            '0 32px 80px -24px rgba(0, 0, 0, 0.9), 0 0 0 0.5px rgba(0, 0, 0, 0.16)',
        }}
        onClick={(e) => { e.stopPropagation(); }}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-ink-line px-6 py-4">
          <div className="flex items-center gap-2.5">
            <Keyboard size={18} className="text-accent-ink" />
            <h2 className="text-[15px] font-semibold text-ink-1">
              Keyboard Shortcuts
            </h2>
          </div>
          <button
            onClick={onClose}
            className="btn-press w-7 h-7 flex items-center justify-center rounded-lg transition-colors text-ink-3 hover:bg-fill-2 hover:text-ink-1"
          >
            <X size={14} />
          </button>
        </div>

        {/* Shortcuts */}
        <div className="px-6 py-4 space-y-5">
          {SHORTCUT_GROUPS.map((group) => (
            <div key={group.title}>
              <h3 className="mb-2.5 text-[11px] font-semibold uppercase tracking-wider text-ink-3">
                {group.title}
              </h3>
              <div className="space-y-1">
                {group.shortcuts.map((s) => (
                  <div
                    key={s.keys}
                    className="flex items-center justify-between py-1.5"
                  >
                    <span className="text-[12px] text-ink-2">
                      {s.description}
                    </span>
                    <div className="flex items-center gap-1">
                      {s.keys.split(' + ').map((key, i) => (
                        <span key={i} className="flex items-center gap-1">
                          {i > 0 && <span className="text-[11px] text-ink-3">+</span>}
                          <kbd
                            className="surf-clear px-1.5 py-0.5 rounded text-[11px] font-mono font-medium text-ink-1 border border-line-1"
                          >
                            {key}
                          </kbd>
                        </span>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="border-t border-ink-line px-6 py-3 text-center">
          <p className="text-[11px] text-ink-3">
            Press <kbd
              className="surf-clear px-1 py-0.5 rounded text-[11px] font-mono text-ink-1 border border-line-1"
            >Esc</kbd> to close
          </p>
        </div>
      </div>
    </div>
  );
};

export default ShortcutsOverlay;
