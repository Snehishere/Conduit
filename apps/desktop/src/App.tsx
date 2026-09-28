import { useState, useEffect, useCallback, useRef, lazy, Suspense } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import { Toaster } from 'sonner';
import { Monitor, MousePointer2 } from 'lucide-react';
import { ErrorBoundary } from './components/ErrorBoundary';

import FloatingDock from './components/layout/FloatingDock';
import StatusBar from './components/layout/StatusBar';
import DeviceHub from './components/network/DeviceHub';
import InboxScreen from './components/inbox/InboxScreen';
import IncomingCall from './components/calls/IncomingCall';
import CallsNotificationsSplit from './components/calls/CallsNotificationsSplit';
import FilesScreen from './components/files/FilesScreen';
import FileDropZone from './components/files/FileDropZone';
import PairingFlow from './components/pairing/PairingFlow';
import EmptyState from './components/ui/EmptyState';
import { SkeletonList } from './components/ui/Skeleton';

const Settings = lazy(() => import('./components/settings/Settings'));
const ScreenMirror = lazy(() => import('./components/screen-mirror/ScreenMirror'));
const RemoteInput = lazy(() => import('./components/screen-mirror/RemoteInput'));
const AutomationPanel = lazy(() => import('./components/automation/AutomationPanel'));
import Onboarding from './components/Onboarding';
import { useWebSocket } from './hooks/useWebSocket';
import { useClipboard } from './hooks/useClipboard';
import { useFiles } from './hooks/useFiles';
import { useSms } from './hooks/useSms';
import { useCalls } from './hooks/useCalls';
import { useEncryption } from './hooks/useEncryption';
import { useDiscovery } from './hooks/useDiscovery';
import { ThemeProvider } from './lib/theme';
import { showError } from './lib/toast';
import SplashScreen from './components/ui/SplashScreen';
import WhatsNewDialog from './components/ui/WhatsNewDialog';
import InteractiveTutorial from './components/ui/InteractiveTutorial';
import ShortcutsOverlay from './components/ui/ShortcutsOverlay';
import SearchBar from './components/ui/SearchBar';
import { useAppShell } from './hooks/useAppShell';
import { useAppNavigation } from './hooks/useAppNavigation';
import { usePairing } from './hooks/usePairing';
import { useFileDrop } from './hooks/useFileDrop';
import { useMessageHandlers } from './hooks/useMessageHandlers';
import { useClipboardState } from './hooks/useClipboardState';
import { invoke, getCurrentVersion } from './lib/tauri';
import { NavigationProvider, useNavigation } from './contexts/NavigationContext';

