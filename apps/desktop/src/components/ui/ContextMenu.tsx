import React, { useState, useEffect, useRef, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { LucideIcon } from 'lucide-react';
import { cn } from '../../lib/utils';

export type ContextMenuItem =
  | { type: 'separator' }
  | {
      label: string;
      icon?: LucideIcon;
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      onClick?: () => void;
    };

interface ContextMenuProps {
  items: ContextMenuItem[];
  children: React.ReactNode;
  className?: string;
}

export default function ContextMenu({ items, children, className }: ContextMenuProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState({ x: 0, y: 0 });
  const menuRef = useRef<HTMLDivElement>(null);

  const handleContext = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Calculate position, keeping menu within viewport
    const x = Math.min(e.clientX, window.innerWidth - 220);
    const y = Math.min(e.clientY, window.innerHeight - items.length * 36 - 16);
    setPos({ x, y });
    setOpen(true);
  }, [items.length]);

  const close = useCallback(() => { setOpen(false); }, []);

  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        close();
      }
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    document.addEventListener('mousedown', handleClick);
    document.addEventListener('keydown', handleKey);
    return () => {
      document.removeEventListener('mousedown', handleClick);
      document.removeEventListener('keydown', handleKey);
    };
  }, [open, close]);

  return (
    <>
      <div onContextMenu={handleContext} className={className} style={{ cursor: 'context-menu' }}>
        {children}
      </div>
      {open && createPortal(
        <div
          ref={menuRef}
          className="surf-frost fixed z-[9999] min-w-[200px] py-1.5 rounded-[14px] animate-fade-in"
          style={{
            left: pos.x,
            top: pos.y,
          }}
        >
          {items.map((item, i) => {
            // `'type' in item` narrows to the separator variant — no further
            // comparison against the literal is needed.
            if ('type' in item) {
              return (
                <div
                  key={`sep-${i}`}
                  className="my-1 mx-2"
                  style={{ borderTop: '1px solid var(--ink-line)' }}
                />
              );
            }

            // After the `'type' in item` guard, TS has already narrowed `item`
            // to the regular menu-item variant — no assertion needed.
            const menu = item;
            const Icon = menu.icon;

            return (
              <button
                key={menu.label}
                onClick={() => {
                  if (!menu.disabled) {
                    menu.onClick?.();
                    close();
                  }
                }}
                disabled={menu.disabled}
                className={cn(
                  'w-full flex items-center gap-2.5 px-3 py-1.5 text-[12px] font-medium transition-colors text-left',
                  menu.disabled && 'opacity-40 cursor-not-allowed',
                  !menu.disabled && !menu.danger && 'hover:bg-fill-1',
                  // `text-red-400` kept for compatibility; the inline color
                  // below wins and stays legible on white frost (spec §B7).
                  !menu.disabled && menu.danger && 'text-red-400 hover:bg-[rgba(185,28,28,0.10)]',
                )}
                style={{ color: menu.danger ? 'var(--danger-ink)' : 'var(--ink-2)' }}
              >
                {Icon && <Icon size={13} strokeWidth={1.5} />}
                <span className="flex-1">{menu.label}</span>
                {menu.shortcut && (
                  <span className="text-[10px] opacity-60 font-mono">{menu.shortcut}</span>
                )}
              </button>
            );
          })}
        </div>,
        document.body,
      )}
    </>
  );
}
