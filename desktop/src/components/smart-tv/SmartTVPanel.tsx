import React, { useState } from 'react';
import { useSmartTV } from '../../hooks/useSmartTV';
import {
  IconDeviceTV,
  IconVolume,
  IconMute,
  IconPower,
  IconArrowUp,
  IconArrowDown,
  IconArrowLeft,
  IconArrowRight,
  IconHome,
  IconMenu,
  IconChevronLeft,
  IconCast,
  IconPlay,
} from '../icons';

interface SmartTVPanelProps {
  onCastScreen?: (tvId: string) => void;
  onCastMedia?: (tvId: string, url: string) => void;
}

export default function SmartTVPanel({ onCastScreen, onCastMedia }: SmartTVPanelProps) {
  const {
    tvs,
    selectedTV,
    setSelectedTV,
    isDiscovering,
    discoverTVs,
    powerOn,
    powerOff,
    setVolume,
    toggleMute,
    setInput,
    launchApp,
    sendRemoteCommand,
    castScreen,
    castMedia,
  } = useSmartTV();

  const [mediaUrl, setMediaUrl] = useState('');

  const remoteKeys = [
    ['power', 'input', 'mute'],
    ['up', 'ok', 'down'],
    ['left', 'back', 'right'],
    ['vol_up', 'ch_up', 'home'],
    ['vol_down', 'ch_down', 'menu'],
  ];

  const inputs = ['HDMI 1', 'HDMI 2', 'HDMI 3', 'AV', 'USB', 'Screen Mirroring'];

  const apps: { id: string; name: string; icon: React.FC<{ size?: number }> }[] = [
    { id: 'netflix', name: 'Netflix', icon: IconDeviceTV },
    { id: 'youtube', name: 'YouTube', icon: IconPlay },
    { id: 'prime', name: 'Prime Video', icon: IconDeviceTV },
    { id: 'disney', name: 'Disney+', icon: IconDeviceTV },
    { id: 'spotify', name: 'Spotify', icon: IconVolume },
    { id: 'browser', name: 'Browser', icon: IconCast },
  ];

  const renderRemoteKeyContent = (key: string) => {
    switch (key) {
      case 'ok': return 'OK';
      case 'up': return <IconArrowUp size={16} />;
      case 'down': return <IconArrowDown size={16} />;
      case 'left': return <IconArrowLeft size={16} />;
      case 'right': return <IconArrowRight size={16} />;
      case 'power': return <IconPower size={16} />;
      case 'mute': return <IconMute size={16} />;
      case 'input': return <IconCast size={16} />;
      case 'vol_up': return <>VOL+</>;
      case 'vol_down': return <>VOL-</>;
      case 'ch_up': return <>CH+</>;
      case 'ch_down': return <>CH-</>;
      case 'back': return <IconChevronLeft size={16} />;
      case 'home': return <IconHome size={16} />;
      case 'menu': return <IconMenu size={16} />;
      default: return key;
    }
  };

  return (
    <div className="smart-tv-panel">
      <div className="smart-tv-header">
        <h3>Smart TV Control</h3>
        <button
          onClick={discoverTVs}
          disabled={isDiscovering}
          className="discover-btn"
        >
          {isDiscovering ? 'Discovering...' : 'Discover TVs'}
        </button>
      </div>

      {tvs.length === 0 ? (
        <div className="no-tvs">
          <div className="no-tvs-icon">
            <IconDeviceTV size={64} />
          </div>
          <p>No TVs found</p>
          <p className="no-tvs-hint">Click "Discover TVs" to find nearby smart TVs</p>
        </div>
      ) : (
        <div className="smart-tv-content">
          <div className="tv-list">
            {tvs.map(tv => (
              <div
                key={tv.id}
                className={`tv-item ${selectedTV?.id === tv.id ? 'selected' : ''}`}
                onClick={() => setSelectedTV(tv)}
              >
                <div className="tv-icon">
                  <IconDeviceTV size={24} />
                </div>
                <div className="tv-info">
                  <div className="tv-name">{tv.name}</div>
                  <div className="tv-manufacturer">{tv.manufacturer}</div>
                  <div className={`tv-status ${tv.isOnline ? 'online' : 'offline'}`}>
                    {tv.isOnline ? 'Online' : 'Offline'}
                  </div>
                </div>
                <div className="tv-volume">
                  <IconVolume size={14} /> {tv.volume}%
                </div>
              </div>
            ))}
          </div>

          {selectedTV && (
            <div className="tv-controls">
              <div className="tv-controls-header">
                <h4>{selectedTV.name}</h4>
                <div className="tv-power-controls">
                  <button
                    onClick={() => powerOn(selectedTV.id)}
                    disabled={selectedTV.isOnline}
                    className="power-btn on"
                  >
                    <IconPower size={14} /> On
                  </button>
                  <button
                    onClick={() => powerOff(selectedTV.id)}
                    disabled={!selectedTV.isOnline}
                    className="power-btn off"
                  >
                    <IconPower size={14} /> Off
                  </button>
                </div>
              </div>

              <div className="remote-control">
                <div className="remote-keys">
                  {remoteKeys.map((row, i) => (
                    <div key={i} className="remote-row">
                      {row.map(key => (
                        <button
                          key={key}
                          className={`remote-key ${key === 'ok' ? 'ok' : ''}`}
                          onClick={() => sendRemoteCommand(selectedTV.id, key)}
                        >
                          {renderRemoteKeyContent(key)}
                        </button>
                      ))}
                    </div>
                  ))}
                </div>
              </div>

              <div className="volume-control">
                <span className="volume-label">Volume</span>
                <input
                  type="range"
                  min="0"
                  max="100"
                  value={selectedTV.volume}
                  onChange={(e) => setVolume(selectedTV.id, parseInt(e.target.value))}
                  className="volume-slider"
                />
                <span className="volume-value">{selectedTV.volume}%</span>
                <button
                  onClick={() => toggleMute(selectedTV.id)}
                  className={`mute-btn ${selectedTV.isMuted ? 'muted' : ''}`}
                >
                  {selectedTV.isMuted ? <IconMute size={16} /> : <IconVolume size={16} />}
                </button>
              </div>

              <div className="input-selection">
                <span className="input-label">Input</span>
                <div className="input-buttons">
                  {inputs.map(input => (
                    <button
                      key={input}
                      className="input-btn"
                      onClick={() => setInput(selectedTV.id, input)}
                    >
                      {input}
                    </button>
                  ))}
                </div>
              </div>

              <div className="app-launcher">
                <span className="app-label">Apps</span>
                <div className="app-grid">
                  {apps.map(app => {
                    const AppIcon = app.icon;
                    return (
                      <button
                        key={app.id}
                        className="app-btn"
                        onClick={() => launchApp(selectedTV.id, app.id)}
                      >
                        <span className="app-icon">
                          <AppIcon size={22} />
                        </span>
                        <span className="app-name">{app.name}</span>
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="cast-controls">
                <button
                  onClick={() => castScreen(selectedTV.id)}
                  className="cast-btn"
                >
                  <IconCast size={16} /> Cast Screen
                </button>
                <div className="media-cast">
                  <input
                    type="text"
                    placeholder="Enter media URL"
                    value={mediaUrl}
                    onChange={(e) => setMediaUrl(e.target.value)}
                    className="media-input"
                  />
                  <button
                    onClick={() => {
                      if (mediaUrl) {
                        castMedia(selectedTV.id, mediaUrl);
                        setMediaUrl('');
                      }
                    }}
                    disabled={!mediaUrl}
                    className="cast-media-btn"
                  >
                    <IconPlay size={14} /> Play
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
