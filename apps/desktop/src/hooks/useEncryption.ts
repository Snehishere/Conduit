import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '../lib/tauri';
import { showWarning } from '../lib/toast';

interface EncryptionInfo {
  public_key: string;
  device_id: string;
  name: string;
}

// ─── Honest protection model ──────────────────────────────────────────────────
//
// The status bar used to render a hard-coded "E2E" chip. Nothing in the app
// backed that claim: `sendEncrypted` had zero call sites, so every payload left
// in the clear. This module now owns the claim, and the table below is the
// evidence for it.
//
// Read the code paths, not the design docs:
//
//  * `server/mod.rs:475-529` DECRYPTS any inbound `encrypted` envelope and
//    processes the inner JSON. The relay therefore *terminates* the encryption
//    — an envelope is hop encryption between the app and the relay process,
//    never end-to-end between two devices.
//  * `server/mod.rs:301-333` wraps outbound frames for the destination client,
//    but its `else` at :328-330 writes the frame verbatim when the destination
//    is not in the sync engine, so the wrap is best-effort, not a guarantee.
//  * `server/mod.rs:196,273` keys `ctx.clients` by a per-connection UUID while
//    `WsServer::send_to` (mod.rs:777) looks it up by *device* id, so the
//    envelope produced by `send_encrypted_message` misses the local client table
//    and falls through to the relay branch. With no relay configured it returns
//    `false`, and `send_encrypted_message` still resolves `Ok(())`
//    (pairing.rs:197-208) — a payload reported as sent that never left.
//  * There is no inbound decrypt command in the Tauri surface (see the command
//    list in main.rs), so the frontend cannot open an envelope it receives.
//    Every inbound sensitive payload is handled in the clear.
//  * `commands/file.rs:53-101` broadcasts base64 file chunks verbatim. File
//    bytes never traverse the JS socket and are never encrypted; the only
//    integrity check is the SHA-256 `checksum` field.
//
// Consequently NO message type is end-to-end encrypted in this build, and the
// status bar must never say otherwise. `MESSAGE_PROTECTION` records what each
// type actually gets; the runtime counters in {@link EncryptionStatus} record
// what was observed leaving the app in this session.

/** Protection a wire message type actually has. */
export type MessageProtection = 'envelope' | 'plaintext';

export interface ProtectionEntry {
  /** Wire `type`, e.g. `call`. */
  type: string;
  /** The actions covered by this entry, for the status-bar detail text. */
  actions: string;
  protection: MessageProtection;
  /** One-line justification, citing the code that decides it. */
  reason: string;
}

export const MESSAGE_PROTECTION: readonly ProtectionEntry[] = [
  {
    type: 'call',
    actions: 'answer, reject, forward',
    protection: 'envelope',
    reason:
      'Routed through send_encrypted_message; the relay still decrypts the envelope, so this is hop encryption and delivery is unverified.',
  },
  {
    type: 'sms',
    actions: 'send',
    protection: 'plaintext',
    reason:
      'Broadcast to every paired peer (server/mod.rs:644-646) with no destination device, so no single shared secret can be selected.',
  },
  {
    type: 'file',
    actions: 'request, accept, chunk, progress, complete, cancel, resume',
    protection: 'plaintext',
    reason: 'Base64 chunks are broadcast verbatim by commands/file.rs:53-101.',
  },
  {
    type: 'notification',
    actions: 'post, reply, dismiss, mark_read',
    protection: 'plaintext',
    reason:
      'No destination device id on the payload, and no envelope wrapper on the send path (server/mod.rs:608-619).',
  },
  {
    type: 'clipboard',
    actions: 'sync, request',
    protection: 'plaintext',
    reason: 'Forwarded verbatim by server/mod.rs:620-622 with no envelope wrapper.',
  },
  {
    type: 'screen_mirror',
    actions: 'frame, touch, key, scroll',
    protection: 'plaintext',
    reason: 'Frame data and input events are broadcast verbatim; see handlers/screen_mirror.rs.',
  },
  {
    type: 'remote_input',
    actions: 'move, click, scroll',
    protection: 'plaintext',
    reason: 'Input events are broadcast verbatim; see handlers/remote_input.rs.',
  },
  {
    type: 'audio',
    actions: 'stream_data, playback_data',
    protection: 'plaintext',
    reason: 'Handled by handlers/audio.rs with no envelope wrapper.',
  },
];

/** Message types whose declared protection is an envelope (not end-to-end). */
export const ENVELOPE_TYPES: readonly string[] = MESSAGE_PROTECTION
  .filter((e) => e.protection === 'envelope')
  .map((e) => e.type);

// ─── Status ───────────────────────────────────────────────────────────────────

/**
 * `no-key`   — `get_device_info` produced no public key, so nothing can be
 *              encrypted at all.
 * `partial`  — a key exists and the envelope command is wired up, but the relay
 *              terminates the envelope, so the traffic is not end-to-end.
 *
 * There is deliberately no `e2e` member: no path in this build earns one, and
 * adding a state that the code can never produce is how the original lie came
 * about.
 */
export type EncryptionLevel = 'no-key' | 'partial';

export interface EncryptionStatus {
  level: EncryptionLevel;
  /** Chip text for the status bar. Never claims end-to-end encryption. */
  label: string;
  /** Full sentence for the tooltip and the accessible name. */
  detail: string;
  /** Message types observed being handed to the envelope command this session. */
  encryptedTypes: readonly string[];
  /** Message types observed leaving the app in the clear this session. */
  plaintextTypes: readonly string[];
  /** Static per-type classification, so the detail can be expanded later. */
  coverage: readonly ProtectionEntry[];
}

/** Outcome of routing one payload through the encrypted path. */
export interface SendResult {
  /** True when the payload was accepted by `send_encrypted_message`. */
  encrypted: boolean;
  /** Populated when `encrypted` is false. */
  reason?: string;
}

