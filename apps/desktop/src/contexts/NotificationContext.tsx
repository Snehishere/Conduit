import { createContext, useContext, useState, useCallback, ReactNode } from 'react';

interface NotificationContextType {
  panelOpen: boolean;
  setPanelOpen: (open: boolean) => void;
  togglePanel: () => void;
}

const NotificationContext = createContext<NotificationContextType | null>(null);

export function useNotificationPanel(): NotificationContextType {
  const ctx = useContext(NotificationContext);
  if (!ctx) throw new Error('useNotificationPanel must be used within NotificationProvider');
  return ctx;
}

export function NotificationProvider({ children }: { children: ReactNode }) {
  const [panelOpen, setPanelOpen] = useState(false);

  const togglePanel = useCallback(() => { setPanelOpen((prev) => !prev); }, []);

  return (
    <NotificationContext.Provider value={{ panelOpen, setPanelOpen, togglePanel }}>
      {children}
    </NotificationContext.Provider>
  );
}
