// @ts-nocheck
/**
 * Shared mock factories and fixtures for Conduit desktop hook tests.
 */

import { vi } from 'vitest';
import type { AutomationRule } from '../../hooks/useAutomation';

export type { AutomationRule };

// ─── Types (mirror src types for fixture safety) ──────────────────────────────

export interface Device {
  id: string;
  name: string;
  device_type: string;
  os: string;
  battery?: number;
  signal?: string;
  status: string;
  last_seen: number;
}

export interface Notification {
  id: string;
  device_id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  actions?: string;
  dismissed: boolean;
}

export type { AutomationRule };

// ─── Fixture factories ────────────────────────────────────────────────────────

export function makeDevice(overrides: Partial<Device> = {}): Device {
  return {
    id: 'dev-1',
    name: 'Pixel 7',
    device_type: 'phone',
    os: 'Android 14',
    battery: 85,
    signal: 'strong',
    status: 'connected',
    last_seen: Math.floor(Date.now() / 1000),
    ...overrides,
  };
}

export function makeNotification(overrides: Partial<Notification> = {}): Notification {
  return {
    id: 'notif-1',
    device_id: 'dev-1',
    app: 'Messages',
    title: 'New message from Alice',
    body: 'Hey, are you free tonight?',
    timestamp: Math.floor(Date.now() / 1000),
    dismissed: false,
    ...overrides,
  };
}

export function makeAutomationRule(overrides: Partial<AutomationRule> = {}): AutomationRule {
  return {
    id: 'rule-1',
    name: 'Auto silence on connect',
    trigger: { type: 'device_connect', device_id: 'dev-1' },
    action: { type: 'set_phone_profile', profile: 'silent' },
    enabled: true,
    triggerCount: 0,
    ...overrides,
  };
}

// ─── Mock WebSocket factory ───────────────────────────────────────────────────

export interface MockWebSocket {
  send: ReturnType<typeof vi.fn>;
  close: ReturnType<typeof vi.fn>;
  readyState: number;
  onopen: ((ev: Event) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onerror: ((ev: Event) => void) | null;
}

export function createMockWebSocket(): MockWebSocket {
  return {
    send: vi.fn(),
    close: vi.fn(),
    readyState: 1, // OPEN
    onopen: null,
    onclose: null,
    onmessage: null,
    onerror: null,
  };
}

// ─── Mock Tauri invoke factory ────────────────────────────────────────────────

export function createMockInvoke() {
  return vi.fn();
}

// ─── Mock sendMessage factory ─────────────────────────────────────────────────

export function createMockSendMessage() {
  return vi.fn();
}
