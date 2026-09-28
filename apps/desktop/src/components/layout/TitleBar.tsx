import { useState, useEffect } from 'react';
import { Minus, Square, X, Copy } from 'lucide-react';
import ConduitLogo from '../icons/ConduitLogo';

async function getAppWindow() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow();
}

const TitleBar = () => {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    getAppWindow().then((win) => {
      if (cancelled) return;
      win.isMaximized().then(setIsMaximized).catch(() => {});
      win.onResized(() => {
        win.isMaximized().then(setIsMaximized).catch(() => {});
      }).then((fn) => {
        unlisten = fn;
      }).catch(() => {});
    }).catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  const handleMinimize = async () => {
    const win = await getAppWindow();
    await win.minimize();
  };

  const handleMaximize = async () => {
    const win = await getAppWindow();
    if (isMaximized) {
      await win.unmaximize();
    } else {
      await win.maximize();
    }
  };

  const handleClose = async () => {
    const win = await getAppWindow();
    await win.close();
  };

  const btnStyle: React.CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: 46,
    height: 32,
    color: 'var(--text-3)',
    transition: 'background 0.15s ease, color 0.15s ease',
  };

  return (
    <div
      className="relative flex items-center shrink-0 select-none"
      style={{
        height: 32,
        background: 'var(--glass-bg)',
        backdropFilter: 'blur(20px) saturate(180%)',
        WebkitBackdropFilter: 'blur(20px) saturate(180%)',
        borderBottom: '1px solid var(--glass-border)',
      }}
    >
      <div className="absolute inset-0" style={{ WebkitAppRegion: 'drag' } as React.CSSProperties} />

      <div className="flex items-center gap-2 pl-3 z-10">
        <ConduitLogo size={15} />
        <span style={{
          fontSize: 12,
          fontWeight: 600,
          fontFamily: 'var(--font-display)',
          background: 'var(--gradient-brand)',
          WebkitBackgroundClip: 'text',
          WebkitTextFillColor: 'transparent',
          backgroundClip: 'text',
          letterSpacing: '0.02em',
        }}>
          Conduit
        </span>
      </div>

      <div className="flex ml-auto z-10">
        <button
          style={btnStyle}
          onMouseEnter={(e) => {
            e.currentTarget.style.color = 'var(--text-1)';
            e.currentTarget.style.background = 'var(--bg-3)';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.color = 'var(--text-3)';
            e.currentTarget.style.background = 'transparent';
          }}
          onClick={handleMinimize}
          aria-label="Minimize"
        >
          <Minus size={14} strokeWidth={1.5} />
        </button>
        <button
          style={btnStyle}
          onMouseEnter={(e) => {
            e.currentTarget.style.color = 'var(--text-1)';
            e.currentTarget.style.background = 'var(--bg-3)';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.color = 'var(--text-3)';
            e.currentTarget.style.background = 'transparent';
          }}
          onClick={handleMaximize}
          aria-label={isMaximized ? 'Restore' : 'Maximize'}
        >
          {isMaximized ? (
            <Copy size={13} strokeWidth={1.5} />
          ) : (
            <Square size={11} strokeWidth={1.5} />
          )}
        </button>
        <button
          style={btnStyle}
          onMouseEnter={(e) => {
            e.currentTarget.style.color = '#fff';
            e.currentTarget.style.background = 'var(--danger)';
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.color = 'var(--text-3)';
            e.currentTarget.style.background = 'transparent';
          }}
          onClick={handleClose}
          aria-label="Close"
        >
          <X size={14} strokeWidth={1.5} />
        </button>
      </div>
    </div>
  );
};

export default TitleBar;
