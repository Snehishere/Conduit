import {
  Zap,
  Bell,
  Smartphone,
  Volume2,
  VolumeX,
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
// `id` is checked against the generated protocol union, so a trigger or action
// added to `packages/protocol` cannot be shipped without a label here — see
// the compile-time coverage assertions at the bottom of this file.
//
// `as const satisfies` rather than a plain annotation: the annotation
// `: { id: AutomationTrigger['type']; label: string }[]` widens every `id` back
// to the full generated union, which makes the coverage assertions below
// vacuously `never`. `satisfies` still rejects an id the protocol does not
// declare, but the literal ids survive for `Exclude` to work on.
export const TRIGGER_TYPES = [
  { id: 'device_connect', label: 'Device connects' },
  { id: 'device_disconnect', label: 'Device disconnects' },
  { id: 'time', label: 'Time of day' },
  { id: 'battery_level', label: 'Battery level' },
  { id: 'wifi_change', label: 'Wi-Fi network change' },
  { id: 'app_open', label: 'App opens' },
  { id: 'audio_device_connect', label: 'Audio device connects' },
  { id: 'audio_device_disconnect', label: 'Audio device disconnects' },
] as const satisfies readonly { id: AutomationTrigger['type']; label: string }[];

export const ACTION_TYPES = [
  { id: 'send_notification', label: 'Send notification' },
  { id: 'set_phone_profile', label: 'Set phone profile' },
  { id: 'route_audio', label: 'Route audio' },
  { id: 'run_shell_command', label: 'Run command' },
  { id: 'toggle_wifi', label: 'Toggle Wi-Fi' },
  { id: 'toggle_bluetooth', label: 'Toggle Bluetooth' },
  { id: 'open_url', label: 'Open URL' },
  { id: 'open_app', label: 'Open app' },
] as const satisfies readonly { id: AutomationActionPayload['type']; label: string }[];

const TRIGGER_ICONS: Record<string, LucideIcon | undefined> = {
  device_connect: Link,
  device_disconnect: Link,
  time: Clock,
  battery_level: Battery,
  wifi_change: Wifi,
  app_open: AppWindow,
  audio_device_connect: Volume2,
  audio_device_disconnect: VolumeX,
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

// ── Compile-time coverage (W4.26) ───────────────────────────────────────────
//
// The claim this file used to make — "adding a trigger or action to
// `packages/protocol` makes this list a compile error" — was false: an array
// literal annotated `{ id: T['type']; label: string }[]` only has to be a
// *subset* of `T['type']`, so omitting ids compiled silently and
// `audio_device_disconnect` and `set_window_state` shipped with no UI entry.
//
// `never` is the bottom type, so `T extends never` holds only when `T` is
// empty. These two aliases therefore fail to compile exactly when a
// generated union grows an id with no entry above, which is what the comment
// always claimed.
type AssertNever<T extends never> = T;

export type EveryTriggerHasALabel = AssertNever<
  Exclude<AutomationTrigger['type'], (typeof TRIGGER_TYPES)[number]['id']>
>;

// `set_window_state` is the one deliberate exception, and it is not a label
// problem. `ActionEditor.tsx` has no control for its `state` field, so a rule
// built from it would carry `{ type: 'set_window_state' }`, which the Rust
// parser rejects (`automation.rs` requires `state`), and even a well-formed one
// reports failure — `execute_action` has no window handle and says so. Adding
// the entry would put a guaranteed-to-fail option in front of the user.
// Tracked for the action that implements it, not as a label.
type SelectableActionId = Exclude<AutomationActionPayload['type'], 'set_window_state'>;

export type EverySelectableActionHasALabel = AssertNever<
  Exclude<SelectableActionId, (typeof ACTION_TYPES)[number]['id']>
>;
