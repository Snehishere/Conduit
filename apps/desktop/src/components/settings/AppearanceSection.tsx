import React from 'react';
import { Monitor, Sun, Moon } from 'lucide-react';
import { useTheme, ACCENT_COLORS, setAccentColor } from '../../lib/theme';
import { cn } from '../../lib/utils';
import type { SectionProps } from './settingsTypes';

const AppearanceSection: React.FC<SectionProps> = ({ settings, updateSetting }) => {
  const { theme, setTheme } = useTheme();

  return (
    <div className="space-y-6">
      {/* Theme Toggle */}
      <div role="group" aria-labelledby="settings-theme-label">
        <div id="settings-theme-label" className="text-[13px] font-medium block mb-3 text-ink-1">Theme</div>
        <div className="grid grid-cols-3 gap-2">
          {([
            { value: 'dark', label: 'Dark', icon: Moon },
            { value: 'light', label: 'Light', icon: Sun },
            { value: 'system', label: 'System', icon: Monitor },
          ] as const).map(({ value, label, icon: Icon }) => (
            <button
              key={value}
              type="button"
              onClick={() => { setTheme(value); updateSetting('theme', value); }}
              aria-pressed={theme === value}
              className={cn(
                'btn-press flex flex-col items-center gap-2 py-3 rounded-[10px] text-[12px] font-medium transition-colors border',
                theme === value
                  ? 'bg-accent-dim-ink border-[rgba(14,116,144,0.35)] text-accent-ink'
                  : 'bg-fill-1 border-line-1 text-ink-2 hover:bg-fill-2 hover:text-ink-1'
              )}
            >
              <Icon size={18} strokeWidth={1.5} />
              {label}
            </button>
          ))}
        </div>
      </div>

      {/* Accent Color */}
      <div role="group" aria-labelledby="settings-accent-label">
        <div id="settings-accent-label" className="text-[13px] font-medium block mb-3 text-ink-1">Accent color</div>
        <div className="flex items-center gap-2">
          {ACCENT_COLORS.map((c) => {
            const isSelected =
              settings.accent_color === c.value ||
              settings.accent_color === c.dark ||
              settings.accent_color === c.light ||
              settings.accent_color === c.ink;
            return (
              <button
                key={c.value}
                type="button"
                onClick={() => {
                  const resolved = theme === 'light' ? c.light : c.dark;
                  setAccentColor(resolved, theme === 'light' ? 'light' : 'dark');
                  updateSetting('accent_color', resolved);
                }}
                className="btn-press w-10 h-10 flex items-center justify-center rounded-full transition-transform hover:scale-105"
                title={c.name}
                aria-label={`Accent color: ${c.name}`}
                aria-pressed={isSelected}
              >
                <span
                  className="block w-8 h-8 rounded-full transition-all"
                  style={{
                    // Swatch color is a runtime value, so it stays inline.
                    background: c.value,
                    boxShadow: isSelected
                      ? '0 0 0 2px var(--dot-ring), 0 0 0 4px var(--ink-1)'
                      : '0 0 0 1px var(--line-2)',
                  }}
                />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
};

export default AppearanceSection;
