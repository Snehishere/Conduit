// @ts-nocheck
import React, { useState, useEffect, useRef } from 'react';
import QRCode from './QRCode';
import { X, Check, Loader2, AlertTriangle, ShieldOff } from 'lucide-react';
import { invoke } from '../../lib/tauri';
import type { MessageHandler } from '../../types/websocket';

const MAX_DEVICES = 5;

interface PairingFlowProps {
  onClose: () => void;
  deviceCount: number;
  registerHandler?: (type: string, handler: MessageHandler) => () => void;
}

const PairingFlow: React.FC<PairingFlowProps> = ({ onClose, deviceCount, registerHandler }) => {
  const [step, setStep] = useState<'generating' | 'scan' | 'waiting' | 'complete' | 'error' | 'limit'>('generating');
  const [token, setToken] = useState('');
  const [publicKey, setPublicKey] = useState('');
  const [error, setError] = useState('');
  const [pairedDeviceName, setPairedDeviceName] = useState('');
  const [deviceName, setDeviceName] = useState('Desktop');
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (deviceCount >= MAX_DEVICES) {
      setStep('limit');
    } else {
      void generateToken();
    }
  }, [deviceCount]);

  useEffect(() => {
    if (!registerHandler || (step !== 'waiting' && step !== 'scan')) return;

    const unregister = registerHandler('pairing', (data) => {
      // Narrow on `type` — 'accept' is the only pairing variant, so the action
      // is statically known (an extra `action` comparison trips
      // no-unnecessary-condition / no-unnecessary-type-conversion).
      if (data.type === 'pairing') {
        const name = data.device_info.name || 'Device';
        setPairedDeviceName(name);
        setStep('complete');
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
      }
    });

    return unregister;
  }, [registerHandler, step]);

  useEffect(() => {
    if (step === 'scan') {
      timeoutRef.current = setTimeout(() => {
        setError('Pairing timed out. The device did not connect within 120 seconds.');
        setStep('error');
      }, 120000);
    }
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [step]);

  // Focus management: trap focus in dialog, restore on close
  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement;
    // Focus the close button on mount
    closeButtonRef.current?.focus();
    return () => {
      // Restore focus when dialog closes
      previousFocusRef.current?.focus();
    };
  }, []);

  // Escape + focus trap at document level so they work even when focus
  // temporarily leaves the dialog (e.g. after clicking the backdrop).
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
        return;
      }
      // Focus trap: Tab and Shift+Tab cycle within dialog
      if (e.key === 'Tab' && dialogRef.current) {
        const focusableElements = dialogRef.current.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
        );
        if (focusableElements.length === 0) return;
        const firstElement = focusableElements[0];
        const lastElement = focusableElements[focusableElements.length - 1];
        const active = document.activeElement;
        const focusInside = dialogRef.current.contains(active);

        if (e.shiftKey) {
          if (!focusInside || active === firstElement) {
            e.preventDefault();
            lastElement.focus();
          }
        } else {
          if (!focusInside || active === lastElement) {
            e.preventDefault();
            firstElement.focus();
          }
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => { document.removeEventListener('keydown', handleKeyDown); };
  }, [onClose]);

  const [ipAddress, setIpAddress] = useState<string>('');

  const generateToken = async () => {
    try {
      setStep('generating');

      const [newToken, deviceInfo, localIp] = await Promise.all([
        invoke<string>('generate_pairing_token'),
        invoke<{ public_key: string; name: string; type: string; os: string }>('get_device_info'),
        invoke<string>('get_local_ip'),
      ]);

      setToken(newToken);
      setPublicKey(deviceInfo.public_key);
      setIpAddress(localIp);
      if (deviceInfo.name) setDeviceName(deviceInfo.name);
      setStep('scan');
    } catch (err) {
      console.error('Pairing error:', err);
      setError(`Failed to generate pairing token: ${String(err)}`);
      setStep('error');
    }
  };



  const pairingData = JSON.stringify({
    token,
    public_key: publicKey,
    device_name: deviceName,
    device_type: 'desktop',
    ip: ipAddress,
    port: 9527,
  });

  return (
    <div
      className="scrim fixed inset-0 z-[200] flex items-center justify-center"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label="Pair a new device"
        className="surf-frost w-[352px] rounded-[24px] overflow-hidden"
        style={{
          boxShadow:
            '0 32px 80px -24px rgba(0, 0, 0, 0.9), 0 0 0 0.5px rgba(0, 0, 0, 0.16)',
        }}
        onClick={(e) => { e.stopPropagation(); }}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-ink-line px-5 py-3">
          <h2 className="text-[15px] font-semibold text-ink-1" id="pairing-dialog-title">Pair device</h2>
          <button
            ref={closeButtonRef}
            onClick={onClose}
            className="flex w-7 h-7 items-center justify-center rounded-lg bg-fill-1 text-ink-3 transition-colors hover:bg-fill-2 hover:text-ink-1"
            aria-label="Close pairing dialog"
          >
            <X size={14} strokeWidth={1.5} />
          </button>
        </div>

        {/* Content */}
        <div className="px-5 py-5" aria-live="polite">
          {step === 'generating' && (
            <div className="flex flex-col items-center gap-3 py-6" role="status">
              <Loader2 size={24} className="text-accent-ink animate-spin" strokeWidth={1.5} aria-hidden="true" />
              <div className="text-center">
                <p className="text-[14px] text-ink-1 font-medium">Generating pairing code...</p>
                <p className="text-[12px] text-ink-2 mt-1">Preparing a pairing code</p>
              </div>
            </div>
          )}

          {step === 'scan' && (
            <div className="flex flex-col items-center gap-4">
              <div
                className="w-[176px] h-[176px] rounded-xl bg-white border border-line-1 p-3 flex items-center justify-center"
                role="img"
                aria-label="QR code for device pairing"
              >
                <QRCode data={pairingData} size={152} />
              </div>
              <div className="text-center">
                <p className="text-[14px] text-ink-1 font-medium">Scan with your mobile device</p>
                <p className="text-[12px] text-ink-2 mt-1">
                  Open the Conduit app and tap "Scan QR Code"
                </p>
              </div>
              <div className="text-center">
                <p className="text-[11px] text-ink-3 mb-1.5">Or enter manually:</p>
                <code
                  className="rounded-lg px-3 py-1.5 text-[13px] font-mono text-accent-ink"
                  style={{
                    background: 'rgba(14, 116, 144, 0.10)',
                    border: '1px solid rgba(14, 116, 144, 0.22)',
                  }}
                  aria-label={`Pairing code: ${token.length >= 8 ? `${token.slice(0, 4)}-${token.slice(4, 8)}` : token}`}
                >
                  {token.length >= 8 ? `${token.slice(0, 4)}-${token.slice(4, 8)}` : token}
                </code>
              </div>
              <div className="mt-4 flex flex-col items-center">
                <button
                  onClick={async () => {
                    try {
                      await invoke('fix_firewall');
                    } catch (e) {
                      console.error('Failed to fix firewall', e);
                    }
                  }}
                  className="surf-clear rounded-full px-3 py-1.5 text-[12px] font-medium text-ink-1 transition-colors hover:bg-fill-2 border border-line-1"
                  aria-label="Troubleshoot connection — fix Windows Firewall permissions"
                >
                  Troubleshoot connection
                </button>
                  <p className="text-[11px] text-ink-3 mt-1.5 text-center max-w-[280px]">
                    Use this if the mobile app cannot connect. It will ask for
                    administrator permission to allow Conduit through the Windows
                    firewall.
                  </p>
              </div>
            </div>
          )}

          {step === 'complete' && (
            <div className="flex flex-col items-center gap-3 py-6" role="status">
              <div className="w-12 h-12 rounded-full flex items-center justify-center" style={{ background: 'rgba(21, 128, 61, 0.10)' }}>
                <Check size={22} className="text-success-ink" strokeWidth={2} aria-hidden="true" />
              </div>
              <div className="text-center">
                <p className="text-[15px] text-ink-1 font-semibold">Device paired</p>
                <p className="text-[12px] text-ink-2 mt-1">{pairedDeviceName} is now connected</p>
              </div>
              <button
                onClick={onClose}
                className="btn-press flex h-9 w-full items-center justify-center rounded-full btn-solid text-[13px] font-semibold transition-colors"
              >
                Done
              </button>
            </div>
          )}

          {step === 'error' && (
            <div className="flex flex-col items-center gap-3 py-6" role="alert">
              <div className="w-12 h-12 rounded-full flex items-center justify-center" style={{ background: 'rgba(185, 28, 28, 0.10)' }}>
                <AlertTriangle size={22} className="text-danger-ink" strokeWidth={1.5} aria-hidden="true" />
              </div>
              <div className="text-center">
                <p className="text-[14px] text-ink-1 font-medium">Pairing failed</p>
                <p className="text-[12px] text-ink-2 mt-1 max-w-[280px]">{error}</p>
              </div>
              <button
                onClick={generateToken}
                className="btn-press flex h-9 w-full items-center justify-center rounded-full btn-solid text-[13px] font-semibold transition-colors"
              >
                Try again
              </button>
            </div>
          )}

          {step === 'limit' && (
            <div className="flex flex-col items-center gap-3 py-6" role="alert">
              <div className="w-12 h-12 rounded-full flex items-center justify-center" style={{ background: 'rgba(180, 83, 9, 0.10)' }}>
                <ShieldOff size={22} className="text-warning-ink" strokeWidth={1.5} aria-hidden="true" />
              </div>
              <div className="text-center">
                <p className="text-[14px] text-ink-1 font-medium">Maximum devices reached</p>
                <p className="text-[12px] text-ink-2 mt-1">Unpair an existing device first (limit: {MAX_DEVICES})</p>
              </div>
              <button
                onClick={onClose}
                className="btn-press flex h-9 w-full items-center justify-center rounded-full btn-solid text-[13px] font-semibold transition-colors"
              >
                Close
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default PairingFlow;
