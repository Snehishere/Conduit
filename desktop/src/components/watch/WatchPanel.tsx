import React, { useState } from 'react';
import { useWatch } from '../../hooks/useWatch';
import {
  IconWatch,
  IconX,
  IconSearch,
  IconChevronLeft,
  IconChevronRight,
  IconPlay,
  IconPause,
  IconBattery,
  IconSend,
  IconEdit,
  IconTrash,
} from '../icons';

interface WatchPanelProps {
  onFindPhone?: (watchId: string) => void;
}

export default function WatchPanel({ onFindPhone }: WatchPanelProps) {
  const {
    watches,
    selectedWatch,
    setSelectedWatch,
    notifications,
    isDiscovering,
    currentTrack,
    discoverWatches,
    sendNotification,
    quickReply,
    findPhone,
    controlMusic,
    dismissNotification,
  } = useWatch();

  const [notificationText, setNotificationText] = useState('');
  const [replyText, setReplyText] = useState('');
  const [selectedNotification, setSelectedNotification] = useState<string | null>(null);

  const quickReplies = [
    'OK',
    'Thanks!',
    'On my way',
    'Call you later',
    'Busy right now',
  ];

  return (
    <div className="watch-panel">
      <div className="watch-header">
        <h3>Watch Companion</h3>
        <button
          onClick={discoverWatches}
          disabled={isDiscovering}
          className="discover-btn"
        >
          {isDiscovering ? 'Discovering...' : 'Discover Watches'}
        </button>
      </div>

      {watches.length === 0 ? (
        <div className="no-watches">
          <div className="no-watches-icon">
            <IconWatch size={64} />
          </div>
          <p>No watches found</p>
          <p className="no-watches-hint">Click "Discover Watches" to find nearby watches</p>
        </div>
      ) : (
        <div className="watch-content">
          {/* Watch List */}
          <div className="watch-list">
            {watches.map(watch => (
              <div
                key={watch.id}
                className={`watch-item ${selectedWatch?.id === watch.id ? 'selected' : ''}`}
                onClick={() => setSelectedWatch(watch)}
              >
                <div className="watch-icon">
                  <IconWatch size={24} />
                </div>
                <div className="watch-info">
                  <div className="watch-name">{watch.name}</div>
                  <div className="watch-type">{watch.type.replace('_', ' ')}</div>
                  <div className={`watch-status ${watch.isConnected ? 'connected' : 'disconnected'}`}>
                    {watch.isConnected ? 'Connected' : 'Disconnected'}
                  </div>
                </div>
                <div className="watch-battery">
                  <IconBattery size={14} /> {watch.battery}%
                </div>
              </div>
            ))}
          </div>

          {/* Watch Controls */}
          {selectedWatch && (
            <div className="watch-controls">
              <div className="watch-controls-header">
                <h4>{selectedWatch.name}</h4>
                <div className="watch-battery-large">
                  <IconBattery size={16} /> {selectedWatch.battery}%
                </div>
              </div>

              {/* Find My Phone */}
              <div className="find-phone-section">
                <button
                  onClick={() => {
                    if (onFindPhone) {
                      onFindPhone(selectedWatch.id);
                    } else {
                      findPhone(selectedWatch.id);
                    }
                  }}
                  className="find-phone-btn"
                >
                  <IconSearch size={16} /> Find My Phone
                </button>
                <p className="find-phone-hint">
                  Makes your phone ring even if it's on silent
                </p>
              </div>

              {/* Music Control */}
              {currentTrack && (
                <div className="music-control">
                  <div className="music-info">
                    <div className="music-title">{currentTrack.title}</div>
                    <div className="music-artist">{currentTrack.artist}</div>
                  </div>
                  <div className="music-controls">
                    <button
                      onClick={() => controlMusic(selectedWatch.id, 'prev')}
                      className="music-btn"
                    >
                      <IconChevronLeft size={16} />
                    </button>
                    <button
                      onClick={() => controlMusic(selectedWatch.id, currentTrack.isPlaying ? 'pause' : 'play')}
                      className="music-btn play"
                    >
                      {currentTrack.isPlaying ? <IconPause size={16} /> : <IconPlay size={16} />}
                    </button>
                    <button
                      onClick={() => controlMusic(selectedWatch.id, 'next')}
                      className="music-btn"
                    >
                      <IconChevronRight size={16} />
                    </button>
                  </div>
                </div>
              )}

              {/* Send Notification */}
              <div className="send-notification">
                <h5>Send Notification to Watch</h5>
                <div className="notification-input">
                  <input
                    type="text"
                    placeholder="Enter notification text"
                    value={notificationText}
                    onChange={(e) => setNotificationText(e.target.value)}
                    className="notification-text-input"
                  />
                  <button
                    onClick={() => {
                      if (notificationText) {
                        sendNotification(selectedWatch.id, 'Conduit', notificationText);
                        setNotificationText('');
                      }
                    }}
                    disabled={!notificationText}
                    className="send-btn"
                  >
                    <IconSend size={14} /> Send
                  </button>
                </div>
              </div>

              {/* Notifications */}
              <div className="watch-notifications">
                <h5>Recent Notifications</h5>
                {notifications.length === 0 ? (
                  <p className="no-notifications">No notifications yet</p>
                ) : (
                  <div className="notification-list">
                    {notifications.slice(0, 10).map(notification => (
                      <div
                        key={notification.id}
                        className={`notification-item ${notification.dismissed ? 'dismissed' : ''}`}
                      >
                        <div className="notification-content">
                          <div className="notification-app">{notification.app}</div>
                          <div className="notification-title">{notification.title}</div>
                          <div className="notification-body">{notification.body}</div>
                          <div className="notification-time">
                            {new Date(notification.timestamp * 1000).toLocaleTimeString()}
                          </div>
                        </div>
                        {!notification.dismissed && (
                          <div className="notification-actions">
                            {selectedNotification === notification.id ? (
                              <div className="quick-reply-input">
                                <input
                                  type="text"
                                  placeholder="Quick reply"
                                  value={replyText}
                                  onChange={(e) => setReplyText(e.target.value)}
                                  className="reply-input"
                                />
                                <button
                                  onClick={() => {
                                    if (replyText) {
                                      quickReply(selectedWatch.id, notification.id, replyText);
                                      setReplyText('');
                                      setSelectedNotification(null);
                                      dismissNotification(notification.id);
                                    }
                                  }}
                                  className="reply-btn"
                                >
                                  <IconSend size={14} /> Send
                                </button>
                                <button
                                  onClick={() => setSelectedNotification(null)}
                                  className="cancel-btn"
                                >
                                  <IconX size={14} /> Cancel
                                </button>
                              </div>
                            ) : (
                              <div className="quick-replies">
                                {quickReplies.map(reply => (
                                  <button
                                    key={reply}
                                    onClick={() => {
                                      quickReply(selectedWatch.id, notification.id, reply);
                                      dismissNotification(notification.id);
                                    }}
                                    className="quick-reply-btn"
                                  >
                                    {reply}
                                  </button>
                                ))}
                                <button
                                  onClick={() => setSelectedNotification(notification.id)}
                                  className="custom-reply-btn"
                                >
                                  <IconEdit size={14} /> Custom...
                                </button>
                                <button
                                  onClick={() => dismissNotification(notification.id)}
                                  className="dismiss-btn"
                                >
                                  <IconTrash size={14} /> Dismiss
                                </button>
                              </div>
                            )}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}