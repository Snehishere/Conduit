import { useState, useRef, useEffect, useCallback } from 'react';
import { Search, X, ArrowUp, ArrowDown, CornerDownLeft, Clock, Trash2, ArrowUpDown } from 'lucide-react';
import { cn } from '../../lib/utils';

export interface SearchResult {
  id: string;
  title: string;
  subtitle?: string;
  category: 'device' | 'file' | 'message' | 'notification';
  onClick: () => void;
}

interface SearchBarProps {
  results: SearchResult[];
  onSearch: (query: string) => void;
  placeholder?: string;
  className?: string;
}

const categoryLabels: Record<string, string> = {
  device: 'Devices',
  file: 'Files',
  message: 'Messages',
  notification: 'Notifications',
};

type SortKey = 'default' | 'name' | 'date' | 'type';

const HISTORY_KEY = 'conduit_search_history';
const MAX_HISTORY = 10;

function loadHistory(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(HISTORY_KEY) || '[]');
    return Array.isArray(parsed) ? parsed.filter((entry): entry is string => typeof entry === 'string') : [];
  } catch {
    return [];
  }
}

function saveHistory(history: string[]) {
  localStorage.setItem(HISTORY_KEY, JSON.stringify(history));
}

function addToHistory(query: string, history: string[]): string[] {
  const trimmed = query.trim();
  if (!trimmed) return history;
  const filtered = history.filter((h) => h.toLowerCase() !== trimmed.toLowerCase());
  return [trimmed, ...filtered].slice(0, MAX_HISTORY);
}

/* ── Frost surface tokens (spec §B4/B5) ───────────────────────────────────
   `.surf-frost` (globals.css) owns background/border/shadow. Because it is
   UNLAYERED CSS, a layered Tailwind shadow would lose to it — so the focus
   ring is an IMPORTANT focus-within shadow that re-states the frost shadow
   and APPENDS the 3px accent-ink ring (never a bare cyan border).
   Rows use `!` backgrounds for the same cascade reason: globals.css declares
   an unlayered `button { background: none }` that outranks layered utilities.
   NOTE: this must stay a literal string — Tailwind's scanner picks candidates
   out of source text, and the ring has to be generated at build time. */
const FOCUS_RING =
  'focus-within:shadow-[inset_0_1px_0_var(--frost-highlight),0_0_0_0.5px_rgba(0,0,0,0.16),0_24px_56px_-20px_rgba(0,0,0,0.80),0_0_0_3px_rgba(14,116,144,0.30)]!';

/* Menu / dropdown chrome: frosted white, 14px radius (spec §B4/B5). */
const MENU = 'surf-frost rounded-[14px]';
/* Row states: ink tint when selected/keyboard-active, clear hover otherwise. */
const ROW_SELECTED = 'bg-accent-dim-ink!';
const ROW_HOVER = 'hover:bg-fill-1!';
const CATEGORY_LABEL =
  'px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-ink-3';
const FOOTER_HINT =
  'px-3 py-1.5 flex items-center gap-3 text-[10px] text-ink-3';

