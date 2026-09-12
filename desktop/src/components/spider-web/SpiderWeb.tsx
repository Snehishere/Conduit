import React, { useRef, useEffect, useState, useCallback } from 'react';
import DeviceDetail from './DeviceDetail';
import {
  IconDeviceDesktop,
  IconDevicePhone,
  IconDeviceTablet,
  IconDeviceEarbuds,
  IconDeviceHeadphones,
  IconDeviceTV,
  IconWatch,
  IconWeb,
  IconPlus,
} from '../icons';

interface Device {
  id: string;
  name: string;
  device_type: string;
  os: string;
  battery?: number;
  signal?: string;
  status: string;
}

interface SpiderWebProps {
  devices: Device[];
  selectedDevice: string | null;
  onSelectDevice: (id: string | null) => void;
  onPairDevice: () => void;
  onUnpairDevice?: (id: string) => void;
  onPingDevice?: (id: string) => void;
  onStartScreenMirror?: (deviceId: string, deviceName: string) => void;
  onStartRemoteInput?: (deviceId: string, deviceName: string) => void;
}

const DEVICE_ICON_PATHS: Record<string, string> = {
  desktop: 'M2 3h20v14H2zM8 21h8M12 17v4',
  phone: 'M5 2h14a2 2 0 012 2v16a2 2 0 01-2 2H5a2 2 0 01-2-2V4a2 2 0 012-2zM12 18h.01',
  tablet: 'M4 2h16a2 2 0 012 2v16a2 2 0 01-2 2H4a2 2 0 01-2-2V4a2 2 0 012-2zM12 18h.01',
  earbuds: 'M6 12h2a2 2 0 002-2V7a2 2 0 00-2-2H6v7zM16 12h2a2 2 0 002-2V7a2 2 0 00-2-2h-2v7zM8 17h8M7 17a3 3 0 01-3-3M17 17a3 3 0 003-3',
  headphones: 'M3 18v-6a9 9 0 0118 0v6M21 19a2 2 0 01-2 2h-1a2 2 0 01-2-2v-3a2 2 0 012-2h3zM3 19a2 2 0 002 2h1a2 2 0 002-2v-3a2 2 0 00-2-2H3z',
  watch: 'M12 7a5 5 0 100 10 5 5 0 000-10zM12 9v2l1.5 1.5M16.5 17.35l-.35 3.83a2 2 0 01-2 1.82H9.83a2 2 0 01-2-1.82l-.35-3.83M16.16 11l.35-3.83A2 2 0 0014.51 5.36L12 4.64',
  tv: 'M2 7h20v12a2 2 0 01-2 2H4a2 2 0 01-2-2V7zM17 2l-5 5-5-5',
};

