import { createContext, useContext, useState, useEffect, useCallback, ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';

export type Theme = 'dark' | 'light' | 'system';

interface ThemeContextType {
  theme: Theme;
  resolvedTheme: 'dark' | 'light';
  setTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextType | null>(null);

function getSystemTheme(): 'dark' | 'light' {
  if (typeof window === 'undefined') return 'dark';
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function applyTheme(resolved: 'dark' | 'light') {
  document.documentElement.setAttribute('data-theme', resolved);
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(() => {
    try {
      const saved = localStorage.getItem('conduit-theme');
      if (saved === 'dark' || saved === 'light' || saved === 'system') return saved;
    } catch {
      /* localStorage unavailable — fall through to default */
    }
    return 'dark';
  });

  const [resolvedTheme, setResolved] = useState<'dark' | 'light'>(() =>
    theme === 'system' ? getSystemTheme() : theme
  );

  // Resolve and apply
  useEffect(() => {
    const resolved = theme === 'system' ? getSystemTheme() : theme;
    setResolved(resolved);
    applyTheme(resolved);
  }, [theme]);

  // Listen for system theme changes when in system mode
  useEffect(() => {
    if (theme !== 'system') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => {
      const resolved = getSystemTheme();
      setResolved(resolved);
      applyTheme(resolved);
    };
    mq.addEventListener('change', handler);
    return () => { mq.removeEventListener('change', handler); };
  }, [theme]);

  const setTheme = useCallback((newTheme: Theme) => {
    setThemeState(newTheme);
    try {
      localStorage.setItem('conduit-theme', newTheme);
    } catch {
      /* localStorage unavailable — in-memory only */
    }
    // Persist to backend settings
    // import('@tauri-apps/api/core').then(({ invoke }) => {
      invoke('save_settings_field', { key: 'theme', value: newTheme }).catch(() => {});
    // }).catch(() => {});
  }, []);

  // Restore the persisted accent (both --accent and the white-surface
  // --accent-ink / --accent-dim-ink pair) on load — WITHOUT re-invoking the
  // backend save, which is setAccentColor's job.
  useEffect(() => {
    const saved = loadAccentColor();
    if (saved) applyAccentVars(saved, resolvedTheme);
    // Re-apply on theme flip so --accent-dim matches the resolved theme alpha.
  }, [resolvedTheme]);

  return (
    <ThemeContext.Provider value={{ theme, resolvedTheme, setTheme }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextType {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider');
  return ctx;
}

// ─── Accent Colors ───

// `value`/`dark`/`light` drive the bright accent used on dark/starfield
// surfaces. `ink` is the ≥4.5:1-on-white counterpart — every frosted panel
// reads var(--accent-ink) so a bright accent can never land on white
// (spec §B7: never #00F0FF-class cyan on a light surface).
export const ACCENT_COLORS = [
  { name: 'Cyan', value: '#00f0ff', dark: '#00f0ff', light: '#0891b2', ink: '#155E75' },
  { name: 'Blue', value: '#60a5fa', dark: '#60a5fa', light: '#2563eb', ink: '#1D4ED8' },
  { name: 'Violet', value: '#a78bfa', dark: '#a78bfa', light: '#7c3aed', ink: '#6D28D9' },
  { name: 'Rose', value: '#fb7185', dark: '#fb7185', light: '#e11d48', ink: '#BE123C' },
  { name: 'Amber', value: '#fbbf24', dark: '#fbbf24', light: '#B45309', ink: '#B45309' },
  { name: 'Aqua', value: '#22d3ee', dark: '#22d3ee', light: '#0891b2', ink: '#155E75' },
  { name: 'Orange', value: '#fb923c', dark: '#fb923c', light: '#ea580c', ink: '#9A3412' },
  { name: 'Pink', value: '#f472b6', dark: '#f472b6', light: '#db2777', ink: '#BE185D' },
] as const;

const DEFAULT_ACCENT = '#00F0FF';
const INK_FALLBACK = '#155E75';

/** Normalize any hex (#rgb / #rrggbb, any case) to lowercase #rrggbb, else null. */
function normalizeHex(hex: string): string | null {
  const trimmed = hex.trim();
  const short = /^#([0-9a-f]{3})$/i.exec(trimmed);
  if (short) {
    const [r, g, b] = short[1].split('');
    return `#${r}${r}${g}${g}${b}${b}`.toLowerCase();
  }
  const full = /^#([0-9a-f]{6})$/i.exec(trimmed);
  return full ? `#${full[1]}`.toLowerCase() : null;
}

function srgbToLinear(channel: number): number {
  const v = channel / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance of a #rrggbb color. */
function luminance(hex: string): number {
  const n = normalizeHex(hex);
  if (!n) return 1;
  const r = parseInt(n.slice(1, 3), 16);
  const g = parseInt(n.slice(3, 5), 16);
  const b = parseInt(n.slice(5, 7), 16);
  return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);
}

/** Contrast ratio against a white (#FFFFFF) surface. */
function contrastOnWhite(hex: string): number {
  return 1.05 / (luminance(hex) + 0.05);
}

/** Darken #rrggbb by `factor` (0.88 = −12% per channel), clamped to black. */
function darken(hex: string, factor: number): string {
  const n = normalizeHex(hex) ?? INK_FALLBACK;
  const ch = (start: number) =>
    Math.max(0, Math.min(255, Math.round(parseInt(n.slice(start, start + 2), 16) * factor)));
  const to2 = (v: number) => v.toString(16).padStart(2, '0');
  return `#${to2(ch(1))}${to2(ch(3))}${to2(ch(5))}`;
}

/**
 * Guarantee ≥4.5:1 against white by darkening in steps — used when a stored
 * accent has no curated `ink` counterpart (spec §B7).
 */
function ensureInkContrast(hex: string): string {
  const base = normalizeHex(hex) ?? INK_FALLBACK;
  let color = base;
  for (let i = 0; i < 24 && contrastOnWhite(color) < 4.5; i++) {
    color = darken(color, 0.88);
  }
  return color;
}

/** Curated ink for a known accent (matched on value/dark/light), else derived. */
export function accentInkFor(color: string): string {
  const normalized = normalizeHex(color);
  if (!normalized) return INK_FALLBACK;
  const match = ACCENT_COLORS.find(
    (c) =>
      c.value.toLowerCase() === normalized ||
      c.dark.toLowerCase() === normalized ||
      c.light.toLowerCase() === normalized
  );
  return match ? match.ink : ensureInkContrast(normalized);
}

/**
 * Write the accent var pair(s). Pure — never persists, so it is safe to call
 * on mount (restore) as well as from setAccentColor (user action).
 *   --accent / --accent-dim        → bright accent (page / dark surfaces)
 *   --accent-ink / --accent-dim-ink → accent that is legible ON THE CURRENT
 *     THEME'S PANELS: in the dark theme every panel is smoked glass, so the
 *     "ink" accent collapses onto the bright one; in the light theme panels
 *     are white frost and it stays the curated ≥4.5:1-on-white counterpart.
 */
function applyAccentVars(color: string, resolvedTheme: 'dark' | 'light') {
  const normalized = normalizeHex(color) ?? DEFAULT_ACCENT;
  const ink = accentInkFor(normalized);
  const dark = resolvedTheme === 'dark';
  const root = document.documentElement.style;
  root.setProperty('--accent', normalized);
  root.setProperty('--accent-dim', hexToRgba(normalized, dark ? 0.10 : 0.08));
  root.setProperty('--accent-ink', dark ? normalized : ink);
  root.setProperty('--accent-dim-ink', hexToRgba(dark ? normalized : ink, dark ? 0.16 : 0.12));
}

export function setAccentColor(color: string, resolvedTheme: 'dark' | 'light') {
  applyAccentVars(color, resolvedTheme);
  try {
    localStorage.setItem('conduit-accent', color);
  } catch {
    /* localStorage unavailable — in-memory only */
  }
  // import('@tauri-apps/api/core').then(({ invoke }) => {
    invoke('save_settings_field', { key: 'accent_color', value: color }).catch(() => {});
  // }).catch(() => {});
}

export function loadAccentColor() {
  try {
    const saved = localStorage.getItem('conduit-accent');
    if (saved) return saved;
  } catch {
    /* localStorage unavailable — no saved accent */
  }
  return null;
}

function hexToRgba(hex: string, alpha: number): string {
  const n = normalizeHex(hex) ?? INK_FALLBACK;
  const r = parseInt(n.slice(1, 3), 16);
  const g = parseInt(n.slice(3, 5), 16);
  const b = parseInt(n.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