const SearchBar: React.FC<SearchBarProps> = ({ results, onSearch, placeholder = 'Search...', className }) => {
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [sortKey, setSortKey] = useState<SortKey>('default');
  const [showSortMenu, setShowSortMenu] = useState(false);
  const [history, setHistory] = useState<string[]>(loadHistory);
  const inputRef = useRef<HTMLInputElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const sortMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    onSearch(query);
    setSelectedIdx(0);
  }, [query, onSearch]);

  const sortOptions: { key: SortKey; label: string }[] = [
    { key: 'default', label: 'Default' },
    { key: 'name', label: 'Name' },
    { key: 'date', label: 'Date' },
    { key: 'type', label: 'Type' },
  ];

  const sortedResults = [...results];
  if (sortKey === 'name') {
    sortedResults.sort((a, b) => a.title.localeCompare(b.title));
  } else if (sortKey === 'type') {
    sortedResults.sort((a, b) => a.category.localeCompare(b.category));
  }

  // Map instead of Record: `get()` returns `| undefined` honestly, so the
  // first-occurrence branch passes no-unnecessary-condition (Record index
  // access would always look truthy without `noUncheckedIndexedAccess`).
  const grouped = new Map<string, SearchResult[]>();
  for (const r of sortedResults) {
    const bucket = grouped.get(r.category);
    if (bucket) {
      bucket.push(r);
    } else {
      grouped.set(r.category, [r]);
    }
  }

  const flatResults = [...grouped.values()].flat();

  const historyCount = history.length;
  const showHistory = open && !query && historyCount > 0;
  const showResults = open && query && flatResults.length > 0;
  const showEmpty = open && query && flatResults.length === 0;

  const handleHistorySelect = useCallback((item: string) => {
    setQuery(item);
    inputRef.current?.focus();
  }, []);

  const handleClearHistory = useCallback(() => {
    setHistory([]);
    saveHistory([]);
  }, []);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    const totalItems = showHistory ? historyCount : flatResults.length;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIdx((i) => Math.min(i + 1, totalItems - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      if (showHistory && history[selectedIdx]) {
        setQuery(history[selectedIdx]);
        inputRef.current?.focus();
      } else if (flatResults[selectedIdx]) {
        flatResults[selectedIdx].onClick();
        setOpen(false);
        setQuery('');
        setHistory((h) => {
          const updated = addToHistory(query, h);
          saveHistory(updated);
          return updated;
        });
      }
    } else if (e.key === 'Escape') {
      setOpen(false);
      inputRef.current?.blur();
    }
  }, [flatResults, selectedIdx, showHistory, history, historyCount, query]);

  useEffect(() => {
    setSelectedIdx(0);
  }, [query, showHistory]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      const inDropdown = dropdownRef.current?.contains(e.target as Node);
      const inInput = inputRef.current?.contains(e.target as Node);
      const inSort = sortMenuRef.current?.contains(e.target as Node);
      if (!inDropdown && !inInput && !inSort) {
        setOpen(false);
        setShowSortMenu(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => { document.removeEventListener('mousedown', handler); };
  }, [open]);

  let globalIdx = -1;

  return (
    <div className={cn('relative', className)}>
      <div
        className={`surf-frost flex h-10 items-center gap-2 rounded-xl px-3 transition-shadow ${FOCUS_RING}`}
      >
        <Search size={14} className="shrink-0 text-ink-3" />
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => { setQuery(e.target.value); }}
          onFocus={() => { setOpen(true); }}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          className="flex-1 bg-transparent text-[13px] outline-none placeholder:text-ink-3"
          /* outline suppressed: the field-level frost ring below is the focus
             indicator (spec §B5 — ring appended to the frost shadow) */
          style={{ color: 'var(--ink-1)', outline: 'none' }}
        />
        {query && (
          <button
            onClick={() => { setQuery(''); inputRef.current?.focus(); }}
            className="btn-press text-ink-3 transition-colors hover:text-ink-1"
          >
            <X size={14} />
          </button>
        )}
        {query && (
          <div className="relative" ref={sortMenuRef}>
            <button
              onClick={() => { setShowSortMenu((s) => !s); }}
              className={cn(
                'btn-press transition-colors',
                sortKey !== 'default' ? 'text-accent-ink' : 'text-ink-3 hover:text-ink-1',
              )}
              title="Sort results"
            >
              <ArrowUpDown size={14} />
            </button>
            {showSortMenu && (
              <div
                className={cn(MENU, 'absolute top-full right-0 mt-1 z-50 min-w-[120px] animate-fade-in py-1')}
              >
                {sortOptions.map((opt) => (
                  <button
                    key={opt.key}
                    onClick={() => { setSortKey(opt.key); setShowSortMenu(false); }}
                    className={cn(
                      'w-full px-3 py-1.5 text-left text-[12px] transition-colors',
                      ROW_HOVER,
                      sortKey === opt.key ? 'text-accent-ink font-semibold' : 'text-ink-2',
                    )}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {showHistory && (
        <div
          ref={dropdownRef}
          className={cn(MENU, 'absolute top-full left-0 right-0 mt-1 animate-fade-in')}
          style={{ maxHeight: 280, overflowY: 'auto' }}
        >
          <div className="flex items-center justify-between px-3 py-1.5">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-3">
              Recent searches
            </span>
            <button
              onClick={handleClearHistory}
              className="btn-press flex items-center gap-1 text-[10px] text-ink-3 transition-colors hover:text-danger-ink"
            >
              <Trash2 size={10} /> Clear
            </button>
          </div>
          {history.map((item, i) => (
            <button
              key={`${item}-${i}`}
              onClick={() => { handleHistorySelect(item); }}
              className={cn(
                'w-full px-3 py-2 flex items-center gap-2 text-left transition-colors',
                i === selectedIdx ? ROW_SELECTED : ROW_HOVER,
              )}
              onMouseEnter={() => { setSelectedIdx(i); }}
            >
              <Clock size={12} className="shrink-0 text-ink-3" />
              <span className={cn('text-[12px]', i === selectedIdx ? 'text-accent-ink' : 'text-ink-2')}>
                {item}
              </span>
            </button>
          ))}
          <div className={cn(FOOTER_HINT)} style={{ borderTop: '1px solid var(--line-1)' }}>
            <span className="flex items-center gap-1"><ArrowUp size={10} /><ArrowDown size={10} /> navigate</span>
            <span className="flex items-center gap-1"><CornerDownLeft size={10} /> select</span>
          </div>
        </div>
      )}

      {showResults && (
        <div
          ref={showHistory ? undefined : dropdownRef}
          className={cn(MENU, 'absolute top-full left-0 right-0 mt-1 animate-fade-in overflow-hidden')}
          style={{ maxHeight: 320, overflowY: 'auto' }}
        >
          {[...grouped.entries()].map(([category, items]) => (
            <div key={category}>
              <div className={CATEGORY_LABEL}>
                {categoryLabels[category] || category}
              </div>
              {items.map((item) => {
                globalIdx++;
                const idx = globalIdx;
                const isSelected = idx === selectedIdx;
                return (
                  <button
                    key={item.id}
                    onClick={() => {
                      item.onClick();
                      setOpen(false);
                      setQuery('');
                      setHistory((h) => addToHistory(query, h));
                    }}
                    className={cn(
                      'w-full px-3 py-2 flex flex-col text-left transition-colors',
                      isSelected ? ROW_SELECTED : ROW_HOVER,
                    )}
                    onMouseEnter={() => { setSelectedIdx(idx); }}
                  >
                    <span className={cn('text-[12px] font-medium', isSelected ? 'text-accent-ink' : 'text-ink-1')}>
                      {item.title}
                    </span>
                    {item.subtitle && (
                      <span className="text-[11px] text-ink-3">
                        {item.subtitle}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          ))}
          <div className={cn(FOOTER_HINT)} style={{ borderTop: '1px solid var(--line-1)' }}>
            <span className="flex items-center gap-1"><ArrowUp size={10} /><ArrowDown size={10} /> navigate</span>
            <span className="flex items-center gap-1"><CornerDownLeft size={10} /> select</span>
            <span>esc close</span>
          </div>
        </div>
      )}

      {showEmpty && (
        <div
          className={cn(MENU, 'absolute top-full left-0 right-0 mt-1 px-4 py-6 text-center animate-fade-in')}
        >
          <p className="text-[12px] text-ink-3">No results for "{query}"</p>
        </div>
      )}
    </div>
  );
};

export default SearchBar;
