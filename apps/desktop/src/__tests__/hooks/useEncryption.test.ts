import { renderHook, act, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  useEncryption,
  buildStatus,
  MESSAGE_PROTECTION,
  ENVELOPE_TYPES,
  type SendResult,
} from '../../hooks/useEncryption';

// ─── Mocks ────────────────────────────────────────────────────────────────────

const mockInvoke = vi.fn();

vi.mock('../../lib/tauri', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockShowWarning = vi.fn();

vi.mock('../../lib/toast', () => ({
  showError: vi.fn(),
  showSuccess: vi.fn(),
  showInfo: vi.fn(),
  showWarning: (...args: unknown[]) => mockShowWarning(...args),
}));

const DEVICE_INFO = { device_id: 'desktop-1', public_key: 'aabbcc', name: 'Workstation' };

// ─── Tests ────────────────────────────────────────────────────────────────────

describe('buildStatus', () => {
  it('never claims end-to-end encryption when a key exists', () => {
    const status = buildStatus('aabbcc', [], []);
    expect(status.label).toBe('Not E2E');
    expect(status.level).toBe('partial');
    expect(status.detail).toContain('Not end-to-end encrypted');
  });

  it('never claims end-to-end encryption even when everything went out encrypted', () => {
    const status = buildStatus('aabbcc', ['call'], []);
    expect(status.label).not.toBe('E2E');
    expect(status.detail).toContain('Not end-to-end encrypted');
  });

  it('reports a missing key rather than a false assurance', () => {
    const status = buildStatus('', [], []);
    expect(status.level).toBe('no-key');
    expect(status.label).toBe('No key');
    expect(status.detail).toContain('unencrypted');
  });

  it('names the message types observed leaving in the clear', () => {
    const status = buildStatus('aabbcc', ['call'], ['sms', 'clipboard']);
    expect(status.detail).toContain('Sent in the clear this session: clipboard, sms.');
  });

  it('names the message types observed using the envelope', () => {
    const status = buildStatus('aabbcc', ['call'], []);
    expect(status.detail).toContain('Handed to the envelope command this session: call.');
  });

  it('says nothing has been sent yet when there is no traffic', () => {
    expect(buildStatus('aabbcc', [], []).detail).toContain('Nothing sensitive has been sent yet.');
  });
});

describe('MESSAGE_PROTECTION', () => {
  it('classifies every type with a justification', () => {
    for (const entry of MESSAGE_PROTECTION) {
      expect(entry.type).toBeTruthy();
      expect(entry.reason).toBeTruthy();
      expect(['envelope', 'plaintext']).toContain(entry.protection);
    }
  });

  it('cannot express an end-to-end state at all', () => {
    // MessageProtection is closed over exactly two values, so there is no
    // compile- or run-time way for the table to claim E2E. This is the
    // invariant that stops the original regression from coming back.
    const levels = new Set(MESSAGE_PROTECTION.map((e) => e.protection));
    expect([...levels].sort()).toEqual(['envelope', 'plaintext']);
  });

  it('marks sms, file, notification and clipboard as plaintext', () => {
    for (const type of ['sms', 'file', 'notification', 'clipboard']) {
      const entry = MESSAGE_PROTECTION.find((e) => e.type === type);
      expect(entry?.protection).toBe('plaintext');
    }
  });

  it('only routes call control through the envelope', () => {
    expect(ENVELOPE_TYPES).toEqual(['call']);
  });
});

describe('useEncryption', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue(DEVICE_INFO);
  });

  it('loads the device key on mount', async () => {
    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.publicKey).toBe('aabbcc'); });
    expect(result.current.deviceId).toBe('desktop-1');
    expect(mockInvoke).toHaveBeenCalledWith('get_device_info');
  });

  it('surfaces Not E2E rather than E2E once the key is loaded', async () => {
    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.status.level).toBe('partial'); });
    expect(result.current.status.label).toBe('Not E2E');
  });

  it('sendEncrypted hands the payload to the Tauri command', async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === 'get_device_info' ? Promise.resolve(DEVICE_INFO) : Promise.resolve(null),
    );

    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    let outcome;
    await act(async () => {
      outcome = await result.current.sendEncrypted('dev-1', 'hello');
    });

    expect(outcome).toEqual({ encrypted: true });
    expect(mockInvoke).toHaveBeenCalledWith('send_encrypted_message', {
      targetDeviceId: 'dev-1',
      plaintext: 'hello',
    });
  });

  it('sendEncrypted reports the failure reason instead of swallowing it', async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === 'get_device_info'
        ? Promise.resolve(DEVICE_INFO)
        : Promise.reject(new Error('DeviceNotFound')),
    );

    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    let outcome;
    await act(async () => {
      outcome = await result.current.sendEncrypted('nope', 'hello');
    });

    expect(outcome).toEqual({ encrypted: false, reason: 'DeviceNotFound' });
  });

  it('sendSensitive refuses to invent a destination when none is given', async () => {
    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    let outcome: SendResult | undefined;
    await act(async () => {
      outcome = await result.current.sendSensitive(null, { type: 'sms', action: 'send' });
    });

    expect(outcome?.encrypted).toBe(false);
    expect(mockInvoke).not.toHaveBeenCalledWith('send_encrypted_message', expect.anything());
  });

  it('records a type as sent in the clear when the envelope path fails', async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === 'get_device_info'
        ? Promise.resolve(DEVICE_INFO)
        : Promise.reject(new Error('DeviceNotFound')),
    );

    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    await act(async () => {
      await result.current.sendSensitive(null, { type: 'sms', action: 'send' });
    });

    expect(result.current.status.plaintextTypes).toEqual(['sms']);
    expect(result.current.status.encryptedTypes).toEqual([]);
  });

  it('records a type as enveloped when the command accepts it', async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === 'get_device_info' ? Promise.resolve(DEVICE_INFO) : Promise.resolve(null),
    );

    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    await act(async () => {
      await result.current.sendSensitive('dev-1', { type: 'call', action: 'answer' });
    });

    expect(result.current.status.encryptedTypes).toEqual(['call']);
    expect(result.current.status.plaintextTypes).toEqual([]);
  });

  it('warns the user that a payload went out unencrypted', async () => {
    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    await act(async () => {
      await result.current.sendSensitive(null, { type: 'sms', action: 'send' });
    });

    expect(mockShowWarning).toHaveBeenCalledWith(
      expect.stringContaining('Sending "sms" unencrypted'),
    );
  });

  it('warns at most once per distinct failure reason', async () => {
    const { result } = renderHook(() => useEncryption());
    await waitFor(() => { expect(result.current.initialized).toBe(true); });

    await act(async () => {
      await result.current.sendSensitive(null, { type: 'sms', action: 'send' });
      await result.current.sendSensitive(null, { type: 'sms', action: 'send' });
      await result.current.sendSensitive(null, { type: 'call', action: 'answer' });
    });

    expect(mockShowWarning).toHaveBeenCalledTimes(1);
  });
});
