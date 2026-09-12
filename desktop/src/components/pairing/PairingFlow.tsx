import React, { useState, useEffect, useRef } from 'react';
import QRCode from './QRCode';
import { IconX, IconCheck } from '../icons';

const MAX_DEVICES = 5;

interface PairingFlowProps {
  onClose: () => void;
  deviceCount: number;
  registerHandler?: (type: string, handler: (data: any) => void) => () => void;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
  return tauriInvoke(cmd, args);
}

const PairingFlow: React.FC<PairingFlowProps> = ({ onClose, deviceCount, registerHandler }) => {
  const [step, setStep] = useState<'generating' | 'scan' | 'waiting' | 'complete' | 'error' | 'limit'>('generating');
  const [token, setToken] = useState('');
  const [publicKey, setPublicKey] = useState('');
  const [error, setError] = useState('');
  const [pairedDeviceName, setPairedDeviceName] = useState('');
  const [deviceName, setDeviceName] = useState('Desktop');
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (deviceCount >= MAX_DEVICES) {
      setStep('limit');
    } else {
      generateToken();
    }
  }, [deviceCount]);

  useEffect(() => {
    if (!registerHandler || step !== 'waiting') return;

    const unregister = registerHandler('pairing', (data: Record<string, unknown>) => {
      if (data.action === 'accept') {
        const name = (data.device_info as Record<string, unknown>)?.name as string || 'Device';
        setPairedDeviceName(name);
        setStep('complete');
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
      }
    });

    return unregister;
  }, [registerHandler, step]);

  useEffect(() => {
    if (step === 'waiting') {
      timeoutRef.current = setTimeout(() => {
        setError('Pairing timed out. The device did not connect within 60 seconds.');
        setStep('error');
      }, 60000);
    }
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [step]);

  const generateToken = async () => {
    try {
      setStep('generating');

      const [newToken, deviceInfo] = await Promise.all([
        invoke<string>('generate_pairing_token'),
        invoke<{ public_key: string; name: string; type: string; os: string }>('get_device_info'),
      ]);

      setToken(newToken);
      setPublicKey(deviceInfo.public_key);
      if (deviceInfo.name) setDeviceName(deviceInfo.name);
      setStep('scan');
    } catch (err) {
      console.error('Pairing error:', err);
      setError(`Failed to generate pairing token: ${String(err)}`);
      setStep('error');
    }
  };

  const handleScanComplete = () => {
    setStep('waiting');
  };

  const pairingData = JSON.stringify({
    token,
    public_key: publicKey,
    device_name: deviceName,
    device_type: 'desktop',
  });

  return (
    <div className="pairing-overlay" onClick={onClose}>
      <div className="pairing-modal" onClick={(e) => e.stopPropagation()}>
        <div className="pairing-header">
          <h2>Pair Device</h2>
          <button className="pairing-close" onClick={onClose}>
            <IconX size={16} />
          </button>
        </div>
        <div className="pairing-body">
          {step === 'generating' && (
            <div className="pairing-loading">
              <div className="spinner" />
              <p>Generating pairing code...</p>
            </div>
          )}
          {step === 'scan' && (
            <div className="pairing-scan">
              <QRCode data={pairingData} size={200} />
              <p className="pairing-instruction">
                Scan this QR code with your mobile device
              </p>
              <p className="pairing-token">Or enter manually: {token.slice(0, 8)}...</p>
              <button className="retry-btn" style={{ marginTop: 12 }} onClick={handleScanComplete}>
                I've Scanned It
              </button>
            </div>
          )}
          {step === 'waiting' && (
            <div className="pairing-waiting">
              <div className="spinner" />
              <p>Waiting for device to connect...</p>
              <p style={{ fontSize: 12, color: 'var(--text-muted)', marginTop: 8 }}>Make sure the mobile app is scanning</p>
            </div>
          )}
          {step === 'complete' && (
            <div className="pairing-complete">
              <span className="pairing-check"><IconCheck size={48} /></span>
              <h3>Device Paired!</h3>
              <p>{pairedDeviceName} is now connected</p>
              <button className="retry-btn" style={{ marginTop: 12 }} onClick={onClose}>Done</button>
            </div>
          )}
          {step === 'error' && (
            <div className="pairing-error">
              <span className="pairing-error-icon">⚠️</span>
              <p>{error}</p>
              <button className="retry-btn" onClick={generateToken}>
                Try Again
              </button>
            </div>
          )}
          {step === 'limit' && (
            <div className="pairing-error">
              <span className="pairing-error-icon">🚫</span>
              <p>Maximum {MAX_DEVICES} devices reached</p>
              <p className="pairing-limit-hint">Unpair an existing device first</p>
              <button className="retry-btn" onClick={onClose}>OK</button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default PairingFlow;
