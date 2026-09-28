import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
import {
  Monitor,
  Smartphone,
  Tablet,
  Headphones,
  Watch,
  Tv,
  type LucideIcon,
} from 'lucide-react';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const DEVICE_ICON_MAP: Record<string, LucideIcon | undefined> = {
  desktop: Monitor,
  phone: Smartphone,
  tablet: Tablet,
  earbuds: Headphones,
  headphones: Headphones,
  watch: Watch,
  tv: Tv,
};

export function getDeviceIconComponent(deviceType: string): LucideIcon {
  return DEVICE_ICON_MAP[deviceType] ?? Smartphone;
}

export function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMin = Math.floor(diffMs / 60000);
  const diffHr = Math.floor(diffMin / 60);
  const diffDay = Math.floor(diffHr / 24);

  if (diffMin < 1) return 'now';
  if (diffMin < 60) return `${diffMin}m`;
  if (diffHr < 24) return `${diffHr}h`;
  if (diffDay < 7) return `${diffDay}d`;
  return date.toLocaleDateString();
}
