import React from 'react';
import {
  IconDeviceDesktop,
  IconDevicePhone,
  IconDeviceTablet,
  IconDeviceEarbuds,
  IconDeviceHeadphones,
  IconDeviceTV,
  IconWatch,
  IconFolder,
  type IconProps,
} from '../components/icons';

export { IconProps };

export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

export function formatTime(timestamp: number): string {
  const minutes = Math.floor((Date.now() - timestamp * 1000) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function getDeviceIconComponent(type: string): React.FC<IconProps> {
  const map: Record<string, React.FC<IconProps>> = {
    desktop: IconDeviceDesktop,
    phone: IconDevicePhone,
    tablet: IconDeviceTablet,
    earbuds: IconDeviceEarbuds,
    headphones: IconDeviceHeadphones,
    watch: IconWatch,
    tv: IconDeviceTV,
  };
  return map[type] || IconDevicePhone;
}

export function getFileIconComponent(mime: string): React.FC<IconProps> {
  if (mime.startsWith('image/')) return IconDevicePhone;
  if (mime.startsWith('video/')) return IconDeviceTV;
  if (mime.startsWith('audio/')) return IconDeviceHeadphones;
  if (mime.includes('pdf')) return IconDeviceDesktop;
  return IconFolder;
}

// Legacy helpers kept for backward compatibility
export function getDeviceIcon(type: string): string {
  return '';
}

export function getDeviceColor(type: string): string {
  return '#e6e1e5';
}