function drawDeviceIcon(ctx: CanvasRenderingContext2D, type: string, x: number, y: number, size: number) {
  const pathData = DEVICE_ICON_PATHS[type] || DEVICE_ICON_PATHS.phone;
  ctx.save();
  ctx.translate(x, y);

  const scale = size / 24;
  ctx.scale(scale, scale);

  ctx.beginPath();
  const parts = pathData.split(/(?=[MLCHVQSTAZ])/i);
  let lastX = 0;
  let lastY = 0;
  let controlX = 0;
  let controlY = 0;

  for (const part of parts) {
    const match = part.trim().match(/^([MLCHVQSTAZ])\s*(.*)$/i);
    if (!match) continue;
    const [, cmd, args] = match;
    const nums = args ? args.split(/[\s,]+/).filter(Boolean).map(Number) : [];
    switch (cmd.toUpperCase()) {
      case 'M':
        lastX = nums[0];
        lastY = nums[1];
        ctx.moveTo(lastX, lastY);
        break;
      case 'L':
        lastX = nums[0];
        lastY = nums[1];
        ctx.lineTo(lastX, lastY);
        break;
      case 'H':
        lastX = nums[0];
        ctx.lineTo(lastX, lastY);
        break;
      case 'V':
        lastY = nums[0];
        ctx.lineTo(lastX, lastY);
        break;
      case 'C':
        controlX = nums[0];
        controlY = nums[1];
        lastX = nums[4];
        lastY = nums[5];
        ctx.bezierCurveTo(nums[0], nums[1], nums[2], nums[3], lastX, lastY);
        break;
      case 'Q':
        controlX = nums[0];
        controlY = nums[1];
        lastX = nums[2];
        lastY = nums[3];
        ctx.quadraticCurveTo(controlX, controlY, lastX, lastY);
        break;
      case 'A':
        // For simplicity, we'll approximate arcs with lines
        // In a production app, you might want to use a proper arc approximation
        const rx = nums[0];
        const ry = nums[1];
        const xAxisRotation = nums[2] * Math.PI / 180;
        const largeArcFlag = nums[3];
        const sweepFlag = nums[4];
        lastX = nums[5];
        lastY = nums[6];
        
        // Simple approximation: just draw a line
        // For better accuracy, you'd want to break this into multiple segments
        ctx.lineTo(lastX, lastY);
        break;
      case 'Z':
        ctx.closePath();
        break;
    }
  }
  
  ctx.strokeStyle = '#e6e1e5';
  ctx.lineWidth = 1.5;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.stroke();
  ctx.restore();
}

