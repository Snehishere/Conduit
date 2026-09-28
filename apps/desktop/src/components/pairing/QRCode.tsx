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
      .catch((e: unknown) => {
        console.error('Failed to generate QR code:', e);
      });
  }, [data]);

  if (!src) {
    return <div className="flex items-center justify-center bg-bg-secondary rounded-lg text-text-muted text-[13px]" style={{ width: size, height: size }}>Loading QR…</div>;
  }

  return <img src={src} alt="QR Code" className="rounded-lg" width={size} height={size} />;
};

export default QRCode;
