import React, { useState, useEffect, useCallback } from 'react';
import TitleBar from './components/layout/TitleBar';
import Sidebar from './components/layout/Sidebar';
import StatusBar from './components/layout/StatusBar';
import DeviceHub from './components/spider-web/DeviceHub';
import NotificationPanel from './components/notifications/NotificationPanel';
import ConversationList from './components/messages/ConversationList';
import MessageThread from './components/messages/MessageThread';
import IncomingCall from './components/calls/IncomingCall';
import CallHistory from './components/calls/CallHistory';
import AudioDeviceSelector from './components/calls/AudioDeviceSelector';
import FileExplorer from './components/files/FileExplorer';
import FileDropZone from './components/files/FileDropZone';
import PairingFlow from './components/pairing/PairingFlow';
import Settings from './components/settings/Settings';
import ScreenMirror from './components/screen-mirror/ScreenMirror';
import RemoteInput from './components/screen-mirror/RemoteInput';
import SmartTVPanel from './components/smart-tv/SmartTVPanel';
import WatchPanel from './components/watch/WatchPanel';
import AutomationPanel from './components/automation/AutomationPanel';
import { useWebSocket } from './hooks/useWebSocket';
import { useDevices } from './hooks/useDevices';
import { useClipboard } from './hooks/useClipboard';
import { useFiles } from './hooks/useFiles';
import { useSms } from './hooks/useSms';
import { useCalls } from './hooks/useCalls';
import { useEncryption } from './hooks/useEncryption';
import { useDiscovery } from './hooks/useDiscovery';

type ActiveView = 'web' | 'notifications' | 'messages' | 'calls' | 'files' | 'settings' | 'screen_mirror' | 'remote_input' | 'smart_tv' | 'watch' | 'automation';

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

