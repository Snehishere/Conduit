import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface QRCodeProps {
  data: string;
  size?: number;
}

const QRCode: React.FC<QRCodeProps> = ({ data, size = 200 }) => {
  const [src, setSrc] = useState<string>('');

  useEffect(() => {
    if (!data) return;
    invoke<string>('generate_qr_code', { data })
      .then(setSrc)
      .catch((e) => {
        console.error('Failed to generate QR code:', e);
      });
  }, [data]);

  if (!src) {
    return <div className="qr-code" style={{ width: size, height: size, display: 'flex', alignItems: 'center', justifyContent: 'center', background: '#1a1a2e', borderRadius: 8, color: '#888', fontSize: 13 }}>Loading QR…</div>;
  }

  return <img src={src} alt="QR Code" className="qr-code" width={size} height={size} style={{ borderRadius: 8 }} />;
};

export default QRCode;
