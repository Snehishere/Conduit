import React, { useState } from 'react';
import {
  IconWeb,
  IconBell,
  IconChat,
  IconPhone,
  IconFolder,
  IconMonitor,
  IconCursor,
  IconDeviceTV,
  IconWatch,
  IconZap,
  IconSettings,
  IconPlus,
} from '../icons';

type ActiveView = 'web' | 'notifications' | 'messages' | 'calls' | 'files' | 'settings' | 'screen_mirror' | 'remote_input' | 'smart_tv' | 'watch' | 'automation';

interface SidebarProps {
  activeView: ActiveView;
  onViewChange: (view: ActiveView) => void;
  onPairDevice: () => void;
  notificationCount: number;
  deviceCount: number;
  fileCount: number;
  messageCount: number;
  callActive: boolean;
}

const navItems: { id: ActiveView; label: string; icon: React.FC<{ size?: number }> }[] = [
  { id: 'web', label: 'Web', icon: IconWeb },
  { id: 'notifications', label: 'Notifs', icon: IconBell },
  { id: 'messages', label: 'SMS', icon: IconChat },
  { id: 'calls', label: 'Calls', icon: IconPhone },
  { id: 'files', label: 'Files', icon: IconFolder },
  { id: 'screen_mirror', label: 'Mirror', icon: IconMonitor },
  { id: 'remote_input', label: 'Remote', icon: IconCursor },
  { id: 'smart_tv', label: 'TV', icon: IconDeviceTV },
  { id: 'watch', label: 'Watch', icon: IconWatch },
  { id: 'automation', label: 'Auto', icon: IconZap },
  { id: 'settings', label: 'Settings', icon: IconSettings },
];

const Sidebar: React.FC<SidebarProps> = ({
  activeView,
  onViewChange,
  onPairDevice,
  notificationCount,
  deviceCount,
  fileCount,
  messageCount,
  callActive,
}) => {
  const [isExpanded, setIsExpanded] = useState(false);

  return (
    <aside
      className={`sidebar ${isExpanded ? 'expanded' : ''}`}
      onMouseEnter={() => setIsExpanded(true)}
      onMouseLeave={() => setIsExpanded(false)}
    >
      <div className="sidebar-nav">
        {navItems.map((item) => {
          const IconComponent = item.icon;
          return (
            <button
              key={item.id}
              className={`sidebar-item ${activeView === item.id ? 'active' : ''}`}
              onClick={() => onViewChange(item.id)}
            >
              <span className="sidebar-icon">
                <IconComponent size={20} />
              </span>
              <span className="sidebar-label">{item.label}</span>
              {item.id === 'notifications' && notificationCount > 0 && (
                <span className="sidebar-badge">{notificationCount}</span>
              )}
              {item.id === 'messages' && messageCount > 0 && (
                <span className="sidebar-badge">{messageCount}</span>
              )}
              {item.id === 'calls' && callActive && (
                <span className="sidebar-badge sidebar-badge-pulse">!</span>
              )}
              {item.id === 'files' && fileCount > 0 && (
                <span className="sidebar-badge">{fileCount}</span>
              )}
            </button>
          );
        })}
      </div>
      <div className="sidebar-footer">
        <button className="sidebar-pair-btn" onClick={onPairDevice}>
          <IconPlus size={18} />
        </button>
        <div className="sidebar-device-count">
          {deviceCount} device{deviceCount !== 1 ? 's' : ''}
        </div>
      </div>
    </aside>
  );
};

export default Sidebar;