function App() {
  const [activeView, setActiveView] = useState<ActiveView>('web');
  const [showPairing, setShowPairing] = useState(false);
  const { devices, selectedDevice, setSelectedDevice, refreshDevices, removeDevice } = useDevices();
  const { connected, notifications, dismissNotification, replyNotification, syncClipboard, sendMessage, registerHandler } = useWebSocket();
  const [deviceId, setDeviceId] = useState('');
  const {
    transfers,
    activeTransferId,
    activeProgress,
    sendFile,
    acceptTransfer,
    cancelTransfer,
    resumeTransfer,
    openFile,
    handleFileMessage,
    formatFileSize,
    getFileIcon,
  } = useFiles();

  const {
    threads: smsThreads,
    selectedThread: selectedSmsThread,
    setSelectedThread: setSelectedSmsThread,
    handleSmsMessage,
    sendMessage: sendSms,
    markRead,
    getSelectedThread,
    totalUnread,
    timeAgo,
  } = useSms(sendMessage);

  const {
    activeCall,
    callHistory,
    audioDevices,
    currentRoute,
    isStreaming,
    handleCallMessage,
    handleAudioMessage,
    answerCall,
    rejectCall,
    endCall,
    forwardCall,
    setAudioRoute,
    startAudioStream,
    stopAudioStream,
    getCallerName,
    getCallDuration,
    timeAgo: callTimeAgo,
  } = useCalls(sendMessage);

  const {
    publicKey,
    deviceId: encDeviceId,
    deviceName: encDeviceName,
  } = useEncryption();

  const {
    discoveredDevices,
    isDiscovering,
    handleDiscoveryMessage,
  } = useDiscovery();

  // File drop state
  const [showDropZone, setShowDropZone] = useState(false);
  const [dropTargetId, setDropTargetId] = useState<string | null>(null);

  // Screen mirror and remote input state
  const [screenMirrorDevice, setScreenMirrorDevice] = useState<{ id: string; name: string } | null>(null);
  const [remoteInputDevice, setRemoteInputDevice] = useState<{ id: string; name: string } | null>(null);

  useEffect(() => {
    refreshDevices();
    invoke<{ device_id: string }>('get_device_info')
      .then((info) => setDeviceId(info.device_id))
      .catch((err) => console.error('Failed to get device info:', err));
  }, [refreshDevices]);

  const { isMonitoring, startMonitoring, stopMonitoring, handleIncoming } = useClipboard({
    sendMessage,
    deviceId,
    onSynced: () => refreshDevices(),
  });

  // Start clipboard monitoring when connected
  useEffect(() => {
    if (connected && !isMonitoring) {
      startMonitoring();
    } else if (!connected && isMonitoring) {
      stopMonitoring();
    }
  }, [connected, isMonitoring, startMonitoring, stopMonitoring]);

  // Register message handlers via the shared WebSocket provider
  useEffect(() => {
    const unsubs = [
      registerHandler('clipboard', (data: Record<string, unknown>) => {
        if (data.action === 'sync') {
          handleIncoming({
            content: data.content as string,
            mime: data.mime as string,
            sourceDevice: data.source_device as string,
            timestamp: data.timestamp as number,
          });
        }
      }),
      registerHandler('file', (data: Record<string, unknown>) => handleFileMessage(data)),
      registerHandler('sms', (data: Record<string, unknown>) => handleSmsMessage(data)),
      registerHandler('call', (data: Record<string, unknown>) => handleCallMessage(data)),
      registerHandler('audio', (data: Record<string, unknown>) => handleAudioMessage(data)),
      registerHandler('discovery', (data: Record<string, unknown>) => handleDiscoveryMessage(data)),
    ];
    return () => unsubs.forEach((unsub) => unsub());
  }, [registerHandler, handleIncoming, handleFileMessage, handleSmsMessage, handleCallMessage, handleAudioMessage, handleDiscoveryMessage]);

  const handlePingDevice = async (targetId: string) => {
    sendMessage({
      type: 'notification',
      action: 'post',
      id: `ping_${Date.now()}`,
      app: 'Conduit',
      title: 'Device Ping',
      body: 'Ping from desktop!',
      timestamp: Math.floor(Date.now() / 1000),
      device_id: targetId,
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
    // Use Tauri file dialog to get file paths
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        multiple: true,
        title: 'Select files to send',
      });

      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        for (const path of paths) {
          await sendFile(targetDeviceId, path);
        }
      }
    } catch {
      // Fallback: for each file in the drop, try to read it
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        // We can't directly get the file path from browser File API
        // So we show the drop zone and use the Tauri dialog instead
        console.log('File dropped:', file.name);
      }
    }
    setShowDropZone(false);
  }, [sendFile]);

  const handleSpiderWebFileDrop = useCallback(() => {
    setShowDropZone(true);
  }, []);

  const pendingFileCount = transfers.filter(
    (t) => t.status === 'pending' || t.status === 'transferring'
  ).length;

  return (
    <div className="app">
      <TitleBar />
      <div className="app-drag-region" />
      <div className="app-content">
        <Sidebar
          activeView={activeView}
          onViewChange={setActiveView}
          onPairDevice={() => setShowPairing(true)}
          notificationCount={notifications.filter((n) => !n.dismissed).length}
          deviceCount={devices.filter((d) => d.status === 'connected').length}
          fileCount={pendingFileCount}
          messageCount={totalUnread}
          callActive={activeCall !== null}
        />
        <main className="main-content">
          {activeView === 'web' && (
            <DeviceHub
              devices={devices}
              selectedDevice={selectedDevice}
              onSelectDevice={setSelectedDevice}
              onPairDevice={() => setShowPairing(true)}
              onUnpairDevice={removeDevice}
              onPingDevice={handlePingDevice}
              onStartScreenMirror={handleStartScreenMirror}
              onStartRemoteInput={handleStartRemoteInput}
            />
          )}
          {activeView === 'notifications' && (
            <NotificationPanel
              notifications={notifications}
              onDismiss={dismissNotification}
              onReply={replyNotification}
            />
          )}
          {activeView === 'messages' && (
            selectedSmsThread && getSelectedThread() ? (
              <MessageThread
                thread={getSelectedThread() as NonNullable<ReturnType<typeof getSelectedThread>>}
                onSend={sendSms}
                onBack={() => setSelectedSmsThread(null)}
                timeAgo={timeAgo}
              />
            ) : (
              <ConversationList
                threads={smsThreads}
                selectedThread={selectedSmsThread}
                onSelectThread={setSelectedSmsThread}
                timeAgo={timeAgo}
              />
            )
          )}
          {activeView === 'calls' && (
            <div className="calls-view">
              <AudioDeviceSelector
                devices={audioDevices}
                currentRoute={currentRoute}
                onSelectRoute={setAudioRoute}
                isStreaming={isStreaming}
                onStartStream={startAudioStream}
                onStopStream={stopAudioStream}
              />
              <CallHistory
                history={callHistory}
                getCallerName={getCallerName}
                getCallDuration={getCallDuration}
                timeAgo={callTimeAgo}
              />
            </div>
          )}
          {activeView === 'files' && (
            <FileExplorer
              transfers={transfers}
              activeTransferId={activeTransferId}
              activeProgress={activeProgress}
              onAccept={acceptTransfer}
              onCancel={cancelTransfer}
              onResume={resumeTransfer}
              onOpen={openFile}
              formatFileSize={formatFileSize}
              getFileIcon={getFileIcon}
            />
          )}
          {activeView === 'settings' && <Settings />}
          {activeView === 'screen_mirror' && screenMirrorDevice && (
            <ScreenMirror
              deviceId={screenMirrorDevice.id}
              deviceName={screenMirrorDevice.name}
              onStop={() => {
                setScreenMirrorDevice(null);
                setActiveView('web');
              }}
            />
          )}
          {activeView === 'remote_input' && remoteInputDevice && (
            <RemoteInput
              deviceId={remoteInputDevice.id}
              deviceName={remoteInputDevice.name}
              onStop={() => {
                setRemoteInputDevice(null);
                setActiveView('web');
              }}
            />
          )}
          {activeView === 'smart_tv' && <SmartTVPanel />}
          {activeView === 'watch' && <WatchPanel />}
          {activeView === 'automation' && <AutomationPanel />}
        </main>
      </div>
      <StatusBar
        connected={connected}
        deviceCount={devices.filter((d) => d.status === 'connected').length}
        encryptionEnabled
      />
      {showPairing && (
        <PairingFlow
          onClose={() => setShowPairing(false)}
          deviceCount={devices.length}
          registerHandler={registerHandler}
        />
      )}
      {activeCall && activeCall.status === 'ringing' && (
        <IncomingCall
          call={activeCall}
          onAnswer={answerCall}
          onReject={rejectCall}
          onForward={() => forwardCall('desktop')}
          getCallerName={getCallerName}
        />
      )}
      <FileDropZone
        visible={showDropZone}
        targetDeviceId={dropTargetId}
        targetDeviceName={devices.find((d) => d.id === dropTargetId)?.name || 'Unknown'}
        onDrop={handleFileDrop}
        onClose={() => setShowDropZone(false)}
      />
    </div>
  );
}

export default App;