const SpiderWeb: React.FC<SpiderWebProps> = ({
  devices,
  selectedDevice,
  onSelectDevice,
  onPairDevice,
  onUnpairDevice,
  onPingDevice,
  onStartScreenMirror,
  onStartRemoteInput,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [positions, setPositions] = useState<Map<string, { x: number; y: number }>>(new Map());

  const activeSelected = devices.find((d) => d.id === selectedDevice);

  const layoutDevices = useCallback((width: number, height: number) => {
    const newPositions = new Map<string, { x: number; y: number }>();
    const centerX = width / 2;
    const centerY = height / 2;
    const radius = Math.min(width, height) * 0.35;

    if (devices.length > 0) {
      newPositions.set(devices[0].id, { x: centerX, y: centerY });
    }

    const otherDevices = devices.slice(1);
    const angleStep = (2 * Math.PI) / Math.max(otherDevices.length, 1);

    otherDevices.forEach((device, index) => {
      const angle = angleStep * index - Math.PI / 2;
      newPositions.set(device.id, {
        x: centerX + radius * Math.cos(angle),
        y: centerY + radius * Math.sin(angle),
      });
    });

    setPositions(newPositions);
  }, [devices]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const resizeCanvas = () => {
      const rect = canvas.parentElement?.getBoundingClientRect();
      if (!rect) return;
      const dpr = window.devicePixelRatio || 1;
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      canvas.style.width = `${rect.width}px`;
      canvas.style.height = `${rect.height}px`;
      const ctx = canvas.getContext('2d');
      if (ctx) ctx.scale(dpr, dpr);
      layoutDevices(rect.width, rect.height);
    };

    resizeCanvas();
    window.addEventListener('resize', resizeCanvas);
    return () => window.removeEventListener('resize', resizeCanvas);
  }, [layoutDevices]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = canvas.width / dpr;
    const h = canvas.height / dpr;

    ctx.clearRect(0, 0, w, h);

    const hubDevice = devices[0];
    if (!hubDevice) return;
    const hubPos = positions.get(hubDevice.id);
    if (!hubPos) return;

    // Draw threads
    devices.slice(1).forEach((device) => {
      const pos = positions.get(device.id);
      if (!pos) return;

      ctx.beginPath();
      ctx.moveTo(hubPos.x, hubPos.y);
      ctx.lineTo(pos.x, pos.y);

      ctx.strokeStyle = device.status === 'connected' ? 'rgba(45, 212, 191, 0.15)' : 'rgba(255,255,255,0.03)';
      ctx.lineWidth = device.status === 'connected' ? 1 : 1;
      ctx.setLineDash(device.status === 'paired' ? [5, 5] : []);
      ctx.stroke();
      ctx.setLineDash([]);
    });

    // Draw device nodes
    devices.forEach((device) => {
      const pos = positions.get(device.id);
      if (!pos) return;

      const isHub = device.device_type === 'desktop';
      const radius = isHub ? 36 : 28;
      const isSelected = selectedDevice === device.id;

      // Selected glow
      if (isSelected) {
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, radius + 8, 0, 2 * Math.PI);
        const gradient = ctx.createRadialGradient(pos.x, pos.y, radius, pos.x, pos.y, radius + 8);
        gradient.addColorStop(0, 'rgba(45, 212, 191, 0.12)');
        gradient.addColorStop(1, 'transparent');
        ctx.fillStyle = gradient;
        ctx.fill();
      }

      // Node circle
      ctx.beginPath();
      ctx.arc(pos.x, pos.y, radius, 0, 2 * Math.PI);
      ctx.fillStyle = 'rgba(16, 16, 24, 0.8)';
      ctx.fill();
      ctx.strokeStyle = isSelected ? 'rgba(45, 212, 191, 0.4)' : 'rgba(255,255,255,0.06)';
      ctx.lineWidth = isSelected ? 1.5 : 1;
      ctx.stroke();

      // Device icon inside node
      drawDeviceIcon(ctx, device.device_type, pos.x, pos.y, radius * 0.6);

      // Online dot (bottom-right)
      if (device.status === 'connected') {
        ctx.beginPath();
        ctx.arc(pos.x + radius * 0.65, pos.y + radius * 0.65, 5, 0, 2 * Math.PI);
        ctx.fillStyle = '#34d399';
        ctx.fill();
        ctx.strokeStyle = 'rgba(16, 16, 24, 0.8)';
        ctx.lineWidth = 2;
        ctx.stroke();
      }

      // Name below node
      ctx.fillStyle = '#938f99';
      ctx.font = '11px Inter, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillText(device.name, pos.x, pos.y + radius + 10);

      // Battery below name
      if (device.battery !== undefined) {
        ctx.fillStyle = '#605d64';
        ctx.font = '10px Inter, sans-serif';
        ctx.fillText(`${device.battery}%`, pos.x, pos.y + radius + 24);
      }
    });
  }, [positions, devices, selectedDevice]);

  const findDeviceAt = useCallback((x: number, y: number): string | null => {
    for (const device of devices) {
      const pos = positions.get(device.id);
      if (!pos) continue;
      const radius = device.device_type === 'desktop' ? 36 : 28;
      if (Math.hypot(x - pos.x, y - pos.y) < radius + 5) return device.id;
    }
    return null;
  }, [devices, positions]);

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    onSelectDevice(findDeviceAt(e.clientX - rect.left, e.clientY - rect.top));
  };

  const handleCanvasMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    canvas.style.cursor = findDeviceAt(e.clientX - rect.left, e.clientY - rect.top) ? 'pointer' : 'default';
  };

  if (devices.length === 0) {
    return (
      <div className="spider-web-empty">
        <div className="spider-web-empty-icon">
          <IconWeb size={64} />
        </div>
        <h2>No devices connected</h2>
        <p>Pair your first device to get started</p>
        <button className="pair-btn" onClick={onPairDevice}>
          <IconPlus size={16} /> Pair Device
        </button>
      </div>
    );
  }

  return (
    <div className="spider-web-container">
      <canvas
        ref={canvasRef}
        className="spider-web-canvas"
        onClick={handleCanvasClick}
        onMouseMove={handleCanvasMove}
      />
      {activeSelected && (
        <DeviceDetail
          device={activeSelected}
          onClose={() => onSelectDevice(null)}
          onUnpair={onUnpairDevice}
          onSendNotification={onPingDevice}
          onStartScreenMirror={onStartScreenMirror}
          onStartRemoteInput={onStartRemoteInput}
        />
      )}
    </div>
  );
};

export default SpiderWeb;
