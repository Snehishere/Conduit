import React, { useState, useEffect } from 'react';
import { IconWeb } from '../icons';

async function getAppWindow() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow();
}

const TitleBar: React.FC = () => {
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

  return (
    <div className="titlebar">
      <div className="titlebar-drag-region" />
      <div className="titlebar-title">
        <span className="titlebar-icon">
          <IconWeb size={16} />
        </span>
        <span>Conduit</span>
      </div>
      <div className="titlebar-controls">
        <button
          className="titlebar-btn titlebar-minimize"
          onClick={handleMinimize}
          aria-label="Minimize"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <rect x="2" y="5.5" width="8" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          className="titlebar-btn titlebar-maximize"
          onClick={handleMaximize}
          aria-label={isMaximized ? 'Restore' : 'Maximize'}
        >
          {isMaximized ? (
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <rect x="3.5" y="3.5" width="6" height="6" rx="0.5" stroke="currentColor" strokeWidth="1" fill="none" />
              <rect x="2" y="2" width="6" height="6" rx="0.5" stroke="currentColor" strokeWidth="1" fill="none" />
            </svg>
          ) : (
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <rect x="2" y="2" width="8" height="8" rx="0.5" stroke="currentColor" strokeWidth="1" fill="none" />
            </svg>
          )}
        </button>
        <button
          className="titlebar-btn titlebar-close"
          onClick={handleClose}
          aria-label="Close"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <path d="M3 3L9 9M9 3L3 9" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default TitleBar;
