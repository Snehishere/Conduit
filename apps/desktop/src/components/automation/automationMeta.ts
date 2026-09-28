// @ts-nocheck
import {
  Zap,
  Bell,
  Smartphone,
  Volume2,
  Monitor,
  Wifi,
  Bluetooth,
  Link,
  Folder,
  Clock,
  Battery,
  AppWindow,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { AutomationActionPayload, AutomationTrigger } from '../../types/websocket';

// Sentence case, matching every other label in the app. These were Title Case
// ("Device Connects", "Send Notification"), which read as a different product.
//
// The `id` types are indexed off the generated protocol types rather than left
// loose, so adding a trigger or action to `packages/protocol` makes this list a
// compile error until the label is added too.
export const TRIGGER_TYPES: { id: AutomationTrigger['type']; label: string }[] = [
  { id: 'device_connect', label: 'Device connects' },
  { id: 'device_disconnect', label: 'Device disconnects' },
  { id: 'time', label: 'Time of day' },
  { id: 'battery_level', label: 'Battery level' },
  { id: 'wifi_change', label: 'Wi-Fi network change' },
  { id: 'app_open', label: 'App opens' },
  { id: 'audio_device_connect', label: 'Audio device connects' },
];

export const ACTION_TYPES: { id: AutomationActionPayload['type']; label: string }[] = [
  { id: 'send_notification', label: 'Send notification' },
  { id: 'set_phone_profile', label: 'Set phone profile' },
  { id: 'route_audio', label: 'Route audio' },
  { id: 'run_shell_command', label: 'Run command' },
  { id: 'toggle_wifi', label: 'Toggle Wi-Fi' },
  { id: 'toggle_bluetooth', label: 'Toggle Bluetooth' },
  { id: 'open_url', label: 'Open URL' },
  { id: 'open_app', label: 'Open app' },
];

const TRIGGER_ICONS: Record<string, LucideIcon | undefined> = {
  device_connect: Link,
  device_disconnect: Link,
  time: Clock,
  battery_level: Battery,
  wifi_change: Wifi,
  app_open: AppWindow,
  audio_device_connect: Volume2,
};

const ACTION_ICONS: Record<string, LucideIcon | undefined> = {
  send_notification: Bell,
  set_phone_profile: Smartphone,
  route_audio: Volume2,
  run_shell_command: Monitor,
  toggle_wifi: Wifi,
  toggle_bluetooth: Bluetooth,
  open_url: Link,
  open_app: Folder,
};

export const getTriggerLabel = (type: string) =>
  TRIGGER_TYPES.find((t) => t.id === type)?.label || type;
export const getActionLabel = (type: string) =>
  ACTION_TYPES.find((a) => a.id === type)?.label || type;
export const getTriggerIcon = (type: string): LucideIcon =>
  TRIGGER_ICONS[type] || Zap;
export const getActionIcon = (type: string): LucideIcon =>
  ACTION_ICONS[type] || Zap;