/**
 * Builds the status-bar summary. Exported so the wording is unit-testable
 * without mounting the hook.
 *
 * There is no branch that yields an "E2E" label: no path in this build is
 * end-to-end encrypted, so the honest floor is `Not E2E` whenever a key exists
 * and `No key` when it does not.
 */
export function buildStatus(
  publicKey: string,
  observedEncrypted: readonly string[],
  observedPlaintext: readonly string[],
): EncryptionStatus {
  // Sorted here rather than at the call site so the wording is deterministic.
  const encryptedTypes = [...observedEncrypted].sort();
  const plaintextTypes = [...observedPlaintext].sort();

  if (!publicKey) {
    return {
      level: 'no-key',
      label: 'No key',
      detail:
        'No device key is available, so every message leaves this app unencrypted.',
      encryptedTypes,
      plaintextTypes,
      coverage: MESSAGE_PROTECTION,
    };
  }

  const parts = [
    'Not end-to-end encrypted: the relay decrypts message envelopes, so sensitive content is readable in transit.',
    'Only call control uses the envelope, and its delivery is unverified.',
  ];
  if (plaintextTypes.length > 0) {
    parts.push(`Sent in the clear this session: ${plaintextTypes.join(', ')}.`);
  }
  if (encryptedTypes.length > 0) {
    parts.push(`Handed to the envelope command this session: ${encryptedTypes.join(', ')}.`);
  }
  if (plaintextTypes.length === 0 && encryptedTypes.length === 0) {
    parts.push('Nothing sensitive has been sent yet.');
  }

  return {
    level: 'partial',
    label: 'Not E2E',
    detail: parts.join(' '),
    encryptedTypes,
    plaintextTypes,
    coverage: MESSAGE_PROTECTION,
  };
}

export function useEncryption() {
  const [publicKey, setPublicKey] = useState('');
  const [deviceId, setDeviceId] = useState('');
  const [deviceName, setDeviceName] = useState('');
  const [initialized, setInitialized] = useState(false);

  // Session observation. The sets are refs so `record` — and therefore
  // `sendSensitive` — stay referentially stable: the callers pass it into
  // useCalls/useSms, whose callbacks are dependencies of the message-handler
  // registration effect, and churn there would re-register every handler on
  // each send. The sorted snapshots in state are what drive the status bar.
  const encryptedTypesRef = useRef<Set<string>>(new Set());
  const plaintextTypesRef = useRef<Set<string>>(new Set());
  const [observed, setObserved] = useState<{ encrypted: string[]; plaintext: string[] }>({
    encrypted: [],
    plaintext: [],
  });
  // Warn once per distinct failure reason — a send loop must not spam toasts.
  const warnedRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    void initEncryption();
  }, []);

  const initEncryption = async () => {
    try {
      const info = await invoke<EncryptionInfo>('get_device_info');
      setPublicKey(info.public_key || '');
      setDeviceId(info.device_id || '');
      setDeviceName(info.name || 'Desktop');
      setInitialized(true);
    } catch (err) {
      console.error('Failed to initialize encryption:', err);
    }
  };

  const getPublicKey = useCallback(async (): Promise<string> => {
    if (publicKey) return publicKey;
    const info = await invoke<EncryptionInfo>('get_device_info');
    setPublicKey(info.public_key);
    return info.public_key;
  }, [publicKey]);

  const record = useCallback((type: string, encrypted: boolean) => {
    const bucket = encrypted ? encryptedTypesRef.current : plaintextTypesRef.current;
    if (bucket.has(type)) return;
    bucket.add(type);
    setObserved({
      encrypted: [...encryptedTypesRef.current].sort(),
      plaintext: [...plaintextTypesRef.current].sort(),
    });
  }, []);

  /**
   * Hand one payload to the `send_encrypted_message` Tauri command.
   *
   * Returns a result rather than a bare boolean so the failure reason reaches
   * the caller and the status bar; it no longer swallows the error.
   */
  const sendEncrypted = useCallback(async (
    targetDeviceId: string,
    plaintext: string,
  ): Promise<SendResult> => {
    if (!targetDeviceId) {
      return { encrypted: false, reason: 'no destination device for this message' };
    }
    try {
      await invoke('send_encrypted_message', {
        targetDeviceId,
        plaintext,
      });
      return { encrypted: true };
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      console.error('Failed to send encrypted message:', err);
      return { encrypted: false, reason };
    }
  }, []);

  /**
   * The single entry point for sensitive payloads.
   *
   * Tries the envelope path first. On failure it records the type as having
   * left in the clear and warns once, then reports `encrypted: false` so the
   * caller can decide what to do — the point is that the decision is visible
   * rather than a silent downgrade buried in this function.
   */
  const sendSensitive = useCallback(async (
    targetDeviceId: string | null | undefined,
    message: Record<string, unknown>,
  ): Promise<SendResult> => {
    const type = typeof message.type === 'string' ? message.type : 'unknown';
    const result = await sendEncrypted(targetDeviceId ?? '', JSON.stringify(message));

    if (result.encrypted) {
      record(type, true);
      return result;
    }

    record(type, false);
    const reason = result.reason ?? 'unknown error';
    if (!warnedRef.current.has(reason)) {
      warnedRef.current.add(reason);
      showWarning(`Sending "${type}" unencrypted — encrypted delivery unavailable (${reason})`);
    }
    return result;
  }, [record, sendEncrypted]);

  const status = buildStatus(publicKey, observed.encrypted, observed.plaintext);

  return {
    publicKey,
    deviceId,
    deviceName,
    initialized,
    getPublicKey,
    sendEncrypted,
    sendSensitive,
    status,
  };
}
