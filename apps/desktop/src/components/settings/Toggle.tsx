import React from 'react';

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}

const Toggle: React.FC<ToggleProps> = ({ checked, onChange, label, description, disabled }) => {
  const toggleId = `toggle-${label.replace(/\s+/g, '-').toLowerCase()}`;
  return (
    <div className="flex items-center justify-between py-4 border-b border-line-1 last:border-0">
      <div className="pr-4">
        <div className="text-[13px] font-medium text-ink-1" id={`${toggleId}-label`}>{label}</div>
        {description && (
          <div className="text-[11px] mt-0.5 text-ink-3" id={`${toggleId}-desc`}>{description}</div>
        )}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-labelledby={`${toggleId}-label`}
        aria-describedby={description ? `${toggleId}-desc` : undefined}
        disabled={disabled}
        className="relative w-11 h-8 shrink-0 flex items-center disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent-ink)]"
        onClick={() => { onChange(!checked); }}
      >
        {/* Track 44×26 — off #CBD2DC, on var(--accent-ink) (white knob on
            #00F0FF was 1.3:1 and fails contrast) */}
        <span
          className="block h-[26px] w-11 rounded-[13px] transition-colors duration-200"
          style={{ background: checked ? 'var(--accent-ink)' : '#CBD2DC' }}
        >
          {/* Knob 20×20 — left 3px, slides 0 → 18px */}
          <span
            className="block h-5 w-5 rounded-full bg-white transition-transform duration-200"
            style={{
              marginLeft: 3,
              marginTop: 3,
              transform: checked ? 'translateX(18px)' : 'translateX(0)',
              boxShadow: '0 1px 3px rgba(0, 0, 0, 0.3)',
            }}
          />
        </span>
      </button>
    </div>
  );
};

export default Toggle;