function AppInner() {
  const {
    activeView, setActiveView, selectedDevice, setSelectedDevice,
    screenMirrorDevice, setScreenMirrorDevice,
    remoteInputDevice, setRemoteInputDevice,
    setSearchQuery, setSearchFocused, searchResults,
  } = useAppNavigation();

  // Devices come from the single NavigationContext instance. `useAppNavigation`
  // is owned by another agent and does not forward them yet, so they are read
  // straight off the context here.
  const { devices, refreshDevices, removeDevice } = useNavigation();

  const { showPairing, openPairing, closePairing } = usePairing();
  const { showDropZone, closeDropZone, openDropZone } = useFileDrop();

  const shell = useAppShell();
  const { connected, notifications, dismissNotification, replyNotification, sendMessage, registerHandler } = useWebSocket();
  const [deviceId, setDeviceId] = useState('');
  const [appVersion, setAppVersion] = useState('');
  const [readNotificationIds, setReadNotificationIds] = useState<Set<string>>(() => {
    try { return new Set(JSON.parse(localStorage.getItem('conduit-read-notifications') || '[]') as string[]); }
    catch { return new Set(); }
  });

  useEffect(() => {
    const refreshReadState = () => {
      try { setReadNotificationIds(new Set(JSON.parse(localStorage.getItem('conduit-read-notifications') || '[]') as string[])); }
      catch { setReadNotificationIds(new Set()); }
    };
    window.addEventListener('conduit-notifications-read-change', refreshReadState);
    window.addEventListener('storage', refreshReadState);

    const unreg = registerHandler('notification', (msg: any) => {
      if (msg.action === 'mark_read' && msg.id) {
        setReadNotificationIds((prev) => {
          if (prev.has(msg.id)) return prev;
          const next = new Set(prev);
          next.add(msg.id);
          try { localStorage.setItem('conduit-read-notifications', JSON.stringify([...next])); } catch {}
          return next;
        });
      }
    });

    return () => {
      window.removeEventListener('conduit-notifications-read-change', refreshReadState);
      window.removeEventListener('storage', refreshReadState);
      unreg();
    };
  }, [registerHandler]);

  useEffect(() => {
    // Cleanup stale read IDs
    setReadNotificationIds(prev => {
      if (prev.size === 0 || notifications.length === 0) return prev;
      let changed = false;
      const next = new Set(prev);
      const activeIds = new Set(notifications.map(n => n.id));
      for (const id of next) {
        if (!activeIds.has(id)) {
          next.delete(id);
          changed = true;
        }
      }
      if (changed) {
        try { localStorage.setItem('conduit-read-notifications', JSON.stringify([...next])); } catch {}
      }
      return changed ? next : prev;
    });
  }, [notifications]);

  const markReadNotification = useCallback((id: string) => {
    sendMessage({ type: 'notification', action: 'mark_read', id });
  }, [sendMessage]);

  // Unread total feeds the dock badge and the Inbox tab badge (same formula
  // NotificationPanel uses locally).
  const unreadNotificationCount = notifications.filter(
    (n) => !n.dismissed && !readNotificationIds.has(n.id)
  ).length;

  // Encryption first: `sendSensitive` is the encrypted send path that useSms
  // and useCalls consult for every outbound sensitive payload. Its return value
  // used to be discarded entirely, which is why nothing was ever encrypted.
  const { sendSensitive, status: encryptionStatus } = useEncryption();
  const { handleDiscoveryMessage } = useDiscovery();

  const {
    transfers, activeTransferId, activeProgress,
    sendFile, acceptTransfer, cancelTransfer, resumeTransfer, openFile,
    handleFileMessage, formatFileSize, getFileIcon,
  } = useFiles();

  const {
    threads: smsThreads, selectedThread: selectedSmsThread, setSelectedThread: setSelectedSmsThread,
    handleSmsMessage, sendMessage: sendSms, getSelectedThread, totalUnread, timeAgo,
  } = useSms(sendMessage, sendSensitive);

  const {
    activeCall, callHistory,
    handleCallMessage, handleAudioMessage, answerCall, rejectCall, forwardCall,
    getCallerName, getCallDuration, timeAgo: callTimeAgo,
  } = useCalls(sendMessage, sendSensitive);

  const { clipboardItems, copyItem, pinItem, deleteItem, clearAll } = useClipboardState(activeView);

  const { isMonitoring, startMonitoring, stopMonitoring, handleIncoming } = useClipboard({
    sendMessage,
    deviceId,
    onSynced: () => refreshDevices(),
  });

  useMessageHandlers({
    handleFileMessage, handleSmsMessage, handleCallMessage,
    handleAudioMessage, handleDiscoveryMessage, handleIncoming, registerHandler,
  });

  useEffect(() => {
    void refreshDevices();
    invoke<{ device_id: string }>('get_device_info')
      .then((info) => { setDeviceId(info.device_id); })
      .catch(() => {});
  }, [refreshDevices]);

  // W2: fetch the app version once on mount for display in the status bar.
  // Non-critical — a failure only leaves the version hidden.
  useEffect(() => {
    getCurrentVersion()
      .then((version) => { setAppVersion(version); })
      .catch(() => {
        /* version display is non-critical */
      });
  }, []);

  useEffect(() => {
    if (connected && !isMonitoring) startMonitoring();
    else if (!connected && isMonitoring) stopMonitoring();
  }, [connected, isMonitoring, startMonitoring, stopMonitoring]);

  // Ref-based keyboard shortcut handler to avoid stale closures.
  // The handler always reads the latest state/functions via refs,
  // so the event listener only needs to be registered once.
  const handlersRef = useRef({
    openPairing, closePairing,
    setActiveView, setSearchFocused,
    refreshDevices, closeDropZone,
    toggleShortcuts: shell.toggleShortcuts,
    dismissShortcuts: shell.dismissShortcuts,
    setShowTutorial: shell.setShowTutorial,
  });
  handlersRef.current = {
    openPairing, closePairing,
    setActiveView, setSearchFocused,
    refreshDevices, closeDropZone,
    toggleShortcuts: shell.toggleShortcuts,
    dismissShortcuts: shell.dismissShortcuts,
    setShowTutorial: shell.setShowTutorial,
  };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const h = handlersRef.current;
      const ctrl = e.ctrlKey || e.metaKey;
      if (ctrl && e.key === 'n') { e.preventDefault(); h.openPairing(); }
      if (ctrl && e.key === 't') { e.preventDefault(); h.setActiveView('files'); }
      if (ctrl && e.key === 'm') { e.preventDefault(); h.setActiveView('messages'); }
      if (ctrl && e.key === ',') { e.preventDefault(); h.setActiveView('settings'); }
      if (ctrl && e.key === 'f') { e.preventDefault(); h.setSearchFocused(true); setTimeout(() => { h.setSearchFocused(false); }, 100); }
      if (ctrl && e.key === '?') { e.preventDefault(); h.toggleShortcuts(); }
      if (ctrl && e.key === 'r') { e.preventDefault(); void h.refreshDevices(); }
      if (ctrl && e.key === 'w') { e.preventDefault(); h.closePairing(); h.closeDropZone(); }
      if (e.key === 'Escape') {
        h.dismissShortcuts();
        h.setShowTutorial(false);
        h.closePairing();
        h.closeDropZone();
      }
    };
    window.addEventListener('keydown', handler);
    return () => { window.removeEventListener('keydown', handler); };
  }, []); // Stable: handler reads from ref

  const handlePingDevice = (targetId: string) => {
    sendMessage({
      type: 'notification', action: 'post',
      id: `ping_${Date.now()}`, app: 'Conduit', title: 'Device Ping',
      body: 'Ping from desktop!', timestamp: Math.floor(Date.now() / 1000), device_id: targetId,
    });
  };

  const handleStartScreenMirror = (deviceId: string, deviceName: string) => {
    setScreenMirrorDevice({ id: deviceId, name: deviceName });
    setActiveView('screen_mirror');
  };

  const handleStartRemoteInput = (deviceId: string, deviceName: string) => {
    setRemoteInputDevice({ id: deviceId, name: deviceName });
    setActiveView('remote_input');
  };

  const handleFileDrop = useCallback(async (files: FileList, targetDeviceId: string) => {
    try {
      const paths: string[] = [];
      for (let i = 0; i < files.length; i++) {
        const p = (files[i] as any).path;
        if (p) paths.push(p);
      }
      if (paths.length === 0) {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({ multiple: true, title: 'Select files to send' });
        if (selected) {
          const p = Array.isArray(selected) ? selected : [selected];
          paths.push(...p);
        }
      }
      for (const path of paths) await sendFile(targetDeviceId, path);
    } catch {
      // Dialog unavailable (e.g. non-Tauri context) — surface it instead of logging.
      showError(`Could not open the file picker for ${files.length} dropped file(s)`);
    }
    closeDropZone();
  }, [sendFile, closeDropZone]);

  const pendingFileCount = transfers.filter((t) => t.status === 'pending' || t.status === 'transferring').length;

  const PAGE_VARIANTS = {
    initial: { opacity: 0, y: 6 },
    animate: { opacity: 1, y: 0, transition: { duration: 0.18, ease: 'easeOut' as const } },
    exit:    { opacity: 0, transition: { duration: 0.1 } },
  };

  const renderView = () => {
    switch (activeView) {
      case 'web':
        return (
          <DeviceHub
            devices={devices} selectedDevice={selectedDevice}
            onSelectDevice={setSelectedDevice} onPairDevice={openPairing}
            onUnpairDevice={removeDevice} onPingDevice={handlePingDevice}
            onStartScreenMirror={handleStartScreenMirror} onStartRemoteInput={handleStartRemoteInput}
            loading={!connected}
          />
        );
      case 'notifications':
      case 'messages':
        // ONE unified feed — notifications + threads merged, no tabs. The
        // view id still distinguishes 'notifications' | 'messages' so
        // shortcuts/search/dock keep addressing them independently.
        return (
          <InboxScreen
            notificationCount={unreadNotificationCount}
            messageCount={totalUnread}
            notifications={notifications}
            onDismiss={dismissNotification}
            onReply={replyNotification}
            onMarkRead={markReadNotification}
            threads={smsThreads}
            selectedThread={selectedSmsThread}
            onSelectThread={setSelectedSmsThread}
            thread={selectedSmsThread ? getSelectedThread() : null}
            onSend={sendSms}
            onBack={() => { setSelectedSmsThread(null); }}
            timeAgo={timeAgo}
            loading={!connected}
          />
        );
      case 'calls':
        return (
          <CallsNotificationsSplit
            notifications={notifications}
            onDismissNotification={dismissNotification}
            onReplyNotification={replyNotification}
            onMarkReadNotification={markReadNotification}
            callHistory={callHistory}
            activeCall={activeCall}
            getCallerName={getCallerName}
            getCallDuration={getCallDuration}
            timeAgo={callTimeAgo}
            devices={devices}
            onForward={forwardCall}
            onAnswer={answerCall}
            onReject={rejectCall}
            loading={!connected}
          />
        );
      case 'clipboard':
      case 'files':
        // ONE adaptive surface — clipboard + transfers interleaved in a single
        // masonry, no tabs.
        return (
          <FilesScreen
            transfers={transfers}
            activeTransferId={activeTransferId}
            activeProgress={activeProgress}
            onAccept={acceptTransfer}
            onCancel={cancelTransfer}
            onResume={resumeTransfer}
            onOpen={openFile}
            formatFileSize={formatFileSize}
            getFileIcon={getFileIcon}
            loading={!connected}
            items={clipboardItems}
            onCopy={copyItem}
            onPin={pinItem}
            onDelete={deleteItem}
            onClearAll={clearAll}
          />
        );
      case 'settings':
        return <Settings />;
      // These two views used to `return null` when no device was selected,
      // which rendered a blank white panel with no explanation and no way back
      // except the dock. Mirror and Remote both address exactly one device, so
      // an explicit EmptyState with a route back to the device list is the
      // honest rendering.
      case 'screen_mirror':
        return screenMirrorDevice
          ? <ScreenMirror deviceId={screenMirrorDevice.id} deviceName={screenMirrorDevice.name} onStop={() => { setScreenMirrorDevice(null); setActiveView('web'); }} />
          : (
            <EmptyState
              icon={Monitor}
              title="No device to mirror"
              description="Screen mirroring addresses a single paired device. Pick one on the device list to start mirroring its screen."
              action={{ label: 'Choose a device', onClick: () => { setActiveView('web'); } }}
              className="mt-[12vh]"
            />
          );
      case 'remote_input':
        return remoteInputDevice
          ? <RemoteInput deviceId={remoteInputDevice.id} deviceName={remoteInputDevice.name} onStop={() => { setRemoteInputDevice(null); setActiveView('web'); }} />
          : (
            <EmptyState
              icon={MousePointer2}
              title="No device for remote input"
              description="Remote input sends your mouse and keyboard to a single paired device. Pick one on the device list to take control."
              action={{ label: 'Choose a device', onClick: () => { setActiveView('web'); } }}
              className="mt-[12vh]"
            />
          );
      case 'automation':
        return <AutomationPanel />;
      default:
        return null;
    }
  };

  return (
    <>
      {shell.showSplash && <SplashScreen onComplete={shell.dismissSplash} />}
      {shell.showWhatsNew && (
        <WhatsNewDialog version={shell.currentVersion} features={[
          'Customizable theme system with dark, light, and system modes',
          '8 accent color options',
          'Keyboard shortcuts for faster navigation',
          'Global search across devices, files, and messages',
          'Improved settings with categorized options',
        ]} onComplete={shell.dismissWhatsNew} />
      )}
      {shell.showTutorial && <InteractiveTutorial onComplete={shell.dismissTutorial} />}
      {shell.showShortcuts && <ShortcutsOverlay open={shell.showShortcuts} onClose={shell.toggleShortcuts} />}
      <Toaster position="top-right" />
      <div aria-live="assertive" aria-atomic="true" style={{ position: 'fixed', width: 1, height: 1, overflow: 'hidden', clip: 'rect(0,0,0,0)', whiteSpace: 'nowrap' }} id="sr-announcer" />
      {shell.showOnboarding && <Onboarding onComplete={shell.completeOnboarding} />}
      <div 
        className="flex flex-col h-full"
        onDragEnter={(e) => {
          if (e.dataTransfer.types.includes('Files') && selectedDevice) {
            openDropZone();
          }
        }}
      >

        {!connected && (
          <div role="status" aria-live="polite" className="w-full bg-danger/[0.12] border-b border-danger/30 px-5 py-2 flex items-center justify-center gap-2 text-[13px] text-danger animate-slide-down">
            <span className="w-2 h-2 rounded-full bg-danger animate-pulse-dot" />
            <span>Offline — clipboard, files, and messages are not syncing</span>
          </div>
        )}
        <div className="flex justify-center pt-[20px] pb-[12px] z-[90] shrink-0">
          <SearchBar results={searchResults} onSearch={setSearchQuery} placeholder="Search devices, files, messages... (Ctrl+F)" className="w-[320px] shrink-0" />
        </div>
        {/* Bottom padding clears the fixed dock (bottom:48 + 56 tall = 104px
            above the viewport, StatusBar sits 26px lower → 78px) plus a gap. */}
        <div className="flex flex-1 overflow-hidden relative pb-[84px]">
          <main className="flex-1 overflow-hidden relative">
            <AnimatePresence initial={false}>
              <motion.div
                key={activeView}
                layoutId="conduit-active-panel"
                variants={PAGE_VARIANTS}
                initial="initial"
                animate="animate"
                exit="exit"
                className="h-full w-full"
              >
                <Suspense fallback={<div className="p-6"><SkeletonList count={3} /></div>}>
                  {renderView()}
                </Suspense>
              </motion.div>
            </AnimatePresence>
          </main>
        </div>
        <FloatingDock
          activeView={activeView} onViewChange={setActiveView} onPairDevice={openPairing}
          notificationCount={unreadNotificationCount}
          deviceCount={devices.filter((d) => d.status === 'connected').length}
          fileCount={pendingFileCount} messageCount={totalUnread} clipboardCount={clipboardItems.length}
          callActive={!!activeCall}
        />
        <StatusBar connected={connected} deviceCount={devices.filter((d) => d.status === 'connected').length} encryption={encryptionStatus} version={appVersion} />
        {showPairing && <PairingFlow onClose={closePairing} deviceCount={devices.length} registerHandler={registerHandler} />}
        {activeView !== 'calls' && activeCall && (activeCall.status === 'ringing' || activeCall.status === 'active') && (
          <IncomingCall call={activeCall} onAnswer={answerCall} onReject={rejectCall} onForward={forwardCall} getCallerName={getCallerName} getCallDuration={getCallDuration} devices={devices} />
        )}
        <FileDropZone visible={showDropZone} targetDeviceId={selectedDevice ?? null} targetDeviceName={devices.find(d => d.id === selectedDevice)?.name ?? "Unknown"} onDrop={handleFileDrop} onClose={closeDropZone} />
      </div>
    </>
  );
}

function App() {
  return (
    <ThemeProvider>
      <ErrorBoundary fallback={(_error, reset) => (
        <div className="flex flex-col items-center justify-center h-screen gap-4">
          <h1 className="text-danger text-2xl font-semibold">Something went wrong</h1>
          <p className="text-text-secondary">The interface hit an unexpected error. Restart Conduit if it keeps happening.</p>
          {/* `reset` re-renders the tree below the boundary; it does not restart
              the process, so the label must not claim that it does. */}
          <button onClick={reset} className="px-6 py-3 bg-accent text-bg-primary rounded-md text-base font-medium hover:opacity-90 transition-opacity">
            Try again
          </button>
        </div>
      )}>
        <NavigationProvider>
          <AppInner />
        </NavigationProvider>
      </ErrorBoundary>
    </ThemeProvider>
  );
}

export default App;
