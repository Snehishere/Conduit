import { createContext, useContext, useState, useCallback, useMemo, ReactNode } from 'react';
import { useDevices } from '../hooks/useDevices';

export type ActiveView = 'web' | 'notifications' | 'messages' | 'calls' | 'files' | 'clipboard' | 'settings' | 'screen_mirror' | 'remote_input' | 'devices' | 'automation';

interface DeviceTarget {
  id: string;
  name: string;
}

interface NavigationContextType {
  activeView: ActiveView;
  setActiveView: (view: ActiveView) => void;

  // ── Devices (single source of truth) ──────────────────────────────────────
  //
  // `useDevices()` used to be instantiated twice — once in App.tsx and once in
  // useSearch.ts — each with its own `setInterval(refreshDevices, 5000)`. The
  // two instances disagreed about `selectedDevice`, so a device picked in
  // global search set a selection the rest of the app never read. The instance
  // here is the only one; see the note in useSearch.ts on what it must change.
  devices: ReturnType<typeof useDevices>['devices'];
  selectedDevice: string | null;
  setSelectedDevice: (id: string | null) => void;
  refreshDevices: () => Promise<void>;
  removeDevice: (id: string) => Promise<void>;
  devicesError: string | null;

  screenMirrorDevice: DeviceTarget | null;
  setScreenMirrorDevice: (device: DeviceTarget | null) => void;
  remoteInputDevice: DeviceTarget | null;
  setRemoteInputDevice: (device: DeviceTarget | null) => void;
}

const NavigationContext = createContext<NavigationContextType | null>(null);

export function useNavigation(): NavigationContextType {
  const ctx = useContext(NavigationContext);
  if (!ctx) throw new Error('useNavigation must be used within NavigationProvider');
  return ctx;
}

export function NavigationProvider({ children }: { children: ReactNode }) {
  const [activeView, setActiveView] = useState<ActiveView>('web');
  const [screenMirrorDevice, setScreenMirrorDevice] = useState<DeviceTarget | null>(null);
  const [remoteInputDevice, setRemoteInputDevice] = useState<DeviceTarget | null>(null);

  // The one and only useDevices() instance for the app. Mounting it here
  // rather than in AppInner also means the 5s refresh survives a view change.
  const {
    devices,
    selectedDevice,
    setSelectedDevice,
    refreshDevices,
    removeDevice,
    error: devicesError,
  } = useDevices();

  const handleSetActiveView = useCallback((view: ActiveView) => {
    setActiveView(view);
  }, []);

  const value = useMemo<NavigationContextType>(() => ({
    activeView,
    setActiveView: handleSetActiveView,
    devices,
    selectedDevice,
    setSelectedDevice,
    refreshDevices,
    removeDevice,
    devicesError,
    screenMirrorDevice,
    setScreenMirrorDevice,
    remoteInputDevice,
    setRemoteInputDevice,
  }), [
    activeView, handleSetActiveView, devices, selectedDevice, setSelectedDevice,
    refreshDevices, removeDevice, devicesError, screenMirrorDevice, remoteInputDevice,
  ]);

  return (
    <NavigationContext.Provider value={value}>
      {children}
    </NavigationContext.Provider>
  );
}
